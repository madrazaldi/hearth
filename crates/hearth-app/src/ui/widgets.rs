//! Small shared widgets: ghost icon buttons, pills, tooltips.

use super::avatar::icon;
use super::prelude::*;
use super::theme::{HearthTheme as _, Palette};
use crate::host::Hearth;
use gpui_kit::component::tooltip::Tooltip;
use hearth_model::state::ToastKind;

/// A ghost icon button: `(size, icon_size)` px, the button square, rounded, tinted on hover.
pub fn icon_button(
    id: impl Into<ElementId>,
    name: &str,
    (size, icon_size): (f32, f32),
    color: Hsla,
    hover_bg: Hsla,
    tip: Option<&'static str>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px(6.75))
        .text_color(color)
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .child(icon(name, icon_size, color))
        .when_some(tip, |d, tip| d.tooltip(move |window, cx| Tooltip::new(tip).build(window, cx)))
        .on_click(move |_, window, cx| on_click(window, cx))
}

/// A filled count pill (mentions, unread).
pub fn count_badge(n: usize, cx: &App) -> Div {
    let p = cx.hearth();
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_none()
        .h(px(15.75))
        .min_w(px(15.75))
        .px(px(5.25))
        .rounded_full()
        .bg(p.brand)
        .text_color(p.brand_text)
        .text_size(px(10.5))
        .font_weight(FontWeight::SEMIBOLD)
        .line_height(relative(1.0))
        .child(n.to_string())
}

/// A keyboard hint such as "Ctrl K".
pub fn kbd_hint(text: impl Into<SharedString>, cx: &App) -> Div {
    let p = cx.hearth();
    div()
        .flex_none()
        .h(px(17.5))
        .min_w(px(17.5))
        .px(px(3.5))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.75))
        .bg(p.muted)
        .text_color(p.muted_foreground)
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

/// An outlined label such as "notice".
pub fn tag(text: impl Into<SharedString>, cx: &App) -> Div {
    let p = cx.hearth();
    div()
        .flex_none()
        .px(px(6.))
        .h(px(16.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(p.border)
        .text_color(p.muted_foreground)
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

/// The model's toasts stacked in the bottom corner; each is dismissed by a click or its timer.
pub fn toast_layer(model: &Entity<Hearth>, p: &Palette, cx: &App) -> Option<AnyElement> {
    let toasts = model.read(cx).app.toasts.clone();
    if toasts.is_empty() {
        return None;
    }
    Some(
        v_flex()
            .absolute()
            .right(px(20.))
            .bottom(px(20.))
            .w(px(340.))
            .gap(px(8.))
            .children(toasts.into_iter().map(|t| {
                let error = t.kind == ToastKind::Error;
                let (dismiss, act) = (model.clone(), model.clone());
                let id = t.id;
                h_flex()
                    .id(SharedString::from(format!("toast-{id}")))
                    .gap(px(10.))
                    .items_center()
                    .py(px(10.))
                    .px(px(14.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(if error { p.destructive.opacity(0.5) } else { p.border })
                    .bg(p.popover)
                    .shadow_lg()
                    .text_size(px(13.))
                    .cursor_pointer()
                    .on_click(move |_, _, cx| dismiss.update(cx, |h, cx| h.act(cx, |app| app.dismiss_toast(id))))
                    .child(div().flex_none().size(px(8.)).rounded_full().bg(if error { p.destructive } else { p.brand }))
                    .child(div().flex_1().min_w_0().child(t.text))
                    .children(t.action.map(|a| {
                        div()
                            .id(SharedString::from(format!("toast-act-{id}")))
                            .flex_none()
                            .px(px(8.))
                            .h(px(24.))
                            .flex()
                            .items_center()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(p.input)
                            .font_weight(FontWeight::MEDIUM)
                            .child(a.label)
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                act.update(cx, |h, cx| h.act(cx, |app| app.run_toast_action(id)))
                            })
                    }))
            }))
            .into_any_element(),
    )
}
