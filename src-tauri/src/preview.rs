//! Link previews, fetched by the backend so the webview never talks to
//! third-party hosts. Images come back as data URLs.
//!
//! Only public addresses are fetched: a link in a channel must not be able to
//! make Hearth probe the user's LAN or localhost.

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_HTML: usize = 512 * 1024;
const MAX_IMAGE: usize = 4 * 1024 * 1024;
const CACHE: usize = 300;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub url: String,
    /// "image" for a direct image link, "page" for an HTML page.
    pub kind: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site: Option<String>,
    /// A data: URL.
    pub image: Option<String>,
}

type Cached = Option<Arc<Preview>>;

pub struct Previewer {
    client: reqwest::Client,
    cache: Mutex<(HashMap<String, Cached>, VecDeque<String>)>,
}

impl Previewer {
    pub fn new() -> Self {
        let policy = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 4 {
                return attempt.error("too many redirects");
            }
            let url = attempt.url();
            let literal_ok = match url.host() {
                Some(url::Host::Ipv4(ip)) => is_public(&IpAddr::V4(ip)),
                Some(url::Host::Ipv6(ip)) => is_public(&IpAddr::V6(ip)),
                Some(url::Host::Domain(d)) => !is_local_name(d),
                None => false,
            };
            if matches!(url.scheme(), "http" | "https") && literal_ok {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        let client = reqwest::Client::builder()
            .redirect(policy)
            .timeout(Duration::from_secs(8))
            .connect_timeout(Duration::from_secs(5))
            .user_agent("Mozilla/5.0 (compatible; Hearth link preview)")
            .build()
            .expect("HTTP client");
        Previewer { client, cache: Mutex::new((HashMap::new(), VecDeque::new())) }
    }

    pub async fn preview(&self, url: &str) -> Result<Preview> {
        if let Some(hit) = self.cache.lock().unwrap().0.get(url).cloned() {
            return hit.map(|p| (*p).clone()).ok_or_else(|| anyhow!("no preview"));
        }
        let result = self.fetch(url).await;
        let entry = result.as_ref().ok().map(|p| Arc::new(p.clone()));
        let mut cache = self.cache.lock().unwrap();
        if cache.0.insert(url.to_string(), entry).is_none() {
            cache.1.push_back(url.to_string());
        }
        while cache.1.len() > CACHE {
            if let Some(old) = cache.1.pop_front() {
                cache.0.remove(&old);
            }
        }
        result
    }

    async fn fetch(&self, url: &str) -> Result<Preview> {
        let parsed = reqwest::Url::parse(url).context("invalid URL")?;
        check_public(&parsed).await?;
        let resp = self.client.get(parsed.clone()).send().await?.error_for_status()?;
        let final_url = resp.url().clone();
        let ctype = content_type(&resp);
        if ctype.starts_with("image/") {
            let image = image_data_url(&ctype, read_limited(resp, MAX_IMAGE).await?)?;
            return Ok(Preview {
                url: url.to_string(),
                kind: "image".into(),
                title: None,
                description: None,
                site: None,
                image: Some(image),
            });
        }
        if !ctype.starts_with("text/html") && !ctype.starts_with("application/xhtml") {
            return Err(anyhow!("nothing to preview"));
        }
        let body = read_limited(resp, MAX_HTML).await?;
        let meta = parse_meta(&String::from_utf8_lossy(&body));
        let pick = |keys: &[&str]| keys.iter().find_map(|k| meta.get(*k).cloned()).filter(|v| !v.trim().is_empty());
        let title = pick(&["og:title", "twitter:title", "<title>"]);
        let description = pick(&["og:description", "twitter:description", "description"]);
        let site = pick(&["og:site_name"]).or_else(|| final_url.host_str().map(|h| h.trim_start_matches("www.").to_string()));
        let mut image = None;
        if let Some(src) = pick(&["og:image:secure_url", "og:image", "twitter:image", "twitter:image:src"]) {
            if let Ok(img_url) = final_url.join(&src) {
                image = self.fetch_image(img_url).await.ok();
            }
        }
        if title.is_none() && description.is_none() && image.is_none() {
            return Err(anyhow!("nothing to preview"));
        }
        Ok(Preview {
            url: url.to_string(),
            kind: "page".into(),
            title: title.map(|t| clip(&t, 200)),
            description: description.map(|d| clip(&d, 400)),
            site,
            image,
        })
    }

    async fn fetch_image(&self, url: reqwest::Url) -> Result<String> {
        check_public(&url).await?;
        let resp = self.client.get(url).send().await?.error_for_status()?;
        let ctype = content_type(&resp);
        if !ctype.starts_with("image/") {
            return Err(anyhow!("not an image"));
        }
        image_data_url(&ctype, read_limited(resp, MAX_IMAGE).await?)
    }
}

impl Default for Previewer {
    fn default() -> Self {
        Self::new()
    }
}

fn content_type(resp: &reqwest::Response) -> String {
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn image_data_url(ctype: &str, bytes: Vec<u8>) -> Result<String> {
    let mime = ctype.split(';').next().unwrap_or("").trim();
    // SVG can carry script and external references; raster formats only.
    if !matches!(mime, "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif") {
        return Err(anyhow!("unsupported image type"));
    }
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

async fn read_limited(mut resp: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if resp.content_length().is_some_and(|n| n as usize > limit) {
        return Err(anyhow!("response too large"));
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        out.extend_from_slice(&chunk);
        if out.len() > limit {
            // HTML metadata lives in the head; a truncated page is still useful.
            if limit == MAX_HTML {
                out.truncate(limit);
                break;
            }
            return Err(anyhow!("response too large"));
        }
    }
    Ok(out)
}

async fn check_public(url: &reqwest::Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(anyhow!("only http and https links are previewed"));
    }
    let host = url.host_str().ok_or_else(|| anyhow!("URL has no host"))?;
    if is_local_name(host) {
        return Err(anyhow!("refusing to preview a local address"));
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<_> = tokio::net::lookup_host((host, port)).await.context("could not resolve host")?.collect();
    if addrs.is_empty() || addrs.iter().any(|a| !is_public(&a.ip())) {
        return Err(anyhow!("refusing to preview a private address"));
    }
    Ok(())
}

fn is_local_name(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") || h.ends_with(".internal") || !h.contains('.') && h.parse::<IpAddr>().is_err()
}

pub fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1]))
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(&IpAddr::V4(v4));
            }
            let seg = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (seg[0] & 0xfe00) == 0xfc00
                || (seg[0] & 0xffc0) == 0xfe80
                || (seg[0] == 0x2001 && seg[1] == 0x0db8))
        }
    }
}

fn clip(s: &str, max: usize) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= max {
        return s;
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Pull `<meta property|name=… content=…>` pairs and the `<title>` out of HTML.
/// Keys are lowercased; the title is stored under `<title>`.
fn parse_meta(html: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let lower = html.to_ascii_lowercase();
    // Metadata is in <head>; stop at <body> to avoid scanning huge pages.
    let end = lower.find("<body").unwrap_or(lower.len());
    let mut pos = 0;
    while let Some(i) = lower[pos..end].find("<meta") {
        let start = pos + i;
        let Some(close) = lower[start..end].find('>') else { break };
        let tag = &html[start + 5..start + close];
        let attrs = parse_attrs(tag);
        let key = attrs.get("property").or_else(|| attrs.get("name"));
        if let (Some(k), Some(v)) = (key, attrs.get("content")) {
            out.entry(k.to_ascii_lowercase()).or_insert_with(|| decode_entities(v));
        }
        pos = start + close;
    }
    if let Some(i) = lower[..end].find("<title") {
        if let Some(gt) = lower[i..end].find('>') {
            let from = i + gt + 1;
            if let Some(stop) = lower[from..end].find("</title") {
                out.insert("<title>".into(), decode_entities(html[from..from + stop].trim()));
            }
        }
    }
    out
}

fn parse_attrs(tag: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let b = tag.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'/') {
            i += 1;
        }
        let ks = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'=' && b[i] != b'/' {
            i += 1;
        }
        let key = tag[ks..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'=' {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            let value = if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let q = b[i];
                i += 1;
                let vs = i;
                while i < b.len() && b[i] != q {
                    i += 1;
                }
                let v = &tag[vs..i];
                i += 1;
                v
            } else {
                let vs = i;
                while i < b.len() && !b[i].is_ascii_whitespace() {
                    i += 1;
                }
                &tag[vs..i]
            };
            if !key.is_empty() {
                out.insert(key, value.to_string());
            }
        } else if key.is_empty() {
            i += 1;
        }
    }
    out
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let name = &rest[1..semi];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_open_graph() {
        let html = r#"<html><head><title>Plain &amp; simple</title>
            <meta property="og:title" content="Hearth &#8212; IRC">
            <meta name=description content='A quiet client'>
            <META PROPERTY="og:image" CONTENT="/img.png"/>
            </head><body><meta property="og:title" content="ignored"></body></html>"#;
        let m = parse_meta(html);
        assert_eq!(m.get("og:title").unwrap(), "Hearth — IRC");
        assert_eq!(m.get("description").unwrap(), "A quiet client");
        assert_eq!(m.get("og:image").unwrap(), "/img.png");
        assert_eq!(m.get("<title>").unwrap(), "Plain & simple");
    }

    #[test]
    fn classifies_addresses() {
        for ip in ["10.0.0.1", "127.0.0.1", "192.168.1.1", "169.254.1.1", "100.64.0.1", "::1", "fd00::1", "fe80::1", "::ffff:10.0.0.1"] {
            assert!(!is_public(&ip.parse().unwrap()), "{ip} should be private");
        }
        for ip in ["1.1.1.1", "93.184.216.34", "2606:4700::1111"] {
            assert!(is_public(&ip.parse().unwrap()), "{ip} should be public");
        }
        assert!(is_local_name("localhost"));
        assert!(is_local_name("printer.local"));
        assert!(is_local_name("router"));
        assert!(!is_local_name("example.com"));
    }

    #[test]
    fn clips_and_rejects_svg() {
        assert_eq!(clip("a   b\n c", 10), "a b c");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert!(image_data_url("image/svg+xml", vec![1]).is_err());
        assert!(image_data_url("image/png; charset=binary", vec![1]).unwrap().starts_with("data:image/png;base64,"));
    }
}
