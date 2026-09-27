// Backfilling, saved buffers, MONITOR, unread counts and search
// (history.ts, saved.ts, monitor.ts, unread.ts and search.ts).

import Foundation

// ─── History ─────────────────────────────────────────────────────

private func lineKey(_ l: Line) -> String {
  l.msgid ?? "\(l.time)|\(l.type.rawValue)|\(l.nick)|\(l.text)"
}

/// `existing` plus the lines of `incoming` it doesn't already have, sorted by
/// time. Returns nil when nothing is new.
func mergeLines(_ existing: [Line], _ incoming: [Line]) -> [Line]? {
  if incoming.isEmpty { return nil }
  let seen = Set(existing.map(lineKey))
  // Lines without a msgid can differ by a few ms between the live copy and
  // the logged one (our own messages without echo-message), so also match
  // same-content lines that are close in time.
  var loose: [String: [Int]] = [:]
  for l in existing where l.msgid == nil {
    loose["\(l.type.rawValue)|\(l.nick)|\(l.text)", default: []].append(l.time)
  }
  let fresh = incoming.filter { l in
    if seen.contains(lineKey(l)) { return false }
    if l.msgid != nil { return true }
    return !(loose["\(l.type.rawValue)|\(l.nick)|\(l.text)"]?.contains { abs($0 - l.time) < 3000 } ?? false)
  }
  if fresh.isEmpty { return nil }
  return (existing + fresh).sorted { $0.time != $1.time ? $0.time < $1.time : $0.id < $1.id }
}

enum HistoryMode { case latest, before, after }

/// The CHATHISTORY command to page `buf` in `mode`, and the limit it asks for.
@MainActor
func chathistoryRequest(_ buf: Buffer, _ mode: HistoryMode, max: Int) -> (limit: Int, line: String) {
  if mode == .after, let last = buf.messages.last {
    let limit = min(1000, max)
    return (limit, "CHATHISTORY AFTER \(buf.name) timestamp=\(isoTime(last.time)) \(limit)")
  }
  let limit = min(100, max)
  if mode == .before, let first = buf.messages.first {
    return (limit, "CHATHISTORY BEFORE \(buf.name) timestamp=\(isoTime(first.time)) \(limit)")
  }
  return (limit, "CHATHISTORY LATEST \(buf.name) * \(limit)")
}

extension AppModel {
  func fetchHistory(_ buf: Buffer, _ mode: HistoryMode) async {
    let conn = buf.conn
    if buf.kind == .server { return }
    let remote = conn.has("draft/chathistory") && conn.status == .connected
    // The local log only fills gaps before what we have: anything newer
    // arrived live in this session.
    if !remote && (!settings.storeLogs || mode == .after) { return }
    if buf.history == .loading || (mode == .before && buf.history == .done) { return }
    let max = Int(conn.isupport["CHATHISTORY"] ?? "") ?? 100
    buf.history = .loading
    do {
      let msgs: [IrcMessage]
      let limit: Int
      if remote {
        let req = chathistoryRequest(buf, mode, max: max > 0 ? max : 100)
        limit = req.limit
        msgs = try await backend.history(conn.id, kind: "messages", target: buf.name, line: req.line)
      } else {
        limit = 100
        msgs = try await backend.localHistory(
          conn.id, target: buf.key, before: mode == .before ? buf.messages.first?.time : nil, limit: limit)
      }
      applyHistory(buf, msgs)
      buf.history = mode != .after && msgs.count < limit ? .done : .idle
    } catch {
      buf.history = .idle
      NSLog("hearth: history \(buf.name): \(error)")
    }
  }

  /// Load history around a message so it can be scrolled to.
  func loadAround(_ buf: Buffer, time: Int, msgid: String?) async {
    do {
      var msgs: [IrcMessage] = []
      if buf.conn.has("draft/chathistory") && buf.conn.status == .connected {
        let at = msgid.map { "msgid=\($0)" } ?? "timestamp=\(isoTime(time))"
        msgs = try await backend.history(
          buf.conn.id, kind: "messages", target: buf.name, line: "CHATHISTORY AROUND \(buf.name) \(at) 100")
      } else if settings.storeLogs {
        async let before = backend.localHistory(buf.conn.id, target: buf.key, before: time + 1, limit: 60)
        async let after = backend.localHistory(buf.conn.id, target: buf.key, after: time, limit: 60)
        msgs = try await before + after
      }
      applyHistory(buf, msgs)
    } catch {
      NSLog("hearth: jump: \(error)")
    }
  }

  private func applyHistory(_ buf: Buffer, _ msgs: [IrcMessage]) {
    buf.merge(msgs.compactMap { historyLine(buf.conn, $0) })
    for m in msgs {
      if m.command == "TAGMSG", let emoji = m.tags["+draft/react"], let parent = m.tags["+draft/reply"],
        let src = m.source, !isIgnored(buf.conn, src)
      {
        buf.react(msgid: parent, emoji: emoji, nick: src.nick)
      }
    }
  }

  /// Open query buffers for recent conversations the server remembers.
  func fetchTargets(_ conn: Conn) async {
    if !conn.has("draft/chathistory") || conn.isControl { return }
    let now = nowMs()
    let line =
      "CHATHISTORY TARGETS timestamp=\(isoTime(now + 60000)) timestamp=\(isoTime(now - 14 * 86_400_000)) 50"
    do {
      let msgs = try await backend.history(conn.id, kind: "targets", target: "*", line: line)
      for m in msgs {
        guard let target = m.param(1), !target.isEmpty, !conn.isChannel(target), !conn.isMe(target) else { continue }
        let b = conn.ensure(target, .query)
        if b.messages.isEmpty { Task { await fetchHistory(b, .latest) } }
      }
    } catch {
      NSLog("hearth: targets: \(error)")
    }
  }

  // ─── Saved buffers ─────────────────────────────────────────────

  func saveBuffersSoon(_ conn: Conn) {
    if !conn.buffersLoaded || conn.isControl { return }
    saveBufferTasks[conn.id]?.cancel()
    saveBufferTasks[conn.id] = Task { [backend] in
      try? await Task.sleep(for: .milliseconds(500))
      if Task.isCancelled { return }
      do {
        try await backend.saveBuffers(conn.id, savedList(conn.buffers.values))
      } catch {
        NSLog("hearth: save buffers: \(error)")
      }
    }
  }

  func restoreBuffers(_ conn: Conn) async {
    do {
      for s in try await backend.loadBuffers(conn.id) {
        // Bouncers keep their own channel list; only restore conversations there.
        if s.kind == "channel" && conn.netid != nil { continue }
        let b = conn.ensure(s.name, s.kind == "channel" ? .channel : .query)
        if s.kind == "channel" {
          b.wantJoined = true
          b.joinKey = s.key
        } else if b.messages.isEmpty {
          Task { await fetchHistory(b, .latest) }
        }
        maybeRestoreActive(conn, key: b.key)
      }
    } catch {
      NSLog("hearth: restore buffers: \(error)")
    }
    conn.buffersLoaded = true
  }

  // ─── MONITOR ───────────────────────────────────────────────────

  func isWatched(_ conn: Conn, _ nick: String) -> Bool {
    conn.watched.contains { conn.fold($0) == conn.fold(nick) }
  }

  /// How many nicks the server lets us MONITOR.
  private func monitorLimit(_ conn: Conn) -> Int { Int(conn.isupport["MONITOR"] ?? "") ?? Int.max }

  /// Everything we MONITOR: watched nicks and open conversations, by folded nick.
  private func monitored(_ conn: Conn) -> [(key: String, nick: String)] {
    var seen = Set<String>()
    var out: [(String, String)] = []
    for n in conn.watched + conn.buffers.values.filter({ $0.kind == .query }).map(\.name)
    where seen.insert(conn.fold(n)).inserted {
      out.append((conn.fold(n), n))
    }
    return out
  }

  func watch(_ conn: Conn, _ nick: String) {
    if isWatched(conn, nick) { return }
    if conn.canMonitor {
      let subs = monitored(conn)
      let limit = monitorLimit(conn)
      if !subs.contains(where: { $0.key == conn.fold(nick) }) && subs.count >= limit {
        return toast("Can't watch \(nick): \(conn.name) allows \(limit) nicknames", .error)
      }
    }
    settings.watch[conn.prefKey] = conn.watched + [nick]
    if conn.canMonitor { conn.send("MONITOR + \(nick)") }
  }

  func unwatch(_ conn: Conn, _ nick: String) {
    settings.watch[conn.prefKey] = conn.watched.filter { conn.fold($0) != conn.fold(nick) }
    if conn.canMonitor && conn.buffer(nick) == nil {
      conn.send("MONITOR - \(nick)")
      conn.online.removeValue(forKey: conn.fold(nick))
    }
  }

  /// Subscribe to presence for watched nicks and open conversations.
  func syncMonitor(_ conn: Conn) {
    guard conn.canMonitor else { return }
    conn.online = [:]
    for line in monitorCommands(monitored(conn).map(\.nick), limit: monitorLimit(conn)) { conn.send(line) }
  }

  // ─── Search ────────────────────────────────────────────────────

  /// Search the local log and, where supported, the bouncer. Newest first.
  func search(_ query: String, conn scope: Conn?, buffer: Buffer?) async -> [SearchHit] {
    let q = query.trimmingCharacters(in: .whitespaces)
    if q.isEmpty { return [] }
    let target = buffer.flatMap { $0.kind == .server ? nil : $0 }
    let remote = (scope.map { [$0] } ?? Array(conns.values)).filter {
      $0.has("soju.im/search") && $0.status == .connected && !$0.isControl
    }
    let backend = self.backend
    let storeLogs = settings.storeLogs
    let network = scope?.id
    let targetKey = target?.key
    return await withTaskGroup(of: [SearchHit].self) { group in
      if storeLogs {
        group.addTask { @MainActor in
          (try? await backend.search(q, network: network, target: targetKey, limit: 200)) ?? []
        }
      }
      for c in remote {
        group.addTask { @MainActor in await remoteSearch(c, q, target) }
      }
      var all: [SearchHit] = []
      for await hits in group { all += hits }
      return dedupeHits(all)
    }
  }
}

@MainActor
private func remoteSearch(_ c: Conn, _ q: String, _ target: Buffer?) async -> [SearchHit] {
  var attrs = "text=\(escapeAttr(q));limit=100"
  if let target { attrs += ";in=\(escapeAttr(target.name))" }
  guard let msgs = try? await c.app.backend.history(c.id, kind: "search", target: "*", line: "SEARCH \(attrs)") else {
    return []
  }
  return msgs.map { msg in
    let to = msg.param(0) ?? ""
    let peer = c.isChannel(to) || msg.source == nil || c.isMe(msg.source?.nick) ? to : msg.source!.nick
    return SearchHit(network: c.id, target: c.fold(peer), msg: msg)
  }
}

/// Drop hits found by both the log and the bouncer, then sort newest first.
func dedupeHits(_ all: [SearchHit]) -> [SearchHit] {
  var seen = Set<String>()
  var out: [SearchHit] = []
  for h in all {
    let k =
      h.msg.tags["msgid"].map { "\(h.network)|\($0)" }
      ?? "\(h.network)|\(h.target)|\(h.msg.tags["time"] ?? "")|\(h.msg.param(1) ?? "")"
    if seen.insert(k).inserted { out.append(h) }
  }
  let t = { (h: SearchHit) in h.msg.tags["time"].flatMap(parseIso) ?? 0 }
  return out.sorted { t($0) > t($1) }
}

/// Queries, plus channels we mean to be in.
@MainActor
func savedList(_ buffers: some Sequence<Buffer>) -> [SavedBuffer] {
  buffers.filter { $0.kind == .query || ($0.kind == .channel && $0.wantJoined) }.map {
    SavedBuffer(name: $0.name, kind: $0.kind.rawValue, key: $0.joinKey)
  }
}

/// JOIN commands for `chans`, ten per line.
func rejoinCommands(_ chans: [(name: String, joinKey: String)]) -> [String] {
  stride(from: 0, to: chans.count, by: 10).map { i in
    let group = chans[i..<min(i + 10, chans.count)]
    // Keyed channels must come first so keys line up by position.
    let keyed = group.filter { !$0.joinKey.isEmpty }
    let names = (keyed + group.filter { $0.joinKey.isEmpty }).map(\.name).joined(separator: ",")
    return keyed.isEmpty ? "JOIN \(names)" : "JOIN \(names) \(keyed.map(\.joinKey).joined(separator: ","))"
  }
}

/// MONITOR + commands for `nicks`, keeping each line's target list within 400 bytes.
func monitorCommands(_ nicks: [String], limit: Int = Int.max) -> [String] {
  var out: [String] = []
  var chunk: [String] = []
  var bytes = 0
  func flush() {
    if !chunk.isEmpty { out.append("MONITOR + \(chunk.joined(separator: ","))") }
    chunk = []
    bytes = 0
  }
  for n in nicks.prefix(limit) {
    let size = n.utf8.count
    if !chunk.isEmpty && bytes + 1 + size > 400 { flush() }
    bytes += (chunk.isEmpty ? 0 : 1) + size
    chunk.append(n)
  }
  flush()
  return out
}

// ─── Unread ──────────────────────────────────────────────────────

/// Messages from others after `readMarker`.
func countUnread(_ messages: [Line], _ readMarker: Int) -> Int {
  var n = 0
  for l in messages.reversed() {
    if l.time <= readMarker { break }
    if !l.isSelf && l.type.isTalk { n += 1 }
  }
  return n
}

/// Highlights after `readMarker`; with `everything`, every message counts.
func countHighlights(_ messages: [Line], _ readMarker: Int, everything: Bool) -> Int {
  var n = 0
  for l in messages.reversed() {
    if l.time <= readMarker { break }
    if !l.isSelf && (l.highlight || (everything && l.type.isTalk)) { n += 1 }
  }
  return n
}

@MainActor
func markBufferRead(_ buf: Buffer) {
  guard let last = buf.messages.last, last.time > buf.readMarker else { return }
  buf.readMarker = last.time
  if buf.kind == .server || !buf.conn.has("draft/read-marker") { return }
  // Debounced so scrolling through a burst sends one MARKREAD.
  buf.markTask?.cancel()
  buf.markTask = Task {
    try? await Task.sleep(for: .milliseconds(400))
    if Task.isCancelled { return }
    buf.markTask = nil
    buf.conn.send("MARKREAD \(buf.name) timestamp=\(isoTime(buf.readMarker))")
  }
}
