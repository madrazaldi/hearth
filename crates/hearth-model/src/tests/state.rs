//! App-level state: connections, buffers and members.

use crate::buffer::HistoryState;
use crate::effect::{Effect, Reply};
use crate::history::HistMode;
use crate::irc::parse_time;
use crate::state::SendKind;
use crate::testing::*;
use crate::types::{IrcMessage, Line, LineType, NotifyLevel, SavedBuffer};

fn conn(app: &crate::App) -> &crate::Conn {
    app.conn(CONN).unwrap()
}

#[test]
fn files_channel_and_private_messages_and_flags_mentions() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    feed(&mut app, &[":me!u@h JOIN #hearth", ":ann!a@h PRIVMSG #hearth :hello there", ":bob!b@h PRIVMSG #hearth :me: ping"]);
    feed(&mut app, &[":cat!c@h PRIVMSG me :psst"]);
    let chan = buf(&app, "#hearth");
    let got: Vec<(&str, bool)> = chan.lines().iter().map(|l| (l.nick.as_str(), l.highlight)).collect();
    assert_eq!(got, [("ann", false), ("bob", true)]);
    assert_eq!(buf(&app, "cat").kind, crate::BufferKind::Query);
    assert_eq!(texts(&app, "cat"), ["psst"]);
}

#[test]
fn drops_messages_from_ignored_masks() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    app.settings.ignores = vec!["spammer".into(), "*!*@bad.host".into()];
    feed(&mut app, &[":me!u@h JOIN #a", ":Spammer!x@y PRIVMSG #a :buy", ":eve!e@bad.host PRIVMSG #a :hi", ":ann!a@ok PRIVMSG #a :fine"]);
    feed(&mut app, &["@+typing=active :spammer!x@y TAGMSG #a"]);
    assert_eq!(texts(&app, "#a"), ["fine"]);
    assert!(buf(&app, "#a").typing.is_empty());
}

#[test]
fn rate_limits_ctcp_replies() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    for i in 0..10 {
        feed(&mut app, &[&format!(":x{i}!u@h PRIVMSG me :\x01VERSION\x01")]);
    }
    assert_eq!(sent(&mut app).iter().filter(|l| l.starts_with("NOTICE")).count(), 3);
    assert!(server_texts(&app).iter().any(|t| t.contains("flood")));
}

#[test]
fn attaches_reactions_and_replies() {
    let mut app = new_app();
    connect(&mut app, &["message-tags"], &[]);
    feed(&mut app, &[":me!u@h JOIN #a", "@msgid=m1 :ann!a@h PRIVMSG #a :ship it?"]);
    feed(&mut app, &["@+draft/react=\u{1f44d};+draft/reply=m1 :bob!b@h TAGMSG #a", "@+draft/react=\u{1f44d};+draft/reply=m1 :bob!b@h TAGMSG #a"]);
    feed(&mut app, &["@msgid=m2;+draft/reply=m1 :cat!c@h PRIVMSG #a :yes"]);
    let lines = buf(&app, "#a").lines();
    assert_eq!(lines[0].reactions, [("\u{1f44d}".to_string(), vec!["bob".to_string()])]);
    assert_eq!(lines[1].reply_to.as_deref(), Some("m1"));
}

#[test]
fn sends_replies_and_reactions_with_tags() {
    let mut app = new_app();
    connect(&mut app, &["message-tags"], &[]);
    feed(&mut app, &[":me!u@h JOIN #a", "@msgid=m1 :ann!a@h PRIVMSG #a :hi"]);
    let id = id_of(&app, "#a");
    let first = buf(&app, "#a").lines()[0].clone();
    app.send_message(&id, "hello", SendKind::Privmsg, false, Some("m1"));
    app.react(&id, &first, "\u{1f389}");
    let out = sent(&mut app);
    assert!(out.contains(&"@+draft/reply=m1 PRIVMSG #a :hello".to_string()), "{out:?}");
    assert!(out.contains(&"@+draft/react=\u{1f389};+draft/reply=m1 TAGMSG #a".to_string()), "{out:?}");
    // Without echo-message the reaction shows up locally at once.
    assert_eq!(buf(&app, "#a").lines()[0].reactions, [("\u{1f389}".to_string(), vec!["me".to_string()])]);
}

#[test]
fn trims_the_oldest_lines_in_a_chunk_once_past_the_cap() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    feed(&mut app, &[":me!u@h JOIN #big"]);
    let t = app.now();
    let b = app.conn_mut(CONN).unwrap().buffer_mut("#big").unwrap();
    b.set_lines((0..20_000).map(|i| Line::new(i + 1, t + i as i64, LineType::Privmsg, "a", i.to_string())).collect());
    b.history = HistoryState::Done;
    b.push(Line::new(99_999, t + 20_000, LineType::Privmsg, "a", "last"));
    assert_eq!(b.lines().len(), 18_000);
    assert_eq!(b.lines()[0].text, "2001");
    assert_eq!(b.lines().last().unwrap().text, "last");
    // Older lines can be fetched again.
    assert_eq!(b.history, HistoryState::Idle);
}

#[test]
fn sends_labeled_replies_to_the_buffer_that_asked() {
    let mut app = new_app();
    connect(&mut app, &["labeled-response"], &[]);
    feed(&mut app, &[":me!u@h JOIN #a", ":me!u@h JOIN #b"]);
    let (a, b) = (id_of(&app, "#a"), id_of(&app, "#b"));
    app.open(&b);
    app.conn_mut(CONN).unwrap().send_from(&a.key, "WHOIS ann");
    let line = sent(&mut app).pop().unwrap();
    let label = line.strip_prefix("@label=").and_then(|r| r.strip_suffix(" WHOIS ann")).expect("labeled WHOIS").to_string();
    feed(&mut app, &[&format!("@label={label} :srv 311 me ann a h * :Ann"), &format!("@label={label} :srv 318 me ann :End")]);
    assert_eq!(buf(&app, "#a").lines().last().unwrap().kind, LineType::Whois);
    assert!(!buf(&app, "#b").lines().iter().any(|l| l.kind == LineType::Whois));
}

#[test]
fn collects_list_replies_into_the_channel_browser() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    feed(&mut app, &[":srv 321 me Channel :Users Name", ":srv 322 me #a 12 :topic a", ":srv 322 me #b 3 :topic b", ":srv 323 me :End"]);
    let list = conn(&app).channel_list.as_ref().unwrap();
    assert!(!list.loading);
    let got: Vec<(&str, u32)> = list.items.iter().map(|c| (c.name.as_str(), c.users)).collect();
    assert_eq!(got, [("#a", 12), ("#b", 3)]);
    assert!(server_texts(&app).is_empty());
}

#[test]
fn the_channel_browser_sorts_by_each_column_and_reverses_on_a_second_click() {
    use crate::panels::ChannelSort;
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    feed(
        &mut app,
        &[
            ":srv 321 me Channel :Users Name",
            ":srv 322 me #b 3 :zebra talk",
            ":srv 322 me #a 12 :Apple pie",
            ":srv 322 me #c 12 :",
            ":srv 322 me #d 1 :mango",
            ":srv 323 me :End",
        ],
    );
    app.browse_channels(CONN);
    let names = |app: &crate::App| app.browser_view().unwrap().items.iter().map(|c| c.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&app), ["#a", "#c", "#b", "#d"], "busiest first, ties by name");
    app.browser_set_sort(ChannelSort::Users);
    assert_eq!(names(&app), ["#d", "#b", "#a", "#c"], "again: fewest first");
    app.browser_set_sort(ChannelSort::Name);
    assert_eq!(names(&app), ["#a", "#b", "#c", "#d"]);
    app.browser_set_sort(ChannelSort::Name);
    assert_eq!(names(&app), ["#d", "#c", "#b", "#a"]);
    app.browser_set_sort(ChannelSort::Topic);
    assert_eq!(names(&app), ["#a", "#d", "#b", "#c"], "by topic; the one without a topic is last");
    app.browser_set_sort(ChannelSort::Topic);
    assert_eq!(names(&app), ["#b", "#d", "#a", "#c"], "reversed; still last");
}

#[test]
fn tracks_monitor_presence_and_announces_watched_nicks() {
    let mut app = new_app();
    connect(&mut app, &[], &[("MONITOR", "100")]);
    app.settings.watch.insert("c|".into(), vec!["Ann".into()]);
    feed(&mut app, &[":srv 730 me :ann!a@h,bob!b@h"]);
    assert_eq!(conn(&app).online.get("ann"), Some(&true));
    assert!(server_texts(&app).is_empty());
    feed(&mut app, &[":srv 731 me :ann"]);
    assert_eq!(conn(&app).online.get("ann"), Some(&false));
    assert_eq!(server_texts(&app).last().map(String::as_str), Some("ann is offline"));
}

fn saved(name: &str, kind: &str, key: &str) -> SavedBuffer {
    SavedBuffer { name: name.into(), kind: kind.into(), key: key.into() }
}

#[test]
fn restores_saved_buffers_and_rejoins_with_keys() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    let load = requests(&mut app).into_iter().find_map(|e| match e {
        Effect::LoadBuffers { req, .. } => Some(req),
        _ => None,
    });
    app.resolve(
        load.expect("asks for the saved buffers"),
        Ok(Reply::Buffers(vec![saved("#open", "channel", ""), saved("#locked", "channel", "sekrit"), saved("ann", "query", "")])),
    );
    advance(&mut app, 2000);
    assert_eq!(buf(&app, "ann").kind, crate::BufferKind::Query);
    assert!(sent(&mut app).contains(&"JOIN #locked,#open sekrit".to_string()));
}

#[test]
fn saves_the_buffer_list_and_forgets_parted_channels() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    // Nothing is saved until the previous list has been loaded.
    for e in requests(&mut app) {
        if let Effect::LoadBuffers { req, .. } = e {
            app.resolve(req, Ok(Reply::Buffers(Vec::new())));
        }
    }
    app.join(CONN, "#keyed", Some("k1"));
    feed(&mut app, &[":me!u@h JOIN #keyed", ":me!u@h JOIN #gone", ":me!u@h PART #gone"]);
    advance(&mut app, 600);
    let last = effects(&mut app).into_iter().rev().find_map(|e| match e {
        Effect::SaveBuffers { conn, buffers } => Some((conn, buffers)),
        _ => None,
    });
    let (c, buffers) = last.expect("saves the list");
    assert_eq!(c, "c");
    let got: Vec<(&str, &str, &str)> = buffers.iter().map(|b| (b.name.as_str(), b.kind.as_str(), b.key.as_str())).collect();
    assert_eq!(got, [("#keyed", "channel", "k1")]);
}

#[test]
fn falls_back_to_the_local_log_without_chathistory() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    feed(&mut app, &[":me!u@h JOIN #a", "@time=2026-01-01T00:00:10.000Z :ann!a@h PRIVMSG #a :live"]);
    let id = id_of(&app, "#a");
    app.buffer_mut(&id).unwrap().history = HistoryState::Idle;
    requests(&mut app);
    app.fetch_history(&id, HistMode::Before);
    let req = requests(&mut app).into_iter().find_map(|e| match e {
        Effect::LocalHistory { req, conn, target, before, after, limit } => {
            assert_eq!((conn.as_str(), target.as_str(), before, after, limit), ("c", "#a", parse_time("2026-01-01T00:00:10.000Z"), None, 100));
            Some(req)
        }
        _ => None,
    });
    let old = IrcMessage::parse("@time=2026-01-01T00:00:01.000Z :bob!b@h PRIVMSG #a :old").unwrap();
    app.resolve(req.expect("asks the local log"), Ok(Reply::Messages(vec![old])));
    assert_eq!(texts(&app, "#a"), ["old", "live"]);
    assert_eq!(buf(&app, "#a").history, HistoryState::Done);
}

#[test]
fn dedupes_history_lines_that_match_live_ones_closely_in_time() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    feed(&mut app, &[":me!u@h JOIN #a"]);
    let id = id_of(&app, "#a");
    app.send_message(&id, "mine", SendKind::Privmsg, false, None);
    let b = app.buffer_mut(&id).unwrap();
    let mut dup = b.lines()[0].clone();
    dup.id = 999;
    dup.time -= 5;
    b.merge(vec![dup]);
    assert_eq!(b.lines().len(), 1);
}

#[test]
fn mutes_and_escalates_per_buffer() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    feed(&mut app, &[":me!u@h JOIN #a", ":ann!a@h PRIVMSG #a :chatter", ":ann!a@h PRIVMSG #a :me: look"]);
    let id = id_of(&app, "#a");
    let pref = {
        let b = app.buffer_mut(&id).unwrap();
        b.read_marker = 0;
        b.pref_key()
    };
    assert_eq!(app.summary(&id).highlights, 1);
    app.settings.buffer_notify.insert(pref.clone(), NotifyLevel::All);
    assert_eq!(app.summary(&id).highlights, 2);
    app.settings.buffer_notify.insert(pref.clone(), NotifyLevel::None);
    assert_eq!(app.summary(&id).highlights, 0);
    app.focused = false;
    notifications(&mut app);
    feed(&mut app, &[":ann!a@h PRIVMSG #a :me: again"]);
    assert!(notifications(&mut app).is_empty());
    app.settings.buffer_notify.insert(pref, NotifyLevel::All);
    feed(&mut app, &[":ann!a@h PRIVMSG #a :anything"]);
    assert_eq!(notifications(&mut app).len(), 1);
}
