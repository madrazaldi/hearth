//! Slash commands and plain text from the composer.

use crate::testing::*;
use crate::types::LineType;
use crate::{App, BufId, BufferKind};

fn in_channel(caps: &[&str]) -> App {
    let mut app = new_app();
    connect(&mut app, caps, &[]);
    feed(&mut app, &[":me!u@h JOIN #a", ":srv 353 me = #a :@me ann", ":srv 366 me #a :End"]);
    sent(&mut app);
    app
}

fn server_id() -> BufId {
    BufId::server(CONN)
}

fn run(app: &mut App, input: &str) -> bool {
    let id = id_of(app, "#a");
    app.run_input(&id, input, None)
}

fn run_in(app: &mut App, id: &BufId, input: &str) -> bool {
    app.run_input(id, input, None)
}

fn toasts(app: &App) -> Vec<String> {
    app.toast_texts().into_iter().map(String::from).collect()
}

fn query(app: &mut App, nick: &str) -> BufId {
    let key = app.conn_mut(CONN).unwrap().ensure(nick, Some(BufferKind::Query));
    BufId::new(CONN, key)
}

// ── channel commands ──────────────────────────────────────────

#[test]
fn join_adds_a_missing_prefix_passes_keys_and_opens_the_channel_on_join() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    sent(&mut app);
    run_in(&mut app, &server_id(), "/join hearth");
    run_in(&mut app, &server_id(), "/j #locked sekrit");
    assert_eq!(sent(&mut app), ["JOIN #hearth", "JOIN #locked sekrit"]);
    feed(&mut app, &[":me!u@h JOIN #locked"]);
    assert_eq!(buf(&app, "#locked").join_key, "sekrit");
    assert_eq!(app.active(), Some(&id_of(&app, "#locked")));
}

#[test]
fn join_of_a_channel_we_are_in_just_switches_to_it() {
    let mut app = in_channel(&[]);
    app.open(&server_id());
    run_in(&mut app, &server_id(), "/join #A");
    assert!(sent(&mut app).is_empty());
    assert_eq!(app.active(), Some(&id_of(&app, "#a")));
}

#[test]
fn part_with_and_without_an_explicit_channel() {
    let mut app = in_channel(&[]);
    run(&mut app, "/part see you all");
    run(&mut app, "/part #other bye now");
    run(&mut app, "/leave");
    assert_eq!(sent(&mut app), ["PART #a :see you all", "PART #other :bye now", "PART #a"]);
}

#[test]
fn topic_shows_or_sets_the_topic() {
    let mut app = in_channel(&["labeled-response"]);
    run(&mut app, "/topic");
    run(&mut app, "/topic new: topic here");
    let out = sent(&mut app);
    assert!(out[0].starts_with("@label=") && out[0].ends_with(" TOPIC #a"), "{}", out[0]);
    assert_eq!(out[1], "TOPIC #a :new: topic here");
}

#[test]
fn kick_ban_unban_op_and_friends_target_the_current_channel() {
    let mut app = in_channel(&[]);
    for c in ["/kick ann", "/kick ann stop that now", "/ban ann", "/ban *!*@bad.host", "/ban troll!*", "/unban *!*@bad.host", "/op ann bob", "/devoice ann"] {
        run(&mut app, c);
    }
    assert_eq!(
        sent(&mut app),
        [
            "KICK #a ann",
            "KICK #a ann :stop that now",
            "MODE #a +b ann!*@*",
            "MODE #a +b *!*@bad.host",
            "MODE #a +b troll!*",
            "MODE #a -b *!*@bad.host",
            "MODE #a +oo ann bob",
            "MODE #a -v ann",
        ]
    );
}

#[test]
fn mode_on_the_channel_another_channel_or_ourselves() {
    let mut app = in_channel(&[]);
    for c in ["/mode +m", "/mode", "/mode #b +i", "/mode me +i"] {
        run(&mut app, c);
    }
    assert_eq!(sent(&mut app), ["MODE #a +m", "MODE #a", "MODE #b +i", "MODE me +i"]);
}

#[test]
fn channel_only_commands_fail_with_usage_in_a_query() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    sent(&mut app);
    let q = query(&mut app, "ann");
    assert!(!run_in(&mut app, &q, "/kick bob"));
    assert!(!run_in(&mut app, &q, "/topic hi"));
    assert!(!run_in(&mut app, &q, "/me"));
    assert!(sent(&mut app).is_empty());
    assert_eq!(toasts(&app), ["Usage: This command needs a channel", "Usage: This command needs a channel", "Usage: /me <action>"]);
}

// ── messaging commands ────────────────────────────────────────

#[test]
fn msg_sends_to_a_nick_and_opens_a_quiet_query() {
    let mut app = in_channel(&[]);
    assert!(run(&mut app, "/msg ann hello there"));
    assert_eq!(sent(&mut app), ["PRIVMSG ann :hello there"]);
    let got: Vec<(&str, bool)> = buf(&app, "ann").lines().iter().map(|l| (l.text.as_str(), l.is_self)).collect();
    assert_eq!(got, [("hello there", true)]);
    assert!(!run(&mut app, "/msg ann"));
    assert!(toasts(&app).contains(&"Usage: /msg <target> <text>".to_string()));
}

#[test]
fn msg_with_echo_message_waits_for_the_echo() {
    let mut app = in_channel(&["echo-message"]);
    run(&mut app, "/m ann hi");
    assert!(buf(&app, "ann").lines().is_empty());
}

#[test]
fn me_sends_a_ctcp_action() {
    let mut app = in_channel(&[]);
    run(&mut app, "/me waves");
    assert_eq!(sent(&mut app), ["PRIVMSG #a :\x01ACTION waves\x01"]);
    let l = buf(&app, "#a").lines().last().unwrap();
    assert_eq!((l.kind, l.text.as_str(), l.is_self), (LineType::Action, "waves", true));
}

#[test]
fn notice_and_ctcp() {
    let mut app = in_channel(&[]);
    for c in ["/notice ann heads up", "/ctcp ann version", "/ctcp ann ping 123"] {
        run(&mut app, c);
    }
    assert_eq!(sent(&mut app), ["NOTICE ann :heads up", "PRIVMSG ann :\x01VERSION\x01", "PRIVMSG ann :\x01PING 123\x01"]);
}

#[test]
fn query_opens_a_conversation_and_optionally_sends() {
    let mut app = in_channel(&[]);
    run(&mut app, "/q bob hey");
    assert_eq!(app.active(), Some(&id_of(&app, "bob")));
    assert_eq!(sent(&mut app), ["PRIVMSG bob :hey"]);
}

// ── client-side commands ──────────────────────────────────────

#[test]
fn notify_sets_the_buffer_level_and_mute_unmute_are_shortcuts() {
    use crate::types::NotifyLevel;
    let mut app = in_channel(&[]);
    let pref = buf(&app, "#a").pref_key();
    run(&mut app, "/notify ALL");
    assert_eq!(app.settings.buffer_notify.get(&pref), Some(&NotifyLevel::All));
    run(&mut app, "/mute");
    assert_eq!(app.settings.buffer_notify.get(&pref), Some(&NotifyLevel::None));
    run(&mut app, "/unmute");
    assert!(!app.settings.buffer_notify.contains_key(&pref));
    assert!(!run(&mut app, "/notify loud"));
    assert!(!run_in(&mut app, &server_id(), "/notify all"));
}

#[test]
fn ignore_and_unignore_manage_masks() {
    let mut app = in_channel(&[]);
    for c in ["/ignore troll", "/ignore *@spam.host", "/ignore troll"] {
        run(&mut app, c);
    }
    assert_eq!(app.settings.ignores, ["troll!*@*", "*!*@spam.host"]);
    feed(&mut app, &[":Troll!t@x PRIVMSG #a :hi"]);
    assert!(buf(&app, "#a").lines().is_empty());
    run(&mut app, "/ignore");
    assert_eq!(buf(&app, "#a").lines().last().unwrap().text, "Ignoring: troll!*@*, *!*@spam.host");
    run(&mut app, "/unignore troll");
    run(&mut app, "/unignore nobody");
    assert_eq!(app.settings.ignores, ["*!*@spam.host"]);
    let t = toasts(&app);
    assert_eq!(t[t.len() - 2..], ["No longer ignoring troll", "nobody was not ignored"]);
}

#[test]
fn clear_empties_the_buffer() {
    let mut app = in_channel(&[]);
    feed(&mut app, &[":ann!a@h PRIVMSG #a :one"]);
    run(&mut app, "/clear");
    assert!(buf(&app, "#a").lines().is_empty());
    assert!(sent(&mut app).is_empty());
}

#[test]
fn whois_in_a_query_defaults_to_the_peer() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    let q = query(&mut app, "ann");
    sent(&mut app);
    run_in(&mut app, &q, "/whois");
    run_in(&mut app, &q, "/w bob");
    assert_eq!(sent(&mut app), ["WHOIS ann ann", "WHOIS bob bob"]);
}

// ── raw and unknown commands ──────────────────────────────────

#[test]
fn sends_unknown_commands_verbatim_with_the_name_upper_cased() {
    let mut app = in_channel(&[]);
    assert!(run(&mut app, "/cs op #a me"));
    run(&mut app, "/knock #secret please let me in");
    run(&mut app, "/lusers");
    assert_eq!(sent(&mut app), ["CS op #a me", "KNOCK #secret please let me in", "LUSERS"]);
}

#[test]
fn raw_and_quote_pass_the_line_through() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    sent(&mut app);
    run_in(&mut app, &server_id(), "/raw PRIVMSG ann :hi  there");
    run_in(&mut app, &server_id(), "/quote WHO #a");
    assert_eq!(sent(&mut app), ["PRIVMSG ann :hi  there", "WHO #a"]);
}

#[test]
fn a_double_slash_sends_a_literal_slash() {
    let mut app = in_channel(&[]);
    run(&mut app, "//shrug");
    assert_eq!(sent(&mut app), ["PRIVMSG #a :/shrug"]);
}

// ── plain text ────────────────────────────────────────────────

#[test]
fn ignores_blank_input_and_refuses_to_chat_in_the_server_buffer() {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    sent(&mut app);
    assert!(!run_in(&mut app, &server_id(), "   "));
    assert!(!run_in(&mut app, &server_id(), "hello"));
    assert!(sent(&mut app).is_empty());
    assert!(toasts(&app)[0].contains("server buffer"));
}

#[test]
fn sends_each_non_empty_line_separately_replying_only_with_the_first() {
    let mut app = in_channel(&["message-tags"]);
    let id = id_of(&app, "#a");
    app.run_input(&id, "one\n\n  \ntwo\nthree", Some("m1"));
    assert_eq!(sent(&mut app), ["@+draft/reply=m1 PRIVMSG #a :one", "PRIVMSG #a :two", "PRIVMSG #a :three"]);
}

#[test]
fn splits_long_messages_on_word_boundaries_within_the_line_limit() {
    let mut app = in_channel(&[]);
    let words: Vec<String> = (0..200).map(|i| format!("word{i}")).collect();
    let text = words.join(" ");
    run(&mut app, &text);
    let out = sent(&mut app);
    assert!(out.len() > 1);
    let parts: Vec<String> = out.iter().map(|l| l.strip_prefix("PRIVMSG #a :").unwrap().to_string()).collect();
    for p in &parts {
        let framed = format!(":me!{}@{} PRIVMSG #a :{p}", "u".repeat(10), "h".repeat(63));
        assert!(framed.len() <= 510, "{} bytes", framed.len());
        assert!(!p.starts_with(' ') && !p.ends_with(' '));
    }
    // Nothing lost, nothing broken mid-word.
    assert_eq!(parts.join(" "), text);
    let split: Vec<&str> = parts.iter().flat_map(|p| p.split(' ')).collect();
    assert_eq!(split, words.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(texts(&app, "#a"), parts);
}

#[test]
fn splits_multi_byte_text_without_breaking_characters() {
    let mut app = in_channel(&[]);
    let text = "\u{e9}".repeat(500);
    run(&mut app, &text);
    let parts: Vec<String> = sent(&mut app).iter().map(|l| l.strip_prefix("PRIVMSG #a :").unwrap().to_string()).collect();
    assert!(parts.len() > 1);
    assert_eq!(parts.concat(), text);
    assert!(parts.iter().all(|p| p.len() <= 400));
}
