pub mod config;
pub mod irc;
mod upload;

use config::{Config, SaslMechanism, ServerConfig, Store};
use irc::client::HistoryKind;
use irc::events::Emitter;
use irc::manager::Manager;
use irc::message::Message;
use std::sync::{Arc, Mutex};
use tauri::{Emitter as _, Manager as _, State};

struct AppState {
    store: Store,
    config: Mutex<Config>,
    irc: Arc<Manager>,
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
    if server.sasl == SaslMechanism::None {
        server.sasl_password.clear();
    }
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
    state.persist()
}

#[tauri::command]
fn save_settings(state: State<AppState>, settings: serde_json::Value) -> CmdResult<()> {
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
    let kind = if kind == "targets" { HistoryKind::Targets } else { HistoryKind::Messages };
    state.irc.history(&conn, kind, target, line).await.map_err(err)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(if cfg!(debug_assertions) { log::LevelFilter::Info } else { log::LevelFilter::Warn })
                .build(),
        )
        .setup(|app| {
            let dir = app.path().app_config_dir()?;
            let store = Store::new(&dir);
            let config = store.load();
            let handle = app.handle().clone();
            let irc = Manager::new(Emitter::spawn(move |events| {
                if let Err(e) = handle.emit("irc", events) {
                    log::error!("emit failed: {e}");
                }
            }));
            app.manage(AppState { store, config: Mutex::new(config), irc });
            Ok(())
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
            upload_file,
            upload_bytes,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Hearth")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                app.state::<AppState>().irc.disconnect_all();
            }
        });
}
