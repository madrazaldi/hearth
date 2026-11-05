//! Pictures the engine hands over as `data:` URLs (avatars, link previews),
//! decoded once into images GPUI can paint.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use gpui_kit::{Image, ImageFormat, ImageSource};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// Enough for a screenful of avatars and previews with room to scroll back.
const CAPACITY: usize = 400;

/// Something a view found missing while painting, to be requested from the model.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ask {
    Avatar(String),
    Preview(String),
}

#[derive(Default)]
pub struct Images {
    inner: RefCell<Inner>,
    /// Only what is waiting for the next flush: once requested, the model tracks
    /// loading and failures, and asks again after its cache drops an entry.
    queue: RefCell<Vec<Ask>>,
}

#[derive(Default)]
struct Inner {
    map: HashMap<String, Option<Arc<Image>>>,
    order: VecDeque<String>,
}

impl Images {
    /// Note that `url` is wanted. True when it was not queued yet, when the caller should flush the queue.
    pub fn ask(&self, ask: Ask) -> bool {
        let mut queue = self.queue.borrow_mut();
        if queue.contains(&ask) {
            return false;
        }
        queue.push(ask);
        true
    }

    /// Whether anything is waiting to be requested.
    pub fn pending(&self) -> bool {
        !self.queue.borrow().is_empty()
    }

    pub fn take_asks(&self) -> Vec<Ask> {
        std::mem::take(&mut *self.queue.borrow_mut())
    }

    /// The resting first frame of the animation at `data_url`, remembered under `key`.
    pub fn still(&self, key: &str, data_url: &str) -> Option<ImageSource> {
        let key = format!("still:{key}");
        let mut inner = self.inner.borrow_mut();
        if let Some(found) = inner.map.get(&key) {
            return found.clone().map(ImageSource::Image);
        }
        let frame = still_frame(data_url).map(Arc::new);
        if inner.order.len() >= CAPACITY
            && let Some(old) = inner.order.pop_front()
        {
            inner.map.remove(&old);
        }
        inner.order.push_back(key.clone());
        inner.map.insert(key, frame.clone());
        frame.map(ImageSource::Image)
    }

    /// The image for `data_url`, remembered under `key` (the request's URL, which
    /// is cheap to hash, unlike the data). A URL that does not decode is remembered too.
    pub fn get(&self, key: &str, data_url: &str) -> Option<ImageSource> {
        let mut inner = self.inner.borrow_mut();
        if let Some(found) = inner.map.get(key) {
            return found.clone().map(ImageSource::Image);
        }
        let decoded = decode(data_url).map(Arc::new);
        if inner.order.len() >= CAPACITY
            && let Some(old) = inner.order.pop_front()
        {
            inner.map.remove(&old);
        }
        inner.order.push_back(key.to_string());
        inner.map.insert(key.to_string(), decoded.clone());
        decoded.map(ImageSource::Image)
    }
}

/// The first frame of an animated image as a PNG, so an animation can rest until it is hovered.
pub fn still_frame(data_url: &str) -> Option<Image> {
    let image = decode(data_url)?;
    let format = match image.format {
        ImageFormat::Gif => image::ImageFormat::Gif,
        ImageFormat::Webp => image::ImageFormat::WebP,
        ImageFormat::Png => image::ImageFormat::Png,
        _ => return None,
    };
    let first = image::load_from_memory_with_format(&image.bytes, format).ok()?;
    let mut png = std::io::Cursor::new(Vec::new());
    first.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(Image::from_bytes(ImageFormat::Png, png.into_inner()))
}

/// `data:image/png;base64,...` into an image. Anything but base64 is refused.
pub fn decode(data_url: &str) -> Option<Image> {
    let rest = data_url.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    let format = match mime {
        "image/png" => ImageFormat::Png,
        "image/jpeg" | "image/jpg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        "image/webp" => ImageFormat::Webp,
        "image/bmp" => ImageFormat::Bmp,
        "image/svg+xml" => ImageFormat::Svg,
        _ => return None,
    };
    let bytes = STANDARD.decode(payload.trim()).ok()?;
    Some(Image::from_bytes(format, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_base64_png() {
        // A 1x1 PNG.
        let url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        assert!(decode(url).is_some());
        assert!(decode("data:text/plain;base64,aGk=").is_none());
        assert!(decode("data:image/png,raw").is_none());
        assert!(decode("https://example.com/a.png").is_none());
    }

    /// A two-frame GIF, red then blue, 2x2.
    fn gif_data_url() -> String {
        use image::{Delay, Frame, RgbaImage, codecs::gif::GifEncoder};
        let mut bytes = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut bytes);
            for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                let frame = Frame::from_parts(RgbaImage::from_pixel(2, 2, image::Rgba(color)), 0, 0, Delay::from_numer_denom_ms(100, 1));
                enc.encode_frame(frame).unwrap();
            }
        }
        format!("data:image/gif;base64,{}", STANDARD.encode(bytes))
    }

    #[test]
    fn an_animation_rests_on_its_first_frame() {
        let url = gif_data_url();
        let still = still_frame(&url).expect("decodes the first frame");
        assert_eq!(still.format, ImageFormat::Png, "re-encoded as a still picture");
        let px = image::load_from_memory(&still.bytes).unwrap().to_rgba8();
        assert_eq!(px.get_pixel(0, 0).0[..3], [255, 0, 0], "the first frame, not the last");
        let images = Images::default();
        assert!(images.still("k", &url).is_some());
        assert!(images.get("k", &url).is_some(), "the animation itself is kept apart from its still");
    }

    #[test]
    fn a_picture_can_be_asked_for_again_after_the_queue_was_flushed() {
        // The model's cache may drop it later; it decides whether to fetch again.
        let images = Images::default();
        let ask = Ask::Preview("https://a.example/".into());
        assert!(images.ask(ask.clone()));
        assert!(!images.ask(ask.clone()), "queued once per flush");
        assert_eq!(images.take_asks(), std::slice::from_ref(&ask));
        assert!(images.ask(ask));
    }

    #[test]
    fn remembers_by_key() {
        let images = Images::default();
        assert!(images.get("k", "data:image/png;base64,!!!").is_none());
        // The answer is cached, even though the data differs now.
        let url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
        assert!(images.get("k", url).is_none());
        assert!(images.get("other", url).is_some());
    }
}
