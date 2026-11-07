//! Per-network highlight words and
//! "never highlight" senders, on top of the global list.

use crate::testing::*;
use crate::types::NetworkHighlights;
use crate::App;

fn setup() -> App {
    let mut app = new_app();
    app.focused = false;
    connect(&mut app, &[], &[]);
    feed(&mut app, &[":me!u@h JOIN #a"]);
    notifications(&mut app);
    app
}

fn highlights(app: &App) -> Vec<(String, bool)> {
    buf(app, "#a").lines().iter().map(|l| (l.text.clone(), l.highlight)).collect()
}

fn rules(words: &[&str], never: &[&str]) -> NetworkHighlights {
    NetworkHighlights { words: words.iter().map(|s| s.to_string()).collect(), never: never.iter().map(|s| s.to_string()).collect() }
}

fn pairs(v: &[(&str, bool)]) -> Vec<(String, bool)> {
    v.iter().map(|(t, h)| (t.to_string(), *h)).collect()
}

#[test]
fn adds_words_and_regexes_for_this_server_to_the_nick_and_global_words() {
    let mut app = setup();
    app.update_settings(|s| {
        s.highlight_words = vec!["hearth".into()];
        s.network_highlights.insert("c".into(), rules(&["deploy", "/\\bci (failed|broke)\\b/i"], &[]));
    });
    feed(
        &mut app,
        &[
            ":ann!a@h PRIVMSG #a :me: ping",
            ":ann!a@h PRIVMSG #a :hearth is nice",
            ":ann!a@h PRIVMSG #a :deploy done",
            ":ann!a@h PRIVMSG #a :CI broke again",
            ":ann!a@h PRIVMSG #a :redeploying",
        ],
    );
    assert_eq!(
        highlights(&app),
        pairs(&[("me: ping", true), ("hearth is nice", true), ("deploy done", true), ("CI broke again", true), ("redeploying", false)])
    );
    assert_eq!(notifications(&mut app).len(), 4);
}

#[test]
fn does_not_apply_another_servers_rules() {
    let mut app = setup();
    app.update_settings(|s| {
        s.network_highlights.insert("other".into(), rules(&["deploy"], &[]));
    });
    feed(&mut app, &[":ann!a@h PRIVMSG #a :deploy done"]);
    assert_eq!(highlights(&app), pairs(&[("deploy done", false)]));
}

#[test]
fn never_highlights_listed_nicks_or_masks_even_for_our_nick() {
    let mut app = setup();
    app.update_settings(|s| {
        s.network_highlights.insert("c".into(), rules(&[], &["GitBot", "*!*@relay.example"]));
    });
    feed(&mut app, &[":gitbot!g@h PRIVMSG #a :me pushed 3 commits", ":bridge!b@relay.example PRIVMSG #a :<x> me: hi", ":ann!a@h PRIVMSG #a :me: hi"]);
    assert_eq!(highlights(&app), pairs(&[("me pushed 3 commits", false), ("<x> me: hi", false), ("me: hi", true)]));
    assert_eq!(notifications(&mut app).len(), 1);
}

#[test]
fn applies_to_replayed_history_too() {
    let mut app = setup();
    app.update_settings(|s| {
        s.network_highlights.insert("c".into(), rules(&["deploy"], &["gitbot"]));
    });
    let line = |app: &App, raw: &str| crate::incoming::history_line(app, app.conn(CONN).unwrap(), &parse(raw)).unwrap().highlight;
    assert!(line(&app, ":ann!a@h PRIVMSG #a :deploy now"));
    assert!(!line(&app, ":GitBot!g@h PRIVMSG #a :deploy now"));
}
