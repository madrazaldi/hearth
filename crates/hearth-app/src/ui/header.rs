//! The buffer header: icon tile, name, topic, search, member toggle, menu.

use super::avatar::{Avatar, dot, icon};
use super::data::*;
use super::menus::{BufferFacts, buffer_menu};
use super::prelude::*;
use super::rich_text::plain;
use super::theme::{HearthTheme as _, Palette};
use super::widgets::icon_button;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::DropdownMenu as _;
use hearth_model::types::NotifyLevel;

#[derive(IntoElement)]
pub struct BufferHeader {
    view: HeaderView,
    dispatch: Dispatch,
    topic_open: bool,
    filter: Option<Entity<InputState>>,
}

impl BufferHeader {
    pub fn new(view: HeaderView, dispatch: Dispatch) -> Self {
        BufferHeader { view, dispatch, topic_open: false, filter: None }
    }

    /// The message filter field, shown in place of the filter button while it is open.
    pub fn filter(mut self, field: Option<Entity<InputState>>) -> Self {
        self.filter = field;
        self
    }

    /// Show the whole topic, and who set it, under the header.
    pub fn topic_open(mut self, open: bool) -> Self {
        self.topic_open = open;
        self
    }
}

impl RenderOnce for BufferHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p: Palette = cx.hearth().clone();
        let BufferHeader { view: v, dispatch, topic_open, filter } = self;
        let muted = p.muted_foreground;

        let tile = match v.kind {
            BufferKind::Query => div().flex_none().child(Avatar::new(v.title.clone(), 32.).picture(&v.picture)).into_any_element(),
            kind => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(32.))
                .rounded(px(10.))
                .border_1()
                .border_color(p.border)
                .bg(p.muted.opacity(0.6))
                .child(icon(
                    match kind {
                        BufferKind::Channel => "hash",
                        _ => "square-terminal",
                    },
                    16.,
                    muted,
                ))
                .into_any_element(),
        };

        let title = if v.kind == BufferKind::Channel { v.title.trim_start_matches('#').to_string() } else { v.title.clone() };
        let sub = match (&v.topic, &v.subtitle) {
            (Some(t), _) if v.kind == BufferKind::Channel => {
                let d = dispatch.clone();
                div()
                    .id("topic")
                    .mt(px(2.))
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.5))
                    .text_color(muted)
                    .cursor_pointer()
                    .hover({
                        let fg = p.foreground;
                        move |s| s.text_color(fg)
                    })
                    .on_click(move |_, w, cx| d(UiAction::ShowTopic, w, cx))
                    .child(plain(t))
                    .into_any_element()
            }
            (_, Some(s)) => div().mt(px(2.)).truncate().text_size(px(12.5)).text_color(muted).child(s.clone()).into_any_element(),
            _ if v.kind == BufferKind::Channel => div().mt(px(2.)).text_size(px(12.5)).text_color(muted.opacity(0.7)).child("No topic").into_any_element(),
            _ => div().into_any_element(),
        };

        let (d_search, d_members) = (dispatch.clone(), dispatch.clone());
        let d_close = dispatch.clone();
        let bar = h_flex()
            .h(px(56.))
            .flex_none()
            .items_center()
            .gap(px(12.))
            .pl(px(16.))
            .pr(px(12.))
            .border_b_1()
            .border_color(p.border)
            .child(
                h_flex().flex_1().min_w_0().items_center().gap(px(12.)).window_control_area(WindowControlArea::Drag).child(tile).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).line_height(relative(1.25)).child(title))
                                .when(v.kind == BufferKind::Channel && !v.joined, |d| {
                                    d.child(
                                        div()
                                            .px(px(6.))
                                            .h(px(18.))
                                            .flex()
                                            .items_center()
                                            .rounded(px(6.))
                                            .border_1()
                                            .border_color(p.warn.opacity(0.4))
                                            .text_size(px(10.5))
                                            .text_color(p.warn)
                                            .child("not joined"),
                                    )
                                })
                                .when_some(v.online.filter(|_| v.kind == BufferKind::Query), |d, on| {
                                    d.child(
                                        h_flex()
                                            .gap(px(4.))
                                            .items_center()
                                            .text_size(px(11.5))
                                            .text_color(muted)
                                            .child(dot(6., if on { p.ok } else { muted.opacity(0.6) }, None))
                                            .child(if on { "online" } else { "offline" }),
                                    )
                                })
                                .when(v.level == Some(NotifyLevel::None), |d| d.child(icon("bell-off", 12., muted))),
                        )
                        .child(sub),
                ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(match filter {
                        Some(field) => h_flex()
                            .id("filter")
                            .h(px(32.))
                            .w(px(240.))
                            .pl(px(10.))
                            .pr(px(4.))
                            .gap(px(8.))
                            .items_center()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(p.input)
                            .child(icon("search", 13., muted))
                            .child(
                                div().flex_1().min_w_0().text_size(px(13.)).child(Input::new(&field).appearance(false).bordered(false).focus_bordered(false)),
                            )
                            .child(icon_button("filter-close", "x", (24., 13.), muted, p.muted, Some("Close filter"), move |w, cx| {
                                d_close(UiAction::CloseFilter, w, cx)
                            }))
                            .into_any_element(),
                        None => icon_button("filter", "search", (32., 16.), muted, p.muted, Some("Filter messages"), move |w, cx| {
                            d_search(UiAction::OpenFilter, w, cx)
                        })
                        .into_any_element(),
                    })
                    .when(v.kind == BufferKind::Channel, |d| {
                        let on = v.show_members;
                        d.child(
                            h_flex()
                                .id("members-toggle")
                                .h(px(32.))
                                .px(px(8.))
                                .gap(px(6.))
                                .items_center()
                                .rounded(px(8.))
                                .cursor_pointer()
                                .text_color(if on { p.foreground } else { muted })
                                .when(on, |d| d.bg(p.muted))
                                .hover({
                                    let bg = p.muted;
                                    move |s| s.bg(bg)
                                })
                                .on_click(move |_, w, cx| d_members(UiAction::ToggleMembers, w, cx))
                                .child(icon("users", 15., if on { p.foreground } else { muted }))
                                .when(v.member_count > 0, |d| {
                                    d.child(div().text_size(px(12.5)).font_weight(FontWeight::MEDIUM).child(v.member_count.to_string()))
                                }),
                        )
                    })
                    .when(v.kind != BufferKind::Server, |d| {
                        let (id, kind, joined, level, dispatch) = (v.id.clone(), v.kind, v.joined, v.level, dispatch.clone());
                        d.child(
                            Button::new("header-more")
                                .ghost()
                                .small()
                                .icon(icon("ellipsis", 16., muted))
                                .dropdown_menu(move |m, w, cx| buffer_menu(m, &id, BufferFacts { kind, joined, level }, &dispatch, w, cx)),
                        )
                    }),
            );

        let full = v.topic.clone().filter(|_| topic_open && v.kind == BufferKind::Channel);
        let note = v.topic_note.clone();
        v_flex().flex_none().child(bar).children(full.map(|t| {
            let d = dispatch.clone();
            v_flex()
                .id("topic-full")
                .gap(px(4.))
                .px(px(16.))
                .py(px(10.))
                .border_b_1()
                .border_color(p.border)
                .bg(p.muted.opacity(0.35))
                .cursor_pointer()
                .on_click(move |_, w, cx| d(UiAction::ShowTopic, w, cx))
                .child(div().text_size(px(13.5)).line_height(relative(1.45)).child(plain(&t)))
                .children(note.map(|n| div().text_size(px(11.5)).text_color(muted).child(n)))
        }))
    }
}
