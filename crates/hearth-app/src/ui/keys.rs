//! Global keyboard shortcuts: Cmd on macOS, Ctrl
//! elsewhere. The bindings have no key context and the handlers reach the
//! window through a global, so they work from inside dialogs too.

use super::prelude::*;
use super::shell::ChatWindow;
use std::rc::Rc;

actions!(
    hearth_keys,
    [
        ToggleSwitcher,
        ToggleSearch,
        FilterMessages,
        ToggleShortcuts,
        OpenSettingsPanel,
        ToggleMemberList,
        PrevBuffer,
        NextBuffer,
        PrevUnread,
        NextUnread,
        CloseBuffer,
        GoTo1,
        GoTo2,
        GoTo3,
        GoTo4,
        GoTo5,
        GoTo6,
        GoTo7,
        GoTo8,
        GoTo9,
    ]
);

/// The window the shortcuts act on.
pub struct ShellHandle(pub WeakEntity<ChatWindow>);

impl Global for ShellHandle {}

/// Primary modifier plus `$rest`: Cmd on macOS, Ctrl elsewhere. GPUI's static parser
/// has no `secondary-` alias, so the name is chosen at compile time.
#[cfg(target_os = "macos")]
macro_rules! m {
    ($rest:literal) => {
        concat!("cmd", $rest)
    };
}
#[cfg(not(target_os = "macos"))]
macro_rules! m {
    ($rest:literal) => {
        concat!("ctrl", $rest)
    };
}

fn k<A: Action>(keys: &'static str, action: A) -> KeyBinding {
    KeyBinding::new(keys, action, None)
}

fn on<A: Action>(cx: &mut App, f: impl Fn(&mut ChatWindow, &mut Window, &mut Context<ChatWindow>) + 'static) {
    let f = Rc::new(f);
    cx.on_action(move |_: &A, cx: &mut App| {
        let Some(handle) = cx.try_global::<ShellHandle>().map(|h| h.0.clone()) else { return };
        // Windows that are not frontmost (background runs) still take the shortcut.
        let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else { return };
        // The window is borrowed while it dispatches the key, so act once that is over.
        let f = f.clone();
        cx.defer(move |cx| {
            let _ = window.update(cx, |_, window, cx| {
                let _ = handle.update(cx, |shell, cx| f(shell, window, cx));
            });
        });
    });
}

/// Bind the shortcuts. On macOS Cmd+W stays the window menu's Close (bound by the
/// platform layer); Windows and Linux have no such key, so Ctrl+W closes the buffer.
pub fn init(cx: &mut App) {
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([k("ctrl-w", CloseBuffer)]);
    cx.bind_keys([
        k(m!("-k"), ToggleSwitcher),
        k(m!("-shift-f"), ToggleSearch),
        k(m!("-f"), FilterMessages),
        k(m!("-/"), ToggleShortcuts),
        k(m!("-,"), OpenSettingsPanel),
        k(m!("-shift-m"), ToggleMemberList),
        k("alt-up", PrevBuffer),
        k("alt-down", NextBuffer),
        k("alt-shift-up", PrevUnread),
        k("alt-shift-down", NextUnread),
        k(m!("-1"), GoTo1),
        k(m!("-2"), GoTo2),
        k(m!("-3"), GoTo3),
        k(m!("-4"), GoTo4),
        k(m!("-5"), GoTo5),
        k(m!("-6"), GoTo6),
        k(m!("-7"), GoTo7),
        k(m!("-8"), GoTo8),
        k(m!("-9"), GoTo9),
    ]);
    on::<ToggleSwitcher>(cx, ChatWindow::toggle_switcher);
    on::<ToggleSearch>(cx, |s, w, cx| s.toggle_search(None, w, cx));
    on::<FilterMessages>(cx, ChatWindow::filter_messages);
    on::<ToggleShortcuts>(cx, ChatWindow::toggle_shortcuts);
    on::<OpenSettingsPanel>(cx, ChatWindow::toggle_settings);
    on::<ToggleMemberList>(cx, ChatWindow::toggle_members);
    on::<PrevBuffer>(cx, |s, w, cx| s.step_buffer(-1, false, w, cx));
    on::<NextBuffer>(cx, |s, w, cx| s.step_buffer(1, false, w, cx));
    on::<PrevUnread>(cx, |s, w, cx| s.step_buffer(-1, true, w, cx));
    on::<NextUnread>(cx, |s, w, cx| s.step_buffer(1, true, w, cx));
    on::<CloseBuffer>(cx, ChatWindow::close_current);
    on::<GoTo1>(cx, |s, w, cx| s.go_nth(1, w, cx));
    on::<GoTo2>(cx, |s, w, cx| s.go_nth(2, w, cx));
    on::<GoTo3>(cx, |s, w, cx| s.go_nth(3, w, cx));
    on::<GoTo4>(cx, |s, w, cx| s.go_nth(4, w, cx));
    on::<GoTo5>(cx, |s, w, cx| s.go_nth(5, w, cx));
    on::<GoTo6>(cx, |s, w, cx| s.go_nth(6, w, cx));
    on::<GoTo7>(cx, |s, w, cx| s.go_nth(7, w, cx));
    on::<GoTo8>(cx, |s, w, cx| s.go_nth(8, w, cx));
    on::<GoTo9>(cx, |s, w, cx| s.go_nth(9, w, cx));
}
