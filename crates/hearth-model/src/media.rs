//! Pure helpers for inline images and media cards.

use crate::format::strip_formatting;
use hearth_core::preview::Preview;
use regex::Regex;
use std::collections::VecDeque;
use std::sync::OnceLock;

pub const THUMB_W: u32 = 360;
pub const THUMB_H: u32 = 280;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxSize {
    pub width: u32,
    pub height: u32,
}

/// Scale `w`×`h` down (never up) to fit inside `max_w`×`max_h`, keeping the aspect ratio.
pub fn fit_box_in(w: u32, h: u32, max_w: u32, max_h: u32) -> BoxSize {
    if w == 0 || h == 0 {
        return BoxSize { width: max_w, height: max_h };
    }
    let k = f64::min(1.0, f64::min(max_w as f64 / w as f64, max_h as f64 / h as f64));
    BoxSize { width: ((w as f64 * k).round() as u32).max(1), height: ((h as f64 * k).round() as u32).max(1) }
}

pub fn fit_box(w: u32, h: u32) -> BoxSize {
    fit_box_in(w, h, THUMB_W, THUMB_H)
}

/// Messages that mention NSFW/NSFL get their images blurred until clicked.
pub fn is_nsfw(text: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(^|[^a-z0-9])nsf[wl]([^a-z0-9]|$)").unwrap()).is_match(&strip_formatting(text))
}

pub fn format_bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let units = ["KB", "MB", "GB"];
    let mut v = n as f64 / 1024.0;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v < 10.0 {
        format!("{v:.1} {}", units[i])
    } else {
        format!("{} {}", v.round() as u64, units[i])
    }
}

/// The last path segment of a URL, decoded, for naming a file card.
pub fn file_name(url: &str) -> String {
    let Ok(u) = url::Url::parse(url) else { return url.to_string() };
    match u.path_segments().and_then(|mut s| s.rfind(|p| !p.is_empty())) {
        Some(last) => percent_encoding::percent_decode_str(last).decode_utf8_lossy().into_owned(),
        None => u.host_str().unwrap_or(url).to_string(),
    }
}

/// "Video · MP4 · 12 MB"
pub fn media_label(p: &Preview) -> String {
    let mime = p.mime.as_deref().unwrap_or("");
    let (kind_part, sub) = mime.split_once('/').unwrap_or((mime, ""));
    let kind = match kind_part {
        "video" => "Video",
        "audio" => "Audio",
        "image" => "Image",
        _ => "File",
    };
    let mut parts = vec![kind.to_string()];
    let sub = sub.strip_prefix("x-").unwrap_or(sub);
    let sub = sub.strip_prefix("vnd.").or_else(|| sub.strip_prefix("x-")).unwrap_or(sub);
    let fmt = sub.split(['+', ';', '.']).next().unwrap_or("");
    if !fmt.is_empty() {
        parts.push(if fmt == "mpeg" && kind_part == "audio" { "MP3".into() } else { fmt.to_uppercase() });
    }
    if let (Some(w), Some(h)) = (p.width, p.height) {
        if w > 0 && h > 0 {
            parts.push(format!("{w}×{h}"));
        }
    }
    if let Some(size) = p.size.filter(|s| *s > 0) {
        parts.push(format_bytes(size));
    }
    parts.join(" · ")
}

/// A map that keeps its most recently set entries within an entry and a byte
/// budget, so cached data URLs can't pile up.
pub struct ByteLru<V> {
    order: VecDeque<String>,
    map: std::collections::HashMap<String, (V, usize)>,
    max_entries: usize,
    max_bytes: usize,
    total: usize,
}

impl<V> ByteLru<V> {
    pub fn new(max_entries: usize, max_bytes: usize) -> Self {
        ByteLru { order: VecDeque::new(), map: Default::default(), max_entries, max_bytes, total: 0 }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.total
    }

    pub fn has(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.map.get(key).map(|(v, _)| v)
    }

    pub fn set(&mut self, key: &str, value: V, bytes: usize) {
        self.delete(key);
        self.map.insert(key.to_string(), (value, bytes));
        self.order.push_back(key.to_string());
        self.total += bytes;
        while self.map.len() > 1 && (self.map.len() > self.max_entries || self.total > self.max_bytes) {
            let Some(oldest) = self.order.front().cloned() else { break };
            self.delete(&oldest);
        }
    }

    pub fn delete(&mut self, key: &str) {
        if let Some((_, bytes)) = self.map.remove(key) {
            self.total -= bytes;
            self.order.retain(|k| k != key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(kind: &str, mime: Option<&str>, size: Option<u64>, w: Option<u32>, h: Option<u32>) -> Preview {
        Preview {
            url: String::new(),
            kind: kind.into(),
            title: None,
            description: None,
            site: None,
            image: None,
            width: w,
            height: h,
            animated: false,
            mime: mime.map(String::from),
            size,
        }
    }

    #[test]
    fn fits_images_inside_the_thumbnail_box_without_upscaling() {
        assert_eq!(fit_box(1920, 1080), BoxSize { width: 360, height: 203 });
        assert_eq!(fit_box(600, 1200), BoxSize { width: 140, height: 280 });
        assert_eq!(fit_box(100, 50), BoxSize { width: 100, height: 50 });
        assert_eq!(fit_box(10000, 1), BoxSize { width: 360, height: 1 });
        assert_eq!(fit_box(0, 0), BoxSize { width: 360, height: 280 });
    }

    #[test]
    fn spots_nsfw_markers_as_words() {
        assert!(is_nsfw("NSFW: https://x.example/a.png"));
        assert!(is_nsfw("(nsfl) look"));
        assert!(is_nsfw("\x02nsfw\x02 pic"));
        assert!(!is_nsfw("https://x.example/unsfwx.png"));
        assert!(!is_nsfw("safe for work"));
    }

    #[test]
    fn describes_files() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(12 * 1024 * 1024), "12 MB");
        assert_eq!(media_label(&preview("media", Some("video/mp4"), Some(3 * 1024 * 1024), None, None)), "Video · MP4 · 3.0 MB");
        assert_eq!(media_label(&preview("media", Some("audio/mpeg"), None, None, None)), "Audio · MP3");
        assert_eq!(media_label(&preview("image", Some("image/png"), Some(2048), Some(800), Some(600))), "Image · PNG · 800×600 · 2.0 KB");
        assert_eq!(file_name("https://up.example/uploads/abc/My%20clip.webm?x=1"), "My clip.webm");
        assert_eq!(file_name("https://up.example/"), "up.example");
    }

    #[test]
    fn bounds_a_cache_by_entries_and_bytes() {
        let mut c = ByteLru::new(3, 100);
        c.set("a", "a", 40);
        c.set("b", "b", 40);
        c.set("c", "c", 40);
        assert!(!c.has("a"));
        assert_eq!(c.bytes(), 80);
        c.set("d", "d", 1);
        c.set("e", "e", 1);
        assert_eq!([c.has("b"), c.has("c"), c.has("d"), c.has("e")], [false, true, true, true]);
        // An entry bigger than the budget still stays, alone.
        c.set("huge", "h", 500);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("huge"), Some(&"h"));
        c.set("huge", "h2", 10);
        assert_eq!(c.bytes(), 10);
    }
}
