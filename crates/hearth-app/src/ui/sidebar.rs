//! The left sidebar: app header, "Jump to…", networks with their buffers,
//! and the account footer.

use super::avatar::{Avatar, dot, icon};
use super::data::*;
use super::menus::{BufferFacts, buffer_menu};
use super::prelude::*;
use super::theme::{HearthTheme as _, Palette, nick_hue};
use super::widgets::{count_badge, icon_button, kbd_hint};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::DropdownMenu as _;
use hearth_model::rows::initial;

#[derive(IntoElement)]
pub struct Sidebar {
    view: SidebarView,
    dispatch: Dispatch,
    /// Room for the macOS traffic lights above the header.
    inset_top: bool,
}

impl Sidebar {
    pub fn new(view: SidebarView, dispatch: Dispatch) -> Self {
        Sidebar { view, dispatch, inset_top: cfg!(target_os = "macos") }
    }
}

fn state_color(state: ConnState, away: bool, p: &Palette) -> Hsla {
    if away {
        return p.warn;
    }
    match state {
        ConnState::Connected => p.ok.opacity(0.8),
        ConnState::Connecting => p.warn,
        ConnState::Disconnected => p.destructive,
    }
}

impl RenderOnce for Sidebar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = cx.hearth().clone();
        let Sidebar { view, dispatch, inset_top } = self;

        let mut nav = v_flex().id("sidebar-nav").flex_1().min_h_0().overflow_y_scroll().px(px(8.75)).pt(px(3.5)).pb(px(14.));
        let mut ix = 0usize;
        for net in &view.networks {
            nav = nav.child(network(net, &view, &dispatch, &p, &mut ix, cx));
        }

        v_flex()
            .size_full()
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .child(header(&view, &dispatch, &p, inset_top, cx))
            .child(nav)
            .when_some(view.footer.clone(), |d, f| d.child(footer(&f, &dispatch, &p)))
    }
}

fn header(view: &SidebarView, dispatch: &Dispatch, p: &Palette, inset_top: bool, cx: &App) -> impl IntoElement {
    let faint = p.sidebar_foreground.opacity(0.6);
    let (d1, d2, d3) = (dispatch.clone(), dispatch.clone(), dispatch.clone());
    v_flex()
        .gap(px(8.75))
        .px(px(8.75))
        .pb(px(7.))
        .pt(px(if inset_top { 35. } else { 8.75 }))
        .child(
            h_flex()
                .h(px(28.))
                .pl(px(3.5))
                .gap(px(7.))
                .items_center()
                .window_control_area(WindowControlArea::Drag)
                .child(img("logo.png").size(px(21.)).rounded(px(6.75)))
                .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child("Hearth"))
                .child(
                    h_flex()
                        .ml_auto()
                        .child(icon_button("search-messages", "list", (24.5, 15.), faint, p.sidebar_accent, Some("Search messages"), move |w, cx| {
                            d1(UiAction::OpenSearch, w, cx)
                        }))
                        .child(icon_button("add-network", "plus", (24.5, 15.), faint, p.sidebar_accent, Some("Add network"), move |w, cx| {
                            d2(UiAction::AddNetwork, w, cx)
                        })),
                ),
        )
        .child(
            h_flex()
                .id("jump")
                .h(px(28.))
                .items_center()
                .gap(px(7.))
                .pl(px(8.75))
                .pr(px(5.25))
                .rounded(px(8.75))
                .border_1()
                .border_color(p.sidebar_border)
                .bg(p.sidebar_accent.opacity(0.4))
                .cursor_pointer()
                .hover({
                    let bg = p.sidebar_accent;
                    move |s| s.bg(bg)
                })
                .on_click(move |_, w, cx| d3(UiAction::OpenSwitcher, w, cx))
                .child(icon("search", 14., p.sidebar_foreground.opacity(0.55)))
                .child(div().flex_1().text_size(px(13.)).text_color(p.sidebar_foreground.opacity(0.55)).child("Jump to…"))
                .child(kbd_hint(format!("{}K", view.mod_label), cx)),
        )
}

fn network(net: &NetworkGroup, view: &SidebarView, dispatch: &Dispatch, p: &Palette, ix: &mut usize, cx: &App) -> impl IntoElement {
    let active = view.active.as_ref() == Some(&net.server_buffer);
    let dot_color = state_color(net.state, false, p);
    let label_color = if net.state == ConnState::Disconnected { p.destructive } else { p.muted_foreground };
    let d = dispatch.clone();
    let server = net.server_buffer.clone();
    let id = net.id.clone();

    let row = if net.is_control {
        h_flex()
            .id(SharedString::from(format!("net-{id}")))
            .h(px(24.5))
            .items_center()
            .gap(px(5.25))
            .px(px(5.25))
            .rounded(px(6.75))
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(if active { p.sidebar_foreground } else { p.sidebar_foreground.opacity(0.5) })
            .cursor_pointer()
            .hover({
                let c = p.sidebar_foreground;
                move |s| s.text_color(c)
            })
            .on_click(move |_, w, cx| d(UiAction::OpenBuffer(server.clone()), w, cx))
            .child(icon("layers", 12., p.sidebar_foreground.opacity(0.5)))
            .child(div().truncate().child(net.name.to_uppercase()))
            .child(dot(6., dot_color, None))
            .when(!net.status_label.is_empty(), |d| d.child(div().ml_auto().text_size(px(11.)).text_color(label_color).child(net.status_label.clone())))
    } else {
        let hue = nick_hue(&net.name);
        let hl: usize = if net.collapsed { net.buffers.iter().map(|b| b.highlights).sum() } else { 0 };
        let group = SharedString::from(format!("netrow-{id}"));
        let toggle = {
            let (d, id) = (dispatch.clone(), net.id.clone());
            let collapsed = net.collapsed;
            div()
                .id(SharedString::from(format!("fold-{id}")))
                .flex()
                .items_center()
                .justify_center()
                .flex_none()
                .size(px(19.25))
                .rounded(px(6.75))
                .text_color(p.muted_foreground.opacity(0.7))
                .hover({
                    let (bg, fg) = (p.sidebar_border, p.foreground);
                    move |s| s.bg(bg).text_color(fg)
                })
                .when(!collapsed, |d| d.invisible().group_hover(group.clone(), |s| s.visible()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, w, cx| {
                    cx.stop_propagation();
                    d(UiAction::ToggleNetwork(id.clone()), w, cx)
                })
                .child(icon(if collapsed { "chevron-right" } else { "chevron-down" }, 13., p.muted_foreground.opacity(0.9)))
        };
        h_flex()
            .id(SharedString::from(format!("net-{id}")))
            .group(group.clone())
            .h(px(28.))
            .mt(px(10.5))
            .min_w_0()
            .items_center()
            .gap(px(7.))
            .pl(px(5.25))
            .pr(px(3.5))
            .rounded(px(6.75))
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.sidebar_foreground.opacity(0.85))
            .cursor_pointer()
            .when(active, |d| d.bg(p.sidebar_accent).text_color(p.foreground))
            .when(!active, |d| {
                let (bg, fg) = (p.sidebar_accent.opacity(0.7), p.sidebar_foreground);
                d.hover(move |s| s.bg(bg).text_color(fg))
            })
            .on_click(move |_, w, cx| d(UiAction::OpenBuffer(server.clone()), w, cx))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(17.5))
                    .rounded(px(5.))
                    .bg(p.tile(hue, p.sidebar_accent, 0.22))
                    .text_color(p.nick(hue))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::BOLD)
                    .line_height(relative(1.0))
                    .child(initial(&net.name))
                    .child(div().absolute().right(px(-2.6)).bottom(px(-2.6)).child(dot(7., dot_color, Some(p.sidebar)))),
            )
            .child(div().truncate().child(net.name.clone()))
            .when(!net.status_label.is_empty(), |d| {
                d.child(div().ml_auto().flex_none().text_size(px(11.)).font_weight(FontWeight::NORMAL).text_color(label_color).child(net.status_label.clone()))
            })
            .when(hl > 0, |d| d.child(div().when(net.status_label.is_empty(), |d| d.ml_auto()).child(count_badge(hl, cx))))
            .child(div().when(net.status_label.is_empty() && hl == 0, |d| d.ml_auto()).child(toggle))
    };

    let mut col = v_flex().child(row);
    if !net.collapsed || net.is_control {
        let mut list = v_flex().mt(px(1.75)).gap(px(1.));
        for b in &net.buffers {
            *ix += 1;
            list = list.child(buffer_row(b, *ix, view, dispatch, p, cx));
        }
        col = col.child(list);
    }
    col
}

fn buffer_row(b: &BufferRow, ix: usize, view: &SidebarView, dispatch: &Dispatch, p: &Palette, cx: &App) -> impl IntoElement {
    let active = view.active.as_ref() == Some(&b.id);
    let unread = b.unread > 0 && !b.muted();
    let fg = p.sidebar_foreground;
    let group = SharedString::from(format!("buf-{ix}"));
    let (d, open_id) = (dispatch.clone(), b.id.clone());

    let glyph = if b.kind == BufferKind::Channel {
        div()
            .flex_none()
            .size(px(17.5))
            .flex()
            .items_center()
            .justify_center()
            .child(icon(
                "hash",
                14.,
                if active {
                    p.brand
                } else if unread {
                    fg.opacity(0.8)
                } else {
                    fg.opacity(0.45)
                },
            ))
            .into_any_element()
    } else {
        div()
            .relative()
            .flex_none()
            .size(px(17.5))
            .flex()
            .items_center()
            .justify_center()
            .child(Avatar::new(b.name.clone(), 18.).picture(&b.picture))
            .when_some(b.online, |d, on| {
                d.child(div().absolute().right(px(-1.)).bottom(px(-1.)).child(dot(
                    7.,
                    if on { p.ok } else { p.muted_foreground.opacity(0.6) },
                    Some(p.sidebar),
                )))
            })
            .into_any_element()
    };

    let more = {
        let (dispatch, id) = (dispatch.clone(), b.id.clone());
        let (kind, joined, level) = (b.kind, b.joined, b.level);
        div().invisible().group_hover(group.clone(), |s| s.visible()).on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).child(
            Button::new(("buf-more", ix))
                .ghost()
                .xsmall()
                .icon(icon("ellipsis", 14., p.muted_foreground))
                .dropdown_menu(move |m, w, cx| buffer_menu(m, &id, BufferFacts { kind, joined, level }, &dispatch, w, cx)),
        )
    };

    h_flex()
        .id(("buf", ix))
        .group(group.clone())
        .h(px(26.25))
        .w_full()
        .items_center()
        .gap(px(7.))
        .pl(px(5.25))
        .pr(px(3.5))
        .rounded(px(6.75))
        .text_size(px(13.5))
        .text_color(if unread || active { fg } else { fg.opacity(0.62) })
        .when(unread, |d| d.font_weight(FontWeight::SEMIBOLD))
        .when(active, |d| d.bg(p.sidebar_accent).font_weight(FontWeight::MEDIUM).text_color(p.foreground))
        .when(b.muted(), |d| d.opacity(0.55))
        .when(!active, |d| {
            let (bg, fg) = (p.sidebar_accent.opacity(0.7), fg);
            d.hover(move |s| s.bg(bg).text_color(fg))
        })
        .cursor_pointer()
        .on_click(move |_, w, cx| d(UiAction::OpenBuffer(open_id.clone()), w, cx))
        .child(glyph)
        .child({
            let name = if b.kind == BufferKind::Channel { b.name.trim_start_matches('#').to_string() } else { b.name.clone() };
            div().flex_1().min_w_0().truncate().when(b.kind == BufferKind::Channel && !b.joined, |d| d.italic().opacity(0.55)).child(name)
        })
        .child(
            div()
                .relative()
                .flex_none()
                .h(px(19.25))
                .min_w(px(19.25))
                .child(div().h_full().flex().items_center().justify_end().group_hover(group.clone(), |s| s.invisible()).child(trailing(b, p, cx)))
                .child(div().absolute().top_0().right_0().child(more)),
        )
}

/// Mention badge, mute marker, or typing dots; hidden while the row is hovered.
fn trailing(b: &BufferRow, p: &Palette, cx: &App) -> AnyElement {
    if b.muted() {
        div().pr(px(4.)).child(icon("bell-off", 12., p.muted_foreground.opacity(0.7))).into_any_element()
    } else if b.highlights > 0 {
        count_badge(b.highlights, cx).into_any_element()
    } else if b.typing && b.kind == BufferKind::Query {
        h_flex().gap(px(2.)).pr(px(4.)).children((0..3).map(|_| div().size(px(3.)).rounded_full().bg(p.muted_foreground))).into_any_element()
    } else {
        div().into_any_element()
    }
}

fn footer(f: &FooterView, dispatch: &Dispatch, p: &Palette) -> impl IntoElement {
    let d = dispatch.clone();
    let d_settings = dispatch.clone();
    h_flex()
        .items_center()
        .gap(px(3.5))
        .p(px(7.))
        .border_t_1()
        .border_color(p.sidebar_border)
        .child(
            h_flex()
                .id("account")
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(px(8.75))
                .p(px(5.25))
                .rounded(px(8.75))
                .cursor_pointer()
                .hover({
                    let bg = p.sidebar_accent;
                    move |s| s.bg(bg)
                })
                .on_click(move |_, w, cx| d(UiAction::OpenSettings, w, cx))
                .child(
                    div().relative().flex_none().child(Avatar::new(f.nick.clone(), 30.)).child(div().absolute().right(px(-1.75)).bottom(px(-1.75)).child(dot(
                        8.75,
                        state_color(f.state, f.away, p),
                        Some(p.sidebar),
                    ))),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).line_height(relative(1.25)).child(f.nick.clone()))
                        .child(div().truncate().text_size(px(11.5)).line_height(relative(1.25)).text_color(p.muted_foreground).child(f.status_text.clone())),
                )
                .child(icon("chevrons-up-down", 14., p.muted_foreground)),
        )
        .child(icon_button("settings", "settings", (24.5, 15.), p.sidebar_foreground.opacity(0.6), p.sidebar_accent, Some("Settings"), move |w, cx| {
            d_settings(UiAction::OpenSettings, w, cx)
        }))
}
