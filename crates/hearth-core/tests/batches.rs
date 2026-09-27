//! Batches, labeled responses, history routing and server-time stamping.

mod common;

use common::*;
use hearth_core::irc::client::HistoryKind;
use hearth_core::irc::events::Event;
use hearth_core::irc::message::Message;
use hearth_core::irc::time::{now_ms, parse_ms};
use std::sync::Arc;
use tokio::task::JoinHandle;

async fn registered(caps: &str) -> (Harness, Peer) {
    let h = Harness::new().await;
    h.mgr.connect(&h.server());
    let mut p = Peer::accept(&h.listener).await;
    p.register(caps).await;
    h.event(|m| m.command == "422").await;
    (h, p)
}

type Pending = JoinHandle<anyhow::Result<Vec<Message>>>;

/// Start a history request and return it with the line the server received.
async fn request(h: &Harness, p: &mut Peer, kind: HistoryKind, target: &str, line: &str) -> (Pending, String) {
    let (mgr, target, line) = (Arc::clone(&h.mgr), target.to_string(), line.to_string());
    let prefix = line.split(' ').next().unwrap().to_string();
    let task = tokio::spawn(async move { mgr.history("t", kind, target, line).await });
    let sent = loop {
        let l = p.line().await;
        if l.contains(&prefix) {
            break l;
        }
    };
    (task, sent)
}

fn label_of(line: &str) -> String {
    line.strip_prefix("@label=").unwrap().split(' ').next().unwrap().to_string()
}

async fn result(task: Pending) -> Result<Vec<Message>, String> {
    must("the history reply", task).await.unwrap().map_err(|e| e.to_string())
}

fn texts(msgs: &[Message]) -> Vec<&str> {
    msgs.iter().map(|m| m.param(1)).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn live_messages_without_time_are_stamped() {
    let (h, mut p) = registered("server-time").await;
    let before = now_ms();
    p.send(":x!u@h PRIVMSG #a :untimed").await;
    p.send("@time=2020-02-02T02:02:02.020Z :x!u@h PRIVMSG #a :timed").await;
    let untimed = h.event(|m| m.param(1) == "untimed").await;
    let after = now_ms();
    let stamped = parse_ms(untimed.tag("time").expect("a time tag")).unwrap();
    assert!((before..=after).contains(&stamped), "{before} <= {stamped} <= {after}");
    let timed = h.event(|m| m.param(1) == "timed").await;
    assert_eq!(timed.tag("time"), Some("2020-02-02T02:02:02.020Z"));

    // The local log uses the same stamp the UI saw.
    h.db.flush();
    let logged = h.db.history("t", "#a", None, 10).unwrap();
    let logged = logged.iter().find(|m| m.param(1) == "untimed").unwrap();
    assert_eq!(logged.tag("time"), untimed.tag("time"));
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn unlabeled_history_is_matched_by_target() {
    let (h, mut p) = registered("batch draft/chathistory server-time").await;
    let (a, sent) = request(&h, &mut p, HistoryKind::Messages, "#A", "CHATHISTORY LATEST #A * 10").await;
    assert_eq!(sent, "CHATHISTORY LATEST #A * 10", "no label without labeled-response");
    let (b, _) = request(&h, &mut p, HistoryKind::Messages, "#b", "CHATHISTORY LATEST #b * 10").await;

    // Replies come back out of order, and #a's in a different case.
    p.send(":srv BATCH +1 chathistory #b").await;
    p.send("@batch=1;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #b :for b").await;
    p.send(":srv BATCH -1").await;
    p.send(":srv BATCH +2 chathistory #a").await;
    p.send("@batch=2;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #a :for a").await;
    p.send(":srv BATCH -2").await;
    assert_eq!(texts(&result(b).await.unwrap()), ["for b"]);
    assert_eq!(texts(&result(a).await.unwrap()), ["for a"]);

    // History never shows up as live traffic.
    p.assert_quiet().await;
    h.event(|m| m.command == "PING" && m.param(0) == "sync").await;
    assert_eq!(count(&h.events, |e| matches!(e, Event::Message { msg, .. } if msg.command == "PRIVMSG")), 0);
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn unlabeled_fail_is_matched_by_target() {
    let (h, mut p) = registered("batch draft/chathistory").await;
    let (a, _) = request(&h, &mut p, HistoryKind::Messages, "#a", "CHATHISTORY LATEST #a * 10").await;
    let (b, _) = request(&h, &mut p, HistoryKind::Messages, "#b", "CHATHISTORY LATEST #b * 10").await;
    p.send(":srv FAIL CHATHISTORY INVALID_TARGET LATEST #b :No such channel").await;
    assert_eq!(result(b).await.unwrap_err(), "No such channel");
    p.send(":srv BATCH +1 chathistory #a").await;
    p.send(":srv BATCH -1").await;
    assert!(result(a).await.unwrap().is_empty());
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn targets_and_search_batches_route_to_their_requests() {
    let (h, mut p) = registered("batch draft/chathistory soju.im/search").await;
    let (targets, _) =
        request(&h, &mut p, HistoryKind::Targets, "*", "CHATHISTORY TARGETS timestamp=2026-01-01T00:00:00.000Z timestamp=2026-02-01T00:00:00.000Z 50").await;
    let (search, _) = request(&h, &mut p, HistoryKind::Search, "*", "SEARCH text=hello").await;

    p.send(":srv BATCH +s soju.im/search").await;
    p.send("@batch=s;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #a :hello world").await;
    p.send(":srv BATCH -s").await;
    p.send(":srv BATCH +t draft/chathistory-targets").await;
    p.send("@batch=t :srv CHATHISTORY TARGETS #a 2026-01-05T00:00:00.000Z").await;
    p.send("@batch=t :srv CHATHISTORY TARGETS bob 2026-01-04T00:00:00.000Z").await;
    p.send(":srv BATCH -t").await;

    let found = result(search).await.unwrap();
    assert_eq!(texts(&found), ["hello world"]);
    let targets = result(targets).await.unwrap();
    assert_eq!(targets.iter().map(|m| m.param(1)).collect::<Vec<_>>(), ["#a", "bob"]);
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn requests_need_the_matching_cap() {
    let (h, mut p) = registered("batch draft/chathistory").await;
    let err = h.mgr.history("t", HistoryKind::Search, "*".into(), "SEARCH text=x".into()).await.unwrap_err();
    assert_eq!(err.to_string(), "server does not support this request");
    // Nothing went to the server.
    p.assert_quiet().await;
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn labeled_ack_completes_with_no_messages() {
    let (h, mut p) = registered("batch draft/chathistory labeled-response").await;
    let (a, sent) = request(&h, &mut p, HistoryKind::Messages, "#a", "CHATHISTORY LATEST #a * 10").await;
    p.send(&format!("@label={} :srv ACK", label_of(&sent))).await;
    assert!(result(a).await.unwrap().is_empty());
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn labeled_history_batch_without_a_wrapper() {
    let (h, mut p) = registered("batch draft/chathistory labeled-response").await;
    let (a, sent) = request(&h, &mut p, HistoryKind::Messages, "#a", "CHATHISTORY LATEST #a * 10").await;
    // An unrelated live batch opening first must not steal the reply.
    p.send(":srv BATCH +live chathistory #a").await;
    p.send(&format!("@label={} :srv BATCH +h chathistory #a", label_of(&sent))).await;
    p.send("@batch=h;time=2026-01-01T00:00:00.000Z;msgid=m1 :x!u@h PRIVMSG #a :old").await;
    p.send("@batch=live;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #a :not ours").await;
    p.send(":srv BATCH -h").await;
    p.send(":srv BATCH -live").await;
    assert_eq!(texts(&result(a).await.unwrap()), ["old"]);
    let live = h.event(|m| m.param(1) == "not ours").await;
    assert_eq!(live.tag("batch"), Some("live"));
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn fail_inside_a_history_batch_fails_the_request() {
    let (h, mut p) = registered("batch draft/chathistory labeled-response").await;
    let (a, sent) = request(&h, &mut p, HistoryKind::Messages, "#a", "CHATHISTORY LATEST #a * 10").await;
    p.send(&format!("@label={} :srv BATCH +w labeled-response", label_of(&sent))).await;
    p.send("@batch=w :srv FAIL CHATHISTORY MESSAGE_ERROR LATEST #a :Internal error").await;
    p.send(":srv BATCH -w").await;
    assert_eq!(result(a).await.unwrap_err(), "Internal error");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn nested_batches_inside_history_stay_in_the_reply() {
    let (h, mut p) = registered("batch draft/chathistory labeled-response draft/multiline").await;
    let (a, sent) = request(&h, &mut p, HistoryKind::Messages, "#a", "CHATHISTORY LATEST #a * 10").await;
    p.send(&format!("@label={} :srv BATCH +w labeled-response", label_of(&sent))).await;
    p.send("@batch=w :srv BATCH +h chathistory #a").await;
    p.send("@batch=h;msgid=1;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #a :before").await;
    p.send("@batch=h;msgid=2;time=2026-01-01T00:00:01.000Z :x!u@h BATCH +ml draft/multiline #a").await;
    p.send("@batch=ml :x!u@h PRIVMSG #a :line one").await;
    p.send("@batch=ml :x!u@h PRIVMSG #a :line two").await;
    // The inner batch ending must not complete the request early.
    p.send("@batch=h :x!u@h BATCH -ml").await;
    p.send("@batch=h;msgid=3;time=2026-01-01T00:00:02.000Z :x!u@h PRIVMSG #a :after").await;
    p.send("@batch=w :srv BATCH -h").await;
    p.send(":srv BATCH -w").await;

    let got = result(a).await.unwrap();
    assert_eq!(texts(&got), ["before", "line one\nline two", "after"], "the multiline batch reads as one message");
    assert_eq!(got[1].tag("msgid"), Some("2"));
    p.assert_quiet().await;
    h.event(|m| m.command == "PING" && m.param(0) == "sync").await;
    assert_eq!(
        count(&h.events, |e| matches!(e, Event::Message { msg, .. } if msg.command == "PRIVMSG")),
        0,
        "history leaked out as live messages"
    );
    h.db.flush();
    let logged = h.db.history("t", "#a", None, 10).unwrap();
    assert_eq!(texts(&logged), ["before", "line one\nline two", "after"]);
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_requests_fail_when_the_link_drops() {
    let (h, mut p) = registered("batch draft/chathistory").await;
    let (a, _) = request(&h, &mut p, HistoryKind::Messages, "#a", "CHATHISTORY LATEST #a * 10").await;
    drop(p);
    assert_eq!(result(a).await.unwrap_err(), "disconnected");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn frontend_labels_follow_nested_batches() {
    let (h, mut p) = registered("batch labeled-response message-tags").await;
    h.mgr.send("t", vec!["@label=f1 WHO #a".into()]).unwrap();
    p.expect("@label=f1 WHO").await;
    p.send("@label=f1 :srv BATCH +r labeled-response").await;
    p.send("@batch=r :srv BATCH +n netsplit a.net b.net").await;
    p.send("@batch=n :x!u@h QUIT :a.net b.net").await;
    p.send("@batch=r :srv BATCH -n").await;
    p.send("@batch=r :srv 315 me #a :End of WHO").await;
    p.send(":srv BATCH -r").await;
    p.send(":y!u@h QUIT :bye").await;

    assert_eq!(h.event(|m| m.command == "QUIT" && m.param(0) == "a.net b.net").await.tag("label"), Some("f1"));
    assert_eq!(h.event(|m| m.command == "315").await.tag("label"), Some("f1"));
    assert_eq!(h.event(|m| m.param(0) == "bye").await.tag("label"), None, "label ends with the batch");

    // An ACK for a frontend label reaches the UI.
    h.mgr.send("t", vec!["@label=f2 MODE #a +m".into()]).unwrap();
    p.expect("@label=f2").await;
    p.send("@label=f2 :srv ACK").await;
    assert_eq!(h.event(|m| m.command == "ACK").await.tag("label"), Some("f2"));
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn unrelated_batches_pass_through() {
    let (h, mut p) = registered("batch").await;
    p.send(":srv BATCH +n netsplit a.net b.net").await;
    p.send("@batch=n :x!u@h QUIT :a.net b.net").await;
    p.send(":srv BATCH -n").await;
    let quit = h.event(|m| m.command == "QUIT").await;
    assert_eq!(quit.tag("batch"), Some("n"));
    assert_eq!(quit.tag("label"), None);
    h.event(|m| m.command == "BATCH" && m.param(0) == "-n").await;
    h.mgr.disconnect_all();
}
