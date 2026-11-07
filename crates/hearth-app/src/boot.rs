//! Startup: assets, fonts, the host and the main window. The binary runs
//! [`run`]; the screenshot example and the host tests reuse the pieces.

use crate::host::{self, Boot, Hearth};
use crate::platform;
use crate::ui;
use gpui_kit::assets::AllAssets;
use gpui_kit::*;
use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;
use ui::shell::ChatWindow;

/// Hearth's own assets (logo, and lucide icons the catalog lacks) on top of the bundled lucide icon catalog.
pub struct Assets;

const LOGO: &[u8] = include_bytes!("../assets/logo.png");
const SMILE: &[u8] = include_bytes!("../assets/lucide/smile.svg");

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            "logo.png" => Ok(Some(Cow::Borrowed(LOGO))),
            "icons/smile.svg" => Ok(Some(Cow::Borrowed(SMILE))),
            _ => AllAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        AllAssets.list(path)
    }
}

pub fn load_fonts(cx: &mut App) {
    macro_rules! font {
        ($name:literal) => {
            Cow::Borrowed(&include_bytes!(concat!("../assets/fonts/", $name))[..])
        };
    }
    cx.text_system()
        .add_fonts(vec![
            font!("Geist-Regular.ttf"),
            font!("Geist-Medium.ttf"),
            font!("Geist-SemiBold.ttf"),
            font!("Geist-Bold.ttf"),
            font!("Geist-Italic.ttf"),
            font!("GeistMono-Regular.ttf"),
            font!("GeistMono-Medium.ttf"),
            font!("GeistMono-Bold.ttf"),
        ])
        .expect("bundled fonts load");
}

/// The kit, the fonts and the themes: what every window needs, headless or not.
pub fn init_ui(cx: &mut App) {
    gpui_kit::init(cx);
    load_fonts(cx);
    ui::theme::init(cx);
}

pub fn run() {
    let boot = match host::mock_from_env() {
        Some(options) => Boot::Mock(options),
        None => match host::open_core() {
            Ok(boot) => boot,
            Err(e) => {
                eprintln!("hearth: cannot open the engine: {e:#}");
                std::process::exit(1);
            }
        },
    };
    let (shutdown, close_to_tray): (platform::ShutdownHook, bool) = match &boot {
        Boot::Core(core, _) => {
            let core = core.clone();
            let close_to_tray = core.close_to_tray();
            (Arc::new(move |max| core.shutdown_gracefully(max)), close_to_tray)
        }
        Boot::Mock(_) => (Arc::new(|_| Box::pin(async {})), false),
    };

    let app = gpui_kit::application().with_assets(Assets);
    platform::on_reopen(&app); // Dock click with no window open
    let mut boot = Some(boot);
    app.run(move |cx| {
        init_ui(cx);
        platform::init(cx, platform::Config { open_main_window: Rc::new(open_main_window), shutdown, close_to_tray });
        ui::keys::init(cx);
        if let Some(boot) = boot.take() {
            Hearth::install(boot, true, cx);
        }

        // Screenshot runs must not steal focus or show up in front of other apps:
        // no Dock icon, no activation, and the window parked far off-screen
        // (the compositor still renders it, so `screencapture -l` works).
        let background = std::env::var_os("HEARTH_BACKGROUND").is_some();
        if background {
            cx.set_activation_policy(ActivationPolicy::Accessory);
        }
        open_main_window(cx);
        if !background {
            cx.activate(true);
        }
    });
}

/// A function of its own because the platform layer reopens the window after
/// close-to-tray (Windows/Linux) and on a Dock click with no window.
pub fn open_main_window(cx: &mut App) {
    let background = std::env::var_os("HEARTH_BACKGROUND").is_some();
    let hidden = std::env::var("HEARTH_BACKGROUND").is_ok_and(|v| v == "hidden");
    let bounds = if background {
        WindowBounds::Windowed(Bounds::new(point(px(-10000.), px(-10000.)), size(px(1280.), px(800.))))
    } else {
        WindowBounds::centered(size(px(1280.), px(800.)), cx)
    };
    let options = WindowOptions {
        window_bounds: Some(bounds),
        window_min_size: Some(size(px(720.), px(480.))),
        focus: !background,
        show: !hidden,
        // The desktop file's name, so Linux groups the window under the installed icon.
        app_id: Some(platform::DESKTOP_ID.into()),
        titlebar: Some(TitlebarOptions { title: Some("Hearth".into()), appears_transparent: true, traffic_light_position: Some(point(px(16.), px(16.))) }),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        platform::attach_window(window, cx); // close-to-tray / quit handling
        cx.new(|cx| ChatWindow::new(window, cx))
    })
    .expect("failed to open the window");
}
