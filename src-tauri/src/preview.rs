//! Link previews, fetched by the backend so the webview never talks to
//! third-party hosts. Images come back as data URLs.
//!
//! Avatars (draft/metadata-2 `avatar` URLs) are fetched the same way, with a
//! smaller size limit and every redirect hop checked by hand.
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
const MAX_AVATAR: usize = 1024 * 1024;
const AVATAR_CACHE: usize = 1000;
const MAX_REDIRECTS: usize = 4;
/// Direct image links can be bigger than page thumbnails.
const MAX_INLINE_IMAGE: usize = 8 * 1024 * 1024;
/// What the webview is asked to decode: no absurd sides or pixel counts.
const MAX_SIDE: usize = 16_384;
const MAX_PIXELS: usize = 50_000_000;
/// Total data-URL bytes the cache may hold, on top of its entry cap.
const CACHE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub url: String,
    /// "image" for a direct image link, "page" for an HTML page, "media"
    /// for a video or audio link (never downloaded, only described).
    pub kind: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site: Option<String>,
    /// A data: URL.
    pub image: Option<String>,
    /// Pixel size of `image`, so the view can reserve its space up front.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// `image` has more than one frame.
    pub animated: bool,
    /// Content type of a direct image, video or audio link.
    pub mime: Option<String>,
    /// Byte size of a direct link, when known.
    pub size: Option<u64>,
}

/// A fetched image whose bytes were sniffed and measured.
#[derive(Debug, Clone)]
pub struct Inline {
    pub data_url: String,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
    pub animated: bool,
}

/// A bounded map that forgets its oldest entries. Failures are cached too,
/// so a dead URL isn't fetched again every time it's shown.
struct Cache<T> {
    map: HashMap<String, Option<T>>,
    order: VecDeque<String>,
    cap: usize,
}

impl<T: Clone> Cache<T> {
    fn new(cap: usize) -> Self {
        Cache { map: HashMap::new(), order: VecDeque::new(), cap }
    }

    fn get(&self, key: &str) -> Option<Option<T>> {
        self.map.get(key).cloned()
    }

    fn insert(&mut self, key: &str, value: Option<T>) {
        if self.map.insert(key.to_string(), value).is_none() {
            self.order.push_back(key.to_string());
        }
        while self.order.len() > self.cap {
            self.evict_oldest();
        }
    }

    fn evict_oldest(&mut self) {
        if let Some(old) = self.order.pop_front() {
            self.map.remove(&old);
        }
    }
}

pub struct Previewer {
    client: reqwest::Client,
    /// Follows no redirects, so each hop can be vetted with `check_public`.
    direct: reqwest::Client,
    cache: Mutex<Cache<Arc<Preview>>>,
    avatars: Mutex<Cache<Arc<String>>>,
}

impl Previewer {
    pub fn new() -> Self {
        let policy = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 4 {
                return attempt.error("too many redirects");
            }
            if hop_allowed(attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        let build = |redirect| {
            reqwest::Client::builder()
                .redirect(redirect)
                .timeout(Duration::from_secs(8))
                .connect_timeout(Duration::from_secs(5))
                .user_agent("Mozilla/5.0 (compatible; Hearth link preview)")
                .dns_resolver(Arc::new(PublicResolver))
                .build()
                .expect("HTTP client")
        };
        Previewer {
            client: build(policy),
            direct: build(reqwest::redirect::Policy::none()),
            cache: Mutex::new(Cache::new(CACHE)),
            avatars: Mutex::new(Cache::new(AVATAR_CACHE)),
        }
    }

    pub async fn preview(&self, url: &str) -> Result<Preview> {
        if let Some(hit) = self.cache.lock().unwrap().get(url) {
            return hit.map(|p| (*p).clone()).ok_or_else(|| anyhow!("no preview"));
        }
        let result = self.fetch(url).await;
        let entry = result.as_ref().ok().map(|p| Arc::new(p.clone()));
        let mut cache = self.cache.lock().unwrap();
        cache.insert(url, entry);
        while cache.order.len() > 1 && cached_bytes(&cache.map) > CACHE_BYTES {
            cache.evict_oldest();
        }
        result
    }

    /// A user's avatar image as a data: URL.
    pub async fn avatar(&self, url: &str) -> Result<String> {
        if let Some(hit) = self.avatars.lock().unwrap().get(url) {
            return hit.map(|a| (*a).clone()).ok_or_else(|| anyhow!("no avatar"));
        }
        let result = self.fetch_avatar(url).await;
        let entry = result.as_ref().ok().map(|a| Arc::new(a.clone()));
        self.avatars.lock().unwrap().insert(url, entry);
        result
    }

    async fn fetch_avatar(&self, url: &str) -> Result<String> {
        let mut url = reqwest::Url::parse(url).context("invalid URL")?;
        for _ in 0..=MAX_REDIRECTS {
            check_public(&url).await?;
            let resp = self.direct.get(url.clone()).send().await?;
            if resp.status().is_redirection() {
                let next = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| anyhow!("redirect without a location"))?;
                url = url.join(next).context("invalid redirect")?;
                continue;
            }
            let resp = resp.error_for_status()?;
            let bytes = read_image(resp, MAX_AVATAR).await?;
            return Ok(inspect_image(&bytes)?.data_url);
        }
        Err(anyhow!("too many redirects"))
    }

    async fn fetch(&self, url: &str) -> Result<Preview> {
        let parsed = reqwest::Url::parse(url).context("invalid URL")?;
        check_public(&parsed).await?;
        let resp = self.client.get(parsed.clone()).send().await?.error_for_status()?;
        let final_url = resp.url().clone();
        let ctype = content_type(&resp);
        let mime = ctype.split(';').next().unwrap_or("").trim().to_string();
        if mime.starts_with("video/") || mime.starts_with("audio/") {
            // The CSP keeps remote media out of the webview; describe it instead.
            let size = resp.content_length();
            return Ok(Preview { kind: "media".into(), mime: Some(mime), size, ..Preview::empty(url) });
        }
        if mime.starts_with("image/") || is_generic_binary(&mime) {
            let bytes = read_image(resp, MAX_INLINE_IMAGE).await?;
            let size = bytes.len() as u64;
            let img = inspect_image(&bytes)?;
            return Ok(Preview {
                kind: "image".into(),
                image: Some(img.data_url),
                width: Some(img.width),
                height: Some(img.height),
                animated: img.animated,
                mime: Some(img.mime.into()),
                size: Some(size),
                ..Preview::empty(url)
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
                image = self.fetch_image_info(img_url, MAX_IMAGE).await.ok();
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
            width: image.as_ref().map(|i| i.width),
            height: image.as_ref().map(|i| i.height),
            animated: image.as_ref().is_some_and(|i| i.animated),
            image: image.map(|i| i.data_url),
            ..Preview::empty(url)
        })
    }

    /// Fetch an image as a data URL: raster formats only, sniffed from the bytes.
    pub async fn fetch_image(&self, url: reqwest::Url) -> Result<String> {
        self.fetch_image_info(url, MAX_IMAGE).await.map(|i| i.data_url)
    }

    pub async fn fetch_image_info(&self, url: reqwest::Url, limit: usize) -> Result<Inline> {
        check_public(&url).await?;
        let resp = self.client.get(url).send().await?.error_for_status()?;
        let ctype = content_type(&resp);
        let mime = ctype.split(';').next().unwrap_or("").trim();
        if !mime.starts_with("image/") && !is_generic_binary(mime) {
            return Err(anyhow!("not an image"));
        }
        inspect_image(&read_image(resp, limit).await?)
    }
}

impl Preview {
    fn empty(url: &str) -> Self {
        Preview {
            url: url.to_string(),
            kind: String::new(),
            title: None,
            description: None,
            site: None,
            image: None,
            width: None,
            height: None,
            animated: false,
            mime: None,
            size: None,
        }
    }
}

fn cached_bytes(map: &HashMap<String, Option<Arc<Preview>>>) -> usize {
    map.values().flatten().map(|p| p.image.as_ref().map_or(0, String::len)).sum()
}

/// Servers (soju's FILEHOST among them) often label uploads this way; the
/// bytes decide whether they're an image.
fn is_generic_binary(mime: &str) -> bool {
    matches!(mime, "" | "application/octet-stream" | "binary/octet-stream" | "application/binary")
}

/// Like `read_limited`, but gives up after the first bytes if they aren't an
/// image we'd show, so a mislabelled archive isn't downloaded for nothing.
async fn read_image(mut resp: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if resp.content_length().is_some_and(|n| n as usize > limit) {
        return Err(anyhow!("image too large"));
    }
    let mut out = Vec::new();
    let mut sniffed = false;
    while let Some(chunk) = resp.chunk().await? {
        out.extend_from_slice(&chunk);
        if out.len() > limit {
            return Err(anyhow!("image too large"));
        }
        if !sniffed && out.len() >= 32 {
            sniffed = true;
            sniff(&out)?;
        }
    }
    Ok(out)
}

fn sniff(bytes: &[u8]) -> Result<&'static str> {
    use imagesize::ImageType;
    match imagesize::image_type(bytes) {
        Ok(ImageType::Png) => Ok("image/png"),
        Ok(ImageType::Jpeg) => Ok("image/jpeg"),
        Ok(ImageType::Gif) => Ok("image/gif"),
        Ok(ImageType::Webp) => Ok("image/webp"),
        _ => Err(anyhow!("unsupported image type")),
    }
}

/// Check that the bytes really are a PNG, JPEG, GIF or WebP of sane size.
/// The declared content type is ignored: it's the bytes the webview decodes.
fn inspect_image(bytes: &[u8]) -> Result<Inline> {
    let mime = sniff(bytes)?;
    let size = imagesize::blob_size(bytes).map_err(|_| anyhow!("unreadable image"))?;
    let (w, h) = (size.width, size.height);
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || w * h > MAX_PIXELS {
        return Err(anyhow!("image dimensions out of range"));
    }
    let animated = match mime {
        "image/gif" => gif_frames(bytes) > 1,
        "image/webp" => bytes.len() > 20 && &bytes[12..16] == b"VP8X" && bytes[20] & 0x02 != 0,
        "image/png" => is_apng(bytes),
        _ => false,
    };
    Ok(Inline { data_url: image_data_url(mime, bytes.to_vec())?, mime, width: w as u32, height: h as u32, animated })
}

/// Image frames in a GIF, counting no further than two.
fn gif_frames(b: &[u8]) -> usize {
    let skip_blocks = |mut i: usize| -> Option<usize> {
        loop {
            let n = *b.get(i)? as usize;
            i += 1 + n;
            if n == 0 {
                return Some(i);
            }
        }
    };
    // Header and logical screen descriptor, then the global colour table.
    let Some(&flags) = b.get(10) else { return 0 };
    let mut i = 13;
    if flags & 0x80 != 0 {
        i += 3 << ((flags & 7) + 1);
    }
    let mut frames = 0;
    while let Some(&t) = b.get(i) {
        let next = match t {
            0x21 => skip_blocks(i + 2),
            0x2c => {
                frames += 1;
                let Some(&f) = b.get(i + 9) else { break };
                let table = if f & 0x80 != 0 { 3 << ((f & 7) + 1) } else { 0 };
                // Descriptor, local colour table, LZW code size, data.
                skip_blocks(i + 10 + table + 1)
            }
            _ => None,
        };
        match next {
            Some(n) if frames < 2 => i = n,
            _ => break,
        }
    }
    frames
}

/// An animated PNG has an `acTL` chunk before its first `IDAT`.
fn is_apng(b: &[u8]) -> bool {
    let mut i = 8;
    while i + 8 <= b.len() {
        let len = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        match &b[i + 4..i + 8] {
            b"acTL" => return true,
            b"IDAT" | b"IEND" => return false,
            _ => i += 12 + len,
        }
    }
    false
}

/// Whether a redirect target may be followed without resolving it; the
/// resolver below catches names that resolve to private addresses.
fn hop_allowed(url: &reqwest::Url) -> bool {
    let literal_ok = match url.host() {
        Some(url::Host::Ipv4(ip)) => is_public(&IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_public(&IpAddr::V6(ip)),
        Some(url::Host::Domain(d)) => !is_local_name(d),
        None => false,
    };
    matches!(url.scheme(), "http" | "https") && literal_ok
}

/// Resolves names for every connection the client makes, redirects
/// included, and refuses private addresses. `check_public` alone would let a
/// redirect to a name like `127.0.0.1.nip.io` (or a rebinding DNS answer)
/// reach the LAN.
struct PublicResolver;

impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if addrs.is_empty() || addrs.iter().any(|a| !is_public(&a.ip())) {
                return Err(format!("refusing to connect to a private address for {host}").into());
            }
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
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
    fn cache_forgets_the_oldest_entries() {
        let mut c: Cache<u32> = Cache::new(2);
        c.insert("a", Some(1));
        c.insert("b", None);
        c.insert("a", Some(2));
        assert_eq!(c.get("a"), Some(Some(2)));
        assert_eq!(c.get("b"), Some(None), "failures are remembered");
        c.insert("c", Some(3));
        assert_eq!(c.get("a"), None);
        assert_eq!(c.get("c"), Some(Some(3)));
    }

    #[tokio::test]
    async fn avatars_refuse_private_addresses() {
        let p = Previewer::new();
        for url in ["http://127.0.0.1/a.png", "http://localhost/a.png", "file:///etc/passwd", "http://10.1.2.3/x.png"] {
            assert!(p.avatar(url).await.is_err(), "{url} should be refused");
        }
    }

    #[test]
    fn clips_and_rejects_svg() {
        assert_eq!(clip("a   b\n c", 10), "a b c");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert!(image_data_url("image/svg+xml", vec![1]).is_err());
        assert!(image_data_url("image/png; charset=binary", vec![1]).unwrap().starts_with("data:image/png;base64,"));
    }

    fn png(w: u32, h: u32, extra: &[u8]) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        b.extend_from_slice(extra);
        b.extend_from_slice(b"\0\0\0\0IDAT\0\0\0\0");
        b
    }

    fn gif(frames: usize) -> Vec<u8> {
        // 2x1, global colour table of 2 entries, NETSCAPE loop extension.
        let mut b = b"GIF89a\x02\0\x01\0\x80\0\0\0\0\0\xff\xff\xff".to_vec();
        b.extend_from_slice(b"\x21\xff\x0bNETSCAPE2.0\x03\x01\0\0\0");
        for _ in 0..frames {
            b.extend_from_slice(b"\x21\xf9\x04\0\x0a\0\0\0");
            b.extend_from_slice(b"\x2c\0\0\0\0\x02\0\x01\0\0\x02\x02\x44\x01\0");
        }
        b.push(0x3b);
        b
    }

    #[test]
    fn sniffs_images_by_content() {
        let img = inspect_image(&png(640, 480, &[])).unwrap();
        assert_eq!((img.mime, img.width, img.height, img.animated), ("image/png", 640, 480, false));
        assert!(img.data_url.starts_with("data:image/png;base64,"));
        // An APNG announces its animation before the image data.
        assert!(inspect_image(&png(8, 8, b"\0\0\0\x08acTL\0\0\0\x02\0\0\0\0\0\0\0\0")).unwrap().animated);

        let still = inspect_image(&gif(1)).unwrap();
        assert_eq!((still.mime, still.width, still.height, still.animated), ("image/gif", 2, 1, false));
        assert!(inspect_image(&gif(3)).unwrap().animated);

        // Whatever the server claims, SVG and HTML are never passed on.
        assert!(inspect_image(b"<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>").is_err());
        assert!(inspect_image(b"<!doctype html><html><body>hi</body></html>").is_err());
        assert!(is_generic_binary("application/octet-stream") && !is_generic_binary("text/html"));
    }

    #[test]
    fn rejects_absurd_dimensions() {
        assert!(inspect_image(&png(0, 10, &[])).is_err());
        assert!(inspect_image(&png(20_000, 10, &[])).is_err());
        assert!(inspect_image(&png(10_000, 10_000, &[])).is_err());
        assert!(inspect_image(&png(4000, 3000, &[])).is_ok());
    }

    /// A one-shot HTTP server on loopback that answers with `head` and `body`.
    async fn serve(head: String, body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(head.as_bytes()).await;
            let _ = s.write_all(&body).await;
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn caps_image_size() {
        let big = vec![0u8; MAX_INLINE_IMAGE + 1];
        // Declared too large: refused before reading.
        let url = serve(format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\n\r\n", big.len()), vec![]).await;
        let resp = reqwest::get(&url).await.unwrap();
        assert!(read_image(resp, MAX_INLINE_IMAGE).await.is_err());
        // Undeclared: refused once the stream passes the cap.
        let mut body = png(10, 10, &[]);
        body.resize(MAX_INLINE_IMAGE + 1, 0);
        let url = serve("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nConnection: close\r\n\r\n".into(), body).await;
        let resp = reqwest::get(&url).await.unwrap();
        assert!(read_image(resp, MAX_INLINE_IMAGE).await.is_err());
        // A mislabelled non-image is dropped after its first bytes.
        let url = serve("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n".into(), vec![b'P'; 4096]).await;
        let resp = reqwest::get(&url).await.unwrap();
        assert!(read_image(resp, MAX_INLINE_IMAGE).await.unwrap_err().to_string().contains("unsupported"));
    }

    #[tokio::test]
    async fn refuses_private_hops() {
        use reqwest::dns::Resolve;
        // Redirect targets: literal private addresses and local names stop the chain.
        for url in ["http://127.0.0.1/a.png", "http://10.1.2.3/", "http://[::1]/", "http://router/", "https://nas.local/x", "file:///etc/passwd"] {
            assert!(!hop_allowed(&reqwest::Url::parse(url).unwrap()), "{url}");
        }
        assert!(hop_allowed(&reqwest::Url::parse("https://example.com/cat.gif").unwrap()));
        // Names are checked when resolved, which covers every hop of a redirect.
        let name: reqwest::dns::Name = "localhost".parse().unwrap();
        assert!(PublicResolver.resolve(name).await.is_err());
        // And the first hop is checked before anything is sent.
        let url = serve("HTTP/1.1 302 Found\r\nLocation: http://10.0.0.1/\r\n\r\n".into(), vec![]).await;
        let err = Previewer::new().preview(&url).await.unwrap_err();
        assert!(err.to_string().contains("private"), "{err}");
    }
}
