// Model helpers the settings window needs, ported from the Svelte components
// (AliasEditor, NetworkHighlights, BouncerNetworks), plus small shared views.

import SwiftUI

/// Whether an alias name shadows a built-in command (commands.ts `isBuiltin`).
@MainActor
func isBuiltinCommand(_ name: String) -> Bool { commands[name] != nil || commandAliases[name] != nil }

/// The editable fields of a soju network (BouncerNetworks.svelte `Draft`).
struct BouncerNetworkDraft: Identifiable, Equatable {
  /// Nil while adding a network.
  var netid: String?
  var name = ""
  var host = ""
  var port = "6697"
  var tls = true
  var nickname = ""
  var username = ""
  var pass = ""

  var id: String { netid ?? "\u{0}new" }

  init() {}

  init(netid: String, attrs: [String: String]) {
    self.netid = netid
    name = attrs["name"] ?? ""
    host = attrs["host"] ?? ""
    port = attrs["port"] ?? ""
    tls = attrs["tls"] != "0"
    nickname = attrs["nickname"] ?? ""
    username = attrs["username"] ?? ""
  }

  /// BOUNCER attributes: everything set when adding, only what changed when editing.
  func attrs(original: [String: String]? = nil) -> String {
    var out: [String] = []
    func put(_ k: String, _ v: String) {
      if original.map({ $0[k] != v }) ?? !v.isEmpty { out.append("\(k)=\(escapeAttr(v))") }
    }
    let trim = { (s: String) in s.trimmingCharacters(in: .whitespaces) }
    put("name", trim(name))
    put("host", trim(host))
    if !trim(port).isEmpty { put("port", trim(port)) }
    put("tls", tls ? "1" : "0")
    put("nickname", trim(nickname))
    put("username", trim(username))
    // The bouncer never reports the password back, so it's only sent when typed.
    if !pass.isEmpty { out.append("pass=\(escapeAttr(pass))") }
    return out.joined(separator: ";")
  }
}

extension Conn {
  /// Add or change a network on this soju control connection.
  func saveBouncerNetwork(_ d: BouncerNetworkDraft) {
    if let netid = d.netid {
      let changes = d.attrs(original: networks[netid])
      if !changes.isEmpty { send("BOUNCER CHANGENETWORK \(netid) \(changes)") }
    } else {
      send("BOUNCER ADDNETWORK \(d.attrs())")
    }
  }

  func deleteBouncerNetwork(_ netid: String) { send("BOUNCER DELNETWORK \(netid)") }
}

enum HighlightTest: Equatable {
  case hit(String)
  case miss(String)
}

extension AppModel {
  /// Store a server's extra highlight rules, dropping the entry when it's empty.
  func setNetworkHighlights(_ serverId: String, words: [String], never: [String]) {
    if words.isEmpty && never.isEmpty {
      settings.networkHighlights.removeValue(forKey: serverId)
    } else {
      settings.networkHighlights[serverId] = NetworkHighlights(words: words, never: never)
    }
  }

  /// Would `text` from `from` highlight us on this server with these rules?
  func testHighlight(serverId: String, nick: String, words: [String], never: [String], text: String, from: String)
    -> HighlightTest?
  {
    if text.trimmingCharacters(in: .whitespaces).isEmpty { return nil }
    let conn = conns.values.first { $0.serverId == serverId && !$0.isControl }
    let fold: (String) -> String = { conn?.fold($0) ?? casefold($0) }
    let sender = from.trimmingCharacters(in: .whitespaces)
    if !sender.isEmpty && Hearth.isIgnored(never, Source(nick: sender), fold: fold) {
      return .miss("\(sender) never highlights")
    }
    let me = conn.map(\.nick).flatMap { $0.isEmpty ? nil : $0 } ?? nick
    let rules = ([me] + settings.highlightWords + words).filter { !$0.isEmpty }
    guard let rule = matchingRule(text, rules) else { return .miss("No rule matches") }
    return .hit(rule == me ? "Matches your nickname" : "Matches \(rule)")
  }

  /// The server a saved `bufferNotify` / `watch` key belongs to, for display.
  func serverName(prefKey: String) -> String {
    let parts = prefKey.split(separator: "|", omittingEmptySubsequences: false).map(String.init)
    let serverId = parts.first ?? ""
    let netid = parts.count > 1 ? parts[1] : ""
    if !netid.isEmpty, let n = conns.values.first(where: { $0.serverId == serverId && $0.netid == netid })?.name {
      return n
    }
    guard let s = servers.first(where: { $0.id == serverId }) else { return "Removed server" }
    return s.name.isEmpty ? s.host : s.name
  }
}

/// Splits one-per-line or comma-separated text into trimmed, non-empty items.
func settingsList(_ text: String, separators: String) -> [String] {
  text.split { separators.contains($0) }.map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
}

/// A connection-status dot, as in the Svelte server list.
struct SettingsStatusDot: View {
  enum Kind { case ok, busy, off, failed }
  var state: Kind
  @Environment(\.palette) private var p

  var body: some View {
    Circle()
      .fill(color)
      .frame(width: 8, height: 8)
      .accessibilityHidden(true)
  }

  private var color: Color {
    switch state {
    case .ok: p.ok
    case .busy: p.warn
    case .off: p.text4.opacity(0.6)
    case .failed: p.danger
    }
  }
}

extension SettingsStatusDot.Kind {
  @MainActor init(_ conn: Conn?) {
    guard let conn else {
      self = .off
      return
    }
    switch conn.status {
    case .connected: self = .ok
    case .connecting, .registering: self = .busy
    case .disconnected: self = conn.error == nil ? .off : .failed
    }
  }
}

/// A grey explanatory line under a setting, as macOS settings panes use.
struct SettingsNote: View {
  var text: String
  var tone: Tone = .plain
  enum Tone { case plain, error, warning, success }
  @Environment(\.palette) private var p

  init(_ text: String, _ tone: Tone = .plain) {
    self.text = text
    self.tone = tone
  }

  var body: some View {
    Text(.init(text))
      .font(.callout)
      .foregroundStyle(color)
      .fixedSize(horizontal: false, vertical: true)
  }

  private var color: Color {
    switch tone {
    case .plain: .secondary
    case .error: p.danger
    case .warning: p.warn
    case .success: p.ok
    }
  }
}

/// A label with a secondary description underneath, for toggles and pickers.
struct SettingsLabel: View {
  var title: String
  var hint: String?

  init(_ title: String, hint: String? = nil) {
    self.title = title
    self.hint = hint
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 2) {
      Text(title)
      if let hint {
        Text(.init(hint)).font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
      }
    }
  }
}
