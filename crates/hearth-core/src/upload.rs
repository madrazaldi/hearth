//! Uploads to a soju.im/FILEHOST endpoint, authenticated with the same
//! credentials as the IRC session.

use crate::config::ServerConfig;
use anyhow::{anyhow, Context, Result};

pub async fn upload(server: &ServerConfig, url: &str, filename: &str, content_type: &str, body: Vec<u8>) -> Result<String> {
    let (user, pass) = if !server.sasl_password.is_empty() {
        let user = if server.sasl_username.is_empty() { &server.nick } else { &server.sasl_username };
        (user.clone(), server.sasl_password.clone())
    } else {
        (server.username.clone(), server.password.clone())
    };
    let safe_name: String = filename.chars().filter(|c| !c.is_control() && *c != '"').collect();
    let client = reqwest::Client::builder().danger_accept_invalid_certs(server.accept_invalid_certs).redirect(reqwest::redirect::Policy::none()).build()?;
    let mut req = client
        .post(url)
        .header("Content-Type", if content_type.is_empty() { "application/octet-stream" } else { content_type })
        .header("Content-Disposition", format!("attachment; filename=\"{safe_name}\""))
        .body(body);
    if !pass.is_empty() {
        req = req.basic_auth(user, Some(pass));
    }
    let resp = req.send().await.context("upload failed")?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow!("upload rejected ({status}): {}", text.trim()));
    }
    let location = resp.headers().get("Location").and_then(|v| v.to_str().ok()).ok_or_else(|| anyhow!("server did not return a file URL"))?;
    let base = reqwest::Url::parse(url)?;
    Ok(base.join(location)?.to_string())
}

pub fn guess_type(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" => "text/plain; charset=utf-8",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Seen {
        method: String,
        headers: Vec<(String, Vec<u8>)>,
        body: Vec<u8>,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&[u8]> {
            self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_slice())
        }

        fn text(&self, name: &str) -> Option<String> {
            self.header(name).map(|v| String::from_utf8_lossy(v).into_owned())
        }
    }

    type Log = Arc<Mutex<Vec<Seen>>>;

    /// An HTTP server on loopback that answers every request with `response`.
    async fn host(response: impl Into<String>) -> (String, Log) {
        let response: Arc<str> = Arc::from(response.into());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/files/up", listener.local_addr().unwrap());
        let seen: Log = Arc::default();
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (log, response) = (log.clone(), response.clone());
                tokio::spawn(async move {
                    let mut raw = Vec::new();
                    let mut buf = [0u8; 4096];
                    let head_end = loop {
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        raw.extend_from_slice(&buf[..n]);
                        if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let head = raw[..head_end].to_vec();
                    let mut lines = head.split(|&b| b == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l));
                    let method = String::from_utf8_lossy(lines.next().unwrap()).split(' ').next().unwrap().to_string();
                    let headers: Vec<(String, Vec<u8>)> = lines
                        .filter_map(|l| {
                            let i = l.iter().position(|&b| b == b':')?;
                            Some((String::from_utf8_lossy(&l[..i]).into_owned(), l[i + 1..].trim_ascii().to_vec()))
                        })
                        .collect();
                    let want: usize = headers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .and_then(|(_, v)| String::from_utf8_lossy(v).parse().ok())
                        .unwrap_or(0);
                    let mut body = raw[head_end..].to_vec();
                    while body.len() < want {
                        let n = sock.read(&mut buf).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        body.extend_from_slice(&buf[..n]);
                    }
                    log.lock().unwrap().push(Seen { method, headers, body });
                    let _ = sock.write_all(response.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        (url, seen)
    }

    const CREATED: &str = "HTTP/1.1 201 Created\r\nLocation: /files/abc.png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    fn cfg() -> ServerConfig {
        ServerConfig { id: "s".into(), nick: "ember".into(), ..Default::default() }
    }

    #[tokio::test]
    async fn posts_the_file_and_resolves_the_location_against_the_endpoint() {
        let (url, seen) = host(CREATED).await;
        let got = upload(&cfg(), &url, "a.png", "image/png", vec![1, 2, 3]).await.unwrap();
        assert_eq!(got, reqwest::Url::parse(&url).unwrap().join("/files/abc.png").unwrap().to_string());
        let seen = seen.lock().unwrap();
        let s = &seen[0];
        assert_eq!(s.method, "POST");
        assert_eq!(s.text("content-type").as_deref(), Some("image/png"));
        assert_eq!(s.text("content-disposition").as_deref(), Some("attachment; filename=\"a.png\""));
        assert_eq!(s.body, [1, 2, 3]);
        assert!(s.header("authorization").is_none());
    }

    #[tokio::test]
    async fn absolute_location_headers_are_kept() {
        let (url, _) = host("HTTP/1.1 201 Created\r\nLocation: https://files.example/x/y.png\r\nContent-Length: 0\r\n\r\n").await;
        assert_eq!(upload(&cfg(), &url, "y.png", "image/png", vec![]).await.unwrap(), "https://files.example/x/y.png");
    }

    #[tokio::test]
    async fn authenticates_with_the_sasl_login_when_there_is_one_else_the_server_password() {
        let (url, seen) = host(CREATED).await;
        let basic = |u: &str, p: &str| format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{u}:{p}")));
        let plain = ServerConfig { sasl: crate::config::SaslMechanism::Plain, sasl_username: "acct".into(), sasl_password: "pw1".into(), ..cfg() };
        upload(&plain, &url, "f", "", vec![]).await.unwrap();
        // The SASL username defaults to the nick.
        upload(&ServerConfig { sasl_password: "pw2".into(), ..cfg() }, &url, "f", "", vec![]).await.unwrap();
        upload(&ServerConfig { username: "u".into(), password: "serverpw".into(), ..cfg() }, &url, "f", "", vec![]).await.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].text("authorization"), Some(basic("acct", "pw1")));
        assert_eq!(seen[1].text("authorization"), Some(basic("ember", "pw2")));
        assert_eq!(seen[2].text("authorization"), Some(basic("u", "serverpw")));
        // An empty content type becomes a generic one.
        assert_eq!(seen[2].text("content-type").as_deref(), Some("application/octet-stream"));
    }

    #[tokio::test]
    async fn file_names_are_cleaned_of_quotes_and_control_characters_and_keep_their_utf8_bytes() {
        let (url, seen) = host(CREATED).await;
        upload(&cfg(), &url, "we\"ird\r\nna\0me caf\u{e9}\u{2603}\u{85}.txt", "text/plain", vec![]).await.unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].header("content-disposition").unwrap(), "attachment; filename=\"weirdname caf\u{e9}\u{2603}.txt\"".as_bytes());
    }

    #[tokio::test]
    async fn a_rejection_carries_the_status_and_the_server_text() {
        let (url, _) = host("HTTP/1.1 413 Payload Too Large\r\nContent-Length: 17\r\nConnection: close\r\n\r\n quota exceeded \n").await;
        let err = upload(&cfg(), &url, "a", "", vec![]).await.unwrap_err();
        assert_eq!(err.to_string(), "upload rejected (413 Payload Too Large): quota exceeded");
    }

    #[tokio::test]
    async fn a_missing_location_is_an_error() {
        let (url, _) = host("HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n").await;
        let err = upload(&cfg(), &url, "a", "", vec![]).await.unwrap_err();
        assert_eq!(err.to_string(), "server did not return a file URL");
    }

    #[tokio::test]
    async fn redirects_are_not_followed_so_credentials_stay_put() {
        let (other, other_seen) = host(CREATED).await;
        let (url, _) = host(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {other}\r\nContent-Length: 0\r\n\r\n")).await;
        let err = upload(&ServerConfig { sasl_password: "pw".into(), ..cfg() }, &url, "a", "", vec![]).await.unwrap_err();
        assert!(err.to_string().starts_with("upload rejected (307"), "{err}");
        assert!(other_seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn connection_failures_are_labelled() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/up", listener.local_addr().unwrap());
        drop(listener);
        let err = upload(&cfg(), &url, "a", "", vec![]).await.unwrap_err();
        assert!(format!("{err:#}").starts_with("upload failed:"), "{err:#}");
    }

    #[test]
    fn guess_type_maps_known_extensions_case_insensitively_and_defaults_to_binary() {
        assert_eq!(guess_type("a.PNG"), "image/png");
        assert_eq!(guess_type("a.jpeg"), "image/jpeg");
        assert_eq!(guess_type("notes.log"), "text/plain; charset=utf-8");
        assert_eq!(guess_type("clip.webm"), "video/webm");
        assert_eq!(guess_type("x.tar.gz"), "application/octet-stream");
        assert_eq!(guess_type("noext"), "application/octet-stream");
    }
}
