//! Hearth's design tokens (`src/app.css`) mapped onto gpui-component's theme.
//!
//! The CSS tokens are written in OKLCH, so the palette is too; `oklch` converts
//! at startup. gpui-component gets two registered themes (light and dark) for
//! the colours it draws itself (buttons, menus, scrollbars, inputs), while
//! `Palette` carries the Hearth-only tokens (brand, mention tint, IRC colours,
//! nick lightness/chroma) that the views read directly.

use super::prelude::*;
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode, ThemeRegistry};
use hearth_model::format::irc_palette;
use std::rc::Rc;

/// A stable hue (degrees) for a nick, from the model so every client agrees.
pub fn nick_hue(nick: &str) -> f32 {
    hearth_model::nickcolor::nick_hue(nick) as f32
}

pub const SANS: &str = "Geist";
pub const MONO: &str = "Geist Mono";

/// OKLCH to sRGB, clipped to gamut. `l` is 0..1, `h` in degrees.
pub fn oklch(l: f32, c: f32, h: f32, a: f32) -> Hsla {
    let hr = h.to_radians();
    let (la, lb) = (c * hr.cos(), c * hr.sin());
    let l_ = l + 0.396_337_78 * la + 0.215_803_76 * lb;
    let m_ = l - 0.105_561_346 * la - 0.063_854_17 * lb;
    let s_ = l - 0.089_484_18 * la - 1.291_485_5 * lb;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3;
    let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_4 * s3;
    let b = -0.004_196_086 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
    let enc = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
    };
    Rgba { r: enc(r), g: enc(g), b: enc(b), a }.into()
}

fn hex(c: Hsla) -> String {
    let c: Rgba = c.into();
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b), b(c.a))
}

fn rgb(hex: u32) -> Hsla {
    rgba((hex << 8) | 0xff).into()
}

/// The resolved tokens for one appearance.
#[derive(Clone)]
pub struct Palette {
    pub dark: bool,
    pub background: Hsla,
    pub foreground: Hsla,
    pub card: Hsla,
    pub popover: Hsla,
    pub primary: Hsla,
    pub primary_foreground: Hsla,
    pub secondary: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub accent: Hsla,
    pub destructive: Hsla,
    pub border: Hsla,
    pub input: Hsla,
    pub ring: Hsla,
    pub sidebar: Hsla,
    pub sidebar_foreground: Hsla,
    pub sidebar_accent: Hsla,
    pub sidebar_border: Hsla,
    pub brand: Hsla,
    pub brand_text: Hsla,
    pub brand_soft: Hsla,
    pub mention: Hsla,
    pub ok: Hsla,
    pub warn: Hsla,
    pub text_2: Hsla,
    pub bg_sunken: Hsla,
    nick_l: f32,
    nick_c: f32,
}

impl Palette {
    pub fn dark() -> Palette {
        let primary = oklch(0.735, 0.155, 50.0, 1.0);
        let white = |a| oklch(1.0, 0.0, 0.0, a);
        Palette {
            dark: true,
            background: oklch(0.183, 0.005, 56.0, 1.0),
            foreground: oklch(0.968, 0.003, 80.0, 1.0),
            card: oklch(0.212, 0.006, 56.0, 1.0),
            popover: oklch(0.222, 0.006, 56.0, 1.0),
            primary,
            primary_foreground: oklch(0.2, 0.035, 45.0, 1.0),
            secondary: oklch(0.262, 0.007, 56.0, 1.0),
            muted: oklch(0.252, 0.006, 56.0, 1.0),
            muted_foreground: oklch(0.685, 0.012, 62.0, 1.0),
            accent: oklch(0.268, 0.008, 56.0, 1.0),
            destructive: oklch(0.69, 0.19, 24.0, 1.0),
            border: white(0.08),
            input: white(0.12),
            ring: primary.opacity(0.5),
            sidebar: oklch(0.148, 0.004, 52.0, 1.0),
            sidebar_foreground: oklch(0.93, 0.004, 80.0, 1.0),
            sidebar_accent: oklch(0.225, 0.007, 56.0, 1.0),
            sidebar_border: white(0.07),
            brand: primary,
            brand_text: oklch(0.2, 0.035, 45.0, 1.0),
            brand_soft: primary.opacity(0.15),
            mention: primary.opacity(0.08),
            ok: oklch(0.76, 0.14, 150.0, 1.0),
            warn: oklch(0.82, 0.13, 82.0, 1.0),
            text_2: oklch(0.86, 0.006, 62.0, 1.0),
            bg_sunken: oklch(0.16, 0.004, 52.0, 1.0),
            nick_l: 0.8,
            nick_c: 0.095,
        }
    }

    pub fn light() -> Palette {
        let primary = oklch(0.6, 0.17, 45.0, 1.0);
        Palette {
            dark: false,
            background: oklch(1.0, 0.0, 0.0, 1.0),
            foreground: oklch(0.2, 0.006, 50.0, 1.0),
            card: oklch(1.0, 0.0, 0.0, 1.0),
            popover: oklch(1.0, 0.0, 0.0, 1.0),
            primary,
            primary_foreground: oklch(0.99, 0.002, 80.0, 1.0),
            secondary: oklch(0.962, 0.004, 75.0, 1.0),
            muted: oklch(0.962, 0.004, 75.0, 1.0),
            muted_foreground: oklch(0.5, 0.013, 58.0, 1.0),
            accent: oklch(0.952, 0.006, 70.0, 1.0),
            destructive: oklch(0.577, 0.215, 27.0, 1.0),
            border: oklch(0.915, 0.005, 70.0, 1.0),
            input: oklch(0.885, 0.006, 70.0, 1.0),
            ring: primary.opacity(0.45),
            sidebar: oklch(0.962, 0.005, 75.0, 1.0),
            sidebar_foreground: oklch(0.25, 0.008, 50.0, 1.0),
            sidebar_accent: oklch(0.918, 0.008, 70.0, 1.0),
            sidebar_border: oklch(0.9, 0.006, 70.0, 1.0),
            brand: primary,
            brand_text: oklch(0.99, 0.0, 0.0, 1.0),
            brand_soft: primary.opacity(0.12),
            mention: primary.opacity(0.06),
            ok: oklch(0.58, 0.14, 150.0, 1.0),
            warn: oklch(0.62, 0.13, 75.0, 1.0),
            text_2: oklch(0.37, 0.01, 60.0, 1.0),
            bg_sunken: oklch(0.975, 0.003, 75.0, 1.0),
            nick_l: 0.52,
            nick_c: 0.14,
        }
    }

    /// Nick text colour for a hue; lightness and chroma depend on the appearance.
    pub fn nick(&self, hue: f32) -> Hsla {
        oklch(self.nick_l, self.nick_c, hue, 1.0)
    }

    /// An avatar/tile tint: the nick colour mixed 22-24% into `base`.
    pub fn tile(&self, hue: f32, base: Hsla, amount: f32) -> Hsla {
        let n = self.nick(hue);
        let (n, b): (Rgba, Rgba) = (n.into(), base.into());
        let mix = |a: f32, b: f32| a * amount + b * (1.0 - amount);
        Rgba { r: mix(n.r, b.r), g: mix(n.g, b.g), b: mix(n.b, b.b), a: 1.0 }.into()
    }

    /// `--bg-hover`: the accent colour at 55%.
    pub fn hover(&self) -> Hsla {
        self.accent.opacity(0.55)
    }

    /// A colour from a formatting code, resolved against this appearance.
    pub fn irc_color(&self, c: hearth_model::format::Color) -> Hsla {
        use hearth_model::format::Color;
        match c {
            Color::Palette(n) => rgb(irc_palette(self.dark)[(n as usize).min(15)]),
            Color::Rgb(v) => rgb(v),
            Color::Foreground => self.foreground,
            Color::Background => self.background,
        }
    }

    /// The dim behind a modal. The old app blurred the window behind its overlays; GPUI has no
    /// backdrop filter, so this flattens contrast the way a blur does instead: it is a veil in the
    /// window's own colour over most of the content, not a plain black wash that leaves text crisp.
    fn scrim(&self) -> Hsla {
        if self.dark { rgba(0x0e_0c_0b_b3).into() } else { rgba(0xcc_c9_c6_a6).into() }
    }

    fn theme_json(&self) -> serde_json::Value {
        let name = if self.dark { "Hearth Dark" } else { "Hearth Light" };
        let primary_hover = self.primary.opacity(0.9);
        let colors: Vec<(&str, Hsla)> = vec![
            ("background", self.background),
            ("foreground", self.foreground),
            ("border", self.border),
            ("input.border", self.input),
            ("ring", self.ring),
            ("caret", self.foreground),
            ("accent.background", self.accent),
            ("accent.foreground", self.foreground),
            ("muted.background", self.muted),
            ("muted.foreground", self.muted_foreground),
            ("popover.background", self.popover),
            ("popover.foreground", self.foreground),
            ("primary.background", self.primary),
            ("primary.hover.background", primary_hover),
            ("primary.active.background", primary_hover),
            ("primary.foreground", self.primary_foreground),
            ("secondary.background", self.secondary),
            ("secondary.hover.background", self.accent),
            ("secondary.active.background", self.accent),
            ("secondary.foreground", self.foreground),
            ("danger.background", self.destructive),
            ("danger.foreground", self.background),
            ("warning.background", self.warn),
            ("success.background", self.ok),
            ("link.foreground", self.brand),
            ("link.hover.foreground", self.brand),
            ("link.active.foreground", self.brand),
            ("list.background", self.background),
            ("list.hover.background", self.hover()),
            ("list.active.background", self.accent),
            ("list.active.border", self.brand),
            ("list.even.background", self.background),
            ("list.head.background", self.background),
            ("scrollbar.background", self.background.opacity(0.0)),
            ("scrollbar.thumb.background", self.input),
            ("scrollbar.thumb.hover.background", self.muted_foreground.opacity(0.5)),
            ("selection.background", self.brand.opacity(0.3)),
            ("sidebar.background", self.sidebar),
            ("sidebar.foreground", self.sidebar_foreground),
            ("sidebar.accent.background", self.sidebar_accent),
            ("sidebar.accent.foreground", self.foreground),
            ("sidebar.border", self.sidebar_border),
            ("sidebar.primary.background", self.primary),
            ("sidebar.primary.foreground", self.primary_foreground),
            ("title_bar.background", self.sidebar),
            ("title_bar.border", self.sidebar_border),
            ("tab_bar.background", self.muted),
            ("tab.active.background", self.background),
            ("skeleton.background", self.muted),
            ("switch.background", self.input),
            ("overlay", self.scrim()),
        ];
        let colors: serde_json::Map<String, serde_json::Value> = colors.into_iter().map(|(k, v)| (k.to_string(), hex(v).into())).collect();
        let mut theme = serde_json::Map::new();
        theme.insert("name".into(), name.into());
        theme.insert("mode".into(), if self.dark { "dark" } else { "light" }.into());
        theme.insert("font.family".into(), SANS.into());
        theme.insert("font.size".into(), 14.into());
        theme.insert("mono_font.family".into(), MONO.into());
        theme.insert("mono_font.size".into(), 12.into());
        theme.insert("radius".into(), 9.into());
        theme.insert("radius.lg".into(), 13.into());
        theme.insert("shadow".into(), false.into());
        theme.insert("colors".into(), colors.into());
        theme.into()
    }
}

struct Palettes {
    light: Palette,
    dark: Palette,
}

impl Global for Palettes {}

/// What the user picked in settings; `System` follows the OS.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemePref {
    System,
    Light,
    Dark,
}

impl From<hearth_model::types::Theme> for ThemePref {
    fn from(t: hearth_model::types::Theme) -> Self {
        match t {
            hearth_model::types::Theme::System => ThemePref::System,
            hearth_model::types::Theme::Light => ThemePref::Light,
            hearth_model::types::Theme::Dark => ThemePref::Dark,
        }
    }
}

/// Registers the Hearth themes with gpui-component. Call after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    let (light, dark) = (Palette::light(), Palette::dark());
    let set = serde_json::json!({ "name": "Hearth", "themes": [light.theme_json(), dark.theme_json()] });
    let registry = ThemeRegistry::global_mut(cx);
    registry.load_themes_from_str(&set.to_string()).expect("hearth theme is valid");
    let themes = registry.themes();
    let (l, d) = (themes["Hearth Light"].clone(), themes["Hearth Dark"].clone());
    cx.set_global(Palettes { light, dark });
    Theme::update(cx, |theme| {
        theme.light_theme = Rc::clone(&l);
        theme.dark_theme = Rc::clone(&d);
    });
}

/// Switches appearance. `System` resolves against the window's appearance.
pub fn apply(pref: ThemePref, window: Option<&mut Window>, cx: &mut App) {
    match pref {
        ThemePref::System => Theme::sync_system_appearance(window, cx),
        ThemePref::Light => Theme::change(ThemeMode::Light, window, cx),
        ThemePref::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

/// `cx.hearth()`: the Hearth tokens for the active appearance.
pub trait HearthTheme {
    fn hearth(&self) -> &Palette;
}

impl HearthTheme for App {
    fn hearth(&self) -> &Palette {
        let p = self.global::<Palettes>();
        if self.theme().is_dark() { &p.dark } else { &p.light }
    }
}
