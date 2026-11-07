//! The settings modal: a tab rail and one body per section. The pages read the model through `bind::settings_data` and
//! write to it through the host; the open page is the model's `settings_open`.

mod aliases;
mod bouncer;
mod highlights;
pub mod server_form;

use self::aliases::{AliasEditor, AliasEvent};
use self::bouncer::BouncerEditor;
use self::highlights::{HighlightsEditor, HighlightsEvent};
use self::server_form::{FormEvent, ServerForm};
use super::avatar::icon;
use super::bind;
use super::controls::*;
use super::data::*;
use super::prelude::*;
use super::shortcuts_view::shortcut_list;
use super::theme::{HearthTheme as _, MONO, Palette};
use crate::host::updates::UpdatePhase;
use crate::host::{hearth, host};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{Disableable as _, Sizable as _};
use hearth_model::Effect;
use hearth_model::ignore::normalize_mask;
use hearth_model::types::{Clock, Layout, Membership, ServerConfig, Settings, Theme};

struct Edit {
    id: Option<String>,
    form: Entity<ServerForm>,
    bouncer: Option<Entity<BouncerEditor>>,
    highlights: Option<Entity<HighlightsEditor>>,
    _subs: Vec<Subscription>,
}

pub struct SettingsView {
    data: SettingsData,
    tab: SettingsTab,
    edit: Option<Edit>,
    words: Entity<InputState>,
    ignore: Entity<InputState>,
    aliases: Entity<AliasEditor>,
    confirm_clear: bool,
    _subs: Vec<Subscription>,
}

const TABS: [(SettingsTab, &str, &str); 7] = [
    (SettingsTab::Servers, "Servers", "server"),
    (SettingsTab::General, "General", "settings"),
    (SettingsTab::Appearance, "Appearance", "moon"),
    (SettingsTab::Notifications, "Notifications", "bell"),
    (SettingsTab::Aliases, "Aliases", "square-terminal"),
    (SettingsTab::Privacy, "Privacy & data", "lock"),
    (SettingsTab::Shortcuts, "Shortcuts", "keyboard"),
];

impl SettingsView {
    pub fn new(_: Dispatch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = hearth(cx);
        let (data, tab, server) = {
            let h = model.read(cx);
            let open = h.app.settings_open.clone();
            let tab = open.as_ref().and_then(|o| SettingsTab::parse(&o.tab)).unwrap_or(SettingsTab::Appearance);
            (bind::settings_data(h), tab, open.and_then(|o| o.server_id))
        };
        let words = new_input(window, cx, &data.settings.highlight_words.join(", "), "hearth, deploy");
        let ignore = new_input(window, cx, "", "nick or nick!user@host");
        let aliases = cx.new(|cx| AliasEditor::new(data.settings.aliases.clone(), window, cx));
        let subs = vec![
            cx.observe_in(&model, window, |this: &mut Self, _, window, cx| this.refresh(window, cx)),
            cx.subscribe_in(&words, window, |this: &mut Self, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    let list = text(&this.words, cx).split(',').map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect();
                    this.change(window, cx, move |s| s.highlight_words = list);
                }
            }),
            cx.subscribe_in(&ignore, window, |this: &mut Self, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::PressEnter { .. }) {
                    this.add_ignore(window, cx);
                }
            }),
            cx.subscribe_in(&aliases, window, |this: &mut Self, _, ev: &AliasEvent, window, cx| match ev {
                AliasEvent::Set(n, b) => {
                    let (n, b) = (n.clone(), b.clone());
                    this.change(window, cx, move |s| {
                        s.aliases.insert(n, b);
                    });
                }
                AliasEvent::Remove(n) => {
                    let n = n.clone();
                    this.change(window, cx, move |s| {
                        s.aliases.remove(&n);
                    });
                }
            }),
        ];
        let mut view = SettingsView { data, tab, edit: None, words, ignore, aliases, confirm_clear: false, _subs: subs };
        view.show(tab, server, window, cx);
        view
    }

    /// The model changed: take its settings, servers and open page.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (data, open) = {
            let h = host(cx);
            (bind::settings_data(h), h.app.settings_open.clone())
        };
        self.data = data;
        if let Some(o) = open {
            let tab = SettingsTab::parse(&o.tab).unwrap_or(self.tab);
            let editing = self.edit.as_ref().map(|e| e.id.clone().unwrap_or_else(|| "new".into()));
            if tab != self.tab || o.server_id != editing {
                self.show(tab, o.server_id, window, cx);
            }
        }
        self.sync(cx);
    }

    /// Ask the model to show a page; `refresh` follows.
    fn goto(&mut self, tab: SettingsTab, server: Option<String>, _: &mut Window, cx: &mut Context<Self>) {
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.open_settings(tab.id(), server.as_deref())));
    }

    fn show(&mut self, tab: SettingsTab, server: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.tab = tab;
        self.edit = None;
        if tab == SettingsTab::Servers
            && let Some(id) = server
        {
            self.open_server(&id, window, cx);
        }
        cx.notify();
    }

    fn open_server(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let is_new = id == "new";
        let row = self.data.servers.iter().find(|r| r.config.id == id).cloned();
        let cfg: ServerConfig = if is_new {
            ServerConfig::default()
        } else if let Some(r) = &row {
            r.config.clone()
        } else {
            return;
        };
        let label = if is_new { "Add & connect" } else { "Save & reconnect" };
        let form = cx.new(|cx| ServerForm::new(cfg.clone(), label, !is_new, window, cx));
        let mut subs = vec![cx.subscribe_in(&form, window, |this: &mut Self, _, ev: &FormEvent, window, cx| {
            let (delete, save) = match ev {
                FormEvent::Cancel => (None, None),
                FormEvent::Delete(id) => (Some(id.clone()), None),
                FormEvent::Save(cfg) => (None, Some((**cfg).clone())),
            };
            hearth(cx).update(cx, |h, cx| {
                h.act(cx, |app| {
                    if let Some(id) = delete {
                        app.delete_server(&id);
                    }
                    if let Some(cfg) = save {
                        app.save_server(cfg, true);
                    }
                })
            });
            this.goto(SettingsTab::Servers, None, window, cx);
        })];
        let bouncer = row.as_ref().filter(|r| r.is_bouncer && !is_new).map(|r| {
            let (conn, nets) = (r.config.id.clone(), r.networks.clone());
            cx.new(|cx| {
                let mut b = BouncerEditor::new(conn, Vec::new(), window, cx);
                b.set_networks(nets, cx);
                b
            })
        });
        let highlights = (!is_new).then(|| {
            let (nick, global) = (cfg.nick.clone(), self.data.settings.highlight_words.clone());
            let initial = self.data.settings.network_highlights.get(&cfg.id).cloned().unwrap_or_default();
            let h = cx.new(|cx| HighlightsEditor::new(cfg.id.clone(), nick, global, initial, window, cx));
            subs.push(cx.subscribe_in(&h, window, |this: &mut Self, _, ev: &HighlightsEvent, window, cx| {
                let HighlightsEvent::Save(id, value) = ev;
                let (id, value) = (id.clone(), value.clone());
                this.change(window, cx, move |s| match value {
                    Some(v) => {
                        s.network_highlights.insert(id, v);
                    }
                    None => {
                        s.network_highlights.remove(&id);
                    }
                });
            }));
            h
        });
        self.edit = Some(Edit { id: (!is_new).then(|| id.to_string()), form, bouncer, highlights, _subs: subs });
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let aliases = self.data.settings.aliases.clone();
        self.aliases.update(cx, |a, cx| a.set_aliases(aliases, cx));
        if let Some(e) = &self.edit {
            let words = self.data.settings.highlight_words.clone();
            if let Some(h) = &e.highlights {
                h.update(cx, |h, cx| h.set_global_words(words, cx));
            }
            if let (Some(b), Some(id)) = (&e.bouncer, &e.id)
                && let Some(r) = self.data.servers.iter().find(|r| &r.config.id == id)
            {
                let nets = r.networks.clone();
                b.update(cx, |b, cx| b.set_networks(nets, cx));
            }
        }
        cx.notify();
    }

    /// Apply an edit to the local copy and report the whole value.
    fn change(&mut self, window: &mut Window, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        let _ = window;
        f(&mut self.data.settings);
        let apply = self.data.settings.clone();
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.update_settings(|s| *s = apply)));
        self.sync(cx);
    }

    fn add_ignore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mask = normalize_mask(&text(&self.ignore, cx));
        if mask.is_empty() {
            return;
        }
        self.ignore.update(cx, |s, cx| s.set_value("", window, cx));
        self.change(window, cx, move |s| {
            if !s.ignores.contains(&mask) {
                s.ignores.push(mask);
            }
        });
    }

    fn setter<V: 'static>(this: &Entity<Self>, apply: impl Fn(&mut Settings, V) + Clone + 'static) -> impl Fn(V, &mut Window, &mut App) + 'static {
        let this = this.clone();
        move |v, w, cx| {
            let apply = apply.clone();
            this.update(cx, |s, cx| s.change(w, cx, move |st| apply(st, v)))
        }
    }

    fn describe(&self) -> String {
        match self.tab {
            SettingsTab::Servers if self.edit.is_some() => "Connection, identity and authentication for this network.".into(),
            SettingsTab::Servers => "Networks and bouncers Hearth connects to. A soju bouncer expands into all of its networks automatically.".into(),
            SettingsTab::General => "Startup behavior, presence and updates.".into(),
            SettingsTab::Appearance => "Theme, message layout and how the conversation is displayed.".into(),
            SettingsTab::Notifications => "Choose what gets your attention.".into(),
            SettingsTab::Aliases => {
                "Your own slash commands. /j → /join $1, /ns → /msg NickServ $*. They show up in command suggestions and Tab completion.".into()
            }
            SettingsTab::Privacy => "What Hearth stores on this computer and what it fetches on your behalf.".into(),
            SettingsTab::Shortcuts => "Keyboard shortcuts available throughout Hearth.".into(),
        }
    }

    fn title(&self) -> String {
        if self.tab == SettingsTab::Servers
            && let Some(e) = &self.edit
        {
            {
                return match &e.id {
                    None => "Add a server".into(),
                    Some(id) => format!("Edit {}", self.data.servers.iter().find(|r| &r.config.id == id).map_or(id.as_str(), |r| r.config.name.as_str())),
                };
            }
        }
        TABS.iter().find(|t| t.0 == self.tab).map_or("Settings", |t| t.1).to_string()
    }
}

fn sep(p: &Palette) -> Div {
    div().h(px(1.)).w_full().my(px(21.)).bg(p.border)
}

fn pill(status: Option<ConnState>, p: &Palette) -> Div {
    let (label, color) = match status {
        None => ("offline", p.muted_foreground.opacity(0.6)),
        Some(ConnState::Connected) => ("connected", p.ok),
        Some(ConnState::Connecting) => ("connecting", p.warn),
        Some(ConnState::Disconnected) => ("disconnected", p.destructive),
    };
    h_flex()
        .flex_none()
        .h(px(17.5))
        .px(px(7.))
        .gap(px(5.25))
        .items_center()
        .rounded_full()
        .border_1()
        .border_color(p.border)
        .text_size(px(10.5))
        .text_color(p.muted_foreground)
        .child(status_dot(color))
        .child(label)
}

impl SettingsView {
    fn servers_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = cx.hearth().clone();
        let this = cx.entity();
        if let Some(e) = &self.edit {
            return v_flex()
                .child(e.form.clone())
                .children(e.bouncer.clone().map(|b| v_flex().child(sep(&p)).child(b)))
                .children(e.highlights.clone().map(|h| v_flex().child(sep(&p)).child(h)))
                .into_any_element();
        }
        let t_add = this.clone();
        v_flex()
            .child(v_flex().gap(px(8.75)).children(self.data.servers.iter().enumerate().map(|(i, r)| {
                let c = &r.config;
                let addr = format!(
                    "{}{}:{}{}",
                    if c.websocket { if c.tls { "wss://" } else { "ws://" } } else { "" },
                    c.host,
                    c.port,
                    if c.websocket { c.websocket_path.as_str() } else { "" }
                );
                let online = matches!(r.status, Some(ConnState::Connected | ConnState::Connecting));
                let (t_edit, t_conn) = (this.clone(), this.clone());
                let (edit_id, conn_id) = (c.id.clone(), c.id.clone());
                h_flex()
                    .gap(px(10.5))
                    .px(px(10.5))
                    .py(px(10.5))
                    .items_center()
                    .rounded(px(12.75))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.card)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().text_size(px(12.25)).font_weight(FontWeight::MEDIUM).child(c.name.clone()))
                            .child(div().truncate().font_family(MONO).text_size(px(10.5)).text_color(p.muted_foreground).child(addr)),
                    )
                    .child(pill(r.status, &p))
                    .child(
                        Button::new(SharedString::from(format!("srv-conn-{i}"))).ghost().small().label(if online { "Disconnect" } else { "Connect" }).on_click(
                            move |_, _, cx| {
                                let server = conn_id.clone();
                                let effect = if online { Effect::Disconnect { server, reason: None } } else { Effect::Connect { server } };
                                hearth(cx).update(cx, |h, cx| h.emit(effect, cx));
                                let _ = &t_conn;
                            },
                        ),
                    )
                    .child(Button::new(SharedString::from(format!("srv-edit-{i}"))).outline().small().label("Edit").on_click(move |_, w, cx| {
                        let id = edit_id.clone();
                        t_edit.update(cx, |s, cx| s.goto(SettingsTab::Servers, Some(id), w, cx))
                    }))
            })))
            .child(
                h_flex().mt(px(14.)).child(
                    Button::new("srv-add")
                        .primary()
                        .icon(icon("plus", 14., p.brand_text))
                        .label("Add server")
                        .on_click(move |_, w, cx| t_add.update(cx, |s, cx| s.goto(SettingsTab::Servers, Some("new".into()), w, cx))),
                ),
            )
            .into_any_element()
    }

    fn general_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = cx.hearth().clone();
        let this = cx.entity();
        let s = &self.data.settings;
        let (phase, version, unavailable) = {
            let h = host(cx);
            (h.updates.phase.clone(), env!("CARGO_PKG_VERSION"), h.updates.unavailable_reason())
        };
        let busy = matches!(phase, UpdatePhase::Checking | UpdatePhase::Installing { .. });
        let status = match (&unavailable, &phase) {
            (Some(why), _) => why.clone(),
            (_, UpdatePhase::Checking) => "Checking for updates…".to_string(),
            (_, UpdatePhase::Current) => "You're up to date.".into(),
            (_, UpdatePhase::Available(v)) => format!("Version {v} is available."),
            (_, UpdatePhase::Installing { version, progress }) => {
                format!("Installing {version}{}", progress.map_or("…".to_string(), |p| format!(" · {}%", (p * 100.).round())))
            }
            (_, UpdatePhase::Error(e)) => format!("Couldn't update: {e}"),
            (_, UpdatePhase::Idle) => "Updates come from GitHub releases and are verified before installing.".into(),
        };
        let button = if matches!(phase, UpdatePhase::Available(_) | UpdatePhase::Installing { .. }) {
            Button::new("install")
                .primary()
                .disabled(busy)
                .label("Install and restart")
                .on_click(move |_, _, cx| hearth(cx).update(cx, |h, cx| h.install_update(cx)))
        } else {
            Button::new("check")
                .outline()
                .disabled(busy || unavailable.is_some())
                .label("Check now")
                .on_click(move |_, _, cx| hearth(cx).update(cx, |h, cx| h.check_for_update(false, cx)))
        };
        rows(
            vec![
                switch_row(
                    "close-tray",
                    "Keep running when the window is closed",
                    Some("Hearth stays connected in the menu bar or system tray. Quit from the tray icon."),
                    s.close_to_tray,
                    Self::setter(&this, |s, v| s.close_to_tray = v),
                    &p,
                ),
                setting_row(
                    "Mark me away",
                    Some("When Hearth has been in the background this long. You're marked back when you return."),
                    segmented(
                        "away",
                        &[(0u32, "Never"), (10, "10 min"), (30, "30 min"), (60, "1 hour")],
                        s.auto_away_minutes,
                        Self::setter(&this, |s, v| s.auto_away_minutes = v),
                        cx,
                    ),
                    &p,
                ),
                // No spellcheck toggle: GPUI has no spellchecker hook yet, so the composer
                // cannot underline words. `Settings::spellcheck` is kept for when it can.
                switch_row(
                    "updates",
                    "Check for updates automatically",
                    Some("Looks for a new release when Hearth starts and every few hours. Nothing is installed without asking."),
                    s.check_updates,
                    Self::setter(&this, |s, v| s.check_updates = v),
                    &p,
                ),
                h_flex()
                    .gap(px(16.))
                    .items_center()
                    .child(
                        v_flex().flex_1().min_w_0().gap(px(2.)).child(div().text_size(px(13.5)).child(format!("Hearth {version}"))).child(
                            div()
                                .text_size(px(12.))
                                .text_color(if matches!(phase, UpdatePhase::Error(_)) { p.destructive } else { p.muted_foreground })
                                .child(status),
                        ),
                    )
                    .child(div().flex_none().child(button))
                    .into_any_element(),
            ],
            &p,
        )
        .into_any_element()
    }

    fn privacy_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = cx.hearth().clone();
        let this = cx.entity();
        let s = self.data.settings.clone();
        let t_clear = this.clone();
        let clear = if self.confirm_clear {
            let t_cancel = this.clone();
            h_flex()
                .gap(px(8.))
                .child(Button::new("clear-yes").danger().label("Delete all?").on_click({
                    let t = this.clone();
                    move |_, _, cx| {
                        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.clear_logs()));
                        t.update(cx, |s, cx| {
                            s.confirm_clear = false;
                            cx.notify()
                        })
                    }
                }))
                .child(Button::new("clear-no").ghost().label("Cancel").on_click(move |_, _, cx| {
                    t_cancel.update(cx, |s, cx| {
                        s.confirm_clear = false;
                        cx.notify()
                    })
                }))
        } else {
            h_flex().child(Button::new("clear").danger().label("Delete saved messages").on_click(move |_, _, cx| {
                t_clear.update(cx, |s, cx| {
                    s.confirm_clear = true;
                    cx.notify()
                })
            }))
        };

        let ignores = s.ignores.clone();
        let n = ignores.len();
        let list = (n > 0).then(|| {
            v_flex().rounded(px(8.)).border_1().border_color(p.border).overflow_hidden().children(ignores.into_iter().enumerate().map(|(i, mask)| {
                let t = this.clone();
                let m = mask.clone();
                h_flex()
                    .items_center()
                    .justify_between()
                    .py(px(4.))
                    .pr(px(6.))
                    .pl(px(12.))
                    .text_size(px(12.5))
                    .when(i + 1 < n, |d| d.border_b_1().border_color(p.border))
                    .child(div().font_family(MONO).child(mask))
                    .child(super::widgets::icon_button(
                        SharedString::from(format!("unignore-{i}")),
                        "x",
                        (24., 13.),
                        p.muted_foreground,
                        p.muted,
                        Some("Stop ignoring"),
                        move |w, cx| {
                            let m = m.clone();
                            t.update(cx, |s, cx| s.change_ctx(w, cx, move |st| st.ignores.retain(|x| *x != m)))
                        },
                    ))
            }))
        });
        let t_ignore = this.clone();
        let ignore_field = v_flex()
            .gap(px(8.))
            .child(label("Ignored people"))
            .children(list)
            .child(
                h_flex().gap(px(8.)).child(div().flex_1().child(mono_input(&self.ignore))).child(
                    Button::new("ignore-add")
                        .outline()
                        .disabled(text(&self.ignore, cx).trim().is_empty())
                        .label("Ignore")
                        .on_click(move |_, w, cx| t_ignore.update(cx, |s, cx| s.add_ignore(w, cx))),
                ),
            )
            .child(rich_hint("Hides their messages, notices, invites and reactions everywhere. `*` and `?` are wildcards.", &p))
            .into_any_element();

        let mut r = vec![
            switch_row(
                "logs",
                "Save messages on this computer",
                Some("Keeps scrollback across restarts and makes messages searchable. Stored only on this device."),
                s.store_logs,
                Self::setter(&this, |s, v| s.store_logs = v),
                &p,
            ),
            setting_row("Saved messages", Some("Deleting them doesn't affect history kept by a server or bouncer."), clear, &p),
            switch_row(
                "previews",
                "Show link previews",
                Some(
                    "Hearth fetches linked pages and images itself, so those sites see your IP address. Links to local or private addresses are never fetched.",
                ),
                s.link_previews,
                Self::setter(&this, |s, v| s.link_previews = v),
                &p,
            ),
            switch_row(
                "avatars",
                "Show profile pictures",
                Some("People can publish an avatar on servers that support it. Hearth fetches the image itself, so its host sees your IP address."),
                s.load_avatars,
                Self::setter(&this, |s, v| s.load_avatars = v),
                &p,
            ),
        ];
        if s.link_previews {
            r.push(switch_row("inline", "Show inline images", Some("Image links appear under the message. Images in messages that mention NSFW stay blurred until clicked. Video and audio are only described."), s.inline_images, Self::setter(&this, |s, v| s.inline_images = v), &p));
            r.push(switch_row(
                "autoplay",
                "Play animations automatically",
                Some("Otherwise animated images play while the pointer is over them. Never when your system asks for reduced motion."),
                s.autoplay_animations,
                Self::setter(&this, |s, v| s.autoplay_animations = v),
                &p,
            ));
        }
        r.push(ignore_field);
        rows(r, &p).into_any_element()
    }

    /// `change` for callers that only have an `App` and a window.
    fn change_ctx(&mut self, window: &mut Window, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        self.change(window, cx, f)
    }

    fn appearance_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = cx.hearth().clone();
        let this = cx.entity();
        let s = &self.data.settings;
        v_flex()
            .child(heading("Theme", None, true, &p))
            .child(choice_cards(
                "theme",
                vec![
                    (Theme::System, "System", theme_preview("system")),
                    (Theme::Light, "Light", theme_preview("light")),
                    (Theme::Dark, "Dark", theme_preview("dark")),
                ],
                s.theme,
                Self::setter(&this, |s, v| s.theme = v),
                cx,
            ))
            .child(heading(
                "Message layout",
                Some("Modern groups messages under avatars; columns lines nicks up in a gutter; compact stacks them above messages."),
                false,
                &p,
            ))
            .child(choice_cards(
                "layout",
                vec![
                    (Layout::Cozy, "Modern", layout_preview("cozy", &p)),
                    (Layout::Columns, "Columns", layout_preview("columns", &p)),
                    (Layout::Compact, "Compact", layout_preview("compact", &p)),
                ],
                s.layout,
                Self::setter(&this, |s, v| s.layout = v),
                cx,
            ))
            .child(heading("Display", None, false, &p))
            .child(rows(
                vec![
                    setting_row(
                        "Text size",
                        None,
                        segmented("fs", &[(13u32, "S"), (14, "M"), (15, "L"), (16, "XL")], s.font_size, Self::setter(&this, |s, v| s.font_size = v), cx),
                        &p,
                    ),
                    setting_row(
                        "Clock",
                        None,
                        segmented("clock", &[(Clock::H24, "24-hour"), (Clock::H12, "12-hour")], s.clock, Self::setter(&this, |s, v| s.clock = v), cx),
                        &p,
                    ),
                    setting_row(
                        "Joins, parts and quits",
                        Some("Collapse folds consecutive events into one line."),
                        segmented(
                            "membership",
                            &[(Membership::Show, "Show"), (Membership::Collapse, "Collapse"), (Membership::Hide, "Hide")],
                            s.membership,
                            Self::setter(&this, |s, v| s.membership = v),
                            cx,
                        ),
                        &p,
                    ),
                    switch_row("seconds", "Show seconds in timestamps", None, s.show_seconds, Self::setter(&this, |s, v| s.show_seconds = v), &p),
                    switch_row("nickcolors", "Colored nicknames", None, s.nick_colors, Self::setter(&this, |s, v| s.nick_colors = v), &p),
                    switch_row("showtyping", "Show when others are typing", None, s.show_typing, Self::setter(&this, |s, v| s.show_typing = v), &p),
                    switch_row("sendtyping", "Let others see when I'm typing", None, s.send_typing, Self::setter(&this, |s, v| s.send_typing = v), &p),
                ],
                &p,
            ))
            .into_any_element()
    }

    fn notifications_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = cx.hearth().clone();
        let this = cx.entity();
        let s = &self.data.settings;
        rows(
            vec![
                switch_row(
                    "n-mentions",
                    "Mentions",
                    Some("When someone says your nickname or a highlight word."),
                    s.notify_highlights,
                    Self::setter(&this, |s, v| s.notify_highlights = v),
                    &p,
                ),
                switch_row("n-private", "Private messages", None, s.notify_private, Self::setter(&this, |s, v| s.notify_private = v), &p),
                v_flex()
                    .gap(px(6.))
                    .child(label("Highlight words"))
                    .child(Input::new(&self.words))
                    .child(hint("Comma-separated. Your nickname always counts. Add per-server words and exceptions on each server's settings page.", &p))
                    .into_any_element(),
            ],
            &p,
        )
        .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let this = cx.entity();
        let height = (window.viewport_size().height.as_f32() * 0.88).min(640.);

        let nav = v_flex()
            .flex_none()
            .w(px(208.))
            .h_full()
            .gap(px(1.75))
            .p(px(10.5))
            .border_r_1()
            .border_color(p.sidebar_border)
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .child(
                div().px(px(7.)).pt(px(5.25)).pb(px(10.5)).text_size(px(12.25)).line_height(relative(1.0)).font_weight(FontWeight::SEMIBOLD).child("Settings"),
            )
            .children(TABS.iter().map(|&(tab, name, glyph)| {
                let on = self.tab == tab;
                let t = this.clone();
                let (accent, fg) = (p.sidebar_accent, p.sidebar_foreground);
                h_flex()
                    .id(SharedString::from(format!("tab-{name}")))
                    .h(px(28.))
                    .px(px(7.))
                    .gap(px(8.75))
                    .items_center()
                    .rounded(px(8.75))
                    .text_size(px(13.5))
                    .cursor_pointer()
                    .when(on, |d| d.bg(accent).font_weight(FontWeight::MEDIUM).text_color(fg).shadow_xs())
                    .when(!on, |d| d.text_color(fg.opacity(0.65)).hover(move |s| s.bg(accent).text_color(fg)))
                    .on_click(move |_, w, cx| t.update(cx, |s, cx| s.goto(tab, None, w, cx)))
                    .child(icon(glyph, 15., if on { p.brand } else { fg.opacity(0.65) }))
                    .child(name)
            }))
            .child(div().flex_1())
            .child(div().px(px(7.)).text_size(px(10.5)).text_color(p.muted_foreground).child(format!("Hearth {}", env!("CARGO_PKG_VERSION"))));

        let body = match self.tab {
            SettingsTab::Servers => self.servers_tab(cx),
            SettingsTab::General => self.general_tab(cx),
            SettingsTab::Appearance => self.appearance_tab(cx),
            SettingsTab::Notifications => self.notifications_tab(cx),
            SettingsTab::Aliases => self.aliases.clone().into_any_element(),
            SettingsTab::Privacy => self.privacy_tab(cx),
            SettingsTab::Shortcuts => shortcut_list(&p).into_any_element(),
        };
        // A scroll area per section and server, so each starts at the top.
        let scroll_id = SharedString::from(format!("settings-body-{:?}-{}", self.tab, self.edit.as_ref().map_or("list", |e| e.id.as_deref().unwrap_or("new"))));

        h_flex().h(px(height)).w_full().overflow_hidden().child(nav).child(
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(
                    v_flex()
                        .flex_none()
                        .gap(px(3.5))
                        .px(px(28.))
                        .pt(px(24.5))
                        .pr(px(49.))
                        .pb(px(7.))
                        .child(div().truncate().text_size(px(17.5)).font_weight(FontWeight::SEMIBOLD).child(self.title()))
                        .child(div().text_size(px(13.)).text_color(p.muted_foreground).child(self.describe())),
                )
                .child(
                    div()
                        .id("settings-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .id(scroll_id)
                        .child(div().px(px(28.)).pt(px(14.)).pb(px(28.)).child(div().max_w(px(504.)).child(body))),
                ),
        )
    }
}
