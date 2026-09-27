//! Drives a real connection actor against a scripted IRC server on localhost.

mod common;

use common::*;
use hearth_core::config::SaslMechanism;
use hearth_core::irc::client::HistoryKind;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

#[tokio::test(flavor = "multi_thread")]
async fn scram_falls_back_to_plain() {
    let h = Harness::new().await;
    let mut server = h.server();
    server.sasl = SaslMechanism::Plain;
    server.sasl_username = "acct".into();
    server.sasl_password = "pw".into();
    h.mgr.connect(&server);

    let mut p = Peer::accept(&h.listener).await;
    p.expect("CAP LS").await;
    p.send(":srv CAP * LS :sasl=PLAIN,SCRAM-SHA-256 server-time").await;
    let req = p.expect("CAP REQ").await;
    assert!(req.contains("sasl") && req.contains("server-time"));
    p.send(":srv CAP * ACK :sasl server-time").await;
    assert_eq!(p.expect("AUTHENTICATE").await, "AUTHENTICATE SCRAM-SHA-256");
    p.send("AUTHENTICATE +").await;
    let first = p.expect("AUTHENTICATE").await;
    let decoded = String::from_utf8(base64_decode(first.trim_start_matches("AUTHENTICATE "))).unwrap();
    assert!(decoded.starts_with("n,,n=acct,r="), "client-first: {decoded}");
    p.send(":srv 904 me :SASL authentication failed").await;
    assert_eq!(p.expect("AUTHENTICATE").await, "AUTHENTICATE PLAIN");
    p.send("AUTHENTICATE +").await;
    let plain = p.expect("AUTHENTICATE").await;
    assert_eq!(base64_decode(plain.trim_start_matches("AUTHENTICATE ")), b"acct\0acct\0pw");
    p.send(":srv 903 me :SASL authentication successful").await;
    p.expect("CAP END").await;
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn sts_upgrades_plaintext_to_tls() {
    let h = Harness::new().await;
    let tls_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tls_port = tls_listener.local_addr().unwrap().port();
    h.mgr.connect(&h.server());

    let mut p = Peer::accept(&h.listener).await;
    p.expect("CAP LS").await;
    p.send(&format!(":srv CAP * LS :sts=port={tls_port},duration=300 server-time")).await;

    // The client must hang up without requesting caps and reconnect over TLS.
    let (mut sock, _) = tokio::time::timeout(Duration::from_secs(10), tls_listener.accept()).await.unwrap().unwrap();
    let mut hello = [0u8; 3];
    sock.read_exact(&mut hello).await.unwrap();
    assert_eq!(hello[0], 0x16, "TLS handshake record");
    assert_eq!(hello[1], 0x03);
    // A policy learned over plaintext must not be persisted.
    assert!(h.db.sts_policy("127.0.0.1").is_none());
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn labeled_history_and_frontend_labels() {
    let h = Harness::new().await;
    h.mgr.connect(&h.server());
    let mut p = Peer::accept(&h.listener).await;
    p.register("batch draft/chathistory labeled-response message-tags server-time").await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Two requests in flight; the replies arrive in the opposite order.
    let m1 = h.mgr.clone();
    let a = tokio::spawn(async move {
        m1.history("t", HistoryKind::Messages, "#a".into(), "CHATHISTORY LATEST #a * 10".into()).await
    });
    let req_a = p.expect("@label=").await;
    let m2 = h.mgr.clone();
    let b = tokio::spawn(async move {
        m2.history("t", HistoryKind::Messages, "#b".into(), "CHATHISTORY LATEST #b * 10".into()).await
    });
    let req_b = p.expect("@label=").await;
    let label = |l: &str| l.split(' ').next().unwrap().trim_start_matches("@label=").to_string();
    let (la, lb) = (label(&req_a), label(&req_b));

    p.send(&format!("@label={lb} :srv FAIL CHATHISTORY INVALID_TARGET LATEST #b :No such channel")).await;
    p.send(&format!("@label={la} :srv BATCH +w labeled-response")).await;
    p.send("@batch=w :srv BATCH +h chathistory #a").await;
    p.send("@batch=h;msgid=1;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #a :one").await;
    p.send("@batch=h;msgid=2;time=2026-01-01T00:00:01.000Z :x!u@h PRIVMSG #a :two").await;
    p.send("@batch=w :srv BATCH -h").await;
    p.send(":srv BATCH -w").await;

    let got_a = a.await.unwrap().expect("history for #a");
    assert_eq!(got_a.len(), 2);
    assert_eq!(got_a[1].param(1), "two");
    assert_eq!(b.await.unwrap().unwrap_err().to_string(), "No such channel");

    // History with msgids is backfilled into the local log.
    h.db.flush();
    assert_eq!(h.db.history("t", "#a", None, 10).unwrap().len(), 2);

    // A labeled response to a line the frontend sent keeps its label, even
    // for messages nested in the batch.
    h.mgr.send("t", vec!["@label=f7 WHOIS x".into()]).unwrap();
    p.expect("@label=f7 WHOIS").await;
    p.send("@label=f7 :srv BATCH +r labeled-response").await;
    p.send("@batch=r :srv 311 me x u h * :Real").await;
    p.send(":srv BATCH -r").await;
    let whois = h.event(|m| m.command == "311").await;
    assert_eq!(whois.tag("label"), Some("f7"));
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_live_and_outgoing_messages() {
    let h = Harness::new().await;
    h.mgr.connect(&h.server());
    let mut p = Peer::accept(&h.listener).await;
    p.register("").await;

    p.send(":x!u@h PRIVMSG #Chan :hello channel").await;
    p.send(":x!u@h PRIVMSG me :hello you").await;
    p.send(":x!u@h PRIVMSG me :\x01VERSION\x01").await;
    p.send(":srv.example NOTICE me :server notice").await;
    h.event(|m| m.command == "NOTICE").await;
    h.mgr.send("t", vec!["PRIVMSG x :reply".into()]).unwrap();
    p.expect("PRIVMSG x").await;

    h.db.flush();
    assert_eq!(h.db.history("t", "#chan", None, 10).unwrap().len(), 1, "channel names are case-folded");
    let query = h.db.history("t", "x", None, 10).unwrap();
    assert_eq!(query.len(), 2, "incoming and our own reply, no CTCP: {query:?}");
    assert_eq!(query[1].source.as_ref().unwrap().nick, "me");
    assert!(h.db.history("t", "srv.example", None, 10).unwrap().is_empty());
    h.mgr.disconnect_all();
}
