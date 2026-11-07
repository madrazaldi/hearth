//! Stable per-nick colours computed in OKLCH, resolved to sRGB for the UI.

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static HUES: RefCell<HashMap<String, u16>> = RefCell::new(HashMap::new());
}

/// A stable hue (degrees) for a nick; lightness and chroma come from the theme.
pub fn nick_hue(nick: &str) -> u16 {
    let key = nick.to_lowercase();
    let key = key.trim_end_matches(['_', '`', '|', '^']);
    if let Some(h) = HUES.with(|c| c.borrow().get(key).copied()) {
        return h;
    }
    // FNV-1a over UTF-16 units, as the TypeScript client did, so colours carry over.
    let mut hash: u32 = 2166136261;
    for unit in key.encode_utf16() {
        hash ^= unit as u32;
        hash = hash.wrapping_mul(16777619);
    }
    let h = (hash % 360) as u16;
    HUES.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 5000 {
            c.clear();
        }
        c.insert(key.to_string(), h);
    });
    h
}

/// Lightness and chroma the nick colours use, per theme (from app.css).
pub fn nick_lc(dark: bool) -> (f32, f32) {
    if dark {
        (0.8, 0.095)
    } else {
        (0.52, 0.14)
    }
}

/// The nick's colour as 0xRRGGBB.
pub fn nick_color(nick: &str, dark: bool) -> u32 {
    let (l, c) = nick_lc(dark);
    oklch(l, c, nick_hue(nick) as f32)
}

fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// OKLCH to linear sRGB, unclamped.
fn oklch_linear(l: f32, c: f32, h_deg: f32) -> [f32; 3] {
    let h = h_deg.to_radians();
    let (a, b) = (c * h.cos(), c * h.sin());
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_93 * s3,
        -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_4 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3,
    ]
}

fn in_gamut(rgb: [f32; 3]) -> bool {
    rgb.iter().all(|v| (-0.0005..=1.0005).contains(v))
}

/// OKLCH to 0xRRGGBB. Out-of-gamut colours lose chroma until they fit, as CSS does.
pub fn oklch(l: f32, c: f32, h_deg: f32) -> u32 {
    let mut rgb = oklch_linear(l, c, h_deg);
    if !in_gamut(rgb) {
        let (mut lo, mut hi) = (0.0, c);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if in_gamut(oklch_linear(l, mid, h_deg)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        rgb = oklch_linear(l, lo, h_deg);
    }
    let byte = |v: f32| (linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u32;
    (byte(rgb[0]) << 16) | (byte(rgb[1]) << 8) | byte(rgb[2])
}

/// Mix two 0xRRGGBB colours, `amount` (0..1) of `a` into `b`, in sRGB.
pub fn mix(a: u32, b: u32, amount: f32) -> u32 {
    let ch = |shift: u32| {
        let (x, y) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        (x * amount + y * (1.0 - amount)).round() as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hues_are_stable_and_ignore_case_and_trailing_decorations() {
        assert_eq!(nick_hue("Ann"), nick_hue("ann"));
        assert_eq!(nick_hue("ann_"), nick_hue("ann"));
        assert_eq!(nick_hue("ann|^"), nick_hue("ann"));
        assert!(nick_hue("bob") < 360);
        // FNV-1a of "ann" computed by hand: pins the hash against the TypeScript one.
        let mut h: u32 = 2166136261;
        for b in b"ann" {
            h = (h ^ *b as u32).wrapping_mul(16777619);
        }
        assert_eq!(nick_hue("ann") as u32, h % 360);
    }

    #[test]
    fn oklch_resolves_to_srgb() {
        assert_eq!(oklch(1.0, 0.0, 0.0), 0xffffff);
        assert_eq!(oklch(0.0, 0.0, 0.0), 0x000000);
        // Mid-grey: oklch(0.5998 0 0) is sRGB 50%.
        let grey = oklch(0.5998, 0.0, 0.0);
        assert!((0x7e..=0x81).contains(&(grey & 0xff)), "{grey:x}");
        // A saturated colour pulled into gamut still gets a sensible, non-black result.
        assert_ne!(nick_color("ann", true), 0);
        assert_eq!(mix(0xffffff, 0x000000, 0.5), 0x808080);
    }
}
