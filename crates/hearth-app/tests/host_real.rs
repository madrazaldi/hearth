//! The host against the real engine: connect to a scripted IRC server on
//! localhost, join, receive, send, and find the history again after a restart.
//!
//! Alone in its file because it points the config and data directories at a
//! temporary folder through the environment for the whole process. No server
//! has a password, so the system keychain is never touched.

use gpui_kit::TestAppContext;
use hearth_app::host::{Hearth, hearth, host, open_core};
use hearth_model::buffer::BufId;
use hearth_model::types::{ConnStatus, ServerConfig};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A just-enough IRC server: registers anyone, lets them join `#test`, greets with one message from alice, records what clients send.
struct Server {
    port: u16,
    received: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let received = Arc::new(Mutex::new(Vec::new()));
        let log = received.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let log = log.clone();
                std::thread::spawn(move || serve(stream, log));
            }
        });
        Server { port, received }
    }

    fn said(&self, line: &str) -> bool {
        self.received.lock().unwrap().iter().any(|l| l == line)
    }
}

fn serve(stream: TcpStream, log: Arc<Mutex<Vec<String>>>) {
    let mut out = stream.try_clone().unwrap();
    let mut send = move |line: String| {
        let _ = out.write_all(format!("{line}\r\n").as_bytes());
    };
    let mut nick = String::from("*");
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        let (cmd, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        match cmd {
            "CAP" if rest.starts_with("LS") => send(":srv CAP * LS :".into()),
            "CAP" if rest.starts_with("END") => {
                send(format!(":srv 001 {nick} :Welcome"));
                send(format!(":srv 005 {nick} CHANTYPES=# PREFIX=(ov)@+ NETWORK=Testnet :are supported by this server"));
                send(format!(":srv 376 {nick} :End of MOTD"));
            }
            "NICK" => nick = rest.to_string(),
            "JOIN" => {
                send(format!(":{nick}!u@h JOIN {rest}"));
                send(format!(":srv 353 {nick} = {rest} :{nick} alice"));
                send(format!(":srv 366 {nick} {rest} :End of NAMES"));
                send(format!(":alice!a@h PRIVMSG {rest} :hello from alice"));
            }
            "PING" => send(format!(":srv PONG srv {rest}")),
            "PRIVMSG" => log.lock().unwrap().push(line.clone()),
            _ => {}
        }
    }
}

/// Run the app's executor and the wall clock until `cond` holds.
fn wait(cx: &TestAppContext, what: &str, cond: impl Fn(&Hearth) -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        cx.run_until_parked();
        if cx.update(|app| cond(host(app))) {
            return;
        }
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
        cx.executor().advance_clock(Duration::from_millis(10));
    }
}

fn texts(h: &Hearth, conn: &str) -> Vec<String> {
    h.app.conn(conn).and_then(|c| c.buffer("#test")).map(|b| b.lines().iter().map(|l| l.text.clone()).collect()).unwrap_or_default()
}

fn boot(cx: &TestAppContext) {
    cx.executor().allow_parking();
    cx.update(|app| {
        Hearth::install(open_core().expect("engine opens in the temp directories"), false, app);
    });
}

#[test]
fn connects_chats_and_remembers_across_a_restart() {
    let base = std::env::temp_dir().join(format!("hearth-host-{}", std::process::id()));
    let (config, data) = (base.join("config"), base.join("data"));
    // SAFETY: nothing else in this process reads the environment concurrently.
    unsafe {
        std::env::set_var("HEARTH_CONFIG_DIR", &config);
        std::env::set_var("HEARTH_DATA_DIR", &data);
    }
    let server = Server::start();

    // First run: add the server through the model, as the welcome flow does.
    let cx = TestAppContext::single();
    boot(&cx);
    let cfg = ServerConfig {
        name: "Testnet".into(),
        host: "127.0.0.1".into(),
        port: server.port,
        tls: false,
        nick: "tester".into(),
        username: "tester".into(),
        realname: "Test".into(),
        autojoin: vec!["#test".into()],
        ..ServerConfig::default()
    };
    cx.update(|app| hearth(app).update(app, |h, cx| h.act(cx, |app| app.save_server(cfg, true))));
    wait(&cx, "the server to be saved", |h| !h.app.servers.is_empty());
    let id = cx.update(|app| host(app).app.servers[0].id.clone());
    assert!(!id.is_empty(), "the engine assigns an id");

    wait(&cx, "registration", |h| h.app.conn(&id).is_some_and(|c| c.status == ConnStatus::Connected));
    wait(&cx, "the join and alice's greeting", |h| texts(h, &id).iter().any(|t| t == "hello from alice"));

    let buf = BufId::new(id.clone(), "#test");
    cx.update(|app| hearth(app).update(app, |h, cx| h.act(cx, |app| app.run_input(&buf, "hi from the test", None))));
    wait(&cx, "our message to reach the server", |_| server.said("PRIVMSG #test :hi from the test"));
    wait(&cx, "our message to echo into the buffer", |h| texts(h, &id).iter().any(|t| t == "hi from the test"));

    // Quit the way the app does: QUIT goes out and the log is flushed and closed.
    cx.update(|app| host(app).core().expect("real mode").close());
    drop(cx);
    assert!(data.join("hearth.db").is_file());
    assert!(config.join("config.json").is_file());

    // Second run on the same directories: the saved server reconnects on its own
    // and the earlier conversation is read back from hearth.db.
    let cx = TestAppContext::single();
    boot(&cx);
    wait(&cx, "the saved server to reconnect", |h| h.app.conn(&id).is_some_and(|c| c.status == ConnStatus::Connected));
    let buf = BufId::new(id.clone(), "#test");
    cx.update(|app| hearth(app).update(app, |h, cx| h.act(cx, |app| app.open(&buf))));
    wait(&cx, "history from the local log", |h| {
        let lines = texts(h, &id);
        lines.iter().any(|t| t == "hello from alice") && lines.iter().any(|t| t == "hi from the test")
    });
    cx.update(|app| host(app).core().expect("real mode").close());
    drop(cx);
    let _ = std::fs::remove_dir_all(base);
}
