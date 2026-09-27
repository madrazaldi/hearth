// Types shared with the Rust core (serde camelCase JSON). They mirror
// src/lib/types.ts so config.json and the settings blob stay compatible.

import Foundation

struct Source: Codable, Equatable, Sendable {
  var nick: String
  var user: String?
  var host: String?
}

struct IrcMessage: Codable, Equatable, Sendable {
  var tags: [String: String]
  var source: Source?
  var command: String
  var params: [String]

  init(tags: [String: String] = [:], source: Source? = nil, command: String, params: [String]) {
    self.tags = tags
    self.source = source
    self.command = command
    self.params = params
  }

  /// `params[i]`, or nil past the end.
  func param(_ i: Int) -> String? { i < params.count ? params[i] : nil }
}

enum ConnStatus: String, Codable, Sendable {
  case connecting, registering, connected, disconnected
}

enum IrcEvent: Decodable, Sendable {
  case connAdded(conn: String, server: String, parent: String?, netid: String?)
  case connRemoved(conn: String)
  case status(conn: String, status: ConnStatus, error: String?, retryIn: Int?)
  case registered(conn: String, nick: String, caps: [String], capValues: [String: String])
  case caps(conn: String, caps: [String], capValues: [String: String])
  case isupport(conn: String, isupport: [String: String])
  case message(conn: String, msg: IrcMessage)
  case unknown

  private enum K: String, CodingKey {
    case type, conn, server, parent, netid, status, error, retryIn, nick, caps, capValues, isupport, msg
  }

  init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: K.self)
    let conn = (try? c.decode(String.self, forKey: .conn)) ?? ""
    switch try c.decode(String.self, forKey: .type) {
    case "conn-added":
      self = .connAdded(
        conn: conn, server: try c.decode(String.self, forKey: .server),
        parent: try c.decodeIfPresent(String.self, forKey: .parent), netid: try c.decodeIfPresent(String.self, forKey: .netid))
    case "conn-removed":
      self = .connRemoved(conn: conn)
    case "status":
      self = .status(
        conn: conn, status: try c.decode(ConnStatus.self, forKey: .status),
        error: try c.decodeIfPresent(String.self, forKey: .error), retryIn: try c.decodeIfPresent(Int.self, forKey: .retryIn))
    case "registered":
      self = .registered(
        conn: conn, nick: try c.decode(String.self, forKey: .nick), caps: try c.decode([String].self, forKey: .caps),
        capValues: try c.decodeIfPresent([String: String].self, forKey: .capValues) ?? [:])
    case "caps":
      self = .caps(
        conn: conn, caps: try c.decode([String].self, forKey: .caps),
        capValues: try c.decodeIfPresent([String: String].self, forKey: .capValues) ?? [:])
    case "isupport":
      self = .isupport(conn: conn, isupport: try c.decode([String: String].self, forKey: .isupport))
    case "message":
      self = .message(conn: conn, msg: try c.decode(IrcMessage.self, forKey: .msg))
    default:
      self = .unknown
    }
  }
}

enum SaslMechanism: String, Codable, Sendable, CaseIterable {
  case none, plain, external
}

struct ServerConfig: Codable, Equatable, Identifiable, Sendable {
  var id = ""
  var name = ""
  var host = ""
  var port = 6697
  var tls = true
  var acceptInvalidCerts = false
  var websocket = false
  var websocketPath = "/"
  var nick = ""
  var username = ""
  var realname = ""
  var password = ""
  var sasl = SaslMechanism.none
  var saslUsername = ""
  var saslPassword = ""
  var autoconnect = true
  var autojoin: [String] = []
  var perform: [String] = []
  /// PEM file with a client certificate and key (SASL EXTERNAL / CertFP).
  var clientCert = ""

  init() {}

  init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    func v<T: Decodable>(_ k: CodingKeys, _ d: T) -> T { (try? c.decodeIfPresent(T.self, forKey: k)) ?? d }
    id = v(.id, "")
    name = v(.name, "")
    host = v(.host, "")
    port = v(.port, 6697)
    tls = v(.tls, true)
    acceptInvalidCerts = v(.acceptInvalidCerts, false)
    websocket = v(.websocket, false)
    websocketPath = v(.websocketPath, "/")
    nick = v(.nick, "")
    username = v(.username, "")
    realname = v(.realname, "")
    password = v(.password, "")
    sasl = v(.sasl, .none)
    saslUsername = v(.saslUsername, "")
    saslPassword = v(.saslPassword, "")
    autoconnect = v(.autoconnect, true)
    autojoin = v(.autojoin, [])
    perform = v(.perform, [])
    clientCert = v(.clientCert, "")
  }
}

enum NotifyLevel: String, Codable, Sendable, CaseIterable {
  case all, mentions, none
}

struct NetworkHighlights: Codable, Equatable, Sendable {
  /// Words, or `/regex/flags`.
  var words: [String] = []
  /// Nicks or masks whose messages never highlight (bots, relays).
  var never: [String] = []
}

enum Theme: String, Codable, Sendable, CaseIterable { case system, dark, light }
enum Layout: String, Codable, Sendable, CaseIterable { case columns, compact }
enum Clock: String, Codable, Sendable, CaseIterable {
  case h24 = "24h"
  case h12 = "12h"
}
enum Membership: String, Codable, Sendable, CaseIterable { case show, collapse, hide }

/// The frontend-owned settings blob. Keys match the Svelte app's `Settings`,
/// and missing keys fall back to the same defaults.
struct Settings: Codable, Equatable, Sendable {
  var theme = Theme.system
  var layout = Layout.columns
  var fontSize = 14
  var clock = Clock.h24
  var showSeconds = false
  var membership = Membership.collapse
  var nickColors = true
  var showMembers = true
  var notifyHighlights = true
  var notifyPrivate = true
  var highlightWords: [String] = []
  var sendTyping = true
  var showTyping = true
  var sidebarWidth = 248
  /// Keep a local copy of messages (enables offline scrollback and search).
  var storeLogs = true
  /// Fetch link and image previews (through the core, never directly).
  var linkPreviews = false
  /// Fetch avatars people set with draft/metadata-2 (through the core).
  var loadAvatars = false
  /// With link previews on, show direct image links as images under the message.
  var inlineImages = true
  var autoplayAnimations = false
  var closeToTray = false
  var checkUpdates = true
  /// Mark away after this many minutes with the window in the background; 0 disables.
  var autoAwayMinutes = 0
  /// nick or nick!user@host masks, `*` and `?` wildcards.
  var ignores: [String] = []
  /// Per-buffer notification level, keyed by `Buffer.prefKey`. Absent means the default.
  var bufferNotify: [String: NotifyLevel] = [:]
  /// Nicks to watch with MONITOR, keyed by `Conn.prefKey`.
  var watch: [String: [String]] = [:]
  /// Extra highlight rules per server id; bouncer networks share their bouncer's.
  var networkHighlights: [String: NetworkHighlights] = [:]
  /// User slash commands: name (no slash) → `;`-separated commands.
  var aliases: [String: String] = [:]
  var spellcheck = true

  init() {}

  init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    let d = Settings()
    func v<T: Decodable>(_ k: CodingKeys, _ def: T) -> T { (try? c.decodeIfPresent(T.self, forKey: k)) ?? def }
    theme = v(.theme, d.theme)
    layout = v(.layout, d.layout)
    fontSize = v(.fontSize, d.fontSize)
    clock = v(.clock, d.clock)
    showSeconds = v(.showSeconds, d.showSeconds)
    membership = v(.membership, d.membership)
    nickColors = v(.nickColors, d.nickColors)
    showMembers = v(.showMembers, d.showMembers)
    notifyHighlights = v(.notifyHighlights, d.notifyHighlights)
    notifyPrivate = v(.notifyPrivate, d.notifyPrivate)
    highlightWords = v(.highlightWords, d.highlightWords)
    sendTyping = v(.sendTyping, d.sendTyping)
    showTyping = v(.showTyping, d.showTyping)
    sidebarWidth = v(.sidebarWidth, d.sidebarWidth)
    storeLogs = v(.storeLogs, d.storeLogs)
    linkPreviews = v(.linkPreviews, d.linkPreviews)
    loadAvatars = v(.loadAvatars, d.loadAvatars)
    inlineImages = v(.inlineImages, d.inlineImages)
    autoplayAnimations = v(.autoplayAnimations, d.autoplayAnimations)
    closeToTray = v(.closeToTray, d.closeToTray)
    checkUpdates = v(.checkUpdates, d.checkUpdates)
    autoAwayMinutes = v(.autoAwayMinutes, d.autoAwayMinutes)
    ignores = v(.ignores, d.ignores)
    bufferNotify = v(.bufferNotify, d.bufferNotify)
    watch = v(.watch, d.watch)
    networkHighlights = v(.networkHighlights, d.networkHighlights)
    aliases = v(.aliases, d.aliases)
    spellcheck = v(.spellcheck, d.spellcheck)
  }
}

struct Config: Decodable, Sendable {
  var servers: [ServerConfig]
  var settings: Settings

  private enum K: String, CodingKey { case servers, settings }

  init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: K.self)
    servers = (try? c.decodeIfPresent([ServerConfig].self, forKey: .servers)) ?? []
    settings = (try? c.decodeIfPresent(Settings.self, forKey: .settings)) ?? Settings()
  }
}

struct SavedBuffer: Codable, Equatable, Sendable {
  var name: String
  var kind: String
  var key: String
}

struct SearchHit: Decodable, Sendable {
  var network: String
  var target: String
  var msg: IrcMessage
}

struct Preview: Decodable, Sendable {
  var url: String
  /// `media` is a video or audio link, described but never loaded.
  var kind: String
  var title: String?
  var description: String?
  var site: String?
  /// A data: URL.
  var image: String?
  var width: Int?
  var height: Int?
  var animated: Bool
  var mime: String?
  var size: Int?
}

struct CertInfo: Decodable, Sendable {
  var path: String
  var sha256: String
  var sha512: String
}
