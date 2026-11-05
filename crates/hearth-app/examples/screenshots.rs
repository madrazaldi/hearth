//! Regenerates the README screenshots in `docs/screenshots` from the scripted
//! mock. Nothing is shown on screen: each scene is drawn by GPUI's headless
//! renderer into an image. Run it with `cargo xtask screenshots`.

use gpui_kit::*;
use hearth_app::boot::{Assets, init_ui};
use hearth_app::host::{Boot, Hearth, hearth};
use hearth_app::ui::{keys, shell::ChatWindow};
use hearth_model::mock::{MockOptions, View};
use hearth_model::types::{Layout, Theme};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const SIZE: (f32, f32) = (1280., 800.);
/// The mock replays its session over about this long (model time).
const SETTLE_MS: i64 = 4000;
const STEP_MS: i64 = 50;

struct Shot {
    name: &'static str,
    options: fn() -> MockOptions,
    /// Rows to scroll the message list up, so the shot shows conversation rather than media.
    scroll: usize,
}

fn base() -> MockOptions {
    MockOptions::default()
}

fn light(o: MockOptions) -> MockOptions {
    MockOptions { theme: Theme::Light, ..o }
}

fn shots() -> Vec<Shot> {
    vec![
        Shot { name: "chat-dark", options: base, scroll: 24 },
        Shot { name: "chat-light", options: || MockOptions { theme: Theme::Light, ..base() }, scroll: 24 },
        Shot { name: "columns", options: || MockOptions { layout: Layout::Columns, ..base() }, scroll: 24 },
        Shot { name: "search", options: || MockOptions { view: Some(View::Search), ..base() }, scroll: 0 },
        Shot { name: "settings", options: || MockOptions { view: Some(View::Settings), ..base() }, scroll: 0 },
        Shot { name: "switcher", options: || MockOptions { view: Some(View::Switcher), ..base() }, scroll: 0 },
        Shot { name: "channels", options: || MockOptions { view: Some(View::Channels), ..base() }, scroll: 0 },
    ]
}

/// Extra scenes for visual review (`screenshots <dir> extras`); they are not README material.
fn extras() -> Vec<Shot> {
    macro_rules! tab {
        ($t:literal) => {
            || MockOptions { view: Some(View::Settings), tab: Some($t.into()), ..base() }
        };
    }
    vec![
        Shot { name: "x-settings-general", options: tab!("general"), scroll: 0 },
        Shot { name: "x-settings-notifications", options: tab!("notifications"), scroll: 0 },
        Shot { name: "x-settings-aliases", options: tab!("aliases"), scroll: 0 },
        Shot { name: "x-settings-privacy", options: tab!("privacy"), scroll: 0 },
        Shot { name: "x-settings-shortcuts", options: tab!("shortcuts"), scroll: 0 },
        Shot { name: "x-settings-servers", options: tab!("servers"), scroll: 0 },
        Shot {
            name: "x-settings-server-edit",
            options: || MockOptions { view: Some(View::Settings), tab: Some("servers".into()), server: Some("soju".into()), ..base() },
            scroll: 0,
        },
        Shot { name: "x-settings-light", options: || light(MockOptions { view: Some(View::Settings), ..base() }), scroll: 0 },
        Shot { name: "x-search-light", options: || light(MockOptions { view: Some(View::Search), ..base() }), scroll: 0 },
        Shot { name: "x-switcher-light", options: || light(MockOptions { view: Some(View::Switcher), ..base() }), scroll: 0 },
        Shot { name: "x-channels-light", options: || light(MockOptions { view: Some(View::Channels), ..base() }), scroll: 0 },
        Shot { name: "x-shortcuts", options: || MockOptions { view: Some(View::Shortcuts), ..base() }, scroll: 0 },
        Shot { name: "x-shortcuts-light", options: || light(MockOptions { view: Some(View::Shortcuts), ..base() }), scroll: 0 },
        Shot { name: "x-compact", options: || MockOptions { layout: Layout::Compact, ..base() }, scroll: 24 },
        Shot { name: "x-compact-light", options: || light(MockOptions { layout: Layout::Compact, ..base() }), scroll: 24 },
        Shot { name: "x-columns-light", options: || light(MockOptions { layout: Layout::Columns, ..base() }), scroll: 24 },
        Shot { name: "x-bottom-dark", options: base, scroll: 0 },
        Shot { name: "x-bottom-light", options: || light(base()), scroll: 0 },
        Shot { name: "x-scrolled-dark", options: base, scroll: 60 },
        Shot { name: "x-welcome", options: || MockOptions { empty: true, ..base() }, scroll: 0 },
        Shot { name: "x-welcome-light", options: || light(MockOptions { empty: true, ..base() }), scroll: 0 },
        Shot { name: "x-query", options: || MockOptions { buf: Some("mira".into()), ..base() }, scroll: 0 },
        Shot { name: "x-stress", options: || MockOptions { buf: Some("#hearth-design".into()), ..base() }, scroll: 0 },
    ]
}

/// Move the model's clock and the executor's together, so the mock's script and the debounces run.
fn advance(cx: &mut HeadlessAppContext, ms: i64) {
    for _ in 0..ms / STEP_MS {
        cx.update(|app| {
            let host = hearth(app);
            host.update(app, |h, cx| {
                h.app.set_now(h.app.now() + STEP_MS);
                h.tick(cx);
            });
        });
        cx.advance_clock(Duration::from_millis(STEP_MS as u64));
        cx.run_until_parked();
    }
}

fn render(shot: &Shot, out: &Path) -> anyhow::Result<()> {
    let text_system = gpui_kit::platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(Assets), gpui_kit::platform::current_headless_renderer);
    cx.allow_parking();
    let options = (shot.options)();
    let (window, chat) = cx.update(|app| {
        init_ui(app);
        keys::init(app);
        Hearth::install(Boot::Mock(options), false, app);
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(SIZE.0), px(SIZE.1)));
        gpui_kit::open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), focus: false, show: false, ..Default::default() },
            app,
            |window, cx| cx.new(|cx| ChatWindow::new(window, cx)),
        )
    })?;
    advance(&mut cx, SETTLE_MS);
    if shot.scroll > 0 {
        cx.update(|app| chat.update(app, |c: &mut ChatWindow, cx| c.scroll_messages_up(shot.scroll, cx)));
        advance(&mut cx, 2 * STEP_MS);
    }
    let image = cx.capture_screenshot(window)?;
    let path = out.join(format!("{}.png", shot.name));
    image.save(&path)?;
    println!("{} ({}x{})", path.display(), image.width(), image.height());
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let out = args.next().map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots"));
    let only = args.next();
    std::fs::create_dir_all(&out)?;
    match only.as_deref() {
        Some("gaps") => return gap_scenes(&out),
        Some("welcome") => return welcome_scenes(&out),
        _ => {}
    }
    let list = if only.as_deref() == Some("extras") { extras() } else { shots() };
    for shot in list.iter().filter(|s| only.as_deref().is_none_or(|o| o == "extras" || o == s.name)) {
        render(shot, &out)?;
    }
    Ok(())
}

// ── Scenes for features beyond the README set (written to a scratch directory) ──

/// One window kept open while the scenes change it.
struct Scene {
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    chat: Entity<ChatWindow>,
    out: PathBuf,
}

impl Scene {
    fn new(options: MockOptions, out: &Path) -> anyhow::Result<Scene> {
        let text_system = gpui_kit::platform::current_platform(true).text_system();
        let mut cx = HeadlessAppContext::with_platform(text_system, Arc::new(Assets), gpui_kit::platform::current_headless_renderer);
        cx.allow_parking();
        let (window, chat) = cx.update(|app| {
            init_ui(app);
            keys::init(app);
            Hearth::install(Boot::Mock(options), false, app);
            let bounds = Bounds::new(point(px(0.), px(0.)), size(px(SIZE.0), px(SIZE.1)));
            gpui_kit::open_window(
                WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), focus: false, show: false, ..Default::default() },
                app,
                |window, cx| cx.new(|cx| ChatWindow::new(window, cx)),
            )
        })?;
        advance(&mut cx, SETTLE_MS);
        std::fs::create_dir_all(out)?;
        Ok(Scene { cx, window, chat, out: out.to_path_buf() })
    }

    fn shot(&mut self, name: &str) -> anyhow::Result<()> {
        advance(&mut self.cx, 2 * STEP_MS);
        let image = self.cx.capture_screenshot(self.window)?;
        let path = self.out.join(format!("{name}.png"));
        image.save(&path)?;
        println!("{} ({}x{})", path.display(), image.width(), image.height());
        Ok(())
    }

    fn chat(&mut self, f: impl FnOnce(&mut ChatWindow, &mut Window, &mut Context<ChatWindow>)) {
        let chat = self.chat.clone();
        self.cx.update_window(self.window, |_, window, cx| chat.update(cx, |c, cx| f(c, window, cx))).unwrap();
    }
}

/// `screenshots <dir> gaps`: the message filter, the upload spinner and the drop target.
fn gap_scenes(out: &Path) -> anyhow::Result<()> {
    let mut s = Scene::new(base(), out)?;
    s.chat(|c, w, cx| c.set_message_filter("oskar", w, cx));
    s.shot("filter")?;
    s.chat(|c, w, cx| c.set_message_filter("no such thing", w, cx));
    s.shot("filter-empty")?;
    s.chat(|c, w, cx| c.set_message_filter("", w, cx));

    // An upload in flight (the mock answers on the next pump, so it is not pumped here).
    s.cx.update(|app| {
        let host = hearth(app);
        host.update(app, |h, cx| {
            if let Some(id) = h.app.active().cloned() {
                h.app.upload_bytes(&id, "clip.png", "image/png", vec![1, 2, 3]);
            }
            cx.notify();
        });
    });
    s.shot("uploading")?;

    // Files dragged over the window.
    s.cx.update_window(s.window, |_, window, cx| {
        use gpui_kit::{ExternalPaths, FileDropEvent, InputEvent as _};
        let paths = ExternalPaths([PathBuf::from("/tmp/screenshot.png")].into_iter().collect());
        window.dispatch_event(FileDropEvent::Entered { position: point(px(700.), px(400.)), paths }.to_platform_input(), cx);
    })?;
    s.shot("drop")?;
    Ok(())
}

impl Scene {
    fn welcome(&mut self, f: impl FnOnce(&mut hearth_app::ui::welcome::Welcome, &mut Window, &mut Context<hearth_app::ui::welcome::Welcome>)) {
        let w = self.cx.update(|app| self.chat.read(app).welcome()).expect("the first-run flow is showing");
        self.cx.update_window(self.window, |_, window, cx| w.update(cx, |w, cx| f(w, window, cx))).unwrap();
    }

    fn wait(&mut self, ms: i64) {
        advance(&mut self.cx, ms);
    }
}

/// `screenshots <dir> welcome`: every step of the first-run flow, from an empty config.
fn welcome_scenes(out: &Path) -> anyhow::Result<()> {
    let empty = || MockOptions { empty: true, ..base() };
    let mut s = Scene::new(empty(), out)?;
    s.shot("welcome-1-network")?;
    for (name, step) in [
        ("2-nick", "nick"),
        ("3-nick-error", "nick-error"),
        ("4-account", "account"),
        ("5-channels", "channels"),
        ("6-bouncer", "bouncer"),
        ("7-custom", "custom"),
    ] {
        s.welcome(|w, win, cx| w.preview(step, win, cx));
        s.wait(100);
        s.shot(&format!("welcome-{name}"))?;
    }

    // A real run against the scripted server: libera, nick "ember", no account.
    s.welcome(|w, win, cx| w.preview("channels", win, cx));
    s.welcome(|w, win, cx| w.submit(win, cx));
    s.wait(900);
    s.shot("welcome-8-connecting")?;
    s.wait(3000);
    s.shot("welcome-9-connected")?;

    // The server turns the account login down but lets the connection through.
    s.cx.update(|app| {
        use hearth_model::types::IrcEvent;
        let host = hearth(app);
        host.update(app, |h, cx| {
            let conn = h.app.welcome.saved_id.clone();
            let msg = hearth_model::testing::parse(":irc.libera.chat 904 ember :SASL authentication failed");
            h.act(cx, |a| a.handle_events(&[IrcEvent::Message { conn, msg }]));
        });
    });
    s.shot("welcome-10-connected-sasl-failed")?;

    // An address that does not resolve.
    let mut s = Scene::new(empty(), out)?;
    s.welcome(|w, win, cx| w.preview("connect", win, cx));
    s.cx.update(|app| {
        hearth(app).update(app, |h, cx| {
            h.act(cx, |a| {
                let cfg = hearth_model::types::ServerConfig {
                    name: "Nowhere".into(),
                    host: "irc.nowhere.invalid".into(),
                    port: 6697,
                    tls: true,
                    nick: "ember".into(),
                    ..Default::default()
                };
                a.welcome_connect(Some(cfg))
            })
        })
    });
    s.wait(4000);
    s.shot("welcome-11-failed")?;
    Ok(())
}
