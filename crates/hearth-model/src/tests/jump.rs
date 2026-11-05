//! Jumping to a message, filling gaps, and per-buffer bouncer search.
//! Requests are answered by a scripted backend, as the mock API did.

use crate::buffer::Gap;
use crate::effect::{Effect, Reply};
use crate::history::GapFrom;
use crate::irc::{iso_time, parse_time};
use crate::search::SearchScope;
use crate::testing::*;
use crate::types::IrcMessage;
use crate::{App, Line};
use std::collections::VecDeque;

fn t(s: i64) -> i64 {
    parse_time("2026-01-01T00:00:00.000Z").unwrap() + s * 1000
}

fn iso(s: i64) -> String {
    iso_time(t(s))
}

fn at(s: i64, text: &str, target: &str) -> IrcMessage {
    parse(&format!("@time={} :ann!a@h PRIVMSG {target} :{text}", iso(s)))
}

/// Answer every queued request until the queue is quiet; returns the history lines asked.
fn pump(app: &mut App, mut answer: impl FnMut(&str) -> Result<Vec<IrcMessage>, String>) -> Vec<String> {
    let mut seen = Vec::new();
    loop {
        let reqs = requests(app);
        if reqs.is_empty() {
            return seen;
        }
        for e in reqs {
            match e {
                Effect::History { req, line, .. } => {
                    seen.push(line.clone());
                    app.resolve(req, answer(&line).map(Reply::Messages));
                }
                Effect::LocalHistory { req, .. } => app.resolve(req, Ok(Reply::Messages(Vec::new()))),
                Effect::LoadBuffers { req, .. } => app.resolve(req, Ok(Reply::Buffers(Vec::new()))),
                other => panic!("unexpected request {other:?}"),
            }
        }
    }
}

/// Latest-lines requests get nothing; the rest are served from `queue` in order.
fn queued(queue: &mut VecDeque<Vec<IrcMessage>>) -> impl FnMut(&str) -> Result<Vec<IrcMessage>, String> + '_ {
    move |line| Ok(if line.contains("LATEST") { Vec::new() } else { queue.pop_front().unwrap_or_default() })
}

fn history_app() -> App {
    let mut app = new_app();
    connect(&mut app, &["draft/chathistory", "server-time"], &[("CHATHISTORY", "1000")]);
    app
}

fn gaps(app: &App, id: &crate::BufId) -> Vec<(i64, i64)> {
    app.buffer(id).unwrap().gaps.iter().map(|g| (g.after, g.before)).collect()
}

#[test]
fn loads_around_an_old_message_by_timestamp_and_fills_the_gap_to_the_present() {
    let mut app = history_app();
    feed(&mut app, &[":me!u@h JOIN #a", &format!("@time={} :bob!b@h PRIVMSG #a :now", iso(1000))]);
    let id = id_of(&app, "#a");
    pump(&mut app, |_| Ok(Vec::new()));
    let mut queue = VecDeque::from([vec![at(10, "before", "#a"), at(20, "target", "#a"), at(30, "after", "#a")]]);
    // soju has no msgids; its MSGREFTYPES is timestamp only.
    app.jump_to(&id, t(20), Some("id-from-another-server"));
    let asked = pump(&mut app, queued(&mut queue));
    assert_eq!(asked.last().unwrap(), &format!("CHATHISTORY AROUND #a timestamp={} 100", iso(20)));
    assert_eq!(app.buffer(&id).unwrap().focus.as_ref().map(|f| f.time), Some(t(20)));
    assert_eq!(gaps(&app, &id), [(t(30), t(1000))]);

    // Scrolling down into the gap pages forward until it closes.
    let mut queue = VecDeque::from([(0..200).map(|i| at(31 + i, &format!("fill {i}"), "#a")).collect::<Vec<_>>()]);
    app.fill_gap(&id, Gap { after: t(30), before: t(1000) }, GapFrom::After);
    let asked = pump(&mut app, queued(&mut queue));
    assert_eq!(asked.last().unwrap(), &format!("CHATHISTORY BETWEEN #a timestamp={} timestamp={} 200", iso(30), iso(1000)));
    assert_eq!(gaps(&app, &id), [(t(230), t(1000))]);
    let mut queue = VecDeque::from([vec![at(500, "last of it", "#a")]]);
    app.fill_gap(&id, Gap { after: t(230), before: t(1000) }, GapFrom::After);
    pump(&mut app, queued(&mut queue));
    assert!(gaps(&app, &id).is_empty());
    assert_eq!(app.buffer(&id).unwrap().lines().len(), 3 + 200 + 1 + 1);
}

#[test]
fn pages_up_into_a_gap_from_the_present() {
    let mut app = history_app();
    feed(&mut app, &[":me!u@h JOIN #a", &format!("@time={} :bob!b@h PRIVMSG #a :now", iso(1000))]);
    let id = id_of(&app, "#a");
    pump(&mut app, |_| Ok(Vec::new()));
    let gap = Gap { after: t(30), before: t(1000) };
    app.buffer_mut(&id).unwrap().gaps = vec![gap];
    let mut queue = VecDeque::from([vec![at(900, "recent", "#a")]]);
    app.fill_gap(&id, gap, GapFrom::Before);
    let asked = pump(&mut app, queued(&mut queue));
    assert_eq!(asked.last().unwrap(), &format!("CHATHISTORY BETWEEN #a timestamp={} timestamp={} 200", iso(1000), iso(30)));
    assert!(gaps(&app, &id).is_empty());
}

#[test]
fn uses_msgid_when_the_server_takes_it() {
    let mut app = new_app();
    connect(&mut app, &["draft/chathistory"], &[("MSGREFTYPES", "msgid,timestamp")]);
    feed(&mut app, &[":me!u@h JOIN #a"]);
    let id = id_of(&app, "#a");
    pump(&mut app, |_| Ok(Vec::new()));
    app.jump_to(&id, t(20), Some("abc"));
    let asked = pump(&mut app, |_| Ok(Vec::new()));
    assert_eq!(asked.last().unwrap(), "CHATHISTORY AROUND #a msgid=abc 100");
}

#[test]
fn waits_for_a_new_buffers_latest_lines_before_loading_around() {
    let mut app = history_app();
    pump(&mut app, |_| Ok(Vec::new()));
    let key = app.conn_mut(CONN).unwrap().ensure("#new", None);
    let id = crate::BufId::new(CONN, key);
    app.jump_to(&id, t(20), None);
    let asked = pump(&mut app, |line| Ok(if line.contains("LATEST") { vec![at(1000, "latest", "#new")] } else { vec![at(20, "old", "#new")] }));
    assert_eq!(asked, ["CHATHISTORY LATEST #new * 100".to_string(), format!("CHATHISTORY AROUND #new timestamp={} 100", iso(20))]);
    assert_eq!(gaps(&app, &id), [(t(20), t(1000))]);
}

#[test]
fn searches_buffer_by_buffer_when_the_bouncer_requires_in() {
    let mut app = new_app();
    connect(&mut app, &["soju.im/search"], &[]);
    feed(&mut app, &[":me!u@h JOIN #a", ":me!u@h JOIN #b"]);
    app.settings.store_logs = false;
    pump(&mut app, |_| Ok(Vec::new()));
    let backend = |line: &str| -> Result<Vec<IrcMessage>, String> {
        if !line.contains(";in=") {
            return Err("The in parameter is mandatory".into());
        }
        Ok(if line.contains("in=#b") { vec![at(5, "deploy", "#b")] } else { Vec::new() })
    };
    let job = app.search("deploy from:ann", &SearchScope::default(), None);
    let mut asked = pump(&mut app, backend);
    let page = app.take_search(job).expect("the search finished");
    let hits: Vec<(&str, &str)> = page.hits.iter().map(|h| (h.target.as_str(), h.msg.param(1))).collect();
    assert_eq!(hits, [("#b", "deploy")]);
    assert_eq!(asked, ["SEARCH limit=100;text=deploy;from=ann", "SEARCH limit=50;text=deploy;from=ann;in=#a", "SEARCH limit=50;text=deploy;from=ann;in=#b"]);
    // Remembered: the next search goes straight to the buffers.
    let job = app.search("x", &SearchScope::default(), None);
    asked.extend(pump(&mut app, backend));
    app.take_search(job).expect("the second search finished");
    assert_eq!(asked.len(), 5);
}

#[test]
fn history_lines_keep_their_order_when_merged_around_a_jump() {
    // The jump's window sits in the middle of two live lines and must not reorder them.
    let mut app = history_app();
    feed(&mut app, &[":me!u@h JOIN #a", &format!("@time={} :bob!b@h PRIVMSG #a :now", iso(1000))]);
    let id = id_of(&app, "#a");
    pump(&mut app, |_| Ok(Vec::new()));
    let mut queue = VecDeque::from([vec![at(10, "x", "#a"), at(20, "y", "#a")]]);
    app.jump_to(&id, t(20), None);
    pump(&mut app, queued(&mut queue));
    let times: Vec<i64> = app.buffer(&id).unwrap().lines().iter().map(|l: &Line| l.time).collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
}
