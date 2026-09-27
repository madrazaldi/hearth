//! Byte transports for IRC: plain TCP, TLS, and WebSocket (over either).
//!
//! Every transport is reduced to a line reader and a line writer so the
//! protocol layer never cares how bytes travel.

use anyhow::{anyhow, Context, Result};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
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
    /// PEM file with a client certificate and key to present (CertFP).
    pub client_cert: Option<String>,
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
        let identity = match &ep.client_cert {
            Some(path) => Some(load_identity(path)?),
            None => None,
        };
        let connector = TlsConnector::from(tls_config(ep.accept_invalid_certs, identity)?);
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

type Identity = (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>);

/// Read a certificate chain and private key from one PEM file.
pub fn load_identity(path: &str) -> Result<Identity> {
    let certs = CertificateDer::pem_file_iter(path)
        .with_context(|| format!("could not read client certificate {path}"))?
        .collect::<Result<Vec<_>, _>>()
        .context("invalid client certificate")?;
    if certs.is_empty() {
        return Err(anyhow!("{path} contains no certificate"));
    }
    let key = PrivateKeyDer::from_pem_file(path).context("client certificate file has no usable private key")?;
    Ok((certs, key))
}

fn tls_config(accept_invalid: bool, identity: Option<Identity>) -> Result<Arc<rustls::ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("default TLS versions");
    let builder = if accept_invalid {
        builder.dangerous().with_custom_certificate_verifier(Arc::new(NoVerify(provider)))
    } else {
        let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        builder.with_root_certificates(roots)
    };
    let config = match identity {
        Some((certs, key)) => builder.with_client_auth_cert(certs, key).context("unusable client certificate")?,
        None => builder.with_no_client_auth(),
    };
    Ok(Arc::new(config))
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

    async fn listen() -> (TcpListener, Endpoint) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ep = Endpoint {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            tls: false,
            accept_invalid_certs: false,
            websocket_path: None,
            client_cert: None,
        };
        (listener, ep)
    }

    #[tokio::test]
    async fn raw_lines_are_split_trimmed_and_decoded() {
        let (listener, ep) = listen().await;
        let (client, server) = tokio::join!(connect(&ep), listener.accept());
        let (mut reader, mut writer) = client.unwrap();
        let (mut sock, _) = server.unwrap();

        sock.write_all(b"PING a\r\n\r\n\nPING b\n:x PRIVMSG #a :caf\xe9\r\n:x PRIVMSG #a :caf\xc3\xa9\r\nPARTIAL").await.unwrap();
        sock.shutdown().await.unwrap();
        assert_eq!(reader.next_line().await.unwrap().as_deref(), Some("PING a"));
        assert_eq!(reader.next_line().await.unwrap().as_deref(), Some("PING b"));
        // Invalid UTF-8 falls back to Latin-1.
        assert_eq!(reader.next_line().await.unwrap().as_deref(), Some(":x PRIVMSG #a :café"));
        assert_eq!(reader.next_line().await.unwrap().as_deref(), Some(":x PRIVMSG #a :café"));
        // A final line without a terminator still counts.
        assert_eq!(reader.next_line().await.unwrap().as_deref(), Some("PARTIAL"));
        assert_eq!(reader.next_line().await.unwrap(), None);

        writer.send_line("NICK me").await.unwrap();
        writer.close().await;
        let mut got = String::new();
        sock.read_to_string(&mut got).await.unwrap();
        assert_eq!(got, "NICK me\r\n");
    }

    #[tokio::test]
    async fn overlong_lines_are_rejected() {
        let (listener, ep) = listen().await;
        let (client, server) = tokio::join!(connect(&ep), listener.accept());
        let (mut reader, _writer) = client.unwrap();
        let (mut sock, _) = server.unwrap();
        tokio::spawn(async move {
            let _ = sock.write_all(&vec![b'a'; MAX_LINE + 10]).await;
            let _ = sock.write_all(b"\r\n").await;
            let _ = sock.shutdown().await;
        });
        assert!(reader.next_line().await.is_err());
    }

    #[tokio::test]
    async fn websocket_frames_are_lines() {
        let (listener, mut ep) = listen().await;
        ep.websocket_path = Some("irc".into());
        let server = tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            // The callback's signature is tungstenite's, large error type and all.
            #[allow(clippy::result_large_err)]
            let pick = |req: &Request, mut resp: Response| {
                assert_eq!(req.uri().path(), "/irc");
                let offered = req.headers()["Sec-WebSocket-Protocol"].to_str().unwrap();
                assert!(offered.contains("text.ircv3.net"), "{offered}");
                resp.headers_mut().insert("Sec-WebSocket-Protocol", HeaderValue::from_static("text.ircv3.net"));
                Ok(resp)
            };
            let mut ws = tokio_tungstenite::accept_hdr_async(sock, pick).await.unwrap();
            ws.send(WsMessage::text("PING a\r\n")).await.unwrap();
            ws.send(WsMessage::Ping(vec![1].into())).await.unwrap();
            ws.send(WsMessage::binary(b"PING b".to_vec())).await.unwrap();
            // The client answers our ping on its own; skip to its text.
            let got = loop {
                match ws.next().await.unwrap().unwrap() {
                    WsMessage::Pong(_) => continue,
                    other => break other,
                }
            };
            ws.close(None).await.unwrap();
            got
        });
        let (mut reader, mut writer) = connect(&ep).await.unwrap();
        assert_eq!(reader.next_line().await.unwrap().as_deref(), Some("PING a"));
        // Control frames are skipped.
        assert_eq!(reader.next_line().await.unwrap().as_deref(), Some("PING b"));
        writer.send_line("PONG a").await.unwrap();
        assert_eq!(reader.next_line().await.unwrap(), None);
        assert_eq!(server.await.unwrap(), WsMessage::text("PONG a"));
    }

    #[tokio::test]
    async fn unreachable_hosts_name_the_address() {
        let (listener, ep) = listen().await;
        drop(listener);
        let err = connect(&ep).await.err().expect("nothing listens there");
        assert!(format!("{err:#}").contains(&format!("could not reach 127.0.0.1:{}", ep.port)), "{err:#}");
    }

    #[test]
    fn client_certificates_must_contain_a_certificate() {
        let err = load_identity("/nonexistent/hearth-cert.pem").err().unwrap();
        assert!(format!("{err:#}").contains("could not read client certificate"));
        let path = std::env::temp_dir().join(format!("hearth-empty-{}.pem", std::process::id()));
        std::fs::write(&path, "not a pem file\n").unwrap();
        let err = load_identity(path.to_str().unwrap()).err().unwrap();
        std::fs::remove_file(&path).ok();
        assert!(format!("{err:#}").contains("contains no certificate"), "{err:#}");
    }

    #[test]
    fn decode_prefers_utf8() {
        assert_eq!(decode("naïve ☃".as_bytes()), "naïve ☃");
        assert_eq!(decode(b"\xa3\xff"), "£ÿ");
    }
}
