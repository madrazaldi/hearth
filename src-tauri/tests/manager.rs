//! The connection manager: soju bouncer children, send limits, and actor
//! lifetimes.

mod common;

use common::*;
use hearth_lib::irc::events::Event;
use hearth_lib::irc::manager::child_id;

const BOUNCER_CAPS: &str = "batch soju.im/bouncer-networks soju.im/bouncer-networks-notify";

fn added(e: &Event, id: &str) -> bool {
    matches!(e, Event::ConnAdded { conn, .. } if conn == id)
}

fn removed(e: &Event, id: &str) -> bool {
    matches!(e, Event::ConnRemoved { conn } if conn == id)
}

/// Register a bouncer child: it binds to its network before ending CAP.
async fn register_child(p: &mut Peer, netid: &str) {
    p.expect("CAP LS").await;
    p.send(&format!(":srv CAP * LS :{BOUNCER_CAPS}")).await;
    let req = p.expect("CAP REQ").await;
    assert!(!req.contains("bouncer-networks-notify"), "children don't need network updates: {req}");
    p.send(&format!(":srv CAP * ACK :{}", req.split_once(" :").unwrap().1)).await;
    assert_eq!(p.line().await, format!("BOUNCER BIND {netid}"));
    assert_eq!(p.line().await, "CAP END");
    p.welcome().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bouncer_networks_come_and_go_live() {
    let h = Harness::new().await;
    let mut server = h.server();
    server.autojoin = vec!["#a".into()];
    h.mgr.connect(&server);

    let mut control = Peer::accept(&h.listener).await;
    control.expect("CAP LS").await;
    control.send(&format!(":srv CAP * LS :{BOUNCER_CAPS}")).await;
    let req = control.expect("CAP REQ").await;
    assert!(req.contains("soju.im/bouncer-networks-notify"), "{req}");
    control.send(&format!(":srv CAP * ACK :{}", req.split_once(" :").unwrap().1)).await;
    assert_eq!(control.line().await, "CAP END");
    control.welcome().await;
    // The control connection joins nothing; each network does its own.
    control.assert_quiet().await;

    control.send("@batch=b :srv BOUNCER NETWORK 42 name=Libera;state=connected").await;
    let mut child = Peer::accept(&h.listener).await;
    let ev = wait_event(&h.events, "the child to be added", |e| added(e, "t/42")).await;
    let Event::ConnAdded { server, parent, netid, .. } = ev else { unreachable!() };
    assert_eq!((server.as_str(), parent.as_deref(), netid.as_deref()), ("t", Some("t"), Some("42")));
    register_child(&mut child, "42").await;
    // The bouncer restores channels, so the child doesn't autojoin either.
    child.assert_quiet().await;

    // Attribute updates for a known network don't spawn a second child.
    control.send(":srv BOUNCER NETWORK 42 state=disconnected").await;
    control.assert_quiet().await;
    assert!(Peer::try_accept(&h.listener, ms(200)).await.is_none());
    assert_eq!(count(&h.events, |e| added(e, "t/42")), 1);

    // Children log under their own id; the control connection logs nothing.
    child.send(":x!u@h PRIVMSG #chan :on libera").await;
    control.send(":x!u@h PRIVMSG #chan :on control").await;
    child.assert_quiet().await;
    control.assert_quiet().await;
    h.db.flush();
    assert_eq!(h.db.history("t/42", "#chan", None, 10).unwrap().len(), 1);
    assert!(h.db.history("t", "#chan", None, 10).unwrap().is_empty());

    // Only the control connection manages networks.
    child.send(":srv BOUNCER NETWORK 99 name=Other").await;
    child.assert_quiet().await;
    assert!(Peer::try_accept(&h.listener, ms(200)).await.is_none());

    // Removal quits the child exactly once.
    control.send(":srv BOUNCER NETWORK 42 *").await;
    assert_eq!(child.closed().await, vec!["QUIT :Hearth"]);
    wait_event(&h.events, "the child to be removed", |e| removed(e, "t/42")).await;
    control.assert_quiet().await;
    real_sleep(ms(50)).await;
    assert_eq!(count(&h.events, |e| removed(e, "t/42")), 1);
    assert!(h.mgr.send("t/42", vec!["PRIVMSG #a :x".into()]).is_err());

    // Re-adding it brings up a fresh child.
    control.send(":srv BOUNCER NETWORK 42 name=Libera").await;
    let mut child = Peer::accept(&h.listener).await;
    register_child(&mut child, "42").await;

    // Disconnecting the server takes its children down too.
    h.mgr.disconnect("t", Some("bye".into()));
    assert_eq!(control.closed().await, vec!["QUIT :bye"]);
    assert_eq!(child.closed().await.last().map(String::as_str), Some("QUIT :bye"));
}

#[test]
fn child_ids_nest_under_the_server() {
    assert_eq!(child_id("srv", "42"), "srv/42");
}

#[tokio::test(flavor = "multi_thread")]
async fn sends_are_sanitised_and_bounded() {
    let h = Harness::new().await;
    h.mgr.connect(&h.server());
    let mut p = Peer::accept(&h.listener).await;
    p.register("").await;
    h.event(|m| m.command == "422").await;

    assert_eq!(h.mgr.send("nope", vec!["PING x".into()]).unwrap_err().to_string(), "no such connection");

    // Embedded line breaks can't smuggle in a second command; blank lines are dropped.
    h.mgr.send("t", vec!["PRIVMSG #a :one\r\nQUIT :x".into(), "".into(), "   ".into(), "PRIVMSG #a :two".into()]).unwrap();
    assert_eq!(p.line().await, "PRIVMSG #a :one  QUIT :x");
    assert_eq!(p.line().await, "PRIVMSG #a :two");

    // A flood far beyond what the pacing can drain is refused, not queued.
    let flood: Vec<String> = (0..400).map(|i| format!("PRIVMSG #a :{i}")).collect();
    h.mgr.send("t", flood).unwrap();
    let refused = must("the backlog cap to kick in", async {
        loop {
            match h.mgr.send("t", vec!["PRIVMSG #a :more".into()]) {
                Err(e) => return e.to_string(),
                Ok(()) => real_sleep(ms(5)).await,
            }
        }
    })
    .await;
    assert!(refused.contains("too many lines"), "{refused}");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_restarts_the_session() {
    let h = Harness::new().await;
    h.mgr.connect(&h.server());
    let mut p = Peer::accept(&h.listener).await;
    p.register("").await;
    h.event(|m| m.command == "422").await;
    h.mgr.reconnect("t");
    assert_eq!(p.closed().await, vec!["QUIT :Reconnecting"]);
    let mut p = Peer::accept(&h.listener).await;
    p.expect("CAP LS").await;
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_actor_leaves_its_successor_alone() {
    let h = Harness::new().await;
    h.mgr.connect(&h.server());
    let mut old = Peer::accept(&h.listener).await;
    old.expect("USER").await;

    // Replace the connection before the old actor has wound down.
    h.mgr.disconnect("t", None);
    h.mgr.connect(&h.server());
    let mut new = Peer::accept(&h.listener).await;
    assert_eq!(old.closed().await, vec!["QUIT :Hearth"]);
    new.register("").await;
    h.event(|m| m.command == "422").await;

    real_sleep(ms(50)).await;
    assert_eq!(count(&h.events, |e| removed(e, "t")), 1, "only the explicit disconnect");
    h.mgr.send("t", vec!["PRIVMSG #a :still here".into()]).unwrap();
    assert_eq!(new.line().await, "PRIVMSG #a :still here");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn connecting_twice_keeps_one_connection() {
    let h = Harness::new().await;
    h.mgr.connect(&h.server());
    h.mgr.connect(&h.server());
    let mut p = Peer::accept(&h.listener).await;
    p.expect("USER").await;
    assert!(Peer::try_accept(&h.listener, ms(200)).await.is_none());
    assert_eq!(count(&h.events, |e| added(e, "t")), 1);
    h.mgr.disconnect_all();
}
