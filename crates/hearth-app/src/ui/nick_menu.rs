//! The menu behind a nick: who they are, then message / mention / whois /
//! watch / ignore and, for operators, op / voice / kick.

use super::avatar::{Avatar, icon};
use super::data::*;
use super::prelude::*;
use super::theme::{HearthTheme as _, MONO, nick_hue};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};

fn badge(text: &str, color: Hsla, border: Hsla) -> Div {
    div()
        .px(px(7.))
        .h(px(17.5))
        .flex()
        .items_center()
        .rounded_full()
        .border_1()
        .border_color(border)
        .text_size(px(10.5))
        .text_color(color)
        .child(text.to_string())
}

fn header(v: &NickMenuView, cx: &App) -> AnyElement {
    let p = cx.hearth().clone();
    let has_meta = !v.prefix.is_empty() || v.online.is_some() || v.away || v.ignored || v.account.is_some();
    v_flex()
        .w(px(252.))
        .child(
            h_flex()
                .gap(px(10.5))
                .px(px(6.))
                .pt(px(4.))
                .pb(px(8.))
                .when(v.show_avatar, |d| d.child(Avatar::new(v.nick.clone(), 40.).picture(&v.picture)))
                .child(
                    v_flex()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .truncate()
                                .text_size(px(15.))
                                .line_height(relative(1.25))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.nick(nick_hue(&v.nick)))
                                .child(v.nick.clone()),
                        )
                        .children(
                            v.display_name
                                .clone()
                                .filter(|n| *n != v.nick)
                                .map(|n| div().truncate().text_size(px(12.5)).text_color(p.muted_foreground).child(n)),
                        )
                        .children(v.user_host.clone().map(|u| div().mt(px(2.)).font_family(MONO).text_size(px(11.5)).text_color(p.muted_foreground).child(u))),
                ),
        )
        .when(has_meta || v.realname.is_some(), |d| {
            d.child(
                v_flex()
                    .gap(px(5.25))
                    .px(px(6.))
                    .pb(px(6.))
                    .when(has_meta, |d| {
                        d.child(
                            h_flex()
                                .flex_wrap()
                                .gap(px(3.5))
                                .when(!v.prefix.is_empty(), |d| {
                                    d.child(
                                        div()
                                            .px(px(6.))
                                            .h(px(18.))
                                            .flex()
                                            .items_center()
                                            .rounded(px(6.))
                                            .bg(p.secondary)
                                            .font_family(MONO)
                                            .text_size(px(11.))
                                            .child(v.prefix.clone()),
                                    )
                                })
                                .children(v.online.map(|on| badge(if on { "online" } else { "offline" }, if on { p.ok } else { p.muted_foreground }, p.border)))
                                .when(v.away, |d| d.child(badge("away", p.warn, p.border)))
                                .when(v.ignored, |d| d.child(badge("ignored", p.warn, p.border)))
                                .children(v.account.clone().map(|a| badge(&format!("signed in as {a}"), p.muted_foreground, p.border))),
                        )
                    })
                    .children(v.realname.clone().map(|r| div().text_size(px(12.5)).text_color(p.foreground.opacity(0.8)).child(r))),
            )
        })
        .into_any_element()
}

pub fn nick_menu(menu: PopupMenu, v: &NickMenuView, dispatch: &Dispatch, cx: &mut App) -> PopupMenu {
    let fg = cx.hearth().foreground;
    let item = |label: &str, icon_name: &str, action: UiAction| {
        let d = dispatch.clone();
        PopupMenuItem::new(label.to_string()).icon(icon(icon_name, 15., fg)).on_click(move |_, window, cx| d(action.clone(), window, cx))
    };
    let nick = v.nick.clone();
    let view = v.clone();
    let mut menu = menu.item(PopupMenuItem::element(move |_, cx| header(&view, cx))).separator();
    if !v.is_self {
        menu =
            menu.item(item("Message", "message-square", UiAction::OpenQuery(nick.clone()))).item(item("Mention", "at-sign", UiAction::Mention(nick.clone())));
    }
    menu = menu.item(item("Whois", "info", UiAction::Whois(nick.clone())));
    if !v.is_self && v.can_monitor {
        menu = menu.item(item(
            if v.watched { "Stop watching" } else { "Tell me when online" },
            if v.watched { "bell-off" } else { "bell" },
            UiAction::Watch(nick.clone(), !v.watched),
        ));
    }
    if !v.is_self {
        menu = menu.item(item(
            if v.ignored { "Stop ignoring" } else { "Ignore" },
            if v.ignored { "eye" } else { "eye-off" },
            UiAction::Ignore(nick.clone(), !v.ignored),
        ));
    }
    if v.can_moderate && !v.is_self {
        menu = menu
            .separator()
            .item(item(
                if v.is_op { "Remove operator" } else { "Make operator" },
                if v.is_op { "shield-off" } else { "shield" },
                UiAction::ChannelMode { nick: nick.clone(), mode: 'o', on: !v.is_op },
            ))
            .item(item(
                if v.is_voice { "Remove voice" } else { "Give voice" },
                if v.is_voice { "mic-off" } else { "mic" },
                UiAction::ChannelMode { nick: nick.clone(), mode: 'v', on: !v.is_voice },
            ))
            .item(item("Kick", "user-x", UiAction::Kick(nick)));
    }
    menu
}
