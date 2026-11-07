//! The right-hand member list: sections (Operators, Voiced, Members) of
//! avatar + coloured nick + prefix glyph.

use super::avatar::{Avatar, dot};
use super::bind::nick_pic;
use super::controls::search_box;
use super::data::*;
use super::prelude::*;
use super::theme::{HearthTheme as _, MONO, nick_hue};
use crate::host::{host, request_flush};
use gpui_kit::component::input::InputState;
use hearth_model::panels::MemberRow;
use std::rc::Rc;

pub const WIDTH: f32 = 232.;
const ROW_H: f32 = 32.;

#[derive(IntoElement)]
pub struct MemberList {
    buffer: BufferId,
    rows: Rc<Vec<MemberRow>>,
    /// Everyone in the channel, before the find field narrows the rows.
    everyone: usize,
    dispatch: Dispatch,
    find: Option<Entity<InputState>>,
}

/// Channels with more members than this get a "Find member" field.
const FIND_AT: usize = 30;

impl MemberList {
    pub fn new(buffer: BufferId, rows: Rc<Vec<MemberRow>>, everyone: usize, dispatch: Dispatch) -> Self {
        MemberList { buffer, rows, everyone, dispatch, find: None }
    }

    /// The input the list filters by; shown only for big channels.
    pub fn find(mut self, state: Entity<InputState>) -> Self {
        self.find = Some(state);
        self
    }
}

impl RenderOnce for MemberList {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = cx.hearth().clone();
        let everyone = self.everyone;
        let finder = self.find.clone().filter(|_| everyone > FIND_AT);
        let rows = self.rows.clone();
        let buffer = self.buffer.clone();
        let total = rows.len();
        let dispatch = self.dispatch;

        v_flex()
            .w(px(WIDTH))
            .h_full()
            .flex_none()
            .border_l_1()
            .border_color(p.border)
            .children(finder.map(|f| div().flex_none().px(px(10.5)).pt(px(10.5)).pb(px(3.5)).child(search_box(&f, None, &p).h(px(28.)))))
            .child(if total == 0 {
                div()
                    .p(px(8.))
                    .text_size(px(12.5))
                    .text_color(p.muted_foreground)
                    .child(if everyone == 0 { "Loading members…" } else { "No one matches" })
                    .into_any_element()
            } else {
                let n = rows.len();
                uniform_list("members", n, move |range, _, cx| {
                    let p = cx.hearth().clone();
                    let out: Vec<AnyElement> = range
                        .map(|i| match &rows[i] {
                            MemberRow::Head { title, count } => div()
                                .h(px(ROW_H))
                                .px(px(14.))
                                .flex()
                                .items_end()
                                .pb(px(5.25))
                                .gap(px(5.25))
                                .text_size(px(11.))
                                .text_color(p.muted_foreground.opacity(0.8))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title.to_uppercase())
                                .child(div().font_weight(FontWeight::MEDIUM).opacity(0.7).child(format!("— {count}")))
                                .into_any_element(),
                            MemberRow::Member { member: m, .. } => {
                                let h = host(cx);
                                let pic = h.app.conn(&buffer.conn).map(|c| nick_pic(h, c, &m.nick)).unwrap_or_default();
                                let hue = nick_hue(&m.nick);
                                let d = dispatch.clone();
                                let nick = m.nick.clone();
                                let hover = p.accent.opacity(0.7);
                                div()
                                    .h(px(ROW_H))
                                    .px(px(7.))
                                    .child(
                                        h_flex()
                                            .id(SharedString::from(format!("member-{}", m.nick)))
                                            .size_full()
                                            .px(px(7.))
                                            .gap(px(8.75))
                                            .items_center()
                                            .rounded(px(6.75))
                                            .cursor_pointer()
                                            .hover(move |s| s.bg(hover))
                                            .on_click(move |_, w, cx| d(UiAction::OpenNick(nick.clone()), w, cx))
                                            .child(
                                                div()
                                                    .relative()
                                                    .flex_none()
                                                    .when(m.away, |d| d.opacity(0.45))
                                                    .child(Avatar::new(m.nick.clone(), 24.).picture(&pic))
                                                    .when(m.away, |d| {
                                                        d.child(div().absolute().right(px(-1.75)).bottom(px(-1.75)).child(dot(
                                                            8.75,
                                                            p.warn,
                                                            Some(p.background),
                                                        )))
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(13.5))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(p.nick(hue))
                                                    .when(m.away, |d| d.opacity(0.45))
                                                    .child(m.nick.clone()),
                                            )
                                            .when(!m.prefix.is_empty(), |d| {
                                                d.child(
                                                    div()
                                                        .flex_none()
                                                        .font_family(MONO)
                                                        .text_size(px(11.))
                                                        .text_color(p.muted_foreground.opacity(0.7))
                                                        .child(m.prefix.clone()),
                                                )
                                            }),
                                    )
                                    .into_any_element()
                            }
                        })
                        .collect();
                    if host(cx).images.pending() {
                        request_flush(cx);
                    }
                    out
                })
                .flex_1()
                .min_h_0()
                .into_any_element()
            })
    }
}
