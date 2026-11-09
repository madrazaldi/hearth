//! The window: sidebar, inset main panel (header, messages, composer) and the
//! member list, bound to the host's model. The window holds no chat state of
//! its own: it reads the model while painting, reports what the user did
//! through `handle`, and keeps the dialogs in step with the model's overlay
//! flags (`switcher`, `search_panel.open`, `browser_conn`, `settings_open`,
//! `shortcuts_open`).

use super::bind;
use super::channels::ChannelBrowser;
use super::composer::Composer;
use super::data::*;
use super::header::BufferHeader;
use super::keys::ShellHandle;
use super::lightbox::Lightbox;
use super::members::MemberList;
use super::messages::{MessageList, MessageSettings};
use super::nick_menu::nick_menu;
use super::overlay::{self, Spec};
use super::prelude::*;
use super::search::SearchPanel;
use super::settings::SettingsView;
use super::shortcuts_view::ShortcutsHelp;
use super::sidebar::Sidebar;
use super::switcher::Switcher;
use super::theme::{self, HearthTheme as _, SANS, ThemePref};
use super::welcome::Welcome;
use super::widgets::toast_layer;
use crate::host::{Hearth, hearth, host, request_flush};
use gpui_kit::base::TextSelection;
use gpui_kit::base::input::Copy;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::{ResizableState, h_resizable, resizable_panel};
use hearth_model::buffer::BufId;
use hearth_model::buffer::BufferKind as Kind;
use hearth_model::panels::MemberRow;
use hearth_model::types::{Line, Theme};
use std::rc::Rc;

/// Which dialog is showing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Overlay {
    Switcher,
    Search,
    Channels,
    Settings,
    Shortcuts,
    Image,
}

const OVERLAYS: [Overlay; 6] = [Overlay::Switcher, Overlay::Search, Overlay::Channels, Overlay::Settings, Overlay::Shortcuts, Overlay::Image];

struct Shown {
    kind: Overlay,
    /// Tells a dismissal of this dialog from the late callback of one that was replaced.
    serial: u64,
}

struct MemberCache {
    buffer: BufId,
    rev: u64,
    query: String,
    rows: Rc<Vec<MemberRow>>,
    everyone: usize,
}

pub struct ChatWindow {
    hearth: Entity<Hearth>,
    messages: Entity<MessageList>,
    composer: Entity<Composer>,
    resizable: Entity<ResizableState>,
    dispatch: Dispatch,
    shown: Option<Shown>,
    serial: u64,
    /// The overlay flags as of the last sync, to notice which one was raised.
    flags: [bool; 6],
    image: Option<(String, ImageView)>,
    welcome: Option<Entity<Welcome>>,
    member_filter: Entity<InputState>,
    /// The message filter in the header: open until closed or the buffer changes.
    filter: Entity<InputState>,
    filter_open: bool,
    members: Option<MemberCache>,
    topic_open: bool,
    last_active: Option<BufId>,
    theme: Theme,
    nick_menu: Option<(Entity<PopupMenu>, Point<Pixels>, Subscription)>,
    /// Where the pointer last was: the nick menu opens there.
    pointer: Point<Pixels>,
    _subs: Vec<Subscription>,
}

impl ChatWindow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = hearth(cx);
        let weak = cx.weak_entity();
        let dispatch: Dispatch = Rc::new(move |action, window, cx| {
            let _ = weak.update(cx, |this, cx| this.handle(action, window, cx));
        });
        let messages = cx.new(|cx| MessageList::new(dispatch.clone(), cx));
        let composer = cx.new(|cx| Composer::new(dispatch.clone(), window, cx));
        let resizable = cx.new(|_| ResizableState::default());
        let member_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Find member"));
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter messages"));

        // The old app's root font size was 14px, so every rem-based size (the kit's controls too) is
        // 0.875 of the usual one.
        window.set_rem_size(px(14.));
        let theme = model.read(cx).app.settings.theme;
        theme::apply(ThemePref::from(theme), Some(window), cx);

        let escape_to = cx.weak_entity();
        let subs = vec![
            cx.observe_in(&model, window, |this, _, window, cx| this.sync(window, cx)),
            cx.observe(&member_filter, |_, _, cx| cx.notify()),
            cx.subscribe_in(&filter, window, |this: &mut Self, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.sync(window, cx);
                }
            }),
            // Escape in the filter box closes it, clearing the filter.
            cx.intercept_keystrokes(move |ev, window, cx| {
                let Some(this) = escape_to.upgrade() else { return };
                let k = &ev.keystroke;
                if k.key == "escape" {
                    if this.update(cx, |s, cx| s.escape_filter(window, cx)) {
                        cx.stop_propagation();
                    }
                } else if k.key_char.is_some() && !k.modifiers.control && !k.modifiers.platform && !k.modifiers.alt && !k.modifiers.function {
                    // Typing with nothing focused (after clicking the messages) lands in the composer.
                    let typed = k.key_char.clone().unwrap_or_default();
                    if this.update(cx, |s, cx| s.type_to_compose(&typed, window, cx)) {
                        cx.stop_propagation();
                    }
                }
            }),
            cx.observe_window_appearance(window, |this: &mut Self, window, cx| {
                if ThemePref::from(this.theme) == ThemePref::System {
                    theme::apply(ThemePref::System, Some(window), cx);
                }
            }),
            // Auto-away and read markers follow whether the window is in front.
            cx.observe_window_activation(window, |this: &mut Self, window, cx| {
                let active = window.is_window_active();
                this.act(cx, move |app| app.set_focused(active));
            }),
        ];
        cx.set_global(ShellHandle(cx.weak_entity()));

        let mut this = ChatWindow {
            hearth: model,
            messages,
            composer,
            resizable,
            dispatch,
            shown: None,
            serial: 0,
            flags: [false; 6],
            image: None,
            welcome: None,
            member_filter,
            filter,
            filter_open: false,
            members: None,
            topic_open: false,
            last_active: None,
            theme,
            nick_menu: None,
            pointer: point(px(0.), px(0.)),
            _subs: subs,
        };
        let active = window.is_window_active();
        this.act(cx, move |app| app.set_focused(active));
        this.sync(window, cx);
        // The dialog layer exists once the window is up.
        cx.defer_in(window, |this, window, cx| this.open_from_env(window, cx));
        this
    }

    /// Scroll the conversation up from its end; the screenshots use it to show talk rather than media.
    pub fn scroll_messages_up(&mut self, rows: usize, cx: &mut Context<Self>) {
        self.messages.update(cx, |m, cx| m.scroll_up_by_rows(rows, cx));
    }

    /// The first-run flow, while it covers the window.
    pub fn welcome(&self) -> Option<Entity<Welcome>> {
        self.welcome.clone()
    }

    /// The text of the open message filter, or `None` while it is closed.
    pub fn message_filter(&self, cx: &App) -> Option<String> {
        self.filter_open.then(|| self.filter.read(cx).value().to_string())
    }

    /// Open the message filter with `text` in it (the screenshots show it this way).
    pub fn set_message_filter(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_open = true;
        self.filter.update(cx, |s, cx| s.set_value(text.to_string(), window, cx));
        self.sync(window, cx);
    }

    /// How many rows the message list shows (after the filter).
    pub fn visible_rows(&self, cx: &App) -> usize {
        self.messages.read(cx).row_count()
    }

    /// Open the file picker of the composer's paperclip.
    pub fn attach_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |c, cx| c.attach(window, cx));
    }

    /// Change the model through the host, which performs whatever that asks for.
    fn act<R>(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut hearth_model::App) -> R) -> R {
        self.hearth.update(cx, |h, cx| h.act(cx, f))
    }

    fn active(&self, cx: &App) -> Option<BufId> {
        self.hearth.read(cx).app.active().cloned()
    }

    /// Push the model's current state into the stateful child views and the dialogs.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.active(cx);
        let switched = active != self.last_active;
        if switched {
            self.topic_open = false;
            self.last_active = active.clone();
            if self.filter_open {
                self.close_filter(window, cx);
            }
        }
        let filter = self.filter_text(cx);
        let (theme, view, composer, st, welcoming) = {
            let h = self.hearth.read(cx);
            (h.app.settings.theme, bind::messages(h, &filter), bind::composer(h), MessageSettings::from(&h.app.settings), h.app.welcoming())
        };
        if theme != self.theme {
            self.theme = theme;
            theme::apply(ThemePref::from(theme), Some(window), cx);
        }
        if let Some(v) = view {
            self.messages.update(cx, |m, cx| m.set_view(v, st, cx));
        }
        if let Some(c) = composer {
            self.composer.update(cx, |comp, cx| comp.set_view(c, window, cx));
        }
        // A jump (search result, reply link) left a request to scroll to a line.
        if let Some(id) = active.clone()
            && let Some(t) = self.act(cx, |app| app.take_focus(&id, &filter))
        {
            self.messages.update(cx, |m, cx| m.scroll_to_item(t.item, cx));
        }

        // The newest lines are on screen and the window is in front: they count as read, whether they
        // arrived just now, the buffer was just opened or the window just regained focus (the old list's effect).
        if let Some(id) = active.clone().filter(|_| !welcoming && filter.trim().is_empty()) {
            self.act(cx, move |app| {
                if switched {
                    app.at_bottom = true;
                }
                if app.focused && app.at_bottom {
                    app.mark_read(&id);
                }
            });
        }

        match (welcoming, self.welcome.is_some()) {
            (true, false) => {
                let d = self.dispatch.clone();
                self.welcome = Some(cx.new(|cx| Welcome::new(d, window, cx)));
            }
            (false, true) => {
                self.welcome = None;
                self.composer.update(cx, |c, cx| c.focus(window, cx));
            }
            _ => {}
        }
        self.reconcile(window, cx);
        // Opening a buffer from the sidebar or the switcher leaves typing where it belongs.
        if switched && self.shown.is_none() && self.welcome.is_none() && self.nick_menu.is_none() && window.is_window_active() {
            self.composer.update(cx, |c, cx| c.focus(window, cx));
        }
        cx.notify();
    }

    // ── Overlays ────────────────────────────────────────────

    fn flags_now(&self, cx: &App) -> [bool; 6] {
        let a = &self.hearth.read(cx).app;
        [a.switcher.is_some(), a.search_panel.open, a.browser_conn.is_some(), a.settings_open.is_some(), a.shortcuts_open, self.image.is_some()]
    }

    /// Drop the model's state for every overlay but `keep`.
    fn close_others(&mut self, keep: Overlay, cx: &mut Context<Self>) {
        self.act(cx, |app| {
            if keep != Overlay::Switcher && app.switcher.is_some() {
                app.close_switcher();
            }
            if keep != Overlay::Search && app.search_panel.open {
                app.search_close();
            }
            if keep != Overlay::Channels && app.browser_conn.is_some() {
                app.close_channel_browser();
            }
            if keep != Overlay::Settings && app.settings_open.is_some() {
                app.close_settings();
            }
            if keep != Overlay::Shortcuts && app.shortcuts_open {
                app.toggle_shortcuts();
            }
        });
        if keep != Overlay::Image {
            self.image = None;
        }
    }

    /// Show the dialog the model (or an image click) asks for, and only that one.
    fn reconcile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let welcoming = self.welcome.is_some();
        let mut flags = self.flags_now(cx);
        let fresh = (0..6).find(|&i| flags[i] && !self.flags[i]);
        if let (Some(i), false) = (fresh, welcoming) {
            // Something new was asked for while another dialog was up: the new one wins.
            if flags.iter().filter(|f| **f).count() > 1 {
                self.close_others(OVERLAYS[i], cx);
                flags = self.flags_now(cx);
            }
        }
        self.flags = flags;
        let want = if welcoming {
            None
        } else {
            fresh
                .map(|i| OVERLAYS[i])
                .or(self.shown.as_ref().map(|s| s.kind).filter(|k| flags[OVERLAYS.iter().position(|o| o == k).unwrap_or(0)]))
                .or_else(|| (0..6).find(|&i| flags[i]).map(|i| OVERLAYS[i]))
        };
        let current = self.shown.as_ref().map(|s| s.kind);
        if want == current {
            return;
        }
        if current.is_some() {
            self.shown = None;
            overlay::close_all(window, cx);
        }
        if let Some(kind) = want {
            self.open_overlay(kind, window, cx);
        }
    }

    fn open_overlay(&mut self, kind: Overlay, window: &mut Window, cx: &mut Context<Self>) {
        self.serial += 1;
        let serial = self.serial;
        let d = self.dispatch.clone();
        let closed = {
            let d = d.clone();
            move |window: &mut Window, cx: &mut App| {
                let d = d.clone();
                // The shell may be mid-update when a dialog closes under it.
                window.defer(cx, move |window, cx| d(UiAction::DialogClosed(serial), window, cx));
            }
        };
        self.shown = Some(Shown { kind, serial });
        match kind {
            Overlay::Switcher => {
                let e = cx.new(|cx| Switcher::new(d, window, cx));
                let f = e.clone();
                overlay::open(
                    window,
                    cx,
                    e.into(),
                    Spec { width: 504., top: Some(112.), close_button: false },
                    move |w, cx| f.update(cx, |s, cx| s.focus(w, cx)),
                    closed,
                );
            }
            Overlay::Search => {
                let e = cx.new(|cx| SearchPanel::new(d, window, cx));
                let f = e.clone();
                overlay::open(
                    window,
                    cx,
                    e.into(),
                    Spec { width: 588., top: Some(96.), close_button: false },
                    move |w, cx| f.update(cx, |s, cx| s.focus(w, cx)),
                    closed,
                );
            }
            Overlay::Channels => {
                let e = cx.new(|cx| ChannelBrowser::new(d, window, cx));
                let f = e.clone();
                overlay::open(
                    window,
                    cx,
                    e.into(),
                    Spec { width: 672., top: None, close_button: true },
                    move |w, cx| f.update(cx, |s, cx| s.focus(w, cx)),
                    closed,
                );
            }
            Overlay::Settings => {
                let e = cx.new(|cx| SettingsView::new(d, window, cx));
                overlay::open(window, cx, e.into(), Spec { width: 784., top: None, close_button: true }, |_, _| {}, closed);
            }
            Overlay::Shortcuts => {
                let e = cx.new(|_| ShortcutsHelp);
                overlay::open(window, cx, e.into(), Spec { width: 448., top: None, close_button: true }, |_, _| {}, closed);
            }
            Overlay::Image => {
                let Some((url, image)) = self.image.clone() else { return };
                let width = Lightbox::width(&image, window.viewport_size());
                let e = cx.new(|_| Lightbox::new(url, image, d));
                overlay::open(window, cx, e.into(), Spec { width, top: None, close_button: true }, |_, _| {}, closed);
            }
        }
    }

    /// The dialog was dismissed by the user: the model forgets it was open.
    fn dialog_closed(&mut self, serial: u64, cx: &mut Context<Self>) {
        let Some(shown) = self.shown.as_ref().filter(|s| s.serial == serial) else { return };
        let kind = shown.kind;
        self.shown = None;
        self.act(cx, |app| match kind {
            Overlay::Switcher => app.close_switcher(),
            Overlay::Search => app.search_close(),
            Overlay::Channels => app.close_channel_browser(),
            Overlay::Settings => app.close_settings(),
            Overlay::Shortcuts => app.toggle_shortcuts(),
            Overlay::Image => {}
        });
        if kind == Overlay::Image {
            self.image = None;
            self.flags[5] = false;
        }
    }

    fn open_nick(&mut self, nick: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = bind::nick_menu(self.hearth.read(cx), nick) else { return };
        let d = self.dispatch.clone();
        let menu = PopupMenu::build(window, cx, move |m, _, cx| nick_menu(m, &view, &d, cx));
        let sub = cx.subscribe(&menu, |this: &mut Self, _, _: &DismissEvent, cx| {
            this.nick_menu = None;
            cx.notify();
        });
        window.focus(&menu.focus_handle(cx), cx);
        self.nick_menu = Some((menu, self.pointer, sub));
        cx.notify();
        request_flush(cx);
    }

    /// `HEARTH_VIEW=nickmenu` opens the menu on a nick at launch; the other views are the model's own
    /// (`mock::start` raises their flags), so they open through `sync`.
    fn open_from_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::env::var("HEARTH_VIEW").as_deref() == Ok("nickmenu") {
            self.pointer = point(px(760.), px(300.));
            let nick = std::env::var("HEARTH_NICK").unwrap_or_else(|_| "mira".into());
            self.open_nick(&nick, window, cx);
        }
    }

    // ── Shortcuts ───────────────────────────────────────────

    fn locked(&self) -> bool {
        self.welcome.is_some()
    }

    pub(super) fn toggle_switcher(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.locked() {
            self.act(cx, |app| app.toggle_switcher());
        }
    }

    pub(super) fn toggle_search(&mut self, scope: Option<hearth_model::search_panel::PanelScope>, _: &mut Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        self.act(cx, |app| {
            if app.search_panel.open && scope.is_none() {
                app.search_close();
                return;
            }
            app.search_open();
            if let Some(s) = scope.filter(|_| app.search_panel.origin.is_some()) {
                app.search_set_scope(s);
            }
        });
    }

    /// What the filter box holds while it is open.
    fn filter_text(&self, cx: &App) -> String {
        if self.filter_open { self.filter.read(cx).value().to_string() } else { String::new() }
    }

    /// Cmd/Ctrl+F: open the message filter in the header and focus it (select its text if already open).
    pub(super) fn filter_messages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.locked() || self.active(cx).is_none() {
            return;
        }
        if !self.filter_open {
            self.filter_open = true;
            cx.notify();
        }
        // The field exists in the tree after the next frame; focusing earlier would find nothing to focus.
        cx.defer_in(window, |this, window, cx| {
            this.filter.update(cx, |s, cx| {
                s.focus(window, cx);
                s.select_all(window, cx);
            })
        });
    }

    /// Files dropped on the window go to the open buffer's file host.
    fn drop_files(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let (Some(id), false) = (self.active(cx), self.locked()) else { return };
        let paths = paths.paths().to_vec();
        self.hearth.update(cx, |h, cx| h.upload_paths(&id, &paths, cx));
    }

    /// What the drop target says while files are dragged over the window.
    fn drop_label(&self, cx: &App) -> String {
        let app = &self.hearth.read(cx).app;
        match app.active() {
            Some(id) if app.can_upload(id) => format!("Drop to upload to {}", app.conn(&id.conn).map(|c| c.name().to_string()).unwrap_or_default()),
            _ => "This server does not accept file uploads".to_string(),
        }
    }

    /// `typed` was pressed while nothing had the keyboard: the composer takes the focus and the
    /// character (the key is consumed, as the field was not there to receive it). True when it did.
    fn type_to_compose(&mut self, typed: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if typed.is_empty() || self.locked() || overlay::is_open(window, cx) || self.nick_menu.is_some() || window.focused(cx).is_some() {
            return false;
        }
        let Some(id) = self.active(cx) else { return false };
        self.act(cx, |app| {
            let (text, caret) = app.composer(&id).map(|c| (c.text.clone(), c.caret.min(c.text.len()))).unwrap_or_default();
            let next = format!("{}{typed}{}", &text[..caret], &text[caret..]);
            app.composer_edit(&id, &next, caret + typed.len());
        });
        self.sync(window, cx);
        self.composer.update(cx, |c, cx| c.focus(window, cx));
        true
    }

    /// Hide the filter and show every line again.
    fn close_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.filter_open {
            return;
        }
        self.filter_open = false;
        self.filter.update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    /// Escape while typing in the filter closes it; false when the key is not for the filter.
    fn escape_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.filter_open || !self.filter.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        self.close_filter(window, cx);
        self.sync(window, cx);
        self.composer.update(cx, |c, cx| c.focus(window, cx));
        true
    }

    pub(super) fn toggle_shortcuts(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.locked() {
            self.act(cx, |app| app.toggle_shortcuts());
        }
    }

    pub(super) fn toggle_settings(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.locked() {
            return;
        }
        self.act(cx, |app| if app.settings_open.is_some() { app.close_settings() } else { app.open_settings("appearance", None) });
    }

    pub(super) fn toggle_members(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.act(cx, |app| app.toggle_members());
    }

    fn browsing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        !self.locked() && !overlay::is_open(window, cx)
    }

    pub(super) fn step_buffer(&mut self, delta: i32, unread: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.browsing(window, cx) {
            self.act(cx, |app| app.step(delta, unread));
        }
    }

    pub(super) fn go_nth(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.browsing(window, cx) {
            return;
        }
        self.act(cx, |app| {
            let Some(conn) = app.active().map(|a| a.conn.clone()) else { return };
            let target = app.sidebar().into_iter().filter(|s| s.conn == conn && s.kind == Kind::Channel).filter_map(|s| s.buf).nth(n.saturating_sub(1));
            if let Some(id) = target {
                app.open(&id);
            }
        });
    }

    pub(super) fn close_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.browsing(window, cx) {
            return;
        }
        if let Some(id) = self.active(cx) {
            self.act(cx, |app| app.close_buffer(&id));
        }
    }

    // ── Actions ─────────────────────────────────────────────

    /// A line of the open buffer by id.
    fn line(&self, id: u64, cx: &App) -> Option<(BufId, Line)> {
        let app = &self.hearth.read(cx).app;
        let buf = app.active()?.clone();
        let line = app.buffer(&buf)?.lines().iter().rev().find(|l| l.id == id)?.clone();
        Some((buf, line))
    }

    fn handle(&mut self, action: UiAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            UiAction::DialogClosed(serial) => self.dialog_closed(serial, cx),
            UiAction::OpenUrl(url) => {
                if let Err(e) = crate::platform::open_url(cx, &url) {
                    self.act(cx, |app| app.toast_error(e));
                }
            }
            UiAction::OpenBuffer(id) => self.act(cx, |app| app.open(&id)),
            UiAction::ToggleNetwork(conn) => self.act(cx, |app| app.toggle_collapsed(&conn)),
            UiAction::CloseBuffer(id) => self.act(cx, |app| app.close_buffer(&id)),
            UiAction::Rejoin(id) => self.act(cx, |app| {
                if let Some(name) = app.buffer(&id).map(|b| b.name.clone()) {
                    app.join(&id.conn, &name, None);
                }
            }),
            UiAction::MarkRead(id) => self.act(cx, |app| app.mark_read(&id)),
            UiAction::SetNotify(id, level) => self.act(cx, |app| app.set_notify(&id, level)),
            UiAction::OpenNick(nick) => self.open_nick(&nick, window, cx),
            UiAction::JoinChannel(name) => {
                if let Some(a) = self.active(cx) {
                    self.act(cx, |app| app.join(&a.conn, &name, None));
                }
            }
            UiAction::Reply(id) => {
                if let Some((buf, line)) = self.line(id, cx) {
                    self.act(cx, |app| app.start_reply(&buf, &line));
                    self.composer.update(cx, |c, cx| c.focus(window, cx));
                }
            }
            UiAction::CancelReply => {
                if let Some(a) = self.active(cx) {
                    self.act(cx, |app| app.cancel_reply(&a));
                }
            }
            UiAction::React(id, emoji) => {
                if let Some((buf, line)) = self.line(id, cx) {
                    self.act(cx, |app| app.react(&buf, &line, &emoji));
                }
            }
            UiAction::CopyLine(id) => {
                if let Some((_, line)) = self.line(id, cx) {
                    self.act(cx, |app| app.copy_line(&line));
                }
            }
            UiAction::DeleteLine(id) => {
                if let Some((buf, line)) = self.line(id, cx) {
                    self.act(cx, |app| app.redact(&buf, &line));
                }
            }
            UiAction::ToggleMembers => self.act(cx, |app| app.toggle_members()),
            UiAction::OpenSearch => self.toggle_search(None, window, cx),
            UiAction::OpenFilter => self.filter_messages(window, cx),
            UiAction::CloseFilter => {
                self.close_filter(window, cx);
                self.sync(window, cx);
                self.composer.update(cx, |c, cx| c.focus(window, cx));
            }
            UiAction::OpenSwitcher => self.act(cx, |app| app.toggle_switcher()),
            UiAction::AddNetwork => self.act(cx, |app| app.open_settings("servers", Some("new"))),
            UiAction::OpenSettings => self.act(cx, |app| app.open_settings("appearance", None)),
            UiAction::SetSidebarWidth(w) => self.act(cx, |app| app.set_sidebar_width(w.round() as i32)),
            UiAction::ShowTopic => {
                self.topic_open = !self.topic_open;
                cx.notify();
            }
            UiAction::OpenImage { url, image } => {
                self.image = Some((url, image));
                self.sync(window, cx);
            }

            // Nick menu
            UiAction::OpenQuery(nick) => {
                if let Some(a) = self.active(cx) {
                    self.act(cx, |app| {
                        if let Some(id) = app.query(&a.conn, &nick) {
                            app.open(&id);
                        }
                    });
                }
            }
            UiAction::Mention(nick) => {
                if let Some(a) = self.active(cx) {
                    self.act(cx, |app| app.composer_mention(&a, &format!("{nick}: ")));
                    self.composer.update(cx, |c, cx| c.focus(window, cx));
                }
            }
            UiAction::Whois(nick) => self.run(&format!("/whois {nick}"), cx),
            UiAction::Watch(nick, on) => {
                if let Some(a) = self.active(cx) {
                    self.act(cx, |app| if on { app.watch(&a.conn, &nick) } else { app.unwatch(&a.conn, &nick) });
                }
            }
            UiAction::Ignore(nick, on) => self.act(cx, |app| {
                if on {
                    app.ignore(&nick);
                } else {
                    app.unignore(&nick);
                }
            }),
            UiAction::ChannelMode { nick, mode, on } => {
                let cmd = match (mode, on) {
                    ('o', true) => "op",
                    ('o', false) => "deop",
                    ('v', true) => "voice",
                    _ => "devoice",
                };
                self.run(&format!("/{cmd} {nick}"), cx);
            }
            UiAction::Kick(nick) => self.run(&format!("/kick {nick}"), cx),
        }
    }

    /// Run a slash command in the open buffer.
    fn run(&mut self, line: &str, cx: &mut Context<Self>) {
        if let Some(a) = self.active(cx) {
            self.act(cx, |app| app.run_input(&a, line, None));
        }
    }
}

impl Render for ChatWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let d = self.dispatch.clone();
        if let Some(w) = &self.welcome {
            return div()
                .size_full()
                .bg(p.background)
                .text_color(p.foreground)
                .font_family(SANS)
                .text_size(px(14.))
                .child(w.clone())
                .children(toast_layer(&self.hearth, &p, cx))
                .into_any_element();
        }
        let (show_members, sidebar_w) = {
            let s = &host(cx).app.settings;
            (s.show_members, s.sidebar_width as f32)
        };

        let sidebar = Sidebar::new(bind::sidebar(host(cx)), d.clone());
        let header = bind::header(host(cx));
        let notice = bind::connection_notice(host(cx));
        if host(cx).images.pending() {
            request_flush(cx);
        }
        let main = match header {
            Some(header) => {
                let is_channel = header.kind == BufferKind::Channel;
                let members = (show_members && is_channel).then(|| self.member_list(&d, cx));
                v_flex()
                    .size_full()
                    .child(BufferHeader::new(header, d.clone()).topic_open(self.topic_open).filter(self.filter_open.then(|| self.filter.clone())))
                    .children(notice.map(|(text, busy)| connection_strip(&text, busy, &p)))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .child(
                                        // A press in the chat may start a selection there, which then
                                        // replaces the composer's as the one Copy takes.
                                        div().flex_1().min_h_0().flex().flex_col().child(self.messages.clone()).on_any_mouse_down(cx.listener(
                                            |this, ev: &MouseDownEvent, _, cx| {
                                                if ev.button == MouseButton::Left {
                                                    this.composer.update(cx, |c, cx| c.collapse_selection(cx));
                                                }
                                            },
                                        )),
                                    )
                                    .child(self.composer.clone()),
                            )
                            .children(members),
                    )
                    .into_any_element()
            }
            None => div().size_full().flex().items_center().justify_center().text_color(p.muted_foreground).child("Connecting…").into_any_element(),
        };

        let dispatch = d.clone();
        let drop_label = self.drop_label(cx);
        let menu =
            self.nick_menu.as_ref().map(|(m, at, _)| deferred(anchored().position(*at).snap_to_window_with_margin(px(8.)).child(m.clone())).with_priority(1));
        let _ = window;
        div()
            .size_full()
            .bg(p.sidebar)
            .text_color(p.foreground)
            .font_family(SANS)
            .text_size(px(14.))
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, _| this.pointer = ev.position))
            // The composer keeps the focus, and its own Copy would take the
            // keystroke and copy its empty selection. Text dragged out in the
            // chat goes first, unless the composer has a selection made since.
            .capture_action(cx.listener(|this, _: &Copy, window, cx| {
                if this.composer.read(cx).has_selection(cx) {
                    return;
                }
                let text = TextSelection::selected_text(window, cx).trim().to_string();
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    cx.stop_propagation();
                }
            }))
            .child(
                h_resizable("shell")
                    .with_state(&self.resizable)
                    .on_resize(move |state, window, cx| {
                        if let Some(w) = state.read(cx).sizes().first() {
                            dispatch(UiAction::SetSidebarWidth(f32::from(*w)), window, cx);
                        }
                    })
                    .child(resizable_panel().size(px(sidebar_w)).size_range(px(190.)..px(420.)).fixed().child(sidebar))
                    .child(
                        resizable_panel().child(
                            // The conversation is a panel inset into the window, which is the sidebar's colour.
                            div().size_full().pt(px(8.)).pr(px(8.)).pb(px(8.)).child(
                                div()
                                    .relative()
                                    .size_full()
                                    .overflow_hidden()
                                    .rounded(px(14.))
                                    .bg(p.background)
                                    .border_1()
                                    .border_color(p.border)
                                    .shadow_xs()
                                    .child(main)
                                    .child(
                                        // Invisible until files are dragged over the window; the drag style fades it in.
                                        div()
                                            .absolute()
                                            .inset_0()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(14.))
                                            .opacity(0.)
                                            .bg(p.background.opacity(0.85))
                                            .border_2()
                                            .border_color(p.brand)
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.brand)
                                            .drag_over::<ExternalPaths>(|s, _, _, _| s.opacity(1.))
                                            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.drop_files(paths, cx)))
                                            .child(drop_label),
                                    ),
                            ),
                        ),
                    ),
            )
            .children(menu)
            .children(toast_layer(&self.hearth, &p, cx))
            .into_any_element()
    }
}

impl ChatWindow {
    /// The member list for the open channel, rebuilt only when the buffer or the find text changed.
    fn member_list(&mut self, d: &Dispatch, cx: &mut Context<Self>) -> MemberList {
        let query = self.member_filter.read(cx).value().trim().to_lowercase();
        let h = self.hearth.read(cx);
        let Some(b) = h.app.active_buffer() else { return MemberList::new(BufId::server(""), Rc::new(Vec::new()), 0, d.clone()) };
        let (id, rev) = (b.id(), b.rev());
        let fresh = self.members.as_ref().is_some_and(|m| m.buffer == id && m.rev == rev && m.query == query);
        if !fresh {
            let rows = Rc::new(h.app.member_rows(&id, &query));
            let everyone = b.member_count();
            self.members = Some(MemberCache { buffer: id.clone(), rev, query, rows, everyone });
        }
        let m = self.members.as_ref().expect("filled above");
        MemberList::new(id, m.rows.clone(), m.everyone, d.clone()).find(self.member_filter.clone())
    }
}

/// The warning under the header while the network is not connected.
fn connection_strip(text: &str, busy: bool, p: &super::theme::Palette) -> impl IntoElement + use<> {
    let color = if busy { p.warn } else { p.destructive };
    h_flex()
        .flex_none()
        .h(px(28.))
        .px(px(16.))
        .gap(px(8.))
        .items_center()
        .border_b_1()
        .border_color(p.border)
        .bg(color.opacity(0.1))
        .text_size(px(12.5))
        .text_color(color)
        .child(div().size(px(6.)).rounded_full().bg(color))
        .child(text.to_string())
}
