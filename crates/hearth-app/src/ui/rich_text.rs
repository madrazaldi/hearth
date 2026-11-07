//! mIRC-formatted text as one GPUI text element: styled runs, with links,
//! channel names and nicks clickable. Built as a single `StyledText` so it
//! wraps like a paragraph, which div-per-segment layouts cannot do.

use super::data::{Dispatch, UiAction};
use super::prelude::*;
use super::theme::{HearthTheme as _, MONO, Palette, SANS, nick_hue};
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

    pub fn finish(self, id: impl Into<ElementId>, dispatch: Dispatch) -> AnyElement {
        let styled = StyledText::new(self.text).with_runs(self.runs);
        if self.clicks.is_empty() {
            return styled.into_any_element();
        }
        let ranges: Vec<Range<usize>> = self.clicks.iter().map(|c| c.0.clone()).collect();
        let actions: Vec<Click> = self.clicks.into_iter().map(|c| c.1).collect();
        InteractiveText::new(id, styled)
            .on_click(ranges, move |ix, window, cx| {
                let action = match &actions[ix] {
                    Click::Url(u) => UiAction::OpenUrl(u.clone()),
                    Click::Channel(c) => UiAction::JoinChannel(c.clone()),
                    Click::Nick(n) => UiAction::OpenNick(n.clone()),
                };
                dispatch(action, window, cx);
            })
            .into_any_element()
    }
}

/// Plain formatted message text in the surrounding colour.
pub fn message_text(text: &str, base: Hsla, id: impl Into<ElementId>, dispatch: Dispatch, cx: &App) -> AnyElement {
    let mut rt = RichText::new(base);
    rt.segments(&parse_formatting(text), cx.hearth());
    rt.finish(id, dispatch)
}

/// Topic and similar one-liners: formatting stripped, no links.
pub fn plain(text: &str) -> String {
    strip_formatting(text)
}
