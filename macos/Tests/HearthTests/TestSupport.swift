// Test harness mirroring src/lib/test-utils.ts: an AppModel wired to a fake
// backend, fed scripted IRC events parsed from raw lines.

import Foundation

@testable import Hearth

/// A backend that never touches the network or keychain. It records every
/// call, collects `send` lines in `sent`, and answers other commands with
/// canned JSON.
@MainActor
final class FakeBackend: Backend {
  var onEvents: (([IrcEvent]) -> Void)?
  var closeToTray: Bool { false }

  /// Every line passed to `send`, in order (test-utils.ts `sent`).
  var sent: [String] = []
  /// Every command with its arguments, in order.
  var calls: [(command: String, args: [String: Any])] = []

  /// Answers by command: a JSON-compatible value, or a closure over the args.
  var responses: [String: ([String: Any]) throws -> Any] = [
    "get_config": { _ in ["servers": [Any](), "settings": [String: Any]()] },
    "load_buffers": { _ in [Any]() },
    "history": { _ in [Any]() },
    "local_history": { _ in [Any]() },
    "search": { _ in [Any]() },
    // Like mockApi.saveServer.mockImplementation(async (s) => s).
    "save_server": { args in args["server"] ?? NSNull() },
    "avatar": { args in "data:image/png;base64,\(Data(((args["url"] as? String) ?? "").utf8).base64EncodedString())" },
  ]
  /// One-shot answers, used before `responses` (mockResolvedValueOnce).
  var once: [String: [Any]] = [:]

  func respond(_ command: String, with value: Any) { responses[command] = { _ in value } }
  func respondOnce(_ command: String, with value: Any) { once[command, default: []].append(value) }

  func calls(_ command: String) -> [[String: Any]] { calls.filter { $0.command == command }.map(\.args) }

  func call(_ command: String, _ args: [String: Any]) async throws -> Data {
    calls.append((command, args))
    if command == "send" { sent += (args["lines"] as? [String]) ?? [] }
    let value: Any
    if var q = once[command], !q.isEmpty {
      value = q.removeFirst()
      once[command] = q
    } else if let r = responses[command] {
      value = try r(args)
    } else {
      value = NSNull()
    }
    return try JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed])
  }
}

/// JSON for an IrcMessage, as the core would return it from history commands.
func json(_ m: IrcMessage) -> Any { try! jsonObject(m) }

// ─── Raw line parsing ──────────────────────────────────────────────

private func unescapeTag(_ v: Substring) -> String {
  var out = ""
  var it = v.makeIterator()
  while let c = it.next() {
    guard c == "\\" else {
      out.append(c)
      continue
    }
    switch it.next() {
    case ":": out.append(";")
    case "s": out.append(" ")
    case "\\": out.append("\\")
    case "r": out.append("\r")
    case "n": out.append("\n")
    case let other?: out.append(other)
    case nil: break
    }
  }
  return out
}

/// Parse a raw IRC line into the shape the backend emits (test-utils.ts `msg`).
func msg(_ line: String) -> IrcMessage {
  var rest = Substring(line)
  var tags: [String: String] = [:]
  if rest.hasPrefix("@") {
    let sp = rest.firstIndex(of: " ") ?? rest.endIndex
    for t in rest[rest.index(after: rest.startIndex)..<sp].split(separator: ";") {
      if let eq = t.firstIndex(of: "=") {
        tags[String(t[..<eq])] = unescapeTag(t[t.index(after: eq)...])
      } else {
        tags[String(t)] = ""
      }
    }
    rest = sp < rest.endIndex ? rest[rest.index(after: sp)...] : ""
  }
  var source: Source?
  if rest.hasPrefix(":") {
    let sp = rest.firstIndex(of: " ") ?? rest.endIndex
    source = parseMask(String(rest[rest.index(after: rest.startIndex)..<sp]))
    rest = sp < rest.endIndex ? rest[rest.index(after: sp)...] : ""
  }
  let trailing = rest.range(of: " :")
  let head = trailing.map { rest[..<$0.lowerBound] } ?? rest
  var params = head.split(separator: " ").map(String.init)
  let command = params.isEmpty ? "" : params.removeFirst().uppercased()
  if let trailing { params.append(String(rest[trailing.upperBound...])) }
  return IrcMessage(tags: tags, source: source, command: command, params: params)
}

// ─── Driving the app ───────────────────────────────────────────────

/// Let pending main-actor tasks (sends, onConnected, history fetches) run.
@MainActor
func settle() async {
  for _ in 0..<5 {
    for _ in 0..<20 { await Task.yield() }
    try? await Task.sleep(for: .milliseconds(1))
  }
}

@MainActor
func sleep(ms: Int) async {
  try? await Task.sleep(for: .milliseconds(ms))
  await settle()
}

/// A fresh app on a fake backend, recording desktop notifications.
@MainActor
final class Harness {
  let backend = FakeBackend()
  let app: AppModel
  /// (title, body) of every desktop notification.
  var notified: [(title: String, body: String)] = []

  init() {
    app = AppModel(backend: backend)
    app.notify = { [unowned self] t, b, _ in self.notified.append((t, b)) }
  }

  var sent: [String] {
    get { backend.sent }
    set { backend.sent = newValue }
  }

  var conn: Conn { app.conns["c"]! }
  func buf(_ name: String) -> Buffer { conn.buffer(name)! }
  func lines(_ name: String) -> [Line] { buf(name).messages }
  func texts(_ name: String) -> [String] { lines(name).map(\.text) }
  var toasts: [String] { app.toasts.map(\.text) }

  /// Register connection "c" (test-utils.ts `connect`), then let the
  /// connected handler run as far as it would synchronously in the TS.
  func connect(
    id: String = "c", caps: [String] = [], capValues: [String: String] = [:], isupport: [String: String] = [:],
    nick: String = "me"
  ) async {
    HearthTests.connect(app, id: id, caps: caps, capValues: capValues, isupport: isupport, nick: nick)
    await settle()
  }

  /// Feed raw lines on connection "c", then let resulting tasks run.
  func feed(_ lines: String...) async {
    feedLines(app, "c", lines)
    await settle()
  }

  /// Join `chan` and load a NAMES list.
  func joinWith(_ chan: String, _ names: String) async {
    await feed(":me!u@h JOIN \(chan)", ":srv 353 me = \(chan) :\(names)", ":srv 366 me \(chan) :End of /NAMES list.")
  }

  func nicks(_ name: String) -> [String] { buf(name).sortedMembers.map { $0.prefix + $0.nick } }

  func run(_ input: String, _ b: Buffer? = nil, replyTo: String? = nil) async -> Bool {
    let ok = await app.runInput(b ?? buf("#a"), input, replyTo: replyTo)
    await settle()
    return ok
  }
}

/// Drive conn-added, registered, isupport and connected events
/// (test-utils.ts `connect`). Call `settle()` afterwards to run onConnected.
@MainActor
func connect(
  _ app: AppModel, id: String = "c", caps: [String] = [], capValues: [String: String] = [:],
  isupport: [String: String] = [:], nick: String = "me"
) {
  var sup = ["CHANTYPES": "#", "PREFIX": "(ov)@+", "CASEMAPPING": "ascii"]
  sup.merge(isupport) { $1 }
  app.handleEvents([
    .connAdded(conn: id, server: id, parent: nil, netid: nil),
    .registered(conn: id, nick: nick, caps: caps, capValues: capValues),
    .isupport(conn: id, isupport: sup),
    .status(conn: id, status: .connected, error: nil, retryIn: nil),
  ])
}

/// Parse raw lines and hand them to the app as message events (test-utils.ts `feed`).
@MainActor
func feed(_ app: AppModel, _ conn: String = "c", _ lines: String...) { feedLines(app, conn, lines) }

@MainActor
func feedLines(_ app: AppModel, _ conn: String, _ lines: [String]) {
  app.handleEvents(lines.map { .message(conn: conn, msg: msg($0)) })
}

/// Milliseconds for an ISO timestamp, like Date.parse.
func ms(_ iso: String) -> Int { parseIso(iso)! }
