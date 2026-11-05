//! The image viewer: click an inline image to see it at full size.
//! Escape and the close button come from the dialog.

use super::avatar::icon;
use super::data::*;
use super::messages::{fit, image_box};
use super::prelude::*;
use super::theme::{HearthTheme as _, MONO};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use hearth_model::media::file_name;

pub struct Lightbox {
    url: String,
    image: ImageView,
    dispatch: Dispatch,
}

impl Lightbox {
    pub fn new(url: String, image: ImageView, dispatch: Dispatch) -> Self {
        Lightbox { url, image, dispatch }
    }

    /// The dialog width that fits this image on a window of `viewport` width.
    pub fn width(image: &ImageView, viewport: Size<Pixels>) -> f32 {
        let (w, _) = fit(image.width, image.height, viewport.width.as_f32() - 120., viewport.height.as_f32() - 220.);
        (w + 21.).max(420.)
    }
}

impl Render for Lightbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let vp = window.viewport_size();
        let (w, h) = fit(self.image.width, self.image.height, vp.width.as_f32() - 120., vp.height.as_f32() - 220.);
        let (url, d) = (self.url.clone(), self.dispatch.clone());
        v_flex()
            .w_full()
            .child(
                h_flex()
                    .h(px(42.))
                    .flex_none()
                    .gap(px(7.))
                    .items_center()
                    .pl(px(14.))
                    .pr(px(52.))
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(7.))
                            .items_baseline()
                            .child(div().truncate().text_size(px(12.25)).font_weight(FontWeight::MEDIUM).child(file_name(&self.url)))
                            .child(
                                div()
                                    .flex_none()
                                    .font_family(MONO)
                                    .text_size(px(10.5))
                                    .text_color(p.muted_foreground)
                                    .child(format!("{}×{}", self.image.width as u32, self.image.height as u32)),
                            ),
                    )
                    .child(
                        Button::new("lightbox-open")
                            .small()
                            .outline()
                            .icon(icon("external-link", 14., p.foreground))
                            .label("Open in browser")
                            .on_click(move |_, w, cx| d(UiAction::OpenUrl(url.clone()), w, cx)),
                    ),
            )
            .child(div().flex().items_center().justify_center().p(px(10.5)).bg(p.muted.opacity(0.3)).child(image_box(&self.image, w, h, 6.75)))
    }
}
