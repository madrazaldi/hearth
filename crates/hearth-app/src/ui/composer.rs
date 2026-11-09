//! The typing indicator and the composer: nick label, multi-line input, send
//! button, the reply banner and the command/emoji suggestion popup.
//!
//! The draft lives in the model (one per buffer, so switching buffers keeps
//! what was typed) and the text field only displays it: edits go to the model,
//! and when the model changes the text itself (history recall, completion,
//! an emoji pick, a sent message) the field is set to match.

use super::avatar::icon;
use super::data::*;
use super::prelude::*;
use super::theme::{HearthTheme as _, MONO, nick_hue};
use super::widgets::icon_button;
use crate::host::hearth;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Sizable as _, Size};
use hearth_model::composer::{ComposerKey, KeyOutcome, Suggestions};

pub struct Composer {
    input: Entity<TextareaState>,
    view: Option<ComposerView>,
    dispatch: Dispatch,
    _subs: Vec<Subscription>,
}

impl Composer {
    pub fn new(dispatch: Dispatch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 8).submit_on_enter(true).placeholder("Message"));
        let on_input = cx.subscribe_in(&input, window, |this: &mut Self, input, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.edited(input, cx);
            }
        });
        // Up, Down, Tab, Enter and Escape belong to the model while the box has focus: they pick
        // suggestions, recall history, complete nicks and send. An interceptor runs before the
        // field's own bindings, which would otherwise take the key first.
        let weak = cx.weak_entity();
        let on_key = cx.intercept_keystrokes(move |ev, window, cx| {
            let Some(this) = weak.upgrade() else { return };
            let k = &ev.keystroke;
            let m = &k.modifiers;
            // Cmd/Ctrl+V of a picture or files uploads them; text falls through to the field.
            let primary = if cfg!(target_os = "macos") { m.platform && !m.control } else { m.control && !m.platform };
            if primary && !m.alt && !m.shift && k.key == "v" {
                if this.update(cx, |c, cx| c.paste_files(window, cx)) {
                    cx.stop_propagation();
                }
                return;
            }
            if m.control || m.alt || m.platform || m.function {
                return;
            }
            let key = match k.key.as_str() {
                "up" => ComposerKey::Up,
                "down" => ComposerKey::Down,
                "tab" => ComposerKey::Tab,
                "enter" => ComposerKey::Enter,
                "escape" => ComposerKey::Escape,
                _ => return,
            };
            let handled = this.update(cx, |c, cx| c.key(key, m.shift, window, cx));
            if handled {
                cx.stop_propagation();
            }
        });
        Composer { input, view: None, dispatch, _subs: vec![on_input, on_key] }
    }

    /// Bind the buffer being composed in.
    pub fn set_view(&mut self, view: ComposerView, window: &mut Window, cx: &mut Context<Self>) {
        let same_buffer = self.view.as_ref().is_some_and(|v| v.buffer == view.buffer);
        if !same_buffer || self.view.as_ref().is_some_and(|v| v.placeholder != view.placeholder) {
            let ph = view.placeholder.clone();
            self.input.update(cx, |s, cx| s.set_placeholder(ph, window, cx));
        }
        let shown = self.input.read(cx).value();
        if shown.as_ref() != view.draft.as_str() {
            let (draft, caret) = (view.draft.clone(), view.caret);
            self.input.update(cx, |s, cx| {
                s.set_value(draft, window, cx);
                s.set_selected_range(caret..caret, cx);
            });
        }
        self.view = Some(view);
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.focus(window, cx));
    }

    /// Whether text is selected in the field, which Copy should take over the chat's.
    pub fn has_selection(&self, cx: &App) -> bool {
        !self.input.read(cx).selected_range().is_empty()
    }

    /// Drops the field's selection, keeping the caret where it was.
    pub fn collapse_selection(&mut self, cx: &mut Context<Self>) {
        let end = self.input.read(cx).selected_range().end;
        self.input.update(cx, |s, cx| s.set_selected_range(end..end, cx));
    }

    fn buffer(&self) -> Option<BufferId> {
        self.view.as_ref().map(|v| v.buffer.clone())
    }

    /// The field changed because the user typed or pasted: tell the model.
    fn edited(&mut self, input: &Entity<TextareaState>, cx: &mut Context<Self>) {
        let Some(id) = self.buffer() else { return };
        let (text, caret) = {
            let s = input.read(cx);
            (s.value().to_string(), s.cursor())
        };
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.composer_edit(&id, &text, caret)));
    }

    /// A key the model may want. True when it took it, so the field must not.
    fn key(&mut self, key: ComposerKey, shift: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let Some(id) = self.buffer() else { return false };
        let caret = self.input.read(cx).cursor();
        let outcome = hearth(cx).update(cx, |h, cx| {
            h.act(cx, |app| {
                // Arrow keys and clicks move the caret without an edit, and history recall and the
                // emoji popup depend on where it is. Moving it forgets a completion, so only when it moved.
                if app.composer(&id).is_some_and(|c| c.caret != caret) {
                    app.composer_set_caret(&id, caret);
                }
                app.composer_key(&id, key, shift)
            })
        });
        match outcome {
            KeyOutcome::Ignored => false,
            KeyOutcome::Blur => {
                window.blur(cx);
                true
            }
            KeyOutcome::Handled | KeyOutcome::Sent => true,
        }
    }

    /// A paste that carries a picture or files goes to the file host. True when it did, so the field must not paste.
    fn paste_files(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return false;
        }
        let (Some(id), Some(item)) = (self.buffer(), cx.read_from_clipboard()) else { return false };
        hearth(cx).update(cx, |h, cx| h.upload_clipboard(&id, &item, cx))
    }

    /// The nickname button fills the box with `/nick <current>` to edit.
    fn prompt_nick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.buffer() else { return };
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.composer_prompt_nick(&id)));
        self.focus(window, cx);
    }

    /// The paperclip: pick files, then upload them.
    pub fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.buffer() else { return };
        let chosen = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Upload".into()) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else { return };
            let _ = this.update_in(cx, |c, window, cx| {
                hearth(cx).update(cx, |h, cx| h.upload_paths(&id, &paths, cx));
                c.focus(window, cx);
            });
        })
        .detach();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.buffer() else { return };
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| app.composer_submit(&id)));
        self.focus(window, cx);
    }

    fn pick(&mut self, emoji: bool, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.buffer() else { return };
        hearth(cx).update(cx, |h, cx| h.act(cx, |app| if emoji { app.composer_pick_emoji(&id, index) } else { app.composer_pick_command(&id, index) }));
        self.focus(window, cx);
    }

    fn suggestions(&self, p: &super::theme::Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let view = self.view.as_ref()?;
        let this = cx.entity();
        let row = |ix: usize, active: bool, emoji: bool, lead: AnyElement, name: String, hint: String| {
            let this = this.clone();
            h_flex()
                .id(SharedString::from(format!("sg-{emoji}-{ix}")))
                .gap(px(10.))
                .px(px(10.))
                .h(px(30.))
                .items_center()
                .rounded(px(8.))
                .cursor_pointer()
                .when(active, |d| d.bg(p.accent))
                .hover({
                    let bg = p.accent;
                    move |s| s.bg(bg)
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(move |_, window, cx| this.update(cx, |c, cx| c.pick(emoji, ix, window, cx)))
                .child(lead)
                .child(div().flex_none().font_weight(FontWeight::MEDIUM).text_size(px(13.)).child(name))
                .child(div().flex_1().min_w_0().truncate().text_size(px(12.)).text_color(p.muted_foreground).child(hint))
        };
        let rows: Vec<AnyElement> = match &view.suggestions {
            Suggestions::None => return None,
            Suggestions::Commands { items, index } => items
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let lead = div().font_family(MONO).text_size(px(12.)).text_color(p.muted_foreground).child("/").into_any_element();
                    let hint = if c.usage.is_empty() { c.help.clone() } else { format!("{} \u{b7} {}", c.usage, c.help) };
                    row(i, i == *index, false, lead, c.name.clone(), hint).into_any_element()
                })
                .collect(),
            Suggestions::Emoji { items, index, .. } => items
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let lead = div().w(px(20.)).text_size(px(16.)).child(e.ch).into_any_element();
                    row(i, i == *index, true, lead, format!(":{}:", e.name), String::new()).into_any_element()
                })
                .collect(),
        };
        Some(
            v_flex()
                .absolute()
                .left(px(16.))
                .right(px(16.))
                .bottom_full()
                .mb(px(4.))
                .p(px(4.))
                .rounded(px(12.))
                .border_1()
                .border_color(p.border)
                .bg(p.popover)
                .shadow_lg()
                .children(rows)
                .into_any_element(),
        )
    }
}

impl Render for Composer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = cx.hearth().clone();
        let Some(view) = self.view.clone() else { return div().into_any_element() };
        let dispatch = self.dispatch.clone();

        let typing = match view.typing.as_slice() {
            [] => None,
            [a] => Some(format!("{a} is typing")),
            [a, b] => Some(format!("{a} and {b} are typing")),
            n => Some(format!("{} people are typing", n.len())),
        };
        let typing_row = h_flex().h(px(20.)).flex_none().px(px(22.)).gap(px(8.)).items_center().text_size(px(12.)).text_color(p.muted_foreground).when_some(
            typing,
            |d, text| {
                let who = view.typing.first().cloned().unwrap_or_default();
                let rest = text.strip_prefix(&who).map(str::to_string);
                d.child(h_flex().gap(px(3.)).children((0..3).map(|_| div().size(px(4.)).rounded_full().bg(p.muted_foreground)))).child(
                    match (view.typing.len(), rest) {
                        (1, Some(rest)) => h_flex()
                            .child(div().font_weight(FontWeight::MEDIUM).text_color(p.foreground.opacity(0.8)).child(who))
                            .child(rest)
                            .into_any_element(),
                        _ => div().child(text).into_any_element(),
                    },
                )
            },
        );

        let reply = view.reply_to.clone().map(|r| {
            let d = dispatch.clone();
            h_flex()
                .items_center()
                .gap(px(8.))
                .py(px(6.))
                .pr(px(6.))
                .pl(px(14.))
                .rounded_t(px(14.))
                .border_1()
                .border_b_0()
                .border_color(p.border)
                .bg(p.muted.opacity(0.5))
                .text_size(px(12.5))
                .text_color(p.muted_foreground)
                .child(icon("reply", 13., p.muted_foreground))
                .child(
                    h_flex()
                        .gap(px(4.))
                        .child("Replying to")
                        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(p.nick(nick_hue(&r.nick))).child(r.nick.clone())),
                )
                .child(div().flex_1().min_w_0().truncate().child(r.text.clone()))
                .child(icon_button("cancel-reply", "x", (24., 13.), p.muted_foreground, p.muted, Some("Cancel reply"), move |w, cx| {
                    d(UiAction::CancelReply, w, cx)
                }))
        });
        let has_reply = reply.is_some();

        let can_send = !view.draft.trim().is_empty();
        let send = {
            let this = cx.entity();
            let (bg, fg) = if can_send { (p.primary, p.primary_foreground) } else { (gpui_kit::transparent_black(), p.muted_foreground.opacity(0.6)) };
            div()
                .id("send")
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(32.))
                .rounded(px(10.))
                .bg(bg)
                .when(can_send, |d| d.cursor_pointer())
                .child(icon("arrow-up", 16., fg))
                .on_click(move |_, window, cx| this.update(cx, |c, cx| c.send(window, cx)))
        };

        let attach = view.can_upload.then(|| {
            let this = cx.entity();
            icon_button("attach", "paperclip", (32., 16.), p.muted_foreground, p.muted, Some("Attach file"), move |window, cx| {
                this.update(cx, |c, cx| c.attach(window, cx))
            })
        });
        let uploading = (view.uploading > 0).then(|| div().size(px(14.)).mr(px(4.)).child(Spinner::new().color(p.muted_foreground)));

        let popup = self.suggestions(&p, cx);
        v_flex()
            .relative()
            .flex_none()
            .child(typing_row)
            .child(
                v_flex().relative().px(px(16.)).pb(px(16.)).children(reply).children(popup).child(
                    h_flex()
                        .min_h(px(44.))
                        .items_end()
                        .rounded(px(14.))
                        .when(has_reply, |d| d.rounded_t(px(0.)))
                        .border_1()
                        .border_color(p.input)
                        .bg(p.card)
                        .shadow_xs()
                        .child(
                            h_flex().flex_none().self_end().py(px(6.)).pl(px(6.)).gap(px(2.)).children(attach).child(
                                div()
                                    .id("nick")
                                    .h(px(32.))
                                    .max_w(px(144.))
                                    .cursor_pointer()
                                    .hover({
                                        let bg = p.muted;
                                        move |s| s.bg(bg)
                                    })
                                    .tooltip(|window, cx| Tooltip::new("Change nickname").build(window, cx))
                                    .on_click({
                                        let this = cx.entity();
                                        move |_, window, cx| this.update(cx, |c, cx| c.prompt_nick(window, cx))
                                    })
                                    .px(px(8.))
                                    .flex()
                                    .items_center()
                                    .rounded(px(10.))
                                    .font_family(MONO)
                                    .text_size(px(12.))
                                    .text_color(p.muted_foreground)
                                    .truncate()
                                    .child(view.nick.clone()),
                            ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .pb(px(9.))
                                .text_size(px(14.))
                                .child(Textarea::new(&self.input).appearance(false).bordered(false).with_size(Size::Size(px(16.)))),
                        )
                        .child(h_flex().flex_none().self_end().items_center().py(px(6.)).pr(px(6.)).children(uploading).child(send)),
                ),
            )
            .into_any_element()
    }
}
