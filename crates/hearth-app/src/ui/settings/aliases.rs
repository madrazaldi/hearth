//! Your own slash commands.

use crate::ui::controls::*;
use crate::ui::prelude::*;
use crate::ui::theme::{HearthTheme as _, MONO};
use crate::ui::widgets::{icon_button, tag};
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use hearth_model::aliases::{alias_error, alias_name};
use hearth_model::commands::is_builtin;
use std::collections::BTreeMap;

pub enum AliasEvent {
    Set(String, String),
    Remove(String),
}

impl EventEmitter<AliasEvent> for AliasEditor {}

pub struct AliasEditor {
    aliases: BTreeMap<String, String>,
    name: Entity<InputState>,
    body: Entity<InputState>,
    tried: bool,
    _subs: Vec<Subscription>,
}

impl AliasEditor {
    pub fn new(aliases: BTreeMap<String, String>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = new_input(window, cx, "", "/ns");
        let body = new_input(window, cx, "", "/msg NickServ $*");
        let subs = [&name, &body]
            .map(|s| {
                cx.subscribe_in(s, window, |this: &mut Self, _, ev: &InputEvent, window, cx| match ev {
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } => this.save(window, cx),
                    _ => {}
                })
            })
            .into();
        AliasEditor { aliases, name, body, tried: false, _subs: subs }
    }

    pub fn set_aliases(&mut self, aliases: BTreeMap<String, String>, cx: &mut Context<Self>) {
        self.aliases = aliases;
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tried = true;
        let key = alias_name(&text(&self.name, cx));
        let body = text(&self.body, cx);
        if alias_error(&key, &body).is_some() {
            return cx.notify();
        }
        cx.emit(AliasEvent::Set(key, body.trim().to_string()));
        self.name.update(cx, |s, cx| s.set_value("", window, cx));
        self.body.update(cx, |s, cx| s.set_value("", window, cx));
        self.tried = false;
        cx.notify();
    }

    fn edit(&mut self, n: &str, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.aliases.get(n).cloned().unwrap_or_default();
        self.name.update(cx, |s, cx| s.set_value(n.to_string(), window, cx));
        self.body.update(cx, |s, cx| s.set_value(body, window, cx));
    }
}

impl Render for AliasEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let this = cx.entity();
        let (name, body) = (text(&self.name, cx), text(&self.body, cx));
        let key = alias_name(&name);
        let error = alias_error(&key, &body);
        let exists = self.aliases.contains_key(&key);
        let n = self.aliases.len();
        let hover = p.hover();

        let list = (n > 0).then(|| {
            v_flex().mb(px(4.)).rounded(px(8.)).border_1().border_color(p.border).overflow_hidden().children(self.aliases.iter().enumerate().map(
                |(i, (a, b))| {
                    let (t_edit, t_rm) = (this.clone(), this.clone());
                    let (edit_name, rm_name) = (a.clone(), a.clone());
                    h_flex()
                        .gap(px(8.))
                        .py(px(4.))
                        .pr(px(6.))
                        .items_center()
                        .text_size(px(12.5))
                        .when(i + 1 < n, |d| d.border_b_1().border_color(p.border))
                        .child(
                            h_flex()
                                .id(SharedString::from(format!("alias-{a}")))
                                .flex_1()
                                .min_w_0()
                                .gap(px(12.))
                                .px(px(12.))
                                .py(px(4.))
                                .rounded(px(4.))
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover))
                                .on_click(move |_, w, cx| {
                                    let n = edit_name.clone();
                                    t_edit.update(cx, |e, cx| e.edit(&n, w, cx))
                                })
                                .child(div().flex_none().min_w(px(64.)).font_family(MONO).child(format!("/{a}")))
                                .child(div().truncate().font_family(MONO).text_color(p.muted_foreground).child(b.clone())),
                        )
                        .when(is_builtin(a), |d| d.child(tag("replaces built-in", cx)))
                        .child(icon_button(
                            SharedString::from(format!("alias-rm-{a}")),
                            "x",
                            (24., 13.),
                            p.muted_foreground,
                            p.muted,
                            Some("Delete alias"),
                            move |_, cx| {
                                let n = rm_name.clone();
                                t_rm.update(cx, |_, cx| cx.emit(AliasEvent::Remove(n)))
                            },
                        ))
                },
            ))
        });

        let t_save = this.clone();
        v_flex()
            .gap(px(12.))
            .children(list)
            .child(
                h_flex()
                    .gap(px(8.))
                    .child(div().w(px(112.)).flex_none().child(mono_input(&self.name)))
                    .child(div().flex_1().min_w_0().child(mono_input(&self.body)))
                    .child(Button::new("alias-add").outline().disabled(name.trim().is_empty() || body.trim().is_empty()).label(if exists { "Update" } else { "Add" }).on_click(move |_, w, cx| t_save.update(cx, |e, cx| e.save(w, cx)))),
            )
            .child(match (self.tried, error) {
                (true, Some(e)) => error_text(e, &p),
                _ if !key.is_empty() && is_builtin(&key) => div().text_size(px(12.)).text_color(p.warn).child(format!("/{key} is a built-in command. This alias will replace it; inside the alias, /{key} still runs the built-in.")),
                _ => div(),
            })
            .child(rich_hint("`$1`…`$9` are arguments, `$2-` is the second onward, `$*` is all of them. `$nick`, `$channel` and `$network` fill in the obvious. Separate commands with `;` (`\\;` for a literal one). Lines without a leading `/` are sent as messages.", &p))
    }
}
