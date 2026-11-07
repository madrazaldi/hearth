//! Desktop integration: application menu, tray, close-to-tray, graceful quit,
//! notifications, the unread badge, opening links and self-update.
//!
//! The rest of the app talks to this module only through the functions below,
//! all called on GPUI's main thread unless noted. `init` runs once, inside
//! `Application::run`, before the first window opens.

// Much of this is API for the code that wires the model into the shell to call.
#![allow(dead_code)]

mod menus;
mod notify;
mod open;
mod tray;
pub mod updater;

#[cfg(target_os = "macos")]
mod mac;

use gpui_kit::{App, Application, AsyncApp, FutureExt as _, Global, QuitMode, Window};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

pub use open::check_openable;

pub const APP_ID: &str = "app.hearth.irc";
pub const APP_NAME: &str = "Hearth";
/// The installed `.desktop` file's name (the binary's), which Linux matches windows to.
pub const DESKTOP_ID: &str = "hearth";

/// The running version, from the workspace `Cargo.toml` (the single source of truth).
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// How long quitting waits for the QUIT lines to reach the servers.
pub const SHUTDOWN_BUDGET: Duration = Duration::from_millis(1500);

pub type ShutdownFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
/// Called with the time budget when the app is about to exit. It runs on the
/// engine's tokio runtime, e.g. `Arc::new(move |max| core.shutdown_gracefully(max))`.
pub type ShutdownHook = Arc<dyn Fn(Duration) -> ShutdownFuture + Send + Sync>;

pub struct Config {
    /// Opens the main window (and calls `attach_window` on it). Used when the
    /// window was closed to the tray, and when the Dock icon is clicked with
    /// no window open.
    pub open_main_window: Rc<dyn Fn(&mut App)>,
    pub shutdown: ShutdownHook,
    /// The `closeToTray` setting at startup; keep it current with `set_close_to_tray`.
    pub close_to_tray: bool,
}

/// What the tray menu and notification clicks ask of the app. They come from
/// other threads (or native callbacks), so they travel over a channel.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Command {
    Show,
    Quit,
}
pub(crate) type Commands = async_channel::Sender<Command>;

struct State {
    open_main_window: Rc<dyn Fn(&mut App)>,
    shutdown: ShutdownHook,
    close_to_tray: bool,
    /// Windows and Linux cannot hide a GPUI window, so closing to the tray
    /// closes it and Show opens a new one; this keeps that from quitting.
    in_tray: bool,
    quitting: bool,
    shutdown_done: bool,
    tray: Option<tray::Tray>,
    #[cfg(target_os = "macos")]
    _notifier: Option<mac::Notifier>,
}

impl Global for State {}

pub fn init(cx: &mut App, config: Config) {
    // Quitting is decided here (last window closed, tray Quit, menu), never by GPUI on its own.
    cx.set_quit_mode(QuitMode::Explicit);

    let (tx, rx) = async_channel::unbounded();
    notify::set_commands(tx.clone());
    menus::init(cx);

    cx.set_global(State {
        open_main_window: config.open_main_window,
        shutdown: config.shutdown,
        close_to_tray: config.close_to_tray,
        in_tray: false,
        quitting: false,
        shutdown_done: false,
        tray: tray::create(tx.clone()),
        #[cfg(target_os = "macos")]
        _notifier: mac::init_notifications(tx),
    });

    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() && !cx.global::<State>().in_tray {
            quit(cx);
        }
    })
    .detach();

    // The Dock menu's Quit and a logout end the process through the OS, which
    // gives these handlers only a moment. `quit` has normally been through here already.
    cx.on_app_quit(|cx| {
        let state = cx.global_mut::<State>();
        let hook = (!state.shutdown_done).then(|| state.shutdown.clone());
        state.shutdown_done = true;
        async move {
            if let Some(hook) = hook {
                let _ = hearth_core::runtime::spawn(hook(Duration::from_millis(150))).await;
            }
        }
    })
    .detach();

    cx.spawn(async move |cx| {
        while let Ok(command) = rx.recv().await {
            cx.update(|cx| match command {
                Command::Show => show_main_window(cx),
                Command::Quit => quit(cx),
            });
        }
    })
    .detach();
}

/// Dock-icon clicks with no window open (macOS). GPUI only offers this on the
/// `Application`, before `run`.
pub fn on_reopen(app: &Application) {
    app.on_reopen(show_main_window);
}

/// Call on the main window as it opens, so closing it can hide to the tray.
pub fn attach_window(window: &mut Window, cx: &mut App) {
    window.on_window_should_close(cx, |_, cx| should_close(cx));
}

/// Whether the window may really close. Also what the Window menu's Close does.
fn should_close(cx: &mut App) -> bool {
    let state = cx.global_mut::<State>();
    if state.quitting {
        return true;
    }
    if cfg!(target_os = "macos") {
        // Hiding the app keeps the window and its state, like the old shell's `win.hide()`.
        if state.close_to_tray {
            cx.hide();
            return false;
        }
    } else if state.close_to_tray && state.tray.is_some() {
        state.in_tray = true;
        return true;
    }
    true
}

pub fn set_close_to_tray(cx: &mut App, on: bool) {
    cx.global_mut::<State>().close_to_tray = on;
}

/// Brings the app back: from the tray, the Dock, or behind other windows.
pub fn show_main_window(cx: &mut App) {
    #[cfg(target_os = "macos")]
    mac::unhide_and_restore();

    let state = cx.global_mut::<State>();
    state.in_tray = false;
    let open = state.open_main_window.clone();
    match cx.windows().first() {
        Some(window) => {
            window.update(cx, |_, window, _| window.activate_window()).ok();
        }
        None => open(cx),
    }
    cx.activate(true);
}

/// Quits after giving the connections a moment to send QUIT. Safe to call twice.
pub fn quit(cx: &mut App) {
    let state = cx.global_mut::<State>();
    if state.quitting {
        return;
    }
    state.quitting = true;
    cx.spawn(async move |cx| {
        shutdown_connections(cx).await;
        cx.update(|cx| cx.quit());
    })
    .detach();
}

/// Runs the shutdown hook, bounded even if the engine's runtime is stuck.
pub(crate) async fn shutdown_connections(cx: &mut AsyncApp) {
    let hook = cx.update(|cx| {
        let state = cx.global_mut::<State>();
        state.quitting = true;
        let hook = (!state.shutdown_done).then(|| state.shutdown.clone());
        state.shutdown_done = true;
        hook
    });
    if let Some(hook) = hook {
        let task = hearth_core::runtime::spawn(hook(SHUTDOWN_BUDGET));
        let _ = task.with_timeout(SHUTDOWN_BUDGET + Duration::from_millis(500), cx.background_executor()).await;
    }
}

/// Starts a fresh copy of the app once this process exits. Call after the shutdown hook has run.
pub(crate) fn relaunch(cx: &mut App) {
    #[cfg(target_os = "linux")]
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        // The running binary lives in the AppImage's temporary mount, which the update replaced.
        cx.set_restart_path(appimage.into());
    }
    cx.restart();
    cx.quit();
}

/// The unread-mention count on the Dock tile (macOS) and the tray tooltip.
pub fn set_badge(cx: &mut App, count: u32) {
    #[cfg(target_os = "macos")]
    mac::set_dock_badge(badge_label(count).as_deref());
    if let Some(tray) = &cx.global::<State>().tray {
        tray.set_badge(count);
    }
}

/// A system notification. Any thread.
pub fn notify(title: &str, body: &str) {
    notify::send(title, body);
}

/// Opens a link from a chat message, if it is a web, mail or phone link.
pub fn open_url(cx: &App, raw: &str) -> Result<(), String> {
    let url = check_openable(raw)?;
    cx.open_url(url.as_str());
    Ok(())
}

/// The text on the badge: nothing for zero, capped so it fits the tile.
pub(crate) fn badge_label(count: u32) -> Option<String> {
    match count {
        0 => None,
        1..=999 => Some(count.to_string()),
        _ => Some("999+".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_label_is_empty_for_zero_and_capped() {
        assert_eq!(badge_label(0), None);
        assert_eq!(badge_label(7).as_deref(), Some("7"));
        assert_eq!(badge_label(999).as_deref(), Some("999"));
        assert_eq!(badge_label(1000).as_deref(), Some("999+"));
    }

    #[test]
    fn version_is_the_workspace_version() {
        assert!(updater::parse_version(version()).is_some());
    }
}
