//! Raw IRC lines in, App state out.

use crate::irc::parse_time;
use crate::state::TypingState;
use crate::testing::*;
use crate::types::{Line, LineType};
use crate::{App, BufferKind};

fn conn(app: &App) -> &crate::Conn {
    app.conn(CONN).unwrap()
}

fn lines<'a>(app: &'a App, name: &str) -> &'a [Line] {
    buf(app, name).lines()
}

fn types(app: &App, name: &str) -> Vec<LineType> {
    lines(app, name).iter().map(|l| l.kind).collect()
}

/// Members with their prefixes, in list order.
fn nicks(app: &App, name: &str) -> Vec<String> {
    buf(app, name).sorted_members(conn(app).prefix()).iter().map(|m| format!("{}{}", m.prefix, m.nick)).collect()
}

fn last<'a>(app: &'a App, name: &str) -> &'a Line {
    lines(app, name).last().unwrap_or_else(|| panic!("{name} has no lines"))
}

/// Join `chan` and load a NAMES list.
fn join_with(app: &mut App, chan: &str, names: &str) {
    feed(app, &[&format!(":me!u@h JOIN {chan}"), &format!(":srv 353 me = {chan} :{names}"), &format!(":srv 366 me {chan} :End of /NAMES list.")]);
}

fn setup(caps: &[&str], isupport: &[(&str, &str)]) -> App {
    let mut app = new_app();
    connect(&mut app, caps, isupport);
    app
}

// ── NICK ──────────────────────────────────────────────────────

#[test]
fn nick_renames_another_user_in_every_shared_channel_and_their_query() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "@me ann bob");
    join_with(&mut app, "#b", "me +ann");
    join_with(&mut app, "#c", "me bob");
    feed(&mut app, &[":ann!a@h PRIVMSG me :hi"]);
    feed(&mut app, &[":ann!a@h NICK anna"]);

    assert_eq!(nicks(&app, "#a"), ["@me", "anna", "bob"]);
    assert_eq!(nicks(&app, "#b"), ["+anna", "me"]);
    assert_eq!(types(&app, "#a"), [LineType::Nick]);
    assert_eq!((lines(&app, "#a")[0].nick.as_str(), lines(&app, "#a")[0].target.as_deref()), ("ann", Some("anna")));
    assert_eq!(types(&app, "#b"), [LineType::Nick]);
    assert!(lines(&app, "#c").is_empty());

    // The private query follows the rename and is filed under the new name.
    assert!(conn(&app).buffer("ann").is_none());
    let q = buf(&app, "anna");
    assert_eq!((q.name.as_str(), q.kind), ("anna", BufferKind::Query));
    assert_eq!(types(&app, "anna"), [LineType::Privmsg, LineType::Nick]);
    feed(&mut app, &[":anna!a@h PRIVMSG me :still me"]);
    assert_eq!(texts(&app, "anna"), ["hi", "", "still me"]);
}

#[test]
fn nick_does_not_merge_a_query_into_an_existing_one_for_the_new_nick() {
    let mut app = setup(&[], &[]);
    feed(&mut app, &[":ann!a@h PRIVMSG me :one", ":anna!a@h PRIVMSG me :two"]);
    feed(&mut app, &[":ann!a@h NICK anna"]);
    assert_eq!(buf(&app, "ann").name, "ann");
    assert_eq!(texts(&app, "anna"), ["two"]);
}

#[test]
fn nick_updates_the_query_name_on_a_case_only_change() {
    let mut app = setup(&[], &[]);
    feed(&mut app, &[":ann!a@h PRIVMSG me :hi", ":ann!a@h NICK Ann"]);
    assert_eq!(buf(&app, "ann").name, "Ann");
    assert_eq!(buf(&app, "Ann").member("ann").unwrap().nick, "Ann");
}

#[test]
fn nick_tracks_our_own_change_everywhere() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "@me ann");
    feed(&mut app, &[":ann!a@h PRIVMSG me :hi"]);
    feed(&mut app, &[":me!u@h NICK me2"]);
    assert_eq!(conn(&app).nick, "me2");
    assert_eq!(nicks(&app, "#a"), ["@me2", "ann"]);
    let l = last(&app, "#a");
    assert_eq!((l.kind, l.is_self, l.target.as_deref()), (LineType::Nick, true, Some("me2")));
    // Queries and the server buffer hear about it too.
    assert_eq!(types(&app, "ann"), [LineType::Privmsg, LineType::Nick]);
    let s = conn(&app).server.lines().last().unwrap();
    assert_eq!((s.kind, s.target.as_deref()), (LineType::Nick, Some("me2")));
    // Mentions now use the new nick.
    feed(&mut app, &[":ann!a@h PRIVMSG #a :me2: yo", ":ann!a@h PRIVMSG #a :me: old"]);
    let hl: Vec<bool> = lines(&app, "#a").iter().rev().take(2).rev().map(|l| l.highlight).collect();
    assert_eq!(hl, [true, false]);
}

// ── KICK, PART, QUIT ──────────────────────────────────────────

#[test]
fn kicking_someone_else_removes_only_them() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "@me ann bob");
    feed(&mut app, &[":me!u@h KICK #a bob :bye"]);
    assert_eq!(nicks(&app, "#a"), ["@me", "ann"]);
    assert!(buf(&app, "#a").joined);
    let l = last(&app, "#a");
    assert_eq!((l.kind, l.nick.as_str(), l.target.as_deref(), l.text.as_str(), l.highlight), (LineType::Kick, "me", Some("bob"), "bye", false));
}

#[test]
fn being_kicked_leaves_the_channel_and_highlights() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me @ann");
    feed(&mut app, &[":ann!a@h KICK #a me :out"]);
    assert!(!buf(&app, "#a").joined);
    assert_eq!(buf(&app, "#a").member_count(), 0);
    let l = last(&app, "#a");
    assert_eq!((l.kind, l.target.as_deref(), l.text.as_str(), l.highlight), (LineType::Kick, Some("me"), "out", true));
}

#[test]
fn part_removes_the_member_and_our_own_part_clears_the_list() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me ann bob");
    feed(&mut app, &[":ann!a@h PART #a :later"]);
    assert_eq!(nicks(&app, "#a"), ["bob", "me"]);
    let l = last(&app, "#a");
    assert_eq!((l.kind, l.nick.as_str(), l.text.as_str()), (LineType::Part, "ann", "later"));
    feed(&mut app, &[":me!u@h PART #a"]);
    assert!(!buf(&app, "#a").joined);
    assert_eq!(buf(&app, "#a").member_count(), 0);
}

#[test]
fn quit_only_shows_in_buffers_shared_with_the_user() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me ann");
    join_with(&mut app, "#b", "me ann");
    join_with(&mut app, "#c", "me bob");
    feed(&mut app, &[":ann!a@h PRIVMSG me :hi"]);
    feed(&mut app, &["@+typing=active :ann!a@h TAGMSG #a"]);
    feed(&mut app, &[":ann!a@h QUIT :Quit: gone"]);
    let l = last(&app, "#a");
    assert_eq!((l.kind, l.nick.as_str(), l.text.as_str()), (LineType::Quit, "ann", "Quit: gone"));
    assert_eq!(types(&app, "#b"), [LineType::Quit]);
    assert!(lines(&app, "#c").is_empty());
    assert_eq!(types(&app, "ann"), [LineType::Privmsg, LineType::Quit]);
    assert!(buf(&app, "#a").member("ann").is_none());
    assert!(!buf(&app, "#a").typing.contains_key("ann"));
}

#[test]
fn quit_reaches_a_query_even_when_the_member_list_is_empty() {
    let mut app = setup(&[], &[]);
    app.conn_mut(CONN).unwrap().ensure("ann", Some(BufferKind::Query));
    feed(&mut app, &[":ann!a@h QUIT :bye"]);
    assert_eq!(types(&app, "ann"), [LineType::Quit]);
}

// ── MODE ──────────────────────────────────────────────────────

fn prefix_of(app: &App, chan: &str, nick: &str) -> String {
    buf(app, chan).member(nick).unwrap().prefix.clone()
}

#[test]
fn mode_adds_and_removes_prefixes_with_multi_prefix_keeping_rank_order() {
    let mut app = setup(&["multi-prefix"], &[("PREFIX", "(qaohv)~&@%+")]);
    join_with(&mut app, "#a", "me ann +bob");
    feed(&mut app, &[":srv MODE #a +v ann", ":srv MODE #a +o ann"]);
    assert_eq!(prefix_of(&app, "#a", "ann"), "@+");
    feed(&mut app, &[":srv MODE #a +q ann"]);
    assert_eq!(prefix_of(&app, "#a", "ann"), "~@+");
    feed(&mut app, &[":srv MODE #a -o ann"]);
    assert_eq!(prefix_of(&app, "#a", "ann"), "~+");
    feed(&mut app, &[":srv MODE #a +vv ann bob"]);
    assert_eq!(prefix_of(&app, "#a", "bob"), "+");
    assert_eq!(prefix_of(&app, "#a", "ann"), "~+");
}

#[test]
fn mode_applies_several_modes_with_mixed_signs_and_arguments_in_one_line() {
    let mut app = setup(&[], &[("CHANMODES", "beI,k,l,imnpst")]);
    join_with(&mut app, "#a", "me @ann bob +cat");
    feed(&mut app, &[":ann!a@h MODE #a +ov-v+lk-o bob bob cat 50 sekrit ann"]);
    // bob gets both +o and +v; cat loses voice; ann deops herself.
    assert_eq!(prefix_of(&app, "#a", "bob"), "@+");
    assert_eq!(prefix_of(&app, "#a", "cat"), "");
    assert_eq!(prefix_of(&app, "#a", "ann"), "");
    assert_eq!(buf(&app, "#a").join_key, "sekrit");
    let l = last(&app, "#a");
    assert_eq!((l.kind, l.nick.as_str(), l.text.as_str()), (LineType::Mode, "ann", "+ov-v+lk-o bob bob cat 50 sekrit ann"));
    assert_eq!(nicks(&app, "#a"), ["@+bob", "ann", "cat", "me"]);
}

#[test]
fn mode_does_not_consume_an_argument_for_minus_l_but_does_for_bans() {
    let mut app = setup(&[], &[("CHANMODES", "beI,k,l,imnpst")]);
    join_with(&mut app, "#a", "me ann");
    feed(&mut app, &[":srv MODE #a -l+b+v *!*@spam ann"]);
    assert_eq!(prefix_of(&app, "#a", "ann"), "+");
}

#[test]
fn mode_forgets_the_key_on_minus_k_and_ignores_a_hidden_key() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "@me");
    feed(&mut app, &[":srv MODE #a +k real"]);
    assert_eq!(buf(&app, "#a").join_key, "real");
    feed(&mut app, &[":srv MODE #a +k *"]);
    assert_eq!(buf(&app, "#a").join_key, "real");
    feed(&mut app, &[":srv MODE #a -k *"]);
    assert_eq!(buf(&app, "#a").join_key, "");
}

#[test]
fn mode_records_channel_modes_from_324_and_user_modes_on_us() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "@me");
    feed(&mut app, &[":srv 324 me #a +ntkl hunter2 10"]);
    assert_eq!(buf(&app, "#a").modes, "+ntkl hunter2 10");
    assert_eq!(buf(&app, "#a").join_key, "hunter2");
    feed(&mut app, &[":me MODE me :+iw"]);
    let l = conn(&app).server.lines().last().unwrap();
    assert_eq!((l.kind, l.text.as_str()), (LineType::Mode, "+iw"));
}

#[test]
fn mode_parses_multi_prefix_names_and_ranks_members() {
    let mut app = setup(&["multi-prefix"], &[("PREFIX", "(qaohv)~&@%+")]);
    join_with(&mut app, "#a", "+zed @+ann ~@bob %cat me");
    assert_eq!(prefix_of(&app, "#a", "ann"), "@+");
    assert_eq!(nicks(&app, "#a"), ["~@bob", "@+ann", "%cat", "+zed", "me"]);
}

#[test]
fn mode_stamps_the_senders_highest_prefix_on_channel_messages() {
    let mut app = setup(&["multi-prefix"], &[("PREFIX", "(ov)@+")]);
    join_with(&mut app, "#a", "me @+ann zed");
    feed(&mut app, &[":ann!a@h PRIVMSG #a :hi", ":zed!z@h PRIVMSG #a :yo", ":ann!a@h PART #a"]);
    let prefixes: Vec<Option<&str>> = lines(&app, "#a").iter().map(|l| l.prefix.as_deref()).collect();
    assert_eq!(prefixes, [Some("@"), None, None]);
    // The prefix is what they had at the time, not what they have now.
    feed(&mut app, &[":srv MODE #a +o zed", ":zed!z@h PRIVMSG #a :op now"]);
    let zed: Vec<Option<&str>> = lines(&app, "#a").iter().filter(|l| l.nick == "zed").map(|l| l.prefix.as_deref()).collect();
    assert_eq!(zed, [None, Some("@")]);
}

// ── IRCv3 membership updates ──────────────────────────────────

#[test]
fn extended_join_records_account_and_realname() {
    let mut app = setup(&["extended-join"], &[]);
    join_with(&mut app, "#a", "me");
    feed(&mut app, &[":ann!a@host.example JOIN #a annacct :Ann Example", ":bob!b@h JOIN #a * :Bob"]);
    let ann = buf(&app, "#a").member("ann").unwrap();
    assert_eq!(
        (ann.account.as_deref(), ann.realname.as_deref(), ann.user.as_deref(), ann.host.as_deref()),
        (Some("annacct"), Some("Ann Example"), Some("a"), Some("host.example"))
    );
    assert_eq!(buf(&app, "#a").member("bob").unwrap().account, None);
    let got: Vec<(LineType, &str, Option<&str>)> =
        lines(&app, "#a").iter().map(|l| (l.kind, l.nick.as_str(), l.data.as_ref().and_then(|d| d.get("host")).map(String::as_str))).collect();
    assert_eq!(got, [(LineType::Join, "ann", Some("a@host.example")), (LineType::Join, "bob", Some("b@h"))]);
}

#[test]
fn userhost_in_names_fills_user_and_host() {
    let mut app = setup(&["userhost-in-names"], &[]);
    join_with(&mut app, "#a", "@me!u@h ann!a@ann.host");
    let ann = buf(&app, "#a").member("ann").unwrap();
    assert_eq!((ann.nick.as_str(), ann.user.as_deref(), ann.host.as_deref(), ann.prefix.as_str()), ("ann", Some("a"), Some("ann.host"), ""));
    assert_eq!(prefix_of(&app, "#a", "me"), "@");
}

#[test]
fn keeps_away_and_account_across_a_names_refresh() {
    let mut app = setup(&["away-notify", "account-notify"], &[]);
    join_with(&mut app, "#a", "me ann");
    feed(&mut app, &[":ann!a@h AWAY :lunch", ":ann!a@h ACCOUNT annacct"]);
    feed(&mut app, &[":srv 353 me = #a :me @ann", ":srv 366 me #a :End"]);
    let ann = buf(&app, "#a").member("ann").unwrap();
    assert_eq!((ann.prefix.as_str(), ann.away, ann.account.as_deref()), ("@", true, Some("annacct")));
}

#[test]
fn chghost_setname_account_notify_and_away_notify_patch_every_channel() {
    let mut app = setup(&["chghost", "setname", "account-notify", "away-notify"], &[]);
    join_with(&mut app, "#a", "me ann");
    join_with(&mut app, "#b", "me ann");
    feed(&mut app, &[":ann!a@old CHGHOST newuser new.host", ":ann!newuser@new.host SETNAME :Ann Two"]);
    feed(&mut app, &[":ann!a@h ACCOUNT annacct", ":ann!a@h AWAY :brb"]);
    for c in ["#a", "#b"] {
        let m = buf(&app, c).member("ann").unwrap();
        assert_eq!(
            (m.user.as_deref(), m.host.as_deref(), m.realname.as_deref(), m.account.as_deref(), m.away),
            (Some("newuser"), Some("new.host"), Some("Ann Two"), Some("annacct"), true)
        );
    }
    feed(&mut app, &[":ann!a@h ACCOUNT *", ":ann!a@h AWAY"]);
    let m = buf(&app, "#a").member("ann").unwrap();
    assert_eq!((m.account.as_deref(), m.away), (None, false));
    // None of these produce chat lines.
    assert!(lines(&app, "#a").is_empty());
}

#[test]
fn tracks_our_own_away_state() {
    let mut app = setup(&[], &[]);
    feed(&mut app, &[":srv 306 me :You have been marked as being away"]);
    assert!(conn(&app).away);
    feed(&mut app, &[":srv 305 me :You are no longer marked as being away"]);
    assert!(!conn(&app).away);
}

// ── TOPIC and NAMES ───────────────────────────────────────────

#[test]
fn stores_the_topic_from_332_and_333_without_a_line() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me");
    feed(&mut app, &[":srv 332 me #a :Welcome to #a", ":srv 333 me #a ann!a@h 1700000000"]);
    let b = buf(&app, "#a");
    assert_eq!((b.topic.as_str(), b.topic_by.as_str(), b.topic_time), ("Welcome to #a", "ann", 1_700_000_000_000));
    assert!(b.lines().is_empty());
    feed(&mut app, &[":srv 331 me #a :No topic is set"]);
    assert_eq!(buf(&app, "#a").topic, "");
}

#[test]
fn a_live_topic_updates_state_and_adds_a_line() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me");
    feed(&mut app, &["@time=2026-01-02T03:04:05.000Z :bob!b@h TOPIC #a :new topic"]);
    let b = buf(&app, "#a");
    assert_eq!((b.topic.as_str(), b.topic_by.as_str(), Some(b.topic_time)), ("new topic", "bob", parse_time("2026-01-02T03:04:05.000Z")));
    let l = last(&app, "#a");
    assert_eq!((l.kind, l.nick.as_str(), l.text.as_str()), (LineType::Topic, "bob", "new topic"));
}

#[test]
fn accumulates_names_across_353s_and_swaps_on_366() {
    let mut app = setup(&[], &[]);
    feed(&mut app, &[":me!u@h JOIN #a", ":srv 353 me = #a :me ann", ":srv 353 me = #a :@bob"]);
    // Nothing visible until the list is complete.
    assert_eq!(buf(&app, "#a").member_count(), 0);
    feed(&mut app, &[":srv 366 me #a :End"]);
    assert_eq!(nicks(&app, "#a"), ["@bob", "ann", "me"]);
    // A refresh replaces the list rather than adding to it.
    feed(&mut app, &[":srv 353 me = #a :me", ":srv 366 me #a :End"]);
    assert_eq!(nicks(&app, "#a"), ["me"]);
}

#[test]
fn ignores_names_for_channels_we_have_no_buffer_for() {
    let mut app = setup(&[], &[]);
    feed(&mut app, &[":srv 353 me = #nope :ann bob", ":srv 366 me #nope :End"]);
    assert!(conn(&app).buffer("#nope").is_none());
}

// ── echo-message, labels, redaction ───────────────────────────

#[test]
fn shows_our_message_once_when_the_server_echoes_it() {
    let mut app = setup(&["echo-message", "message-tags"], &[]);
    join_with(&mut app, "#a", "me");
    let id = id_of(&app, "#a");
    app.send_message(&id, "hello", crate::state::SendKind::Privmsg, false, None);
    assert!(sent(&mut app).contains(&"PRIVMSG #a :hello".to_string()));
    assert!(lines(&app, "#a").is_empty());
    feed(&mut app, &["@msgid=e1 :me!u@h PRIVMSG #a :hello"]);
    let got: Vec<(&str, bool, Option<&str>)> = lines(&app, "#a").iter().map(|l| (l.text.as_str(), l.is_self, l.msgid.as_deref())).collect();
    assert_eq!(got, [("hello", true, Some("e1"))]);
}

#[test]
fn files_echoed_private_messages_under_the_recipient() {
    let mut app = setup(&["echo-message"], &[]);
    feed(&mut app, &[":me!u@h PRIVMSG ann :psst"]);
    assert_eq!(buf(&app, "ann").kind, BufferKind::Query);
    let l = &lines(&app, "ann")[0];
    assert_eq!((l.is_self, l.text.as_str(), l.highlight), (true, "psst", false));
    assert!(notifications(&mut app).is_empty());
}

#[test]
fn reactions_wait_for_the_echo() {
    let mut app = setup(&["echo-message", "message-tags"], &[]);
    join_with(&mut app, "#a", "me");
    feed(&mut app, &["@msgid=m1 :ann!a@h PRIVMSG #a :ship it"]);
    let id = id_of(&app, "#a");
    let first = lines(&app, "#a")[0].clone();
    app.react(&id, &first, "\u{1f680}");
    assert!(lines(&app, "#a")[0].reactions.is_empty());
    feed(&mut app, &["@+draft/react=\u{1f680};+draft/reply=m1 :me!u@h TAGMSG #a"]);
    feed(&mut app, &["@+draft/react=\u{1f680};+draft/reply=m1 :Me!u@h TAGMSG #a"]);
    assert_eq!(lines(&app, "#a")[0].reactions, [("\u{1f680}".to_string(), vec!["me".to_string()])]);
}

#[test]
fn routes_labeled_replies_and_errors_to_the_asking_buffer() {
    let mut app = setup(&["labeled-response"], &[]);
    join_with(&mut app, "#a", "me");
    join_with(&mut app, "#b", "me");
    app.open(&id_of(&app, "#b"));
    let a = id_of(&app, "#a");
    app.conn_mut(CONN).unwrap().send_from(&a.key, "TOPIC #zzz");
    let line = sent(&mut app).pop().unwrap();
    let label = line.strip_prefix("@label=").and_then(|r| r.split(' ').next()).unwrap().to_string();
    feed(&mut app, &[&format!("@label={label} :srv 403 me #zzz :No such channel")]);
    let l = last(&app, "#a");
    assert_eq!((l.kind, l.text.as_str()), (LineType::Error, "#zzz No such channel"));
    assert!(lines(&app, "#b").is_empty());
}

#[test]
fn marks_redacted_messages_in_channels_and_queries() {
    let mut app = setup(&["draft/message-redaction", "message-tags"], &[]);
    join_with(&mut app, "#a", "me");
    feed(&mut app, &["@msgid=m1 :ann!a@h PRIVMSG #a :oops", "@msgid=m2 :ann!a@h PRIVMSG #a :fine"]);
    feed(&mut app, &["@+draft/react=\u{1f44d};+draft/reply=m1 :bob!b@h TAGMSG #a"]);
    feed(&mut app, &[":ann!a@h REDACT #a m1 :typo"]);
    let got: Vec<(Option<&str>, bool)> = lines(&app, "#a").iter().map(|l| (l.msgid.as_deref(), l.redacted)).collect();
    assert_eq!(got, [(Some("m1"), true), (Some("m2"), false)]);
    assert!(lines(&app, "#a")[0].reactions.is_empty());

    feed(&mut app, &["@msgid=p1 :ann!a@h PRIVMSG me :secret"]);
    feed(&mut app, &[":ann!a@h REDACT me p1"]);
    assert!(lines(&app, "ann")[0].redacted);
}

#[test]
fn lets_ops_and_authors_redact() {
    let mut app = setup(&["draft/message-redaction"], &[]);
    join_with(&mut app, "#a", "me ann");
    feed(&mut app, &["@msgid=m1 :ann!a@h PRIVMSG #a :x", "@msgid=m2 :me!u@h PRIVMSG #a :mine"]);
    let id = id_of(&app, "#a");
    let (theirs, mine) = (lines(&app, "#a")[0].clone(), lines(&app, "#a")[1].clone());
    assert!(!app.can_redact(&id, &theirs));
    assert!(app.can_redact(&id, &mine));
    feed(&mut app, &[":srv MODE #a +o me"]);
    assert!(app.can_redact(&id, &theirs));
    app.redact(&id, &theirs);
    assert_eq!(sent(&mut app).last().map(String::as_str), Some("REDACT #a m1"));
}

// ── Typing and read markers ───────────────────────────────────

#[test]
fn tracks_typing_notifications_per_buffer() {
    let mut app = setup(&["message-tags"], &[]);
    join_with(&mut app, "#a", "me ann bob");
    feed(&mut app, &["@+typing=active :ann!a@h TAGMSG #a", "@+typing=paused :bob!b@h TAGMSG #a"]);
    let typing = &buf(&app, "#a").typing;
    assert_eq!(typing.keys().cloned().collect::<Vec<_>>(), ["ann", "bob"]);
    assert!(typing["bob"] - typing["ann"] > 20_000);
    feed(&mut app, &["@+typing=done :bob!b@h TAGMSG #a"]);
    assert!(!buf(&app, "#a").typing.contains_key("bob"));
    // Sending a message clears the indicator.
    feed(&mut app, &[":ann!a@h PRIVMSG #a :done typing"]);
    assert!(buf(&app, "#a").typing.is_empty());
}

#[test]
fn files_private_typing_under_the_sender_and_skips_our_own() {
    let mut app = setup(&["message-tags"], &[]);
    feed(&mut app, &[":ann!a@h PRIVMSG me :hi", "@+typing=active :ann!a@h TAGMSG me"]);
    assert!(buf(&app, "ann").typing.contains_key("ann"));
    join_with(&mut app, "#a", "me");
    feed(&mut app, &["@+typing=active :me!u@h TAGMSG #a"]);
    assert!(buf(&app, "#a").typing.is_empty());
}

#[test]
fn respects_the_show_typing_setting() {
    let mut app = setup(&["message-tags"], &[]);
    app.settings.show_typing = false;
    join_with(&mut app, "#a", "me ann");
    feed(&mut app, &["@+typing=active :ann!a@h TAGMSG #a"]);
    assert!(buf(&app, "#a").typing.is_empty());
}

#[test]
fn sends_typing_only_when_allowed_and_throttled() {
    let mut app = setup(&["message-tags"], &[("CLIENTTAGDENY", "*,-typing")]);
    join_with(&mut app, "#a", "me");
    sent(&mut app);
    let id = id_of(&app, "#a");
    app.send_typing(&id, TypingState::Active);
    app.send_typing(&id, TypingState::Active);
    app.send_typing(&id, TypingState::Done);
    assert_eq!(sent(&mut app), ["@+typing=active TAGMSG #a", "@+typing=done TAGMSG #a"]);
    let mut isupport = conn(&app).isupport.clone();
    isupport.insert("CLIENTTAGDENY".into(), "typing".into());
    app.conn_mut(CONN).unwrap().set_isupport(isupport);
    app.send_typing(&id, TypingState::Active);
    assert!(sent(&mut app).is_empty());
}

#[test]
fn moves_the_read_marker_forward_only() {
    let mut app = setup(&["draft/read-marker"], &[]);
    join_with(&mut app, "#a", "me");
    assert_eq!(buf(&app, "#a").read_marker, 0);
    feed(&mut app, &[":srv MARKREAD #a timestamp=2026-01-01T00:00:10.000Z"]);
    let t = parse_time("2026-01-01T00:00:10.000Z").unwrap();
    assert_eq!(buf(&app, "#a").read_marker, t);
    feed(&mut app, &[":srv MARKREAD #a timestamp=2026-01-01T00:00:05.000Z", ":srv MARKREAD #a *"]);
    assert_eq!(buf(&app, "#a").read_marker, t);
}

#[test]
fn asks_for_the_read_marker_of_a_new_query() {
    let mut app = setup(&["draft/read-marker"], &[]);
    sent(&mut app);
    feed(&mut app, &["@time=2026-01-01T00:00:01.000Z :ann!a@h PRIVMSG me :hi"]);
    assert!(sent(&mut app).contains(&"MARKREAD ann".to_string()));
    feed(&mut app, &[":srv MARKREAD ann timestamp=2026-01-01T00:00:01.000Z"]);
    assert_eq!(buf(&app, "ann").unread(), 0);
}

#[test]
fn counts_unread_lines_after_the_marker_and_syncs_mark_read() {
    let mut app = setup(&["draft/read-marker"], &[]);
    join_with(&mut app, "#a", "me");
    feed(&mut app, &["@time=2026-01-01T00:00:01.000Z :ann!a@h PRIVMSG #a :one", "@time=2026-01-01T00:00:02.000Z :ann!a@h PRIVMSG #a :two"]);
    feed(&mut app, &[":srv MARKREAD #a timestamp=2026-01-01T00:00:01.000Z"]);
    assert_eq!(buf(&app, "#a").unread(), 1);
    let id = id_of(&app, "#a");
    sent(&mut app);
    app.mark_read(&id);
    assert_eq!(buf(&app, "#a").unread(), 0);
    advance(&mut app, 500);
    assert_eq!(sent(&mut app).last().map(String::as_str), Some("MARKREAD #a timestamp=2026-01-01T00:00:02.000Z"));
}

// ── PRIVMSG, NOTICE and CTCP ──────────────────────────────────

#[test]
fn shows_actions_as_action_lines() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me ann");
    feed(&mut app, &[":ann!a@h PRIVMSG #a :\x01ACTION waves at me\x01", ":ann!a@h PRIVMSG #a :\x01ACTION\x01"]);
    let got: Vec<(LineType, &str, bool)> = lines(&app, "#a").iter().map(|l| (l.kind, l.text.as_str(), l.highlight)).collect();
    assert_eq!(got, [(LineType::Action, "waves at me", true), (LineType::Action, "", false)]);
}

#[test]
fn answers_ctcp_requests_and_reports_replies_without_chat_lines() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me ann");
    app.open(&id_of(&app, "#a"));
    sent(&mut app);
    feed(&mut app, &[":ann!a@h PRIVMSG me :\x01PING 12345\x01", ":ann!a@h PRIVMSG #a :\x01VERSION\x01", ":ann!a@h PRIVMSG me :\x01FOO\x01"]);
    let out = sent(&mut app);
    assert_eq!(out.len(), 2, "{out:?}");
    assert_eq!(out[0], "NOTICE ann :\x01PING 12345\x01");
    assert!(out[1].starts_with("NOTICE ann :\x01VERSION ") && out[1].ends_with('\x01'), "{}", out[1]);
    feed(&mut app, &[":ann!a@h NOTICE me :\x01VERSION irssi\x01"]);
    assert_eq!(
        texts(&app, "#a"),
        ["CTCP PING request from ann", "CTCP VERSION request from ann", "CTCP FOO request from ann", "CTCP VERSION reply from ann: irssi"]
    );
    assert!(conn(&app).buffer("ann").is_none());
}

#[test]
fn routes_server_notices_to_the_server_buffer() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me");
    app.open(&id_of(&app, "#a"));
    feed(&mut app, &[":irc.example.net NOTICE * :*** Looking up your hostname", ":irc.example.net NOTICE me :server says hi", "NOTICE me :sourceless"]);
    assert_eq!(server_texts(&app), ["*** Looking up your hostname", "server says hi", "sourceless"]);
    assert!(lines(&app, "#a").is_empty());
}

#[test]
fn routes_user_notices_to_an_open_query_else_the_active_buffer() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me");
    app.open(&id_of(&app, "#a"));
    feed(&mut app, &[":NickServ!s@services. NOTICE me :Please identify"]);
    assert!(conn(&app).buffer("NickServ").is_none());
    let l = last(&app, "#a");
    assert_eq!((l.kind, l.nick.as_str(), l.text.as_str()), (LineType::Notice, "NickServ", "Please identify"));
    feed(&mut app, &[":ann!a@h PRIVMSG me :hi", ":ann!a@h NOTICE me :fyi"]);
    assert_eq!(types(&app, "ann"), [LineType::Privmsg, LineType::Notice]);
    feed(&mut app, &[":ann!a@h NOTICE #a :channel notice"]);
    let l = last(&app, "#a");
    assert_eq!((l.kind, l.text.as_str()), (LineType::Notice, "channel notice"));
}

#[test]
fn strips_statusmsg_prefixes() {
    let mut app = setup(&[], &[("STATUSMSG", "@+")]);
    join_with(&mut app, "#a", "me");
    feed(&mut app, &[":ann!a@h NOTICE @#a :ops only"]);
    assert_eq!(texts(&app, "#a"), ["ops only"]);
    assert!(conn(&app).buffer("@#a").is_none());
}

#[test]
fn highlights_on_nick_and_configured_words_as_whole_words() {
    let mut app = setup(&[], &[]);
    app.settings.highlight_words = vec!["hearth".into(), "c++".into()];
    join_with(&mut app, "#a", "me ann");
    feed(
        &mut app,
        &[
            ":ann!a@h PRIVMSG #a :ME, look",
            ":ann!a@h PRIVMSG #a :meme time",
            ":ann!a@h PRIVMSG #a :I love Hearth!",
            ":ann!a@h PRIVMSG #a :hearthstone",
            ":ann!a@h PRIVMSG #a :learning c++ today",
            ":ann!a@h PRIVMSG #a :\x02me\x02 in bold",
        ],
    );
    let hl: Vec<bool> = lines(&app, "#a").iter().map(|l| l.highlight).collect();
    assert_eq!(hl, [true, false, true, false, true, true]);
    // Our own messages never highlight.
    feed(&mut app, &[":me!u@h PRIVMSG #a :hearth me"]);
    assert!(!last(&app, "#a").highlight);
}

#[test]
fn notifies_for_highlights_and_private_messages_when_not_looking() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me ann");
    app.focused = false;
    notifications(&mut app);
    feed(&mut app, &[":ann!a@h PRIVMSG #a :chatter", ":ann!a@h PRIVMSG #a :me: ping", ":ann!a@h PRIVMSG me :dm"]);
    let titles: Vec<String> = notifications(&mut app).into_iter().map(|(t, _)| t.split(" \u{b7} ").next().unwrap().to_string()).collect();
    assert_eq!(titles, ["ann in #a", "ann"]);
}

#[test]
fn drops_everything_from_ignored_masks_case_insensitively() {
    let mut app = setup(&[], &[("CASEMAPPING", "rfc1459")]);
    app.settings.ignores = vec!["troll[1]!*@*".into(), "*!*@*.spam.example".into()];
    join_with(&mut app, "#a", "me");
    app.open(&id_of(&app, "#a"));
    feed(
        &mut app,
        &[
            ":TROLL{1}!x@y PRIVMSG #a :hi",
            ":bot!b@node.spam.example PRIVMSG me :buy now",
            ":bot!b@node.spam.example INVITE me #spam",
            ":bot!b@node.spam.example NOTICE me :notice spam",
            ":ok!o@good.example PRIVMSG #a :real",
        ],
    );
    assert_eq!(texts(&app, "#a"), ["real"]);
    assert!(conn(&app).buffer("bot").is_none());
}

#[test]
fn announces_invites() {
    let mut app = setup(&[], &[]);
    feed(&mut app, &[":ann!a@h INVITE me #party", ":ann!a@h INVITE bob #party"]);
    let got: Vec<(&str, bool)> = conn(&app).server.lines().iter().map(|l| (l.text.as_str(), l.highlight)).collect();
    assert_eq!(got, [("invited you to #party", true), ("invited bob to #party", false)]);
}

// ── ISUPPORT ──────────────────────────────────────────────────

#[test]
fn folds_nicks_and_channels_with_rfc1459_casemapping() {
    let mut app = setup(&[], &[("CASEMAPPING", "rfc1459")]);
    feed(&mut app, &[":me!u@h JOIN #Chan[x]", ":Ann[away]!a@h JOIN #chan{X}", ":ann{AWAY}!a@h PRIVMSG #CHAN{x} :hi"]);
    assert_eq!(conn(&app).buffer_count(), 1);
    let b = buf(&app, "#chan{x}");
    assert_eq!(b.name, "#Chan[x]");
    assert_eq!(b.member("ANN{away}").unwrap().nick, "Ann[away]");
    assert_eq!(texts(&app, "#Chan[x]"), ["", "hi"]);
    feed(&mut app, &[":ann~!a@h PRIVMSG me :tilde", ":ANN^!a@h PRIVMSG me :caret"]);
    assert_eq!(texts(&app, "ann^"), ["tilde", "caret"]);
    assert!(conn(&app).is_me("ME"));
}

#[test]
fn ascii_casemapping_keeps_brackets_distinct() {
    let mut app = setup(&[], &[("CASEMAPPING", "ascii")]);
    feed(&mut app, &[":a[1]!a@h PRIVMSG me :x", ":A{1}!a@h PRIVMSG me :y"]);
    assert_eq!(texts(&app, "a[1]"), ["x"]);
    assert_eq!(texts(&app, "a{1}"), ["y"]);
}

#[test]
fn uses_chantypes_to_tell_channels_from_nicks() {
    let mut app = setup(&[], &[("CHANTYPES", "#&")]);
    feed(&mut app, &[":me!u@h JOIN &local", ":ann!a@h PRIVMSG &local :hey"]);
    assert_eq!(buf(&app, "&local").kind, BufferKind::Channel);
    assert_eq!(texts(&app, "&local"), ["hey"]);
    assert!(!conn(&app).is_channel("!odd"));
    assert!(!conn(&app).is_channel(""));
}

#[test]
fn defaults_prefix_when_it_is_missing_or_malformed() {
    let mut app = setup(&[], &[("PREFIX", "(ohv)@+")]);
    let p = conn(&app).prefix();
    assert_eq!((p.modes.as_str(), p.symbols.as_str()), ("ov", "@+"));
    let mut isupport = conn(&app).isupport.clone();
    isupport.insert("PREFIX".into(), "(yqaohv)!~&@%+".into());
    app.conn_mut(CONN).unwrap().set_isupport(isupport);
    let p = conn(&app).prefix();
    assert_eq!((p.modes.as_str(), p.symbols.as_str()), ("yqaohv", "!~&@%+"));
}

// ── Numerics ──────────────────────────────────────────────────

#[test]
fn routes_errors_about_a_buffer_to_that_buffer() {
    let mut app = setup(&[], &[]);
    join_with(&mut app, "#a", "me");
    join_with(&mut app, "#b", "me");
    feed(&mut app, &[":ann!a@h PRIVMSG me :hi"]);
    app.open(&id_of(&app, "#b"));
    feed(&mut app, &[":srv 404 me #a :Cannot send to channel"]);
    feed(&mut app, &[":srv 482 me #a :You're not channel operator"]);
    feed(&mut app, &[":srv 401 me ann :No such nick/channel"]);
    let got: Vec<(LineType, &str)> = lines(&app, "#a").iter().map(|l| (l.kind, l.text.as_str())).collect();
    assert_eq!(got, [(LineType::Error, "#a Cannot send to channel"), (LineType::Error, "#a You're not channel operator")]);
    let l = last(&app, "ann");
    assert_eq!((l.kind, l.text.as_str()), (LineType::Error, "ann No such nick/channel"));
    assert!(lines(&app, "#b").is_empty());
}

#[test]
fn falls_back_to_the_active_buffer_then_the_server_buffer() {
    let mut app = setup(&[], &[]);
    feed(&mut app, &[":srv 403 me #nowhere :No such channel"]);
    let s = conn(&app).server.lines().last().unwrap();
    assert_eq!((s.kind, s.text.as_str()), (LineType::Error, "#nowhere No such channel"));
    join_with(&mut app, "#a", "me");
    app.open(&id_of(&app, "#a"));
    feed(&mut app, &[":srv 401 me ghost :No such nick/channel", ":srv 421 me FOO :Unknown command"]);
    assert_eq!(texts(&app, "#a"), ["ghost No such nick/channel", "FOO Unknown command"]);
}

#[test]
fn reports_nick_collisions_and_sasl_failures() {
    let mut app = setup(&[], &[]);
    feed(&mut app, &[":srv 433 me taken :Nickname is already in use", ":srv 904 me :SASL authentication failed"]);
    let got: Vec<(LineType, &str)> = conn(&app).server.lines().iter().map(|l| (l.kind, l.text.as_str())).collect();
    assert_eq!(got, [(LineType::Error, "Nickname taken is already in use"), (LineType::Error, "SASL: SASL authentication failed")]);
}

#[test]
fn collects_the_motd_into_one_block_dropping_the_375_header() {
    let mut app = setup(&[], &[]);
    feed(&mut app, &[":srv 375 me :- srv Message of the day", ":srv 372 me :- line one", ":srv 372 me :-line two", ":srv 376 me :End"]);
    let l = conn(&app).server.lines();
    assert_eq!(l.len(), 1);
    let data = l[0].data.as_ref().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data["motd"], "line one\nline two");
}

#[test]
fn assembles_whois_replies_into_one_card() {
    let mut app = setup(&[], &[]);
    feed(
        &mut app,
        &[
            ":srv 311 me ann a host.example * :Ann Example",
            ":srv 312 me ann irc.example :The Server",
            ":srv 319 me ann :@#a",
            ":srv 319 me ann :+#b",
            ":srv 330 me ann annacct :is logged in as",
            ":srv 301 me ann :lunch",
            ":srv 318 me ann :End of /WHOIS list.",
        ],
    );
    let card = conn(&app).server.lines().last().unwrap();
    assert_eq!(card.kind, LineType::Whois);
    let d = card.data.as_ref().unwrap();
    for (k, v) in [
        ("nick", "ann"),
        ("user", "a"),
        ("host", "host.example"),
        ("realname", "Ann Example"),
        ("server", "irc.example \u{2014} The Server"),
        ("channels", "@#a +#b"),
        ("account", "annacct"),
        ("away", "lunch"),
    ] {
        assert_eq!(d.get(k).map(String::as_str), Some(v), "{k}");
    }
}

#[test]
fn shows_fail_for_the_asking_buffer_and_hides_chathistory_failures() {
    let mut app = setup(&[], &[]);
    feed(&mut app, &["FAIL CHATHISTORY INVALID_TARGET #a :nope", "FAIL NICK ERRONEOUS_NICKNAME * :Bad nick", "NOTE SERVER RESTART :soon"]);
    let got: Vec<(LineType, &str)> = conn(&app).server.lines().iter().map(|l| (l.kind, l.text.as_str())).collect();
    assert_eq!(got, [(LineType::Error, "NICK failed: Bad nick"), (LineType::Info, "soon")]);
}
