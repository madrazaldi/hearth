//! The keyboard shortcut list, shown in the help overlay (Cmd/Ctrl+/) and in
//! Settings.

use super::controls::{eyebrow, keycap};
use super::prelude::*;
use super::theme::{HearthTheme as _, Palette};
use hearth_model::shortcuts::{Platform, shortcut_groups};

pub fn shortcut_list(p: &Palette) -> Div {
    v_flex().gap(px(21.)).children(shortcut_groups(Platform::current()).into_iter().map(|g| {
        v_flex().gap(px(3.5)).child(div().mb(px(3.5)).child(eyebrow(g.title, p))).children(g.items.into_iter().enumerate().map(|(i, sc)| {
            let (combos, label) = (sc.keys, sc.label);
            h_flex()
                .py(px(7.))
                .gap(px(21.))
                .items_center()
                .justify_between()
                .when(i > 0, |d| d.border_t_1().border_color(p.border))
                .text_size(px(13.5))
                .child(div().text_color(p.foreground.opacity(0.9)).child(label))
                .child(h_flex().flex_none().gap(px(5.25)).items_center().children(combos.into_iter().enumerate().flat_map(|(ci, keys)| {
                    let mut out: Vec<AnyElement> = Vec::new();
                    if ci > 0 {
                        out.push(div().text_size(px(10.5)).text_color(p.muted_foreground.opacity(0.7)).child("or").into_any_element());
                    }
                    out.push(h_flex().gap(px(3.5)).children(keys.iter().map(|k| keycap(k, p))).into_any_element());
                    out
                })))
        }))
    }))
}

/// The help overlay's content.
pub struct ShortcutsHelp;

impl Render for ShortcutsHelp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        // Sized to its content like the old dialog, up to 86% of the window.
        let content: f32 = shortcut_groups(Platform::current()).iter().map(|g| 24.5 + g.items.len() as f32 * 34.6 + 21.).sum();
        let h = (73. + 35. + content).min(window.viewport_size().height.as_f32() * 0.86);
        v_flex()
            .h(px(h))
            .w_full()
            .child(
                v_flex()
                    .flex_none()
                    .gap(px(3.5))
                    .px(px(21.))
                    .py(px(14.))
                    .border_b_1()
                    .border_color(p.border)
                    .child(div().text_size(px(15.75)).font_weight(FontWeight::SEMIBOLD).child("Keyboard shortcuts"))
                    .child(div().text_size(px(12.25)).text_color(p.muted_foreground).child("Available anywhere in Hearth.")),
            )
            .child(div().id("shortcuts-scroll").flex_1().min_h_0().overflow_y_scroll().px(px(21.)).py(px(17.5)).child(shortcut_list(&p)))
    }
}
