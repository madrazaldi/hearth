//! The quick switcher (Cmd/Ctrl+K): a fuzzy list of buffers across networks.
//! The ranking is ours (the kit's own filtering is off).

use super::avatar::icon;
use super::controls::{footer, footer_hint};
use super::data::*;
use super::prelude::*;
use super::theme::HearthTheme as _;
use super::widgets::count_badge;
use crate::host::{hearth, host};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use hearth_model::panels::SwitcherItem;

pub struct Switcher {
    state: Entity<CommandState>,
    /// The kit's own query field is flush with the dialog edge; the old one is an inset box, so it is ours.
    input: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl Switcher {
    pub fn new(_: Dispatch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| CommandState::new(window, cx));
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Jump to a channel, conversation or network"));
        let subs = vec![
            cx.observe(&hearth(cx), |_, _, cx| cx.notify()),
            cx.subscribe(&input, |this: &mut Self, input, ev: &InputEvent, cx| {
                if !matches!(ev, InputEvent::Change) {
                    return;
                }
                let q = input.read(cx).value().to_string();
                let _ = &this.input;
                // Only a real change should reset the highlight.
                cx.defer(move |cx| {
                    hearth(cx).update(cx, |h, cx| {
                        if h.app.switcher.as_ref().is_some_and(|s| s.query != q) {
                            h.act(cx, |app| app.switcher_set_query(&q));
                        }
                    })
                });
            }),
        ];
        Switcher { state, input, _subs: subs }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |s, cx| s.focus(window, cx));
    }
}

fn glyph(kind: BufferKind) -> &'static str {
    match kind {
        BufferKind::Channel => "hash",
        BufferKind::Query => "at-sign",
        BufferKind::Server => "server",
    }
}

impl Render for Switcher {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let results = host(cx).app.switcher_results();
        let (buffers, joins): (Vec<_>, Vec<_>) = results.into_iter().partition(|r| matches!(r, SwitcherItem::Buffer { .. }));
        let n_buffers = buffers.len();
        let items: Vec<CommandItem> = buffers
            .into_iter()
            .filter_map(|r| match r {
                SwitcherItem::Buffer { label, place, kind, unread, highlights, muted, .. } => Some((label, place, kind, unread, highlights, muted)),
                SwitcherItem::Join { .. } => None,
            })
            .map(|(label, place, kind, unread, highlights, muted)| {
                let p = p.clone();
                CommandItem::new().label(label.clone()).child(move |_, cx| {
                    let name = if kind == BufferKind::Channel { label.trim_start_matches('#').to_string() } else { label.clone() };
                    h_flex()
                        .w_full()
                        .h(px(21.5))
                        .gap(px(7.))
                        .items_center()
                        .child(icon(glyph(kind), 14., p.muted_foreground))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.25))
                                .when(unread > 0 || highlights > 0, |d| d.font_weight(FontWeight::MEDIUM).text_color(p.foreground))
                                .when(unread == 0 && highlights == 0, |d| d.text_color(p.foreground.opacity(if muted { 0.5 } else { 0.85 })))
                                .child(name),
                        )
                        .when(unread > 0 && highlights == 0, |d| d.child(div().flex_none().size(px(6.)).rounded_full().bg(p.brand)))
                        .when(highlights > 0, |d| d.child(count_badge(highlights, cx)))
                        .child(div().flex_1())
                        .child(div().flex_none().text_size(px(10.5)).text_color(p.muted_foreground).child(place.clone()))
                })
            })
            .collect();

        let mut command = Command::new(&self.state)
            .filterable(false)
            .bordered(false)
            .max_h(px(400.))
            .items(items)
            .empty(|_, _, _| div().py(px(28.)).text_center().text_size(px(12.25)).child("No matches"))
            .searchable(false)
            .header({
                let (input, p) = (self.input.clone(), p.clone());
                move |_, _, _| {
                    div().p(px(3.5)).child(
                        Input::new(&input)
                            .prefix(div().pl(px(2.)).child(icon("search", 14., p.muted_foreground)))
                            .h(px(28.))
                            .focus_bordered(false)
                            .rounded(px(8.75))
                            .border_color(p.input.opacity(0.3))
                            .bg(p.input.opacity(0.3)),
                    )
                }
            });
        if let Some(SwitcherItem::Join { channel }) = joins.into_iter().next() {
            let net = host(cx).app.active_conn().map(|c| c.name().to_string()).unwrap_or_default();
            let p = p.clone();
            command = command.group(CommandGroup::new().label("Join").item(CommandItem::new().label(format!("Join {channel}")).child(move |_, _| {
                h_flex()
                    .w_full()
                    .h(px(21.5))
                    .gap(px(7.))
                    .items_center()
                    .child(icon("plus", 14., p.muted_foreground))
                    .child(div().min_w_0().truncate().text_size(px(12.25)).child(format!("Join {channel}")))
                    .child(div().flex_1())
                    .child(div().flex_none().text_size(px(10.5)).text_color(p.muted_foreground).child(net.clone()))
            })));
        }
        // The join offer is the model's last result, so its index is simply past the buffers.
        command = command.on_confirm(move |ix, window, cx| {
            let at = if ix.section == 0 { ix.row } else { n_buffers };
            gpui_kit::component::WindowExt::close_dialog(window, cx);
            hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.switcher_choose(Some(at))));
        });
        command.footer({
            let p = p.clone();
            move |_, _, _| {
                footer(
                    vec![
                        footer_hint(&["↑↓"], "navigate", &p).into_any_element(),
                        footer_hint(&["↵"], "open", &p).into_any_element(),
                        footer_hint(&["esc"], "close", &p).into_any_element(),
                        div().ml_auto().child("Tip: type a #channel to join it").into_any_element(),
                    ],
                    &p,
                )
            }
        })
    }
}
