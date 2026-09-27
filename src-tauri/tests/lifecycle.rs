//! Reconnects, backoff, keepalive and send pacing, on a paused tokio clock
//! that only moves when a test advances it.

mod common;

use common::*;
use hearth_lib::irc::client::Cmd;
use hearth_lib::irc::events::{Event, Status};
use std::sync::atomic::Ordering;

/// Wait for the actor to report the session ended, returning its error.
async fn disconnected(d: &Direct, nth: usize) -> Option<String> {
    let evs = wait_count(&d.events, nth, |e| is_status(e, "t", Status::Disconnected)).await;
    match &evs[nth - 1] {
        Event::Status { error, .. } => error.clone(),
        _ => unreachable!(),
    }
}

/// The connection must not come back before `delay`, and must right after.
async fn expect_retry_after(d: &Direct, delay: u64) -> Peer {
    advance(ms(delay * 1000 - 10), secs(1)).await;
    assert!(Peer::try_accept(&d.listener, ms(100)).await.is_none(), "reconnected before {delay}s");
    advance(ms(10), ms(10)).await;
    Peer::accept(&d.listener).await
}

#[tokio::test(start_paused = true)]
async fn reconnects_with_backoff_and_rejoins_channels() {
    let _clock = hold_clock();
    let d = Direct::start(
        |s| {
            s.autojoin = vec!["#a".into(), " ".into(), "#b".into()];
            s.perform = vec!["/MODE $nick +i".into()];
        },
        None,
    )
    .await;

    // Failures before registration back off exponentially: 2s, 4s, 8s.
    let mut p = Peer::accept(&d.listener).await;
    p.expect("CAP LS").await;
    drop(p);
    // Closed or reset, depending on whether our side had unread input.
    assert!(disconnected(&d, 1).await.is_some());
    let p = expect_retry_after(&d, 2).await;
    drop(p);
    disconnected(&d, 2).await;
    let p = expect_retry_after(&d, 4).await;
    drop(p);
    disconnected(&d, 3).await;
    let mut p = expect_retry_after(&d, 8).await;

    p.register("").await;
    assert_eq!(p.expect("JOIN").await, "JOIN #a,#b");
    assert_eq!(p.line().await, "MODE me +i");
    wait_event(&d.events, "connected", |e| is_status(e, "t", Status::Connected)).await;

    // Losing a registered session resets the backoff, and the channels are
    // joined again on the new link.
    p.send("ERROR :Closing link (Ping timeout)").await;
    assert!(p.closed().await.is_empty());
    assert_eq!(disconnected(&d, 4).await.as_deref(), Some("Closing link (Ping timeout)"));
    let mut p = expect_retry_after(&d, 2).await;
    p.register("").await;
    assert_eq!(p.expect("JOIN").await, "JOIN #a,#b");

    // Channels the user joined by hand are rejoined by the frontend, keys
    // and all; the actor must pass that through on the new link.
    d.send(&["JOIN #secret,#open hunter2"]);
    assert_eq!(p.expect("JOIN").await, "JOIN #secret,#open hunter2");

    assert_eq!(retry_delays(&d.events), vec![Some(2), Some(4), Some(8), Some(2)]);
    d.quit().await;
}

#[tokio::test(start_paused = true)]
async fn backoff_is_capped_at_two_minutes() {
    let _clock = hold_clock();
    let d = Direct::start(|_| {}, None).await;
    let delays = [2, 4, 8, 16, 32, 64, 120, 120];
    for (i, delay) in delays.iter().enumerate() {
        drop(Peer::accept(&d.listener).await);
        disconnected(&d, i + 1).await;
        advance(secs(*delay), secs(*delay)).await;
    }
    let want: Vec<_> = delays.iter().map(|d| Some(*d)).collect();
    assert_eq!(retry_delays(&d.events), want);
    d.quit().await;
}

#[tokio::test(start_paused = true)]
async fn quit_during_backoff_stops_the_actor() {
    let _clock = hold_clock();
    let d = Direct::start(|_| {}, None).await;
    drop(Peer::accept(&d.listener).await);
    disconnected(&d, 1).await;

    // Requests made while offline fail fast instead of waiting for a link.
    let (reply, rx) = tokio::sync::oneshot::channel();
    d.tx.send(Cmd::History {
        kind: hearth_lib::irc::client::HistoryKind::Messages,
        target: "#a".into(),
        line: "CHATHISTORY LATEST #a * 10".into(),
        reply,
    })
    .unwrap();
    assert_eq!(must("history reply", rx).await.unwrap().unwrap_err(), "not connected");
    d.quit().await;
}

#[tokio::test(start_paused = true)]
async fn manual_reconnect_skips_the_backoff() {
    let _clock = hold_clock();
    let d = Direct::start(|_| {}, None).await;
    let mut p = Peer::accept(&d.listener).await;
    p.register("").await;

    d.tx.send(Cmd::Reconnect).unwrap();
    assert_eq!(p.closed().await.last().map(String::as_str), Some("QUIT :Reconnecting"));
    // No clock movement needed: a requested reconnect is immediate.
    let mut p = Peer::accept(&d.listener).await;
    p.expect("CAP LS").await;
    disconnected(&d, 1).await;
    assert_eq!(retry_delays(&d.events), vec![None]);

    // During a backoff, Reconnect cuts the wait short.
    drop(p);
    disconnected(&d, 2).await;
    d.tx.send(Cmd::Reconnect).unwrap();
    Peer::accept(&d.listener).await;
    d.quit().await;
}

#[tokio::test(start_paused = true)]
async fn keepalive_pings_and_times_out() {
    let _clock = hold_clock();
    let d = Direct::start(|_| {}, None).await;
    let mut p = Peer::accept(&d.listener).await;
    p.register("").await;
    d.handled("422").await;

    // Silence for more than 90s draws a PING (checked every 15s).
    advance(secs(90), secs(15)).await;
    assert!(p.try_line(ms(100)).await.is_none(), "pinged too early");
    advance(secs(15), secs(15)).await;
    assert_eq!(p.line().await, "PING :hearth");

    // Any traffic counts as an answer and keeps the link up.
    p.send(":srv PONG srv :hearth").await;
    d.handled("PONG").await;
    advance(secs(90), secs(15)).await;
    assert!(p.try_line(ms(100)).await.is_none(), "pinged too early after PONG");
    advance(secs(15), secs(15)).await;
    assert_eq!(p.line().await, "PING :hearth");

    // No answer at all: the link is dropped once 150s have passed.
    advance(secs(45), secs(15)).await;
    assert!(p.try_line(ms(100)).await.is_none(), "gave up too early");
    advance(secs(15), secs(15)).await;
    assert!(p.closed().await.is_empty());
    assert_eq!(disconnected(&d, 1).await.as_deref(), Some("ping timeout"));

    // ...and reconnects after the reset backoff.
    let mut p = expect_retry_after(&d, 2).await;
    p.expect("CAP LS").await;
    d.quit().await;
}

#[tokio::test(start_paused = true)]
async fn registration_times_out() {
    let _clock = hold_clock();
    let d = Direct::start(|_| {}, None).await;
    let mut p = Peer::accept(&d.listener).await;
    p.expect("USER").await;
    // The server keeps talking but never welcomes us.
    advance(secs(45), secs(15)).await;
    p.send(":srv NOTICE * :*** Looking up your hostname").await;
    d.handled("NOTICE").await;
    advance(secs(15), secs(15)).await;
    assert!(p.try_line(ms(100)).await.is_none(), "gave up too early");
    advance(secs(15), secs(15)).await;
    p.closed().await;
    assert_eq!(disconnected(&d, 1).await.as_deref(), Some("registration timed out"));
    assert_eq!(retry_delays(&d.events), vec![Some(2)]);
    d.quit().await;
}

#[tokio::test(start_paused = true)]
async fn direct_connections_are_paced() {
    let _clock = hold_clock();
    let d = Direct::start(|_| {}, None).await;
    let mut p = Peer::accept(&d.listener).await;
    p.register("").await;
    d.handled("422").await;
    // Let the token bucket refill after registration.
    advance(secs(10), secs(5)).await;

    let lines: Vec<String> = (0..12).map(|i| format!("PRIVMSG #a :{i}")).collect();
    d.tx.send(Cmd::Send(lines)).unwrap();
    // A burst of eight goes out at once...
    for i in 0..8 {
        assert_eq!(p.line().await, format!("PRIVMSG #a :{i}"));
    }
    assert!(p.try_line(ms(100)).await.is_none(), "burst exceeded");
    assert_eq!(d.backlog.load(Ordering::Relaxed), 4, "lines still waiting");
    // ...then one every 400ms.
    for i in 8..12 {
        advance(ms(390), ms(390)).await;
        assert!(p.try_line(ms(50)).await.is_none(), "line {i} too early");
        advance(ms(10), ms(10)).await;
        assert_eq!(p.line().await, format!("PRIVMSG #a :{i}"));
    }
    must("backlog drained", async {
        while d.backlog.load(Ordering::Relaxed) != 0 {
            real_sleep(ms(2)).await;
        }
    })
    .await;
    d.quit().await;
}

#[tokio::test(start_paused = true)]
async fn bouncer_children_are_not_paced() {
    let _clock = hold_clock();
    let d = Direct::start(|_| {}, Some("7")).await;
    let mut p = Peer::accept(&d.listener).await;
    p.expect("CAP LS").await;
    p.send(":srv CAP * LS :batch").await;
    p.expect("CAP REQ").await;
    p.send(":srv CAP * ACK :batch").await;
    assert_eq!(p.line().await, "BOUNCER BIND 7");
    assert_eq!(p.line().await, "CAP END");
    p.welcome().await;

    let lines: Vec<String> = (0..30).map(|i| format!("PRIVMSG #a :{i}")).collect();
    d.tx.send(Cmd::Send(lines)).unwrap();
    // All of it arrives without the clock moving at all.
    for i in 0..30 {
        assert_eq!(p.line().await, format!("PRIVMSG #a :{i}"));
    }
    d.quit().await;
}

#[tokio::test(start_paused = true)]
async fn quit_is_flushed_with_its_reason() {
    let _clock = hold_clock();
    let d = Direct::start(|_| {}, None).await;
    let mut p = Peer::accept(&d.listener).await;
    p.register("").await;
    d.tx.send(Cmd::Quit(Some("see you".into()))).unwrap();
    assert_eq!(p.closed().await, vec!["QUIT :see you"]);
    must("the actor to stop", d.task).await.unwrap();
    // No reconnect was scheduled.
    real_sleep(ms(50)).await;
    assert_eq!(retry_delays(&d.events), Vec::<Option<u64>>::new());
}
