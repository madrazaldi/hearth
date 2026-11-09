//! mIRC-formatted text as one GPUI text element: styled runs, with links,
//! channel names and nicks clickable. Built as a single `StyledText` so it
//! wraps like a paragraph, which div-per-segment layouts cannot do. The text
//! joins the window's text selection, so it can be drag-selected and copied.

use super::data::{Dispatch, UiAction};
use super::prelude::*;
use super::theme::{HearthTheme as _, MONO, Palette, SANS, nick_hue};
use gpui_kit::base::{TextSelection, TextSelectionHandle, TextSelectionRegistration, TextSelectionRun, Theme};
use hearth_model::format::{Span, parse_formatting, strip_formatting};
use std::ops::Range;

#[derive(Clone, Debug)]
pub enum Click {
    Url(String),
    Channel(String),
    Nick(String),
}

/// Accumulates text and per-range styles; `finish` turns it into an element.
pub struct RichText {
    text: String,
    runs: Vec<TextRun>,
    clicks: Vec<(Range<usize>, Click)>,
    base: Hsla,
}

impl RichText {
    pub fn new(base: Hsla) -> Self {
        RichText { text: String::new(), runs: Vec::new(), clicks: Vec::new(), base }
    }

    fn base_run(&self) -> TextRun {
        TextRun { len: 0, font: font(SANS), color: self.base, background_color: None, underline: None, strikethrough: None }
    }

    /// Appends `text` with the base style adjusted by `style`.
    pub fn push(&mut self, text: &str, style: impl FnOnce(&mut TextRun)) -> Range<usize> {
        let start = self.text.len();
        if text.is_empty() {
            return start..start;
        }
        let mut run = self.base_run();
        style(&mut run);
        run.len = text.len();
        self.text.push_str(text);
        self.runs.push(run);
        start..self.text.len()
    }

    pub fn plain(&mut self, text: &str) {
        self.push(text, |_| {});
    }

    pub fn nick(&mut self, nick: &str, p: &Palette, color_nicks: bool) {
        let color = if color_nicks { p.nick(nick_hue(nick)) } else { p.text_2 };
        let r = self.push(nick, |r| {
            r.color = color;
            r.font.weight = FontWeight::MEDIUM;
        });
        self.clicks.push((r, Click::Nick(nick.to_string())));
    }

    /// Formatted message text.
    pub fn segments(&mut self, segs: &[Span], p: &Palette) {
        for s in segs {
            let range = self.push(&s.text, |r| {
                if let Some(c) = s.fg {
                    r.color = p.irc_color(c);
                }
                r.background_color = s.bg.map(|c| p.irc_color(c));
                if s.bold {
                    r.font.weight = FontWeight::SEMIBOLD;
                }
                if s.italic {
                    r.font.style = FontStyle::Italic;
                }
                if s.mono {
                    r.font = font(MONO);
                }
                if s.underline {
                    r.underline = Some(UnderlineStyle { thickness: px(1.), color: Some(r.color), wavy: false });
                }
                if s.strike {
                    r.strikethrough = Some(StrikethroughStyle { thickness: px(1.), color: Some(r.color) });
                }
                if s.href.is_some() {
                    r.color = p.brand;
                }
            });
            if let Some(href) = &s.href {
                self.clicks.push((range, Click::Url(href.clone())));
            } else if let Some(ch) = &s.channel {
                self.clicks.push((range.clone(), Click::Channel(ch.clone())));
            }
        }
    }

    /// `order` places the text among the other selectable texts in the
    /// window: a selection dragged across several covers the ones between.
    pub fn finish(self, id: impl Into<ElementId>, order: u64, dispatch: Dispatch) -> AnyElement {
        let id = id.into();
        let text = SharedString::from(self.text);
        let styled = StyledText::new(text.clone()).with_runs(self.runs);
        let layout = styled.layout().clone();
        let inner = if self.clicks.is_empty() {
            Inner::Plain(styled)
        } else {
            let ranges: Vec<Range<usize>> = self.clicks.iter().map(|c| c.0.clone()).collect();
            let actions: Vec<Click> = self.clicks.into_iter().map(|c| c.1).collect();
            Inner::Interactive(InteractiveText::new(id.clone(), styled).on_click(ranges, move |ix, window, cx| {
                // Ending a drag on the link it started on selects; it doesn't open it.
                if !TextSelection::selected_text(window, cx).is_empty() {
                    return;
                }
                let action = match &actions[ix] {
                    Click::Url(u) => UiAction::OpenUrl(u.clone()),
                    Click::Channel(c) => UiAction::JoinChannel(c.clone()),
                    Click::Nick(n) => UiAction::OpenNick(n.clone()),
                };
                dispatch(action, window, cx);
            }))
        };
        Selectable { id, text, layout, inner, order }.into_any_element()
    }
}

/// Plain formatted message text in the surrounding colour.
pub fn message_text(text: &str, base: Hsla, id: impl Into<ElementId>, order: u64, dispatch: Dispatch, cx: &App) -> AnyElement {
    let mut rt = RichText::new(base);
    rt.segments(&parse_formatting(text), cx.hearth());
    rt.finish(id, order, dispatch)
}

enum Inner {
    Plain(StyledText),
    Interactive(InteractiveText),
}

/// Styled text that takes part in the window's text selection, like
/// gpui-base's `SelectableText`, which only takes an unstyled string.
struct Selectable {
    id: ElementId,
    text: SharedString,
    layout: TextLayout,
    inner: Inner,
    order: u64,
}

impl IntoElement for Selectable {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Selectable {
    type RequestLayoutState = TextSelectionHandle;
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, TextSelectionHandle) {
        let handle = window.with_element_state(id.expect("Selectable has an id"), |kept: Option<TextSelectionHandle>, _| {
            let handle = kept.unwrap_or_else(|| TextSelectionHandle::new(self.text.to_string(), cx));
            (handle.clone(), handle)
        });
        let (layout_id, ()) = match &mut self.inner {
            Inner::Plain(t) => t.request_layout(id, inspector_id, window, cx),
            Inner::Interactive(t) => t.request_layout(id, inspector_id, window, cx),
        };
        (layout_id, handle)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut TextSelectionHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        let hitbox = match &mut self.inner {
            Inner::Plain(t) => {
                t.prepaint(id, inspector_id, bounds, &mut (), window, cx);
                window.insert_hitbox(bounds, HitboxBehavior::Normal)
            }
            Inner::Interactive(t) => t.prepaint(id, inspector_id, bounds, &mut (), window, cx),
        };
        let registration = TextSelectionRegistration::new(hitbox.clone(), bounds)
            .with_document_order(self.order)
            .with_text_bounds(vec![bounds])
            .with_rendered_element(handle, window, cx);
        handle.register(registration, window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        handle: &mut TextSelectionHandle,
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let run = TextSelectionRun::new(self.text.clone(), self.layout.clone(), bounds).with_document_order(self.order);
        let projection = handle.update_runs(&[run], cx);
        let color = Theme::global(cx).tokens.colors.selection;
        for range in projection.ranges().iter().flatten() {
            paint_selection(&self.layout, range.clone(), color, window);
        }
        match &mut self.inner {
            Inner::Plain(t) => t.paint(id, inspector_id, bounds, &mut (), &mut (), window, cx),
            Inner::Interactive(t) => t.paint(id, inspector_id, bounds, &mut (), hitbox, window, cx),
        }
    }
}

/// The highlight behind `range`, one quad per wrapped line it spans.
fn paint_selection(layout: &TextLayout, range: Range<usize>, color: Hsla, window: &mut Window) {
    let (Some(start), Some(end)) = (layout.position_for_index(range.start), layout.position_for_index(range.end)) else { return };
    let (bounds, lh) = (layout.bounds(), layout.line_height());
    let mut quads = Vec::new();
    if start.y == end.y {
        quads.push(Bounds::from_corners(start, point(end.x, end.y + lh)));
    } else {
        quads.push(Bounds::from_corners(start, point(bounds.right(), start.y + lh)));
        if end.y > start.y + lh {
            quads.push(Bounds::from_corners(point(bounds.left(), start.y + lh), point(bounds.right(), end.y)));
        }
        quads.push(Bounds::from_corners(point(bounds.left(), end.y), point(end.x, end.y + lh)));
    }
    for q in quads {
        window.paint_quad(fill(q, color));
    }
}

/// Topic and similar one-liners: formatting stripped, no links.
pub fn plain(text: &str) -> String {
    strip_formatting(text)
}
