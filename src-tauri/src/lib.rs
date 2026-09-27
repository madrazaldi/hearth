pub mod config;
pub mod irc;
pub mod preview;
pub mod store;
mod certs;
mod secrets;
mod upload;

use config::{Config, SaslMechanism, ServerConfig, Store};
use irc::client::HistoryKind;
use irc::events::Emitter;
use irc::manager::Manager;
use irc::message::Message;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use store::{Db, SavedBuffer, SearchHit};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter as _, Manager as _, State};

struct AppState {
    store: Store,
    config: Mutex<Config>,
    irc: Arc<Manager>,
    previews: preview::Previewer,
    close_to_tray: AtomicBool,
    load_avatars: AtomicBool,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl AppState {
    fn server(&self, id: &str) -> CmdResult<ServerConfig> {
        let cfg = self.config.lock().unwrap();
        cfg.servers.iter().find(|s| s.id == id).cloned().ok_or_else(|| "unknown server".into())
    }

    fn persist(&self) -> CmdResult<()> {
        let cfg = self.config.lock().unwrap().clone();
        self.store.save(&cfg).map_err(err)
    }

    fn db(&self) -> &Arc<Db> {
        self.irc.db()
    }

    /// Settings the backend acts on itself.
    fn apply_settings(&self, settings: &serde_json::Value) {
        let flag = |k: &str, d: bool| settings.get(k).and_then(serde_json::Value::as_bool).unwrap_or(d);
        self.db().set_logging(flag("storeLogs", true));
        self.close_to_tray.store(flag("closeToTray", false), Ordering::Relaxed);
        self.load_avatars.store(flag("loadAvatars", false), Ordering::Relaxed);
    }
}

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn save_server(state: State<AppState>, mut server: ServerConfig) -> CmdResult<ServerConfig> {
    if server.host.trim().is_empty() {
        return Err("host is required".into());
    }
    if server.id.is_empty() {
        server.id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    }
    if server.name.trim().is_empty() {
        server.name = server.host.clone();
    }
    if server.sasl != SaslMechanism::Plain {
        server.sasl_password.clear();
    }
    secrets::store(&mut server);
    {
        let mut cfg = state.config.lock().unwrap();
        match cfg.servers.iter_mut().find(|s| s.id == server.id) {
            Some(existing) => *existing = server.clone(),
            None => cfg.servers.push(server.clone()),
        }
    }
    state.persist()?;
    Ok(server)
}

#[tauri::command]
fn delete_server(state: State<AppState>, id: String) -> CmdResult<()> {
    state.irc.disconnect(&id, None);
    state.config.lock().unwrap().servers.retain(|s| s.id != id);
    secrets::forget(&id);
    if let Err(e) = state.db().forget_server(&id) {
        log::warn!("could not delete logs for {id}: {e}");
    }
    state.persist()
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: serde_json::Value) -> CmdResult<()> {
    state.apply_settings(&settings);
    state.config.lock().unwrap().settings = settings;
    state.persist()
}

#[tauri::command]
fn connect(state: State<AppState>, server: String) -> CmdResult<()> {
    let cfg = state.server(&server)?;
    state.irc.connect(&cfg);
    Ok(())
}

#[tauri::command]
fn disconnect(state: State<AppState>, server: String, reason: Option<String>) {
    state.irc.disconnect(&server, reason);
}

#[tauri::command]
fn reconnect(state: State<AppState>, server: String) {
    state.irc.reconnect(&server);
}

/// Called when the UI (re)loads: drop every live connection so the frontend
/// rebuilds its state from a clean registration burst.
#[tauri::command]
fn reset(state: State<AppState>) {
    state.irc.disconnect_all();
}

#[tauri::command]
fn send(state: State<AppState>, conn: String, lines: Vec<String>) -> CmdResult<()> {
    state.irc.send(&conn, lines).map_err(err)
}

#[tauri::command]
async fn history(
    state: State<'_, AppState>,
    conn: String,
    kind: String,
    target: String,
    line: String,
) -> CmdResult<Vec<Message>> {
    let kind = match kind.as_str() {
        "targets" => HistoryKind::Targets,
        "search" => HistoryKind::Search,
        _ => HistoryKind::Messages,
    };
    state.irc.history(&conn, kind, target, line).await.map_err(err)
}

/// Messages from the local log. `target` is the case-folded buffer name.
#[tauri::command]
async fn local_history(
    state: State<'_, AppState>,
    conn: String,
    target: String,
    before: Option<i64>,
    after: Option<i64>,
    limit: usize,
) -> CmdResult<Vec<Message>> {
    let db = state.db().clone();
    let limit = limit.clamp(1, 1000);
    tauri::async_runtime::spawn_blocking(move || match after {
        Some(t) => db.history_after(&conn, &target, t, limit),
        None => db.history(&conn, &target, before, limit),
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
async fn search(
    state: State<'_, AppState>,
    query: String,
    network: Option<String>,
    target: Option<String>,
    limit: usize,
) -> CmdResult<Vec<SearchHit>> {
    let db = state.db().clone();
    tauri::async_runtime::spawn_blocking(move || db.search(&query, network.as_deref(), target.as_deref(), limit.clamp(1, 500)))
        .await
        .map_err(err)?
        .map_err(err)
}

#[tauri::command]
fn load_buffers(state: State<AppState>, conn: String) -> CmdResult<Vec<SavedBuffer>> {
    state.db().buffers(&conn).map_err(err)
}

#[tauri::command]
fn save_buffers(state: State<AppState>, conn: String, buffers: Vec<SavedBuffer>) -> CmdResult<()> {
    state.db().save_buffers(&conn, &buffers, |s| s.to_lowercase()).map_err(err)
}

#[tauri::command]
async fn clear_logs(state: State<'_, AppState>) -> CmdResult<()> {
    let db = state.db().clone();
    tauri::async_runtime::spawn_blocking(move || db.clear_logs()).await.map_err(err)?.map_err(err)
}

#[tauri::command]
async fn link_preview(state: State<'_, AppState>, url: String) -> CmdResult<preview::Preview> {
    state.previews.preview(&url).await.map_err(err)
}

/// A draft/metadata-2 avatar as a data: URL. The webview's CSP forbids
/// remote images, so the backend fetches it (public addresses only).
#[tauri::command]
async fn avatar(state: State<'_, AppState>, url: String) -> CmdResult<String> {
    if !state.load_avatars.load(Ordering::Relaxed) {
        return Err("avatars are turned off".into());
    }
    state.previews.avatar(&url).await.map_err(err)
}

#[tauri::command]
fn generate_cert(app: AppHandle, server: String) -> CmdResult<certs::CertInfo> {
    let dir = app.path().app_config_dir().map_err(err)?.join("certs");
    certs::generate(&dir, &server).map_err(err)
}

#[tauri::command]
fn cert_info(path: String) -> CmdResult<certs::CertInfo> {
    certs::info(&path).map_err(err)
}

#[tauri::command]
async fn upload_file(state: State<'_, AppState>, server: String, url: String, path: String) -> CmdResult<String> {
    let cfg = state.server(&server)?;
    let body = tokio::fs::read(&path).await.map_err(err)?;
    let name = std::path::Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    upload::upload(&cfg, &url, &name, upload::guess_type(&name), body).await.map_err(err)
}

#[tauri::command]
async fn upload_bytes(
    state: State<'_, AppState>,
    server: String,
    url: String,
    name: String,
    content_type: String,
    data: Vec<u8>,
) -> CmdResult<String> {
    let cfg = state.server(&server)?;
    let ct = if content_type.is_empty() { upload::guess_type(&name).to_string() } else { content_type };
    upload::upload(&cfg, &url, &name, &ct, data).await.map_err(err)
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show Hearth", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Hearth", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Hearth")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone()).icon_as_template(false);
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(if cfg!(debug_assertions) { log::LevelFilter::Info } else { log::LevelFilter::Warn })
                .build(),
        )
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let store = Store::new(&dir);
            let config = store.load();
            let db = Arc::new(Db::open(&app.path().app_data_dir()?.join("hearth.db"))?);
            let handle = app.handle().clone();
            let irc = Manager::new(
                Emitter::spawn(move |events| {
                    if let Err(e) = handle.emit("irc", events) {
                        log::error!("emit failed: {e}");
                    }
                }),
                db,
            );
            let state = AppState {
                store,
                config: Mutex::new(config.clone()),
                irc,
                previews: preview::Previewer::new(),
                close_to_tray: AtomicBool::new(false),
                load_avatars: AtomicBool::new(false),
            };
            state.apply_settings(&config.settings);
            app.manage(state);
            if let Err(e) = build_tray(app.handle()) {
                log::warn!("could not create tray icon: {e}");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.state::<AppState>().close_to_tray.load(Ordering::Relaxed) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_server,
            delete_server,
            save_settings,
            connect,
            disconnect,
            reconnect,
            reset,
            send,
            history,
            local_history,
            search,
            load_buffers,
            save_buffers,
            clear_logs,
            link_preview,
            avatar,
            generate_cert,
            cert_info,
            upload_file,
            upload_bytes,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Hearth")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { .. } => {
                let state = app.state::<AppState>();
                state.irc.disconnect_all();
                state.db().flush();
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_main(app),
            _ => {}
        });
}
