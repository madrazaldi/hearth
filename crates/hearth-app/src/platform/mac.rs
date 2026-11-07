//! AppKit and UserNotifications pieces GPUI does not wrap: the Dock badge, the
//! About panel, un-hiding the app, and notifications.

use super::{Command, Commands};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, define_class, msg_send};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSBundle, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};
use std::sync::atomic::{AtomicU64, Ordering};

pub fn set_dock_badge(label: Option<&str>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let tile = NSApplication::sharedApplication(mtm).dockTile();
    tile.setBadgeLabel(label.map(NSString::from_str).as_deref());
}

pub fn show_about() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanel(None);
    }
}

/// Undoes Hide (Cmd+H or close-to-tray) and Minimize.
pub fn unhide_and_restore() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.unhide(None);
    for window in app.windows() {
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
    }
}

/// UserNotifications only works for a process inside an app bundle.
fn bundled() -> bool {
    let bundle = NSBundle::mainBundle();
    bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
}

struct Ivars {
    commands: Commands,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the ivars are only a channel sender.
    #[unsafe(super(NSObject))]
    #[name = "HearthNotificationDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        // Without this, macOS holds back banners while the app is frontmost, which is
        // exactly when a mention in another channel should still be announced.
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            done: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            done.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(&self, _center: &UNUserNotificationCenter, _response: &UNNotificationResponse, done: &block2::DynBlock<dyn Fn()>) {
            let _ = self.ivars().commands.try_send(Command::Show);
            done.call(());
        }
    }
);

impl Delegate {
    fn new(commands: Commands) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { commands });
        unsafe { msg_send![super(this), init] }
    }
}

/// Keeps the notification delegate alive (the center only holds it weakly).
pub struct Notifier {
    _delegate: Retained<Delegate>,
}

/// Asks for permission and routes notification clicks to `Command::Show`.
/// `None` outside an app bundle, where `notify` falls back to AppleScript.
pub fn init_notifications(commands: Commands) -> Option<Notifier> {
    if !bundled() {
        return None;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = Delegate::new(commands);
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let options = UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge;
    let result = RcBlock::new(|granted: Bool, error: *mut NSError| {
        if !granted.as_bool() {
            let reason = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
            log::info!("notifications are not allowed: {}", reason.as_deref().unwrap_or("denied"));
        }
    });
    center.requestAuthorizationWithOptions_completionHandler(options, &result);
    Some(Notifier { _delegate: delegate })
}

pub fn notify(title: &str, body: &str) {
    if !bundled() {
        return script_notification(title, body);
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let id = NSString::from_str(&format!("hearth-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
    UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, None);
}

/// `cargo run` builds are not bundles, so UserNotifications is unavailable to them.
fn script_notification(title: &str, body: &str) {
    fn quoted(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
    let script = format!("display notification {} with title {}", quoted(body), quoted(title));
    // A thread waits for the child so it does not linger as a zombie.
    std::thread::spawn(move || {
        if let Err(e) = std::process::Command::new("osascript").args(["-e", &script]).status() {
            log::warn!("could not post a notification: {e}");
        }
    });
}
