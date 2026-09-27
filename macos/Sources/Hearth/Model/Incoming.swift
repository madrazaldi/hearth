// Turns IRC messages from the server into state changes and buffer lines
// (src/lib/incoming.ts).

import Foundation

private struct Base {
  var time: Int
  var nick: String
  var isSelf: Bool
  var msgid: String?
  var account: String?
  var replyTo: String?
}

@MainActor
private func baseOf(_ conn: Conn, _ m: IrcMessage) -> Base {
  let nick = m.source?.nick ?? ""
  return Base(
    time: msgTime(m.tags), nick: nick, isSelf: conn.isMe(nick), msgid: m.tags["msgid"], account: m.tags["account"],
    replyTo: m.tags["+draft/reply"])
}

@MainActor
private func mk(
  _ b: Base, _ type: LineType, nick: String? = nil, text: String = "", highlight: Bool = false, target: String? = nil,
  data: [String: String]? = nil
) -> Line {
  Line(
    type: type, time: b.time, nick: nick ?? b.nick, text: text, isSelf: b.isSelf, highlight: highlight, msgid: b.msgid,
    target: target, account: b.account, data: data, replyTo: b.replyTo)
}

private func messageBody(_ m: IrcMessage) -> (type: LineType?, text: String) {
  let raw = m.param(1) ?? ""
  if let ctcp = parseCtcp(raw) {
    return ctcp.command == "ACTION" ? (.action, ctcp.args) : (nil, "")
  }
  return (m.command == "NOTICE" ? .notice : .privmsg, raw)
}

private func hostData(_ m: IrcMessage) -> [String: String] {
  ["host": "\(m.source?.user ?? "")@\(m.source?.host ?? "")"]
}

/// "ACCOUNT_REQUIRED" → "Account required", for replies without a description.
private func humanize(_ code: String) -> String {
  let s = code.replacingOccurrences(of: "_", with: " ").lowercased()
  return s.prefix(1).uppercased() + s.dropFirst()
}

// Commands whose failures are reported to whoever asked (history requests).
private let quietReplies: Set<String> = ["CHATHISTORY", "MARKREAD", "SEARCH"]

// Error numerics whose first parameter is the channel or nick they concern.
private let targetErrors: Set<String> = [
  "401", "403", "404", "405", "406", "442", "467", "471", "473", "474", "475", "477", "478", "482", "489",
]

extension AppModel {
  /// Build a line for a message replayed from history (server or local log).
  func historyLine(_ conn: Conn, _ m: IrcMessage) -> Line? {
    let base = baseOf(conn, m)
    switch m.command {
    case "PRIVMSG", "NOTICE":
      if isIgnored(conn, m.source) { return nil }
      let (type, text) = messageBody(m)
      guard let type else { return nil }
      return mk(base, type, text: text, highlight: !base.isSelf && isHighlight(conn, m.source, text))
    case "JOIN": return mk(base, .join, data: hostData(m))
    case "PART": return mk(base, .part, text: m.param(1) ?? "")
    case "QUIT": return mk(base, .quit, text: m.param(0) ?? "")
    case "KICK": return mk(base, .kick, text: m.param(2) ?? "", target: m.param(1))
    case "NICK": return mk(base, .nick, target: m.param(0))
    case "TOPIC": return mk(base, .topic, text: m.param(1) ?? "")
    case "MODE": return mk(base, .mode, text: m.params.dropFirst().joined(separator: " "))
    default: return nil
    }
  }

  /// Walk a channel mode string, calling `each` for every mode letter with its
  /// argument (if it takes one).
  fileprivate func walkModes(
    _ conn: Conn, _ modestr: String, _ args: [String], _ each: (_ adding: Bool, _ mode: Character, _ arg: String?) -> Void
  ) {
    let groups = (conn.isupport["CHANMODES"] ?? "beI,k,l,imnpst").split(separator: ",", omittingEmptySubsequences: false)
    let (a, b, c) = (groups.first ?? "", groups.count > 1 ? groups[1] : "", groups.count > 2 ? groups[2] : "")
    let pmodes = conn.prefix.modes
    var adding = true
    var ai = 0
    for ch in modestr {
      if ch == "+" || ch == "-" {
        adding = ch == "+"
        continue
      }
      let takesArg = pmodes.contains(ch) || a.contains(ch) || b.contains(ch) || (c.contains(ch) && adding)
      var arg: String?
      if takesArg {
        arg = ai < args.count ? args[ai] : nil
        ai += 1
      }
      each(adding, ch, arg)
    }
  }

  func handleMessage(_ conn: Conn, _ m: IrcMessage) {
    let src = m.source
    let p = m.params
    let base = baseOf(conn, m)
    let nick = base.nick
    let isSelf = base.isSelf

    switch m.command {
    case "PRIVMSG", "NOTICE":
      return onMessage(conn, m, base)

    case "TAGMSG":
      if isIgnored(conn, src) { return }
      let target = m.param(0) ?? ""
      guard let buf = conn.isChannel(target) ? conn.buffer(target) : conn.buffer(isSelf ? target : nick) else { return }
      if let emoji = m.tags["+draft/react"], let parent = m.tags["+draft/reply"] {
        return buf.react(msgid: parent, emoji: emoji, nick: nick)
      }
      guard let typing = m.tags["+typing"], !isSelf, settings.showTyping else { return }
      if typing == "done" {
        buf.typing.removeValue(forKey: nick)
      } else {
        buf.typing[nick] = nowMs() + (typing == "active" ? 6000 : 30000)
      }

    case "JOIN":
      let chan = m.param(0) ?? ""
      let buf = conn.ensure(chan, .channel)
      if isSelf {
        buf.joined = true
        buf.wantJoined = true
        if let key = conn.pendingKeys.removeValue(forKey: buf.key) { buf.joinKey = key }
        buf.members = [:]
        if pendingActive[conn.id] == conn.fold(chan) {
          pendingActive.removeValue(forKey: conn.id)
          open(buf)
        } else {
          maybeRestoreActive(conn, key: buf.key)
        }
        saveBuffersSoon(conn)
        Task { await fetchHistory(buf, buf.messages.isEmpty ? .latest : .after) }
        return
      }
      buf.setMember(
        Member(
          nick: nick, prefix: "", away: false,
          account: conn.has("extended-join") && m.param(1) != "*" ? m.param(1) : nil, user: src?.user,
          host: src?.host, realname: m.param(2)))
      buf.push(mk(base, .join, data: hostData(m)))

    case "PART":
      guard let buf = conn.buffer(m.param(0) ?? "") else { return }
      if isSelf {
        buf.joined = false
        buf.wantJoined = false
        buf.members = [:]
        saveBuffersSoon(conn)
      } else {
        buf.removeMember(nick)
      }
      buf.push(mk(base, .part, text: m.param(1) ?? ""))

    case "KICK":
      guard let buf = conn.buffer(m.param(0) ?? ""), let victim = m.param(1) else { return }
      if conn.isMe(victim) {
        buf.joined = false
        buf.members = [:]
      } else {
        buf.removeMember(victim)
      }
      buf.push(mk(base, .kick, text: m.param(2) ?? "", highlight: conn.isMe(victim), target: victim))

    case "QUIT":
      conn.metadata.removeValue(forKey: conn.fold(nick))
      for buf in conn.buffers.values {
        if buf.removeMember(nick) || (buf.kind == .query && conn.fold(buf.name) == conn.fold(nick)) {
          buf.push(mk(base, .quit, text: m.param(0) ?? ""))
          buf.typing.removeValue(forKey: nick)
        }
      }

    case "NICK":
      guard let next = m.param(0) else { return }
      if isSelf { conn.nick = next }
      if let meta = conn.metadata.removeValue(forKey: conn.fold(nick)) { conn.metadata[conn.fold(next)] = meta }
      // Snapshot: renaming a query re-inserts it into the map being walked.
      for buf in Array(conn.buffers.values) {
        let mem = buf.member(nick)
        let isQuery = buf.kind == .query && conn.fold(buf.name) == conn.fold(nick)
        if var mem {
          buf.removeMember(nick)
          mem.nick = next
          buf.setMember(mem)
        }
        if mem != nil || isQuery || (isSelf && buf.kind == .query) { buf.push(mk(base, .nick, target: next)) }
        // A case-only change folds to the same key, so the buffer is still "free".
        if isQuery && (conn.buffer(next) ?? buf) === buf {
          conn.buffers.removeValue(forKey: buf.key)
          buf.name = next
          conn.buffers[conn.fold(next)] = buf
          saveBuffersSoon(conn)
        }
      }
      if isSelf { conn.server.push(mk(base, .nick, target: next)) }

    case "MODE":
      return onMode(conn, m, base)

    case "TOPIC":
      guard let buf = conn.buffer(m.param(0) ?? "") else { return }
      buf.topic = m.param(1) ?? ""
      buf.topicBy = nick
      buf.topicTime = base.time
      buf.push(mk(base, .topic, text: m.param(1) ?? ""))

    case "INVITE":
      if isIgnored(conn, src) { return }
      let who = m.param(0) ?? ""
      let chan = m.param(1) ?? ""
      let text = "invited \(conn.isMe(who) ? "you" : who) to \(chan)"
      here(conn).push(mk(base, .invite, text: text, highlight: conn.isMe(who), target: chan))

    case "AWAY":
      let away = !(m.param(0) ?? "").isEmpty
      for buf in conn.buffers.values {
        if var mem = buf.member(nick), mem.away != away {
          mem.away = away
          buf.setMember(mem)
        }
      }
      if isSelf { conn.away = away }

    case "ACCOUNT":
      updateMember(conn, nick) { $0.account = m.param(0) == "*" ? nil : m.param(0) }
    case "CHGHOST":
      updateMember(conn, nick) {
        $0.user = m.param(0)
        $0.host = m.param(1)
      }
    case "SETNAME":
      updateMember(conn, nick) { $0.realname = m.param(0) }

    case "MARKREAD":
      guard let buf = conn.buffer(m.param(0) ?? ""), let stamp = m.param(1), stamp.hasPrefix("timestamp="),
        let ts = parseIso(String(stamp.dropFirst(10)))
      else { return }
      if ts > buf.readMarker { buf.readMarker = ts }

    case "BOUNCER":
      if m.param(0) == "NETWORK" && conn.isControl, let id = m.param(1) {
        if m.param(2) == "*" {
          conn.networks.removeValue(forKey: id)
        } else {
          conn.networks[id, default: [:]].merge(parseAttrs(m.param(2) ?? "")) { $1 }
        }
      } else if m.param(0) == "ADDNETWORK" {
        toast("Network added")
      }

    case "REDACT":
      let target = m.param(0) ?? ""
      guard let buf = conn.isChannel(target) ? conn.buffer(target) : conn.buffer(isSelf ? target : nick),
        let id = m.param(1)
      else { return }
      buf.messages = buf.messages.map {
        guard $0.msgid == id else { return $0 }
        var l = $0
        l.redacted = true
        l.reactions = []
        return l
      }

    case "ERROR":
      conn.server.push(mk(base, .error, nick: "", text: m.param(0) ?? "Connection error"))

    case "FAIL", "WARN", "NOTE":
      return onStandardReply(conn, m, base)

    case "RENAME":
      return onRename(conn, m, base)

    case "METADATA":
      // METADATA <target> <key> <visibility> [:<value>]; no value means unset.
      if p.count >= 3 { conn.setMetadata(p[0], p[1], p.count >= 4 ? p[3] : nil) }

    case "REGISTER", "VERIFY":
      return onRegistration(conn, m, base)

    case "PING", "PONG", "CAP", "AUTHENTICATE", "BATCH", "ACK":
      return

    default:
      if m.command.count == 3 && m.command.allSatisfy(\.isNumber) { onNumeric(conn, m, time: base.time) }
    }
  }

  private func onMessage(_ conn: Conn, _ m: IrcMessage, _ base: Base) {
    let nick = base.nick
    let src = m.source
    let statusmsg = conn.isupport["STATUSMSG"] ?? ""
    var target = m.param(0) ?? ""
    while let f = target.first, statusmsg.contains(f), conn.isChannel(String(target.dropFirst())) {
      target.removeFirst()
    }
    let raw = m.param(1) ?? ""
    let ctcp = parseCtcp(raw)
    let isNotice = m.command == "NOTICE"

    // Server-originated notices: no source, a server name as source, or pre-registration.
    let fromServer = src == nil || target == "*" || (src?.user == nil && (src!.nick.contains(".") || conn.nick.isEmpty))
    if fromServer && !conn.isChannel(target) {
      conn.server.push(mk(base, .notice, text: raw))
      return
    }
    if isIgnored(conn, src) { return }

    if let ctcp, ctcp.command != "ACTION" {
      if !isNotice && !base.isSelf {
        // Answer at most a few CTCPs per 10s: replies share our send queue.
        let now = nowMs()
        conn.ctcpTimes = conn.ctcpTimes.filter { now - $0 < 10_000 }
        if conn.ctcpTimes.count >= 3 {
          if now - conn.ctcpWarned > 60_000 {
            conn.ctcpWarned = now
            conn.server.push(mk(base, .info, text: "Ignoring a flood of CTCP requests"))
          }
          return
        }
        conn.ctcpTimes.append(now)
        if let reply = ctcpReply(ctcp.command, ctcp.args) {
          conn.send("NOTICE \(nick) :\u{1}\(ctcp.command)\(reply.isEmpty ? "" : " " + reply)\u{1}")
        }
        here(conn).push(mk(base, .info, text: "CTCP \(ctcp.command) request from \(nick)"))
      } else if isNotice {
        here(conn).push(mk(base, .info, text: "CTCP \(ctcp.command) reply from \(nick): \(ctcp.args)"))
      }
      return
    }

    let type: LineType = ctcp != nil ? .action : isNotice ? .notice : .privmsg
    let text = ctcp?.args ?? raw
    let isChan = conn.isChannel(target)
    let buf: Buffer
    if isChan {
      buf = conn.ensure(target, .channel)
    } else {
      let peer = base.isSelf ? target : nick
      let existing = conn.buffer(peer)
      if isNotice && existing == nil {
        // Private notices without a conversation land where the user is looking.
        let where_ = here(conn, m.tags["label"])
        where_.push(mk(base, type, text: text, highlight: !base.isSelf, target: base.isSelf ? target : nil))
        if !base.isSelf { maybeNotify(conn, where_, nick: nick, text: text, important: true) }
        return
      }
      buf = existing ?? conn.ensure(peer, .query)
    }

    let highlight = !base.isSelf && isHighlight(conn, src, text)
    buf.typing.removeValue(forKey: nick)
    if !isChan && buf.member(nick) == nil && !base.isSelf {
      buf.setMember(Member(nick: nick, prefix: "", away: false, user: src?.user, host: src?.host))
    }
    buf.push(mk(base, type, text: text, highlight: highlight))
    if !base.isSelf { maybeNotify(conn, buf, nick: nick, text: text, important: highlight || !isChan) }
  }

  /// IRCv3 standard replies: FAIL/WARN/NOTE <command> <code> [<context>…] :<description>.
  /// They go to the buffer that sent the command (by label), else to a buffer
  /// named in the context, else where the user is looking.
  private func onStandardReply(_ conn: Conn, _ m: IrcMessage, _ base: Base) {
    let p = m.params
    let command = m.param(0) ?? "*"
    let code = m.param(1) ?? ""
    let context = p.count > 3 ? Array(p[2..<(p.count - 1)]) : []
    let last = p.count > 2 ? p[p.count - 1] : ""
    let desc = last.isEmpty ? humanize(code) : last
    let kind = m.command
    if quietReplies.contains(command) { return }
    if command == "BOUNCER" && kind == "FAIL" { return toast("Bouncer: \(desc)", .error) }
    if command == "REGISTER" || command == "VERIFY" { return onRegistration(conn, m, base) }
    // Our automatic metadata subscription failing is nothing the user can act on.
    if command == "METADATA" && m.tags["label"] == nil && ["TOO_MANY_SUBS", "KEY_INVALID", "RATE_LIMITED"].contains(code) {
      return
    }
    let label = m.tags["label"]
    let about = label == nil ? context.lazy.compactMap { $0.isEmpty || $0 == "*" ? nil : conn.buffer($0) }.first : nil
    let where_ = label != nil ? here(conn, label) : (about ?? (kind == "NOTE" ? conn.server : here(conn)))
    let what = command == "*" ? "" : command
    let text =
      kind == "FAIL"
      ? (what.isEmpty ? desc : "\(what) failed: \(desc)")
      : kind == "WARN" ? "Warning\(what.isEmpty ? "" : " (\(what))"): \(desc)" : desc
    where_.push(mk(base, kind == "FAIL" ? .error : .info, nick: "", text: text, data: ["code": code]))
  }

  /// draft/channel-rename: RENAME <old> <new> [:<reason>]. The buffer keeps its history and state.
  private func onRename(_ conn: Conn, _ m: IrcMessage, _ base: Base) {
    let from = m.param(0) ?? ""
    let to = m.param(1) ?? ""
    let reason = m.param(2) ?? ""
    guard let buf = conn.buffer(from), !to.isEmpty else { return }
    if let clash = conn.buffer(to), clash !== buf {
      // A stale buffer under the new name (e.g. parted earlier) gives way.
      conn.buffers.removeValue(forKey: clash.key)
      if active === clash { open(buf) }
    }
    let oldPref = buf.prefKey
    conn.buffers.removeValue(forKey: buf.key)
    buf.name = to
    conn.buffers[conn.fold(to)] = buf
    if let level = settings.bufferNotify[oldPref] {
      settings.bufferNotify[oldPref] = nil
      settings.bufferNotify[buf.prefKey] = level
    }
    if active === buf { rememberActive() }
    saveBuffersSoon(conn)
    let who = !base.nick.isEmpty && !base.isSelf ? "\(base.nick) renamed" : "Renamed"
    buf.push(mk(base, .info, nick: "", text: "\(who) \(from) to \(to)\(reason.isEmpty ? "" : " (\(reason))")"))
  }

  /// draft/account-registration replies:
  ///   REGISTER SUCCESS|VERIFICATION_REQUIRED <account> :<message>
  ///   VERIFY SUCCESS <account> :<message>
  ///   FAIL REGISTER|VERIFY <code> [<account>] :<description>
  private func onRegistration(_ conn: Conn, _ m: IrcMessage, _ base: Base) {
    let p = m.params
    let fail = m.command == "FAIL"
    let command = fail ? (m.param(0) ?? "") : m.command
    let status = (fail ? m.param(1) : m.param(0)) ?? ""
    let account = fail ? "" : (m.param(1) ?? "")
    let desc = p.count > 2 ? p[p.count - 1] : ""
    let where_ = here(conn, m.tags["label"])
    func say(_ text: String, _ type: LineType = .info) { where_.push(mk(base, type, nick: "", text: text)) }

    if fail {
      let what = command == "VERIFY" ? "Verification" : "Registration"
      say("\(what) failed: \(desc.isEmpty ? humanize(status) : desc)", .error)
      // A wrong code can be retried; a refused registration is over.
      if command == "REGISTER" { conn.registration = nil }
      return
    }
    if status == "VERIFICATION_REQUIRED" {
      conn.registration?.account = account
      say("\(desc.isEmpty ? "Account \(account) needs verifying" : desc) — then type /verify \(account) <code>")
      return
    }
    guard status == "SUCCESS" else { return }
    say(command == "VERIFY" ? "Account \(account) verified" : "Account \(account) registered")
    let creds = conn.registration
    conn.registration = nil
    // Offer SASL only for a server we connect to directly: a bouncer network's
    // config is the bouncer's own login.
    guard let creds, conn.netid == nil, !conn.isControl, conn.config != nil else { return }
    let name = account.isEmpty ? creds.account : account
    toast(
      "Registered \(name). Sign in automatically from now on?", .info,
      action: Toast.Action(label: "Save login") { [weak self] in
        Task { await self?.saveLogin(conn, account: name, password: creds.password) }
      })
  }

  private func ctcpReply(_ cmd: String, _ args: String) -> String? {
    switch cmd {
    case "VERSION": return "Hearth — a modern IRC client"
    case "PING": return String(args.prefix(64))
    case "TIME": return Date().formatted(date: .complete, time: .complete)
    case "CLIENTINFO": return "ACTION CLIENTINFO PING TIME VERSION"
    default: return nil
    }
  }

  private func updateMember(_ conn: Conn, _ nick: String, _ patch: (inout Member) -> Void) {
    for buf in conn.buffers.values {
      if var mem = buf.member(nick) {
        patch(&mem)
        buf.setMember(mem)
      }
    }
  }

  private func onMode(_ conn: Conn, _ m: IrcMessage, _ base: Base) {
    let target = m.param(0) ?? ""
    let modestr = m.param(1) ?? ""
    let args = Array(m.params.dropFirst(2))
    let text = ([modestr] + args).joined(separator: " ")
    guard conn.isChannel(target) else {
      if conn.isMe(target) { conn.server.push(mk(base, .mode, text: text)) }
      return
    }
    guard let buf = conn.buffer(target) else { return }
    let prefix = conn.prefix
    walkModes(conn, modestr, args) { adding, ch, arg in
      if let pi = prefix.modes.firstIndex(of: ch) {
        guard let arg, var mem = buf.member(arg) else { return }
        let sym = prefix.symbols[pi]
        var p = mem.prefix.filter { $0 != sym }
        if adding { p = String((p + String(sym)).sorted { prefix.rank(of: $0) < prefix.rank(of: $1) }) }
        mem.prefix = p
        buf.setMember(mem)
      } else if ch == "k" {
        // Servers hide the key from non-ops with "*".
        if !adding {
          buf.joinKey = ""
        } else if let arg, arg != "*" {
          buf.joinKey = arg
        }
        saveBuffersSoon(conn)
      }
    }
    buf.push(mk(base, .mode, text: text))
  }

  private func onNumeric(_ conn: Conn, _ m: IrcMessage, time: Int) {
    let p = m.params
    let rest = Array(p.dropFirst())
    let trailing = p.last ?? ""
    let label = m.tags["label"]
    func info(_ text: String, _ buf: Buffer? = nil, _ type: LineType = .info) {
      (buf ?? conn.server).push(Line(type: type, time: time, text: text))
    }
    func arg(_ i: Int) -> String { m.param(i) ?? "" }

    switch m.command {
    case "001":
      return info(trailing)
    case "002", "003", "004", "005", "250", "251", "252", "253", "254", "255", "265", "266", "396", "422", "329", "903",
      "732", "733", "760", "762", "770", "771", "772":
      return
    case "375":
      conn.motd = []
      return
    case "372":
      var line = trailing
      if line.hasPrefix("- ") {
        line.removeFirst(2)
      } else if line.hasPrefix("-") {
        line.removeFirst()
      }
      conn.motd.append(line)
      return
    case "376":
      if !conn.motd.isEmpty {
        let buf = label != nil ? here(conn, label) : conn.server
        buf.push(Line(type: .info, time: time, data: ["motd": conn.motd.joined(separator: "\n")]))
      }
      conn.motd = []
      return
    case "221":
      return info("Your user modes: \(arg(1))", here(conn, label))
    case "324":
      if let buf = conn.buffer(arg(1)) {
        buf.modes = p.dropFirst(2).joined(separator: " ")
        walkModes(conn, arg(2), Array(p.dropFirst(3))) { adding, ch, a in
          if ch == "k", adding, let a, a != "*", buf.joinKey != a {
            buf.joinKey = a
            saveBuffersSoon(conn)
          }
        }
      }
      return
    case "331":
      conn.buffer(arg(1))?.topic = ""
      return
    case "332":
      conn.buffer(arg(1))?.topic = trailing
      return
    case "333":
      if let buf = conn.buffer(arg(1)) {
        buf.topicBy = String(arg(2).split(separator: "!").first ?? "")
        buf.topicTime = (Int(arg(3)) ?? 0) * 1000
      }
      return
    case "353":
      guard let buf = conn.buffer(arg(2)) else { return }
      var acc = buf.namesAcc ?? [:]
      let syms = conn.prefix.symbols
      for entry in trailing.split(separator: " ") {
        let prefix = String(entry.prefix { syms.contains($0) })
        let mask = parseMask(String(entry.dropFirst(prefix.count)))
        let prev = buf.member(mask.nick)
        acc[conn.fold(mask.nick)] = Member(
          nick: mask.nick, prefix: prefix, away: prev?.away ?? false, account: prev?.account, user: mask.user,
          host: mask.host)
      }
      buf.namesAcc = acc
      return
    case "366":
      guard let buf = conn.buffer(arg(1)), let acc = buf.namesAcc else { return }
      buf.members = acc
      buf.namesAcc = nil
      return
    case "301":
      let nick = arg(1)
      if conn.whois[conn.fold(nick)] != nil {
        conn.whois[conn.fold(nick)]?["away"] = trailing
      } else if let buf = conn.buffer(nick) {
        info("\(nick) is away: \(trailing)", buf)
      }
      return
    case "305":
      conn.away = false
      conn.autoAway = false
      return info("You are no longer marked as away", here(conn, label))
    case "306":
      conn.away = true
      return info(
        conn.autoAway ? "You have been marked as away (window in the background)" : "You have been marked as away",
        here(conn, label))
    case "311", "314":
      let nick = arg(1)
      conn.whois[conn.fold(nick)] = ["nick": nick, "user": arg(2), "host": arg(3), "realname": trailing]
      return
    case "312", "313", "317", "319", "320", "330", "338", "378", "379", "671", "276":
      let key = conn.fold(arg(1))
      guard var w = conn.whois[key] else { return info(rest.joined(separator: " "), here(conn, label)) }
      switch m.command {
      case "312": w["server"] = "\(arg(2)) — \(trailing)"
      case "313": w["oper"] = trailing
      case "317":
        w["idle"] = arg(2)
        if !arg(3).isEmpty && arg(3).allSatisfy(\.isNumber) { w["signon"] = arg(3) }
      case "319": w["channels"] = ((w["channels"] ?? "") + " " + trailing).trimmingCharacters(in: .whitespaces)
      case "330": w["account"] = arg(2)
      case "671", "276": w["secure"] = trailing
      case "338": w["actual"] = p.count > 3 ? p[2..<(p.count - 1)].joined(separator: " ") : ""
      default: w["extra\(m.command)"] = trailing
      }
      conn.whois[key] = w
      return
    case "318", "369":
      let key = conn.fold(arg(1))
      if let w = conn.whois.removeValue(forKey: key) {
        here(conn, label).push(Line(type: .whois, time: time, nick: w["nick"] ?? "", data: w))
      }
      return
    case "341":
      return info("Invited \(arg(1)) to \(arg(2))", conn.buffer(arg(2)) ?? here(conn, label))

    // Channel list
    case "321":
      conn.listAcc = []
      conn.channelList = ChannelList(loading: true, items: conn.channelList?.items ?? [], at: nowMs())
      return
    case "322":
      var acc = conn.listAcc ?? []
      acc.append(ChannelInfo(name: arg(1), users: Int(arg(2)) ?? 0, topic: trailing))
      conn.listAcc = acc
      // Publish progress in steps; a big network lists tens of thousands.
      if acc.count % 1000 == 0 { conn.channelList = ChannelList(loading: true, items: acc, at: nowMs()) }
      return
    case "323":
      conn.channelList = ChannelList(loading: false, items: conn.listAcc ?? [], at: nowMs())
      conn.listAcc = nil
      return

    // MONITOR
    case "730", "731":
      let online = m.command == "730"
      for entry in trailing.split(separator: ",") {
        let nick = parseMask(entry.trimmingCharacters(in: .whitespaces)).nick
        if nick.isEmpty { continue }
        let key = conn.fold(nick)
        let was = conn.online[key]
        conn.online[key] = online
        guard let was, was != online, isWatched(conn, nick) else { continue }
        let text = "\(nick) is \(online ? "online" : "offline")"
        info(text)
        if let q = conn.buffer(nick) { info(text, q) }
      }
      return

    // draft/metadata-2
    case "761":
      // <client> <target> <key> <visibility> :<value>
      conn.setMetadata(arg(1), arg(2), m.param(4) ?? "")
      return
    case "766":
      conn.setMetadata(arg(1), arg(2), nil)
      return
    case "774":
      // RPL_METADATASYNCLATER <client> <target> [<retry after>]: ask again later.
      let target = arg(1)
      let secs = min(max(Int(arg(2)) ?? 5, 1), 300)
      Task {
        try? await Task.sleep(for: .seconds(secs))
        if conn.status == .connected { conn.send("METADATA \(target) SYNC") }
      }
      return
    case "734":
      return info("Can't watch more nicknames: the server allows \(arg(1))", conn.server, .error)

    case "900":
      return info(trailing)
    case "902", "904", "905", "906":
      return info("SASL: \(trailing)", conn.server, .error)
    case "433":
      return info("Nickname \(arg(1)) is already in use", here(conn, label), .error)
    default:
      break
    }

    let code = Int(m.command) ?? 0
    if code >= 400 && code < 600 {
      // Unlabeled errors about a channel or nick belong with that conversation,
      // not whatever the user happens to be looking at.
      let about = label == nil && targetErrors.contains(m.command) ? conn.buffer(arg(1)) : nil
      let text = rest.enumerated().filter { $0.offset < rest.count - 1 || !$0.element.isEmpty }.map(\.element)
        .joined(separator: " ")
      return info(text, about ?? here(conn, label), .error)
    }
    info(rest.joined(separator: " "), label != nil ? here(conn, label) : conn.server)
  }
}
