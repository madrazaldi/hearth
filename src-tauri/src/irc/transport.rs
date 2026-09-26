//! Byte transports for IRC: plain TCP, TLS, and WebSocket (over either).
//!
//! Every transport is reduced to a line reader and a line writer so the
//! protocol layer never cares how bytes travel.

use anyhow::{anyhow, Context, Result};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::WebSocketStream;

const MAX_LINE: usize = 64 * 1024;

pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}
type BoxStream = Box<dyn AsyncStream>;

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub accept_invalid_certs: bool,
    /// When set, speak IRC over WebSocket at this path.
    pub websocket_path: Option<String>,
}

pub enum LineReader {
    Raw(BufReader<ReadHalf<BoxStream>>),
    Ws(SplitStream<WebSocketStream<BoxStream>>),
}

pub enum LineWriter {
    Raw(WriteHalf<BoxStream>),
    Ws(SplitSink<WebSocketStream<BoxStream>, WsMessage>),
}

pub async fn connect(ep: &Endpoint) -> Result<(LineReader, LineWriter)> {
    let tcp = tokio::time::timeout(
        Duration::from_secs(20),
        TcpStream::connect((ep.host.as_str(), ep.port)),
    )
    .await
    .map_err(|_| anyhow!("timed out connecting to {}:{}", ep.host, ep.port))?
    .with_context(|| format!("could not reach {}:{}", ep.host, ep.port))?;
    tcp.set_nodelay(true).ok();

    let stream: BoxStream = if ep.tls {
        let connector = TlsConnector::from(tls_config(ep.accept_invalid_certs));
        let name = ServerName::try_from(ep.host.clone()).context("invalid TLS server name")?;
        let tls = tokio::time::timeout(Duration::from_secs(20), connector.connect(name, tcp))
            .await
            .map_err(|_| anyhow!("TLS handshake timed out"))?
            .context("TLS handshake failed")?;
        Box::new(tls)
    } else {
        Box::new(tcp)
    };

    match &ep.websocket_path {
        Some(path) => {
            let scheme = if ep.tls { "wss" } else { "ws" };
            let path = if path.starts_with('/') { path.clone() } else { format!("/{path}") };
            let url = format!("{scheme}://{}:{}{}", ep.host, ep.port, path);
            let mut req = url.into_client_request()?;
            req.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                HeaderValue::from_static("text.ircv3.net, binary.ircv3.net"),
            );
            let mut config = WebSocketConfig::default();
            config.max_message_size = Some(MAX_LINE);
            let (ws, _) = tokio_tungstenite::client_async_with_config(req, stream, Some(config))
                .await
                .context("WebSocket handshake failed")?;
            let (sink, stream) = ws.split();
            Ok((LineReader::Ws(stream), LineWriter::Ws(sink)))
        }
        None => {
            let (r, w) = tokio::io::split(stream);
            Ok((LineReader::Raw(BufReader::new(r)), LineWriter::Raw(w)))
        }
    }
}

impl LineReader {
    /// Next complete line without its terminator. `None` means the peer closed.
    pub async fn next_line(&mut self) -> Result<Option<String>> {
        match self {
            LineReader::Raw(r) => {
                let mut buf = Vec::with_capacity(512);
                loop {
                    let n = r.read_until(b'\n', &mut buf).await?;
                    if n == 0 {
                        return Ok(None);
                    }
                    if buf.len() > MAX_LINE {
                        return Err(anyhow!("line exceeds {MAX_LINE} bytes"));
                    }
                    while matches!(buf.last(), Some(b'\n' | b'\r')) {
                        buf.pop();
                    }
                    if !buf.is_empty() {
                        return Ok(Some(decode(&buf)));
                    }
                }
            }
            LineReader::Ws(s) => loop {
                match s.next().await {
                    None => return Ok(None),
                    Some(Err(e)) => return Err(e.into()),
                    Some(Ok(WsMessage::Text(t))) => return Ok(Some(t.as_str().trim_end().to_string())),
                    Some(Ok(WsMessage::Binary(b))) => return Ok(Some(decode(&b).trim_end().to_string())),
                    Some(Ok(WsMessage::Close(_))) => return Ok(None),
                    Some(Ok(_)) => continue,
                }
            },
        }
    }
}

impl LineWriter {
    pub async fn send_line(&mut self, line: &str) -> Result<()> {
        match self {
            LineWriter::Raw(w) => {
                let mut buf = Vec::with_capacity(line.len() + 2);
                buf.extend_from_slice(line.as_bytes());
                buf.extend_from_slice(b"\r\n");
                w.write_all(&buf).await?;
                w.flush().await?;
            }
            LineWriter::Ws(s) => s.send(WsMessage::text(line)).await?,
        }
        Ok(())
    }

    pub async fn close(&mut self) {
        match self {
            LineWriter::Raw(w) => {
                let _ = w.shutdown().await;
            }
            LineWriter::Ws(s) => {
                let _ = s.close().await;
            }
        }
    }
}

/// IRC has no mandated encoding; prefer UTF-8 and fall back to Latin-1.
fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

fn tls_config(accept_invalid: bool) -> Arc<rustls::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("default TLS versions");
    let config = if accept_invalid {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
            .with_no_client_auth()
    } else {
        let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        builder.with_root_certificates(roots).with_no_client_auth()
    };
    Arc::new(config)
}

#[derive(Debug)]
struct NoVerify(Arc<rustls::crypto::CryptoProvider>);

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
