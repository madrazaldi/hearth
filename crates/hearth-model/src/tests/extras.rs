//! Tests for the logic behind the views (composer, quick
//! switcher, member list, search panel, welcome flow, scroll-to-line), which
//! had no TypeScript counterpart.

use crate::buffer::Focus;
use crate::composer::{ComposerKey, KeyOutcome, Suggestions};
use crate::effect::{Effect, Reply};
use crate::irc::{iso_time, parse_time};
use crate::panels::{MemberRow, SwitcherItem};
use crate::testing::*;
use crate::types::{IrcMessage, SearchHit};
use crate::welcome::Step;
use crate::App;

fn channel_app() -> App {
    let mut app = new_app();
    connect(&mut app, &[], &[]);
    feed(
        &mut app,
        &[
            ":me!u@h JOIN #hearth",
            ":srv 353 me = #hearth :@alice +bob carol Dave me",
            ":srv 366 me #hearth :End of /NAMES list.",
            "@msgid=m1 :carol!c@h PRIVMSG #hearth :hello",
            "@msgid=m2 :bob!b@h PRIVMSG #hearth :hi",
        ],
    );
    // The sidebar lists connections the config knows about.
    app.servers.push(hearth_core::config::ServerConfig { id: CONN.into(), name: "Test".into(), ..Default::default() });
    app.open(&id_of(&app, "#hearth"));
    sent(&mut app);
    // Connecting asks for the saved buffers; nobody answers in these tests.
    requests(&mut app);
    app
}

fn type_text(app: &mut App, id: &crate::BufId, text: &str) {
    app.composer_edit(id, text, text.len());
}

// ── Composer ──────────────────────────────────────────────────

#[test]
fn tab_completes_the_most_recent_speaker_first_and_cycles() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    type_text(&mut app, &id, "b");
    assert_eq!(app.composer_key(&id, ComposerKey::Tab, false), KeyOutcome::Handled);
    assert_eq!(app.composer(&id).unwrap().text, "bob: ");
    type_text(&mut app, &id, "hey ca");
    assert_eq!(app.composer_key(&id, ComposerKey::Tab, false), KeyOutcome::Handled);
    assert_eq!(app.composer(&id).unwrap().text, "hey carol ");
}

#[test]
fn tab_with_nothing_to_complete_is_left_to_the_toolkit() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    assert_eq!(app.composer_key(&id, ComposerKey::Tab, false), KeyOutcome::Ignored);
    type_text(&mut app, &id, "zzz");
    assert_eq!(app.composer_key(&id, ComposerKey::Tab, false), KeyOutcome::Ignored);
}

#[test]
fn tab_never_completes_to_your_own_nick() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    type_text(&mut app, &id, "m");
    assert_eq!(app.composer_key(&id, ComposerKey::Tab, false), KeyOutcome::Ignored);
}

#[test]
fn slash_shows_command_suggestions_and_tab_completes_them() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    type_text(&mut app, &id, "/jo");
    match app.composer_suggestions(&id) {
        Suggestions::Commands { items, .. } => assert!(items.iter().any(|c| c.name == "join")),
        other => panic!("expected commands, got {other:?}"),
    }
    assert_eq!(app.composer_key(&id, ComposerKey::Tab, false), KeyOutcome::Handled);
    assert!(app.composer(&id).unwrap().text.starts_with("/join"));
}

#[test]
fn colon_suggests_emoji_and_picking_replaces_the_word() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    type_text(&mut app, &id, "nice :tada");
    let Suggestions::Emoji { start, items, .. } = app.composer_suggestions(&id) else { panic!("expected emoji suggestions") };
    assert_eq!(start, 5);
    assert!(!items.is_empty());
    app.composer_pick_emoji(&id, 0);
    let text = app.composer(&id).unwrap().text.clone();
    assert!(text.starts_with("nice "), "{text}");
    assert!(!text.contains(':'), "{text}");
    assert_eq!(app.composer_suggestions(&id), Suggestions::None);
}

#[test]
fn escape_closes_emoji_suggestions_before_it_blurs() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    type_text(&mut app, &id, "x :tad");
    assert!(matches!(app.composer_suggestions(&id), Suggestions::Emoji { .. }));
    assert_eq!(app.composer_key(&id, ComposerKey::Escape, false), KeyOutcome::Handled);
    assert_eq!(app.composer_suggestions(&id), Suggestions::None);
    assert_eq!(app.composer_key(&id, ComposerKey::Escape, false), KeyOutcome::Blur);
}

#[test]
fn enter_sends_clears_and_recalls_with_up() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    type_text(&mut app, &id, "first");
    assert_eq!(app.composer_key(&id, ComposerKey::Enter, false), KeyOutcome::Sent);
    assert!(sent(&mut app).iter().any(|l| l.starts_with("PRIVMSG #hearth :first")));
    assert_eq!(app.composer(&id).unwrap().text, "");
    assert_eq!(app.composer_key(&id, ComposerKey::Up, false), KeyOutcome::Handled);
    assert_eq!(app.composer(&id).unwrap().text, "first");
    assert_eq!(app.composer_key(&id, ComposerKey::Down, false), KeyOutcome::Handled);
    assert_eq!(app.composer(&id).unwrap().text, "");
}

#[test]
fn blank_input_is_not_sent() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    type_text(&mut app, &id, "   ");
    assert!(!app.composer_submit(&id));
    assert!(sent(&mut app).is_empty());
}

#[test]
fn placeholder_says_where_the_message_goes() {
    let app = channel_app();
    let id = id_of(&app, "#hearth");
    assert_eq!(app.composer_placeholder(&id), "Message #hearth");
    assert_eq!(app.composer_placeholder(&crate::BufId::new(CONN, "")), "Type /help for commands");
}

#[test]
fn mention_inserts_the_nick_at_the_caret() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    app.composer_mention(&id, "alice");
    let text = app.composer(&id).unwrap().text.clone();
    assert!(text.contains("alice"), "{text}");
}

// ── Quick switcher ────────────────────────────────────────────

#[test]
fn switcher_ranks_prefix_matches_and_offers_to_join_a_new_channel() {
    let mut app = channel_app();
    feed(&mut app, &[":me!u@h JOIN #help", ":me!u@h JOIN #random"]);
    app.toggle_switcher();
    app.switcher_set_query("he");
    let labels: Vec<String> = app
        .switcher_results()
        .into_iter()
        .filter_map(|i| match i {
            SwitcherItem::Buffer { label, .. } => Some(label),
            SwitcherItem::Join { .. } => None,
        })
        .collect();
    assert!(labels.contains(&"#hearth".to_string()) && labels.contains(&"#help".to_string()));
    assert!(!labels.contains(&"#random".to_string()));

    app.switcher_set_query("#brand-new");
    assert_eq!(app.switcher_results().last(), Some(&SwitcherItem::Join { channel: "#brand-new".into() }));
    app.switcher_choose(Some(0));
    assert!(sent(&mut app).contains(&"JOIN #brand-new".to_string()));
}

#[test]
fn switcher_choose_opens_the_buffer_and_closes_the_panel() {
    let mut app = channel_app();
    feed(&mut app, &[":me!u@h JOIN #other"]);
    app.toggle_switcher();
    app.switcher_set_query("other");
    app.switcher_choose(None);
    assert!(app.switcher.is_none());
    assert_eq!(app.active, Some(id_of(&app, "#other")));
}

// ── Member list ───────────────────────────────────────────────

#[test]
fn members_are_grouped_by_rank_with_counts() {
    let app = channel_app();
    let id = id_of(&app, "#hearth");
    let rows = app.member_rows(&id, "");
    let heads: Vec<(String, usize)> = rows
        .iter()
        .filter_map(|r| match r {
            MemberRow::Head { title, count } => Some((title.clone(), *count)),
            MemberRow::Member { .. } => None,
        })
        .collect();
    assert_eq!(heads, [("Operators".to_string(), 1), ("Voiced".to_string(), 1), ("Members".to_string(), 3)]);
    let positions: Vec<usize> = rows
        .iter()
        .filter_map(|r| match r {
            MemberRow::Member { pos, .. } => Some(*pos),
            MemberRow::Head { .. } => None,
        })
        .collect();
    assert_eq!(positions, [1, 2, 3, 4, 5]);
}

#[test]
fn member_filter_narrows_by_nick_and_drops_empty_groups() {
    let app = channel_app();
    let id = id_of(&app, "#hearth");
    let rows = app.member_rows(&id, "ali");
    assert_eq!(rows.len(), 2);
    assert!(matches!(&rows[0], MemberRow::Head { title, .. } if title == "Operators"));
}

// ── Sidebar ───────────────────────────────────────────────────

#[test]
fn sidebar_lists_the_network_then_its_buffers_and_hides_a_collapsed_tree() {
    let mut app = channel_app();
    let nodes = app.sidebar();
    assert!(nodes[0].buf.as_ref().is_none_or(|b| b.is_server()));
    assert!(nodes.iter().any(|n| n.name == "hearth" && n.active));
    app.set_collapsed(CONN, true);
    assert!(app.sidebar().iter().all(|n| n.buf.is_none() || n.active));
}

// ── Scroll to a line ──────────────────────────────────────────

#[test]
fn take_focus_lands_on_the_exact_line_and_flashes_it() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    let target = app.buffer(&id).unwrap().lines().iter().find(|l| l.text == "hi").map(|l| (l.id, l.time)).unwrap();
    app.buffer_mut(&id).unwrap().focus = Some(Focus { time: target.1, msgid: Some("m2".into()), seq: 1 });
    let hit = app.take_focus(&id, "").expect("a target");
    assert!(hit.exact);
    assert_eq!(app.buffer(&id).unwrap().lines()[hit.line].id, target.0);
    assert_eq!(app.flash_line(), Some(target.0));
    assert!(app.buffer(&id).unwrap().focus.is_none());
    advance(&mut app, 2000);
    assert_eq!(app.flash_line(), None);
}

#[test]
fn take_focus_falls_back_to_the_next_line_without_a_flash() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    let first = app.buffer(&id).unwrap().lines().last().unwrap().time;
    app.buffer_mut(&id).unwrap().focus = Some(Focus { time: first + 1, msgid: Some("gone".into()), seq: 1 });
    let hit = app.take_focus(&id, "").expect("a target");
    assert!(!hit.exact);
    assert_eq!(app.flash_line(), None);
}

// ── Search panel ──────────────────────────────────────────────

fn hit_at(target: &str, text: &str, secs: i64) -> SearchHit {
    let mut msg = IrcMessage::new("PRIVMSG", &[target, text]);
    msg.source = Some(crate::types::Source { nick: "ann".into(), user: Some("a".into()), host: Some("h".into()) });
    msg.tags.insert("time".into(), iso_time(parse_time("2026-01-01T00:00:00.000Z").unwrap() + secs * 1000));
    SearchHit { network: CONN.into(), target: target.into(), msg }
}

#[test]
fn search_panel_debounces_then_shows_hits_and_choosing_jumps() {
    let mut app = channel_app();
    let id = id_of(&app, "#hearth");
    app.search_open();
    app.search_set_query("deploy");
    assert!(app.search_panel().busy);
    assert!(requests(&mut app).is_empty(), "nothing is asked before the pause ends");
    advance(&mut app, 200);
    let asked = requests(&mut app);
    let req = asked
        .iter()
        .find_map(|e| {
            if let Effect::Search { req, query } = e {
                assert_eq!(query.text, "deploy");
                Some(*req)
            } else {
                None
            }
        })
        .expect("a local search");
    app.resolve(req, Ok(Reply::Hits(vec![hit_at("#hearth", "deploy done", 5)])));
    let p = app.search_panel();
    assert!(!p.busy);
    assert_eq!(p.hits.len(), 1);
    let view = app.search_hit_view(&p.hits[0]);
    assert_eq!(view.nick, "ann");
    assert!(view.parts.iter().any(|(t, m)| *m && t.eq_ignore_ascii_case("deploy")));

    app.search_choose(0);
    assert!(!app.search_panel().open);
    assert_eq!(app.active, Some(id));
    // Reopening the same query brings the results back without searching again.
    app.search_open();
    assert!(app.search_panel().resumed);
    assert_eq!(app.search_panel().hits.len(), 1);
}

#[test]
fn a_hit_from_a_network_that_is_gone_is_unreachable_and_choosing_it_only_toasts() {
    let mut app = channel_app();
    let mut gone = hit_at("#hearth", "deploy", 1);
    gone.network = "removed-network".into();
    let live = hit_at("#hearth", "deploy", 2);
    assert!(app.search_hit_view(&live).reachable);
    let view = app.search_hit_view(&gone);
    assert!(!view.reachable);
    assert!(view.network.is_empty(), "no network badge for it");

    app.search_open();
    app.search_panel.hits = vec![gone];
    let before = app.active.clone();
    app.search_choose(0);
    assert!(app.search_panel().open, "the panel stays open");
    assert_eq!(app.active, before);
    assert!(app.toasts.iter().any(|t| t.text.contains("not connected")));
}

#[test]
fn typing_again_before_the_pause_ends_searches_only_once() {
    let mut app = channel_app();
    app.search_open();
    app.search_set_query("dep");
    advance(&mut app, 100);
    app.search_set_query("deploy");
    advance(&mut app, 100);
    assert!(requests(&mut app).is_empty());
    advance(&mut app, 100);
    let searches = requests(&mut app).into_iter().filter(|e| matches!(e, Effect::Search { .. })).count();
    assert_eq!(searches, 1);
}

#[test]
fn clearing_the_query_clears_the_hits() {
    let mut app = channel_app();
    app.search_open();
    app.search_set_query("x");
    advance(&mut app, 200);
    for e in requests(&mut app) {
        if let Effect::Search { req, .. } = e {
            app.resolve(req, Ok(Reply::Hits(vec![hit_at("#hearth", "x", 1)])));
        }
    }
    assert_eq!(app.search_panel().hits.len(), 1);
    app.search_set_query("");
    assert!(app.search_panel().hits.is_empty());
    assert!(!app.search_panel().busy);
}

#[test]
fn filter_chips_append_to_the_query() {
    let mut app = channel_app();
    app.search_open();
    app.search_set_query("deploy");
    app.search_add_filter("from:");
    assert_eq!(app.search_panel().q, "deploy from:");
    assert!(app.search_suggestions().iter().any(|n| n == "carol"));
    app.search_complete("carol");
    assert_eq!(app.search_panel().q, "deploy from:carol ");
}

// ── Welcome flow ──────────────────────────────────────────────

#[test]
fn welcome_walks_a_network_preset_to_a_connection() {
    let mut app = new_app();
    app.init(hearth_core::config::Config::default(), None);
    assert!(app.welcoming());
    app.welcome_choose("libera");
    assert_eq!(app.welcome.step, Step::Nick);

    // A blank nickname stops on the same step.
    app.welcome.server.nick.clear();
    app.welcome_submit_nick();
    assert_eq!(app.welcome.step, Step::Nick);

    app.welcome.server.nick = "newcomer".into();
    app.welcome_submit_nick();
    // Libera offers an account step; skipping it leaves SASL off.
    assert_eq!(app.welcome.step, Step::Account);
    app.welcome_submit_account();
    assert_eq!(app.welcome.step, Step::Channels);
    app.welcome_submit_channels();
    assert_eq!(app.welcome.step, Step::Connect);
    assert_eq!(app.welcome_status(), "saving");

    let (req, server) = requests(&mut app)
        .into_iter()
        .find_map(|e| if let Effect::SaveServer { req, server } = e { Some((req, server)) } else { None })
        .expect("the server is saved");
    assert_eq!(server.0.nick, "newcomer");
    assert_eq!(server.0.host, "irc.libera.chat");
    let mut saved = server.0.clone();
    saved.id = "libera-1".into();
    app.resolve(req, Ok(Reply::Server(saved)));
    assert_eq!(app.welcome.saved_id, "libera-1");
    assert_eq!(app.welcome_status(), "connecting");
}

#[test]
fn welcome_warns_when_the_account_login_failed_but_the_connection_came_up() {
    use crate::types::{ConnStatus, IrcEvent};
    let mut app = new_app();
    app.init(hearth_core::config::Config::default(), None);
    app.welcome_choose("libera");
    app.welcome.server.nick = "newcomer".into();
    app.welcome.use_account = true;
    app.welcome.server.sasl_password = "wrong".into();
    app.welcome_submit_nick();
    app.welcome_submit_account();
    app.welcome_submit_channels();
    let (req, server) = requests(&mut app)
        .into_iter()
        .find_map(|e| if let Effect::SaveServer { req, server } = e { Some((req, server)) } else { None })
        .expect("the server is saved");
    let mut saved = server.0.clone();
    saved.id = "libera-1".into();
    app.resolve(req, Ok(Reply::Server(saved)));
    assert!(!app.welcome_sasl_failed());

    let c = "libera-1".to_string();
    let status = |status| IrcEvent::Status { conn: c.clone(), status, error: None, retry_in: None };
    app.handle_events(&[IrcEvent::ConnAdded { conn: c.clone(), server: c.clone(), parent: None, netid: None }, status(ConnStatus::Registering)]);
    app.handle_events(&[
        IrcEvent::Message { conn: c.clone(), msg: parse(":srv 904 newcomer :SASL authentication failed") },
        IrcEvent::Registered { conn: c.clone(), nick: "newcomer".into(), caps: vec![], cap_values: Default::default() },
        status(ConnStatus::Connected),
    ]);
    assert_eq!(app.welcome_status(), "connected");
    assert!(app.welcome.failure.is_none(), "the attempt itself succeeded");
    assert!(app.welcome_sasl_failed(), "the screen says the login did not work");
}

#[test]
fn a_preview_dropped_from_the_cache_can_be_fetched_again_but_a_failed_one_is_not_retried() {
    use crate::state::PreviewState;
    use crate::types::Preview;
    let mut app = new_app();
    let fetch = |app: &mut App, url: &str, ok: bool| {
        app.request_preview(url);
        let req = app.take_effects().into_iter().find_map(|e| matches!(e, Effect::LinkPreview { .. }).then(|| e.req()).flatten()).expect("a request");
        let page = Preview {
            url: url.into(),
            kind: "page".into(),
            title: Some("t".into()),
            description: None,
            site: None,
            image: None,
            width: None,
            height: None,
            animated: false,
            mime: None,
            size: None,
        };
        app.resolve(req, if ok { Ok(Reply::Preview(page)) } else { Err("no preview".into()) });
    };
    fetch(&mut app, "https://a.example/failed", false);
    assert!(matches!(app.preview("https://a.example/failed"), PreviewState::Unavailable));
    app.request_preview("https://a.example/failed");
    assert!(app.take_effects().is_empty(), "a failure is remembered, not retried every frame");

    for i in 0..501 {
        fetch(&mut app, &format!("https://a.example/{i}"), true);
    }
    assert!(matches!(app.preview("https://a.example/0"), PreviewState::NotLoaded), "the oldest was evicted");
    app.request_preview("https://a.example/0");
    assert!(app.take_effects().iter().any(|e| matches!(e, Effect::LinkPreview { url, .. } if url == "https://a.example/0")));
}
