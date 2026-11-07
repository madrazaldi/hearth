//! Popup menus shared by the sidebar rows and the buffer header.

use super::avatar::icon;
use super::data::{BufferId, BufferKind, Dispatch, UiAction};
use super::prelude::*;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use hearth_model::types::NotifyLevel;

/// What the menu offers depends on these.
#[derive(Clone, Copy)]
pub struct BufferFacts {
    pub kind: BufferKind,
    pub joined: bool,
    pub level: Option<NotifyLevel>,
}

pub fn buffer_menu(
    menu: PopupMenu,
    id: &BufferId,
    BufferFacts { kind, joined, level }: BufferFacts,
    dispatch: &Dispatch,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let fg = cx.theme_fg();
    let item = |label: &'static str, icon_name: &str, action: UiAction| {
        let d = dispatch.clone();
        PopupMenuItem::new(label).icon(icon(icon_name, 15., fg)).on_click(move |_, window, cx| d(action.clone(), window, cx))
    };
    let leave = if kind == BufferKind::Channel && joined { "Leave channel" } else { "Close" };
    let mut menu = menu.item(item("Mark as read", "check-check", UiAction::MarkRead(id.clone())));
    if kind == BufferKind::Channel && !joined {
        menu = menu.item(item("Rejoin", "refresh-cw", UiAction::Rejoin(id.clone())));
    }
    menu.separator()
        .submenu("Notify me about", window, cx, {
            let (dispatch, id) = (dispatch.clone(), id.clone());
            move |m, _, _| {
                let n = |label: &'static str, choice: Option<NotifyLevel>, checked: bool| {
                    let (d, id) = (dispatch.clone(), id.clone());
                    PopupMenuItem::new(label).checked(checked).on_click(move |_, window, cx| d(UiAction::SetNotify(id.clone(), choice), window, cx))
                };
                m.item(n("Default", None, level.is_none()))
                    .item(n("All messages", Some(NotifyLevel::All), level == Some(NotifyLevel::All)))
                    .item(n("Mentions only", Some(NotifyLevel::Mentions), level == Some(NotifyLevel::Mentions)))
                    .item(n("Nothing (mute)", Some(NotifyLevel::None), level == Some(NotifyLevel::None)))
            }
        })
        .separator()
        .item(item(leave, "x", UiAction::CloseBuffer(id.clone())))
}

trait ThemeFg {
    fn theme_fg(&self) -> Hsla;
}

impl<T> ThemeFg for Context<'_, T> {
    fn theme_fg(&self) -> Hsla {
        use gpui_kit::component::ActiveTheme as _;
        (**self).theme().foreground
    }
}
