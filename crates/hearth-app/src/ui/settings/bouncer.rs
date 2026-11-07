//! Networks on a soju bouncer: list, add, edit, remove. The changes go out as `BOUNCER` commands.

use crate::host::hearth;
use crate::ui::controls::*;
use crate::ui::data::*;
use crate::ui::prelude::*;
use crate::ui::theme::{HearthTheme as _, MONO};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use hearth_model::panels::NetworkDraft;

pub struct BouncerEditor {
    conn: String,
    networks: Vec<BouncerNetwork>,
    /// `Some(None)` is a new network, `Some(Some(id))` an existing one.
    editing: Option<Option<String>>,
    tls: bool,
    confirm_delete: Option<String>,
    error: String,
    name: Entity<InputState>,
    host: Entity<InputState>,
    port: Entity<InputState>,
    nickname: Entity<InputState>,
    username: Entity<InputState>,
    pass: Entity<InputState>,
}

impl BouncerEditor {
    pub fn new(conn: String, networks: Vec<BouncerNetwork>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        BouncerEditor {
            conn,
            networks,
            editing: None,
            tls: true,
            confirm_delete: None,
            error: String::new(),
            name: new_input(window, cx, "", "Libera.Chat"),
            host: new_input(window, cx, "", "irc.libera.chat"),
            port: new_input(window, cx, "6697", ""),
            nickname: new_input(window, cx, "", "Bouncer default"),
            username: new_input(window, cx, "", "Bouncer default"),
            pass: new_password(window, cx, "", "Usually empty"),
        }
    }

    pub fn set_networks(&mut self, mut networks: Vec<BouncerNetwork>, cx: &mut Context<Self>) {
        networks.sort_by_key(|n| n.name.to_lowercase());
        self.networks = networks;
        cx.notify();
    }

    fn fill(&mut self, n: Option<&BouncerNetwork>, window: &mut Window, cx: &mut Context<Self>) {
        let set = |s: &Entity<InputState>, v: &str, window: &mut Window, cx: &mut Context<Self>| s.update(cx, |s, cx| s.set_value(v.to_string(), window, cx));
        set(&self.name, n.map_or("", |n| &n.name), window, cx);
        set(&self.host, n.map_or("", |n| &n.host), window, cx);
        set(&self.port, n.map_or("6697", |n| &n.port), window, cx);
        set(&self.nickname, n.map_or("", |n| &n.nickname), window, cx);
        set(&self.username, n.map_or("", |n| &n.username), window, cx);
        set(&self.pass, "", window, cx);
        self.tls = n.is_none_or(|n| n.tls);
        self.editing = Some(n.map(|n| n.id.clone()));
        self.error.clear();
        cx.notify();
    }

    fn save(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(netid) = self.editing.clone() else { return };
        let t = |s: &Entity<InputState>| text(s, cx).trim().to_string();
        let draft = NetworkDraft {
            netid,
            name: t(&self.name),
            host: t(&self.host),
            port: t(&self.port),
            tls: self.tls,
            nickname: t(&self.nickname),
            username: t(&self.username),
            pass: text(&self.pass, cx),
        };
        if draft.host.is_empty() {
            self.error = "A host is required".into();
            return cx.notify();
        }
        let conn = self.conn.clone();
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.bouncer_save(&conn, &draft)));
        self.editing = None;
        cx.notify();
    }
}

fn state_color(s: ConnState, p: &crate::ui::theme::Palette) -> Hsla {
    match s {
        ConnState::Connected => p.ok,
        ConnState::Connecting => p.warn,
        ConnState::Disconnected => p.destructive,
    }
}

impl Render for BouncerEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let this = cx.entity();
        let editing = self.editing.clone();

        let form = editing.clone().map(|netid| {
            let (t_tls, t_cancel, t_save) = (this.clone(), this.clone(), this.clone());
            v_flex()
                .gap(px(12.))
                .p(px(14.))
                .mb(px(14.))
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .child(
                    h_flex()
                        .gap(px(10.))
                        .child(field("Name", Input::new(&self.name)).flex_1())
                        .child(field("Host", mono_input(&self.host)).flex_1())
                        .child(field("Port", mono_input(&self.port)).w(px(90.))),
                )
                .child(
                    h_flex()
                        .gap(px(10.))
                        .child(field("Nickname", Input::new(&self.nickname)).flex_1())
                        .child(field("Username", Input::new(&self.username)).flex_1())
                        .child(field("Server password", Input::new(&self.pass).mask_toggle()).flex_1()),
                )
                .child(switch_inline(
                    "bouncer-tls",
                    "Use TLS",
                    None,
                    self.tls,
                    move |v, _, cx| {
                        t_tls.update(cx, |b, cx| {
                            b.tls = v;
                            cx.notify()
                        })
                    },
                    &p,
                ))
                .when(!self.error.is_empty(), |d| d.child(error_text(self.error.clone(), &p)))
                .child(
                    h_flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(Button::new("net-cancel").ghost().label("Cancel").on_click(move |_, _, cx| {
                            t_cancel.update(cx, |b, cx| {
                                b.editing = None;
                                cx.notify()
                            })
                        }))
                        .child(
                            Button::new("net-save")
                                .primary()
                                .label(if netid.is_some() { "Save network" } else { "Add network" })
                                .on_click(move |_, w, cx| t_save.update(cx, |b, cx| b.save(w, cx))),
                        ),
                )
        });

        let n = self.networks.len();
        let list = v_flex().rounded(px(8.)).border_1().border_color(p.border).overflow_hidden().children(self.networks.iter().enumerate().map(|(i, net)| {
            let id = net.id.clone();
            let confirming = self.confirm_delete.as_deref() == Some(&id);
            let (t_edit, t_ask, t_keep, t_del) = (this.clone(), this.clone(), this.clone(), this.clone());
            let (edit_net, del_id, ask_id) = (net.clone(), id.clone(), id.clone());
            let sub = format!(
                "{}{}{}{}",
                net.host,
                if net.port.is_empty() { String::new() } else { format!(":{}", net.port) },
                if net.tls { " · TLS" } else { "" },
                net.error.as_ref().map(|e| format!(" · {e}")).unwrap_or_default()
            );
            h_flex()
                .gap(px(10.))
                .px(px(14.))
                .py(px(10.))
                .items_center()
                .when(i + 1 < n, |d| d.border_b_1().border_color(p.border))
                .child(status_dot(state_color(net.state, &p)))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(if !net.name.is_empty() {
                            net.name.clone()
                        } else if !net.host.is_empty() {
                            net.host.clone()
                        } else {
                            net.id.clone()
                        }))
                        .child(div().truncate().font_family(MONO).text_size(px(12.)).text_color(p.muted_foreground).child(sub)),
                )
                .children(if confirming {
                    vec![
                        Button::new(SharedString::from(format!("net-del-{id}")))
                            .danger()
                            .small()
                            .label("Really remove?")
                            .on_click({
                                let conn = self.conn.clone();
                                move |_, _, cx| {
                                    hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.bouncer_delete(&conn, &del_id)));
                                    t_del.update(cx, |b, cx| {
                                        b.confirm_delete = None;
                                        cx.notify()
                                    });
                                }
                            })
                            .into_any_element(),
                        Button::new(SharedString::from(format!("net-keep-{id}")))
                            .ghost()
                            .small()
                            .label("Keep")
                            .on_click(move |_, _, cx| {
                                t_keep.update(cx, |b, cx| {
                                    b.confirm_delete = None;
                                    cx.notify()
                                })
                            })
                            .into_any_element(),
                    ]
                } else {
                    vec![
                        Button::new(SharedString::from(format!("net-rm-{id}")))
                            .ghost()
                            .small()
                            .text_color(p.destructive)
                            .label("Remove")
                            .on_click(move |_, _, cx| {
                                let id = ask_id.clone();
                                t_ask.update(cx, |b, cx| {
                                    b.confirm_delete = Some(id);
                                    cx.notify()
                                })
                            })
                            .into_any_element(),
                        Button::new(SharedString::from(format!("net-edit-{id}")))
                            .outline()
                            .small()
                            .label("Edit")
                            .on_click(move |_, w, cx| {
                                let n = edit_net.clone();
                                t_edit.update(cx, |b, cx| b.fill(Some(&n), w, cx))
                            })
                            .into_any_element(),
                    ]
                })
        }));

        let t_add = this.clone();
        v_flex()
            .child(
                h_flex()
                    .mb(px(12.))
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Networks on this bouncer"))
                    .when(editing.is_none(), |d| {
                        d.child(
                            Button::new("net-add")
                                .outline()
                                .small()
                                .label("Add network")
                                .on_click(move |_, w, cx| t_add.update(cx, |b, cx| b.fill(None, w, cx))),
                        )
                    }),
            )
            .children(form)
            .child(if n == 0 {
                div().p(px(14.)).text_size(px(13.)).text_color(p.muted_foreground).child("No networks yet.").into_any_element()
            } else {
                list.into_any_element()
            })
    }
}
