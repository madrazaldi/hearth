//! The channel browser (`/list`): filter, sort by users or name, join or open.

use super::avatar::icon;
use super::controls::{search_box, segmented};
use super::data::*;
use super::prelude::*;
use super::rich_text::plain;
use super::theme::HearthTheme as _;
use super::widgets::icon_button;
use crate::host::{hearth, host};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use hearth_model::panels::{BROWSER_SHOWN, ChannelSort};

/// Two lines (13.5px and 12.5px at 1.45) and `py-2`.
const ROW_H: f32 = 51.5;

/// The state lives in the model (`browser`, `browser_conn`); this holds the filter field.
pub struct ChannelBrowser {
    filter: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl ChannelBrowser {
    pub fn new(_: Dispatch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = hearth(cx);
        let start = model.read(cx).app.browser.query.clone();
        let filter = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Filter by name or topic");
            s.set_value(start, window, cx);
            s
        });
        let subs = vec![
            cx.subscribe(&filter, |_, input, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    let q = input.read(cx).value().to_string();
                    hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.browser_set_query(&q)));
                }
            }),
            cx.observe(&model, |_, _, cx| cx.notify()),
        ];
        ChannelBrowser { filter, _subs: subs }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.filter.update(cx, |s, cx| s.focus(window, cx));
    }
}

impl Render for ChannelBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let h = host(cx);
        let Some(view) = h.app.browser_view() else { return div().into_any_element() };
        let conn = h.app.browser_conn.clone().unwrap_or_default();
        let (network, connected) =
            h.app.conn(&conn).map(|c| (c.name().to_string(), c.status == hearth_model::types::ConnStatus::Connected)).unwrap_or_default();
        let (sort, descending) = (h.app.browser.sort, h.app.browser.descending);
        let query = self.filter.read(cx).value().to_string();
        let height = (window.viewport_size().height.as_f32() * 0.88).min(640.);

        let status = if view.loading {
            h_flex()
                .gap(px(6.))
                .items_center()
                .child(div().size(px(12.)).child(Spinner::new().color(p.muted_foreground)))
                .child(if view.total == 0 { "Loading…".to_string() } else { format!("Loading… {} so far", group(view.total)) })
                .into_any_element()
        } else if !connected {
            div().child("Not connected").into_any_element()
        } else {
            div()
                .child(format!("{} channel{}{}", group(view.total), if view.total == 1 { "" } else { "s" }, if query.is_empty() { "" } else { " match" }))
                .into_any_element()
        };

        let rows = std::rc::Rc::new(view.items.clone());
        let list = if rows.is_empty() {
            if view.loading {
                div().into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap(px(8.))
                    .text_color(p.muted_foreground)
                    .child(icon("list-x", 22., p.muted_foreground))
                    .child(div().text_size(px(12.25)).font_weight(FontWeight::MEDIUM).text_color(p.foreground).child(if query.is_empty() {
                        "No channels listed"
                    } else {
                        "No channels match"
                    }))
                    .into_any_element()
            }
        } else {
            let n = rows.len();
            let capped = view.total > BROWSER_SHOWN;
            let total = view.total;
            let conn = conn.clone();
            v_flex()
                .flex_1()
                .min_h_0()
                .child(
                    uniform_list("channel-rows", n, move |range, _, cx| {
                        let p = cx.hearth().clone();
                        range
                            .map(|i| {
                                let c = &rows[i];
                                let name = c.name.clone();
                                let joined = host(cx).app.conn(&conn).is_some_and(|k| k.buffer(&c.name).is_some());
                                let hover = p.accent;
                                // The list sizes rows to their content, so the margin lives on a full-width wrapper.
                                div()
                                    .w_full()
                                    .child(
                                        h_flex()
                                            .id(SharedString::from(format!("ch-{i}")))
                                            .w_full()
                                            .h(px(ROW_H))
                                            .px(px(8.75))
                                            .gap(px(12.25))
                                            .items_center()
                                            .rounded(px(8.75))
                                            .hover(move |s| s.bg(hover))
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child(c.name.clone()))
                                                    .when(!c.topic.is_empty(), |d| {
                                                        d.child(div().truncate().text_size(px(12.5)).text_color(p.muted_foreground).child(plain(&c.topic)))
                                                    }),
                                            )
                                            .child(
                                                h_flex()
                                                    .flex_none()
                                                    .h(px(17.5))
                                                    .px(px(7.))
                                                    .gap(px(3.5))
                                                    .items_center()
                                                    .rounded_full()
                                                    .bg(p.secondary)
                                                    .text_size(px(10.5))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(p.muted_foreground)
                                                    .child(icon("users", 10.5, p.muted_foreground))
                                                    .child(group(c.users as usize)),
                                            )
                                            .child(div().w(px(49.)).flex_none().child({
                                                let b = Button::new(SharedString::from(format!("join-{i}"))).small().w_full().label(if joined {
                                                    "Open"
                                                } else {
                                                    "Join"
                                                });
                                                let b = if joined { b.outline() } else { b.primary() };
                                                b.on_click(move |_, _, cx| {
                                                    let name = name.clone();
                                                    // Joins, or opens the channel we are already in, and closes the browser.
                                                    hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.browser_join(&name)))
                                                })
                                            })),
                                    )
                                    .into_any_element()
                            })
                            .collect()
                    })
                    .flex_1()
                    .min_h_0()
                    .p(px(0.)),
                )
                .when(capped, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .px(px(14.))
                            .py(px(10.5))
                            .text_size(px(13.))
                            .text_color(p.muted_foreground)
                            .child(format!("Showing {BROWSER_SHOWN} of {}. Narrow the filter to see more.", group(total))),
                    )
                })
                .into_any_element()
        };

        let refresh = {
            let conn = conn.clone();
            icon_button("refresh", "refresh-cw", (24.5, 15.), p.muted_foreground, p.muted, Some("Refresh"), move |_, cx| {
                hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.refresh_channel_list(&conn)))
            })
        };

        let sort_bar = segmented(
            "sort",
            &[(ChannelSort::Users, "Most users"), (ChannelSort::Name, "Name")],
            sort,
            move |s, _, cx| {
                if s != sort {
                    hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.browser_set_sort(s)))
                }
            },
            cx,
        );
        let _ = descending;

        v_flex()
            .relative()
            .h(px(height))
            .w_full()
            .child(
                v_flex()
                    .flex_none()
                    .gap(px(3.5))
                    .px(px(14.))
                    .pt(px(14.))
                    .pr(px(70.))
                    .pb(px(10.5))
                    .child(div().text_size(px(15.75)).font_weight(FontWeight::SEMIBOLD).child(format!("Channels on {network}")))
                    .child(div().text_size(px(12.5)).text_color(p.muted_foreground).child(status)),
            )
            .child(div().absolute().top(px(7.)).right(px(35.)).child(refresh))
            .child(
                h_flex()
                    .flex_none()
                    .gap(px(8.75))
                    .px(px(14.))
                    .pb(px(10.5))
                    .border_b_1()
                    .border_color(p.border)
                    .child(div().flex_1().min_w_0().child(search_box(&self.filter, None, &p).h(px(31.5))))
                    .child(sort_bar),
            )
            .child(div().flex_1().min_h_0().flex().flex_col().p(px(7.)).child(list))
            .into_any_element()
    }
}

/// 1234 -> "1,234".
fn group(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
