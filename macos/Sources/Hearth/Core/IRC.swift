// Protocol helpers that don't depend on app state (src/lib/irc.ts).

import Foundation

/// Milliseconds since the epoch, the unit every line time uses.
func nowMs() -> Int { Int(Date().timeIntervalSince1970 * 1000) }

func casefold(_ s: String, _ mapping: String = "rfc1459") -> String {
  var out = s.lowercased()
  if mapping == "rfc1459" || mapping == "rfc1459-strict" {
    out = out.replacingOccurrences(of: "[", with: "{").replacingOccurrences(of: "]", with: "}")
      .replacingOccurrences(of: "\\", with: "|")
    if mapping == "rfc1459" { out = out.replacingOccurrences(of: "~", with: "^") }
  }
  return out
}

struct PrefixMap: Equatable {
  /// e.g. "qaohv"
  var modes: [Character]
  /// e.g. "~&@%+"
  var symbols: [Character]

  func rank(of symbol: Character) -> Int { symbols.firstIndex(of: symbol) ?? symbols.count }
}

func parsePrefix(_ isupport: [String: String]) -> PrefixMap {
  let fallback = PrefixMap(modes: ["o", "v"], symbols: ["@", "+"])
  let raw = isupport["PREFIX"] ?? "(ov)@+"
  guard raw.hasPrefix("("), let close = raw.firstIndex(of: ")") else { return fallback }
  let modes = Array(raw[raw.index(after: raw.startIndex)..<close])
  let symbols = Array(raw[raw.index(after: close)...])
  return modes.count == symbols.count ? PrefixMap(modes: modes, symbols: symbols) : fallback
}

/// Parse a CTCP payload wrapped in \x01.
func parseCtcp(_ text: String) -> (command: String, args: String)? {
  guard text.hasPrefix("\u{1}") else { return nil }
  var body = String(text.dropFirst())
  if body.hasSuffix("\u{1}") { body.removeLast() }
  guard let sp = body.firstIndex(of: " ") else { return (body.uppercased(), "") }
  return (String(body[..<sp]).uppercased(), String(body[body.index(after: sp)...]))
}

private let isoFrac: ISO8601DateFormatter = {
  let f = ISO8601DateFormatter()
  f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
  return f
}()
private let isoPlain = ISO8601DateFormatter()

func isoTime(_ ms: Int) -> String {
  isoFrac.string(from: Date(timeIntervalSince1970: Double(ms) / 1000))
}

func parseIso(_ s: String) -> Int? {
  guard let d = isoFrac.date(from: s) ?? isoPlain.date(from: s) else { return nil }
  return Int((d.timeIntervalSince1970 * 1000).rounded())
}

func msgTime(_ tags: [String: String]) -> Int {
  tags["time"].flatMap(parseIso) ?? nowMs()
}

/// Split text into chunks that fit an IRC line, preferring word boundaries.
func splitMessage(_ text: String, maxBytes: Int) -> [String] {
  if text.utf8.count <= maxBytes { return [text] }
  var out: [String] = []
  var rest = Substring(text)
  while rest.utf8.count > maxBytes {
    // The longest prefix, in whole characters, that fits in maxBytes.
    var cut = rest.startIndex
    var bytes = 0
    for i in rest.indices {
      let n = rest[i].utf8.count
      if bytes + n > maxBytes { break }
      bytes += n
      cut = rest.index(after: i)
    }
    if cut == rest.startIndex { cut = rest.index(after: cut) }
    let chars = rest.distance(from: rest.startIndex, to: cut)
    if let space = rest[..<cut].lastIndex(of: " "), Double(rest.distance(from: rest.startIndex, to: space)) > Double(chars) * 0.6 {
      cut = space
    }
    out.append(String(rest[..<cut]))
    rest = rest[cut...]
    if rest.first == " " { rest = rest.dropFirst() }
  }
  if !rest.isEmpty { out.append(String(rest)) }
  return out
}

/// Parse "a=b;c=d" attribute lists used by soju BOUNCER messages.
func parseAttrs(_ s: String) -> [String: String] {
  var out: [String: String] = [:]
  if s.isEmpty || s == "*" { return out }
  for part in s.split(separator: ";", omittingEmptySubsequences: false) {
    let (k, v): (Substring, Substring)
    if let eq = part.firstIndex(of: "=") {
      (k, v) = (part[..<eq], part[part.index(after: eq)...])
    } else {
      (k, v) = (part, "")
    }
    var value = ""
    var it = v.makeIterator()
    while let c = it.next() {
      guard c == "\\", let e = it.next() else {
        value.append(c)
        continue
      }
      switch e {
      case ":": value.append(";")
      case "s": value.append(" ")
      case "\\": value.append("\\")
      case "r": value.append("\r")
      case "n": value.append("\n")
      default: value.append(e)
      }
    }
    out[String(k)] = value
  }
  return out
}

func escapeAttr(_ v: String) -> String {
  v.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: ";", with: "\\:")
    .replacingOccurrences(of: " ", with: "\\s")
}

/// Escape an IRCv3 message-tag value.
func escapeTag(_ v: String) -> String {
  escapeAttr(v).replacingOccurrences(of: "\r", with: "\\r").replacingOccurrences(of: "\n", with: "\\n")
}

/// Split "nick!user@host" into its parts.
func parseMask(_ s: String) -> Source {
  let at = s.split(separator: "@", maxSplits: 1, omittingEmptySubsequences: false)
  let nu = at[0].split(separator: "!", maxSplits: 1, omittingEmptySubsequences: false)
  return Source(nick: String(nu[0]), user: nu.count > 1 ? String(nu[1]) : nil, host: at.count > 1 ? String(at[1]) : nil)
}
