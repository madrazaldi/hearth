//! Draft/multiline, standard-replies,
//! draft/channel-rename, draft/account-registration and draft/metadata-2.

use crate::effect::{Effect, Reply};
use crate::multiline::{join_parts, multiline_batches, parse_multiline_cap, MultilineLimits};
use crate::state::ToastKind;
use crate::testing::*;
use crate::types::{LineType, NotifyLevel, SaslMechanism, ServerConfig};
use crate::{App, BufId};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

fn in_channel(caps: &[&str], cap_values: &[(&str, &str)]) -> App {
    let mut app = new_app();
    connect_with(&mut app, ConnectOpts { caps, cap_values, ..ConnectOpts::default() });
    feed(&mut app, &[":me!u@h JOIN #a", ":srv 353 me = #a :@me ann", ":srv 366 me #a :End"]);
    sent(&mut app);
    let id = id_of(&app, "#a");
    app.open(&id);
    app
}

fn run(app: &mut App, input: &str) -> bool {
    let id = id_of(app, "#a");
    app.run_input(&id, input, None)
}

/// Replace generated batch refs and labels so sent lines can be compared.
fn normalize(lines: &[String]) -> Vec<String> {
    let (ml, label) = (Regex::new(r"ml\d+").unwrap(), Regex::new(r"label=[^;\s]+").unwrap());
    lines.iter().map(|l| label.replace_all(&ml.replace_all(l, "ML"), "label=L").into_owned()).collect()
}

fn label_of(line: &str) -> String {
    Regex::new(r"label=([^;\s]+)").unwrap().captures(line).expect("labeled line")[1].to_string()
}

fn conn(app: &App) -> &crate::Conn {
    app.conn(CONN).unwrap()
}

const ML: [&str; 3] = ["batch", "draft/multiline", "message-tags"];

fn with_ml(extra: &[&'static str]) -> Vec<&'static str> {
    ML.iter().copied().chain(extra.iter().copied()).collect()
}

// ── draft/multiline ───────────────────────────────────────────

#[test]
fn parses_the_multiline_cap_value() {
    assert_eq!(parse_multiline_cap(Some("max-bytes=4096,max-lines=24")), Some(MultilineLimits { max_bytes: 4096, max_lines: 24 }));
    assert_eq!(parse_multiline_cap(Some("max-bytes=4096")), Some(MultilineLimits { max_bytes: 4096, max_lines: usize::MAX }));
    assert_eq!(parse_multiline_cap(Some("max-lines=5")), None);
    assert_eq!(parse_multiline_cap(None), None);
}

#[test]
fn splits_long_lines_into_concatenated_parts_without_losing_characters() {
    let long = "word ".repeat(40).trim().to_string();
    let text = format!("{long}\nshort");
    let batch = &multiline_batches(&text, MultilineLimits { max_bytes: 4096, max_lines: 10 }, 60)[0];
    assert!(batch.len() > 3);
    assert!(batch.iter().all(|p| p.text.len() <= 60));
    assert_eq!(batch.iter().filter(|p| !p.concat).count(), 2);
    assert_eq!(join_parts(batch), text);
}

#[test]
fn respects_max_lines_and_max_bytes_by_starting_new_batches() {
    let lines = multiline_batches("a\nb\nc\nd", MultilineLimits { max_bytes: 4096, max_lines: 3 }, 400);
    assert_eq!(lines.iter().map(|b| join_parts(b)).collect::<Vec<_>>(), ["a\nb\nc", "d"]);
    let bytes = multiline_batches("aaaa\nbbbb\ncccc", MultilineLimits { max_bytes: 10, max_lines: 99 }, 400);
    assert_eq!(bytes.iter().map(|b| join_parts(b)).collect::<Vec<_>>(), ["aaaa\nbbbb", "cccc"]);
    assert!(multiline_batches("\n\n", MultilineLimits { max_bytes: 10, max_lines: 99 }, 400).is_empty());
}

#[test]
fn sends_composer_text_with_line_breaks_as_one_batch_keeping_blank_lines() {
    let mut app = in_channel(&with_ml(&["labeled-response", "echo-message"]), &[("draft/multiline", "max-bytes=4096,max-lines=24")]);
    let id = id_of(&app, "#a");
    app.run_input(&id, "first\n\nthird\n", Some("parent1"));
    let out = sent(&mut app);
    assert_eq!(
        normalize(&out),
        [
            "@label=L;+draft/reply=parent1 BATCH +ML draft/multiline #a",
            "@batch=ML PRIVMSG #a :first",
            "@batch=ML PRIVMSG #a :",
            "@batch=ML PRIVMSG #a :third",
            "BATCH -ML",
        ]
    );
    // With echo-message, the message shows up once, when the server echoes it.
    assert!(buf(&app, "#a").lines().is_empty());
    feed(&mut app, &[&format!("@label={};msgid=e1 :me!u@h PRIVMSG #a :first\n\nthird", label_of(&out[0]))]);
    assert_eq!(texts(&app, "#a"), ["first\n\nthird"]);
    let b = app.buffer_mut(&id).unwrap();
    let mut dup = b.lines()[0].clone();
    dup.id = u64::MAX;
    b.merge(vec![dup]);
    assert_eq!(b.lines().len(), 1);
}

#[test]
fn shows_our_own_batch_locally_without_echo_message_and_splits_at_max_lines() {
    let mut app = in_channel(&ML, &[("draft/multiline", "max-bytes=4096,max-lines=2")]);
    run(&mut app, "one\ntwo\nthree");
    assert_eq!(
        normalize(&sent(&mut app)),
        [
            "BATCH +ML draft/multiline #a",
            "@batch=ML PRIVMSG #a :one",
            "@batch=ML PRIVMSG #a :two",
            "BATCH -ML",
            "BATCH +ML draft/multiline #a",
            "@batch=ML PRIVMSG #a :three",
            "BATCH -ML",
        ]
    );
    assert_eq!(texts(&app, "#a"), ["one\ntwo", "three"]);
    assert!(buf(&app, "#a").lines().iter().all(|l| l.is_self));
}

#[test]
fn marks_continuations_of_a_long_line_with_multiline_concat() {
    let mut app = in_channel(&ML, &[("draft/multiline", "max-bytes=8192")]);
    let long = "x".repeat(700);
    run(&mut app, &format!("{long}\nend"));
    let body: Vec<String> = sent(&mut app).into_iter().filter(|l| l.contains("PRIVMSG")).collect();
    assert_eq!(body.len(), 3);
    assert!(Regex::new(r"^@batch=ml\d+;draft/multiline-concat PRIVMSG").unwrap().is_match(&body[1]), "{}", body[1]);
    let joined: String = body.iter().map(|l| l.split(" :").nth(1).unwrap()).collect();
    assert_eq!(joined, format!("{long}end"));
}

#[test]
fn falls_back_to_one_privmsg_per_line_without_the_cap_or_for_a_single_line() {
    let mut app = in_channel(&["batch"], &[]);
    run(&mut app, "one\n\ntwo");
    assert_eq!(sent(&mut app), ["PRIVMSG #a :one", "PRIVMSG #a :two"]);
    let c = app.conn_mut(CONN).unwrap();
    c.caps = ML.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
    c.cap_values = BTreeMap::from([("draft/multiline".to_string(), "max-bytes=4096".to_string())]);
    run(&mut app, "just one\n");
    assert_eq!(sent(&mut app), ["PRIVMSG #a :just one"]);
}

#[test]
fn receives_a_combined_multiline_message_as_one_line_with_its_breaks() {
    let mut app = in_channel(&ML, &[]);
    feed(&mut app, &["@msgid=m1 :ann!a@h PRIVMSG #a :hello\nme: are you there?"]);
    let l = &buf(&app, "#a").lines()[0];
    assert_eq!(l.text, "hello\nme: are you there?");
    assert!(l.highlight);
}

// ── standard replies ──────────────────────────────────────────

#[test]
fn routes_labeled_replies_to_the_buffer_that_sent_the_command() {
    let mut app = in_channel(&["labeled-response"], &[]);
    app.open(&BufId::server(CONN));
    run(&mut app, "/topic");
    let label = label_of(&sent(&mut app)[0]);
    feed(&mut app, &[&format!("@label={label} FAIL TOPIC NEED_MORE_PRIVS #a :You're not a channel operator")]);
    let l = buf(&app, "#a").lines().last().unwrap();
    assert_eq!((l.kind, l.text.as_str()), (LineType::Error, "TOPIC failed: You're not a channel operator"));
    assert!(server_texts(&app).is_empty());
}

#[test]
fn files_unlabeled_replies_under_a_buffer_named_in_the_context() {
    let mut app = in_channel(&[], &[]);
    app.open(&BufId::server(CONN));
    feed(&mut app, &["FAIL JOIN CHANNEL_FULL #a :Channel is full"]);
    assert_eq!(texts(&app, "#a").last().map(String::as_str), Some("JOIN failed: Channel is full"));
}

#[test]
fn falls_back_to_the_active_buffer_and_notes_go_to_the_server_buffer() {
    let mut app = in_channel(&[], &[]);
    feed(&mut app, &["WARN REHASH CERTS_EXPIRED :Certificate expires soon", "FAIL * ACCOUNT_REQUIRED", "NOTE * OPER_MESSAGE :Maintenance tonight"]);
    let got: Vec<(LineType, &str)> = buf(&app, "#a").lines().iter().map(|l| (l.kind, l.text.as_str())).collect();
    assert_eq!(got, [(LineType::Info, "Warning (REHASH): Certificate expires soon"), (LineType::Error, "Account required")]);
    let s = conn(&app).server.lines().last().unwrap();
    assert_eq!((s.kind, s.text.as_str()), (LineType::Info, "Maintenance tonight"));
}

#[test]
fn keeps_quiet_about_history_failures_and_toasts_bouncer_ones() {
    let mut app = in_channel(&[], &[]);
    feed(&mut app, &["FAIL CHATHISTORY INVALID_TARGET #a :nope", "FAIL BOUNCER INVALID_NETID 7 :Unknown network"]);
    assert!(buf(&app, "#a").lines().is_empty());
    assert_eq!(app.toast_texts(), ["Bouncer: Unknown network"]);
}

// ── draft/channel-rename ──────────────────────────────────────

#[test]
fn rename_moves_the_buffer_with_its_members_history_unread_state_and_settings() {
    let mut app = in_channel(&["draft/channel-rename"], &[]);
    feed(&mut app, &[":ann!a@h PRIVMSG #a :before"]);
    let a = id_of(&app, "#a");
    app.buffer_mut(&a).unwrap().read_marker = 42;
    app.set_notify(&a, Some(NotifyLevel::All));
    feed(&mut app, &[":ann!a@h RENAME #a #b :cleaning up"]);
    assert!(conn(&app).buffer("#a").is_none());
    let b = buf(&app, "#b");
    assert_eq!(b.name, "#b");
    let members: Vec<&str> = b.sorted_members(conn(&app).prefix()).iter().map(|m| m.nick.as_str()).collect();
    assert_eq!(members, ["me", "ann"]);
    assert_eq!(b.read_marker, 42);
    assert_eq!(texts(&app, "#b"), ["before", "ann renamed #a to #b (cleaning up)"]);
    assert_eq!(app.settings.buffer_notify, BTreeMap::from([("c||#b".to_string(), NotifyLevel::All)]));
    assert_eq!(app.active(), Some(&id_of(&app, "#b")));
    feed(&mut app, &[":ann!a@h PRIVMSG #b :after"]);
    assert_eq!(texts(&app, "#b").last().map(String::as_str), Some("after"));
}

#[test]
fn rename_replaces_a_stale_buffer_that_already_had_the_new_name() {
    let mut app = in_channel(&[], &[]);
    app.conn_mut(CONN).unwrap().ensure("#b", Some(crate::BufferKind::Channel));
    feed(&mut app, &[":op!o@h RENAME #a #b"]);
    assert_eq!(conn(&app).keys(), ["#b"]);
    assert!(buf(&app, "#b").joined);
}

#[test]
fn rename_command_needs_the_cap_and_takes_an_optional_source_channel() {
    let mut app = in_channel(&[], &[]);
    run(&mut app, "/rename #new");
    assert!(sent(&mut app).is_empty());
    assert!(app.toast_texts().last().unwrap().contains("does not support renaming"));
    app.conn_mut(CONN).unwrap().caps = BTreeSet::from(["draft/channel-rename".to_string()]);
    run(&mut app, "/rename #new tidier name");
    app.run_input(&BufId::server(CONN), "/rename #a #new", None);
    assert_eq!(sent(&mut app), ["RENAME #a #new :tidier name", "RENAME #a #new"]);
}

// ── draft/account-registration ────────────────────────────────

const REG: [&str; 2] = ["draft/account-registration", "labeled-response"];

fn registration_app(cap_value: &str) -> App {
    let mut app = new_app();
    app.servers = vec![ServerConfig { id: "c".into(), host: "irc.example".into(), ..ServerConfig::default() }];
    connect_with(&mut app, ConnectOpts { caps: &REG, cap_values: &[("draft/account-registration", cap_value)], ..ConnectOpts::default() });
    feed(&mut app, &[":me!u@h JOIN #a", ":srv 353 me = #a :@me ann", ":srv 366 me #a :End"]);
    sent(&mut app);
    let id = id_of(&app, "#a");
    app.open(&id);
    app
}

fn last_text(app: &App) -> (LineType, String) {
    let l = buf(app, "#a").lines().last().unwrap();
    (l.kind, l.text.clone())
}

#[test]
fn registers_then_offers_to_save_the_login_for_sasl() {
    let mut app = registration_app("custom-account-name");
    run(&mut app, "/register me@example.com hunter2");
    let out = sent(&mut app);
    assert_eq!(normalize(&out), ["@label=L REGISTER * me@example.com :hunter2"]);
    feed(&mut app, &[&format!("@label={} REGISTER SUCCESS me :Account created", label_of(&out[0]))]);
    assert_eq!(last_text(&app).1, "Account me registered");
    let offer = app.toasts.last().cloned().unwrap();
    assert_eq!(offer.action.as_ref().map(|a| a.label.as_str()), Some("Save login"));
    app.run_toast_action(offer.id);
    let saved = effects(&mut app).into_iter().find_map(|e| match e {
        Effect::SaveServer { server, .. } => Some(server.0),
        _ => None,
    });
    let s = saved.expect("saves the server");
    assert_eq!((s.id.as_str(), s.sasl, s.sasl_username.as_str(), s.sasl_password.as_str()), ("c", SaslMechanism::Plain, "me", "hunter2"));
    assert!(conn(&app).registration.is_none());
}

#[test]
fn registration_walks_through_verification_and_reports_failures() {
    let mut app = registration_app("custom-account-name");
    run(&mut app, "/register * hunter2");
    feed(&mut app, &["REGISTER VERIFICATION_REQUIRED me :Check your email"]);
    assert_eq!(last_text(&app).1, "Check your email \u{2014} then type /verify me <code>");
    run(&mut app, "/verify me 123");
    feed(&mut app, &["FAIL VERIFY INVALID_CODE me :That code is wrong"]);
    assert_eq!(last_text(&app), (LineType::Error, "Verification failed: That code is wrong".to_string()));
    assert!(conn(&app).registration.is_some());
    feed(&mut app, &["VERIFY SUCCESS me :Welcome"]);
    assert_eq!(last_text(&app).1, "Account me verified");
    assert!(app.toasts.last().unwrap().action.is_some());
}

#[test]
fn forgets_the_password_when_registration_is_refused() {
    let mut app = registration_app("custom-account-name");
    run(&mut app, "/register * hunter2");
    feed(&mut app, &["FAIL REGISTER ACCOUNT_EXISTS me :Already registered"]);
    assert_eq!(last_text(&app).1, "Registration failed: Already registered");
    assert!(conn(&app).registration.is_none());
    assert!(!app.toasts.iter().any(|t| t.action.is_some()));
}

#[test]
fn checks_usage_and_email_required_and_never_keeps_the_command_for_recall() {
    let mut app = registration_app("email-required");
    run(&mut app, "/register * hunter2");
    assert!(sent(&mut app).is_empty());
    assert!(app.toast_texts().last().unwrap().contains("needs an email"));
    app.set_input_history(vec!["/register a@b.c hunter2".into(), "/verify me 1".into(), "hello".into()]);
    assert_eq!(app.input_history(), ["hello"]);
    assert!(app.toasts.iter().all(|t| t.kind == ToastKind::Error || t.kind == ToastKind::Info));
}

// ── draft/metadata-2 ──────────────────────────────────────────

#[test]
fn tracks_avatar_and_display_name_per_user() {
    let mut app = in_channel(&["draft/metadata-2"], &[]);
    feed(
        &mut app,
        &[
            ":srv METADATA ann avatar * :https://img.example/ann.png",
            ":srv 761 me ann display-name * :Ann Example",
            ":srv 761 me #a url * :https://example.com",
        ],
    );
    let p = conn(&app).profile("ANN");
    assert_eq!((p.avatar.as_deref(), p.display_name.as_deref()), (Some("https://img.example/ann.png"), Some("Ann Example")));
    feed(&mut app, &[":ann!a@h NICK anna"]);
    assert_eq!(conn(&app).profile("anna").display_name.as_deref(), Some("Ann Example"));
    assert_eq!(conn(&app).profile("ann"), Default::default());
    feed(&mut app, &[":srv METADATA anna avatar *", ":srv 766 me anna display-name :key not set"]);
    assert_eq!(conn(&app).profile("anna"), Default::default());
    feed(&mut app, &[":srv METADATA anna avatar * :https://img.example/2.png", ":anna!a@h QUIT :bye"]);
    assert_eq!(conn(&app).profile("anna"), Default::default());
    assert!(server_texts(&app).is_empty());
}

#[test]
fn retries_a_sync_the_server_postponed() {
    let mut app = in_channel(&["draft/metadata-2"], &[]);
    feed(&mut app, &[":srv 774 me #a 3"]);
    advance(&mut app, 2900);
    assert!(sent(&mut app).is_empty());
    advance(&mut app, 200);
    assert_eq!(sent(&mut app), ["METADATA #a SYNC"]);
}

#[test]
fn loads_avatars_through_the_backend_only_when_allowed() {
    let mut app = new_app();
    let url = "https://img.example/a.png";
    app.request_avatar(url);
    assert!(requests(&mut app).is_empty());
    app.settings.load_avatars = true;
    app.request_avatar(url);
    app.request_avatar(url);
    let reqs = requests(&mut app);
    assert_eq!(reqs.len(), 1);
    assert!(matches!(app.avatar(url), crate::state::AvatarState::Loading));
    app.resolve(reqs[0].req().unwrap(), Ok(Reply::Text("data:image/png;base64,AAAA".into())));
    match app.avatar(url) {
        crate::state::AvatarState::Ready(d) => assert!(d.starts_with("data:image/png;base64,")),
        other => panic!("{other:?}"),
    }
}
