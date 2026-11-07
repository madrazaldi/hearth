//! Avatars and small shared widgets: the initial tile / picture stand-in,
//! status dots, and lucide icons by name.

use super::data::Pic;
use super::prelude::*;
use super::theme::{HearthTheme as _, nick_hue};
use gpui_kit::component::Icon;
use hearth_model::rows::initial;

/// A lucide icon from the bundled catalog (`icons/<name>.svg`).
pub fn icon(name: &str, size: f32, color: Hsla) -> Icon {
    Icon::default().path(format!("icons/{name}.svg")).size(px(size)).text_color(color)
}

/// A round status dot, ringed in the surface colour it sits on.
pub fn dot(size: f32, color: Hsla, ring: Option<Hsla>) -> Div {
    div().flex_none().size(px(size)).rounded_full().bg(color).when_some(ring, |d, r| d.border_2().border_color(r))
}

/// The nick's initial on its hue, or the person's picture once it has loaded.
#[derive(IntoElement)]
pub struct Avatar {
    nick: SharedString,
    size: f32,
    picture: Option<ImageSource>,
}

impl Avatar {
    pub fn new(nick: impl Into<SharedString>, size: f32) -> Self {
        Avatar { nick: nick.into(), size, picture: None }
    }

    pub fn picture(mut self, picture: &Pic) -> Self {
        self.picture = picture.0.clone();
        self
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = cx.hearth();
        let hue = nick_hue(&self.nick);
        let s = self.size;
        // The 1px ring is drawn over the content, and the
        // picture is rounded itself because an overflow clip is a rectangle in GPUI.
        let ring = || div().absolute().inset_0().rounded_full().border_1().border_color(p.border);
        let base = div().relative().flex_none().size(px(s)).rounded_full().overflow_hidden();
        if let Some(src) = self.picture {
            return base.child(img(src).size_full().rounded_full().object_fit(ObjectFit::Cover)).child(ring());
        }
        base.bg(p.tile(hue, p.muted, 0.24))
            .flex()
            .items_center()
            .justify_center()
            .text_color(p.nick(hue))
            .text_size(px((s * if s > 24.0 { 0.4 } else { 0.5 }).round()))
            .font_weight(FontWeight::SEMIBOLD)
            .line_height(relative(1.0))
            .child(initial(&self.nick))
            .child(ring())
    }
}
