//! `Core` as the desktop app uses it: from threads and executors that are
//! not tokio's. The tests here are plain `#[test]`s with a hand-made
//! `block_on`, so a method that needed a tokio context would panic.

use hearth_core::config::ServerConfig;
use hearth_core::irc::events::{Event, Status};
use hearth_core::secrets::MemoryKeychain;
use hearth_core::Core;
use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

struct Unpark(std::thread::Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// The smallest executor there is.
fn block_on<F: Future>(fut: F) -> F::Output {
    assert!(tokio::runtime::Handle::try_current().is_err(), "this test must run outside tokio");
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => {
                assert!(Instant::now() < deadline, "the future never finished");
                std::thread::park_timeout(Duration::from_millis(50));
            }
        }
    }
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("hearth-api-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn open(dir: &TempDir) -> (Core, mpsc::Receiver<Event>, Arc<MemoryKeychain>) {
    let vault = Arc::new(MemoryKeychain::default());
    let (tx, rx) = mpsc::channel();
    let core = Core::open_with_keychain(dir.path(), dir.path(), vault.clone(), move |evs| {
        for e in evs {
            let _ = tx.send(e.clone());
        }
    })
    .unwrap();
    (core, rx, vault)
}

fn expect_event(rx: &mpsc::Receiver<Event>, what: &str, pred: impl Fn(&Event) -> bool) -> Event {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(e) if pred(&e) => return e,
            Ok(_) => {}
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

fn line(r: &mut BufReader<TcpStream>) -> String {
    let mut s = String::new();
    r.read_line(&mut s).unwrap();
    s.trim_end().to_string()
}

#[test]
fn the_whole_command_api_works_without_tokio() {
    let dir = TempDir::new();
    let (core, events, vault) = open(&dir);

    // A scripted server on a plain thread.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (sock, _) = listener.accept().unwrap();
        let mut w = sock.try_clone().unwrap();
        let mut r = BufReader::new(sock);
        while !line(&mut r).starts_with("CAP LS") {}
        w.write_all(b":srv CAP * LS :\r\n").unwrap();
        while line(&mut r) != "CAP END" {}
        w.write_all(b":srv 001 me :Welcome\r\n:srv 422 me :No MOTD\r\n").unwrap();
        let mut seen = Vec::new();
        loop {
            let l = line(&mut r);
            if l.is_empty() {
                break;
            }
            seen.push(l);
        }
        seen
    });

    let saved = block_on(core.save_server(ServerConfig {
        id: "t".into(),
        host: "127.0.0.1".into(),
        port,
        tls: false,
        nick: "me".into(),
        password: "pw".into(),
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(saved.name, "127.0.0.1");
    assert_eq!(vault.len(), 1, "the password went to the keychain");
    assert_eq!(core.get_config().servers.len(), 1);

    core.reset();
    core.connect("t").unwrap();
    expect_event(&events, "the connection to be added", |e| matches!(e, Event::ConnAdded { conn, .. } if conn == "t"));
    expect_event(&events, "the connection to be up", |e| matches!(e, Event::Status { status: Status::Connected, .. }));

    core.send("t", vec!["PRIVMSG #a :hello".into()]).unwrap();
    assert!(core.send("nope", vec!["PING x".into()]).is_err());
    let err = block_on(core.history("nope", "messages", "#a".into(), "CHATHISTORY LATEST #a * 10".into())).unwrap_err();
    assert_eq!(err, "no such connection");

    // The log is readable too, including what was just logged.
    let hits = block_on(core.search(hearth_core::store::SearchQuery { text: "hello".into(), limit: 10, ..Default::default() })).unwrap();
    assert_eq!(hits.len(), 1);
    let history = block_on(core.local_history("t", "#a", None, None, 10)).unwrap();
    assert_eq!(history.len(), 1);

    // Leaving: the QUIT reaches the server before the future resolves.
    block_on(core.shutdown_gracefully(Duration::from_secs(5)));
    let seen = server.join().unwrap();
    assert!(seen.contains(&"PRIVMSG #a :hello".to_string()), "{seen:?}");
    assert_eq!(seen.last().map(String::as_str), Some("QUIT :Hearth"));
    core.close();
}

#[test]
fn work_continues_when_its_future_is_dropped() {
    let dir = TempDir::new();
    let (core, _events, vault) = open(&dir);
    drop(core.save_server(ServerConfig { id: "x".into(), host: "h".into(), nick: "n".into(), password: "pw".into(), ..Default::default() }));
    let deadline = Instant::now() + Duration::from_secs(10);
    while core.get_config().servers.is_empty() || vault.is_empty() {
        assert!(Instant::now() < deadline, "the save never happened");
        std::thread::sleep(Duration::from_millis(5));
    }
    core.close();
}

#[test]
fn handles_share_one_engine_across_threads() {
    let dir = TempDir::new();
    let (core, _events, _vault) = open(&dir);
    let threads: Vec<_> = (0..4)
        .map(|i| {
            let core = core.clone();
            std::thread::spawn(move || {
                for n in 0..5 {
                    let name = format!("s{i}x{n}");
                    block_on(core.save_server(ServerConfig { id: name.clone(), host: "h".into(), nick: "n".into(), ..Default::default() })).unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(core.get_config().servers.len(), 20);
    // What is on disk is what is in memory.
    let on_disk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("config.json")).unwrap()).unwrap();
    assert_eq!(on_disk["servers"].as_array().unwrap().len(), 20);
    core.close();
}
