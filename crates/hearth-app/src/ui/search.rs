//! The search panel (Cmd/Ctrl+Shift+F): scope, a query with `from:`/`in:`
//! completion, newest-first results with highlighted matches that load more
//! as you scroll. The model owns the query,
//! the debounce and the results; this view mirrors them.

use super::avatar::icon;
use super::controls::{footer, footer_hint, search_box, segmented};
use super::data::*;
use super::prelude::*;
use super::theme::{HearthTheme as _, MONO, Palette, nick_hue};
use crate::host::{hearth, host};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use hearth_model::search::hit_time;
use hearth_model::search_panel::{FILTERS, HitView, PanelScope, SearchKey};

/// `min-h-48`, and `max-h-[56vh]` of the window.
const LIST_MIN_H: f32 = 168.;
const LIST_MAX_FRAC: f32 = 0.56;
/// What one result row measures, to size the list around its content like the old panel.
const HIT_H: f32 = 48.;
const PICK_H: f32 = 33.5;
/// Asking for older results starts this many rows from the end of the list.
const MORE_AHEAD: usize = 3;

pub struct SearchPanel {
    input: Entity<InputState>,
    list: ListState,
    /// What the list was last sized for, to tell a new search from a longer page.
    shown: (usize, Option<(i64, String)>),
    sel: usize,
    dispatch: Dispatch,
    _subs: [Subscription; 2],
}

fn identity(app: &hearth_model::App) -> Option<(i64, String)> {
    app.search_panel().hits.first().map(|h| (hit_time(h), h.msg.tags.get("msgid").cloned().unwrap_or_default()))
}

impl SearchPanel {
    pub fn new(dispatch: Dispatch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search messages"));
        let typed = cx.subscribe(&input, |_, input, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                let q = input.read(cx).value().to_string();
                // The model's query is the truth; a programmatic set_value echoes it back.
                if host(cx).app.search_panel().q != q {
                    hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.search_set_query(&q)));
                }
            }
        });
        let observed = cx.observe_in(&hearth(cx), window, |this, _, window, cx| this.sync(window, cx));
        let list = ListState::new(0, ListAlignment::Top, px(300.));
        let weak = cx.weak_entity();
        list.set_scroll_handler(move |ev, _, cx| {
            if ev.count > 0 && ev.visible_range.end + MORE_AHEAD >= ev.count {
                let weak = weak.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |_, cx| hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.search_more())));
                });
            }
        });
        let mut panel = SearchPanel { input, list, shown: (0, None), sel: 0, dispatch, _subs: [typed, observed] };
        panel.sync(window, cx);
        panel
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        let resumed = host(cx).app.search_panel().resumed;
        self.input.update(cx, |s, cx| {
            s.focus(window, cx);
            if resumed && !s.value().is_empty() {
                s.select_all(window, cx);
            }
        });
    }

    /// Bring the field and the list in line with the model.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (q, n, first, sel) = {
            let app = &host(cx).app;
            (app.search_panel().q.clone(), app.search_panel().hits.len(), identity(app), app.search_panel().sel)
        };
        if self.input.read(cx).value().as_ref() != q {
            self.input.update(cx, |s, cx| {
                s.set_value(q, window, cx);
                s.focus(window, cx);
            });
        }
        let (old, old_first) = &self.shown;
        if n < *old || first != *old_first {
            self.list.reset(n);
            self.list.scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.) });
        } else if n > *old {
            self.list.splice(*old..*old, n - *old);
        }
        self.shown = (n, first);
        if sel != self.sel {
            self.sel = sel;
            self.list.scroll_to_reveal_item(sel);
        }
        cx.notify();
    }

    fn choose(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        // The model closes the panel; the shell then drops the dialog.
        let _ = window;
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.search_choose(ix)));
    }

    fn on_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = match ev.keystroke.key.as_str() {
            "down" => SearchKey::Down,
            "up" => SearchKey::Up,
            "enter" => SearchKey::Enter,
            "tab" => SearchKey::Tab,
            _ => return,
        };
        if hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.search_key(key))) {
            cx.stop_propagation();
        }
    }
}

/// The hit's text with the query's words backgrounded.
fn highlighted(v: &HitView, p: &Palette) -> StyledText {
    let mut at = 0;
    let mut runs = Vec::new();
    for (piece, hit) in &v.parts {
        if *hit {
            runs.push((at..at + piece.len(), HighlightStyle { background_color: Some(p.brand_soft), color: Some(p.foreground), ..Default::default() }));
        }
        at += piece.len();
    }
    StyledText::new(v.text.clone()).with_highlights(runs)
}

impl Render for SearchPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let max_list = window.viewport_size().height.as_f32() * LIST_MAX_FRAC;
        let app = &host(cx).app;
        let panel = app.search_panel();
        let (query, busy_now, more, sel, scope, n_hits, has_next) =
            (panel.q.clone(), panel.busy, panel.more, panel.sel, panel.scope, panel.hits.len(), panel.next.is_some());
        let sug_sel = panel.sug_sel;
        let sugs = app.search_suggestions();
        let can_search = app.can_search();
        let origin = app.search_origin_name();
        let buffer_scope = panel.origin.as_ref().is_some_and(|o| !o.is_server());
        let this = cx.entity();
        let have_query = !query.trim().is_empty();

        let scope_bar = origin.map(|(net, buffer)| {
            let mut opts = vec![(PanelScope::All, "Everywhere".to_string()), (PanelScope::Network, net)];
            if buffer_scope {
                opts.push((PanelScope::Buffer, buffer));
            }
            let refs: Vec<(PanelScope, &str)> = opts.iter().map(|(s, l)| (*s, l.as_str())).collect();
            segmented("scope", &refs, scope, |s, _, cx| hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.search_set_scope(s))), cx)
        });

        let busy = busy_now.then(|| div().size(px(14.)).child(Spinner::new().color(p.muted_foreground)).into_any_element());

        let body = if !can_search {
            let d = self.dispatch.clone();
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(icon("search-x", 22., p.muted_foreground))
                .child(div().text_size(px(12.25)).font_weight(FontWeight::MEDIUM).child("Search is off"))
                .child(div().text_size(px(12.25)).text_color(p.muted_foreground).child("Search uses messages saved on this computer, which is turned off."))
                .child(Button::new("search-settings").outline().small().label("Change in Settings").on_click(move |_, w, cx| {
                    gpui_kit::component::WindowExt::close_dialog(w, cx);
                    hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.open_settings("privacy", None)));
                    let _ = (&d, w);
                }))
                .into_any_element()
        } else if !sugs.is_empty() {
            let kind = if query.to_lowercase().rsplit_once(char::is_whitespace).map_or(query.as_str(), |(_, w)| w).starts_with("in:") { "in" } else { "from" };
            v_flex()
                .size_full()
                .p(px(5.25))
                .children(sugs.iter().enumerate().map(|(i, name)| {
                    let n = name.clone();
                    let on = i == sug_sel;
                    let hue = nick_hue(name);
                    h_flex()
                        .id(SharedString::from(format!("sug-{i}")))
                        .h(px(PICK_H))
                        .px(px(9.75))
                        .items_center()
                        .rounded(px(8.75))
                        .text_size(px(12.25))
                        .when(on, |d| d.bg(p.accent))
                        .hover({
                            let bg = p.accent.opacity(0.7);
                            move |s| s.bg(bg)
                        })
                        .cursor_pointer()
                        .on_click(move |_, _, cx| hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.search_complete(&n))))
                        .child(div().text_color(p.muted_foreground).child(format!("{kind}:")))
                        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(if kind == "from" { p.nick(hue) } else { p.foreground }).child(name.clone()))
                }))
                .into_any_element()
        } else if !have_query {
            v_flex()
                .size_full()
                .p(px(5.25))
                .child(div().px(px(8.75)).pt(px(5.25)).pb(px(3.5)).child(super::controls::eyebrow("Narrow it down", &p)))
                .children(FILTERS.iter().map(|f| {
                    let prefix = f.prefix;
                    h_flex()
                        .id(SharedString::from(format!("filter-{prefix}")))
                        .h(px(PICK_H))
                        .px(px(9.75))
                        .gap(px(7.))
                        .items_center()
                        .rounded(px(8.75))
                        .text_size(px(12.25))
                        .cursor_pointer()
                        .hover({
                            let bg = p.accent;
                            move |s| s.bg(bg)
                        })
                        .on_click(move |_, _, cx| hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.search_add_filter(prefix))))
                        .child(
                            h_flex()
                                .w(px(126.))
                                .flex_none()
                                .font_family(MONO)
                                .text_size(px(12.5))
                                .child(prefix)
                                .child(div().text_color(p.muted_foreground).child(f.hint)),
                        )
                        .child(div().text_color(p.muted_foreground).child(f.label))
                }))
                .into_any_element()
        } else if n_hits == 0 {
            if busy_now {
                div().into_any_element()
            } else {
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .child(icon("search-x", 22., p.muted_foreground))
                    .child(div().text_size(px(12.25)).font_weight(FontWeight::MEDIUM).child("No messages found"))
                    .child(div().text_size(px(12.25)).text_color(p.muted_foreground).child("Try different words or fewer filters."))
                    .into_any_element()
            }
        } else {
            let t = this.clone();
            div()
                .relative()
                .size_full()
                .child(
                    div().size_full().child(
                        list(self.list.clone(), move |ix, _, cx| {
                            let p = cx.hearth().clone();
                            let app = &host(cx).app;
                            let Some(hit) = app.search_panel().hits.get(ix) else { return div().into_any_element() };
                            let h = app.search_hit_view(hit);
                            let t = t.clone();
                            let tail = (ix + 1 == n_hits && more).then(|| {
                                h_flex()
                                    .justify_center()
                                    .gap(px(8.))
                                    .p(px(10.5))
                                    .text_size(px(13.))
                                    .text_color(p.muted_foreground)
                                    .child(div().size(px(14.)).child(Spinner::new().color(p.muted_foreground)))
                                    .child("Loading older messages…")
                            });
                            v_flex()
                                .w_full()
                                .px(px(5.25))
                                .when(ix == 0, |d| d.pt(px(5.25)))
                                .child(
                                    v_flex()
                                        .id(SharedString::from(format!("hit-{ix}")))
                                        .gap(px(1.75))
                                        .px(px(9.75))
                                        .py(px(6.25))
                                        .rounded(px(8.75))
                                        .when(ix == sel, |d| d.bg(p.accent))
                                        // A hit whose network is gone cannot be jumped to: dimmed, and choosing it only says so.
                                        .when(!h.reachable, |d| d.opacity(0.5))
                                        .when(h.reachable, |d| d.cursor_pointer())
                                        .hover({
                                            let bg = p.accent.opacity(0.7);
                                            move |s| s.bg(bg)
                                        })
                                        .on_click(move |_, w, cx| t.update(cx, |s, cx| s.choose(ix, w, cx)))
                                        .child(
                                            h_flex()
                                                .gap(px(7.))
                                                .items_center()
                                                .text_size(px(10.5))
                                                .text_color(p.muted_foreground)
                                                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(p.foreground.opacity(0.8)).child(h.buffer.clone()))
                                                .when(h.reachable, |d| {
                                                    d.child(
                                                        div()
                                                            .px(px(5.25))
                                                            .h(px(14.))
                                                            .flex()
                                                            .items_center()
                                                            .rounded(px(7.))
                                                            .bg(p.secondary)
                                                            .text_size(px(11.))
                                                            .child(h.network.clone()),
                                                    )
                                                })
                                                .child(div().ml_auto().child(h.stamp.clone())),
                                        )
                                        .child(
                                            h_flex()
                                                .gap(px(7.))
                                                .items_baseline()
                                                .text_size(px(13.5))
                                                .child(
                                                    div()
                                                        .flex_none()
                                                        .font_weight(FontWeight::SEMIBOLD)
                                                        .text_color(p.nick(nick_hue(&h.nick)))
                                                        .child(h.nick.clone()),
                                                )
                                                .child(div().min_w_0().line_clamp(3).text_color(p.foreground.opacity(0.85)).child(highlighted(&h, &p))),
                                        ),
                                )
                                .children(tail)
                                .into_any_element()
                        })
                        .size_full(),
                    ),
                )
                .vertical_scrollbar(&self.list)
                .into_any_element()
        };

        let count = if busy_now {
            "Searching…".to_string()
        } else if n_hits > 0 {
            format!("{}{} result{}", n_hits, if has_next { "+" } else { "" }, if n_hits == 1 { "" } else { "s" })
        } else {
            String::new()
        };
        let list_h = if !can_search {
            LIST_MIN_H
        } else if !sugs.is_empty() {
            sugs.len() as f32 * PICK_H + 10.5
        } else if !have_query {
            FILTERS.len() as f32 * PICK_H + 10.5 + 25.
        } else {
            n_hits as f32 * HIT_H + 10.5
        }
        .clamp(LIST_MIN_H, max_list);
        let enter = if sugs.is_empty() { "jump to message" } else { "complete" };

        v_flex()
            .key_context("SearchPanel")
            .w_full()
            .capture_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| this.on_key(ev, cx)))
            .child(
                v_flex()
                    .flex_none()
                    .gap(px(7.))
                    .p(px(10.5))
                    .border_b_1()
                    .border_color(p.border)
                    .child(search_box(&self.input, busy, &p).h(px(31.5)))
                    .children(scope_bar.map(|s| h_flex().child(s))),
            )
            .child(div().h(px(list_h)).w_full().child(body))
            .child(footer(
                vec![
                    footer_hint(&["↑↓"], "navigate", &p).into_any_element(),
                    footer_hint(&["↵"], enter, &p).into_any_element(),
                    footer_hint(&["esc"], "close", &p).into_any_element(),
                    div().ml_auto().child(count).into_any_element(),
                ],
                &p,
            ))
    }
}
