//! Desktop notifications. macOS goes through `mac`; Windows and Linux through
//! `notify-rust`, which blocks while it talks to the system, so it runs on a
//! thread of its own.

use super::Commands;
use std::sync::OnceLock;

static COMMANDS: OnceLock<Commands> = OnceLock::new();

pub(super) fn set_commands(commands: Commands) {
    let _ = COMMANDS.set(commands);
}

#[cfg(target_os = "macos")]
pub(super) fn send(title: &str, body: &str) {
    super::mac::notify(title, body);
}

#[cfg(not(target_os = "macos"))]
pub(super) fn send(title: &str, body: &str) {
    let (title, body) = (title.to_owned(), body.to_owned());
    let spawned = std::thread::Builder::new().name("hearth-notify".into()).spawn(move || show(&title, &body));
    if let Err(e) = spawned {
        log::warn!("could not post a notification: {e}");
    }
}

#[cfg(not(target_os = "macos"))]
fn show(title: &str, body: &str) {
    let mut notification = notify_rust::Notification::new();
    notification.appname(super::APP_NAME).summary(title).body(body);

    // Windows: toasts need a registered app id; the installer does not create
    // one, so they appear under the system default sender.
    #[cfg(target_os = "linux")]
    {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Waiting for a click holds a thread until the notification goes away.
        static WAITING: AtomicUsize = AtomicUsize::new(0);
        const MAX_WAITING: usize = 8;

        // The icon name is the one the .deb and AppImage install.
        notification.icon("hearth");
        let clickable = WAITING.fetch_add(1, Ordering::AcqRel) < MAX_WAITING;
        if clickable {
            notification.action("default", "Open Hearth");
        }
        match notification.show() {
            Ok(handle) if clickable => handle.wait_for_action(|action| {
                if action == "default"
                    && let Some(commands) = COMMANDS.get()
                {
                    let _ = commands.try_send(super::Command::Show);
                }
            }),
            Ok(_) => {}
            Err(e) => log::warn!("could not post a notification: {e}"),
        }
        WAITING.fetch_sub(1, Ordering::AcqRel);
    }
    #[cfg(windows)]
    if let Err(e) = notification.show() {
        log::warn!("could not post a notification: {e}");
    }
}
