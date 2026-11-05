//! The virtualized message list: the model's timeline (day and "New" dividers,
//! grouped senders, collapsed join/part runs, history gaps) drawn in the three
//! row layouts (cozy, columns, compact).
//!
//! Rows are variable height: GPUI's `list` measures what it renders and keeps
//! the rest estimated, so a 10k-line buffer only ever lays out the viewport
//! plus an overdraw band. The lines themselves stay in the model; a row reads
//! its line from the host while it is painted.

use super::avatar::{Avatar, icon};
use super::bind::avatar_pic;
use super::cards;
use super::data::*;
use super::prelude::*;
use super::rich_text::{RichText, message_text};
use super::theme::{HearthTheme as _, MONO, Palette, SANS, nick_hue};
use super::widgets::{icon_button, tag};
use crate::host::images::Ask;
use crate::host::{host, request_flush};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use hearth_model::format::{parse_formatting, strip_formatting};
use hearth_model::history::{GapFrom, HistMode};
use hearth_model::media::{self, fit_box, media_label};
use hearth_model::rows::{EventPart, RowKind, event_glyph, event_parts, row_kind, summarize_run};
use hearth_model::state::PreviewState;
use hearth_model::timeline::Item;
use hearth_model::types::{Layout, Line, LineType, Settings};
use std::collections::HashSet;
use std::rc::Rc;

const OVERDRAW: f32 = 1000.;
const THUMB_W: f32 = 360.;
const THUMB_H: f32 = 280.;
const REACTIONS: [&str; 8] = ["👍", "❤️", "😂", "🎉", "😮", "😢", "👀", "✅"];
/// Rows from the top at which older history is requested.
const NEAR_TOP: usize = 6;

/// The slice of `Settings` the list reads.
#[derive(Clone, Copy, PartialEq)]
pub struct MessageSettings {
    pub layout: Layout,
    pub font_size: f32,
    pub nick_colors: bool,
    pub link_previews: bool,
    pub inline_images: bool,
    pub load_avatars: bool,
    /// Animations play without hovering.
    pub autoplay: bool,
}

impl From<&Settings> for MessageSettings {
    fn from(s: &Settings) -> Self {
        MessageSettings {
            layout: s.layout,
            font_size: s.font_size as f32,
            nick_colors: s.nick_colors,
            link_previews: s.link_previews,
            inline_images: s.inline_images,
            load_avatars: s.load_avatars,
            autoplay: s.autoplay_animations,
        }
    }
}

// ── The list view ───────────────────────────────────────────

pub struct MessageList {
    state: ListState,
    view: Option<Rc<MessagesView>>,
    settings: MessageSettings,
    expanded: HashSet<String>,
    /// MOTD blocks whose open state differs from their default.
    flipped: HashSet<u64>,
    /// NSFW images the user chose to show.
    revealed: HashSet<u64>,
    /// The line whose animated image is under the pointer, so it plays.
    playing: Option<u64>,
    dispatch: Dispatch,
}

impl MessageList {
    pub fn new(dispatch: Dispatch, cx: &mut Context<Self>) -> Self {
        let state = ListState::new(0, ListAlignment::Top, px(OVERDRAW));
        state.set_follow_mode(FollowMode::Tail);
        let weak = cx.weak_entity();
        // Scrolling changes whether the "Latest" pill shows, and when the top or the bottom is
        // reached the model hears of it: it loads older lines and marks the buffer read.
        state.set_scroll_handler(move |ev, _, cx| {
            let weak = weak.clone();
            let near_top = ev.visible_range.start < NEAR_TOP;
            let at_bottom = ev.is_following_tail || ev.visible_range.end + 1 >= ev.count;
            cx.defer(move |cx| {
                let Some(list) = weak.upgrade() else { return };
                let shown = list.read(cx).view.as_ref().map(|v| (v.buffer.clone(), v.filtered()));
                list.update(cx, |_, cx| cx.notify());
                if let Some((id, filtering)) = shown {
                    crate::host::hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.viewport_changed(&id, at_bottom, near_top, filtering)));
                }
            });
        });
        MessageList {
            state,
            view: None,
            settings: MessageSettings::from(&Settings::default()),
            expanded: HashSet::new(),
            flipped: HashSet::new(),
            revealed: HashSet::new(),
            playing: None,
            dispatch,
        }
    }

    /// Bind new data. The rows that did not change keep their measured heights and the
    /// scroll position; a different buffer resets and jumps to the newest line.
    pub fn set_view(&mut self, view: MessagesView, settings: MessageSettings, cx: &mut Context<Self>) {
        // A new filter is a different list: start again from its end.
        let same_buffer = self.view.as_ref().is_some_and(|v| v.buffer == view.buffer);
        let same_list = same_buffer && self.view.as_ref().is_some_and(|v| v.filter == view.filter);
        if !same_list {
            self.state.reset(view.timeline.items.len());
            self.state.set_follow_mode(FollowMode::Tail);
            self.state.scroll_to_end();
            if !same_buffer {
                self.expanded.clear();
                self.flipped.clear();
                self.revealed.clear();
            }
        } else if let Some(old) = &self.view {
            if self.settings != settings {
                self.state.remeasure();
            }
            if !std::sync::Arc::ptr_eq(&old.timeline, &view.timeline) {
                let (a, b) = (&old.timeline.items, &view.timeline.items);
                let mut prefix = a.iter().zip(b.iter()).take_while(|(x, y)| x.key() == y.key()).count();
                let max_suffix = a.len().min(b.len()) - prefix;
                let suffix = a.iter().rev().zip(b.iter().rev()).take(max_suffix).take_while(|(x, y)| x.key() == y.key()).count();
                // The row before an appended one may have gained a group member; measure it again.
                if suffix == 0 && prefix > 0 {
                    prefix -= 1;
                }
                self.state.splice(prefix..a.len() - suffix, b.len() - suffix - prefix);
            }
        }
        self.view = Some(Rc::new(view));
        self.settings = settings;
        cx.notify();
    }

    /// How many rows the list has, after the filter.
    pub fn row_count(&self) -> usize {
        self.view.as_ref().map_or(0, |v| v.timeline.items.len())
    }

    /// Bring the row at `item` into view, with one row of context above it.
    pub fn scroll_to_item(&mut self, item: usize, cx: &mut Context<Self>) {
        self.state.set_follow_mode(FollowMode::Normal);
        self.state.scroll_to(ListOffset { item_ix: item.saturating_sub(1), offset_in_item: px(0.) });
        cx.notify();
    }

    pub fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.state.set_follow_mode(FollowMode::Tail);
        self.state.scroll_to_end();
        cx.notify();
    }

    /// Scroll up `rows` from the end, which also shows the "Latest" pill.
    pub fn scroll_up_by_rows(&mut self, rows: usize, cx: &mut Context<Self>) {
        let n = self.state.item_count().saturating_sub(rows);
        self.state.set_follow_mode(FollowMode::Normal);
        self.state.scroll_to(ListOffset { item_ix: n, offset_in_item: px(0.) });
        cx.notify();
    }

    fn scrolled_up(&self) -> bool {
        self.state.max_offset_for_scrollbar().y > px(0.) && !self.state.is_following_tail() && !self.state.is_scrolled_to_end().unwrap_or(false)
    }

    fn item_of_line(&self, id: u64, cx: &App) -> Option<usize> {
        let view = self.view.as_ref()?;
        let b = host(cx).app.buffer(&view.buffer)?;
        view.timeline.items.iter().position(|it| matches!(it, Item::Line { line, .. } if b.lines().get(*line).is_some_and(|l| l.id == id)))
    }

    fn toggle_motd(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.flipped.remove(&id) {
            self.flipped.insert(id);
        }
        if let Some(ix) = self.item_of_line(id, cx) {
            self.state.remeasure_items(ix..ix + 1);
        }
        cx.notify();
    }

    /// The pointer entered or left the animated image of line `id`.
    fn hover_image(&mut self, id: u64, over: bool, cx: &mut Context<Self>) {
        let next = if over { Some(id) } else { self.playing.filter(|p| *p != id) };
        if next != self.playing {
            self.playing = next;
            cx.notify();
        }
    }

    fn reveal(&mut self, id: u64, cx: &mut Context<Self>) {
        self.revealed.insert(id);
        if let Some(ix) = self.item_of_line(id, cx) {
            self.state.remeasure_items(ix..ix + 1);
        }
        cx.notify();
    }

    fn toggle_run(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.expanded.remove(&key) {
            self.expanded.insert(key.clone());
        }
        if let Some(ix) = self.view.as_ref().and_then(|v| v.timeline.index_of_key(&key)) {
            self.state.remeasure_items(ix..ix + 1);
        }
        cx.notify();
    }
}

impl Render for MessageList {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let Some(view) = self.view.clone() else { return div().flex_1().into_any_element() };
        let ctx = Rc::new(RowCtx {
            view: view.clone(),
            st: self.settings,
            dispatch: self.dispatch.clone(),
            expanded: self.expanded.clone(),
            flipped: self.flipped.clone(),
            revealed: self.revealed.clone(),
            playing: self.playing,
            state: self.state.clone(),
            this: cx.weak_entity(),
        });
        let list = list(self.state.clone(), move |ix, window, cx| render_item(ix, &ctx, window, cx)).size_full().min_h_0().pb(px(6.));
        let show_latest = self.scrolled_up();
        let none_match = view.filtered() && view.timeline.items.is_empty();
        let unread = view.unread;
        let this = cx.entity();

        div()
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(div().size_full().child(list).vertical_scrollbar(&self.state))
            .when(none_match, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(13.))
                        .text_color(p.muted_foreground)
                        .child("No messages match"),
                )
            })
            .when(show_latest, |d| {
                let label = if unread > 0 { format!("{unread} new") } else { "Latest".to_string() };
                let (bg, fg, border) = if unread > 0 { (p.brand, p.brand_text, gpui_kit::transparent_black()) } else { (p.popover, p.foreground, p.input) };
                d.child(
                    div().absolute().left_0().right_0().bottom(px(12.)).flex().justify_center().child(
                        h_flex()
                            .id("latest")
                            .h(px(32.))
                            .pl(px(12.))
                            .pr(px(14.))
                            .gap(px(6.))
                            .items_center()
                            .rounded_full()
                            .border_1()
                            .border_color(border)
                            .bg(bg)
                            .text_color(fg)
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .shadow_lg()
                            .cursor_pointer()
                            .on_click(move |_, _, cx| this.update(cx, |l, cx| l.scroll_to_end(cx)))
                            .child(icon("arrow-down", 14., fg))
                            .child(label),
                    ),
                )
            })
            .into_any_element()
    }
}

// ── Rows ────────────────────────────────────────────────────

struct RowCtx {
    view: Rc<MessagesView>,
    st: MessageSettings,
    dispatch: Dispatch,
    expanded: HashSet<String>,
    flipped: HashSet<u64>,
    revealed: HashSet<u64>,
    playing: Option<u64>,
    state: ListState,
    this: WeakEntity<MessageList>,
}

/// What a message row needs beyond the line, read from the model while painting.
struct RowData {
    line: Line,
    clock: String,
    full_time: String,
    display_name: Option<String>,
    picture: Pic,
    reply_to: Option<(String, String)>,
    preview: Option<PreviewView>,
    can_reply: bool,
    can_delete: bool,
}

/// A link preview as the row draws it.
enum PreviewView {
    Page { url: String, site: Option<String>, title: Option<String>, description: Option<String>, image: Option<ImageView> },
    Image { url: String, image: ImageView, nsfw: bool },
    File { url: String, name: String, kind: String },
}

fn image_of(h: &crate::host::Hearth, key: &str, data: &str, w: Option<u32>, hgt: Option<u32>, animated: bool) -> Option<ImageView> {
    let src = h.images.get(key, data)?;
    let still = animated.then(|| h.images.still(key, data)).flatten();
    Some(ImageView { src, animated, still, width: w.unwrap_or(0) as f32, height: hgt.unwrap_or(0) as f32 })
}

fn preview_of(h: &crate::host::Hearth, url: &str, st: &MessageSettings, nsfw: bool) -> Option<PreviewView> {
    let p = match h.app.preview(url) {
        PreviewState::Loading => return None,
        PreviewState::Unavailable => {
            h.images.ask(Ask::Preview(url.to_string()));
            return None;
        }
        PreviewState::Ready(p) => p,
    };
    let key = format!("p:{url}");
    match p.kind.as_str() {
        "image" if st.inline_images && p.image.is_some() => {
            let image = image_of(h, &key, p.image.as_deref()?, p.width, p.height, p.animated)?;
            Some(PreviewView::Image { url: url.to_string(), image, nsfw })
        }
        "image" | "media" => Some(PreviewView::File { url: url.to_string(), name: media::file_name(url), kind: media_label(&p) }),
        "page" => Some(PreviewView::Page {
            url: url.to_string(),
            site: p.site.clone(),
            title: p.title.clone(),
            description: p.description.clone(),
            image: p.image.as_deref().and_then(|d| image_of(h, &key, d, p.width, p.height, false)),
        }),
        _ => None,
    }
}

fn row_data(c: &RowCtx, line: &Line, cx: &App) -> RowData {
    let h = host(cx);
    let id = &c.view.buffer;
    let info = h.app.row_info(id, line);
    let preview = match (&info.preview_url, c.st.link_previews && !line.redacted) {
        (Some(url), true) => preview_of(h, url, &c.st, info.nsfw),
        _ => None,
    };
    let picture = if c.st.load_avatars { avatar_pic(h, info.avatar.clone()) } else { Pic(None) };
    RowData {
        line: line.clone(),
        clock: info.clock,
        full_time: info.full_time,
        display_name: info.display_name,
        picture,
        reply_to: info.reply_to,
        preview,
        can_reply: h.app.can_reply(id, line),
        can_delete: h.app.can_redact(id, line),
    }
}

fn render_item(ix: usize, c: &RowCtx, window: &mut Window, cx: &mut App) -> AnyElement {
    let p = cx.hearth().clone();
    let Some(item) = c.view.timeline.items.get(ix) else { return div().into_any_element() };
    let el = match item {
        Item::Day { label, .. } => day_divider(label, &p).into_any_element(),
        Item::Unread { .. } => unread_divider(&p).into_any_element(),
        Item::Gap { gap, .. } => gap_row(ix, *gap, c, &p, cx),
        Item::Run { key, lines, .. } => run_row(key, lines, c, &p, cx),
        Item::Line { line: li, cont, .. } => {
            let Some(line) = host(cx).app.buffer(&c.view.buffer).and_then(|b| b.lines().get(*li)).cloned() else { return div().into_any_element() };
            match row_kind(&line) {
                RowKind::Whois => match host(cx).app.whois_card(&line) {
                    Some(card) => {
                        let clock = host(cx).app.clock(line.time);
                        block_row(&line, clock, cards::whois_card(&line, &card, c.st.nick_colors, &c.dispatch, &p), c, &p)
                    }
                    None => event_row(&line, c, &p, cx),
                },
                RowKind::Block => {
                    let clock = host(cx).app.clock(line.time);
                    let total = host(cx).app.buffer(&c.view.buffer).map_or(0, |b| b.lines().len());
                    let open = cards::motd_default_open(&line, total) != c.flipped.contains(&line.id);
                    let (this, id) = (c.this.clone(), line.id);
                    let card = cards::motd_block(
                        &line,
                        open,
                        move |_, cx| {
                            let _ = this.update(cx, |l, cx| l.toggle_motd(id, cx));
                        },
                        &p,
                    );
                    block_row(&line, clock, card, c, &p)
                }
                RowKind::Talk => {
                    let data = row_data(c, &line, cx);
                    message_row(data, *cont, c, &p, window, cx)
                }
                RowKind::Event => event_row(&line, c, &p, cx),
            }
        }
    };
    if crate::host::host(cx).images.pending() {
        request_flush(cx);
    }
    el
}

/// A card in the timeline: the row's time on the left, the card where the text would be.
fn block_row(l: &Line, clock: String, card: AnyElement, c: &RowCtx, p: &Palette) -> AnyElement {
    let lead = match c.st.layout {
        Layout::Cozy => div().w(px(40.)).flex_none(),
        _ => mono_time(clock, 11., p).w(px(56.)).pt(px(3.)),
    };
    let _ = l;
    h_flex()
        .w_full()
        .items_start()
        .gap(px(12.))
        .pl(px(16.))
        .pr(px(20.))
        .py(px(5.))
        .text_size(px(c.st.font_size))
        .child(lead)
        .child(div().flex_1().min_w_0().child(card))
        .into_any_element()
}

fn day_divider(label: &str, p: &Palette) -> impl IntoElement {
    // Padding rather than margins: list rows are measured as given, and w_full
    // keeps the lines from collapsing to the pill's width.
    div().w_full().pt(px(20.)).pb(px(6.)).pl(px(16.)).pr(px(20.)).child(
        h_flex()
            .w_full()
            .gap(px(12.))
            .items_center()
            .text_size(px(11.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.muted_foreground)
            .child(div().flex_1().h(px(1.)).bg(p.border))
            .child(
                div()
                    .flex_none()
                    .px(px(10.))
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .border_color(p.border)
                    .bg(p.background)
                    .child(label.to_string()),
            )
            .child(div().flex_1().h(px(1.)).bg(p.border)),
    )
}

fn unread_divider(p: &Palette) -> impl IntoElement {
    div().w_full().py(px(10.)).child(
        div().relative().mt(px(4.)).ml(px(16.)).mr(px(20.)).h(px(1.)).bg(p.brand.opacity(0.65)).child(
            div()
                .absolute()
                .right_0()
                .top(px(-9.))
                .px(px(8.))
                .h(px(18.))
                .flex()
                .items_center()
                .rounded_full()
                .bg(p.brand)
                .text_color(p.brand_text)
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child("New messages"),
        ),
    )
}

/// The stretch of history that is not loaded. Showing it asks the model to fill it, from the
/// edge nearer the reader so the lines grow away from the view.
fn gap_row(ix: usize, gap: hearth_model::buffer::Gap, c: &RowCtx, p: &Palette, cx: &mut App) -> AnyElement {
    let can = host(cx).app.can_fill_gap(&c.view.buffer);
    if can {
        let id = c.view.buffer.clone();
        let from = if ix > c.state.logical_scroll_top().item_ix + 3 { GapFrom::After } else { GapFrom::Before };
        let entity = crate::host::hearth(cx);
        cx.defer(move |cx| entity.update(cx, |h, cx| h.act(cx, |app| app.fill_gap(&id, gap, from))));
    }
    h_flex()
        .w_full()
        .gap(px(10.))
        .items_center()
        .pl(px(16.))
        .pr(px(20.))
        .pt(px(14.))
        .pb(px(12.))
        .border_y_1()
        .border_color(p.border)
        .text_size(px(13.))
        .text_color(p.muted_foreground)
        .child(if can { "Loading messages" } else { "Messages here aren't loaded. Reconnect to load them." })
        .into_any_element()
}

fn mono_time(text: String, size: f32, p: &Palette) -> Div {
    div().flex_none().font_family(MONO).text_size(px(size)).text_color(p.muted_foreground).whitespace_nowrap().child(text)
}

/// Shared row chrome: hover tint, mention tint with an accent edge, the flash after a jump.
fn shell(id: impl Into<ElementId>, group: &str, highlight: bool, flash: bool, p: &Palette, c: &RowCtx) -> Stateful<Div> {
    div()
        .id(id)
        .group(group.to_string())
        .relative()
        .w_full()
        .text_size(px(c.st.font_size))
        .line_height(relative(1.5))
        .font_family(SANS)
        .text_color(p.foreground)
        .when(highlight, |d| d.bg(p.mention))
        .when(flash, |d| d.bg(p.brand.opacity(0.18)))
        .hover({
            let bg = if highlight { p.mention.blend(p.muted.opacity(0.5)) } else { p.muted.opacity(0.5) };
            move |s| s.bg(bg)
        })
        .when(highlight, |d| d.child(div().absolute().left_0().top_0().bottom_0().w(px(3.)).bg(p.brand)))
}

fn nick_el(nick: &str, line_id: u64, bold: bool, c: &RowCtx, p: &Palette) -> Stateful<Div> {
    let color = if c.st.nick_colors { p.nick(nick_hue(nick)) } else { p.foreground };
    let d = c.dispatch.clone();
    let n = nick.to_string();
    div()
        .id(SharedString::from(format!("nick-{line_id}-{nick}")))
        .cursor_pointer()
        .text_color(color)
        .font_weight(if bold { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
        .hover(|s| s.underline())
        .on_click(move |_, w, cx| d(UiAction::OpenNick(n.clone()), w, cx))
        .child(nick.to_string())
}

fn message_row(data: RowData, cont: bool, c: &RowCtx, p: &Palette, window: &mut Window, cx: &mut App) -> AnyElement {
    let RowData { line: l, clock, full_time, display_name, picture, reply_to, preview, can_reply, can_delete } = data;
    let group = format!("row{}", l.id);
    let action = l.kind == LineType::Action;
    let layout = c.st.layout;
    let base = match l.kind {
        LineType::Notice | LineType::Action => p.text_2,
        _ => p.foreground,
    };

    // The message body, including the leading "* nick " of an action.
    let body = {
        let mut rt = RichText::new(base);
        if l.redacted {
            rt.push("message deleted", |r| {
                r.color = p.muted_foreground;
                r.font.style = FontStyle::Italic;
            });
        } else {
            if action {
                rt.push("*", |r| {
                    r.font = font(MONO);
                    r.color = p.muted_foreground;
                });
                rt.plain(" ");
                rt.nick(&l.nick, p, c.st.nick_colors);
                rt.plain(" ");
            }
            let mut segs = parse_formatting(&l.text);
            if action {
                segs.iter_mut().for_each(|s| s.italic = true);
            }
            rt.segments(&segs, p);
        }
        rt.finish(SharedString::from(format!("body-{}", l.id)), c.dispatch.clone())
    };

    let quote = l.reply_to.as_ref().filter(|_| !l.redacted).map(|_| {
        h_flex()
            .max_w_full()
            .mt(px(2.))
            .mb(px(3.))
            .py(px(1.))
            .px(px(8.))
            .gap(px(6.))
            .items_baseline()
            .overflow_hidden()
            .border_l_2()
            .border_color(p.input)
            .text_size(px(c.st.font_size * 0.86))
            .text_color(p.muted_foreground)
            .child(div().child("↪"))
            .map(|d| match &reply_to {
                Some((nick, text)) => d
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if c.st.nick_colors { p.nick(nick_hue(nick)) } else { p.text_2 })
                            .child(nick.clone()),
                    )
                    .child(div().min_w_0().truncate().child(snippet(text))),
                None => d.child(div().text_color(p.muted_foreground.opacity(0.7)).child("a message that isn't loaded")),
            })
    });

    let reactions = (!l.redacted && !l.reactions.is_empty()).then(|| {
        h_flex().flex_wrap().gap(px(4.)).mt(px(3.)).mb(px(2.)).children(l.reactions.iter().map(|(emoji, nicks)| {
            let mine = nicks.iter().any(|n| *n == c.view.me);
            let (d, id, e) = (c.dispatch.clone(), l.id, emoji.clone());
            h_flex()
                .id(SharedString::from(format!("react-{}-{}", l.id, emoji)))
                .h(px(24.))
                .px(px(8.))
                .gap(px(5.))
                .items_center()
                .rounded(px(12.))
                .border_1()
                .border_color(if mine { p.brand.opacity(0.45) } else { p.border })
                .bg(if mine { p.brand_soft } else { p.muted.opacity(0.45) })
                .text_size(px(13.))
                .when(!mine && c.view.can_react, |d| {
                    let (bg, bd) = (p.muted, p.input);
                    d.cursor_pointer().hover(move |s| s.bg(bg).border_color(bd))
                })
                .on_click(move |_, w, cx| {
                    if !mine {
                        d(UiAction::React(id, e.clone()), w, cx)
                    }
                })
                .child(emoji.clone())
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if mine { p.brand } else { p.muted_foreground })
                        .child(nicks.len().to_string()),
                )
        }))
    });

    let revealed = c.revealed.contains(&l.id);
    let preview = preview.map(|pv| preview_el(&pv, l.id, revealed, c, p));

    let head_nick = |bold_prefix: bool| {
        h_flex()
            .items_baseline()
            .gap(px(8.))
            .line_height(relative(1.35))
            .mb(px(1.))
            .when(layout == Layout::Compact && c.st.load_avatars, |d| d.child(div().self_center().child(Avatar::new(l.nick.clone(), 18.).picture(&picture))))
            .child(
                h_flex()
                    .when_some(l.prefix.clone().filter(|s| !s.is_empty() && bold_prefix), |d, pre| {
                        d.child(div().text_color(p.muted_foreground).font_weight(FontWeight::MEDIUM).child(pre))
                    })
                    .child(nick_el(&l.nick, l.id, true, c, p)),
            )
            .when_some(display_name.clone().filter(|n| *n != l.nick), |d, n| {
                d.child(div().text_size(px(c.st.font_size * 0.9)).text_color(p.muted_foreground).child(n))
            })
            .when(l.kind == LineType::Notice, |d| d.child(tag("notice", cx)))
            .child(mono_time(clock.clone(), 11., p))
    };

    let content = v_flex()
        .flex_1()
        .min_w_0()
        .when(!cont && !action && layout != Layout::Columns, |d| d.child(head_nick(true)))
        .children(quote)
        .child(body)
        .children(reactions)
        .children(preview);

    let flash = c.view.flash == Some(l.id);
    let tip = SharedString::from(full_time);
    let row = shell(("line", l.id), &group, l.highlight, flash, p, c).tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx));
    let row = match layout {
        Layout::Cozy => row.pl(px(16.)).pr(px(20.)).pt(px(if cont { 1. } else { 10. })).pb(px(if cont { 1. } else { 2. })).child(
            h_flex()
                .gap(px(12.))
                .items_start()
                .child(div().flex_none().w(px(36.)).flex().justify_center().pt(px(2.)).child(if cont || action {
                    mono_time(clock.clone(), 10.5, p).pt(px(3.)).invisible().group_hover(group.clone(), |s| s.visible()).into_any_element()
                } else {
                    let d = c.dispatch.clone();
                    let n = l.nick.clone();
                    div()
                        .id(SharedString::from(format!("av-{}", l.id)))
                        .cursor_pointer()
                        .on_click(move |_, w, cx| d(UiAction::OpenNick(n.clone()), w, cx))
                        .child(Avatar::new(l.nick.clone(), 36.).picture(&picture))
                        .into_any_element()
                }))
                .child(content),
        ),
        Layout::Columns => row.pl(px(14.)).pr(px(20.)).pt(px(if cont { 0. } else { 3. })).pb(px(1.)).child(
            h_flex()
                .gap(px(12.))
                .items_start()
                .child(
                    div()
                        .flex_none()
                        .w(px(52.))
                        .flex()
                        .justify_end()
                        .pt(px(2.))
                        .child(mono_time(clock.clone(), 11., p).when(cont, |d| d.invisible().group_hover(group.clone(), |s| s.visible()))),
                )
                .child(div().flex_none().w(px(132.)).flex().justify_end().overflow_hidden().map(|d| {
                    if action {
                        d.child(div().font_family(MONO).text_size(px(12.)).text_color(p.muted_foreground).child("*"))
                    } else if cont {
                        d
                    } else {
                        d.child(
                            h_flex()
                                .when(l.kind == LineType::Notice, |d| d.child(div().mr(px(4.)).text_color(p.muted_foreground).child("–")))
                                .when_some(l.prefix.clone().filter(|s| !s.is_empty()), |d, pre| {
                                    d.child(div().text_color(p.muted_foreground).font_weight(FontWeight::MEDIUM).child(pre))
                                })
                                .child(nick_el(&l.nick, l.id, true, c, p)),
                        )
                    }
                }))
                .child(content),
        ),
        Layout::Compact => row.pl(px(14.)).pr(px(20.)).pt(px(if cont { 0. } else { 10. })).pb(px(1.)).child(
            h_flex()
                .gap(px(12.))
                .items_start()
                .child(
                    div()
                        .flex_none()
                        .w(px(52.))
                        .flex()
                        .justify_end()
                        .pt(px(if cont || action { 2. } else { c.st.font_size * 1.75 }))
                        .child(mono_time(clock.clone(), 11., p).invisible().group_hover(group.clone(), |s| s.visible())),
                )
                .child(content),
        ),
    };

    row.child(tools(&l, can_reply, can_delete, c, p, &group, window, cx)).into_any_element()
}

fn snippet(text: &str) -> String {
    text.chars().take(140).collect()
}

fn preview_el(pv: &PreviewView, line_id: u64, revealed: bool, c: &RowCtx, p: &Palette) -> AnyElement {
    let d = c.dispatch.clone();
    match pv {
        PreviewView::Page { url, site, title, description, image } => {
            let url = url.clone();
            h_flex()
                .id(SharedString::from(format!("card-{line_id}")))
                .mt(px(6.))
                .mb(px(4.))
                .max_w(px(520.))
                .gap(px(12.))
                .py(px(12.))
                .px(px(14.))
                .items_start()
                .rounded(px(10.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .overflow_hidden()
                .cursor_pointer()
                .hover({
                    let bg = p.card.blend(p.muted.opacity(0.2));
                    move |s| s.bg(bg)
                })
                .on_click(move |_, w, cx| d(UiAction::OpenUrl(url.clone()), w, cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .children(site.clone().map(|s| div().text_size(px(11.5)).text_color(p.muted_foreground).child(s)))
                        .children(title.clone().map(|t| div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(p.foreground).child(t)))
                        .children(description.clone().map(|t| div().text_size(px(12.5)).text_color(p.muted_foreground).line_clamp(3).child(t))),
                )
                .children(image.as_ref().map(|im| image_box(im, 88., 88., 8.)))
                .into_any_element()
        }
        PreviewView::Image { url, image, nsfw } => {
            let (w, h) = fit(image.width, image.height, THUMB_W, THUMB_H);
            let (url, full) = (url.clone(), image.clone());
            let veiled = *nsfw && !revealed;
            let this = c.this.clone();
            // An animation rests on its first frame until it is hovered (or autoplay is on).
            let resting = image.animated && !veiled && !c.st.autoplay && c.playing != Some(line_id);
            let shown = match (&image.still, resting) {
                (Some(still), true) => ImageView { src: still.clone(), ..image.clone() },
                _ => image.clone(),
            };
            let hover_to = c.this.clone();
            div()
                .id(SharedString::from(format!("img-{line_id}")))
                .relative()
                .mt(px(6.))
                .mb(px(4.))
                .w(px(w))
                .cursor_pointer()
                .when(image.animated && !veiled, |d| {
                    d.on_hover(move |over, _, cx| {
                        let _ = hover_to.update(cx, |l, cx| l.hover_image(line_id, *over, cx));
                    })
                })
                .on_click(move |_, win, cx| {
                    if veiled {
                        let _ = this.update(cx, |l, cx| l.reveal(line_id, cx));
                    } else {
                        d(UiAction::OpenImage { url: url.clone(), image: full.clone() }, win, cx)
                    }
                })
                .child(if veiled {
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(w))
                        .h(px(h))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(p.border)
                        .bg(p.muted)
                        .text_size(px(12.))
                        .text_color(p.muted_foreground)
                        .child("NSFW \u{b7} click to show")
                        .into_any_element()
                } else {
                    image_box(&shown, w, h, 10.)
                })
                .when(resting && image.still.is_some(), |d| {
                    d.child(
                        div()
                            .absolute()
                            .left(px(8.))
                            .bottom(px(8.))
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(6.))
                            .bg(gpui_kit::black().opacity(0.6))
                            .text_size(px(10.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(gpui_kit::white())
                            .child("GIF"),
                    )
                })
                .into_any_element()
        }
        PreviewView::File { url, name, kind } => {
            let url = url.clone();
            h_flex()
                .mt(px(6.))
                .mb(px(4.))
                .min_w(px(260.))
                .max_w(px(520.))
                .gap(px(10.))
                .py(px(8.))
                .pr(px(8.))
                .pl(px(10.))
                .items_center()
                .rounded(px(10.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .child(div().flex().flex_none().items_center().justify_center().size(px(32.)).rounded(px(8.)).bg(p.muted).child(icon(
                    "play",
                    16.,
                    p.muted_foreground,
                )))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().truncate().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child(name.clone()))
                        .child(div().text_size(px(11.5)).text_color(p.muted_foreground).child(kind.clone())),
                )
                .child(
                    Button::new(SharedString::from(format!("open-{line_id}")))
                        .small()
                        .outline()
                        .icon(icon("external-link", 14., p.foreground))
                        .label("Open")
                        .on_click(move |_, w, cx| d(UiAction::OpenUrl(url.clone()), w, cx)),
                )
                .into_any_element()
        }
    }
}

/// Scale down (never up) to fit, keeping the aspect ratio.
pub fn fit(w: f32, h: f32, max_w: f32, max_h: f32) -> (f32, f32) {
    let b = media::fit_box_in(w as u32, h as u32, max_w as u32, max_h as u32);
    (b.width as f32, b.height as f32)
}

/// The size the thumbnail of an image occupies.
pub fn thumb(im: &ImageView) -> (f32, f32) {
    let b = fit_box(im.width as u32, im.height as u32);
    (b.width as f32, b.height as f32)
}

/// A loaded picture, cropped to fill its box.
pub fn image_box(im: &ImageView, w: f32, h: f32, radius: f32) -> AnyElement {
    img(im.src.clone()).w(px(w)).h(px(h)).rounded(px(radius)).object_fit(ObjectFit::Cover).into_any_element()
}

// ── Events ──────────────────────────────────────────────────

/// An event sentence as one wrapped text element, plus buttons it carries.
fn event_text(l: &Line, c: &RowCtx, p: &Palette, id: SharedString) -> AnyElement {
    let muted = p.muted_foreground;
    let mut rt = RichText::new(if l.kind == LineType::Error { p.destructive } else { muted });
    let nicks = c.st.nick_colors;
    let mut join: Option<String> = None;
    for part in event_parts(l) {
        match part {
            EventPart::Nick(n) => rt.nick(&n, p, nicks),
            EventPart::Text(t) => rt.plain(&t),
            EventPart::Faint(t) => {
                rt.push(&t, |r| r.color = muted.opacity(0.8));
            }
            EventPart::Mono(t) => {
                rt.push(&t, |r| r.font = font(MONO));
            }
            EventPart::Quote(t) | EventPart::Rich(t) => rt.segments(&parse_formatting(&t), p),
            EventPart::JoinButton(channel) => join = Some(channel),
        }
    }
    let text = rt.finish(id, c.dispatch.clone());
    match join {
        None => text,
        Some(channel) => {
            let d = c.dispatch.clone();
            h_flex()
                .gap(px(8.))
                .items_center()
                .child(text)
                .child(
                    Button::new(SharedString::from(format!("join-{}", l.id)))
                        .xsmall()
                        .outline()
                        .label("Join")
                        .on_click(move |_, w, cx| d(UiAction::JoinChannel(channel.clone()), w, cx)),
                )
                .into_any_element()
        }
    }
}

fn event_row(l: &Line, c: &RowCtx, p: &Palette, cx: &App) -> AnyElement {
    let group = format!("row{}", l.id);
    let clock = host(cx).app.clock(l.time);
    let error = l.kind == LineType::Error;
    let gcolor = if error { p.destructive } else { p.muted_foreground };
    let g = || div().font_family(MONO).text_size(px(12.)).text_color(gcolor).child(event_glyph(l.kind));
    let text = div().flex_1().min_w_0().text_size(px(c.st.font_size * 0.9)).child(event_text(l, c, p, SharedString::from(format!("ev-{}", l.id))));
    let row = shell(("line", l.id), &group, false, c.view.flash == Some(l.id), p, c).py(px(2.));
    match c.st.layout {
        Layout::Cozy => row
            .pl(px(16.))
            .pr(px(20.))
            .child(
                h_flex()
                    .gap(px(12.))
                    .items_start()
                    .child(div().flex_none().w(px(36.)).flex().justify_center().child(g()))
                    .child(text)
                    .child(mono_time(clock, 11., p).invisible().group_hover(group.clone(), |s| s.visible())),
            )
            .into_any_element(),
        Layout::Columns => row
            .pl(px(14.))
            .pr(px(20.))
            .child(
                h_flex()
                    .gap(px(12.))
                    .items_start()
                    .child(div().flex_none().w(px(52.)).flex().justify_end().child(mono_time(clock, 11., p)))
                    .child(div().flex_none().w(px(132.)).flex().justify_end().child(g()))
                    .child(text),
            )
            .into_any_element(),
        Layout::Compact => row
            .pl(px(14.))
            .pr(px(20.))
            .child(
                h_flex()
                    .gap(px(12.))
                    .items_start()
                    .child(div().flex_none().w(px(52.)).flex().justify_end().child(mono_time(clock, 11., p)))
                    .child(h_flex().gap(px(8.)).child(g()).child(text)),
            )
            .into_any_element(),
    }
}

fn run_row(key: &str, lines: &[usize], c: &RowCtx, p: &Palette, cx: &App) -> AnyElement {
    let h = host(cx);
    let Some(b) = h.app.buffer(&c.view.buffer) else { return div().into_any_element() };
    let all: Vec<Line> = lines.iter().filter_map(|&i| b.lines().get(i).cloned()).collect();
    let Some(first) = all.first() else { return div().into_any_element() };
    let open = c.expanded.contains(key);
    let group = format!("run{}", first.id);
    let this = c.this.clone();
    let k = key.to_string();
    let summary = summarize_run(&all.iter().collect::<Vec<_>>());
    let toggle = h_flex()
        .id(SharedString::from(format!("run-{key}")))
        .gap(px(4.))
        .items_center()
        .cursor_pointer()
        .text_color(p.muted_foreground)
        .hover({
            let fg = p.foreground;
            move |s| s.text_color(fg)
        })
        .on_click(move |_, _, cx| {
            let k = k.clone();
            let _ = this.update(cx, |l, cx| l.toggle_run(k, cx));
        })
        .child(summary)
        .child(icon(if open { "chevron-down" } else { "chevron-right" }, 11., p.muted_foreground));

    let details = open.then(|| {
        v_flex().mt(px(4.)).mb(px(2.)).gap(px(1.)).children(all.iter().map(|l| {
            h_flex()
                .gap(px(8.))
                .items_baseline()
                .child(mono_time(h.app.clock(l.time), 11., p))
                .child(div().font_family(MONO).text_size(px(12.)).text_color(p.muted_foreground).child(event_glyph(l.kind)))
                .child(div().child(event_text(l, c, p, SharedString::from(format!("run-ev-{}", l.id)))))
        }))
    });
    let clock = h.app.clock(first.time);
    let text = v_flex().flex_1().min_w_0().text_size(px(c.st.font_size * 0.9)).child(toggle).children(details);
    let g = || div().font_family(MONO).text_size(px(12.)).text_color(p.muted_foreground).child("⇅");
    let row = shell(("run", first.id), &group, false, false, p, c).py(px(2.));
    match c.st.layout {
        Layout::Cozy => row
            .pl(px(16.))
            .pr(px(20.))
            .child(
                h_flex()
                    .gap(px(12.))
                    .items_start()
                    .child(div().flex_none().w(px(36.)).flex().justify_center().child(g()))
                    .child(text)
                    .child(mono_time(clock, 11., p).invisible().group_hover(group.clone(), |s| s.visible())),
            )
            .into_any_element(),
        Layout::Columns => row
            .pl(px(14.))
            .pr(px(20.))
            .child(
                h_flex()
                    .gap(px(12.))
                    .items_start()
                    .child(div().flex_none().w(px(52.)).flex().justify_end().child(mono_time(clock, 11., p)))
                    .child(div().flex_none().w(px(132.)).flex().justify_end().child(g()))
                    .child(text),
            )
            .into_any_element(),
        Layout::Compact => row
            .pl(px(14.))
            .pr(px(20.))
            .child(h_flex().gap(px(12.)).items_start().child(div().flex_none().w(px(52.)).flex().justify_end().child(mono_time(clock, 11., p))).child(text))
            .into_any_element(),
    }
}

// ── Hover tools ─────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn tools(l: &Line, can_reply: bool, can_delete: bool, c: &RowCtx, p: &Palette, group: &str, _: &mut Window, _: &mut App) -> impl IntoElement {
    let id = l.id;
    let muted = p.muted_foreground;
    let hover = p.muted;
    let btn = |name: &'static str, icon_name: &'static str, tip: &'static str, action: UiAction| {
        let d = c.dispatch.clone();
        icon_button(SharedString::from(format!("{name}-{id}")), icon_name, (24., 14.), muted, hover, Some(tip), move |w, cx| d(action.clone(), w, cx))
    };
    let react = (c.view.can_react && l.msgid.is_some() && !l.redacted).then(|| {
        let d = c.dispatch.clone();
        Button::new(SharedString::from(format!("react-btn-{id}"))).ghost().xsmall().icon(icon("smile", 14., muted)).dropdown_menu(move |mut m, _, _| {
            for e in REACTIONS {
                let d = d.clone();
                m = m.item(PopupMenuItem::new(e).on_click(move |_, w, cx| d(UiAction::React(id, e.to_string()), w, cx)));
            }
            m
        })
    });
    h_flex()
        .absolute()
        .top(px(-14.))
        .right(px(16.))
        .gap(px(1.))
        .p(px(2.))
        .items_center()
        .rounded(px(8.))
        .border_1()
        .border_color(p.border)
        .bg(p.popover)
        .shadow_sm()
        .invisible()
        .group_hover(group.to_string(), |s| s.visible())
        .when(can_reply, |d| d.child(btn("reply", "reply", "Reply", UiAction::Reply(id))))
        .children(react)
        .when(!l.redacted, |d| d.child(btn("copy", "copy", "Copy text", UiAction::CopyLine(id))))
        .when(can_delete, |d| d.child(btn("del", "trash-2", "Delete", UiAction::DeleteLine(id))))
}

/// Plain message text in the surrounding colour (kept for callers outside the list).
#[allow(dead_code)]
fn plain_message(text: &str, base: Hsla, id: impl Into<ElementId>, dispatch: Dispatch, cx: &App) -> AnyElement {
    let _ = (HistMode::Before, strip_formatting(text));
    message_text(text, base, id, dispatch, cx)
}
