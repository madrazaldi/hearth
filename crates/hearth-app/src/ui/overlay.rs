//! Opening a view as a modal: the kit's dialog layer gives the backdrop, the
//! focus trap and Escape; this adds the one-time initial focus and tells the
//! shell when the user dismissed the dialog, so the model can drop its state.

use super::prelude::*;
use super::theme::HearthTheme as _;
use gpui_kit::component::WindowExt as _;
use std::cell::Cell;
use std::rc::Rc;

type Hook = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct Spec {
    pub width: f32,
    /// Distance from the top of the window; `None` centres vertically.
    pub top: Option<f32>,
    pub close_button: bool,
}

/// Show `view` in a dialog. `focus` runs once, on the first build, because the
/// dialog shell takes focus when it opens and the builder reruns on every rebuild.
/// `closed` runs after the dialog was dismissed, by Escape, the backdrop or its button.
pub fn open(
    window: &mut Window,
    cx: &mut App,
    view: AnyView,
    spec: Spec,
    focus: impl Fn(&mut Window, &mut App) + 'static,
    closed: impl Fn(&mut Window, &mut App) + 'static,
) {
    let first = Cell::new(true);
    let closed: Hook = Rc::new(closed);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            focus(window, cx);
        }
        let view = view.clone();
        let closed = closed.clone();
        let dialog = dialog
            .bg(cx.hearth().popover)
            .rounded(px(12.75))
            .w(px(spec.width))
            .p_0()
            .close_button(spec.close_button)
            .on_close(move |_, window, cx| closed(window, cx))
            // The old app set `line-height: 1.45` on the body; GPUI's default is looser.
            .content(move |content, _, _| content.child(div().w_full().line_height(relative(1.45)).child(view.clone())));
        match spec.top {
            Some(t) => dialog.margin_top(px(t)),
            None => dialog.centered(),
        }
    });
}

pub fn close_all(window: &mut Window, cx: &mut App) {
    window.close_all_dialogs(cx);
}

pub fn is_open(window: &mut Window, cx: &mut App) -> bool {
    window.has_active_dialog(cx)
}
