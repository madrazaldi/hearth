//! Headless smoke test: connect to a server and print every event.
//! HEARTH_HOST=... HEARTH_PASS=... cargo run --example probe

use hearth_lib::config::{SaslMechanism, ServerConfig};
use hearth_lib::irc::client::HistoryKind;
use hearth_lib::irc::events::Emitter;
use hearth_lib::irc::manager::Manager;
use std::time::Duration;

#[tokio::main]
async fn main() {
    let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
    let server = ServerConfig {
        id: "probe".into(),
        name: "probe".into(),
        host: env("HEARTH_HOST", "irc.libera.chat"),
        port: env("HEARTH_PORT", "6697").parse().unwrap(),
        tls: env("HEARTH_TLS", "1") == "1",
        websocket: env("HEARTH_WS", "0") == "1",
        websocket_path: env("HEARTH_WS_PATH", "/"),
        nick: env("HEARTH_NICK", "hearthprobe"),
        sasl: if std::env::var("HEARTH_PASS").is_ok() { SaslMechanism::Plain } else { SaslMechanism::None },
        sasl_username: env("HEARTH_NICK", "hearthprobe"),
        sasl_password: env("HEARTH_PASS", ""),
        ..Default::default()
    };
    let verbose = std::env::var("VERBOSE").is_ok();
    let mgr = Manager::new(Emitter::spawn(move |events| {
        for e in events {
            let s = serde_json::to_string(e).unwrap();
            if verbose || !s.contains("\"type\":\"message\"") {
                println!("{}", &s[..s.len().min(300)]);
            }
        }
    }));
    mgr.connect(&server);
    tokio::time::sleep(Duration::from_secs(6)).await;
    if let Ok(target) = std::env::var("HEARTH_HISTORY") {
        let conn = env("HEARTH_HISTORY_CONN", "probe");
        let line = format!("CHATHISTORY LATEST {target} * 20");
        match mgr.history(&conn, HistoryKind::Messages, target.clone(), line).await {
            Ok(msgs) => println!("history: {} messages, last: {:?}", msgs.len(), msgs.last().map(|m| m.serialize())),
            Err(e) => println!("history error: {e}"),
        }
        let t = mgr
            .history(&conn, HistoryKind::Targets, "*".into(), "CHATHISTORY TARGETS timestamp=2020-01-01T00:00:00Z timestamp=2100-01-01T00:00:00Z 50".into())
            .await;
        println!("targets: {:?}", t.map(|v| v.iter().map(|m| m.param(1).to_string()).collect::<Vec<_>>()));
    }
    mgr.disconnect_all();
    tokio::time::sleep(Duration::from_millis(500)).await;
}
