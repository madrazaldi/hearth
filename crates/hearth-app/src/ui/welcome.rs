//! First-run onboarding: pick a network, a nickname, an account, channels,
//! then watch the connection come up. The form steps
//! are local; connecting hands the server to the model's `WelcomeFlow`, which
//! saves it, follows the connection and explains a failure.

use super::avatar::icon;
use super::controls::*;
use super::data::*;
use super::prelude::*;
use super::settings::server_form::{FormEvent, ServerForm};
use super::theme::{HearthTheme as _, MONO, Palette};
use crate::host::{hearth, host};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use hearth_model::onboarding::{NetworkPreset, PresetKind, nick_error, parse_channels, presets};
use hearth_model::types::{SaslMechanism, ServerConfig};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Network,
    Server,
    Nick,
    Account,
    Channels,
    Custom,
    Connect,
}

impl Step {
    fn label(self) -> &'static str {
        match self {
            Step::Network => "Network",
            Step::Server | Step::Custom => "Server",
            Step::Nick => "Nickname",
            Step::Account => "Account",
            Step::Channels => "Channels",
            Step::Connect => "Connect",
        }
    }
}

pub struct Welcome {
    step: Step,
    preset: Option<NetworkPreset>,
    cfg: ServerConfig,
    nick: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    user: Entity<InputState>,
    pass: Entity<InputState>,
    extra: Entity<InputState>,
    form: Option<Entity<ServerForm>>,
    nick_touched: bool,
    use_account: bool,
    account_error: String,
    server_error: String,
    picked: Vec<String>,
    details: bool,
    _subs: Vec<Subscription>,
}

impl Welcome {
    pub fn new(_: Dispatch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let nick = new_input(window, cx, "", "ember");
        let host = new_input(window, cx, "", "bouncer.example.org");
        let port = new_input(window, cx, "6697", "");
        let user = new_input(window, cx, "", "");
        let pass = new_password(window, cx, "", "");
        let extra = new_input(window, cx, "", "#rust, #linux");
        let mut subs: Vec<Subscription> = [&nick, &host, &port, &user, &pass, &extra]
            .map(|s| {
                cx.subscribe_in(s, window, |this: &mut Self, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    InputEvent::Blur => {
                        this.nick_touched = this.nick_touched || !text(&this.nick, cx).is_empty();
                        cx.notify()
                    }
                    InputEvent::Change => cx.notify(),
                    _ => {}
                })
            })
            .into();
        subs.push(cx.observe(&hearth(cx), |_, _, cx| cx.notify()));
        let mut w = Welcome {
            step: Step::Network,
            preset: None,
            cfg: ServerConfig::default(),
            nick,
            host,
            port,
            user,
            pass,
            extra,
            form: None,
            nick_touched: false,
            use_account: false,
            account_error: String::new(),
            server_error: String::new(),
            picked: Vec::new(),
            details: false,
            _subs: subs,
        };
        w.dev_preview(window, cx);
        w
    }

    /// `HEARTH_WELCOME=<step>` opens the flow part-way, for checking the screens.
    fn dev_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(want) = std::env::var("HEARTH_WELCOME") {
            self.preview(&want, window, cx);
        }
    }

    /// The label of the step showing, for tests.
    pub fn step_label(&self) -> &'static str {
        self.step.label()
    }

    /// Choose a network by its preset id, as clicking its card does.
    pub fn pick_network(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(p) = presets().into_iter().find(|p| p.id == id) {
            self.choose(p, window, cx);
        }
    }

    /// Open the flow at a step (`network`, `nick-error`, `account`, `channels`, `bouncer`, `custom`, `connect`).
    pub fn preview(&mut self, want: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = presets().into_iter().find(|p| {
            p.id == if want == "bouncer" {
                "soju"
            } else if want == "custom" {
                "custom"
            } else {
                "libera"
            }
        }) else {
            return;
        };
        if want == "network" {
            return;
        }
        self.nick.update(cx, |s, cx| s.set_value("ember", window, cx));
        // Choosing a network moves on from the first step, wherever the preview was before.
        self.step = Step::Network;
        self.choose(p, window, cx);
        match want {
            "nick-error" => {
                self.nick.update(cx, |s, cx| s.set_value("9lives", window, cx));
                self.nick_touched = true;
            }
            "account" => self.step = Step::Account,
            "channels" => self.step = Step::Channels,
            "bouncer" => self.step = Step::Server,
            "connect" => self.step = Step::Connect,
            _ => {}
        }
    }

    fn flow(&self) -> Vec<Step> {
        match &self.preset {
            None => vec![Step::Network],
            Some(p) => match p.kind {
                PresetKind::Custom => vec![Step::Network, Step::Custom, Step::Connect],
                PresetKind::Bouncer => vec![Step::Network, Step::Server, Step::Nick, Step::Account, Step::Connect],
                PresetKind::Network => {
                    let mut f = vec![Step::Network, Step::Nick];
                    if p.sasl == Some(true) {
                        f.push(Step::Account);
                    }
                    f.extend([Step::Channels, Step::Connect]);
                    f
                }
            },
        }
    }

    fn go(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        self.step = step;
        if step == Step::Custom {
            self.open_custom(window, cx);
        }
        let target = match step {
            Step::Nick => Some(self.nick.clone()),
            Step::Server => Some(self.host.clone()),
            Step::Account => Some(self.user.clone()),
            Step::Channels => Some(self.extra.clone()),
            _ => None,
        };
        if let Some(t) = target {
            window.defer(cx, move |w, cx| t.update(cx, |s, cx| s.focus(w, cx)));
        }
        cx.notify();
    }

    fn step_by(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let flow = self.flow();
        let i = flow.iter().position(|s| *s == self.step).unwrap_or(0) as i32 + delta;
        if let Some(s) = flow.get(i.max(0) as usize).copied() {
            self.go(s, window, cx);
        }
    }

    fn choose(&mut self, p: NetworkPreset, window: &mut Window, cx: &mut Context<Self>) {
        // The model drops a failed attempt it already saved, so it is not left behind.
        let id = p.id;
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.welcome_choose(id)));
        let cfg = ServerConfig {
            name: if p.kind == PresetKind::Network { p.label.to_string() } else { String::new() },
            host: p.host.to_string(),
            port: p.port,
            tls: p.tls,
            ..ServerConfig::default()
        };
        self.host.update(cx, |s, cx| s.set_value(p.host, window, cx));
        self.port.update(cx, |s, cx| s.set_value(p.port.to_string(), window, cx));
        self.cfg = cfg;
        self.use_account = p.kind == PresetKind::Bouncer;
        self.picked = p.channels.iter().map(|c| c.to_string()).collect();
        self.extra.update(cx, |s, cx| s.set_value("", window, cx));
        self.nick_touched = false;
        self.account_error.clear();
        self.server_error.clear();
        self.preset = Some(p);
        self.step_by(1, window, cx);
    }

    fn nick_problem(&self, cx: &App) -> Option<String> {
        let p = self.preset.as_ref();
        nick_error(text(&self.nick, cx).trim(), p.and_then(|p| p.nick_len).unwrap_or(30), p.map_or("this network", |p| p.label))
    }

    fn net_name(&self) -> String {
        match &self.preset {
            Some(p) if p.kind == PresetKind::Network => p.label.to_string(),
            _ if !self.cfg.name.is_empty() => self.cfg.name.clone(),
            _ => self.cfg.host.clone(),
        }
    }

    /// Continue to the next step: Enter in a field, or the card's button.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.step {
            Step::Server => {
                self.cfg.host = text(&self.host, cx).trim().to_string();
                self.cfg.port = text(&self.port, cx).trim().parse().unwrap_or(6697);
                self.server_error = if self.cfg.host.is_empty() { "Enter the address of your bouncer.".into() } else { String::new() };
                if self.server_error.is_empty() {
                    self.step_by(1, window, cx);
                } else {
                    cx.notify();
                }
            }
            Step::Nick => {
                self.nick_touched = true;
                if self.nick_problem(cx).is_none() {
                    self.step_by(1, window, cx);
                } else {
                    cx.notify();
                }
            }
            Step::Account => {
                self.account_error.clear();
                if self.use_account && text(&self.pass, cx).is_empty() {
                    self.account_error = "Enter your password, or turn off “I have an account”.".into();
                    return cx.notify();
                }
                if self.preset.as_ref().is_some_and(|p| p.kind == PresetKind::Network) {
                    self.step_by(1, window, cx);
                } else {
                    self.connect(None, window, cx);
                }
            }
            Step::Channels => self.connect(None, window, cx),
            _ => {}
        }
    }

    fn open_custom(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cfg = self.cfg.clone();
        let form = cx.new(|cx| ServerForm::new(cfg, "Connect", false, window, cx));
        self._subs.push(cx.subscribe_in(&form, window, |this: &mut Self, _, ev: &FormEvent, window, cx| match ev {
            FormEvent::Save(cfg) => this.connect(Some((**cfg).clone()), window, cx),
            FormEvent::Cancel => this.step_by(-1, window, cx),
            FormEvent::Delete(_) => {}
        }));
        self.form = Some(form);
    }

    fn connect(&mut self, from: Option<ServerConfig>, window: &mut Window, cx: &mut Context<Self>) {
        let cfg = match from {
            Some(c) => c,
            None => {
                let mut c = self.cfg.clone();
                c.nick = text(&self.nick, cx).trim().to_string();
                c.sasl = if self.use_account { SaslMechanism::Plain } else { SaslMechanism::None };
                c.sasl_username = text(&self.user, cx).trim().to_string();
                c.sasl_password = if self.use_account { text(&self.pass, cx) } else { String::new() };
                if self.use_account && c.sasl_username.is_empty() {
                    c.sasl_username = c.nick.clone();
                }
                if self.preset.as_ref().is_some_and(|p| p.kind == PresetKind::Network) {
                    c.autojoin = parse_channels(&format!("{} {}", self.picked.join(" "), text(&self.extra, cx)));
                }
                c
            }
        };
        self.cfg = cfg.clone();
        self.details = false;
        self.step = Step::Connect;
        let _ = window;
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.welcome_connect(Some(cfg))));
        cx.notify();
    }

    /// Back to the step that can fix the problem.
    fn edit_after_failure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = host(cx).app.welcome.failure.as_ref().map(|f| f.raw.to_lowercase()).unwrap_or_default();
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.welcome_edit_after_failure()));
        let flow = self.flow();
        let auth = raw.contains("sasl") || raw.contains("authenticat");
        let to = if auth && flow.contains(&Step::Account) {
            Step::Account
        } else if flow.contains(&Step::Server) {
            Step::Server
        } else {
            flow[flow.len().saturating_sub(2)]
        };
        self.go(to, window, cx);
    }
}

fn card(title: impl IntoElement, desc: Option<String>, content: impl IntoElement, footer: Option<AnyElement>, p: &Palette) -> Div {
    v_flex()
        .gap(px(21.))
        .py(px(21.))
        .rounded(px(12.75))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .shadow_sm()
        .child(
            v_flex()
                .gap(px(3.5))
                .px(px(21.))
                .child(div().text_size(px(14.)).line_height(relative(1.25)).font_weight(FontWeight::MEDIUM).child(title))
                .children(desc.map(|d| div().text_size(px(12.25)).line_height(relative(1.45)).text_color(p.muted_foreground).child(d))),
        )
        .child(div().px(px(21.)).child(content))
        .children(footer.map(|f| div().px(px(21.)).child(f)))
}

fn optional_badge(p: &Palette) -> Div {
    div()
        .h(px(17.5))
        .px(px(7.))
        .flex()
        .items_center()
        .rounded_full()
        .border_1()
        .border_color(p.border)
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.muted_foreground)
        .child("optional")
}

fn alert(kind_color: Hsla, glyph: &str, title: Option<String>, body: impl IntoElement, p: &Palette) -> Div {
    h_flex()
        .gap(px(7.))
        .items_start()
        .px(px(8.75))
        .py(px(7.))
        .rounded(px(8.75))
        .border_1()
        .border_color(if kind_color == p.destructive { kind_color.opacity(0.4) } else { p.border })
        .child(div().mt(px(1.75)).child(icon(glyph, 14., kind_color)))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.75))
                .children(title.map(|t| div().text_size(px(12.25)).font_weight(FontWeight::MEDIUM).text_color(kind_color).child(t)))
                .child(div().text_size(px(12.25)).text_color(if kind_color == p.destructive { kind_color.opacity(0.9) } else { kind_color }).child(body)),
        )
}

fn rule_with_label(label: &str, p: &Palette) -> Div {
    h_flex()
        .gap(px(7.))
        .items_center()
        .child(div().h(px(1.)).flex_1().bg(p.border))
        .child(div().text_size(px(12.25)).text_color(p.muted_foreground).child(label.to_string()))
        .child(div().h(px(1.)).flex_1().bg(p.border))
}

impl Render for Welcome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let this = cx.entity();
        let flow = self.flow();
        let index = flow.iter().position(|s| *s == self.step).unwrap_or(0);

        let actions = |label: &'static str, this: &Entity<Self>| {
            let (t_back, t_next) = (this.clone(), this.clone());
            h_flex()
                .justify_between()
                .child(Button::new("w-back").ghost().label("Back").on_click(move |_, w, cx| t_back.update(cx, |s, cx| s.step_by(-1, w, cx))))
                .child(Button::new("w-next").primary().label(label).on_click(move |_, w, cx| t_next.update(cx, |s, cx| s.submit(w, cx))))
                .into_any_element()
        };

        let body: AnyElement = match self.step {
            Step::Network => {
                let all = presets();
                let item = |pr: &NetworkPreset, net: bool| {
                    let selected = self.preset.as_ref().is_some_and(|x| x.id == pr.id);
                    let t = this.clone();
                    let pc = pr.clone();
                    let (hover, border) = (p.muted, p.input);
                    h_flex()
                        .id(SharedString::from(format!("preset-{}", pr.id)))
                        .gap(px(8.75))
                        .px(px(10.5))
                        .py(px(8.75))
                        .when(net, |d| d.items_center())
                        .when(!net, |d| d.items_start())
                        .rounded(px(8.75))
                        .border_1()
                        .border_color(if selected { p.primary } else { p.border })
                        .when(selected, |d| d.bg(p.muted.opacity(0.6)))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover).border_color(border))
                        .on_click(move |_, w, cx| {
                            let pc = pc.clone();
                            t.update(cx, |s, cx| s.choose(pc, w, cx))
                        })
                        .when(!net, |d| {
                            d.child(div().mt(px(1.)).child(icon(if pr.kind == PresetKind::Bouncer { "layers" } else { "settings" }, 14., p.muted_foreground)))
                        })
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(1.75))
                                .child(div().text_size(px(12.25)).font_weight(FontWeight::MEDIUM).child(pr.label))
                                .child(div().text_size(px(12.25)).text_color(p.muted_foreground).child(pr.blurb)),
                        )
                        .when(net, |d| {
                            d.child(div().font_family(MONO).text_size(px(11.)).text_color(p.muted_foreground).child(pr.host)).child(icon(
                                if selected { "check" } else { "chevron-right" },
                                14.,
                                p.muted_foreground,
                            ))
                        })
                };
                let nets = v_flex().gap(px(7.)).children(all.iter().filter(|x| x.kind == PresetKind::Network).map(|x| item(x, true)));
                let others = h_flex()
                    .gap(px(7.))
                    .items_stretch()
                    .children(all.iter().filter(|x| x.kind != PresetKind::Network).map(|x| div().flex_1().min_w_0().flex().child(item(x, false).w_full())));
                card(
                    "Choose a network",
                    Some("Pick a public network to begin. You can add more, or a bouncer, any time.".into()),
                    v_flex().gap(px(17.5)).child(nets).child(rule_with_label("Or connect to your own", &p)).child(others),
                    None,
                    &p,
                )
                .into_any_element()
            }
            Step::Server => {
                let t = this.clone();
                card(
                    "Where is your bouncer?",
                    Some("The address Hearth should connect to.".into()),
                    v_flex()
                        .gap(px(14.))
                        .child(
                            h_flex()
                                .gap(px(10.5))
                                .child(field("Host", mono_input(&self.host)).flex_1())
                                .child(field("Port", mono_input(&self.port)).w(px(84.))),
                        )
                        .child(switch_row(
                            "w-tls",
                            "Use TLS",
                            Some("Almost every bouncer supports it; the usual port is 6697."),
                            self.cfg.tls,
                            move |v, _, cx| {
                                t.update(cx, |s, cx| {
                                    s.cfg.tls = v;
                                    cx.notify()
                                })
                            },
                            &p,
                        ))
                        .when(!self.server_error.is_empty(), |d| d.child(error_text(self.server_error.clone(), &p))),
                    Some(actions("Continue", &this)),
                    &p,
                )
                .into_any_element()
            }
            Step::Nick => {
                let problem = self.nick_problem(cx).filter(|_| self.nick_touched);
                let len = self.preset.as_ref().and_then(|p| p.nick_len).unwrap_or(30);
                let pr = self.preset.as_ref();
                card(
                    "Choose a nickname",
                    Some(format!(
                        "How others will see you on {}.",
                        match pr {
                            Some(x) if x.kind == PresetKind::Network => x.label,
                            _ => "the server",
                        }
                    )),
                    v_flex()
                        .gap(px(14.))
                        .child(field("Nickname", Input::new(&self.nick)).child(match problem {
                            Some(e) => error_text(e, &p),
                            None => rich_hint(&format!("Letters, digits and `- _ [ ] {{ }} \\ | ^ \\``, up to {len} characters."), &p),
                        }))
                        .children(
                            pr.filter(|x| x.kind == PresetKind::Network && x.sasl == Some(false) && x.account_note.is_some())
                                .map(|x| alert(p.muted_foreground, "info", None, x.account_note.unwrap_or_default(), &p)),
                        ),
                    Some(actions("Continue", &this)),
                    &p,
                )
                .into_any_element()
            }
            Step::Account => {
                let bouncer = self.preset.as_ref().is_some_and(|x| x.kind == PresetKind::Bouncer);
                let t = this.clone();
                let title = h_flex()
                    .gap(px(7.))
                    .items_center()
                    .child(if bouncer { "Log in to your bouncer" } else { "Log in to an account" })
                    .when(!bouncer, |d| d.child(optional_badge(&p)));
                card(
                    title,
                    self.preset.as_ref().and_then(|x| x.account_note).map(str::to_string),
                    v_flex()
                        .gap(px(14.))
                        .when(!bouncer, |d| {
                            d.child(switch_row(
                                "w-account",
                                "I have an account",
                                Some("Skip this if you haven't registered your nickname."),
                                self.use_account,
                                move |v, _, cx| {
                                    t.update(cx, |s, cx| {
                                        s.use_account = v;
                                        cx.notify()
                                    })
                                },
                                &p,
                            ))
                        })
                        .when(self.use_account, |d| {
                            d.child(
                                h_flex()
                                    .gap(px(10.5))
                                    .child(field(if bouncer { "Username" } else { "Account name" }, Input::new(&self.user)).flex_1())
                                    .child(field("Password", Input::new(&self.pass).mask_toggle()).flex_1()),
                            )
                            .child(
                                h_flex()
                                    .gap(px(7.))
                                    .items_start()
                                    .text_size(px(10.5))
                                    .text_color(p.muted_foreground)
                                    .child(div().mt(px(1.)).child(icon("lock", 14., p.muted_foreground)))
                                    .child("Kept in your system keychain, never in Hearth's config file. Sent with SCRAM-SHA-256 when the server supports it."),
                            )
                        })
                        .when(!self.use_account && !bouncer, |d| {
                            d.child(rich_hint("No account yet? Connect first, then register with `/msg NickServ REGISTER password email`.", &p))
                        })
                        .when(!self.account_error.is_empty(), |d| d.child(error_text(self.account_error.clone(), &p))),
                    Some(actions(if bouncer { "Connect" } else { "Continue" }, &this)),
                    &p,
                )
                .into_any_element()
            }
            Step::Channels => {
                let net = self.net_name();
                let suggested: Vec<String> = self.preset.as_ref().map(|x| x.channels.iter().map(|c| c.to_string()).collect()).unwrap_or_default();
                card(
                    h_flex().gap(px(7.)).items_center().child("Join some channels").child(optional_badge(&p)),
                    Some("Pick a few to open as soon as you connect.".into()),
                    v_flex()
                        .gap(px(14.))
                        .when(!suggested.is_empty(), |d| {
                            d.child(
                                v_flex()
                                    .gap(px(3.5))
                                    .child(div().text_size(px(12.25)).text_color(p.muted_foreground).child(format!("Suggested on {net}")))
                                    .children(suggested.iter().map(|c| {
                                        let (t, name) = (this.clone(), c.clone());
                                        switch_row(
                                            &format!("w-ch-{c}"),
                                            c,
                                            Some("The network's official help and community channel."),
                                            self.picked.contains(c),
                                            move |v, _, cx| {
                                                let name = name.clone();
                                                t.update(cx, |s, cx| {
                                                    s.picked.retain(|x| *x != name);
                                                    if v {
                                                        s.picked.push(name);
                                                    }
                                                    cx.notify()
                                                })
                                            },
                                            &p,
                                        )
                                    })),
                            )
                        })
                        .child(
                            field("Other channels", mono_input(&self.extra))
                                .child(rich_hint("Separate with spaces or commas. You can join more later with `/join` or the channel browser.", &p)),
                        ),
                    Some(actions("Connect", &this)),
                    &p,
                )
                .into_any_element()
            }
            Step::Custom => card(
                "Custom server",
                Some("Any other server, with every option.".into()),
                self.form.clone().map(IntoElement::into_any_element).unwrap_or_else(|| div().into_any_element()),
                None,
                &p,
            )
            .into_any_element(),
            Step::Connect => self.connect_card(&this, &p, cx),
        };

        v_flex()
            .size_full()
            .items_center()
            .bg(p.background)
            .child(
                div().id("welcome-scroll").size_full().overflow_y_scroll().child(
                    v_flex().w_full().items_center().px(px(21.)).pt(px(49.)).pb(px(42.)).child(
                        v_flex()
                            .w_full()
                            .max_w(px(504.))
                            .child(
                                v_flex()
                                    .items_center()
                                    .mb(px(21.))
                                    .child(img("logo.png").size(px(48.)).rounded(px(12.75)).mb(px(14.)))
                                    .child(div().text_size(px(26.25)).line_height(relative(1.15)).font_weight(FontWeight::SEMIBOLD).child("Hearth"))
                                    .child(div().mt(px(5.25)).text_size(px(15.)).text_color(p.muted_foreground).child("A quiet place for IRC.")),
                            )
                            .when(flow.len() > 1, |d| {
                                d.child(h_flex().flex_wrap().justify_center().gap_x(px(14.)).gap_y(px(5.25)).mb(px(14.)).text_size(px(12.5)).children(
                                    flow.iter().enumerate().map(|(i, f)| {
                                        let (cur, done) = (i == index, i < index);
                                        h_flex()
                                            .gap(px(5.25))
                                            .items_center()
                                            .text_color(if cur {
                                                p.foreground
                                            } else if done {
                                                p.foreground.opacity(0.7)
                                            } else {
                                                p.muted_foreground
                                            })
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .size(px(18.))
                                                    .rounded_full()
                                                    .text_size(px(10.5))
                                                    .when(cur, |d| d.bg(p.primary).text_color(p.primary_foreground))
                                                    .when(done, |d| d.bg(p.brand_soft).text_color(p.brand))
                                                    .when(!cur && !done, |d| d.border_1().border_color(p.border))
                                                    .child(if done {
                                                        icon("check", 11., p.brand).into_any_element()
                                                    } else {
                                                        (i + 1).to_string().into_any_element()
                                                    }),
                                            )
                                            .child(f.label())
                                    }),
                                ))
                            })
                            .child(body),
                    ),
                ),
            )
            .child(div().absolute().size_0().child({
                let _ = window;
                div()
            }))
    }
}

impl Welcome {
    fn connect_card(&mut self, this: &Entity<Self>, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let h = host(cx);
        let net = self.net_name();
        let friendly = h.app.welcome_friendly();
        let raw = h.app.welcome.failure.as_ref().map(|f| f.raw.clone());
        let (stages, at) = h.app.welcome_stages();
        let connected = h.app.welcome_status() == "connected" && raw.is_none();
        let nick_note = h.app.welcome_nick_note();
        let sasl_failed = h.app.welcome_sasl_failed();
        if let (Some(raw), Some(f)) = (raw, friendly) {
            let (t_back, t_retry, t_details) = (this.clone(), this.clone(), this.clone());
            let open = self.details;
            let body = v_flex().gap(px(7.)).child(f.hint).child(
                v_flex()
                    .gap(px(5.25))
                    .child(
                        h_flex()
                            .id("failure-details")
                            .gap(px(3.5))
                            .items_center()
                            .cursor_pointer()
                            .text_size(px(10.5))
                            .text_color(p.muted_foreground)
                            .on_click(move |_, _, cx| {
                                t_details.update(cx, |s, cx| {
                                    s.details = !s.details;
                                    cx.notify()
                                })
                            })
                            .child(icon(if open { "chevron-down" } else { "chevron-right" }, 12., p.muted_foreground))
                            .child("Details"),
                    )
                    .when(open, |d| d.child(div().font_family(MONO).text_size(px(11.5)).text_color(p.muted_foreground).child(raw))),
            );
            return card(
                "Couldn't connect",
                None,
                alert(p.destructive, "circle-alert", Some(f.title), body, p),
                Some(
                    h_flex()
                        .justify_between()
                        .child(
                            Button::new("w-fail-back").ghost().label("Back").on_click(move |_, w, cx| t_back.update(cx, |s, cx| s.edit_after_failure(w, cx))),
                        )
                        .child(
                            Button::new("w-retry")
                                .primary()
                                .label("Try again")
                                .on_click(move |_, w, cx| t_retry.update(cx, |s, cx| s.connect(Some(s.cfg.clone()), w, cx))),
                        )
                        .into_any_element(),
                ),
                p,
            )
            .into_any_element();
        }

        let list = v_flex().gap(px(10.5)).text_size(px(13.5)).children(stages.into_iter().enumerate().map(|(i, stage)| {
            let done = i < at || connected;
            let now = i == at && !connected;
            h_flex()
                .gap(px(8.75))
                .items_center()
                .text_color(if now {
                    p.foreground
                } else if done {
                    p.foreground.opacity(0.7)
                } else {
                    p.muted_foreground
                })
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(17.5))
                        .rounded_full()
                        .when(done, |d| d.bg(p.brand_soft).child(icon("check", 12., p.brand)))
                        .when(!done, |d| d.border_1().border_color(p.border))
                        .when(now, |d| d.child(div().size(px(12.25)).child(Spinner::new().color(p.brand)))),
                )
                .child(stage.label)
        }));
        let mut content = v_flex().gap(px(14.)).child(list);
        let mut footer = None;
        if connected {
            if let Some(note) = nick_note {
                content = content.child(alert(p.warn, "triangle-alert", None, note, p));
            }
            if sasl_failed {
                let note = "Your account login failed, so you're connected without it. Check the password in Settings.";
                content = content.child(alert(p.warn, "triangle-alert", None, note, p));
            }
            if !self.cfg.autojoin.is_empty() && self.preset.as_ref().is_some_and(|x| x.kind == PresetKind::Network) {
                content = content.child(div().text_size(px(14.)).text_color(p.muted_foreground).child(format!("Joining {}.", self.cfg.autojoin.join(", "))));
            }
            footer = Some(
                h_flex()
                    .justify_end()
                    .child(
                        Button::new("w-done")
                            .primary()
                            .label("Start chatting")
                            .on_click(move |_, _, cx| hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.welcome_done()))),
                    )
                    .into_any_element(),
            );
        }
        let title = if connected { "You're in".to_string() } else { format!("Connecting to {net}") };
        card(title, None, content, footer, p).into_any_element()
    }
}
