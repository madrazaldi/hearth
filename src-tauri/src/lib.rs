use hearth_core::certs::CertInfo;
use hearth_core::config::{Config, ServerConfig};
use hearth_core::irc::message::Message;
use hearth_core::preview::Preview;
use hearth_core::store::{SavedBuffer, SearchHit};
use hearth_core::Core;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter as _, Manager as _, State};

// Thin wrappers: the command API lives in hearth_core so the macOS app can
// share it. Argument names here are what the frontend's `invoke` calls use.

type CmdResult<T> = Result<T, String>;

#[tauri::command]
fn get_config(core: State<Core>) -> Config {
    core.get_config()
}

#[tauri::command]
fn save_server(core: State<Core>, server: ServerConfig) -> CmdResult<ServerConfig> {
    core.save_server(server)
}

#[tauri::command]
fn delete_server(core: State<Core>, id: String) -> CmdResult<()> {
    core.delete_server(&id)
}

#[tauri::command]
fn save_settings(core: State<Core>, settings: serde_json::Value) -> CmdResult<()> {
    core.save_settings(settings)
}

#[tauri::command]
fn connect(core: State<Core>, server: String) -> CmdResult<()> {
    core.connect(&server)
}

#[tauri::command]
fn disconnect(core: State<Core>, server: String, reason: Option<String>) {
    core.disconnect(&server, reason);
}

#[tauri::command]
fn reconnect(core: State<Core>, server: String) {
    core.reconnect(&server);
}

#[tauri::command]
fn reset(core: State<Core>) {
    core.reset();
}

#[tauri::command]
fn send(core: State<Core>, conn: String, lines: Vec<String>) -> CmdResult<()> {
    core.send(&conn, lines)
}

#[tauri::command]
async fn history(core: State<'_, Core>, conn: String, kind: String, target: String, line: String) -> CmdResult<Vec<Message>> {
    core.history(&conn, &kind, target, line).await
}

#[tauri::command]
async fn local_history(
    core: State<'_, Core>,
    conn: String,
    target: String,
    before: Option<i64>,
    after: Option<i64>,
    limit: usize,
) -> CmdResult<Vec<Message>> {
    core.local_history(conn, target, before, after, limit).await
}

#[tauri::command]
async fn search(
    core: State<'_, Core>,
    query: String,
    network: Option<String>,
    target: Option<String>,
    limit: usize,
) -> CmdResult<Vec<SearchHit>> {
    core.search(query, network, target, limit).await
}

#[tauri::command]
fn load_buffers(core: State<Core>, conn: String) -> CmdResult<Vec<SavedBuffer>> {
    core.load_buffers(&conn)
}

#[tauri::command]
fn save_buffers(core: State<Core>, conn: String, buffers: Vec<SavedBuffer>) -> CmdResult<()> {
    core.save_buffers(&conn, &buffers)
}

#[tauri::command]
async fn clear_logs(core: State<'_, Core>) -> CmdResult<()> {
    core.clear_logs().await
}

#[tauri::command]
async fn link_preview(core: State<'_, Core>, url: String) -> CmdResult<Preview> {
    core.link_preview(&url).await
}

#[tauri::command]
async fn avatar(core: State<'_, Core>, url: String) -> CmdResult<String> {
    core.avatar(&url).await
}

#[tauri::command]
fn generate_cert(core: State<Core>, server: String) -> CmdResult<CertInfo> {
    core.generate_cert(&server)
}

#[tauri::command]
fn cert_info(core: State<Core>, path: String) -> CmdResult<CertInfo> {
    core.cert_info(&path)
}

#[tauri::command]
async fn upload_file(core: State<'_, Core>, server: String, url: String, path: String) -> CmdResult<String> {
    core.upload_file(&server, &url, &path).await
}

#[tauri::command]
async fn upload_bytes(
    core: State<'_, Core>,
    server: String,
    url: String,
    name: String,
    content_type: String,
    data: Vec<u8>,
) -> CmdResult<String> {
    core.upload_bytes(&server, &url, &name, content_type, data).await
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
    // One runtime for Tauri and the connection actors.
    tauri::async_runtime::set(hearth_core::runtime::handle().clone());
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
            let handle = app.handle().clone();
            let core = Core::open(&app.path().app_config_dir()?, &app.path().app_data_dir()?, move |events| {
                if let Err(e) = handle.emit("irc", events) {
                    log::error!("emit failed: {e}");
                }
            })?;
            app.manage(core);
            if let Err(e) = build_tray(app.handle()) {
                log::warn!("could not create tray icon: {e}");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.state::<Core>().close_to_tray() {
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
                app.state::<Core>().shutdown();
            }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_main(app),
            _ => {}
        });
}
