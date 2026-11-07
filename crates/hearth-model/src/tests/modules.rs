//! History merging, unread counts, hit
//! dedupe, MONITOR and rejoin commands, at the App level where it matters.

use crate::effect::{Effect, Reply};
use crate::history::merge_lines;
use crate::search::dedupe_hits;
use crate::testing::*;
use crate::types::{IrcMessage, Line, LineType, SavedBuffer, SearchHit};
use crate::unread::{count_highlights, count_unread};

fn line(id: u64, time: i64, text: &str) -> Line {
    Line::new(id, time, LineType::Privmsg, "ann", text)
}

#[test]
fn merge_returns_none_when_nothing_is_new() {
    let mut a = line(1, 1000, "hi");
    a.msgid = Some("m1".into());
    assert!(merge_lines(std::slice::from_ref(&a), Vec::new()).is_none());
    let copy = Line { id: 99, text: "edited copy".into(), ..a.clone() };
    assert!(merge_lines(&[a], vec![copy]).is_none());
}

#[test]
fn merge_dedupes_msgid_less_lines_by_content_within_3_s() {
    let live = line(1, 10_000, "hi");
    assert!(merge_lines(std::slice::from_ref(&live), vec![line(2, 12_999, "hi")]).is_none());
    assert_eq!(merge_lines(std::slice::from_ref(&live), vec![line(3, 13_000, "hi")]).unwrap().len(), 2);
    assert_eq!(merge_lines(std::slice::from_ref(&live), vec![line(4, 10_000, "other")]).unwrap().len(), 2);
}

#[test]
fn merge_keeps_lines_with_a_new_msgid_even_if_their_text_matches() {
    let live = line(1, 10_000, "hi");
    let mut other = line(2, 10_001, "hi");
    other.msgid = Some("m2".into());
    assert_eq!(merge_lines(&[live], vec![other]).unwrap().len(), 2);
}

#[test]
fn merge_sorts_by_time_then_insertion_order() {
    let a = line(1, 5, "a");
    let (b, c) = (line(2, 1, "b"), line(3, 5, "c"));
    let merged = merge_lines(&[a], vec![c, b]).unwrap();
    assert_eq!(merged.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["b", "a", "c"]);
}

fn unread_lines() -> Vec<Line> {
    let mut v = vec![line(1, 1, "hi"), line(2, 2, "hi"), line(3, 3, "hi"), line(4, 4, "hi"), line(5, 5, "hi")];
    v[1].highlight = true;
    v[2].kind = LineType::Join;
    v[3].is_self = true;
    v[3].highlight = true;
    v[4].kind = LineType::Notice;
    v
}

#[test]
fn counts_messages_from_others_after_the_marker() {
    let msgs = unread_lines();
    assert_eq!(count_unread(&msgs, 0), 3);
    assert_eq!(count_unread(&msgs, 2), 1);
    assert_eq!(count_unread(&msgs, 5), 0);
}

#[test]
fn counts_highlights_or_everything_when_asked() {
    let msgs = unread_lines();
    assert_eq!(count_highlights(&msgs, 0, false), 1);
    assert_eq!(count_highlights(&msgs, 0, true), 3);
    assert_eq!(count_highlights(&msgs, 2, false), 0);
}

#[test]
fn marks_read_and_sends_a_debounced_markread() {
    let mut app = new_app();
    connect(&mut app, &["draft/read-marker"], &[]);
    feed(&mut app, &[":me!u@h JOIN #a", "@time=2026-01-01T00:00:00.000Z :ann!a@h PRIVMSG #a :one"]);
    let id = id_of(&app, "#a");
    assert_eq!(buf(&app, "#a").unread(), 1);
    sent(&mut app);
    app.mark_read(&id);
    assert_eq!(buf(&app, "#a").unread(), 0);
    advance(&mut app, 500);
    assert!(sent(&mut app).contains(&"MARKREAD #a timestamp=2026-01-01T00:00:00.000Z".to_string()));
}

fn hit(network: &str, tags: &[(&str, &str)]) -> SearchHit {
    let mut msg = IrcMessage::new("PRIVMSG", &["#a", "x"]);
    msg.tags = tags.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    SearchHit { network: network.into(), target: "#a".into(), msg }
}

#[test]
fn dedupe_drops_duplicates_by_msgid_or_content_and_sorts_newest_first() {
    let out = dedupe_hits(vec![
        hit("c", &[("msgid", "m1"), ("time", "2026-01-01T00:00:01Z")]),
        hit("c", &[("msgid", "m1"), ("time", "2026-01-01T00:00:01Z")]),
        hit("d", &[("msgid", "m1"), ("time", "2026-01-01T00:00:03Z")]),
        hit("c", &[("time", "2026-01-01T00:00:02Z")]),
        hit("c", &[("time", "2026-01-01T00:00:02Z")]),
    ]);
    let got: Vec<(&str, &str)> = out.iter().map(|h| (h.network.as_str(), h.msg.tags["time"].as_str())).collect();
    assert_eq!(got, [("d", "2026-01-01T00:00:03Z"), ("c", "2026-01-01T00:00:02Z"), ("c", "2026-01-01T00:00:01Z")]);
}

#[test]
fn refuses_to_watch_past_the_server_limit() {
    let mut app = new_app();
    connect(&mut app, &[], &[("MONITOR", "1")]);
    app.watch(CONN, "ann");
    sent(&mut app);
    app.watch(CONN, "bob");
    assert_eq!(crate::monitor::watched(&app, app.conn(CONN).unwrap()), ["ann"]);
    assert!(!sent(&mut app).contains(&"MONITOR + bob".to_string()));
}

#[test]
fn subscribes_watched_nicks_and_open_queries_on_connect() {
    let mut app = new_app();
    app.settings.watch.insert("c|".into(), vec!["Ann".into()]);
    connect(&mut app, &[], &[("MONITOR", "100")]);
    for e in requests(&mut app) {
        if let Effect::LoadBuffers { req, .. } = e {
            app.resolve(req, Ok(Reply::Buffers(vec![SavedBuffer { name: "bob".into(), kind: "query".into(), key: String::new() }])));
        }
    }
    advance(&mut app, 2000);
    assert!(sent(&mut app).contains(&"MONITOR + Ann,bob".to_string()));
}
