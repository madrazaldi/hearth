//! Add or edit a server: connection, identity, authentication (including the
//! client certificate), channels, and the advanced options. Also the card on the welcome screen's custom step.

use crate::host::hearth;
use crate::ui::avatar::icon;
use crate::ui::controls::*;
use crate::ui::data::*;
use crate::ui::prelude::*;
use crate::ui::theme::{HearthTheme as _, MONO};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use hearth_model::onboarding::parse_channels;
use hearth_model::types::{SaslMechanism, ServerConfig};
use hearth_model::{Reply, ReqId};

#[derive(Clone, Copy, PartialEq)]
enum Auth {
    None,
    Password,
    Certificate,
}

pub enum FormEvent {
    Cancel,
    Delete(String),
    Save(Box<ServerConfig>),
}

impl EventEmitter<FormEvent> for ServerForm {}

pub struct ServerForm {
    cfg: ServerConfig,
    submit_label: String,
    removable: bool,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    ws_path: Entity<InputState>,
    nick: Entity<InputState>,
    user: Entity<InputState>,
    realname: Entity<InputState>,
    sasl_user: Entity<InputState>,
    sasl_pass: Entity<InputState>,
    cert: Entity<InputState>,
    autojoin: Entity<InputState>,
    srv_pass: Entity<InputState>,
    perform: Entity<TextareaState>,
    advanced: bool,
    confirm_delete: bool,
    saving: bool,
    error: String,
    cert_info: Option<CertView>,
    cert_error: String,
    /// A certificate lookup or generation in flight; true when it generates one.
    cert_req: Option<(ReqId, bool)>,
    _subs: Vec<Subscription>,
}

impl ServerForm {
    pub fn new(cfg: ServerConfig, submit_label: &str, removable: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let nick_hint = if cfg.nick.is_empty() { "optional".to_string() } else { cfg.nick.clone() };
        let name = new_input(window, cx, &cfg.name, "Libera.Chat");
        let host = new_input(window, cx, &cfg.host, "irc.libera.chat");
        let port = new_input(window, cx, &cfg.port.to_string(), "");
        let ws_path = new_input(window, cx, &cfg.websocket_path, "/socket");
        let nick = new_input(window, cx, &cfg.nick, "");
        let user = new_input(window, cx, &cfg.username, &nick_hint);
        let realname = new_input(window, cx, &cfg.realname, &nick_hint);
        let sasl_user = new_input(window, cx, &cfg.sasl_username, &cfg.nick);
        let sasl_pass = new_password(window, cx, &cfg.sasl_password, "");
        let cert = new_input(window, cx, &cfg.client_cert, "/path/to/cert.pem (certificate and key)");
        let autojoin = new_input(window, cx, &cfg.autojoin.join(", "), "#hearth, #rust");
        let srv_pass = new_password(window, cx, &cfg.password, "Usually empty");
        let perform = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).rows(3).placeholder("MODE $nick +x\nPRIVMSG NickServ :identify …");
            s.set_value(cfg.perform.join("\n"), window, cx);
            s
        });
        let subs = vec![
            cx.subscribe_in(&cert, window, |this: &mut Self, _, ev: &InputEvent, window, cx| {
                if matches!(ev, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.request_cert_info(window, cx);
                }
            }),
            cx.observe_in(&hearth(cx), window, |this: &mut Self, _, window, cx| this.poll_cert(window, cx)),
        ];
        let mut form = ServerForm {
            cfg,
            submit_label: submit_label.to_string(),
            removable,
            name,
            host,
            port,
            ws_path,
            nick,
            user,
            realname,
            sasl_user,
            sasl_pass,
            cert,
            autojoin,
            srv_pass,
            perform,
            advanced: false,
            confirm_delete: false,
            saving: false,
            error: String::new(),
            cert_info: None,
            cert_error: String::new(),
            cert_req: None,
            _subs: subs,
        };
        form.request_cert_info(window, cx);
        form
    }

    pub fn set_error(&mut self, e: impl Into<String>, cx: &mut Context<Self>) {
        self.error = e.into();
        self.saving = false;
        cx.notify();
    }

    /// Pick up the engine's answer to a certificate request; a generated one also fills the path.
    fn poll_cert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((req, generated)) = self.cert_req else { return };
        let Some(result) = hearth(cx).update(cx, |h, _| h.app.take_result(req)) else { return };
        self.cert_req = None;
        let result = match result {
            Ok(Reply::Cert(info)) => Ok(info),
            Ok(_) => Err("unexpected answer".to_string()),
            Err(e) => Err(e),
        };
        self.set_cert(result, generated, window, cx);
    }

    fn set_cert(&mut self, result: Result<CertView, String>, generated: bool, window: &mut Window, cx: &mut Context<Self>) {
        match result {
            Ok(info) => {
                if generated {
                    let path = info.path.clone();
                    self.cert.update(cx, |s, cx| s.set_value(path, window, cx));
                }
                self.cert_info = Some(info);
                self.cert_error.clear();
            }
            Err(e) => {
                self.cert_info = None;
                self.cert_error = e;
            }
        }
        cx.notify();
    }

    fn request_cert_info(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = text(&self.cert, cx).trim().to_string();
        self.cert_info = None;
        self.cert_error.clear();
        if !path.is_empty() {
            let req = hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.request_cert_info(&path)));
            self.cert_req = Some((req, false));
            self.poll_cert(window, cx);
        }
    }

    fn set_transport(&mut self, ws: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.websocket = ws;
        let port: u16 = text(&self.port, cx).trim().parse().unwrap_or(self.cfg.port);
        let tls = self.cfg.tls;
        let next = if ws && (port == 6697 || port == 6667) {
            Some(if tls { 443 } else { 80 })
        } else if !ws && (port == 443 || port == 80) {
            Some(if tls { 6697 } else { 6667 })
        } else {
            None
        };
        if let Some(p) = next {
            self.port.update(cx, |s, cx| s.set_value(p.to_string(), window, cx));
        }
        cx.notify();
    }

    fn set_tls(&mut self, tls: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.cfg.tls = tls;
        let port: u16 = text(&self.port, cx).trim().parse().unwrap_or(self.cfg.port);
        let next = if !self.cfg.websocket && port == if tls { 6667 } else { 6697 } {
            Some(if tls { 6697 } else { 6667 })
        } else if self.cfg.websocket && port == if tls { 80 } else { 443 } {
            Some(if tls { 443 } else { 80 })
        } else {
            None
        };
        if let Some(p) = next {
            self.port.update(cx, |s, cx| s.set_value(p.to_string(), window, cx));
        }
        cx.notify();
    }

    fn generate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = [&self.cfg.id, &self.cfg.name].into_iter().find(|s| !s.is_empty()).cloned().unwrap_or_else(|| "hearth-client".into());
        let req = hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.request_generate_cert(&id)));
        self.cert_req = Some((req, true));
        self.poll_cert(window, cx);
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        self.error.clear();
        let mut c = self.cfg.clone();
        c.name = text(&self.name, cx).trim().to_string();
        c.host = text(&self.host, cx).trim().to_string();
        c.nick = text(&self.nick, cx).trim().to_string();
        c.username = text(&self.user, cx).trim().to_string();
        c.realname = text(&self.realname, cx).trim().to_string();
        c.websocket_path = text(&self.ws_path, cx).trim().to_string();
        c.sasl_username = text(&self.sasl_user, cx).trim().to_string();
        c.sasl_password = text(&self.sasl_pass, cx);
        c.client_cert = text(&self.cert, cx).trim().to_string();
        c.password = text(&self.srv_pass, cx);
        if c.host.is_empty() {
            return self.set_error("A host is required.", cx);
        }
        if c.nick.is_empty() {
            return self.set_error("Choose a nickname.", cx);
        }
        let Ok(port) = text(&self.port, cx).trim().parse::<u16>() else { return self.set_error("The port is a number from 1 to 65535.", cx) };
        c.port = port;
        c.autojoin = parse_channels(&text(&self.autojoin, cx));
        c.perform = self.perform.read(cx).value().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        if c.sasl == SaslMechanism::Plain && c.sasl_username.is_empty() {
            c.sasl_username = c.nick.clone();
        }
        if c.sasl == SaslMechanism::External && c.client_cert.is_empty() {
            return self.set_error("Choose or generate a client certificate.", cx);
        }
        if c.sasl == SaslMechanism::External && !c.tls {
            return self.set_error("Certificate authentication needs TLS.", cx);
        }
        self.saving = true;
        cx.emit(FormEvent::Save(Box::new(c)));
        cx.notify();
    }
}

fn sep(p: &crate::ui::theme::Palette) -> Div {
    div().h(px(1.)).w_full().bg(p.border)
}

impl Render for ServerForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let this = cx.entity();
        let ws = self.cfg.websocket;

        let transport = {
            let t = this.clone();
            segmented("transport", &[(false, "IRC"), (true, "WebSocket")], ws, move |v, w, cx| t.update(cx, |f, cx| f.set_transport(v, w, cx)), cx)
        };
        let tls = {
            let t = this.clone();
            h_flex().w(px(150.)).pb(px(2.)).child(switch_inline(
                "form-tls",
                "Use TLS",
                None,
                self.cfg.tls,
                move |v, w, cx| t.update(cx, |f, cx| f.set_tls(v, w, cx)),
                &p,
            ))
        };

        let auth = {
            let t = this.clone();
            let now = match self.cfg.sasl {
                SaslMechanism::None => Auth::None,
                SaslMechanism::Plain => Auth::Password,
                SaslMechanism::External => Auth::Certificate,
            };
            segmented(
                "sasl",
                &[(Auth::None, "None"), (Auth::Password, "Password"), (Auth::Certificate, "Certificate")],
                now,
                move |v, _, cx| {
                    t.update(cx, |f, cx| {
                        f.cfg.sasl = match v {
                            Auth::None => SaslMechanism::None,
                            Auth::Password => SaslMechanism::Plain,
                            Auth::Certificate => SaslMechanism::External,
                        };
                        cx.notify()
                    })
                },
                cx,
            )
        };
        let auth_fields = match self.cfg.sasl {
            SaslMechanism::None => None,
            SaslMechanism::Plain => Some(
                v_flex()
                    .gap(px(8.))
                    .child(
                        h_flex()
                            .gap(px(12.))
                            .child(field("Account", Input::new(&self.sasl_user)).flex_1())
                            .child(field("Password", Input::new(&self.sasl_pass).mask_toggle()).flex_1()),
                    )
                    .child(hint("Uses SCRAM-SHA-256 when the server offers it, so the password never crosses the wire. Stored in your system keychain.", &p))
                    .into_any_element(),
            ),
            SaslMechanism::External => {
                let t = this.clone();
                Some(
                    v_flex()
                        .gap(px(6.))
                        .child(label("Client certificate"))
                        .child(
                            h_flex()
                                .gap(px(8.))
                                .child(div().flex_1().child(mono_input(&self.cert)))
                                .child(Button::new("gen-cert").outline().label("Generate").on_click(move |_, w, cx| t.update(cx, |f, cx| f.generate(w, cx)))),
                        )
                        .children(self.cert_info.clone().map(|c| {
                            v_flex()
                                .gap(px(4.))
                                .child(h_flex().flex_wrap().gap(px(4.)).text_size(px(12.)).text_color(p.muted_foreground).child("SHA-512").child(div().font_family(MONO).text_size(px(11.5)).text_color(p.foreground).child(c.sha512)))
                                .child(rich_hint("Register it with the network's services while connected — on Libera.Chat and most Atheme networks: `/msg NickServ CERT ADD`", &p))
                        }))
                        .when(!self.cert_error.is_empty(), |d| d.child(error_text(self.cert_error.clone(), &p)))
                        .into_any_element(),
                )
            }
        };

        let advanced = {
            let t = this.clone();
            let open = self.advanced;
            v_flex()
                .gap(px(16.))
                .child(
                    h_flex()
                        .id("advanced")
                        .gap(px(4.))
                        .h(px(28.))
                        .px(px(8.))
                        .items_center()
                        .self_start()
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .text_color(p.muted_foreground)
                        .cursor_pointer()
                        .hover({
                            let bg = p.muted;
                            move |s| s.bg(bg)
                        })
                        .on_click(move |_, _, cx| {
                            t.update(cx, |f, cx| {
                                f.advanced = !f.advanced;
                                cx.notify()
                            })
                        })
                        .child(icon(if open { "chevron-down" } else { "chevron-right" }, 14., p.muted_foreground))
                        .child("Advanced"),
                )
                .when(open, |d| {
                    let (t1, t2) = (this.clone(), this.clone());
                    d.child(
                        v_flex()
                            .gap(px(16.))
                            .pl(px(16.))
                            .border_l_1()
                            .border_color(p.border)
                            .child(field("Server password", Input::new(&self.srv_pass).mask_toggle()))
                            .child(
                                v_flex()
                                    .gap(px(6.))
                                    .child(label("Commands after connecting"))
                                    .child(Textarea::new(&self.perform).font_family(MONO))
                                    .child(rich_hint("One raw IRC line each. `$nick` expands to your nickname.", &p)),
                            )
                            .child(switch_inline(
                                "form-autoconnect",
                                "Connect on launch",
                                None,
                                self.cfg.autoconnect,
                                move |v, _, cx| {
                                    t1.update(cx, |f, cx| {
                                        f.cfg.autoconnect = v;
                                        cx.notify()
                                    })
                                },
                                &p,
                            ))
                            .child(switch_inline(
                                "form-invalid",
                                "Accept invalid certificates",
                                Some("Only for servers you trust with self-signed certificates."),
                                self.cfg.accept_invalid_certs,
                                move |v, _, cx| {
                                    t2.update(cx, |f, cx| {
                                        f.cfg.accept_invalid_certs = v;
                                        cx.notify()
                                    })
                                },
                                &p,
                            )),
                    )
                })
        };

        let delete = self.removable.then(|| {
            let t = this.clone();
            let id = self.cfg.id.clone();
            if self.confirm_delete {
                Button::new("really-remove").danger().label("Really remove?").on_click(move |_, _, cx| {
                    let id = id.clone();
                    t.update(cx, |_, cx| cx.emit(FormEvent::Delete(id)))
                })
            } else {
                Button::new("remove").danger().outline().label("Remove server").on_click(move |_, _, cx| {
                    t.update(cx, |f, cx| {
                        f.confirm_delete = true;
                        cx.notify()
                    })
                })
            }
        });

        let (t_cancel, t_save) = (this.clone(), this.clone());
        v_flex()
            .gap(px(21.))
            .child(
                v_flex()
                    .gap(px(14.))
                    .child(field("Name", Input::new(&self.name)))
                    .child(h_flex().gap(px(10.5)).child(field("Host", mono_input(&self.host)).flex_1()).child(field("Port", mono_input(&self.port)).w(px(96.))))
                    .child(h_flex().items_end().gap(px(21.)).child(v_flex().gap(px(7.)).child(label("Transport")).child(transport)).child(tls))
                    .when(ws, |d| d.child(field("WebSocket path", mono_input(&self.ws_path)))),
            )
            .child(sep(&p))
            .child(
                v_flex()
                    .gap(px(14.))
                    .child(
                        h_flex()
                            .gap(px(10.5))
                            .child(field("Nickname", Input::new(&self.nick)).flex_1())
                            .child(field("Username", Input::new(&self.user)).flex_1())
                            .child(field("Real name", Input::new(&self.realname)).flex_1()),
                    )
                    .child(v_flex().gap(px(7.)).child(label("Authentication")).child(auth))
                    .children(auth_fields),
            )
            .child(sep(&p))
            .child(
                v_flex()
                    .gap(px(7.))
                    .child(label("Channels to join"))
                    .child(mono_input(&self.autojoin))
                    .child(hint("Ignored when connecting through a bouncer, which restores channels itself.", &p)),
            )
            .child(advanced)
            .when(!self.error.is_empty(), |d| d.child(error_text(self.error.clone(), &p)))
            .child(
                h_flex()
                    .gap(px(7.))
                    .items_center()
                    .children(delete)
                    .child(div().flex_1())
                    .child(Button::new("cancel").ghost().label("Cancel").on_click(move |_, _, cx| t_cancel.update(cx, |_, cx| cx.emit(FormEvent::Cancel))))
                    .child(
                        Button::new("submit")
                            .primary()
                            .loading(self.saving)
                            .label(if self.saving { "Saving…".to_string() } else { self.submit_label.clone() })
                            .on_click(move |_, _, cx| t_save.update(cx, |f, cx| f.submit(cx))),
                    ),
            )
    }
}
