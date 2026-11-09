//! Block rows in the message list: the WHOIS card and the collapsible MOTD.
//! The row chrome (time column, layout) is the list's.

use super::avatar::icon;
use super::data::*;
use super::prelude::*;
use super::rich_text::{RichText, plain};
use super::theme::{MONO, Palette, nick_hue};
use hearth_model::format::parse_formatting;
use hearth_model::rows::{WhoisCard, block_of};
use hearth_model::types::Line;

/// Whether a MOTD starts expanded: one asked for by the user (/help), or in a quiet buffer.
pub fn motd_default_open(l: &Line, buffer_lines: usize) -> bool {
    l.data.as_ref().is_some_and(|d| d.contains_key("title")) || buffer_lines < 40
}

pub fn whois_card(l: &Line, order: u64, card: &WhoisCard, nick_colors: bool, dispatch: &Dispatch, p: &Palette) -> AnyElement {
    let d = dispatch.clone();
    let nick = if card.nick.is_empty() { l.nick.clone() } else { card.nick.clone() };
    let n = nick.clone();
    let color = if nick_colors { p.nick(nick_hue(&nick)) } else { p.foreground };

    let rows = card.rows.iter().enumerate().map(|(i, row)| {
        let value = if row.rich {
            let mut rt = RichText::new(p.text_2);
            rt.segments(&parse_formatting(&row.value), p);
            rt.finish(SharedString::from(format!("whois-{}-{i}", l.id)), order + i as u64, dispatch.clone())
        } else if row.mono {
            div().font_family(MONO).text_size(px(12.)).text_color(p.text_2).child(row.value.clone()).into_any_element()
        } else {
            div().text_color(p.text_2).child(row.value.clone()).into_any_element()
        };
        h_flex()
            .gap(px(16.))
            .items_start()
            .child(div().flex_none().w(px(64.)).text_color(p.muted_foreground).child(row.label))
            .child(div().flex_1().min_w_0().child(value))
    });

    v_flex()
        .max_w(px(576.))
        .py(px(10.))
        .px(px(16.))
        .gap(px(8.))
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .text_size(px(13.))
        .child(
            h_flex()
                .items_baseline()
                .gap(px(10.))
                .child(
                    div()
                        .id(SharedString::from(format!("whois-nick-{}", l.id)))
                        .cursor_pointer()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .hover(|s| s.underline())
                        .on_click(move |_, w, cx| d(UiAction::OpenNick(n.clone()), w, cx))
                        .child(nick),
                )
                .child(div().text_size(px(12.)).font_family(MONO).text_color(p.muted_foreground).child(card.user_host.clone())),
        )
        .child(v_flex().gap(px(3.)).children(rows))
        .into_any_element()
}

/// The MOTD block; `toggle` flips it open or shut.
pub fn motd_block(l: &Line, open: bool, toggle: impl Fn(&mut Window, &mut App) + 'static, p: &Palette) -> AnyElement {
    let (title, body) = block_of(l).unwrap_or_default();
    v_flex()
        .max_w(px(640.))
        .py(px(10.))
        .px(px(14.))
        .rounded(px(10.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .child(
            h_flex()
                .id(SharedString::from(format!("motd-{}", l.id)))
                .gap(px(4.))
                .items_center()
                .cursor_pointer()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.muted_foreground)
                .on_click(move |_, w, cx| toggle(w, cx))
                .child(icon(if open { "chevron-down" } else { "chevron-right" }, 12., p.muted_foreground))
                .child(title),
        )
        .when(open, |d| {
            d.child(div().mt(px(8.)).mb(px(2.)).font_family(MONO).text_size(px(12.)).line_height(relative(1.5)).text_color(p.text_2).child(plain(&body)))
        })
        .into_any_element()
}
