//! Form controls shared by the settings, onboarding and panels: segmented
//! control, switch rows, choice cards, labelled fields.

use super::avatar::icon;
use super::prelude::*;
use super::theme::{HearthTheme as _, MONO, Palette};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::switch::Switch;
use std::rc::Rc;

type Change<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

/// A single-line input holding `value`.
pub fn new_input<T: 'static>(window: &mut Window, cx: &mut Context<T>, value: &str, placeholder: &str) -> Entity<InputState> {
    let (value, placeholder) = (value.to_string(), placeholder.to_string());
    cx.new(|cx| {
        let mut s = InputState::new(window, cx).placeholder(placeholder);
        s.set_value(value, window, cx);
        s
    })
}

pub fn new_password<T: 'static>(window: &mut Window, cx: &mut Context<T>, value: &str, placeholder: &str) -> Entity<InputState> {
    let (value, placeholder) = (value.to_string(), placeholder.to_string());
    cx.new(|cx| {
        let mut s = InputState::new(window, cx).masked(true).placeholder(placeholder);
        s.set_value(value, window, cx);
        s
    })
}

pub fn text(state: &Entity<InputState>, cx: &App) -> String {
    state.read(cx).value().to_string()
}

/// A monospace text field, for hosts, masks and commands.
pub fn mono_input(state: &Entity<InputState>) -> Input {
    Input::new(state).font_family(MONO)
}

// ── Segmented ───────────────────────────────────────────────

pub fn segmented<T: Copy + PartialEq + 'static>(
    id: &str,
    options: &[(T, &str)],
    value: T,
    on_change: impl Fn(T, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let p = cx.hearth().clone();
    let on_change: Change<T> = Rc::new(on_change);
    h_flex().flex_none().self_start().p(px(3.)).gap(px(1.)).rounded(px(8.75)).bg(p.muted).children(options.iter().enumerate().map(|(i, (v, label))| {
        let on = *v == value;
        let (v, f) = (*v, on_change.clone());
        let active_bg = if p.dark { p.input.opacity(0.5) } else { p.background };
        h_flex()
            .id(SharedString::from(format!("{id}-{i}")))
            .h(px(24.5))
            .px(px(10.5))
            .items_center()
            .rounded(px(6.75))
            .text_size(px(13.))
            .cursor_pointer()
            .when(on, |d| d.bg(active_bg).text_color(p.foreground).shadow_xs())
            .when(!on, |d| {
                let fg = p.foreground;
                d.text_color(p.muted_foreground).hover(move |s| s.text_color(fg))
            })
            .on_click(move |_, w, cx| f(v, w, cx))
            .child(label.to_string())
    }))
}

// ── Rows ────────────────────────────────────────────────────

/// A card of rows separated by hairlines.
pub fn rows(children: Vec<AnyElement>, p: &Palette) -> Div {
    let n = children.len();
    v_flex().rounded(px(12.75)).border_1().border_color(p.border).bg(p.card).children(
        children.into_iter().enumerate().map(|(i, c)| div().px(px(14.)).py(px(12.25)).when(i + 1 < n, |d| d.border_b_1().border_color(p.border)).child(c)),
    )
}

/// Title and hint on the left, a control on the right.
pub fn setting_row(title: impl Into<SharedString>, hint: Option<&str>, control: impl IntoElement, p: &Palette) -> AnyElement {
    h_flex()
        .gap(px(14.))
        .items_center()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.75))
                .child(div().text_size(px(13.5)).child(title.into()))
                .children(hint.map(|h| div().text_size(px(10.5)).line_height(relative(1.4)).text_color(p.muted_foreground).child(h.to_string()))),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

pub fn switch_row(
    id: &str,
    title: &str,
    hint: Option<&str>,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
    p: &Palette,
) -> AnyElement {
    // A switch row pads itself by another 0.375rem top and bottom.
    div()
        .py(px(5.25))
        .child(setting_row(
            title.to_string(),
            hint,
            Switch::new(SharedString::from(id.to_string()))
                .checked(checked)
                .accessibility_label(title.to_string())
                .on_click(move |v, w, cx| on_change(*v, w, cx)),
            p,
        ))
        .into_any_element()
}

/// A bare switch with its label beside it (forms).
pub fn switch_inline(
    id: &str,
    title: &str,
    hint: Option<&str>,
    checked: bool,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
    p: &Palette,
) -> AnyElement {
    switch_row(id, title, hint, checked, on_change, p)
}

pub fn heading(title: &str, hint: Option<&str>, first: bool, p: &Palette) -> Div {
    v_flex()
        .gap(px(1.75))
        .mb(px(10.5))
        .when(!first, |d| d.mt(px(24.5)))
        .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
        .children(hint.map(|h| div().text_size(px(10.5)).line_height(relative(1.5)).text_color(p.muted_foreground).child(h.to_string())))
}

pub fn label(text: impl Into<SharedString>) -> Div {
    div().text_size(px(12.25)).font_weight(FontWeight::MEDIUM).child(text.into())
}

pub fn hint(text: impl Into<SharedString>, p: &Palette) -> Div {
    div().text_size(px(10.5)).line_height(relative(1.5)).text_color(p.muted_foreground).child(text.into())
}

pub fn error_text(text: impl Into<SharedString>, p: &Palette) -> Div {
    div().text_size(px(12.25)).text_color(p.destructive).child(text.into())
}

/// A labelled control with optional help beneath.
pub fn field(title: &str, control: impl IntoElement) -> Div {
    v_flex().gap(px(7.)).min_w_0().child(label(title.to_string())).child(control)
}

/// Inline code in help text.
pub fn code(text: &str, p: &Palette) -> Div {
    div().px(px(4.)).rounded(px(4.)).bg(p.muted).font_family(MONO).text_size(px(11.5)).text_color(p.foreground).child(text.to_string())
}

/// A paragraph mixing text and `code` runs: text between backticks is code.
pub fn rich_hint(text: &str, p: &Palette) -> Div {
    h_flex().flex_wrap().items_baseline().gap_x(px(3.)).text_size(px(10.5)).line_height(relative(1.6)).text_color(p.muted_foreground).children(
        text.split('`').enumerate().flat_map(|(i, part)| {
            if i % 2 == 1 {
                vec![code(part, p).into_any_element()]
            } else {
                // Each word separately so the run wraps inside the flex row.
                part.split_whitespace().map(|w| div().child(w.to_string()).into_any_element()).collect()
            }
        }),
    )
}

pub fn status_dot(color: Hsla) -> Div {
    div().flex_none().size(px(6.)).rounded_full().bg(color)
}

// ── Choice cards ────────────────────────────────────────────

fn radio(on: bool, p: &Palette) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(12.25))
        .rounded_full()
        .border_1()
        .border_color(if on { p.brand } else { p.input })
        .when(on, |d| d.bg(p.brand).child(div().size(px(5.25)).rounded_full().bg(p.brand_text)))
}

/// A card in the three-column grid of the settings page (`max-w-xl` is 504px, `gap-3` 10.5px).
pub const CARD_W: f32 = 161.;

pub fn choice_cards<T: Copy + PartialEq + 'static>(
    id: &str,
    options: Vec<(T, &str, AnyElement)>,
    value: T,
    on_change: impl Fn(T, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let p = cx.hearth().clone();
    let on_change: Change<T> = Rc::new(on_change);
    h_flex().gap(px(10.5)).children(options.into_iter().enumerate().map(|(i, (v, name, preview))| {
        let on = v == value;
        let f = on_change.clone();
        v_flex()
            .id(SharedString::from(format!("{id}-{i}")))
            .w(px(CARD_W))
            .gap(px(7.))
            .cursor_pointer()
            .group(format!("{id}-{i}"))
            .on_click(move |_, w, cx| f(v, w, cx))
            .child({
                let (brand, input) = (p.brand, p.input);
                // The selected card has a 1px border and a 1px ring. A box shadow would paint under
                // the card in GPUI, so the ring is a padded frame around it.
                div().rounded(px(9.75)).p(px(1.)).when(on, |d| d.bg(brand)).child(
                    div()
                        .rounded(px(8.75))
                        .p(px(3.5))
                        .overflow_hidden()
                        .bg(p.popover)
                        .border_1()
                        .border_color(if on { brand } else { p.border })
                        .when(!on, |d| d.group_hover(format!("{id}-{i}"), move |s| s.border_color(input)))
                        .child(preview),
                )
            })
            .child(
                h_flex()
                    .gap(px(5.25))
                    .px(px(1.75))
                    .items_center()
                    .text_size(px(13.))
                    .text_color(if on { p.foreground } else { p.muted_foreground })
                    .when(on, |d| d.font_weight(FontWeight::MEDIUM))
                    .child(radio(on, &p))
                    .child(name.to_string()),
            )
    }))
}

struct Mini {
    window: u32,
    panel: u32,
    line: u32,
    accent: u32,
}

const MINI_DARK: Mini = Mini { window: 0x151311, panel: 0x211e1c, line: 0x3b3633, accent: 0xe48a3e };
const MINI_LIGHT: Mini = Mini { window: 0xf2efeb, panel: 0xffffff, line: 0xe0dad3, accent: 0xd06a20 };

fn rgb(c: u32) -> Hsla {
    rgba((c << 8) | 0xff).into()
}

fn bar(color: Hsla, w: Option<f32>) -> Div {
    div().flex_none().h(px(4.)).rounded(px(2.)).bg(color).when_some(w, |d, w| d.w(relative(w))).when(w.is_none(), |d| d.w_full())
}

fn mini(m: &Mini, w: f32) -> Div {
    let (line, accent) = (rgb(m.line), rgb(m.accent));
    h_flex()
        .flex_none()
        .w(px(w))
        .h(px(72.))
        .p(px(4.))
        .gap(px(4.))
        .bg(rgb(m.window))
        .child(v_flex().w(relative(0.25)).gap(px(4.)).pt(px(4.)).child(bar(line, None)).child(bar(accent, None)).child(bar(line, None)).child(bar(line, None)))
        .child(
            v_flex()
                .flex_1()
                .p(px(6.))
                .gap(px(4.))
                .justify_end()
                .rounded(px(3.))
                .bg(rgb(m.panel))
                .border_1()
                .border_color(line.opacity(0.7))
                .child(bar(line, Some(0.75)))
                .child(bar(line, Some(0.5)))
                .child(div().h(px(9.)).rounded(px(3.)).border_1().border_color(line)),
        )
}

/// The theme choice previews: a miniature window; `System` is split along the diagonal.
pub fn theme_preview(which: &str) -> AnyElement {
    let w = CARD_W - 11.;
    let frame = |inner: Div| div().relative().w(px(w)).h(px(72.)).rounded(px(6.75)).overflow_hidden().child(inner).into_any_element();
    match which {
        "light" => frame(mini(&MINI_LIGHT, w)),
        "dark" => frame(mini(&MINI_DARK, w)),
        // The dark half is the lower-right triangle (a clip-path in the old app); GPUI has no
        // polygon clip, so it is built from thin bottom-anchored strips.
        _ => {
            const STRIPS: usize = 64;
            let step = w / STRIPS as f32;
            frame(div().relative().size_full().child(mini(&MINI_LIGHT, w)).children((0..STRIPS).map(|i| {
                let x = i as f32 * step;
                let h = 72. * (i as f32 + 0.5) / STRIPS as f32;
                div()
                    .absolute()
                    .left(px(x))
                    .bottom_0()
                    .w(px(step + 0.5))
                    .h(px(h))
                    .overflow_hidden()
                    .child(div().absolute().left(px(-x)).bottom_0().child(mini(&MINI_DARK, w)))
            })))
        }
    }
}

/// The message layout previews.
pub fn layout_preview(which: &str, p: &Palette) -> AnyElement {
    let w = CARD_W - 11.;
    let line = p.input;
    let nick = p.brand.opacity(0.7);
    let base = v_flex().w(px(w)).h(px(72.)).px(px(10.)).gap(px(6.)).justify_center().rounded(px(6.75)).bg(p.muted.opacity(0.5));
    let b = |color: Hsla, frac: f32| div().flex_none().h(px(4.)).w(relative(frac)).rounded(px(2.)).bg(color);
    match which {
        "cozy" => base
            .children((0..2).map(|k| {
                h_flex()
                    .gap(px(6.))
                    .items_start()
                    .child(div().flex_none().size(px(16.)).rounded_full().bg(line))
                    .child(v_flex().flex_1().gap(px(4.)).child(b(nick, 0.33)).child(b(line, if k == 1 { 0.5 } else { 0.83 })))
            }))
            .into_any_element(),
        "columns" => base
            .children((0..4).map(|k| {
                h_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(div().flex_none().w(px(10.)).h(px(4.)).rounded(px(2.)).bg(line.opacity(0.6)))
                    .child(div().flex_none().w(px(16.)).h(px(4.)).rounded(px(2.)).bg(nick))
                    .child(if k % 2 == 1 { b(line, 0.5) } else { b(line, 1.0).flex_1() })
            }))
            .into_any_element(),
        _ => base
            .children((0..2).map(|k| {
                v_flex()
                    .gap(px(4.))
                    .child(h_flex().gap(px(4.)).items_center().child(div().flex_none().size(px(8.)).rounded_full().bg(line)).child(b(nick, 0.25)))
                    .child(b(line, if k == 1 { 0.5 } else { 0.83 }))
            }))
            .into_any_element(),
    }
}

// ── Keycaps ─────────────────────────────────────────────────

pub fn keycap(text: &str, p: &Palette) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(17.5))
        .min_w(px(17.5))
        .px(px(3.5))
        .rounded(px(4.75))
        .bg(p.muted)
        .text_color(p.muted_foreground)
        .text_size(px(10.5))
        .font_weight(FontWeight::MEDIUM)
        .child(text.to_string())
}

/// "↑↓ navigate" style hint used in overlay footers.
pub fn footer_hint(keys: &[&str], label: &str, p: &Palette) -> Div {
    h_flex().gap(px(5.25)).items_center().children(keys.iter().map(|k| keycap(k, p))).child(label.to_string())
}

pub fn footer(children: Vec<AnyElement>, p: &Palette) -> Div {
    h_flex()
        .flex_none()
        .py(px(7.))
        .px(px(10.5))
        .gap(px(14.))
        .items_center()
        .border_t_1()
        .border_color(p.border)
        .text_size(px(10.5))
        .text_color(p.muted_foreground)
        .children(children)
}

/// A search-style input: icon, text, trailing slot. The kit's input draws the border and the
/// focus ring (the accent border plus its soft outer ring) like the old `InputGroup`.
pub fn search_box(state: &Entity<InputState>, trailing: Option<AnyElement>, p: &Palette) -> Input {
    Input::new(state).prefix(div().pl(px(2.)).child(icon("search", 14., p.muted_foreground))).when_some(trailing, |i, t| i.suffix(t))
}

/// A small caption (the old `.eyebrow`: 11px, medium, muted).
pub fn eyebrow(text: &str, p: &Palette) -> Div {
    div().text_size(px(11.)).font_weight(FontWeight::MEDIUM).text_color(p.muted_foreground).child(text.to_string())
}
