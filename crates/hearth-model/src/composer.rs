//! The message box: text, caret, history recall, tab completion and the
//! command and emoji suggestion popups. One composer per buffer, so a draft
//! survives switching away. Carets are byte offsets into `text`.

use crate::buffer::{BufId, BufferKind};
use crate::commands::{command_infos, CommandInfo};
use crate::effect::Effect;
use crate::emoji::{emoji_query, search_emoji, typed_shortcode, Emoji};
use crate::state::{App, ToastKind, TypingState};
use crate::types::Line;
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Completion {
    start: usize,
    end: usize,
    options: Vec<String>,
    index: usize,
    suffix: String,
}

#[derive(Debug, Clone, Default)]
pub struct Composer {
    pub text: String,
    /// Byte offset of the caret (the start of the selection).
    pub caret: usize,
    /// Which input-history entry is showing, if recalling.
    hist_index: Option<usize>,
    completion: Option<Completion>,
    /// Highlighted row of the command popup.
    pub cmd_index: usize,
    /// Highlighted row of the emoji popup.
    pub emoji_index: usize,
    /// The colon position whose suggestions Escape closed.
    emoji_dismissed: Option<usize>,
    /// Uploads in flight.
    pub uploading: u32,
    last_emoji_key: String,
    last_cmd_key: String,
}

/// What the suggestion popup above the box shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Suggestions {
    None,
    Commands { items: Vec<CommandInfo>, index: usize },
    Emoji { start: usize, query: String, items: Vec<Emoji>, index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerKey {
    Up,
    Down,
    Tab,
    Enter,
    Escape,
}

/// What a key press did, so the toolkit knows whether to run its default action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Not ours: let the text field handle it (move the caret, insert a newline, move focus on Tab).
    Ignored,
    /// Consumed; redraw.
    Handled,
    /// The input was run (sent or executed as a command).
    Sent,
    /// Escape with nothing to cancel: release focus.
    Blur,
}

fn cmd_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^/([\w-]*)$").unwrap())
}

impl Composer {
    fn command_matches(&self, aliases: &std::collections::BTreeMap<String, String>) -> Vec<CommandInfo> {
        let Some(m) = cmd_re().captures(&self.text) else { return Vec::new() };
        let q = m[1].to_lowercase();
        command_infos(aliases).into_iter().filter(|c| c.name.starts_with(&q)).take(7).collect()
    }

    fn emoji_state(&self, aliases: &std::collections::BTreeMap<String, String>) -> Option<(usize, String, Vec<Emoji>)> {
        if !self.command_matches(aliases).is_empty() {
            return None;
        }
        let caret = self.caret.min(self.text.len());
        let q = emoji_query(self.text.get(..caret)?)?;
        if self.emoji_dismissed == Some(q.start) {
            return None;
        }
        let items = search_emoji(&q.query, 7);
        (!items.is_empty()).then_some((q.start, q.query, items))
    }

    fn set_text(&mut self, text: String, caret: usize) {
        self.text = text;
        self.caret = caret;
    }

    /// Keep the popup selections sane when what they list changed.
    fn refresh_selection(&mut self, aliases: &std::collections::BTreeMap<String, String>) {
        let cmds = self.command_matches(aliases);
        let ck = cmds.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(",");
        if ck != self.last_cmd_key {
            self.cmd_index = 0;
            self.last_cmd_key = ck;
        }
        let ek = self.emoji_state(aliases).map(|(s, q, _)| format!("{s}:{q}")).unwrap_or_default();
        if ek != self.last_emoji_key {
            self.emoji_index = 0;
            self.last_emoji_key = ek;
        }
    }
}

fn clamp_char_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

impl App {
    pub fn composer(&self, id: &BufId) -> Option<&Composer> {
        self.buffer(id).map(|b| &b.composer)
    }

    fn composer_mut(&mut self, id: &BufId) -> Option<&mut Composer> {
        self.buffer_mut(id).map(|b| &mut b.composer)
    }

    fn with_composer<R>(&mut self, id: &BufId, f: impl FnOnce(&mut Composer, &std::collections::BTreeMap<String, String>) -> R) -> Option<R> {
        let aliases = self.settings.aliases.clone();
        let b = self.buffer_mut(id)?;
        let r = f(&mut b.composer, &aliases);
        b.composer.refresh_selection(&aliases);
        b.touch();
        Some(r)
    }

    pub fn composer_placeholder(&self, id: &BufId) -> String {
        match self.buffer(id) {
            Some(b) if b.kind == BufferKind::Server => "Type /help for commands".into(),
            Some(b) if b.kind == BufferKind::Channel && !b.joined => format!("You're not in {}", b.name),
            Some(b) => format!("Message {}", b.name),
            None => String::new(),
        }
    }

    /// The popup to show above the box right now.
    pub fn composer_suggestions(&self, id: &BufId) -> Suggestions {
        let Some(c) = self.composer(id) else { return Suggestions::None };
        let cmds = c.command_matches(&self.settings.aliases);
        if !cmds.is_empty() {
            return Suggestions::Commands { index: c.cmd_index.min(cmds.len() - 1), items: cmds };
        }
        match c.emoji_state(&self.settings.aliases) {
            Some((start, query, items)) => Suggestions::Emoji { start, query, index: c.emoji_index.min(items.len() - 1), items },
            None => Suggestions::None,
        }
    }

    /// The user edited the text (typed, pasted, cut). `caret` is where it is now.
    pub fn composer_edit(&mut self, id: &BufId, text: &str, caret: usize) {
        let state = self.with_composer(id, |c, _| {
            c.hist_index = None;
            c.completion = None;
            let caret = clamp_char_boundary(text, caret);
            c.set_text(text.to_string(), caret);
            if let Some((start, ch)) = typed_shortcode(&text[..caret]) {
                let next = format!("{}{}{}", &text[..start], ch, &text[caret..]);
                c.set_text(next, start + ch.len());
            } else if emoji_query(&text[..caret]).is_none() {
                c.emoji_dismissed = None;
            }
            c.text.starts_with('/') || c.text.is_empty()
        });
        let Some(done) = state else { return };
        self.send_typing(id, if done { TypingState::Done } else { TypingState::Active });
    }

    /// The caret moved without an edit (arrow keys, a click).
    pub fn composer_set_caret(&mut self, id: &BufId, caret: usize) {
        self.with_composer(id, |c, _| {
            c.caret = clamp_char_boundary(&c.text, caret);
            c.completion = None;
        });
    }

    /// Replace the text outright (the nickname button, the host restoring a draft).
    pub fn composer_set_text(&mut self, id: &BufId, text: &str) {
        self.with_composer(id, |c, _| {
            c.set_text(text.to_string(), text.len());
            c.completion = None;
        });
    }

    /// Put the nickname-change command in the box.
    pub fn composer_prompt_nick(&mut self, id: &BufId) {
        let Some(nick) = self.conns.get(&id.conn).map(|c| c.nick.clone()) else { return };
        self.composer_set_text(id, &format!("/nick {nick}"));
    }

    /// Append text (an uploaded link), separated by a space.
    pub fn composer_insert(&mut self, id: &BufId, s: &str) {
        self.with_composer(id, |c, _| {
            let sep = if !c.text.is_empty() && !c.text.ends_with(' ') { " " } else { "" };
            let t = format!("{}{sep}{s}", c.text);
            c.set_text(t, c.text.len() + sep.len() + s.len());
        });
    }

    /// Append a mention such as `nick: `, trimming trailing whitespace first.
    pub fn composer_mention(&mut self, id: &BufId, s: &str) {
        self.with_composer(id, |c, _| {
            let t = if c.text.is_empty() { s.to_string() } else { format!("{} {s}", c.text.trim_end()) };
            let n = t.len();
            c.set_text(t, n);
        });
    }

    /// Choose a row of the emoji popup (a click).
    pub fn composer_pick_emoji(&mut self, id: &BufId, index: usize) {
        self.with_composer(id, |c, aliases| {
            let Some((start, _, items)) = c.emoji_state(aliases) else { return };
            let Some(em) = items.get(index) else { return };
            pick_emoji(c, start, em);
        });
    }

    /// Choose a row of the command popup (a click).
    pub fn composer_pick_command(&mut self, id: &BufId, index: usize) {
        self.with_composer(id, |c, aliases| {
            if let Some(cmd) = c.command_matches(aliases).get(index) {
                let t = format!("/{} ", cmd.name);
                let n = t.len();
                c.set_text(t, n);
            }
        });
    }

    /// Run a key press through the composer's behaviours.
    pub fn composer_key(&mut self, id: &BufId, key: ComposerKey, shift: bool) -> KeyOutcome {
        let history = self.input_history().to_vec();
        let aliases = self.settings.aliases.clone();
        let Some(b) = self.buffer_mut(id) else { return KeyOutcome::Ignored };
        let pre = key_pre(&mut b.composer, key, shift, &history, &aliases);
        b.composer.refresh_selection(&aliases);
        b.touch();
        match pre {
            KeyPre::Done(o) => o,
            KeyPre::Tab => self.composer_tab(id, shift),
            KeyPre::Submit => {
                if self.composer_submit(id) {
                    KeyOutcome::Sent
                } else {
                    KeyOutcome::Handled
                }
            }
            KeyPre::Escape => match self.buffer_mut(id) {
                Some(b) if b.reply_to.is_some() => {
                    b.reply_to = None;
                    b.touch();
                    KeyOutcome::Handled
                }
                _ => KeyOutcome::Blur,
            },
        }
    }

    /// Tab completion: nicks (recent speakers first), channels and commands.
    /// `Ignored` when there's nothing to complete, so Tab can move focus instead.
    fn composer_tab(&mut self, id: &BufId, backwards: bool) -> KeyOutcome {
        let aliases = self.settings.aliases.clone();
        let commands: Vec<String> = command_infos(&aliases).into_iter().map(|c| c.name).collect();
        let Some(conn) = self.conns.get(&id.conn) else { return KeyOutcome::Ignored };
        let Some(b) = conn.buf(&id.key) else { return KeyOutcome::Ignored };
        let c = &b.composer;
        if let Some(mut comp) = c.completion.clone().filter(|k| c.caret == k.end) {
            comp.index = (comp.index + if backwards { comp.options.len() - 1 } else { 1 }) % comp.options.len();
            return self.apply_completion(id, comp);
        }
        let caret = clamp_char_boundary(&c.text, c.caret);
        let before = &c.text[..caret];
        let start = before.rfind(char::is_whitespace).map_or(0, |i| i + before[i..].chars().next().map_or(1, char::len_utf8));
        let word = &before[start..];
        if word.is_empty() {
            return KeyOutcome::Ignored;
        }
        let lw = word.to_lowercase();
        let mut suffix = " ".to_string();
        let options: Vec<String>;
        if start == 0 && word.starts_with('/') {
            options = commands.iter().filter(|n| n.starts_with(&lw[1..])).map(|n| format!("/{n}")).collect();
        } else if conn.is_channel(word) {
            options = conn.buffers_iter().filter(|b| b.kind == BufferKind::Channel && b.name.to_lowercase().starts_with(&lw)).map(|b| b.name.clone()).collect();
        } else {
            let mut pool = recent_speakers(b);
            let mut all: Vec<String> = b.members().map(|m| m.nick.clone()).collect();
            if b.kind == BufferKind::Query {
                all.push(b.name.clone());
            }
            all.sort_by(|a, b| crate::buffer::collate(a, b));
            pool.extend(all);
            let mut seen = std::collections::HashSet::new();
            options = pool.into_iter().filter(|n| seen.insert(n.clone())).filter(|n| n.to_lowercase().starts_with(&lw) && !conn.is_me(n)).collect();
            if start == 0 {
                suffix = ": ".into();
            }
        }
        if options.is_empty() {
            return KeyOutcome::Ignored;
        }
        let index = if backwards { options.len() - 1 } else { 0 };
        self.apply_completion(id, Completion { start, end: caret, options, index, suffix })
    }

    fn apply_completion(&mut self, id: &BufId, mut comp: Completion) -> KeyOutcome {
        let Some(c) = self.composer_mut(id) else { return KeyOutcome::Ignored };
        let insert = format!("{}{}", comp.options[comp.index], comp.suffix);
        let tail = &c.text[comp.end.min(c.text.len())..];
        let after = tail.strip_prefix(": ").or_else(|| tail.strip_prefix(' ')).unwrap_or(tail);
        let text = format!("{}{insert}{after}", &c.text[..comp.start]);
        comp.end = comp.start + insert.len();
        c.set_text(text, comp.end);
        c.completion = Some(comp);
        self.env.touch();
        KeyOutcome::Handled
    }

    /// Send the box's contents. Returns whether anything was sent.
    pub fn composer_submit(&mut self, id: &BufId) -> bool {
        let Some(b) = self.buffer(id) else { return false };
        let value = b.composer.text.clone();
        if value.trim().is_empty() {
            return false;
        }
        let reply_to = if value.starts_with('/') && !value.starts_with("//") { None } else { b.reply_to.as_ref().and_then(|l| l.msgid.clone()) };
        if !self.run_input(id, &value, reply_to.as_deref()) {
            return false;
        }
        self.push_input_history(&value);
        if let Some(b) = self.buffer_mut(id) {
            if reply_to.is_some() {
                b.reply_to = None;
            }
            b.composer.text.clear();
            b.composer.caret = 0;
            b.composer.hist_index = None;
            b.composer.completion = None;
            b.touch();
        }
        self.send_typing(id, TypingState::Done);
        true
    }

    // ── Replies ──────────────────────────────────────────────

    pub fn can_reply(&self, id: &BufId, line: &Line) -> bool {
        line.msgid.is_some() && !id.is_server() && !line.redacted && self.conns.get(&id.conn).is_some_and(|c| c.can_tag("draft/reply"))
    }

    pub fn start_reply(&mut self, id: &BufId, line: &Line) {
        if let Some(b) = self.buffer_mut(id) {
            b.reply_to = Some(line.clone());
            b.touch();
        }
    }

    pub fn cancel_reply(&mut self, id: &BufId) {
        if let Some(b) = self.buffer_mut(id) {
            if b.reply_to.take().is_some() {
                b.touch();
            }
        }
    }

    /// Copy a message's text (without formatting codes) to the clipboard.
    pub fn copy_line(&mut self, line: &Line) {
        self.env.emit(Effect::CopyText { text: crate::format::strip_formatting(&line.text) });
        self.toast("Copied", ToastKind::Info);
    }
}

/// What a key needs from the app after the composer-local part ran.
pub(crate) enum KeyPre {
    Done(KeyOutcome),
    Tab,
    Submit,
    Escape,
}

fn pick_emoji(c: &mut Composer, start: usize, em: &Emoji) {
    let insert = format!("{} ", em.ch);
    let caret = clamp_char_boundary(&c.text, c.caret);
    let tail = &c.text[caret..];
    let tail = tail.strip_prefix(' ').unwrap_or(tail);
    let text = format!("{}{insert}{tail}", &c.text[..start]);
    c.set_text(text, start + insert.len());
}

/// Up to 50 nicks that spoke in the last 400 lines, newest first, not us.
fn recent_speakers(b: &crate::buffer::Buffer) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let lines = b.lines();
    for l in lines.iter().rev().take(399) {
        if seen.len() >= 50 {
            break;
        }
        if !l.nick.is_empty() && !l.is_self && !seen.contains(&l.nick) {
            seen.push(l.nick.clone());
        }
    }
    seen
}

fn key_pre(c: &mut Composer, key: ComposerKey, shift: bool, history: &[String], aliases: &std::collections::BTreeMap<String, String>) -> KeyPre {
    if let Some((start, query, items)) = c.emoji_state(aliases) {
        let n = items.len();
        let idx = c.emoji_index.min(n - 1);
        match key {
            ComposerKey::Up | ComposerKey::Down => {
                c.emoji_index = (idx + if key == ComposerKey::Up { n - 1 } else { 1 }) % n;
                return KeyPre::Done(KeyOutcome::Handled);
            }
            // Enter still sends a lone `:D` or `:P`; Tab always picks.
            ComposerKey::Tab | ComposerKey::Enter if key == ComposerKey::Tab || (!shift && query.chars().count() > 1) => {
                pick_emoji(c, start, &items[idx]);
                return KeyPre::Done(KeyOutcome::Handled);
            }
            ComposerKey::Escape => {
                c.emoji_dismissed = Some(start);
                return KeyPre::Done(KeyOutcome::Handled);
            }
            _ => {}
        }
    }
    let cmds = c.command_matches(aliases);
    if !cmds.is_empty() {
        let n = cmds.len();
        let idx = c.cmd_index.min(n - 1);
        match key {
            ComposerKey::Up | ComposerKey::Down => {
                c.cmd_index = (idx + if key == ComposerKey::Up { n - 1 } else { 1 }) % n;
                return KeyPre::Done(KeyOutcome::Handled);
            }
            ComposerKey::Tab | ComposerKey::Enter if key == ComposerKey::Tab || (!shift && cmds[idx].name != c.text[1..]) => {
                let t = format!("/{} ", cmds[idx].name);
                let n = t.len();
                c.set_text(t, n);
                return KeyPre::Done(KeyOutcome::Handled);
            }
            _ => {}
        }
    }
    if key == ComposerKey::Tab {
        return KeyPre::Tab;
    }
    c.completion = None;
    match key {
        ComposerKey::Enter if !shift => KeyPre::Submit,
        ComposerKey::Up => {
            let caret = clamp_char_boundary(&c.text, c.caret);
            if !c.text[..caret].contains('\n') {
                let next = c.hist_index.map_or(0, |i| i + 1);
                if next < history.len() && (c.text.is_empty() || c.hist_index.is_some()) {
                    c.hist_index = Some(next);
                    let t = history[next].clone();
                    let n = t.len();
                    c.set_text(t, n);
                    return KeyPre::Done(KeyOutcome::Handled);
                }
            }
            KeyPre::Done(KeyOutcome::Ignored)
        }
        ComposerKey::Down => {
            let caret = clamp_char_boundary(&c.text, c.caret);
            if let (Some(i), false) = (c.hist_index, c.text[caret..].contains('\n')) {
                c.hist_index = i.checked_sub(1);
                let t = c.hist_index.map(|i| history[i].clone()).unwrap_or_default();
                let n = t.len();
                c.set_text(t, n);
                return KeyPre::Done(KeyOutcome::Handled);
            }
            KeyPre::Done(KeyOutcome::Ignored)
        }
        ComposerKey::Escape => KeyPre::Escape,
        _ => KeyPre::Done(KeyOutcome::Ignored),
    }
}
