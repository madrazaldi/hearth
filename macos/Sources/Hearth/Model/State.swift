// App state: connections, buffers and lines (src/lib/state.svelte.ts).
// Views observe these classes directly.

import Foundation
import Observation

// Views are lazy, so the cap is about memory and the O(n) derivations per new
// line (unread counts, msgid index, timeline), not view count.
private let maxLines = 20_000
/// Trimmed in chunks so a busy channel doesn't slice the array on every line.
private let trimTo = 18_000

enum BufferKind: String { case server, channel, query }

struct Member: Equatable, Identifiable {
  var nick: String
  var prefix: String
  var away: Bool
  var account: String?
  var user: String?
  var host: String?
  var realname: String?
  var id: String { nick }
}

/// What someone has published about themselves with draft/metadata-2.
struct Profile {
  /// An image URL; load it with `AppModel.avatar`, never directly.
  var avatar: String?
  var displayName: String?
}

struct ChannelInfo: Identifiable, Equatable {
  var name: String
  var users: Int
  var topic: String
  var id: String { name }
}

struct ChannelList: Equatable {
  var loading: Bool
  var items: [ChannelInfo]
  var at: Int
}

enum HistoryState { case idle, loading, done }

struct Focus: Equatable {
  var time: Int
  var msgid: String?
  var seq: Int
}

@MainActor
@Observable
final class Buffer: Identifiable, Hashable {
  @ObservationIgnored unowned let conn: Conn
  let kind: BufferKind
  var name: String
  var messages: [Line] = [] {
    didSet { msgidIndex = nil }
  }
  var members: [String: Member] = [:] {
    didSet { sortedCache = nil }
  }
  var topic = ""
  var topicBy = ""
  var topicTime = 0
  var modes = ""
  var joined = false
  var readMarker = 0
  var divider = 0
  var history = HistoryState.idle
  /// nick → when the typing indicator expires.
  var typing: [String: Int] = [:]
  /// The message the composer is replying to.
  var replyTo: Line?
  /// A line to scroll to and flash, set by search and reply quotes.
  var focus: Focus?

  // Non-reactive UI memory.
  @ObservationIgnored var wantJoined = false
  /// Channel key, remembered for rejoining.
  @ObservationIgnored var joinKey = ""
  @ObservationIgnored var draft = ""
  @ObservationIgnored var namesAcc: [String: Member]?
  @ObservationIgnored var lastTypingSent = 0
  @ObservationIgnored var markTask: Task<Void, Never>?
  @ObservationIgnored private var msgidIndex: [String: Int]?
  @ObservationIgnored private var sortedCache: [Member]?

  init(conn: Conn, kind: BufferKind, name: String) {
    self.conn = conn
    self.kind = kind
    self.name = name
    self.readMarker = conn.caps.contains("draft/read-marker") ? 0 : nowMs()
  }

  nonisolated static func == (a: Buffer, b: Buffer) -> Bool { a === b }
  nonisolated func hash(into h: inout Hasher) { h.combine(ObjectIdentifier(self)) }

  var key: String { kind == .server ? "" : conn.fold(name) }
  var id: String { "\(conn.id)\u{0}\(key)" }
  /// Stable across restarts; used for saved preferences.
  var prefKey: String { "\(conn.prefKey)|\(key)" }

  var level: NotifyLevel? { conn.app.settings.bufferNotify[prefKey] }

  var unread: Int { countUnread(messages, readMarker) }

  var highlights: Int {
    if level == NotifyLevel.none { return 0 }
    return countHighlights(messages, readMarker, everything: kind == .query || level == .all)
  }

  var sortedMembers: [Member] {
    _ = members
    if let s = sortedCache { return s }
    let p = conn.prefix
    let rank = { (m: Member) in m.prefix.first.map { p.rank(of: $0) } ?? p.symbols.count }
    let s = members.values.map { ($0, rank($0), conn.fold($0.nick)) }.sorted {
      $0.1 != $1.1 ? $0.1 < $1.1 : $0.2 < $1.2
    }.map(\.0)
    sortedCache = s
    return s
  }

  func line(msgid: String) -> Line? {
    _ = messages
    if msgidIndex == nil {
      var idx: [String: Int] = [:]
      for (i, l) in messages.enumerated() { if let m = l.msgid { idx[m] = i } }
      msgidIndex = idx
    }
    return msgidIndex?[msgid].map { messages[$0] }
  }

  func push(_ line: Line) {
    var next = messages
    // Live lines are almost always newest; keep order if a server-time tag says otherwise.
    if let last = next.last, last.time > line.time {
      let i = next.lastIndex { $0.time <= line.time }.map { $0 + 1 } ?? 0
      next.insert(line, at: i)
    } else {
      next.append(line)
    }
    if next.count > maxLines {
      next.removeFirst(next.count - trimTo)
      history = .idle
    }
    messages = next
  }

  /// Add lines from history, skipping ones already present.
  func merge(_ lines: [Line]) {
    if let next = mergeLines(messages, lines) { messages = next }
  }

  /// Record a +draft/react on the message with `msgid`.
  func react(msgid: String, emoji: String, nick: String) {
    guard let i = messages.firstIndex(where: { $0.msgid == msgid }) else { return }
    var l = messages[i]
    if let r = l.reactions.firstIndex(where: { $0.emoji == emoji }) {
      if l.reactions[r].nicks.contains(where: { conn.fold($0) == conn.fold(nick) }) { return }
      l.reactions[r].nicks.append(nick)
    } else {
      l.reactions.append(Reaction(emoji: emoji, nicks: [nick]))
    }
    messages[i] = l
  }

  func member(_ nick: String) -> Member? { members[conn.fold(nick)] }
  func setMember(_ m: Member) { members[conn.fold(m.nick)] = m }
  @discardableResult func removeMember(_ nick: String) -> Bool { members.removeValue(forKey: conn.fold(nick)) != nil }
}

struct Registration {
  var account: String
  var password: String
}

@MainActor
@Observable
final class Conn: Identifiable {
  let id: String
  let serverId: String
  let parentId: String?
  let netid: String?
  @ObservationIgnored unowned let app: AppModel
  var status = ConnStatus.connecting
  var error: String?
  var retryAt = 0
  var nick = ""
  var caps: Set<String> = []
  /// Values of enabled caps that have one, e.g. draft/multiline limits.
  var capValues: [String: String] = [:]
  var isupport: [String: String] = [:] {
    didSet { prefix = parsePrefix(isupport) }
  }
  private(set) var prefix = parsePrefix([:])
  var buffers: [String: Buffer] = [:]
  @ObservationIgnored private var serverBuffer: Buffer?
  var server: Buffer { serverBuffer! }
  var networks: [String: [String: String]] = [:]
  var away = false
  var collapsed = false
  /// MONITOR state: case-folded nick → online.
  var online: [String: Bool] = [:]
  var channelList: ChannelList?
  /// draft/metadata-2 key/values: case-folded nick (or channel) → key → value.
  var metadata: [String: [String: String]] = [:]

  @ObservationIgnored var whois: [String: [String: String]] = [:]
  @ObservationIgnored var motd: [String] = []
  @ObservationIgnored var listAcc: [ChannelInfo]?
  /// We sent AWAY because the window sat in the background.
  @ObservationIgnored var autoAway = false
  /// Saved buffers have been restored, so saving won't clobber them.
  @ObservationIgnored var buffersLoaded = false
  /// Channel keys from /join, applied when the JOIN comes back.
  @ObservationIgnored var pendingKeys: [String: String] = [:]
  /// labeled-response: label → buffer that issued the command.
  @ObservationIgnored var labels: [String: Buffer] = [:]
  @ObservationIgnored private var labelOrder: [String] = []
  @ObservationIgnored var ctcpTimes: [Int] = []
  @ObservationIgnored var ctcpWarned = 0
  /// draft/account-registration: credentials from /register, kept in memory
  /// only until the server answers, so they can be offered for SASL.
  @ObservationIgnored var registration: Registration?

  init(app: AppModel, id: String, serverId: String, parentId: String?, netid: String?) {
    self.app = app
    self.id = id
    self.serverId = serverId
    self.parentId = parentId
    self.netid = netid
    self.serverBuffer = Buffer(conn: self, kind: .server, name: "")
  }

  var config: ServerConfig? { app.servers.first { $0.id == serverId } }
  var prefKey: String { "\(serverId)|\(netid ?? "")" }
  var casemapping: String { isupport["CASEMAPPING"] ?? "rfc1459" }
  func fold(_ s: String) -> String { casefold(s, casemapping) }

  var isControl: Bool { netid == nil && caps.contains("soju.im/bouncer-networks") }

  var name: String {
    if let netid, let parentId, let n = app.conns[parentId]?.networks[netid]?["name"], !n.isEmpty { return n }
    if netid != nil, let n = isupport["NETWORK"] { return n }
    if let n = config?.name, !n.isEmpty { return n }
    return isupport["NETWORK"] ?? config?.host ?? id
  }

  var channelBuffers: [Buffer] {
    let sortKey = { (b: Buffer) in String(self.fold(b.name).drop { "#&+!".contains($0) }) }
    let all = buffers.values.map { ($0, sortKey($0)) }
    let chans = all.filter { $0.0.kind == .channel }.sorted { $0.1 < $1.1 }.map(\.0)
    let queries = all.filter { $0.0.kind == .query }.sorted { $0.1 < $1.1 }.map(\.0)
    return chans + queries
  }

  /// Nicks explicitly watched with MONITOR.
  var watched: [String] { app.settings.watch[prefKey] ?? [] }
  var canMonitor: Bool { isupport["MONITOR"] != nil && !isControl }

  func isChannel(_ name: String) -> Bool {
    guard let f = name.first else { return false }
    return (isupport["CHANTYPES"] ?? "#&").contains(f)
  }

  func isMe(_ nick: String?) -> Bool {
    guard let nick, !nick.isEmpty else { return false }
    return fold(nick) == fold(self.nick)
  }

  func buffer(_ name: String) -> Buffer? { buffers[fold(name)] }

  @discardableResult
  func ensure(_ name: String, _ kind: BufferKind? = nil) -> Buffer {
    if let b = buffer(name) { return b }
    let b = Buffer(conn: self, kind: kind ?? (isChannel(name) ? .channel : .query), name: name)
    buffers[fold(name)] = b
    if b.kind == .query {
      app.saveBuffersSoon(self)
      if canMonitor && status == .connected { send("MONITOR + \(name)") }
    }
    return b
  }

  func allBuffers() -> [Buffer] { [server] + Array(buffers.values) }

  func send(_ lines: String...) { send(lines) }

  func send(_ lines: [String]) {
    let app = self.app
    let id = self.id
    Task {
      do {
        try await app.backend.send(id, lines)
      } catch {
        app.here(self).push(Line(type: .error, text: String(describing: error)))
      }
    }
  }

  /// Send a command whose replies should land in `buf` (via labeled-response).
  func sendFrom(_ buf: Buffer, _ line: String) {
    let label = line.hasPrefix("@") ? nil : labelFor(buf)
    send(label.map { "@label=\($0) \(line)" } ?? line)
  }

  /// A fresh label whose replies go to `buf`, if the server supports labels.
  func labelFor(_ buf: Buffer) -> String? {
    guard caps.contains("labeled-response") else { return nil }
    let label = "f\(nextSeq())"
    labels[label] = buf
    labelOrder.append(label)
    if labelOrder.count > 200 { labels.removeValue(forKey: labelOrder.removeFirst()) }
    return label
  }

  func profile(_ nick: String) -> Profile {
    let m = metadata[fold(nick)]
    return Profile(avatar: m?["avatar"].flatMap { $0.isEmpty ? nil : $0 }, displayName: m?["display-name"].flatMap { $0.isEmpty ? nil : $0 })
  }

  /// Record a metadata key; `value` nil means it was removed.
  func setMetadata(_ target: String, _ key: String, _ value: String?) {
    let k = fold(target)
    var next = metadata[k] ?? [:]
    next[key] = value
    metadata[k] = next.isEmpty ? nil : next
  }

  func has(_ cap: String) -> Bool { caps.contains(cap) }

  func canTag(_ tag: String) -> Bool {
    guard caps.contains("message-tags") else { return false }
    let deny = (isupport["CLIENTTAGDENY"] ?? "").split(separator: ",").map(String.init)
    if deny.contains("*") { return deny.contains("-" + tag) }
    return !deny.contains(tag)
  }
}

struct Toast: Identifiable {
  enum Kind { case info, error }
  struct Action {
    var label: String
    var run: () -> Void
  }
  var id: Int
  var text: String
  var kind: Kind
  var action: Action?
}

/// Composer input that carries a password and must not be kept for recall.
func isSecretInput(_ line: String) -> Bool { line.firstMatch(of: /^\/(?i:register|verify)\b/) != nil }

enum SettingsTab: String, CaseIterable, Identifiable {
  case general, appearance, notifications, privacy, aliases, servers
  var id: String { rawValue }
}

struct SettingsRoute: Equatable {
  var tab: SettingsTab
  var serverId: String?
}

@MainActor
@Observable
final class AppModel {
  @ObservationIgnored let backend: Backend
  /// Shows a desktop notification about `buffer` (a `Buffer.id`); the app installs one once it can.
  @ObservationIgnored var notify: (_ title: String, _ body: String, _ buffer: String) -> Void = { _, _, _ in }

  var servers: [ServerConfig] = []
  var settings = Settings() {
    didSet { if ready { scheduleSettingsSave() } }
  }
  var conns: [String: Conn] = [:]
  var active: Buffer?
  var focused = true
  var ready = false
  var switcherOpen = false
  var searchOpen = false
  var searchQuery = ""
  var browserConn: Conn?
  var settingsRoute: SettingsRoute?
  var shortcutsOpen = false
  /// Latched by the first-run flow so it can show connection progress after the first save.
  var onboarding = false
  var toasts: [Toast] = []
  /// Bumped to ask the composer to take focus.
  var composerFocus = 0

  @ObservationIgnored private var recall: [String] = []
  var inputHistory: [String] {
    get { recall }
    set { recall = newValue.filter { !isSecretInput($0) } }
  }

  /// conn id → folded channel to open when our JOIN arrives.
  @ObservationIgnored var pendingActive: [String: String] = [:]
  @ObservationIgnored private var restoreKey: String?
  @ObservationIgnored private var blurredAt = nowMs()
  @ObservationIgnored var saveBufferTasks: [String: Task<Void, Never>] = [:]
  @ObservationIgnored private var saveSettingsTask: Task<Void, Never>?
  @ObservationIgnored private var previews: [String: Task<Preview?, Never>] = [:]
  @ObservationIgnored private var previewOrder: [String] = []
  @ObservationIgnored private var avatars: [String: Task<String?, Never>] = [:]
  @ObservationIgnored private var timers: [Task<Void, Never>] = []

  init(backend: Backend) {
    self.backend = backend
  }

  func start() async {
    do {
      let cfg = try await backend.getConfig()
      servers = cfg.servers
      settings = cfg.settings
    } catch {
      toast("Couldn't load the configuration: \(error)", .error)
    }
    restoreKey = UserDefaults.standard.string(forKey: "hearth.active")
    backend.onEvents = { [weak self] events in self?.handleEvents(events) }
    try? await backend.reset()
    for s in servers where s.autoconnect { try? await backend.connect(s.id) }
    ready = true
    timers.append(every(seconds: 2) { $0.pruneTyping() })
    timers.append(every(seconds: 30) { $0.checkAutoAway() })
  }

  private func every(seconds: Double, _ f: @escaping (AppModel) -> Void) -> Task<Void, Never> {
    Task { [weak self] in
      while !Task.isCancelled {
        try? await Task.sleep(for: .seconds(seconds))
        guard let self else { return }
        f(self)
      }
    }
  }

  func toast(_ text: String, _ kind: Toast.Kind = .info, action: Toast.Action? = nil) {
    let t = Toast(id: nextSeq(), text: text, kind: kind, action: action)
    toasts.append(t)
    // Leave time to act on an offer.
    Task { [weak self] in
      try? await Task.sleep(for: .seconds(action == nil ? 5 : 20))
      self?.dismissToast(t.id)
    }
  }

  func dismissToast(_ id: Int) {
    toasts.removeAll { $0.id == id }
  }

  /// Root connections in config order, each followed by its bouncer children.
  var orderedConns: [Conn] {
    var out: [Conn] = []
    for s in servers {
      guard let root = conns[s.id] else { continue }
      out.append(root)
      out += conns.values.filter { $0.parentId == s.id }.sorted {
        $0.name.localizedStandardCompare($1.name) == .orderedAscending
      }
    }
    return out
  }

  var allBuffers: [Buffer] { orderedConns.flatMap { [$0.server] + $0.channelBuffers } }

  // ─── Navigation ──────────────────────────────────────────────

  func open(_ buf: Buffer?) {
    if active === buf { return }
    active?.divider = 0
    active = buf
    guard let buf else { return }
    buf.divider = buf.unread > 0 ? buf.readMarker : 0
    rememberActive()
    if buf.kind != .server && buf.history == .idle && buf.messages.count < 30 {
      Task { await fetchHistory(buf, .before) }
    }
  }

  /// Persist which buffer is open, to restore it on the next launch.
  func rememberActive() {
    guard let buf = active else { return }
    UserDefaults.standard.set("\(buf.conn.serverId)|\(buf.conn.netid ?? "")|\(buf.key)", forKey: "hearth.active")
  }

  func step(_ delta: Int, unreadOnly: Bool = false) {
    let list = allBuffers.filter { !$0.conn.collapsed || $0.kind == .server }
    guard !list.isEmpty else { return }
    var i = active.flatMap { a in list.firstIndex { $0 === a } } ?? -1
    for _ in 0..<list.count {
      i = (i + delta + list.count) % list.count
      if !unreadOnly || list[i].unread > 0 { return open(list[i]) }
    }
  }

  func markRead(_ buf: Buffer) { markBufferRead(buf) }

  func closeBuffer(_ buf: Buffer) {
    if buf.kind == .server { return }
    if buf.kind == .channel && buf.joined { buf.conn.send("PART \(buf.name)") }
    buf.wantJoined = false
    let list = allBuffers
    let i = list.firstIndex { $0 === buf }
    buf.conn.buffers.removeValue(forKey: buf.key)
    if buf.kind == .query && buf.conn.canMonitor && !isWatched(buf.conn, buf.name) {
      buf.conn.send("MONITOR - \(buf.name)")
    }
    saveBuffersSoon(buf.conn)
    if active === buf {
      let prev = i.flatMap { $0 > 0 ? list[$0 - 1] : nil }
      let next = i.flatMap { $0 + 1 < list.count ? list[$0 + 1] : nil }
      open(prev ?? next)
    }
  }

  /// Open `buf` and scroll to a message, loading history around it if needed.
  func jumpTo(_ buf: Buffer, time: Int, msgid: String?) async {
    open(buf)
    let present = buf.messages.contains { msgid != nil ? $0.msgid == msgid : $0.time == time }
    if !present { await loadAround(buf, time: time, msgid: msgid) }
    buf.focus = Focus(time: time, msgid: msgid, seq: nextSeq())
  }

  // ─── Settings & servers ─────────────────────────────────────

  private func scheduleSettingsSave() {
    saveSettingsTask?.cancel()
    saveSettingsTask = Task { [weak self] in
      try? await Task.sleep(for: .milliseconds(300))
      guard let self, !Task.isCancelled else { return }
      try? await self.backend.saveSettings(self.settings)
    }
  }

  /// Save now, for quitting.
  func flushSettings() async {
    guard saveSettingsTask != nil else { return }
    saveSettingsTask?.cancel()
    saveSettingsTask = nil
    try? await backend.saveSettings(settings)
  }

  @discardableResult
  func saveServer(_ server: ServerConfig, connectNow: Bool = true) async throws -> ServerConfig {
    let saved = try await backend.saveServer(server)
    if let i = servers.firstIndex(where: { $0.id == saved.id }) {
      servers[i] = saved
    } else {
      servers.append(saved)
    }
    if connectNow {
      try await backend.disconnect(saved.id)
      try await backend.connect(saved.id)
    }
    return saved
  }

  func deleteServer(_ id: String) async throws {
    try await backend.deleteServer(id)
    servers.removeAll { $0.id == id }
    for c in conns.values where c.serverId == id { removeConn(c.id) }
  }

  func connectServer(_ id: String) {
    Task { try? await backend.connect(id) }
  }

  func disconnectServer(_ id: String, reason: String? = nil) {
    Task { try? await backend.disconnect(id, reason: reason) }
  }

  func reconnectServer(_ id: String) {
    Task { try? await backend.reconnect(id) }
  }

  private func removeConn(_ id: String) {
    guard let c = conns[id] else { return }
    conns.removeValue(forKey: id)
    if active?.conn === c { open(allBuffers.first) }
  }

  func setNotify(_ buf: Buffer, _ level: NotifyLevel?) {
    settings.bufferNotify[buf.prefKey] = level
  }

  // ─── Ignore & watch ─────────────────────────────────────────

  func isIgnored(_ conn: Conn, _ src: Source?) -> Bool {
    guard let src, !conn.isMe(src.nick) else { return false }
    return Hearth.isIgnored(settings.ignores, src, fold: conn.fold)
  }

  func ignore(_ mask: String) {
    let m = normalizeMask(mask)
    if m.isEmpty || settings.ignores.contains(m) { return }
    settings.ignores.append(m)
  }

  func unignore(_ mask: String) -> Bool {
    let m = normalizeMask(mask)
    let next = settings.ignores.filter { $0 != m && $0 != mask }
    if next.count == settings.ignores.count { return false }
    settings.ignores = next
    return true
  }

  // ─── Presence ───────────────────────────────────────────────

  func setFocused(_ focused: Bool) {
    self.focused = focused
    if !focused {
      blurredAt = nowMs()
      return
    }
    for c in conns.values where c.autoAway {
      c.autoAway = false
      if c.away && c.status == .connected { c.send("AWAY") }
    }
  }

  private func checkAutoAway() {
    let mins = settings.autoAwayMinutes
    if mins == 0 || focused || nowMs() - blurredAt < mins * 60_000 { return }
    for c in conns.values {
      if c.status != .connected || c.isControl || c.away || c.autoAway { continue }
      c.autoAway = true
      c.send("AWAY :Away from keyboard")
    }
  }

  // ─── Outgoing ────────────────────────────────────────────────

  func sendMessage(_ buf: Buffer, _ text: String, kind: String = "PRIVMSG", action: Bool = false, replyTo: String? = nil) {
    let conn = buf.conn
    let target = buf.name
    let reply = replyTo.flatMap { conn.canTag("draft/reply") ? $0 : nil }
    // Leave room for ":nick!user@host CMD target :" plus CTCP wrapping.
    let budget = 510 - (conn.nick.utf8.count + 100 + kind.count + target.utf8.count + (action ? 9 : 0))
    for (i, part) in splitMessage(text, maxBytes: max(budget, 60)).enumerated() {
      let body = action ? "\u{1}ACTION \(part)\u{1}" : part
      let tags = reply != nil && i == 0 ? "@+draft/reply=\(escapeTag(reply!)) " : ""
      conn.send("\(tags)\(kind) \(target) :\(body)")
      if !conn.has("echo-message") {
        buf.push(
          Line(
            type: action ? .action : kind == "NOTICE" ? .notice : .privmsg, nick: conn.nick, text: part, isSelf: true,
            replyTo: i == 0 ? reply : nil))
      }
    }
    buf.lastTypingSent = 0
  }

  /// Send text with line breaks as draft/multiline batches, so it arrives as
  /// one message. Returns false when the server can't take it that way.
  func sendMultiline(_ buf: Buffer, _ text: String, replyTo: String? = nil) -> Bool {
    let conn = buf.conn
    guard let limits = parseMultilineCap(conn.capValues["draft/multiline"]), conn.has("batch"), buf.kind != .server
    else { return false }
    // A single line (maybe with stray line breaks around it) needs no batch.
    if text.split(separator: "\n").filter({ !$0.trimmingCharacters(in: .whitespaces).isEmpty }).count < 2 {
      return false
    }
    let target = buf.name
    let budget = 510 - (conn.nick.utf8.count + 100 + "PRIVMSG".count + target.utf8.count)
    let batches = multilineBatches(text, limits: limits, lineBytes: max(budget, 60))
    if batches.isEmpty { return false }
    let reply = replyTo.flatMap { conn.canTag("draft/reply") ? $0 : nil }
    for (i, parts) in batches.enumerated() {
      let ref = "ml\(nextSeq())"
      // Labeled, so a FAIL about the batch (too long, say) lands here.
      let label = conn.labelFor(buf)
      let tags = [label.map { "label=\($0)" }, reply != nil && i == 0 ? "+draft/reply=\(escapeTag(reply!))" : nil]
        .compactMap { $0 }
      var lines = ["\(tags.isEmpty ? "" : "@\(tags.joined(separator: ";")) ")BATCH +\(ref) draft/multiline \(target)"]
      lines += parts.map { "@batch=\(ref)\($0.concat ? ";draft/multiline-concat" : "") PRIVMSG \(target) :\($0.text)" }
      lines.append("BATCH -\(ref)")
      conn.send(lines)
      if !conn.has("echo-message") {
        buf.push(Line(type: .privmsg, nick: conn.nick, text: joinParts(parts), isSelf: true, replyTo: i == 0 ? reply : nil))
      }
    }
    buf.lastTypingSent = 0
    return true
  }

  enum TypingState: String { case active, done }

  func sendTyping(_ buf: Buffer, _ state: TypingState) {
    if !settings.sendTyping || buf.kind == .server || !buf.conn.canTag("typing") { return }
    let now = nowMs()
    if state == .active && now - buf.lastTypingSent < 3000 { return }
    if state == .done && buf.lastTypingSent == 0 { return }
    buf.lastTypingSent = state == .active ? now : 0
    buf.conn.send("@+typing=\(state.rawValue) TAGMSG \(buf.name)")
  }

  func canReact(_ buf: Buffer, _ line: Line) -> Bool {
    line.msgid != nil && buf.kind != .server && buf.conn.canTag("draft/react") && buf.conn.canTag("draft/reply")
  }

  func react(_ buf: Buffer, _ line: Line, _ emoji: String) {
    guard let msgid = line.msgid else { return }
    let conn = buf.conn
    if line.reactions.first(where: { $0.emoji == emoji })?.nicks.contains(where: conn.isMe) == true { return }
    conn.send("@+draft/react=\(escapeTag(emoji));+draft/reply=\(escapeTag(msgid)) TAGMSG \(buf.name)")
    if !conn.has("echo-message") { buf.react(msgid: msgid, emoji: emoji, nick: conn.nick) }
  }

  func canRedact(_ buf: Buffer, _ line: Line) -> Bool {
    guard line.msgid != nil, buf.conn.has("draft/message-redaction") else { return false }
    if line.isSelf { return true }
    let p = buf.conn.prefix
    guard let me = buf.member(buf.conn.nick), let top = me.prefix.first, let op = p.symbols.firstIndex(of: "@") else {
      return false
    }
    return p.rank(of: top) <= op
  }

  func redact(_ buf: Buffer, _ line: Line) {
    if let id = line.msgid { buf.conn.send("REDACT \(buf.name) \(id)") }
  }

  func join(_ conn: Conn, _ channel: String, key: String? = nil) {
    pendingActive[conn.id] = conn.fold(channel)
    if let key, !key.isEmpty { conn.pendingKeys[conn.fold(channel)] = key }
    if let existing = conn.buffer(channel), existing.joined {
      open(existing)
      return
    }
    conn.send(key.map { $0.isEmpty ? "JOIN \(channel)" : "JOIN \(channel) \($0)" } ?? "JOIN \(channel)")
  }

  @discardableResult
  func query(_ conn: Conn, _ nick: String) -> Buffer {
    let b = conn.ensure(nick, .query)
    open(b)
    return b
  }

  func browseChannels(_ conn: Conn) {
    browserConn = conn
    let stale = conn.channelList.map { !$0.loading && nowMs() - $0.at > 10 * 60_000 } ?? true
    if stale && conn.status == .connected {
      conn.channelList = ChannelList(loading: true, items: conn.channelList?.items ?? [], at: nowMs())
      conn.send("LIST")
    }
  }

  // ─── Link previews ──────────────────────────────────────────

  func preview(_ url: String) async -> Preview? {
    if let t = previews[url] { return await t.value }
    let backend = self.backend
    let t = Task { try? await backend.linkPreview(url) }
    previews[url] = t
    previewOrder.append(url)
    if previewOrder.count > 300 { previews.removeValue(forKey: previewOrder.removeFirst()) }
    return await t.value
  }

  /// A metadata avatar as a data: URL, or nil if it can't or mustn't be loaded.
  func avatar(_ url: String) async -> String? {
    guard settings.loadAvatars else { return nil }
    if let t = avatars[url] { return await t.value }
    let backend = self.backend
    let t = Task { try? await backend.avatar(url) }
    avatars[url] = t
    if avatars.count > 1000 { avatars.removeAll() }
    return await t.value
  }

  /// Use the account from /register for SASL on this server from now on.
  func saveLogin(_ conn: Conn, account: String, password: String) async {
    guard var cfg = conn.config else { return }
    cfg.sasl = .plain
    cfg.saslUsername = account
    cfg.saslPassword = password
    do {
      try await saveServer(cfg, connectNow: false)
      toast("Hearth will sign in as \(account) on \(conn.name)")
    } catch {
      toast("Couldn't save the login: \(error)", .error)
    }
  }

  // ─── Incoming ────────────────────────────────────────────────

  func handleEvents(_ events: [IrcEvent]) {
    for ev in events { handleEvent(ev) }
  }

  private func handleEvent(_ ev: IrcEvent) {
    switch ev {
    case let .connAdded(id, server, parent, netid):
      if conns[id] == nil { conns[id] = Conn(app: self, id: id, serverId: server, parentId: parent, netid: netid) }
    case let .connRemoved(id):
      removeConn(id)
    case let .status(id, status, error, retryIn):
      guard let conn = conns[id] else { return }
      let was = conn.status
      conn.status = status
      conn.error = error
      conn.retryAt = retryIn.map { nowMs() + $0 * 1000 } ?? 0
      if status == .disconnected {
        for b in conn.buffers.values {
          b.joined = false
          b.members = [:]
          b.typing = [:]
        }
        conn.online = [:]
        conn.metadata = [:]
        if was == .connected || error != nil {
          let retry = retryIn.map { " — retrying in \($0)s" } ?? ""
          conn.server.push(Line(type: .error, text: "Disconnected\(error.map { ": " + $0 } ?? "")\(retry)"))
        }
      }
      if status == .connected { Task { await onConnected(conn) } }
    case let .registered(id, nick, caps, capValues):
      guard let conn = conns[id] else { return }
      conn.nick = nick
      conn.caps = Set(caps)
      conn.capValues = capValues
    case let .caps(id, caps, capValues):
      guard let conn = conns[id] else { return }
      conn.caps = Set(caps)
      conn.capValues = capValues
    case let .isupport(id, isupport):
      conns[id]?.isupport = isupport
    case let .message(id, msg):
      guard let conn = conns[id] else { return }
      handleMessage(conn, msg)
    case .unknown:
      break
    }
  }

  private func onConnected(_ conn: Conn) async {
    conn.error = nil
    conn.autoAway = false
    maybeRestoreActive(conn, key: "")
    if active == nil { open(conn.server) }
    if conn.isControl { return }
    if !conn.buffersLoaded { await restoreBuffers(conn) }
    // Rejoin channels on direct connections; bouncers restore them on their own.
    if conn.netid == nil {
      let auto = Set((conn.config?.autojoin ?? []).map { conn.fold($0.trimmingCharacters(in: .whitespaces)) })
      let rejoin = conn.buffers.values.filter { $0.kind == .channel && $0.wantJoined && !auto.contains($0.key) }
      if !rejoin.isEmpty {
        Task {
          try? await Task.sleep(for: .milliseconds(1500))
          for l in rejoinCommands(rejoin.map { ($0.name, $0.joinKey) }) { conn.send(l) }
        }
      }
    }
    syncMonitor(conn)
    Task {
      try? await Task.sleep(for: .milliseconds(800))
      await fetchTargets(conn)
    }
  }

  func maybeRestoreActive(_ conn: Conn, key: String) {
    guard let restoreKey else { return }
    if restoreKey == "\(conn.serverId)|\(conn.netid ?? "")|\(key)" {
      self.restoreKey = nil
      open(key.isEmpty ? conn.server : (conn.buffer(key) ?? conn.server))
    }
  }

  /// Where replies and errors go: the buffer that sent the command (by label),
  /// else the active buffer if it's on this connection, else the server buffer.
  func here(_ conn: Conn, _ label: String? = nil) -> Buffer {
    if let label, let b = conn.labels[label], b.kind == .server || conn.buffers[b.key] === b { return b }
    if let a = active, a.conn === conn { return a }
    return conn.server
  }

  var welcoming: Bool { ready && (servers.isEmpty || onboarding) }

  func networkHighlights(_ serverId: String) -> NetworkHighlights {
    settings.networkHighlights[serverId] ?? NetworkHighlights()
  }

  func highlightWords(_ conn: Conn) -> [String] {
    [conn.nick] + settings.highlightWords + networkHighlights(conn.serverId).words
  }

  /// Whether a message from `src` highlights us.
  func isHighlight(_ conn: Conn, _ src: Source?, _ text: String) -> Bool {
    let never = networkHighlights(conn.serverId).never
    if let src, Hearth.isIgnored(never, src, fold: conn.fold) { return false }
    return mentions(text, highlightWords(conn))
  }

  func maybeNotify(_ conn: Conn, _ buf: Buffer, nick: String, text: String, important: Bool) {
    let level = buf.level
    if level == NotifyLevel.none || (!important && level != .all) { return }
    let isQuery = buf.kind == .query
    if level == nil && (isQuery ? !settings.notifyPrivate : !settings.notifyHighlights) { return }
    if focused && active === buf { return }
    let where_ = isQuery ? nick : "\(nick) in \(buf.name)"
    let body = stripFormatting(text)
    notify("\(where_) · \(conn.name)", body.count > 240 ? String(body.prefix(239)) + "…" : body, buf.id)
  }

  /// Mentions waiting across every buffer, for the dock badge.
  var totalHighlights: Int {
    conns.values.reduce(0) { n, c in n + c.allBuffers().reduce(0) { $0 + $1.highlights } }
  }

  private func pruneTyping() {
    let now = nowMs()
    for c in conns.values {
      for b in c.buffers.values where b.typing.values.contains(where: { $0 < now }) {
        b.typing = b.typing.filter { $0.value >= now }
      }
    }
  }
}
