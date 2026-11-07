//! Slash commands and composer input.

use crate::aliases::{expand_command, split_commands, AliasVars};
use crate::buffer::{collate, BufId, BufferKind};
use crate::effect::{Effect, Secret};
use crate::state::{App, SendKind, ToastKind};
use crate::types::{LineType, NotifyLevel};
use crate::Conn;
use std::collections::BTreeMap;

/// Built-in commands: name, usage, one-line help. The order is the /help order.
pub const COMMANDS: &[(&str, &str, &str)] = &[
    ("join", "/join <#channel> [key]", "Join a channel"),
    ("part", "/part [#channel] [reason]", "Leave a channel"),
    ("close", "/close", "Close this buffer (parts channels)"),
    ("msg", "/msg <target> <text>", "Send a private message"),
    ("query", "/query <nick> [text]", "Open a conversation"),
    ("me", "/me <action>", "Send an action"),
    ("notice", "/notice <target> <text>", "Send a notice"),
    ("nick", "/nick <newnick>", "Change your nickname"),
    ("topic", "/topic [text]", "Show or set the channel topic"),
    ("whois", "/whois <nick>", "Look up a user"),
    ("away", "/away [message]", "Mark yourself away (no message clears it)"),
    ("back", "/back", "Clear away status"),
    ("invite", "/invite <nick> [#channel]", "Invite someone to a channel"),
    ("kick", "/kick <nick> [reason]", "Remove someone from the channel"),
    ("ban", "/ban <nick|mask>", "Ban a nick or mask"),
    ("unban", "/unban <mask>", "Remove a ban"),
    ("op", "/op <nick\u{2026}>", "Give operator status"),
    ("deop", "/deop <nick\u{2026}>", "Take operator status"),
    ("voice", "/voice <nick\u{2026}>", "Give voice"),
    ("devoice", "/devoice <nick\u{2026}>", "Take voice"),
    ("mode", "/mode [target] <modes> [args]", "Show or change modes"),
    ("names", "/names", "Refresh the member list"),
    ("ctcp", "/ctcp <nick> <command> [args]", "Send a CTCP request"),
    ("raw", "/raw <line>", "Send a raw IRC line"),
    ("quote", "/quote <line>", "Alias for /raw"),
    ("clear", "/clear", "Clear this buffer"),
    ("reconnect", "/reconnect", "Reconnect this server"),
    ("quit", "/quit [reason]", "Disconnect from this server"),
    ("motd", "/motd", "Show the message of the day"),
    ("list", "/list", "Browse the channels on this network"),
    ("search", "/search [text]", "Search message history"),
    ("ignore", "/ignore [nick|mask]", "Hide messages from someone (no argument lists ignores)"),
    ("unignore", "/unignore <nick|mask>", "Stop ignoring someone"),
    ("watch", "/watch [nick]", "Get told when someone comes online (no argument lists them)"),
    ("unwatch", "/unwatch <nick\u{2026}>", "Stop watching someone"),
    ("notify", "/notify <all|mentions|none|default>", "Set notifications for this buffer"),
    ("rename", "/rename [#channel] <#newname> [reason]", "Rename a channel (needs server support)"),
    ("register", "/register <email|*> <password>", "Register an account for your current nick"),
    ("verify", "/verify <account> <code>", "Confirm an account registration"),
    ("help", "/help", "Show commands"),
];

/// Short forms of built-in commands.
pub const ALIASES: &[(&str, &str)] = &[
    ("j", "join"),
    ("p", "part"),
    ("q", "query"),
    ("w", "whois"),
    ("leave", "part"),
    ("m", "msg"),
    ("monitor", "watch"),
    ("mute", "notify"),
    ("unmute", "notify"),
];

fn alias_target(name: &str) -> &str {
    ALIASES.iter().find(|(a, _)| *a == name).map_or(name, |(_, t)| t)
}

fn is_command(name: &str) -> bool {
    COMMANDS.iter().any(|(n, ..)| *n == name)
}

pub fn is_builtin(name: &str) -> bool {
    is_command(name) || ALIASES.iter().any(|(a, _)| *a == name)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInfo {
    pub name: String,
    pub usage: String,
    pub help: String,
    pub alias: bool,
}

/// Built-in commands plus user aliases (which win on a name clash), for suggestions and /help.
pub fn command_infos(aliases: &BTreeMap<String, String>) -> Vec<CommandInfo> {
    let mut user: Vec<(&String, &String)> = aliases.iter().collect();
    user.sort_by(|a, b| collate(a.0, b.0));
    let mut out: Vec<CommandInfo> =
        user.into_iter().map(|(n, body)| CommandInfo { name: n.clone(), usage: format!("/{n}"), help: format!("\u{2192} {body}"), alias: true }).collect();
    let builtin = COMMANDS.iter().filter(|(n, ..)| !aliases.contains_key(*n)).map(|(n, u, h)| CommandInfo {
        name: n.to_string(),
        usage: u.to_string(),
        help: h.to_string(),
        alias: false,
    });
    out.extend(builtin);
    out
}

const MAX_ALIAS_DEPTH: usize = 8;
/// Bounds fan-out (a -> b;b;b, b -> c;c;c, ...) as well as depth.
const MAX_ALIAS_COMMANDS: i32 = 50;

/// A command was used wrongly; the text says how to use it.
struct Usage(String);

type Run = Result<(), Usage>;

fn need(cond: bool, usage: &str) -> Run {
    if cond {
        Ok(())
    } else {
        Err(Usage(usage.to_string()))
    }
}

struct Ctx<'a> {
    app: &'a mut App,
    conn: &'a mut Conn,
    key: &'a str,
    args: Vec<String>,
    rest: String,
}

impl Ctx<'_> {
    fn kind(&self) -> BufferKind {
        self.conn.buf(self.key).map_or(BufferKind::Server, |b| b.kind)
    }

    fn name(&self) -> String {
        self.conn.buf(self.key).map(|b| b.name.clone()).unwrap_or_default()
    }

    /// Send a command whose replies should come back to this buffer.
    fn ask(&mut self, line: &str) {
        self.conn.send_from(self.key, line);
    }

    fn chan_of(&self, arg: Option<&str>) -> Result<String, Usage> {
        if let Some(a) = arg.filter(|a| self.conn.is_channel(a)) {
            return Ok(a.to_string());
        }
        need(self.kind() == BufferKind::Channel, "This command needs a channel")?;
        Ok(self.name())
    }

    fn arg(&self, i: usize) -> &str {
        self.args.get(i).map(String::as_str).unwrap_or("")
    }
}

fn mode_for(ctx: &mut Ctx, sign: char, mode: char) -> Run {
    // The legacy client prints the mode letter here ("/o", "/dev"), not the command name; kept as is.
    let usage = format!("/{}{mode} <nick\u{2026}>", if sign == '+' { "" } else { "de" });
    need(!ctx.args.is_empty(), &usage)?;
    let chan = ctx.chan_of(None)?;
    let modes: String = std::iter::repeat_n(mode, ctx.args.len()).collect();
    ctx.conn.send(format!("MODE {chan} {sign}{modes} {}", ctx.args.join(" ")));
    Ok(())
}

fn run_command(name: &str, ctx: &mut Ctx) -> Run {
    let a0 = ctx.arg(0).to_string();
    match name {
        "join" => {
            need(!a0.is_empty(), "/join <#channel> [key]")?;
            let chan = if ctx.conn.is_channel(&a0) { a0 } else { format!("#{a0}") };
            let key = ctx.args.get(1).cloned();
            ctx.app.join_in(ctx.conn, &chan, key.as_deref());
        }
        "part" => {
            let explicit = !a0.is_empty() && ctx.conn.is_channel(&a0);
            let chan = ctx.chan_of(explicit.then_some(a0.as_str()))?;
            let reason = if explicit { ctx.args[1..].join(" ") } else { ctx.rest.clone() };
            ctx.conn.send(if reason.is_empty() { format!("PART {chan}") } else { format!("PART {chan} :{reason}") });
        }
        "close" => ctx.app.close_buffer_in(ctx.conn, ctx.key),
        "msg" => {
            need(ctx.args.len() >= 2, "/msg <target> <text>")?;
            let target = ctx.conn.ensure(&a0, None);
            let text = ctx.args[1..].join(" ");
            ctx.app.send_message_in(ctx.conn, &target, &text, SendKind::Privmsg, false, None);
        }
        "query" => {
            need(!a0.is_empty(), "/query <nick> [text]")?;
            let q = ctx.app.query_in(ctx.conn, &a0);
            if ctx.args.len() > 1 {
                let text = ctx.args[1..].join(" ");
                ctx.app.send_message_in(ctx.conn, &q.key, &text, SendKind::Privmsg, false, None);
            }
        }
        "me" => {
            need(!ctx.rest.is_empty() && ctx.kind() != BufferKind::Server, "/me <action>")?;
            let rest = ctx.rest.clone();
            ctx.app.send_message_in(ctx.conn, ctx.key, &rest, SendKind::Privmsg, true, None);
        }
        "notice" => {
            need(ctx.args.len() >= 2, "/notice <target> <text>")?;
            let target = ctx.conn.ensure(&a0, None);
            let text = ctx.args[1..].join(" ");
            ctx.app.send_message_in(ctx.conn, &target, &text, SendKind::Notice, false, None);
        }
        "nick" => {
            need(!a0.is_empty(), "/nick <newnick>")?;
            ctx.conn.send(format!("NICK {a0}"));
        }
        "topic" => {
            let chan = ctx.chan_of(None)?;
            if ctx.rest.is_empty() {
                ctx.ask(&format!("TOPIC {chan}"));
            } else {
                ctx.conn.send(format!("TOPIC {chan} :{}", ctx.rest));
            }
        }
        "whois" => {
            let nick = if !a0.is_empty() {
                a0
            } else if ctx.kind() == BufferKind::Query {
                ctx.name()
            } else {
                String::new()
            };
            need(!nick.is_empty(), "/whois <nick>")?;
            ctx.ask(&format!("WHOIS {nick} {nick}"));
        }
        "away" => {
            let line = if ctx.rest.is_empty() { "AWAY".to_string() } else { format!("AWAY :{}", ctx.rest) };
            ctx.ask(&line);
        }
        "back" => ctx.ask("AWAY"),
        "invite" => {
            need(!a0.is_empty(), "/invite <nick> [#channel]")?;
            let chan = ctx.chan_of(ctx.args.get(1).map(String::as_str))?;
            ctx.conn.send(format!("INVITE {a0} {chan}"));
        }
        "kick" => {
            need(!a0.is_empty(), "/kick <nick> [reason]")?;
            let reason = ctx.args[1..].join(" ");
            let chan = ctx.chan_of(None)?;
            ctx.conn.send(format!("KICK {chan} {a0}{}", if reason.is_empty() { String::new() } else { format!(" :{reason}") }));
        }
        "ban" => {
            need(!a0.is_empty(), "/ban <nick|mask>")?;
            let mask = if a0.contains('!') || a0.contains('@') { a0 } else { format!("{a0}!*@*") };
            let chan = ctx.chan_of(None)?;
            ctx.conn.send(format!("MODE {chan} +b {mask}"));
        }
        "unban" => {
            need(!a0.is_empty(), "/unban <mask>")?;
            let chan = ctx.chan_of(None)?;
            ctx.conn.send(format!("MODE {chan} -b {a0}"));
        }
        "op" => mode_for(ctx, '+', 'o')?,
        "deop" => mode_for(ctx, '-', 'o')?,
        "voice" => mode_for(ctx, '+', 'v')?,
        "devoice" => mode_for(ctx, '-', 'v')?,
        "mode" => {
            if !a0.is_empty() && (ctx.conn.is_channel(&a0) || ctx.conn.is_me(&a0)) {
                let line = format!("MODE {}", ctx.args.join(" "));
                ctx.ask(&line);
            } else {
                need(ctx.kind() == BufferKind::Channel, "/mode <target> <modes>")?;
                let line = format!("MODE {} {}", ctx.name(), ctx.args.join(" ")).trim().to_string();
                ctx.ask(&line);
            }
        }
        "names" => {
            let chan = ctx.chan_of(None)?;
            ctx.ask(&format!("NAMES {chan}"));
        }
        "ctcp" => {
            need(ctx.args.len() >= 2, "/ctcp <nick> <command> [args]")?;
            let payload = std::iter::once(ctx.args[1].to_uppercase()).chain(ctx.args[2..].iter().cloned()).collect::<Vec<_>>().join(" ");
            ctx.conn.send(format!("PRIVMSG {a0} :\x01{payload}\x01"));
        }
        "raw" | "quote" => {
            need(!ctx.rest.is_empty(), "/raw <line>")?;
            let rest = ctx.rest.clone();
            ctx.ask(&rest);
        }
        "clear" => {
            if let Some(b) = ctx.conn.buf_mut(ctx.key) {
                b.set_lines(Vec::new());
                b.history = crate::buffer::HistoryState::Done;
            }
        }
        "reconnect" => ctx.app.env.emit(Effect::Reconnect { server: ctx.conn.server_id.clone() }),
        "quit" => ctx.app.env.emit(Effect::Disconnect { server: ctx.conn.server_id.clone(), reason: (!ctx.rest.is_empty()).then(|| ctx.rest.clone()) }),
        "motd" => ctx.ask("MOTD"),
        "list" => ctx.app.browse_channels_in(ctx.conn),
        "search" => {
            let rest = ctx.rest.clone();
            ctx.app.open_search(&rest);
        }
        "ignore" => {
            if a0.is_empty() {
                let list = &ctx.app.settings.ignores;
                let text = if list.is_empty() { "Not ignoring anyone".to_string() } else { format!("Ignoring: {}", list.join(", ")) };
                let mut l = ctx.app.env.line(LineType::Info);
                l.text = text;
                if let Some(b) = ctx.conn.buf_mut(ctx.key) {
                    b.push(l);
                }
            } else {
                ctx.app.ignore(&a0);
                ctx.app.toast(format!("Ignoring {a0}"), ToastKind::Info);
            }
        }
        "unignore" => {
            need(!a0.is_empty(), "/unignore <nick|mask>")?;
            let removed = ctx.app.unignore(&a0);
            ctx.app.toast(if removed { format!("No longer ignoring {a0}") } else { format!("{a0} was not ignored") }, ToastKind::Info);
        }
        "watch" => {
            need(ctx.conn.can_monitor(), "This server does not support MONITOR")?;
            if a0.is_empty() {
                let list = crate::monitor::watched(ctx.app, ctx.conn);
                let text = if list.is_empty() {
                    "Not watching anyone".to_string()
                } else {
                    let items: Vec<String> = list
                        .iter()
                        .map(|n| format!("{n} ({})", if ctx.conn.online.get(&ctx.conn.fold(n)).copied().unwrap_or(false) { "online" } else { "offline" }))
                        .collect();
                    format!("Watching: {}", items.join(", "))
                };
                let mut l = ctx.app.env.line(LineType::Info);
                l.text = text;
                if let Some(b) = ctx.conn.buf_mut(ctx.key) {
                    b.push(l);
                }
            } else {
                for n in ctx.args.clone() {
                    crate::monitor::watch(ctx.app, ctx.conn, &n);
                }
            }
        }
        "unwatch" => {
            need(!a0.is_empty(), "/unwatch <nick\u{2026}>")?;
            for n in ctx.args.clone() {
                crate::monitor::unwatch(ctx.app, ctx.conn, &n);
            }
        }
        "notify" => {
            let level = a0.to_lowercase();
            need(ctx.kind() != BufferKind::Server && ["all", "mentions", "none", "default"].contains(&level.as_str()), "/notify <all|mentions|none|default>")?;
            let lv = match level.as_str() {
                "all" => Some(NotifyLevel::All),
                "mentions" => Some(NotifyLevel::Mentions),
                "none" => Some(NotifyLevel::None),
                _ => None,
            };
            ctx.app.set_notify_in(ctx.conn, ctx.key, lv);
            let name = ctx.name();
            ctx.app.toast(format!("Notifications for {name}: {level}"), ToastKind::Info);
        }
        "rename" => {
            need(ctx.conn.has("draft/channel-rename"), "This server does not support renaming channels")?;
            let both = ctx.args.len() >= 2 && ctx.conn.is_channel(&ctx.args[0]) && ctx.conn.is_channel(&ctx.args[1]);
            let from = if both { ctx.args[0].clone() } else { ctx.chan_of(None)? };
            let to = if both { ctx.arg(1).to_string() } else { a0 };
            need(!to.is_empty() && ctx.conn.is_channel(&to), "/rename [#channel] <#newname> [reason]")?;
            let reason = ctx.args.iter().skip(if both { 2 } else { 1 }).cloned().collect::<Vec<_>>().join(" ");
            ctx.ask(&format!("RENAME {from} {to}{}", if reason.is_empty() { String::new() } else { format!(" :{reason}") }));
        }
        "register" => {
            need(ctx.conn.has("draft/account-registration"), "This server does not support registering here; try /msg NickServ help")?;
            need(ctx.args.len() == 2, "/register <email|*> <password>")?;
            let (email, password) = (ctx.args[0].clone(), ctx.args[1].clone());
            let caps = ctx.conn.cap_values.get("draft/account-registration").cloned().unwrap_or_default();
            need(email != "*" || !caps.split(',').any(|c| c == "email-required"), "This server needs an email address: /register <email> <password>")?;
            ctx.conn.registration = Some(crate::conn::Registration { account: ctx.conn.nick.clone(), password: Secret(password.clone()) });
            ctx.ask(&format!("REGISTER * {email} :{password}"));
        }
        "verify" => {
            need(ctx.conn.has("draft/account-registration"), "This server does not support account registration")?;
            need(ctx.args.len() == 2, "/verify <account> <code>")?;
            let line = format!("VERIFY {} {}", ctx.args[0], ctx.args[1]);
            ctx.ask(&line);
        }
        "help" => {
            let text = command_infos(&ctx.app.settings.aliases).iter().map(|c| format!("{:<32} {}", c.usage, c.help)).collect::<Vec<_>>().join("\n");
            let mut l = ctx.app.env.line(LineType::Info);
            l.data = Some(BTreeMap::from([("motd".to_string(), text), ("title".to_string(), "Commands".to_string())]));
            if let Some(b) = ctx.conn.buf_mut(ctx.key) {
                b.push(l);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Expand and run a user alias. `None` when `name` isn't one (or wraps the built-in it shadows).
fn run_alias(app: &mut App, conn: &mut Conn, key: &str, name: &str, args: &[String], names: &[String], budget: &mut i32) -> Option<bool> {
    let body = app.settings.aliases.get(name)?.clone();
    if names.iter().any(|n| n == name) {
        // An alias may wrap the built-in it shadows, e.g. /join -> /join #$1.
        if is_command(alias_target(name)) {
            return None;
        }
        app.toast(format!("Alias /{name} calls itself"), ToastKind::Error);
        return Some(false);
    }
    if names.len() >= MAX_ALIAS_DEPTH {
        app.toast(format!("Aliases nest too deeply at /{name}"), ToastKind::Error);
        return Some(false);
    }
    let (kind, buf_name) = conn.buf(key).map(|b| (b.kind, b.name.clone())).unwrap_or((BufferKind::Server, String::new()));
    let vars =
        AliasVars { nick: conn.nick.clone(), channel: if kind == BufferKind::Channel { buf_name } else { String::new() }, network: conn.name().to_string() };
    let mut next: Vec<String> = names.to_vec();
    next.push(name.to_string());
    let mut ok = false;
    for cmd in split_commands(&body) {
        *budget -= 1;
        if *budget < 0 {
            let first = next.first().cloned().unwrap_or_else(|| name.to_string());
            app.toast(format!("Alias /{first} runs too many commands"), ToastKind::Error);
            return Some(ok);
        }
        let line = expand_command(&cmd, args, &vars);
        ok = run_input_in(app, conn, key, &line, None, &next, budget) || ok;
    }
    Some(ok)
}

pub(crate) fn run_input_in(app: &mut App, conn: &mut Conn, key: &str, input: &str, reply_to: Option<&str>, names: &[String], budget: &mut i32) -> bool {
    if input.trim().is_empty() {
        return false;
    }
    if input.starts_with('/') && !input.starts_with("//") {
        let body = &input[1..];
        let mut parts = body.split(' ').filter(|s| !s.is_empty());
        let raw_name = body.split(' ').next().unwrap_or("");
        let _ = parts.next();
        let mut args: Vec<String> = parts.map(String::from).collect();
        let lower = raw_name.to_lowercase();
        if let Some(done) = run_alias(app, conn, key, &lower, &args, names, budget) {
            return done;
        }
        let name = alias_target(&lower).to_string();
        if lower == "mute" || lower == "unmute" {
            args = vec![if lower == "mute" { "none".into() } else { "default".into() }];
        }
        let rest = body[raw_name.len()..].strip_prefix(' ').unwrap_or(&body[raw_name.len()..]).to_string();
        if !is_command(&name) {
            // Unknown commands go to the server verbatim, which is what most IRC users expect.
            conn.send(format!("{}{}", raw_name.to_uppercase(), if rest.is_empty() { String::new() } else { format!(" {rest}") }));
            return true;
        }
        let mut ctx = Ctx { app, conn, key, args, rest };
        return match run_command(&name, &mut ctx) {
            Ok(()) => true,
            Err(Usage(u)) => {
                ctx.app.toast(format!("Usage: {u}"), ToastKind::Error);
                false
            }
        };
    }
    let text = input.strip_prefix('/').filter(|_| input.starts_with("//")).unwrap_or(input);
    if key.is_empty() {
        app.toast("This is a server buffer \u{2014} use /raw to send commands", ToastKind::Error);
        return false;
    }
    if app.send_multiline_in(conn, key, text, reply_to) {
        return true;
    }
    for (i, line) in text.split('\n').filter(|l| !l.trim().is_empty()).enumerate() {
        app.send_message_in(conn, key, line, SendKind::Privmsg, false, if i == 0 { reply_to } else { None });
    }
    true
}

impl App {
    /// Run composer input in a buffer: a slash command, a user alias, or a
    /// message. Returns false if nothing was sent (a usage error, say).
    pub fn run_input(&mut self, id: &BufId, input: &str, reply_to: Option<&str>) -> bool {
        let (key, reply) = (id.key.clone(), reply_to.map(String::from));
        let mut budget = MAX_ALIAS_COMMANDS;
        self.with_conn(&id.conn, |app, conn| run_input_in(app, conn, &key, input, reply.as_deref(), &[], &mut budget)).unwrap_or(false)
    }
}
