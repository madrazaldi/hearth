//! Link previews, fetched by the engine so the UI never talks to third-party
//! hosts. Images come back as data URLs.
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
/// What the UI is asked to decode: no absurd sides or pixel counts.
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
    /// Treat loopback as public. Tests serve from it; nothing sets it otherwise.
    loopback: bool,
}

impl Previewer {
    pub fn new() -> Self {
        Self::build(false)
    }

    /// A previewer that can reach 127.0.0.1, for tests that serve from there.
    #[cfg(test)]
    pub(crate) fn allowing_loopback() -> Self {
        Self::build(true)
    }

    fn build(loopback: bool) -> Self {
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            if hop_allowed(attempt.url(), loopback) {
                attempt.follow()
            } else {
                attempt.error("refusing to preview a private or local address")
            }
        });
        let build = |redirect| {
            let mut headers = reqwest::header::HeaderMap::new();
            // The bodies are handed on as they come, so they cannot be compressed.
            headers.insert(reqwest::header::ACCEPT_ENCODING, reqwest::header::HeaderValue::from_static("identity"));
            reqwest::Client::builder()
                .redirect(redirect)
                .timeout(Duration::from_secs(8))
                .connect_timeout(Duration::from_secs(5))
                .user_agent("Mozilla/5.0 (compatible; Hearth link preview)")
                .default_headers(headers)
                .dns_resolver(Arc::new(PublicResolver { loopback }))
                .build()
                .expect("HTTP client")
        };
        Previewer {
            client: build(policy),
            direct: build(reqwest::redirect::Policy::none()),
            cache: Mutex::new(Cache::new(CACHE)),
            avatars: Mutex::new(Cache::new(AVATAR_CACHE)),
            loopback,
        }
    }

    /// GET a URL, following redirects. Every hop must be public: the first is
    /// vetted here, later ones by the redirect policy and the resolver.
    async fn get(&self, url: reqwest::Url) -> Result<reqwest::Response> {
        check_public(&url, self.loopback).await?;
        let resp = self.client.get(url).send().await.map_err(flatten)?;
        refuse_compressed(&resp)?;
        error_for_status(resp)
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
        let mut url = reqwest::Url::parse(url).map_err(|_| anyhow!("invalid URL"))?;
        for _ in 0..=MAX_REDIRECTS {
            check_public(&url, self.loopback).await?;
            let resp = self.direct.get(url.clone()).send().await.map_err(flatten)?;
            if resp.status().is_redirection() {
                let next = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).ok_or_else(|| anyhow!("redirect without a location"))?;
                url = url.join(next).context("invalid redirect")?;
                continue;
            }
            refuse_compressed(&resp)?;
            let resp = error_for_status(resp)?;
            let bytes = read_image(resp, MAX_AVATAR).await?;
            return Ok(inspect_image(&bytes)?.data_url);
        }
        Err(anyhow!("too many redirects"))
    }

    async fn fetch(&self, url: &str) -> Result<Preview> {
        let parsed = reqwest::Url::parse(url).map_err(|_| anyhow!("invalid URL"))?;
        let resp = self.get(parsed).await?;
        let final_url = resp.url().clone();
        let ctype = content_type(&resp);
        let mime = ctype.split(';').next().unwrap_or("").trim().to_string();
        if mime.starts_with("video/") || mime.starts_with("audio/") {
            // The UI never loads remote media; describe it instead.
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
        let resp = self.get(url).await?;
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
    while let Some(chunk) = resp.chunk().await.map_err(flatten)? {
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
/// The declared content type is ignored: it's the bytes the UI decodes.
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
fn hop_allowed(url: &reqwest::Url, loopback: bool) -> bool {
    let literal_ok = match url.host() {
        Some(url::Host::Ipv4(ip)) => permitted(&IpAddr::V4(ip), loopback),
        Some(url::Host::Ipv6(ip)) => permitted(&IpAddr::V6(ip), loopback),
        Some(url::Host::Domain(d)) => !is_local_name(d),
        None => false,
    };
    matches!(url.scheme(), "http" | "https") && literal_ok
}

fn permitted(ip: &IpAddr, loopback: bool) -> bool {
    is_public(ip) || (loopback && ip.is_loopback())
}

/// Resolves names for every connection the client makes, redirects
/// included, and refuses private addresses. `check_public` alone would let a
/// redirect to a name like `127.0.0.1.nip.io` (or a rebinding DNS answer)
/// reach the LAN. The addresses it returns are the ones connected to, so a
/// second lookup cannot give a different answer.
struct PublicResolver {
    loopback: bool,
}

impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        let loopback = self.loopback;
        Box::pin(async move {
            let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if addrs.is_empty() || addrs.iter().any(|a| !permitted(&a.ip(), loopback)) {
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
    resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase()
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
    while let Some(chunk) = resp.chunk().await.map_err(flatten)? {
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

/// Refuse unless the URL is http(s) to a public address. What can be judged
/// from the URL alone is, before anything is resolved.
async fn check_public(url: &reqwest::Url, loopback: bool) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(anyhow!("only http and https links are previewed"));
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let (host, literal) = match url.host() {
        None => return Err(anyhow!("URL has no host")),
        Some(url::Host::Ipv4(ip)) => (ip.to_string(), Some(IpAddr::V4(ip))),
        Some(url::Host::Ipv6(ip)) => (ip.to_string(), Some(IpAddr::V6(ip))),
        Some(url::Host::Domain(d)) => (d.to_string(), None),
    };
    if let Some(ip) = literal {
        return if permitted(&ip, loopback) { Ok(()) } else { Err(anyhow!("refusing to preview a private address")) };
    }
    if is_local_name(&host) {
        return Err(anyhow!("refusing to preview a local address"));
    }
    let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), port)).await.map_err(|e| anyhow!("could not resolve host: {e}"))?.collect();
    if addrs.is_empty() || addrs.iter().any(|a| !permitted(&a.ip(), loopback)) {
        return Err(anyhow!("refusing to preview a private address"));
    }
    Ok(())
}

fn is_local_name(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    let bare = h.trim_start_matches('[').trim_end_matches(']');
    h == "localhost"
        || h.ends_with(".localhost")
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || (!h.contains('.') && bare.parse::<IpAddr>().is_err() && !h.starts_with('['))
}

/// Whether `ip` is globally routable. Addresses that embed an IPv4 one are
/// judged by it, however they are written.
pub fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4.octets()),
        IpAddr::V6(v6) => public_v6(v6.segments()),
    }
}

fn public_v4([a, b, c, _]: [u8; 4]) -> bool {
    !(a == 0
        || a == 10
        || a == 127
        || (a == 172 && (16..32).contains(&b))
        || (a == 192 && b == 168)
        || (a == 169 && b == 254)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        // multicast and reserved
        || a >= 224)
}

fn public_v6(g: [u16; 8]) -> bool {
    let quad = |hi: u16, lo: u16| [(hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8];
    let zero_prefix = g[..5].iter().all(|x| *x == 0);
    // ::, ::1, and the deprecated IPv4-compatible ::a.b.c.d
    if zero_prefix && g[5] == 0 {
        return !(g[6] == 0 && g[7] <= 1) && public_v4(quad(g[6], g[7]));
    }
    // ::ffff:a.b.c.d
    if zero_prefix && g[5] == 0xffff {
        return public_v4(quad(g[6], g[7]));
    }
    // 64:ff9b::/96 (NAT64) reaches the embedded IPv4 address
    if g[0] == 0x64 && g[1] == 0xff9b && g[2..6].iter().all(|x| *x == 0) {
        return public_v4(quad(g[6], g[7]));
    }
    // 2002::/16 (6to4) embeds one too
    if g[0] == 0x2002 {
        return public_v4(quad(g[1], g[2]));
    }
    !((g[0] & 0xfe00) == 0xfc00 // unique local
        || (g[0] & 0xffc0) == 0xfe80 // link-local
        || (g[0] & 0xffc0) == 0xfec0 // deprecated site-local
        || (g[0] & 0xff00) == 0xff00 // multicast
        || (g[0] == 0x2001 && g[1] == 0x0db8) // documentation
        || (g[0] == 0x2001 && g[1] == 0)) // Teredo
}

/// A request error with its causes on one line, since reqwest keeps the
/// useful part (a refused address, a TLS failure) in the source chain.
fn flatten(e: reqwest::Error) -> anyhow::Error {
    let mut text = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(s) = source {
        text.push_str(": ");
        text.push_str(&s.to_string());
        source = s.source();
    }
    anyhow!(text)
}

fn error_for_status(resp: reqwest::Response) -> Result<reqwest::Response> {
    resp.error_for_status().map_err(flatten)
}

/// We ask for identity encoding; a body that is compressed anyway cannot be
/// passed on as it is.
fn refuse_compressed(resp: &reqwest::Response) -> Result<()> {
    match resp.headers().get(reqwest::header::CONTENT_ENCODING).and_then(|v| v.to_str().ok()) {
        Some(enc) if !enc.trim().eq_ignore_ascii_case("identity") => Err(anyhow!("unsupported content encoding: {}", enc.trim().to_ascii_lowercase())),
        _ => Ok(()),
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

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn classifies_addresses() {
        for a in ["10.0.0.1", "127.0.0.1", "192.168.1.1", "169.254.1.1", "100.64.0.1", "::1", "fd00::1", "fe80::1", "::ffff:10.0.0.1"] {
            assert!(!is_public(&ip(a)), "{a} should be private");
        }
        for a in ["1.1.1.1", "93.184.216.34", "2606:4700::1111"] {
            assert!(is_public(&ip(a)), "{a} should be public");
        }
        assert!(is_local_name("localhost"));
        assert!(is_local_name("printer.local"));
        assert!(is_local_name("router"));
        assert!(!is_local_name("example.com"));
    }

    #[test]
    fn covers_the_rest_of_the_reserved_ranges() {
        for a in [
            "0.0.0.0",
            "0.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.1",
            "192.0.2.1",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!is_public(&ip(a)), "{a}");
        }
        for a in ["172.15.0.1", "172.32.0.1", "100.63.255.255", "100.128.0.1", "8.8.8.8"] {
            assert!(is_public(&ip(a)), "{a}");
        }
    }

    #[test]
    fn sees_through_ipv4_mapped_and_embedded_ipv6_forms() {
        for a in [
            "::",
            "::ffff:0a00:0001",
            "::ffff:7f00:1",
            "0:0:0:0:0:ffff:192.168.0.1",
            // IPv4-compatible
            "::10.0.0.1",
            // NAT64
            "64:ff9b::10.0.0.1",
            "64:ff9b::7f00:1",
            // 6to4 embedding 10.0.0.1 and 127.0.0.1
            "2002:0a00:0001::",
            "2002:7f00:1::1",
            "2001:db8::1",
            // Teredo
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "fc00::1",
            "fec0::1",
            "ff02::1",
        ] {
            assert!(!is_public(&ip(a)), "{a}");
        }
        for a in ["::ffff:8.8.8.8", "64:ff9b::808:808", "2002:0808:0808::1", "2606:4700::1111"] {
            assert!(is_public(&ip(a)), "{a}");
        }
    }

    #[test]
    fn local_names() {
        assert!(is_local_name("Printer.LOCAL."));
        assert!(is_local_name("a.localhost"));
        assert!(is_local_name("db.internal"));
        assert!(!is_local_name("[::1]"));
        assert!(!is_local_name("nas.example"));
    }

    #[tokio::test]
    async fn check_public_rejects_what_the_url_alone_gives_away() {
        let url = |s: &str| reqwest::Url::parse(s).unwrap();
        let msg = |s: &str| futures_util::FutureExt::now_or_never(check_public(&url(s), false)).expect("no lookup was needed").unwrap_err().to_string();
        assert!(msg("ftp://example.com/").contains("only http and https"));
        assert!(msg("http://localhost/").contains("local address"));
        assert!(msg("http://10.0.0.1/").contains("private"));
        assert!(msg("http://[::ffff:a00:1]/").contains("private"));
        // The loopback seam admits only loopback.
        assert!(check_public(&url("http://127.0.0.1/"), true).await.is_ok());
        assert!(check_public(&url("http://10.0.0.1/"), true).await.is_err());
        assert!(check_public(&url("http://93.184.216.34/"), false).await.is_ok());
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

    /// A loopback HTTP server that answers every request through `handler`,
    /// which gets the request line and returns the response head and body.
    struct Fixture {
        base: String,
        connections: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Fixture {
        fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }

        fn connections(&self) -> usize {
            self.connections.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    async fn fixture(handler: impl Fn(&str) -> (String, Vec<u8>) + Send + Sync + 'static) -> Fixture {
        use std::sync::atomic::Ordering;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (count, handler) = (connections.clone(), Arc::new(handler));
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else { return };
                count.fetch_add(1, Ordering::SeqCst);
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = s.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).to_string();
                    let (head, body) = handler(request.lines().next().unwrap_or(""));
                    let _ = s.write_all(format!("{head}\r\nConnection: close\r\n\r\n").as_bytes()).await;
                    let _ = s.write_all(&body).await;
                    let _ = s.shutdown().await;
                });
            }
        });
        Fixture { base, connections }
    }

    fn ok(content_type: &str, body: &[u8]) -> (String, Vec<u8>) {
        (format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}", body.len()), body.to_vec())
    }

    #[tokio::test]
    async fn refuses_private_hops() {
        use reqwest::dns::Resolve;
        // Redirect targets: literal addresses and local names stop the chain.
        for url in [
            "http://127.0.0.1/a.png",
            "http://10.1.2.3/",
            "http://[::1]/",
            "http://router/",
            "https://nas.local/x",
            "file:///etc/passwd",
            "http://[::ffff:a00:1]/",
            "http://2130706433/",
        ] {
            assert!(!hop_allowed(&reqwest::Url::parse(url).unwrap(), false), "{url}");
        }
        assert!(hop_allowed(&reqwest::Url::parse("https://example.com/cat.gif").unwrap(), false));
        assert!(hop_allowed(&reqwest::Url::parse("http://93.184.216.34/").unwrap(), false));
        // Names are checked when resolved, which covers every hop of a redirect.
        let name: reqwest::dns::Name = "localhost".parse().unwrap();
        assert!(PublicResolver { loopback: false }.resolve(name).await.is_err());
    }

    #[tokio::test]
    async fn the_first_hop_is_checked_before_anything_is_sent() {
        let f = fixture(|_| ("HTTP/1.1 302 Found\r\nLocation: http://10.0.0.1/\r\nContent-Length: 0".into(), vec![])).await;
        let err = Previewer::new().preview(&f.url("/")).await.unwrap_err();
        assert!(err.to_string().contains("private"), "{err}");
        assert_eq!(f.connections(), 0);
        // With loopback allowed, the redirect target is what stops it.
        let err = Previewer::allowing_loopback().preview(&f.url("/")).await.unwrap_err();
        assert!(err.to_string().contains("private"), "{err:#}");
    }

    #[tokio::test]
    async fn previews_a_page_with_open_graph_metadata_and_an_image() {
        let img = png(640, 480, &[]);
        let f = fixture(move |req| {
            if req.starts_with("GET /img.png") {
                ok("image/png", &img)
            } else {
                let html = r#"<html><head><title>T</title><meta property="og:title" content="  Hearth &#8212; IRC "><meta property="og:description" content="A quiet client"><meta property="og:image" content="/img.png"></head><body></body></html>"#;
                ok("text/html; charset=utf-8", html.as_bytes())
            }
        })
        .await;
        let p = Previewer::allowing_loopback();
        let url = f.url("/page");
        let pv = p.preview(&url).await.unwrap();
        assert_eq!((pv.kind.as_str(), pv.title.as_deref(), pv.description.as_deref()), ("page", Some("Hearth — IRC"), Some("A quiet client")));
        assert_eq!((pv.url.as_str(), pv.site.as_deref()), (url.as_str(), Some("127.0.0.1")));
        assert_eq!((pv.width, pv.height, pv.animated, pv.mime.as_deref(), pv.size), (Some(640), Some(480), false, None, None));
        assert!(pv.image.as_deref().unwrap().starts_with("data:image/png;base64,"));
        // The second call comes from the cache.
        let before = f.connections();
        assert_eq!(p.preview(&url).await.unwrap(), pv);
        assert_eq!(f.connections(), before);
    }

    #[tokio::test]
    async fn the_site_name_comes_from_the_page_or_the_host() {
        let f = fixture(|_| ok("text/html", br#"<head><meta property="og:site_name" content="Hearth"><meta property="og:title" content="x"></head>"#)).await;
        assert_eq!(Previewer::allowing_loopback().preview(&f.url("/")).await.unwrap().site.as_deref(), Some("Hearth"));
    }

    #[tokio::test]
    async fn previews_a_direct_image_link_labelled_or_not() {
        let img = gif(2);
        let size = img.len() as u64;
        let f = fixture(move |req| ok(if req.starts_with("GET /raw") { "application/octet-stream" } else { "image/gif" }, &img)).await;
        let p = Previewer::allowing_loopback();
        for path in ["/cat.gif", "/raw"] {
            let pv = p.preview(&f.url(path)).await.unwrap();
            assert_eq!((pv.kind.as_str(), pv.width, pv.height, pv.animated), ("image", Some(2), Some(1), true), "{path}");
            assert_eq!((pv.mime.as_deref(), pv.size, pv.title), (Some("image/gif"), Some(size), None), "{path}");
        }
    }

    #[tokio::test]
    async fn describes_media_without_downloading_it() {
        let f = fixture(|_| ("HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 18400000".into(), vec![])).await;
        let pv = Previewer::allowing_loopback().preview(&f.url("/v.mp4")).await.unwrap();
        assert_eq!((pv.kind.as_str(), pv.mime.as_deref(), pv.size, pv.image), ("media", Some("video/mp4"), Some(18_400_000), None));
    }

    #[tokio::test]
    async fn failures_are_cached_too() {
        let f = fixture(|_| ok("application/zip", b"PK")).await;
        let p = Previewer::allowing_loopback();
        let url = f.url("/");
        assert!(p.preview(&url).await.unwrap_err().to_string().contains("nothing to preview"));
        let seen = f.connections();
        assert!(p.preview(&url).await.unwrap_err().to_string().contains("no preview"));
        assert_eq!(f.connections(), seen);
    }

    #[tokio::test]
    async fn a_page_with_nothing_to_show_and_a_missing_page_are_errors() {
        let f = fixture(|req| {
            if req.starts_with("GET /empty") {
                ok("text/html", b"<html></html>")
            } else {
                ("HTTP/1.1 404 Not Found\r\nContent-Length: 0".into(), vec![])
            }
        })
        .await;
        let p = Previewer::allowing_loopback();
        assert!(p.preview(&f.url("/empty")).await.unwrap_err().to_string().contains("nothing to preview"));
        assert!(p.preview(&f.url("/missing")).await.unwrap_err().to_string().contains("404"));
        assert!(p.preview("not a url").await.unwrap_err().to_string().contains("invalid URL"));
    }

    #[tokio::test]
    async fn a_blank_first_metadata_key_hides_the_later_ones_as_before() {
        let html = br#"<head><meta property="og:title" content=" "><meta name="twitter:title" content="Other"><title>Fallback</title></head>"#;
        let f = fixture(move |_| ok("text/html", html)).await;
        // Nothing else to show, so the blank og:title makes the whole preview fail.
        assert!(Previewer::allowing_loopback().preview(&f.url("/")).await.unwrap_err().to_string().contains("nothing to preview"));
    }

    #[tokio::test]
    async fn compressed_bodies_are_refused() {
        let f = fixture(|_| ("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Encoding: gzip\r\nContent-Length: 2".into(), vec![0x1f, 0x8b])).await;
        let err = Previewer::allowing_loopback().preview(&f.url("/")).await.unwrap_err();
        assert!(err.to_string().contains("content encoding"), "{err}");
    }

    #[tokio::test]
    async fn redirects_are_followed_and_bounded() {
        let f = fixture(|req| {
            if req.starts_with("GET /loop") {
                ("HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0".into(), vec![])
            } else if req.starts_with("GET /final") {
                ok("text/html", b"<title>Arrived</title>")
            } else {
                ("HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0".into(), vec![])
            }
        })
        .await;
        let p = Previewer::allowing_loopback();
        assert_eq!(p.preview(&f.url("/start")).await.unwrap().title.as_deref(), Some("Arrived"));
        assert!(p.preview(&f.url("/loop")).await.unwrap_err().to_string().contains("redirect"));
    }

    #[tokio::test]
    async fn avatars_are_data_urls_and_non_images_are_rejected() {
        let f = fixture(|req| if req.starts_with("GET /ok") { ok("image/png", &png(40, 40, &[])) } else { ok("image/png", b"<svg/>") }).await;
        let p = Previewer::allowing_loopback();
        assert!(p.avatar(&f.url("/ok")).await.unwrap().starts_with("data:image/png;base64,"));
        assert!(p.avatar(&f.url("/bad")).await.is_err());
        assert!(p.avatar(&f.url("/bad")).await.unwrap_err().to_string().contains("no avatar"));
    }

    #[tokio::test]
    async fn avatar_redirects_are_vetted_hop_by_hop() {
        let f = fixture(|_| ("HTTP/1.1 302 Found\r\nLocation: http://10.0.0.1/a.png\r\nContent-Length: 0".into(), vec![])).await;
        let err = Previewer::allowing_loopback().avatar(&f.url("/a.png")).await.unwrap_err();
        assert!(err.to_string().contains("private"), "{err}");
    }

    #[tokio::test]
    async fn interim_responses_are_skipped() {
        let f = fixture(|_| {
            let (head, body) = ok("text/html", b"<title>After the 100</title>");
            (format!("HTTP/1.1 100 Continue\r\n\r\n{head}"), body)
        })
        .await;
        let pv = Previewer::allowing_loopback().preview(&f.url("/")).await.unwrap();
        assert_eq!(pv.title.as_deref(), Some("After the 100"));
    }

    #[tokio::test]
    async fn oversized_headers_are_refused() {
        let f = fixture(|_| {
            let (head, body) = ok("text/html", b"<title>x</title>");
            (format!("{head}\r\nX-Padding: {}", "a".repeat(2 * 1024 * 1024)), body)
        })
        .await;
        assert!(Previewer::allowing_loopback().preview(&f.url("/")).await.is_err());
    }

    #[tokio::test]
    async fn chunked_bodies_with_trailers_are_decoded() {
        let f = fixture(|_| {
            let html = b"<title>Chunked</title>";
            let mut body = format!("{:x};ext=1\r\n", html.len()).into_bytes();
            body.extend_from_slice(html);
            body.extend_from_slice(b"\r\n0\r\nX-Trailer: yes\r\n\r\n");
            ("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked".into(), body)
        })
        .await;
        assert_eq!(Previewer::allowing_loopback().preview(&f.url("/")).await.unwrap().title.as_deref(), Some("Chunked"));
    }

    #[tokio::test]
    async fn a_body_cut_short_is_an_error_not_a_short_read() {
        let f = fixture(|_| ("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 5000".into(), png(10, 10, &[]))).await;
        assert!(Previewer::allowing_loopback().preview(&f.url("/")).await.is_err());
    }
}
