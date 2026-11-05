//! The application menu (macOS) and the window/app key bindings.

use gpui_kit::*;

// Menu-only actions are unused where there is no menu bar.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod actions {
    use gpui_kit::actions;
    actions!(hearth_platform, [Quit, Hide, HideOthers, ShowAll, About, Minimize, Zoom, CloseWindow, ToggleFullScreen]);
}
use actions::*;

pub(super) fn init(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| super::quit(cx));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &About, _| {
        #[cfg(target_os = "macos")]
        super::mac::show_about();
    });
    cx.on_action(|_: &Minimize, cx| with_active_window(cx, |window| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx| with_active_window(cx, |window| window.zoom_window()));
    cx.on_action(|_: &ToggleFullScreen, cx| with_active_window(cx, |window| window.toggle_fullscreen()));
    cx.on_action(|_: &CloseWindow, cx| {
        // Same path as the close button, so close-to-tray applies.
        if super::should_close(cx) {
            with_active_window(cx, |window| window.remove_window());
        }
    });

    #[cfg(target_os = "macos")]
    {
        // Cmd+W closes the window, as in every Mac app; the buffer-close shortcut
        // is Ctrl+W on the other platforms only.
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        ]);
        cx.set_menus(mac_menus());
    }
    #[cfg(not(target_os = "macos"))]
    {
        // There is no menu bar to show these in; Alt+F4 closes the window natively.
        cx.bind_keys([KeyBinding::new("f11", ToggleFullScreen, None)]);
        #[cfg(target_os = "linux")]
        cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
    }
}

fn with_active_window(cx: &mut App, f: impl FnOnce(&mut Window)) {
    if let Some(handle) = cx.active_window() {
        handle.update(cx, |_, window, _| f(window)).ok();
    }
}

#[cfg(target_os = "macos")]
fn mac_menus() -> Vec<Menu> {
    use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    vec![
        Menu::new("Hearth").items([
            MenuItem::action("About Hearth", About),
            MenuItem::separator(),
            MenuItem::action("Hide Hearth", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Hearth", Quit),
        ]),
        // These dispatch the text inputs' own actions to whatever is focused.
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
            MenuItem::action("Close", CloseWindow),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", ToggleFullScreen),
        ]),
    ]
}
