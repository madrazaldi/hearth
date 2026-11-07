//! The host: the one place the client state (`hearth_model::App`) meets the
//! engine (`hearth_core::Core`), the OS and the clock.
//!
//! [`Hearth`] is a global GPUI entity created once at startup. It outlives any
//! window, which matters where closing to the tray destroys the window.
//!
//! * Events from the engine arrive on a channel, one batch per burst, and are
//!   fed to the model by a foreground task.
//! * [`Hearth::pump`] drains the model's effects and performs them: commands go
//!   to the engine, requests are awaited in tasks and answered with
//!   `App::resolve`, the rest goes to the OS.
//! * Timers: a single task sleeps until `App::next_wakeup` and ticks the model.
//! * In mock mode a scripted backend answers instead of the engine.

pub mod images;
pub mod updates;

use async_channel::Receiver;
use gpui_kit::{App as GpuiApp, AppContext as _, ClipboardEntry, ClipboardItem, Context, Entity, Global, Task, WeakEntity};
use hearth_core::irc::events::Event;
use hearth_core::{CmdResult, Core, Pending};
use hearth_model::BufId;
use hearth_model::mock::{MockBackend, MockOptions};
use hearth_model::state::ToastKind;
use hearth_model::{App, Effect, Reply, ReqId};
use images::{Ask, Images};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;
use updates::Updates;

enum Backend {
    Real(Core),
    Mock(Box<MockBackend>),
}

pub struct Hearth {
    pub app: App,
    backend: Backend,
    pub images: Rc<Images>,
    pub updates: Updates,
    /// Tray, dock badge and the rest of the OS integration are up (not in tests or screenshots).
    platform: bool,
    last_rev: u64,
    close_to_tray: bool,
    wake: Option<(i64, Task<()>)>,
    /// Remembers the open buffer between runs.
    remember: Option<PathBuf>,
}

/// What the host runs against.
pub enum Boot {
    /// The engine, with the receiving end of the channel its event sink feeds.
    Core(Core, Receiver<Vec<Event>>),
    /// The scripted demo.
    Mock(MockOptions),
}

/// The entity the windows read, kept as a global so a window created later (after a close to the tray) finds it.
pub struct HearthHandle(pub Entity<Hearth>);

impl Global for HearthHandle {}

/// The running host.
pub fn hearth(cx: &GpuiApp) -> Entity<Hearth> {
    cx.global::<HearthHandle>().0.clone()
}

/// Read access without cloning the handle, for views that read rows while painting.
pub fn host(cx: &GpuiApp) -> &Hearth {
    cx.global::<HearthHandle>().0.read(cx)
}

/// Ask the model for the pictures that painting found missing. Deferred, because
/// painting cannot change the model.
pub fn request_flush(cx: &mut GpuiApp) {
    let entity = hearth(cx);
    cx.defer(move |cx| entity.update(cx, |h, cx| h.flush_asks(cx)));
}

/// Opens the engine in the user's directories (`HEARTH_CONFIG_DIR`/`HEARTH_DATA_DIR` move them).
pub fn open_core() -> anyhow::Result<Boot> {
    let (tx, rx) = async_channel::unbounded::<Vec<Event>>();
    let core = Core::open_default(move |events| drop(tx.try_send(events.to_vec())))?;
    Ok(Boot::Core(core, rx))
}

/// `HEARTH_MOCK=1` or `--mock` asks for the scripted demo; the `HEARTH_*` view variables shape it.
pub fn mock_from_env() -> Option<MockOptions> {
    if std::env::var_os("HEARTH_MOCK").is_none() && !std::env::args().any(|a| a == "--mock") {
        return None;
    }
    use hearth_model::mock::{Stress, View};
    use hearth_model::types::{Layout, Theme};
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    Some(MockOptions {
        empty: std::env::var_os("HEARTH_EMPTY").is_some(),
        avatars: std::env::var_os("HEARTH_NOAVATARS").is_none(),
        stress: match var("HEARTH_STRESS").as_deref() {
            None => Stress::Off,
            Some("live") => Stress::Live,
            Some(_) => Stress::On,
        },
        theme: match var("HEARTH_THEME").as_deref() {
            Some("light") => Theme::Light,
            Some("system") => Theme::System,
            _ => Theme::Dark,
        },
        layout: match var("HEARTH_LAYOUT").as_deref() {
            Some("columns") => Layout::Columns,
            Some("compact") => Layout::Compact,
            _ => Layout::Cozy,
        },
        view: match var("HEARTH_VIEW").as_deref() {
            Some("switcher") => Some(View::Switcher),
            Some("settings") => Some(View::Settings),
            Some("shortcuts") => Some(View::Shortcuts),
            Some("search") => Some(View::Search),
            Some("channels") => Some(View::Channels),
            _ => None,
        },
        buf: var("HEARTH_BUFFER"),
        query: var("HEARTH_QUERY"),
        tab: var("HEARTH_TAB"),
        server: var("HEARTH_SERVER"),
    })
}

/// The file that remembers which buffer was open.
fn remember_path() -> PathBuf {
    hearth_core::paths::app_dirs().data.join("active-buffer")
}

impl Hearth {
    /// Create the host and put it in the global slot. `platform` turns on the
    /// OS integration (badge, notifications) that needs `platform::init` to have run.
    pub fn install(boot: Boot, platform: bool, cx: &mut GpuiApp) -> Entity<Hearth> {
        let entity = cx.new(|cx| Hearth::new(boot, platform, cx));
        cx.set_global(HearthHandle(entity.clone()));
        entity
    }

    fn new(boot: Boot, platform: bool, cx: &mut Context<Self>) -> Hearth {
        let mut app = App::new();
        let mut remember = None;
        let (backend, events, scripted) = match boot {
            Boot::Core(core, rx) => {
                let path = remember_path();
                let key = std::fs::read_to_string(&path).ok().map(|s| s.trim().to_string());
                app.init(core.get_config(), key);
                remember = Some(path);
                (Backend::Real(core), Some(rx), false)
            }
            Boot::Mock(opts) => (Backend::Mock(Box::new(hearth_model::mock::start(&mut app, opts))), None, true),
        };
        let mut host = Hearth {
            app,
            backend,
            images: Rc::new(Images::default()),
            updates: Updates::new(scripted),
            platform,
            last_rev: u64::MAX,
            close_to_tray: false,
            wake: None,
            remember,
        };
        if let Some(rx) = events {
            cx.spawn(async move |this: WeakEntity<Hearth>, cx| {
                while let Ok(batch) = rx.recv().await {
                    if this.update(cx, |h, cx| h.on_events(&batch, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        host.pump(cx);
        host.start_update_checks(cx);
        host
    }

    pub fn core(&self) -> Option<&Core> {
        match &self.backend {
            Backend::Real(core) => Some(core),
            Backend::Mock(_) => None,
        }
    }

    /// Upload the files at `paths` (directories are skipped); each link lands in the buffer's composer.
    pub fn upload_paths(&mut self, id: &BufId, paths: &[PathBuf], cx: &mut Context<Self>) {
        self.act(cx, |app| {
            for p in paths.iter().filter(|p| p.is_file()) {
                app.upload_file(id, &p.to_string_lossy());
            }
        });
    }

    /// Upload what a paste carries when it is a picture or files rather than text. False when it is
    /// just text, which the field pastes itself.
    pub fn upload_clipboard(&mut self, id: &BufId, item: &ClipboardItem, cx: &mut Context<Self>) -> bool {
        let mut taken = false;
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    let name = format!("pasted.{}", image.format.extension());
                    self.act(cx, |app| app.upload_bytes(id, &name, image.format.mime_type(), image.bytes.clone()));
                    taken = true;
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    self.upload_paths(id, &paths.0, cx);
                    taken = true;
                }
                ClipboardEntry::String(_) => {}
            }
        }
        taken
    }

    /// Request the avatars and previews that views asked for while painting.
    pub fn flush_asks(&mut self, cx: &mut Context<Self>) {
        let asks = self.images.take_asks();
        if asks.is_empty() {
            return;
        }
        self.act(cx, |app| {
            for ask in asks {
                match ask {
                    Ask::Avatar(url) => app.request_avatar(&url),
                    Ask::Preview(url) => app.request_preview(&url),
                }
            }
        });
    }

    fn on_events(&mut self, events: &[Event], cx: &mut Context<Self>) {
        self.app.handle_events(events);
        self.pump(cx);
    }

    /// Change the model, then do what that asked for. Every UI action goes through here.
    pub fn act<R>(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut App) -> R) -> R {
        let r = f(&mut self.app);
        self.pump(cx);
        r
    }

    /// Hand the engine a command the model has no method for (connect, disconnect).
    pub fn emit(&mut self, effect: Effect, cx: &mut Context<Self>) {
        self.app.env().emit(effect);
        self.pump(cx);
    }

    /// Run due timers (and the scripted backend's), then [`Hearth::pump`].
    pub fn tick(&mut self, cx: &mut Context<Self>) {
        self.app.tick();
        self.pump(cx);
    }

    /// Perform what the model asked for, redraw if it changed, and set the next wake-up.
    pub fn pump(&mut self, cx: &mut Context<Self>) {
        for _ in 0..16 {
            let effects = match &mut self.backend {
                Backend::Mock(mock) => mock.drive(&mut self.app),
                Backend::Real(_) => self.app.take_effects(),
            };
            if effects.is_empty() {
                break;
            }
            for effect in effects {
                self.execute(effect, cx);
            }
        }
        let rev = self.app.revision();
        if rev != self.last_rev {
            self.last_rev = rev;
            self.settings_changed(cx);
            cx.notify();
        }
        self.schedule(cx);
    }

    fn settings_changed(&mut self, cx: &mut Context<Self>) {
        let on = self.app.settings.close_to_tray;
        if on != self.close_to_tray {
            self.close_to_tray = on;
            if self.platform {
                crate::platform::set_close_to_tray(cx, on);
            }
        }
    }

    /// Sleep until the model (or the scripted backend) next has work.
    fn schedule(&mut self, cx: &mut Context<Self>) {
        let mock = match &self.backend {
            Backend::Mock(m) => m.next_wakeup(),
            Backend::Real(_) => None,
        };
        let next = [self.app.next_wakeup(), mock].into_iter().flatten().min();
        let Some(at) = next else {
            self.wake = None;
            return;
        };
        if self.wake.as_ref().is_some_and(|(t, _)| *t == at) {
            return;
        }
        let delay = (at - self.app.now()).clamp(1, 60_000) as u64;
        let task = cx.spawn(async move |this: WeakEntity<Hearth>, cx| {
            cx.background_executor().timer(Duration::from_millis(delay)).await;
            let _ = this.update(cx, |h, cx| {
                h.wake = None;
                h.tick(cx);
            });
        });
        self.wake = Some((at, task));
    }

    // ── Effects ──────────────────────────────────────────────

    /// The scripted backend answers everything but the OS effects, so past `host_effect` only the engine is left.
    fn execute(&mut self, effect: Effect, cx: &mut Context<Self>) {
        if self.host_effect(&effect, cx) {
            return;
        }
        let Backend::Real(core) = &self.backend else { return };
        let core = core.clone();
        match effect {
            Effect::Send { conn, lines } => {
                if let Err(e) = core.send(&conn, lines) {
                    self.app.send_failed(&conn, &e);
                }
            }
            Effect::Connect { server } => {
                if let Err(e) = core.connect(&server) {
                    self.app.toast_error(format!("Couldn't connect: {e}"));
                }
            }
            Effect::Disconnect { server, reason } => core.disconnect(&server, reason),
            Effect::Reconnect { server } => core.reconnect(&server),
            Effect::Reset => core.reset(),
            Effect::DeleteServer { id } => self.spawn_unit(core.delete_server(&id), "Couldn't delete the server", cx),
            Effect::SaveSettings { settings } => {
                if let Err(e) = core.save_settings(settings.to_value()) {
                    self.app.toast_error(format!("Couldn't save settings: {e}"));
                }
            }
            Effect::SaveBuffers { conn, buffers } => self.spawn_unit(core.save_buffers(&conn, buffers), "Couldn't save the buffer list", cx),
            Effect::SaveServer { req, server } => self.spawn_request(req, core.save_server(server.0), Reply::Server, cx),
            Effect::LoadBuffers { req, conn } => self.spawn_request(req, core.load_buffers(&conn), Reply::Buffers, cx),
            Effect::History { req, conn, kind, target, line } => self.spawn_request(req, core.history(&conn, kind.as_str(), target, line), Reply::Messages, cx),
            Effect::LocalHistory { req, conn, target, before, after, limit } => {
                self.spawn_request(req, core.local_history(&conn, &target, before, after, limit), Reply::Messages, cx)
            }
            Effect::Search { req, query } => self.spawn_request(req, core.search(query), Reply::Hits, cx),
            Effect::ClearLogs { req } => self.spawn_request(req, core.clear_logs(), |()| Reply::Unit, cx),
            Effect::LinkPreview { req, url } => self.spawn_request(req, core.link_preview(&url), Reply::Preview, cx),
            Effect::Avatar { req, url } => self.spawn_request(req, core.avatar(&url), Reply::Text, cx),
            Effect::GenerateCert { req, server } => self.app.resolve(req, core.generate_cert(&server).map(Reply::Cert)),
            Effect::CertInfo { req, path } => self.app.resolve(req, core.cert_info(&path).map(Reply::Cert)),
            Effect::UploadBytes { req, server, url, name, content_type, data } => {
                self.spawn_request(req, core.upload_bytes(&server, &url, &name, &content_type, data.0), Reply::Text, cx)
            }
            Effect::UploadFile { req, server, url, path } => self.spawn_request(req, core.upload_file(&server, &url, &path), Reply::Text, cx),
            Effect::Notify { .. } | Effect::SetBadge { .. } | Effect::OpenUrl { .. } | Effect::CopyText { .. } | Effect::RememberActive { .. } => {}
        }
    }

    /// Effects for the OS; true if `effect` was one.
    fn host_effect(&mut self, effect: &Effect, cx: &mut Context<Self>) -> bool {
        match effect {
            Effect::Notify { title, body } => {
                if self.platform {
                    crate::platform::notify(title, body);
                }
            }
            Effect::SetBadge { count } => {
                if self.platform {
                    crate::platform::set_badge(cx, *count);
                }
            }
            Effect::OpenUrl { url } => {
                if let Err(e) = crate::platform::open_url(cx, url) {
                    self.app.toast_error(e);
                }
            }
            Effect::CopyText { text } => cx.write_to_clipboard(ClipboardItem::new_string(text.clone())),
            Effect::RememberActive { key } => {
                if let Some(path) = &self.remember {
                    let _ = std::fs::write(path, key);
                }
            }
            _ => return false,
        }
        true
    }

    /// Await a request in the engine and answer the model with the result.
    fn spawn_request<T: Send + 'static>(&mut self, req: ReqId, fut: Pending<CmdResult<T>>, wrap: fn(T) -> Reply, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Hearth>, cx| {
            let result = fut.await.map(wrap);
            let _ = this.update(cx, |h, cx| {
                h.app.resolve(req, result);
                h.pump(cx);
            });
        })
        .detach();
    }

    /// Await a command whose only news is a failure, which is shown as a toast.
    fn spawn_unit(&mut self, fut: Pending<CmdResult<()>>, what: &'static str, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Hearth>, cx| {
            if let Err(e) = fut.await {
                let _ = this.update(cx, |h, cx| {
                    h.app.toast(format!("{what}: {e}"), ToastKind::Error);
                    h.pump(cx);
                });
            }
        })
        .detach();
    }
}
