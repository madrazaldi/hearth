//! The system tray icon: Show Hearth / Quit Hearth, and the unread count as its tooltip.

use super::{APP_NAME, Command, Commands};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// A monochrome template image that macOS tints for the menu bar.
#[cfg(target_os = "macos")]
const ICON: &[u8] = include_bytes!("../../assets/tray/tray-template@2x.png");
#[cfg(not(target_os = "macos"))]
const ICON: &[u8] = include_bytes!("../../assets/tray/tray-icon.png");

const SHOW: &str = "show";
const QUIT: &str = "quit";

pub struct Tray {
    icon: TrayIcon,
}

/// Must run on the main thread, inside GPUI's run loop (macOS and Windows put
/// the icon on the thread that creates it). `None` when the platform has no
/// tray, e.g. a Linux session without a StatusNotifier host.
pub fn create(commands: Commands) -> Option<Tray> {
    let menu = Menu::new();
    let show = MenuItem::with_id(SHOW, format!("Show {APP_NAME}"), true, None);
    let quit = MenuItem::with_id(QUIT, format!("Quit {APP_NAME}"), true, None);
    menu.append_items(&[&show, &PredefinedMenuItem::separator(), &quit]).ok()?;

    // The handler runs on the platform's UI thread (or the tray's own thread
    // on Linux); the channel carries it over to GPUI.
    let menu_commands = commands.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let command = match event.id.0.as_str() {
            SHOW => Command::Show,
            QUIT => Command::Quit,
            _ => return,
        };
        let _ = menu_commands.try_send(command);
    }));
    #[cfg(windows)]
    tray_icon::TrayIconEvent::set_event_handler(Some(move |event: tray_icon::TrayIconEvent| {
        if let tray_icon::TrayIconEvent::DoubleClick { button: tray_icon::MouseButton::Left, .. } = event {
            let _ = commands.try_send(Command::Show);
        }
    }));
    #[cfg(not(windows))]
    drop(commands);

    let builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip(APP_NAME);
    #[cfg(target_os = "macos")]
    let builder = builder.with_icon_templated(icon()?);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.with_icon(icon()?);
    match builder.build() {
        Ok(icon) => Some(Tray { icon }),
        Err(e) => {
            log::warn!("no tray icon: {e}");
            None
        }
    }
}

impl Tray {
    /// There is no per-tray badge on Windows or Linux, so the count rides on the tooltip.
    pub fn set_badge(&self, count: u32) {
        let tooltip = match count {
            0 => APP_NAME.to_string(),
            1 => format!("{APP_NAME}: 1 unread mention"),
            n => format!("{APP_NAME}: {n} unread mentions"),
        };
        let _ = self.icon.set_tooltip(Some(tooltip));
    }
}

fn icon() -> Option<Icon> {
    let rgba = image::load_from_memory(ICON).ok()?.into_rgba8();
    let (width, height) = rgba.dimensions();
    Icon::from_rgba(rgba.into_raw(), width, height).ok()
}
