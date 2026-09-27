// Slash commands and aliases (src/lib/commands.ts).

import Foundation

private struct UsageError: Error {
  let usage: String
}

private func need(_ cond: Bool, _ usage: String) throws {
  if !cond { throw UsageError(usage: usage) }
}

@MainActor
struct CommandContext {
  let app: AppModel
  let buf: Buffer
  let args: [String]
  let rest: String

  /// Send a command whose replies should come back to this buffer.
  func ask(_ line: String) { buf.conn.sendFrom(buf, line) }

  func arg(_ i: Int) -> String? { i < args.count ? args[i] : nil }

  func chan(_ explicit: String? = nil) throws -> String {
    if let explicit, buf.conn.isChannel(explicit) { return explicit }
    try need(buf.kind == .channel, "This command needs a channel")
    return buf.name
  }
}

struct Command {
  let usage: String
  let help: String
  let run: @MainActor (CommandContext) async throws -> Void
}

@MainActor
private func modeFor(_ sign: String, _ mode: String) -> @MainActor (CommandContext) async throws -> Void {
  { ctx in
    try need(!ctx.args.isEmpty, "/\(sign == "+" ? "" : "de")\(mode == "o" ? "op" : "voice") <nick…>")
    let chan = try ctx.chan()
    ctx.buf.conn.send(
      "MODE \(chan) \(sign)\(String(repeating: mode, count: ctx.args.count)) \(ctx.args.joined(separator: " "))")
  }
}

@MainActor
let commands: [String: Command] = [
  "join": Command(usage: "/join <#channel> [key]", help: "Join a channel") { ctx in
    guard var chan = ctx.arg(0) else { throw UsageError(usage: "/join <#channel> [key]") }
    if !ctx.buf.conn.isChannel(chan) { chan = "#" + chan }
    ctx.app.join(ctx.buf.conn, chan, key: ctx.arg(1))
  },
  "part": Command(usage: "/part [#channel] [reason]", help: "Leave a channel") { ctx in
    let explicit = ctx.arg(0).map(ctx.buf.conn.isChannel) ?? false
    let chan = try ctx.chan(explicit ? ctx.arg(0) : nil)
    let reason = explicit ? ctx.args.dropFirst().joined(separator: " ") : ctx.rest
    ctx.buf.conn.send(reason.isEmpty ? "PART \(chan)" : "PART \(chan) :\(reason)")
  },
  "close": Command(usage: "/close", help: "Close this buffer (parts channels)") { ctx in
    ctx.app.closeBuffer(ctx.buf)
  },
  "msg": Command(usage: "/msg <target> <text>", help: "Send a private message") { ctx in
    try need(ctx.args.count >= 2, "/msg <target> <text>")
    ctx.app.sendMessage(ctx.buf.conn.ensure(ctx.args[0]), ctx.args.dropFirst().joined(separator: " "))
  },
  "query": Command(usage: "/query <nick> [text]", help: "Open a conversation") { ctx in
    guard let nick = ctx.arg(0) else { throw UsageError(usage: "/query <nick> [text]") }
    let q = ctx.app.query(ctx.buf.conn, nick)
    if ctx.args.count > 1 { ctx.app.sendMessage(q, ctx.args.dropFirst().joined(separator: " ")) }
  },
  "me": Command(usage: "/me <action>", help: "Send an action") { ctx in
    try need(!ctx.rest.isEmpty && ctx.buf.kind != .server, "/me <action>")
    ctx.app.sendMessage(ctx.buf, ctx.rest, action: true)
  },
  "notice": Command(usage: "/notice <target> <text>", help: "Send a notice") { ctx in
    try need(ctx.args.count >= 2, "/notice <target> <text>")
    ctx.app.sendMessage(ctx.buf.conn.ensure(ctx.args[0]), ctx.args.dropFirst().joined(separator: " "), kind: "NOTICE")
  },
  "nick": Command(usage: "/nick <newnick>", help: "Change your nickname") { ctx in
    guard let n = ctx.arg(0) else { throw UsageError(usage: "/nick <newnick>") }
    ctx.buf.conn.send("NICK \(n)")
  },
  "topic": Command(usage: "/topic [text]", help: "Show or set the channel topic") { ctx in
    let chan = try ctx.chan()
    if ctx.rest.isEmpty { ctx.ask("TOPIC \(chan)") } else { ctx.buf.conn.send("TOPIC \(chan) :\(ctx.rest)") }
  },
  "whois": Command(usage: "/whois <nick>", help: "Look up a user") { ctx in
    let nick = ctx.arg(0) ?? (ctx.buf.kind == .query ? ctx.buf.name : "")
    try need(!nick.isEmpty, "/whois <nick>")
    ctx.ask("WHOIS \(nick) \(nick)")
  },
  "away": Command(usage: "/away [message]", help: "Mark yourself away (no message clears it)") { ctx in
    ctx.ask(ctx.rest.isEmpty ? "AWAY" : "AWAY :\(ctx.rest)")
  },
  "back": Command(usage: "/back", help: "Clear away status") { ctx in ctx.ask("AWAY") },
  "invite": Command(usage: "/invite <nick> [#channel]", help: "Invite someone to a channel") { ctx in
    guard let nick = ctx.arg(0) else { throw UsageError(usage: "/invite <nick> [#channel]") }
    ctx.buf.conn.send("INVITE \(nick) \(try ctx.chan(ctx.arg(1)))")
  },
  "kick": Command(usage: "/kick <nick> [reason]", help: "Remove someone from the channel") { ctx in
    guard let nick = ctx.arg(0) else { throw UsageError(usage: "/kick <nick> [reason]") }
    let reason = ctx.args.dropFirst().joined(separator: " ")
    ctx.buf.conn.send("KICK \(try ctx.chan()) \(nick)\(reason.isEmpty ? "" : " :" + reason)")
  },
  "ban": Command(usage: "/ban <nick|mask>", help: "Ban a nick or mask") { ctx in
    guard let who = ctx.arg(0) else { throw UsageError(usage: "/ban <nick|mask>") }
    let mask = who.contains("!") || who.contains("@") ? who : "\(who)!*@*"
    ctx.buf.conn.send("MODE \(try ctx.chan()) +b \(mask)")
  },
  "unban": Command(usage: "/unban <mask>", help: "Remove a ban") { ctx in
    guard let mask = ctx.arg(0) else { throw UsageError(usage: "/unban <mask>") }
    ctx.buf.conn.send("MODE \(try ctx.chan()) -b \(mask)")
  },
  "op": Command(usage: "/op <nick…>", help: "Give operator status", run: modeFor("+", "o")),
  "deop": Command(usage: "/deop <nick…>", help: "Take operator status", run: modeFor("-", "o")),
  "voice": Command(usage: "/voice <nick…>", help: "Give voice", run: modeFor("+", "v")),
  "devoice": Command(usage: "/devoice <nick…>", help: "Take voice", run: modeFor("-", "v")),
  "mode": Command(usage: "/mode [target] <modes> [args]", help: "Show or change modes") { ctx in
    let conn = ctx.buf.conn
    if let first = ctx.arg(0), conn.isChannel(first) || conn.isMe(first) {
      ctx.ask("MODE \(ctx.args.joined(separator: " "))")
    } else {
      try need(ctx.buf.kind == .channel, "/mode <target> <modes>")
      ctx.ask("MODE \(ctx.buf.name) \(ctx.args.joined(separator: " "))".trimmingCharacters(in: .whitespaces))
    }
  },
  "names": Command(usage: "/names", help: "Refresh the member list") { ctx in ctx.ask("NAMES \(try ctx.chan())") },
  "ctcp": Command(usage: "/ctcp <nick> <command> [args]", help: "Send a CTCP request") { ctx in
    try need(ctx.args.count >= 2, "/ctcp <nick> <command> [args]")
    let payload = ([ctx.args[1].uppercased()] + ctx.args.dropFirst(2)).joined(separator: " ")
    ctx.buf.conn.send("PRIVMSG \(ctx.args[0]) :\u{1}\(payload)\u{1}")
  },
  "raw": Command(usage: "/raw <line>", help: "Send a raw IRC line") { ctx in
    try need(!ctx.rest.isEmpty, "/raw <line>")
    ctx.ask(ctx.rest)
  },
  "quote": Command(usage: "/quote <line>", help: "Alias for /raw") { ctx in
    try need(!ctx.rest.isEmpty, "/quote <line>")
    ctx.ask(ctx.rest)
  },
  "clear": Command(usage: "/clear", help: "Clear this buffer") { ctx in
    ctx.buf.messages = []
    ctx.buf.history = .done
  },
  "reconnect": Command(usage: "/reconnect", help: "Reconnect this server") { ctx in
    try await ctx.app.backend.reconnect(ctx.buf.conn.serverId)
  },
  "quit": Command(usage: "/quit [reason]", help: "Disconnect from this server") { ctx in
    try await ctx.app.backend.disconnect(ctx.buf.conn.serverId, reason: ctx.rest.isEmpty ? nil : ctx.rest)
  },
  "motd": Command(usage: "/motd", help: "Show the message of the day") { ctx in ctx.ask("MOTD") },
  "list": Command(usage: "/list", help: "Browse the channels on this network") { ctx in
    ctx.app.browseChannels(ctx.buf.conn)
  },
  "search": Command(usage: "/search [text]", help: "Search message history") { ctx in
    ctx.app.searchQuery = ctx.rest
    ctx.app.searchOpen = true
  },
  "ignore": Command(usage: "/ignore [nick|mask]", help: "Hide messages from someone (no argument lists ignores)") { ctx in
    guard let who = ctx.arg(0) else {
      let list = ctx.app.settings.ignores
      ctx.buf.push(Line(type: .info, text: list.isEmpty ? "Not ignoring anyone" : "Ignoring: \(list.joined(separator: ", "))"))
      return
    }
    ctx.app.ignore(who)
    ctx.app.toast("Ignoring \(who)")
  },
  "unignore": Command(usage: "/unignore <nick|mask>", help: "Stop ignoring someone") { ctx in
    guard let who = ctx.arg(0) else { throw UsageError(usage: "/unignore <nick|mask>") }
    ctx.app.toast(ctx.app.unignore(who) ? "No longer ignoring \(who)" : "\(who) was not ignored")
  },
  "watch": Command(usage: "/watch [nick]", help: "Get told when someone comes online (no argument lists them)") { ctx in
    let conn = ctx.buf.conn
    try need(conn.canMonitor, "This server does not support MONITOR")
    if ctx.args.isEmpty {
      let list = conn.watched.map { "\($0) (\(conn.online[conn.fold($0)] == true ? "online" : "offline"))" }
      ctx.buf.push(Line(type: .info, text: list.isEmpty ? "Not watching anyone" : "Watching: \(list.joined(separator: ", "))"))
      return
    }
    for n in ctx.args { ctx.app.watch(conn, n) }
  },
  "unwatch": Command(usage: "/unwatch <nick…>", help: "Stop watching someone") { ctx in
    try need(!ctx.args.isEmpty, "/unwatch <nick…>")
    for n in ctx.args { ctx.app.unwatch(ctx.buf.conn, n) }
  },
  "notify": Command(usage: "/notify <all|mentions|none|default>", help: "Set notifications for this buffer") { ctx in
    let level = ctx.arg(0)?.lowercased() ?? ""
    try need(
      ctx.buf.kind != .server && ["all", "mentions", "none", "default"].contains(level),
      "/notify <all|mentions|none|default>")
    ctx.app.setNotify(ctx.buf, NotifyLevel(rawValue: level))
    ctx.app.toast("Notifications for \(ctx.buf.name): \(level)")
  },
  "rename": Command(usage: "/rename [#channel] <#newname> [reason]", help: "Rename a channel (needs server support)") {
    ctx in
    let conn = ctx.buf.conn
    try need(conn.has("draft/channel-rename"), "This server does not support renaming channels")
    let both = ctx.args.count >= 2 && conn.isChannel(ctx.args[0]) && conn.isChannel(ctx.args[1])
    let from = both ? ctx.args[0] : try ctx.chan()
    let to = both ? ctx.args[1] : ctx.arg(0)
    guard let to, conn.isChannel(to) else { throw UsageError(usage: "/rename [#channel] <#newname> [reason]") }
    let reason = ctx.args.dropFirst(both ? 2 : 1).joined(separator: " ")
    ctx.ask("RENAME \(from) \(to)\(reason.isEmpty ? "" : " :\(reason)")")
  },
  "register": Command(usage: "/register <email|*> <password>", help: "Register an account for your current nick") { ctx in
    let conn = ctx.buf.conn
    try need(
      conn.has("draft/account-registration"), "This server does not support registering here; try /msg NickServ help")
    try need(ctx.args.count == 2, "/register <email|*> <password>")
    let (email, password) = (ctx.args[0], ctx.args[1])
    let caps = (conn.capValues["draft/account-registration"] ?? "").split(separator: ",")
    try need(
      email != "*" || !caps.contains("email-required"),
      "This server needs an email address: /register <email> <password>")
    conn.registration = Registration(account: conn.nick, password: password)
    ctx.ask("REGISTER * \(email) :\(password)")
  },
  "verify": Command(usage: "/verify <account> <code>", help: "Confirm an account registration") { ctx in
    try need(ctx.buf.conn.has("draft/account-registration"), "This server does not support account registration")
    try need(ctx.args.count == 2, "/verify <account> <code>")
    ctx.ask("VERIFY \(ctx.args[0]) \(ctx.args[1])")
  },
  "help": Command(usage: "/help", help: "Show commands") { ctx in
    let text = commandInfos(ctx.app.settings.aliases).map {
      $0.usage.padding(toLength: max(32, $0.usage.count), withPad: " ", startingAt: 0) + " " + $0.help
    }.joined(separator: "\n")
    ctx.buf.push(Line(type: .info, data: ["motd": text, "title": "Commands"]))
  },
]

let commandAliases: [String: String] = [
  "j": "join", "p": "part", "q": "query", "w": "whois", "leave": "part", "m": "msg", "monitor": "watch",
  "mute": "notify", "unmute": "notify",
]

struct CommandInfo: Identifiable, Equatable {
  var name: String
  var usage: String
  var help: String
  var alias = false
  var id: String { name }
}

/// Built-in commands plus user aliases (which win on a name clash), for suggestions and /help.
@MainActor
func commandInfos(_ aliases: [String: String]) -> [CommandInfo] {
  let user = aliases.sorted { $0.key < $1.key }.map {
    CommandInfo(name: $0.key, usage: "/\($0.key)", help: "→ \($0.value)", alias: true)
  }
  let taken = Set(user.map(\.name))
  let builtin = commands.filter { !taken.contains($0.key) }.sorted { $0.key < $1.key }.map {
    CommandInfo(name: $0.key, usage: $0.value.usage, help: $0.value.help)
  }
  return user + builtin
}

struct AliasTrail {
  var names: [String] = []
  /// Shared across the whole expansion, so fan-out is bounded as well as depth.
  final class Budget { var left = 50 }
  var budget = Budget()
}

private let maxAliasDepth = 8

extension AppModel {
  /// Expand and run a user alias. Nil when `name` isn't one (or wraps the built-in it shadows).
  private func runAlias(_ buf: Buffer, _ name: String, _ args: [String], _ trail: AliasTrail) async -> Bool? {
    guard let body = settings.aliases[name] else { return nil }
    if trail.names.contains(name) {
      // An alias may wrap the built-in it shadows, e.g. /join → /join #$1.
      if commands[commandAliases[name] ?? name] != nil { return nil }
      toast("Alias /\(name) calls itself", .error)
      return false
    }
    if trail.names.count >= maxAliasDepth {
      toast("Aliases nest too deeply at /\(name)", .error)
      return false
    }
    let vars = AliasVars(nick: buf.conn.nick, channel: buf.kind == .channel ? buf.name : "", network: buf.conn.name)
    var next = trail
    next.names.append(name)
    var ok = false
    for cmd in splitCommands(body) {
      trail.budget.left -= 1
      if trail.budget.left < 0 {
        toast("Alias /\(trail.names.first ?? name) runs too many commands", .error)
        return ok
      }
      ok = await runInput(buf, expandCommand(cmd, args: args, vars: vars), trail: next) || ok
    }
    return ok
  }

  /// Run composer input. Returns false if nothing was sent.
  @discardableResult
  func runInput(_ buf: Buffer, _ input: String, replyTo: String? = nil, trail: AliasTrail = AliasTrail()) async -> Bool {
    if input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return false }
    if input.hasPrefix("/") && !input.hasPrefix("//") {
      let body = input.dropFirst()
      let rawName = String(body.prefix { $0 != " " })
      var args = body.dropFirst(rawName.count).split(separator: " ").map(String.init)
      let lower = rawName.lowercased()
      if let aliased = await runAlias(buf, lower, args, trail) { return aliased }
      let name = commandAliases[lower] ?? lower
      if lower == "mute" || lower == "unmute" { args = [lower == "mute" ? "none" : "default"] }
      var rest = String(body.dropFirst(rawName.count))
      if rest.hasPrefix(" ") { rest.removeFirst() }
      guard let cmd = commands[name] else {
        // Unknown commands go to the server verbatim, which is what most IRC users expect.
        buf.conn.send("\(rawName.uppercased())\(rest.isEmpty ? "" : " " + rest)")
        return true
      }
      do {
        try await cmd.run(CommandContext(app: self, buf: buf, args: args, rest: rest))
      } catch let e as UsageError {
        toast("Usage: \(e.usage)", .error)
        return false
      } catch {
        toast(String(describing: error), .error)
        return false
      }
      return true
    }
    let text = input.hasPrefix("//") ? String(input.dropFirst()) : input
    if buf.kind == .server {
      toast("This is a server buffer — use /raw to send commands", .error)
      return false
    }
    if sendMultiline(buf, text, replyTo: replyTo) { return true }
    let lines = text.split(separator: "\n").map(String.init).filter {
      !$0.trimmingCharacters(in: .whitespaces).isEmpty
    }
    for (i, line) in lines.enumerated() {
      sendMessage(buf, line, replyTo: i == 0 ? replyTo : nil)
    }
    return true
  }
}
