//! IRCv3 extensions the actor handles itself: draft/multiline batches,
//! draft/metadata-2 subscriptions, draft/channel-rename logs and cap values.

mod common;

use common::*;
use hearth_lib::irc::events::Event;

async fn registered(caps: &str) -> (Harness, Peer) {
    let h = Harness::new().await;
    h.mgr.connect(&h.server());
    let mut p = Peer::accept(&h.listener).await;
    p.register(caps).await;
    h.event(|m| m.command == "422").await;
    (h, p)
}

fn privmsgs(h: &Harness) -> usize {
    count(&h.events, |e| matches!(e, Event::Message { msg, .. } if msg.command == "PRIVMSG"))
}

#[tokio::test(flavor = "multi_thread")]
async fn multiline_batches_arrive_as_one_message() {
    let (h, mut p) = registered("batch draft/multiline message-tags").await;
    p.send("@msgid=m1;time=2026-01-01T00:00:00.000Z;+draft/reply=p0 :x!u@h BATCH +ml draft/multiline #a").await;
    p.send("@batch=ml :x!u@h PRIVMSG #a :first line").await;
    p.send("@batch=ml :x!u@h PRIVMSG #a :second ").await;
    p.send("@batch=ml;draft/multiline-concat :x!u@h PRIVMSG #a :line, continued").await;
    p.send("@batch=ml :x!u@h PRIVMSG #a :").await;
    p.send("@batch=ml :x!u@h PRIVMSG #a :after a blank").await;
    p.send(":x!u@h BATCH -ml").await;

    let m = h.event(|m| m.command == "PRIVMSG").await;
    assert_eq!(m.params, ["#a", "first line\nsecond line, continued\n\nafter a blank"]);
    assert_eq!(m.tag("msgid"), Some("m1"));
    assert_eq!(m.tag("time"), Some("2026-01-01T00:00:00.000Z"));
    assert_eq!(m.tag("+draft/reply"), Some("p0"));
    assert_eq!(m.tag("batch"), None);
    assert_eq!(m.source.as_ref().unwrap().nick, "x");
    p.assert_quiet().await;
    h.event(|m| m.command == "PING" && m.param(0) == "sync").await;
    assert_eq!(privmsgs(&h), 1, "the parts must not leak out one by one");
    assert_eq!(count(&h.events, |e| matches!(e, Event::Message { msg, .. } if msg.command == "BATCH")), 0);

    h.db.flush();
    let logged = h.db.history("t", "#a", None, 10).unwrap();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].param(1), "first line\nsecond line, continued\n\nafter a blank");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn echoed_multiline_keeps_its_label() {
    let (h, mut p) = registered("batch draft/multiline echo-message labeled-response message-tags").await;
    h.mgr
        .send(
            "t",
            vec![
                "@label=f1 BATCH +out draft/multiline #a".into(),
                "@batch=out PRIVMSG #a :one".into(),
                "@batch=out PRIVMSG #a :two".into(),
                "BATCH -out".into(),
            ],
        )
        .unwrap();
    p.expect("BATCH -out").await;
    // The label sits on the echoed batch itself...
    p.send("@label=f1;msgid=e1 :me!u@h BATCH +e draft/multiline #a").await;
    p.send("@batch=e :me!u@h PRIVMSG #a :one").await;
    p.send("@batch=e :me!u@h PRIVMSG #a :two").await;
    p.send(":me!u@h BATCH -e").await;
    let m = h.event(|m| m.tag("msgid") == Some("e1")).await;
    assert_eq!((m.param(1), m.tag("label")), ("one\ntwo", Some("f1")));

    // ...or on a labeled-response batch around it.
    p.send("@label=f2 :srv BATCH +r labeled-response").await;
    p.send("@batch=r;msgid=e2 :me!u@h BATCH +e2 draft/multiline #a").await;
    p.send("@batch=e2 :me!u@h PRIVMSG #a :three").await;
    p.send("@batch=e2 :me!u@h PRIVMSG #a :four").await;
    p.send("@batch=r :me!u@h BATCH -e2").await;
    p.send(":srv BATCH -r").await;
    let m = h.event(|m| m.tag("msgid") == Some("e2")).await;
    assert_eq!((m.param(1), m.tag("label")), ("three\nfour", Some("f2")));
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn outgoing_multiline_is_logged_as_one_message() {
    let (h, mut p) = registered("batch draft/multiline").await;
    h.mgr
        .send(
            "t",
            vec![
                "BATCH +out draft/multiline #a".into(),
                "@batch=out PRIVMSG #a :hello".into(),
                "@batch=out;draft/multiline-concat PRIVMSG #a : world".into(),
                "@batch=out PRIVMSG #a :bye".into(),
                "BATCH -out".into(),
            ],
        )
        .unwrap();
    p.expect("BATCH -out").await;
    h.db.flush();
    let logged = h.db.history("t", "#a", None, 10).unwrap();
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert_eq!(logged[0].param(1), "hello world\nbye");
    assert_eq!(logged[0].source.as_ref().unwrap().nick, "me");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn metadata_keys_are_subscribed_after_registration() {
    let h = Harness::new().await;
    h.mgr.connect(&h.server());
    let mut p = Peer::accept(&h.listener).await;
    p.expect("CAP LS").await;
    p.send(":srv CAP * LS :draft/metadata-2=before-connect,max-subs=10 draft/multiline=max-bytes=4096,max-lines=24").await;
    p.expect("CAP REQ").await;
    p.send(":srv CAP * ACK :draft/metadata-2 draft/multiline").await;
    p.expect("CAP END").await;
    p.welcome().await;
    assert_eq!(p.expect("METADATA").await, "METADATA * SUB avatar display-name");

    // The frontend needs the multiline limits to build batches.
    let values = match wait_event(&h.events, "registration", |e| matches!(e, Event::Registered { .. })).await {
        Event::Registered { cap_values, .. } => cap_values,
        _ => unreachable!(),
    };
    assert_eq!(values.get("draft/multiline").map(String::as_str), Some("max-bytes=4096,max-lines=24"));
    assert_eq!(values.get("draft/metadata-2").map(String::as_str), Some("before-connect,max-subs=10"));
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn no_metadata_subscription_without_the_cap() {
    let (h, mut p) = registered("batch").await;
    p.assert_quiet().await;
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn renamed_channels_take_their_log_along() {
    let (h, mut p) = registered("draft/channel-rename").await;
    p.send(":x!u@h PRIVMSG #Old :before").await;
    p.send(":op!u@h RENAME #Old #New :tidying up").await;
    p.send(":x!u@h PRIVMSG #new :after").await;
    let rename = h.event(|m| m.command == "RENAME").await;
    assert_eq!(rename.params, ["#Old", "#New", "tidying up"]);
    h.event(|m| m.param(1) == "after").await;
    h.db.flush();
    assert!(h.db.history("t", "#old", None, 10).unwrap().is_empty());
    let logged = h.db.history("t", "#new", None, 10).unwrap();
    assert_eq!(logged.iter().map(|m| m.param(1)).collect::<Vec<_>>(), ["before", "after"]);
    h.mgr.disconnect_all();
}
