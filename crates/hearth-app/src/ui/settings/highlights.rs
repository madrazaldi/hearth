//! Extra highlight rules for one server, with a live test.

use crate::host::host;
use crate::ui::controls::*;
use crate::ui::prelude::*;
use crate::ui::theme::HearthTheme as _;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use hearth_model::format::rule_error;
use hearth_model::panels::HighlightsForm;
use hearth_model::types::NetworkHighlights;

pub enum HighlightsEvent {
    /// `None` clears the server's rules.
    Save(String, Option<NetworkHighlights>),
}

impl EventEmitter<HighlightsEvent> for HighlightsEditor {}

pub struct HighlightsEditor {
    server: String,
    nick: String,
    global_words: Vec<String>,
    words: Entity<TextareaState>,
    never: Entity<InputState>,
    from: Entity<InputState>,
    sample: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl HighlightsEditor {
    pub fn new(server: String, nick: String, global_words: Vec<String>, initial: NetworkHighlights, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let words = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx).rows(3).placeholder("deploy\n/\\bci (failed|broke)\\b/i");
            s.set_value(initial.words.join("\n"), window, cx);
            s
        });
        let never = new_input(window, cx, &initial.never.join(", "), "ChanServ, *bot*, *!*@services.*");
        let from = new_input(window, cx, "", "from");
        let sample = new_input(window, cx, "", "Type a sample line");
        let mut subs = vec![];
        subs.push(cx.subscribe(&words, |this: &mut Self, _, ev: &InputEvent, cx| match ev {
            InputEvent::Change => cx.notify(),
            InputEvent::Blur => this.save(cx),
            _ => {}
        }));
        for s in [&never, &from, &sample] {
            let commit = *s == never;
            subs.push(cx.subscribe(s, move |this: &mut Self, _, ev: &InputEvent, cx| match ev {
                InputEvent::Change => cx.notify(),
                InputEvent::Blur | InputEvent::PressEnter { .. } if commit => this.save(cx),
                _ => {}
            }));
        }
        HighlightsEditor { server, nick, global_words, words, never, from, sample, _subs: subs }
    }

    pub fn set_global_words(&mut self, words: Vec<String>, cx: &mut Context<Self>) {
        self.global_words = words;
        cx.notify();
    }

    fn lines(&self, cx: &App) -> Vec<String> {
        self.words.read(cx).value().lines().map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect()
    }

    fn never_list(&self, cx: &App) -> Vec<String> {
        text(&self.never, cx).split(|c: char| c.is_whitespace() || c == ',').map(str::trim).filter(|n| !n.is_empty()).map(String::from).collect()
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let lines = self.lines(cx);
        // Keep what's saved until the edit is valid, rather than dropping the bad rule.
        if lines.iter().any(|w| rule_error(w).is_some()) {
            return;
        }
        let never = self.never_list(cx);
        let value = (!(lines.is_empty() && never.is_empty())).then_some(NetworkHighlights { words: lines, never });
        cx.emit(HighlightsEvent::Save(self.server.clone(), value));
    }
}

impl Render for HighlightsEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let lines = self.lines(cx);
        let errors: Vec<String> = lines.iter().filter_map(|w| rule_error(w).map(|e| format!("{w} — {e}"))).collect();
        let (sample, from) = (text(&self.sample, cx), text(&self.from, cx));
        let form = HighlightsForm { words: lines.join("\n"), never: text(&self.never, cx), sample: sample.clone(), sample_from: from.clone() };
        let result = host(cx).app.highlight_test(&self.server, &self.nick, &form).map(|t| (t.hit, t.why));
        v_flex()
            .gap(px(14.))
            .child(
                v_flex().gap(px(4.)).child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Highlights on this server")).child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.muted_foreground)
                        .child("Added to your global highlight words. For a bouncer, these apply to all of its networks."),
                ),
            )
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(label("Extra highlight words"))
                    .child(Textarea::new(&self.words).font_family(crate::ui::theme::MONO))
                    .children(errors.iter().map(|e| error_text(format!("Invalid pattern: {e}"), &p)))
                    .when(errors.is_empty(), |d| {
                        d.child(rich_hint("One per line. Words match whole-word, ignoring case. Wrap a regular expression in slashes: `/pattern/i`.", &p))
                    }),
            )
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(label("Never highlight from"))
                    .child(mono_input(&self.never))
                    .child(rich_hint("Nicks or `nick!user@host` masks, such as bots that repeat your name.", &p)),
            )
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(label("Test a message"))
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .child(div().w(px(120.)).flex_none().child(mono_input(&self.from)))
                            .child(div().flex_1().min_w_0().child(Input::new(&self.sample))),
                    )
                    .children(result.map(|(hit, why)| {
                        div()
                            .text_size(px(12.))
                            .text_color(if hit { p.brand } else { p.muted_foreground })
                            .child(format!("{} · {why}", if hit { "Highlights" } else { "Doesn’t highlight" }))
                    })),
            )
    }
}
