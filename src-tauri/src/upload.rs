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
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(server.accept_invalid_certs)
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
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
    let location = resp
        .headers()
        .get("Location")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| anyhow!("server did not return a file URL"))?;
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
