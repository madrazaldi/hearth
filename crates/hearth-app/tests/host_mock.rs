//! The window against the scripted mock: typing reaches the model's composer,
//! Enter sends, Tab completes without leaving the field, and Cmd/Ctrl+K opens
//! the switcher while the composer has focus.

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, ClipboardItem, Entity, ExternalPaths, FileDropEvent, Image, ImageFormat, InputEvent as _, TestAppContext,
    WindowBounds, WindowOptions, point, px, size,
};
use hearth_app::boot::init_ui;
use hearth_app::host::{Boot, Hearth, hearth, host};
use hearth_app::ui::{keys, shell::ChatWindow, welcome::Welcome};
use hearth_model::buffer::BufId;
use hearth_model::mock::MockOptions;
use std::time::Duration;

/// Move the model's clock and the executor's together so the mock's script plays.
fn advance(cx: &mut TestAppContext, window: AnyWindowHandle, ms: i64) {
    for _ in 0..ms / 50 {
        cx.update(|app| {
            hearth(app).update(app, |h, cx| {
                h.app.set_now(h.app.now() + 50);
                h.tick(cx);
            })
        });
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        draw(cx, window);
    }
}

/// A test window only paints when asked; the key bindings and the focused field exist once it has.
fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.update_window(window, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .unwrap();
}

fn open() -> (TestAppContext, AnyWindowHandle, BufId) {
    let (cx, window, id, _) = open_with(MockOptions::default());
    (cx, window, id)
}

fn open_empty() -> (TestAppContext, AnyWindowHandle, BufId, Entity<ChatWindow>) {
    let mut cx = TestAppContext::single();
    let (window, chat) = cx.update(|app| {
        init_ui(app);
        keys::init(app);
        Hearth::install(Boot::Mock(MockOptions { empty: true, ..MockOptions::default() }), false, app);
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1280.), px(800.)));
        let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
        gpui_kit::open_window(options, app, |window, cx| cx.new(|cx| ChatWindow::new(window, cx))).unwrap()
    });
    cx.update_window(window, |_, w, _| w.activate_window()).unwrap();
    advance(&mut cx, window, 500);
    (cx, window, BufId::server(""), chat)
}

fn open_with(options: MockOptions) -> (TestAppContext, AnyWindowHandle, BufId, Entity<ChatWindow>) {
    let mut cx = TestAppContext::single();
    let (window, chat) = cx.update(|app| {
        init_ui(app);
        keys::init(app);
        Hearth::install(Boot::Mock(options), false, app);
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1280.), px(800.)));
        let options = WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() };
        gpui_kit::open_window(options, app, |window, cx| cx.new(|cx| ChatWindow::new(window, cx))).unwrap()
    });
    cx.update_window(window, |_, w, _| w.activate_window()).unwrap();
    advance(&mut cx, window, 4000);
    let id = cx.update(|app| host(app).app.active().cloned().expect("the mock opens a buffer"));
    (cx, window, id, chat)
}

fn draft(cx: &TestAppContext, id: &BufId) -> String {
    cx.update(|app| host(app).app.composer(id).map(|c| c.text.clone()).unwrap_or_default())
}

#[test]
fn typing_and_enter_send_through_the_model() {
    let (mut cx, window, id) = open();
    cx.simulate_input(window, "hello from a test");
    assert_eq!(draft(&cx, &id), "hello from a test");

    cx.simulate_keystrokes(window, "enter");
    advance(&mut cx, window, 500);
    assert_eq!(draft(&cx, &id), "", "a sent draft is cleared");
    let sent = cx.update(|app| host(app).app.buffer(&id).is_some_and(|b| b.lines().iter().any(|l| l.is_self && l.text == "hello from a test")));
    assert!(sent, "the line shows in the buffer");
}

#[test]
fn tab_completes_a_command_and_keeps_the_focus() {
    let (mut cx, window, id) = open();
    cx.simulate_input(window, "/jo");
    cx.simulate_keystrokes(window, "tab");
    assert!(draft(&cx, &id).starts_with("/join"), "Tab completed the command, got {:?}", draft(&cx, &id));
    cx.simulate_input(window, "x");
    assert!(draft(&cx, &id).contains('x'), "the field still takes typing");
}

#[test]
fn the_switcher_opens_from_the_composer() {
    let (mut cx, window, _) = open();
    let key = if cfg!(target_os = "macos") { "cmd-k" } else { "ctrl-k" };
    cx.simulate_keystrokes(window, key);
    advance(&mut cx, window, 100);
    assert!(cx.update(|app| host(app).app.switcher.is_some()));
    cx.simulate_keystrokes(window, "escape");
    advance(&mut cx, window, 100);
    assert!(cx.update(|app| host(app).app.switcher.is_none()), "Escape closes it again");
}

#[test]
fn only_windows_and_linux_close_a_buffer_with_the_primary_w() {
    let (cx, _, _) = open();
    let bound = cx.update(|app| app.key_bindings().borrow().bindings().any(|b| b.action().name().ends_with("CloseBuffer")));
    // macOS keeps Cmd+W for the window menu's Close.
    assert_eq!(bound, !cfg!(target_os = "macos"));
}

fn primary(key: &str) -> String {
    format!("{}-{key}", if cfg!(target_os = "macos") { "cmd" } else { "ctrl" })
}

fn rows(cx: &TestAppContext, chat: &Entity<ChatWindow>) -> usize {
    cx.update(|app| chat.read(app).visible_rows(app))
}

fn filter_of(cx: &TestAppContext, chat: &Entity<ChatWindow>) -> Option<String> {
    cx.update(|app| chat.read(app).message_filter(app))
}

#[test]
fn the_primary_f_filters_the_open_buffer_in_place() {
    let (mut cx, window, id, chat) = open_with(MockOptions::default());
    let all = rows(&cx, &chat);
    assert!(all > 10, "the mock buffer has a conversation");
    assert_eq!(filter_of(&cx, &chat), None);

    cx.simulate_keystrokes(window, &primary("f"));
    advance(&mut cx, window, 100);
    assert_eq!(filter_of(&cx, &chat), Some(String::new()), "the filter box opened");
    assert!(cx.update(|app| !host(app).app.search_panel.open), "Cmd/Ctrl+F is the in-place filter, not the search panel");

    // The box has the focus, so typing narrows the rows.
    cx.simulate_input(window, "oskar");
    advance(&mut cx, window, 100);
    assert_eq!(filter_of(&cx, &chat).as_deref(), Some("oskar"));
    let some = rows(&cx, &chat);
    assert!(some > 0 && some < all, "the rows narrowed from {all} to {some}");
    assert_eq!(draft(&cx, &id), "", "typing did not reach the composer");
    cx.simulate_input(window, " no-such-word");
    advance(&mut cx, window, 100);
    assert_eq!(rows(&cx, &chat), 0);

    // Escape closes the filter, shows every row again and hands the keyboard back to the composer.
    cx.simulate_keystrokes(window, "escape");
    advance(&mut cx, window, 100);
    assert_eq!(filter_of(&cx, &chat), None);
    assert_eq!(rows(&cx, &chat), all);
    cx.simulate_input(window, "back");
    assert_eq!(draft(&cx, &id), "back");
}

#[test]
fn the_filter_clears_when_the_buffer_changes_and_search_stays_global() {
    let (mut cx, window, _, chat) = open_with(MockOptions::default());
    cx.simulate_keystrokes(window, &primary("f"));
    advance(&mut cx, window, 100);
    cx.simulate_input(window, "oskar");
    advance(&mut cx, window, 100);
    assert_eq!(filter_of(&cx, &chat).as_deref(), Some("oskar"));

    cx.simulate_keystrokes(window, "alt-down");
    advance(&mut cx, window, 200);
    assert_eq!(filter_of(&cx, &chat), None, "another buffer starts unfiltered");

    cx.simulate_keystrokes(window, &primary("shift-f"));
    advance(&mut cx, window, 100);
    assert!(cx.update(|app| host(app).app.search_panel.open), "Cmd/Ctrl+Shift+F is the global search");
}

fn temp_file(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("hearth-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn dropping_a_file_on_the_window_uploads_it_into_the_composer() {
    let (mut cx, window, id, _) = open_with(MockOptions::default());
    assert!(cx.update(|app| host(app).app.can_upload(&id)), "the mock's soju advertises a file host");
    let path = temp_file("notes.txt", b"hello");
    let at = point(px(700.), px(400.));
    let drop = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, w, cx| {
            let paths = || ExternalPaths([path.clone()].into_iter().collect());
            w.dispatch_event(FileDropEvent::Entered { position: at, paths: paths() }.to_platform_input(), cx);
            w.dispatch_event(FileDropEvent::Submit { position: at }.to_platform_input(), cx);
            w.dispatch_event(FileDropEvent::Exited.to_platform_input(), cx);
            w.dispatch_event(FileDropEvent::Ended.to_platform_input(), cx);
        })
        .unwrap();
    };
    drop(&mut cx);
    advance(&mut cx, window, 500);
    assert_eq!(draft(&cx, &id), "https://files.hearth.example/uploads/mock/notes.txt");
    assert_eq!(cx.update(|app| host(app).app.composer(&id).map_or(9, |c| c.uploading)), 0);
}

#[test]
fn pasting_a_picture_uploads_it_and_text_still_pastes() {
    let (mut cx, window, id, _) = open_with(MockOptions::default());
    cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(ImageFormat::Png, vec![0x89, b'P', b'N', b'G'])));
    cx.simulate_keystrokes(window, &primary("v"));
    advance(&mut cx, window, 500);
    assert_eq!(draft(&cx, &id), "https://files.hearth.example/uploads/mock/pasted.png");

    cx.simulate_keystrokes(window, &primary("a"));
    cx.simulate_keystrokes(window, "backspace");
    cx.write_to_clipboard(ClipboardItem::new_string("plain words".into()));
    cx.simulate_keystrokes(window, &primary("v"));
    advance(&mut cx, window, 200);
    assert_eq!(draft(&cx, &id), "plain words", "text is the field's own paste");
}

#[test]
fn the_paperclip_picker_uploads_the_chosen_files() {
    let (mut cx, window, id, chat) = open_with(MockOptions::default());
    let (a, b) = (temp_file("a.png", b"a"), temp_file("b.pdf", b"b"));
    cx.update_window(window, |_, w, cx| chat.update(cx, |c: &mut ChatWindow, cx| c.attach_file(w, cx))).unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && options.multiple && !options.directories);
        Some(vec![a.clone(), b.clone()])
    });
    advance(&mut cx, window, 500);
    let text = draft(&cx, &id);
    assert!(text.contains("uploads/mock/a.png") && text.contains("uploads/mock/b.pdf"), "got {text:?}");
}

fn step(cx: &TestAppContext, chat: &Entity<ChatWindow>) -> Option<&'static str> {
    cx.update(|app| chat.read(app).welcome().map(|w| w.read(app).step_label()))
}

fn welcome<R>(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    chat: &Entity<ChatWindow>,
    f: impl FnOnce(&mut Welcome, &mut gpui_kit::Window, &mut gpui_kit::Context<Welcome>) -> R,
) -> R {
    let w = cx.update(|app| chat.read(app).welcome()).expect("the first-run flow is showing");
    cx.update_window(window, |_, win, app| w.update(app, |w, cx| f(w, win, cx))).unwrap()
}

/// From an empty config: choose a network, give a nickname, skip the account and channels, watch it
/// connect, and arrive in the main window.
#[test]
fn first_run_goes_from_an_empty_config_to_the_main_window() {
    let (mut cx, window, _, chat) = open_empty();
    assert!(cx.update(|app| host(app).app.welcoming()));
    assert_eq!(step(&cx, &chat), Some("Network"), "it starts by asking for a network");

    welcome(&mut cx, window, &chat, |w, win, cx| w.pick_network("libera", win, cx));
    advance(&mut cx, window, 100);
    assert_eq!(step(&cx, &chat), Some("Nickname"));

    // The nickname field has the focus: a bad nick explains itself and stays, a good one moves on.
    cx.simulate_input(window, "9lives");
    cx.simulate_keystrokes(window, "enter");
    advance(&mut cx, window, 100);
    assert_eq!(step(&cx, &chat), Some("Nickname"), "a nick cannot start with a digit");
    cx.simulate_keystrokes(window, &primary("a"));
    cx.simulate_keystrokes(window, "backspace");
    cx.simulate_input(window, "newcomer");
    cx.simulate_keystrokes(window, "enter");
    advance(&mut cx, window, 100);
    assert_eq!(step(&cx, &chat), Some("Account"), "Libera offers an optional account");
    // The account fields are hidden until “I have an account” is on, so Continue is the button.
    welcome(&mut cx, window, &chat, |w, win, cx| w.submit(win, cx));
    advance(&mut cx, window, 100);
    assert_eq!(step(&cx, &chat), Some("Channels"));
    cx.simulate_keystrokes(window, "enter");
    advance(&mut cx, window, 100);
    assert_eq!(step(&cx, &chat), Some("Connect"));
    // The mock saves at once, so the attempt is already past "saving".
    assert_eq!(cx.update(|app| host(app).app.welcome_status()), "connecting");

    advance(&mut cx, window, 4000);
    assert_eq!(cx.update(|app| host(app).app.welcome_status()), "connected");
    assert!(cx.update(|app| host(app).app.welcome.failure.is_none()));

    // "Start chatting" is the card's button; it ends the flow and the main window takes over.
    cx.update(|app| hearth(app).update(app, |h, cx| h.act(cx, |a| a.welcome_done())));
    advance(&mut cx, window, 200);
    assert!(cx.update(|app| !host(app).app.welcoming()));
    assert!(chat.read_with(&cx, |c, _| c.welcome().is_none()), "the flow is gone");
    let active = cx.update(|app| host(app).app.active().cloned());
    assert!(active.is_some(), "a buffer is open: the new network's server buffer");
    assert_eq!(cx.update(|app| host(app).app.servers.len()), 1, "the server was saved");
}

#[test]
fn first_run_explains_an_unreachable_host() {
    let (mut cx, window, _, chat) = open_empty();
    welcome(&mut cx, window, &chat, |w, win, cx| w.pick_network("custom", win, cx));
    welcome(&mut cx, window, &chat, |w, win, cx| w.preview("connect", win, cx));
    advance(&mut cx, window, 100);
    // The mock fails any host that contains "nowhere" at lookup.
    cx.update(|app| {
        hearth(app).update(app, |h, cx| {
            h.act(cx, |a| {
                let mut s =
                    hearth_model::types::ServerConfig { host: "irc.nowhere.invalid".into(), port: 6697, tls: true, nick: "ember".into(), ..Default::default() };
                s.name = "Nowhere".into();
                a.welcome_connect(Some(s))
            })
        })
    });
    advance(&mut cx, window, 4000);
    let friendly = cx.update(|app| host(app).app.welcome_friendly());
    assert!(friendly.is_some(), "the failure is explained in plain language");
    assert_eq!(step(&cx, &chat), Some("Connect"));
}

#[test]
fn lines_arriving_while_the_newest_are_on_screen_are_read_but_not_when_the_window_is_in_the_background() {
    use hearth_model::types::IrcEvent;
    let (mut cx, window, id, _) = open_with(MockOptions::default());
    let unread = |cx: &TestAppContext| cx.update(|app| host(app).app.summary(&id).unread);
    let say = |cx: &mut TestAppContext, text: &str| {
        let (conn, line) = (id.conn.clone(), format!(":mira!m@h PRIVMSG #hearth :{text}"));
        cx.update(|app| {
            hearth(app).update(app, |h, cx| {
                let msg = hearth_model::testing::parse(&line);
                h.act(cx, |a| a.handle_events(&[IrcEvent::Message { conn, msg }]));
            })
        });
    };
    assert_eq!(unread(&cx), 0);
    say(&mut cx, "one");
    advance(&mut cx, window, 100);
    assert_eq!(unread(&cx), 0, "read as it arrived: the buffer is open, in front and at the bottom");

    cx.update(|app| hearth(app).update(app, |h, cx| h.act(cx, |a| a.set_focused(false))));
    say(&mut cx, "two");
    advance(&mut cx, window, 100);
    assert_eq!(unread(&cx), 1, "unread while the window is away");

    cx.update(|app| hearth(app).update(app, |h, cx| h.act(cx, |a| a.set_focused(true))));
    cx.update_window(window, |_, w, _| w.activate_window()).unwrap();
    // Coming back shows the newest lines again, which reads them.
    say(&mut cx, "three");
    advance(&mut cx, window, 100);
    assert_eq!(unread(&cx), 0);
}

#[test]
fn up_and_down_recall_what_was_sent() {
    let (mut cx, window, id, _) = open_with(MockOptions::default());
    for line in ["first", "second"] {
        cx.simulate_input(window, line);
        cx.simulate_keystrokes(window, "enter");
        advance(&mut cx, window, 100);
    }
    cx.simulate_keystrokes(window, "up");
    assert_eq!(draft(&cx, &id), "second");
    cx.simulate_keystrokes(window, "up");
    assert_eq!(draft(&cx, &id), "first");
    cx.simulate_keystrokes(window, "down");
    assert_eq!(draft(&cx, &id), "second");
    cx.simulate_keystrokes(window, "down");
    assert_eq!(draft(&cx, &id), "", "past the newest is the empty box again");
}

#[test]
fn typing_with_nothing_focused_lands_in_the_composer() {
    let (mut cx, window, id, _) = open_with(MockOptions::default());
    cx.update_window(window, |_, w, cx| w.blur(cx)).unwrap();
    draw(&mut cx, window);
    cx.simulate_input(window, "hi");
    assert_eq!(draft(&cx, &id), "hi");
}

#[test]
fn the_nickname_buttons_command_puts_the_nick_change_in_the_box() {
    let (mut cx, window, id, _) = open_with(MockOptions::default());
    let nick = cx.update(|app| host(app).app.conn(&id.conn).map(|c| c.nick.clone())).unwrap();
    // The button calls this and focuses the box.
    cx.update(|app| hearth(app).update(app, |h, cx| h.act(cx, |a| a.composer_prompt_nick(&id))));
    advance(&mut cx, window, 50);
    assert_eq!(draft(&cx, &id), format!("/nick {nick}"));
}

/// `#stress` has about 10k lines and 5k members. Frame cost should not grow with them:
/// run with `cargo test -p hearth-app --release --test host_mock -- --ignored --nocapture`.
#[test]
#[ignore = "timing; run by hand in release mode"]
fn stress_frames_stay_cheap() {
    use hearth_model::mock::Stress;
    let (mut cx, window, _, chat) = open_with(MockOptions { stress: Stress::On, ..MockOptions::default() });
    let started = std::time::Instant::now();
    let frames = 60;
    for i in 0..frames {
        // Scrolling changes what is visible every frame, which is the costly case.
        cx.update(|app| chat.update(app, |c, cx| c.scroll_messages_up(10 + i * 40, cx)));
        draw(&mut cx, window);
    }
    let per = started.elapsed() / frames as u32;
    eprintln!("frame while scrolling 10k lines: {per:?}");
    assert!(per < Duration::from_millis(30), "a frame took {per:?}");
}
