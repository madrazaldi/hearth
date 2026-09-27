// Small pure helpers ported from src/lib: ignore masks, draft/multiline,
// aliases, nick colours, time formatting, onboarding and media labels.

import Foundation

/// A string-keyed memo behind a lock: these helpers are called from views and
/// from tests that run off the main actor.
final class Memo<Value>: @unchecked Sendable {
  private var store: [String: Value] = [:]
  private let lock = NSLock()
  private let limit: Int

  init(limit: Int = .max) { self.limit = limit }

  func value(_ key: String, _ make: () -> Value) -> Value {
    lock.withLock {
      if let v = store[key] { return v }
      let v = make()
      if store.count >= limit { store.removeAll() }
      store[key] = v
      return v
    }
  }
}

// ─── Ignore masks (ignore.ts) ────────────────────────────────────

/// Expand shorthand into a full nick!user@host mask.
func normalizeMask(_ input: String) -> String {
  let m = input.trimmingCharacters(in: .whitespaces)
  if m.isEmpty { return "" }
  if m.contains("!") { return m.contains("@") ? m : "\(m)@*" }
  if m.contains("@") { return "*!\(m)" }
  return "\(m)!*@*"
}

private let maskCache = Memo<NSRegularExpression>()

private func maskRegex(_ mask: String) -> NSRegularExpression {
  maskCache.value(mask) {
    var body = ""
    for c in mask {
      switch c {
      case "*": body += ".*"
      case "?": body += "."
      default: body += NSRegularExpression.escapedPattern(for: String(c))
      }
    }
    return try! NSRegularExpression(pattern: "^\(body)$", options: [.dotMatchesLineSeparators])
  }
}

/// Whether `source` matches any mask. `fold` case-folds per the server's casemapping.
func isIgnored(_ masks: [String], _ source: Source, fold: (String) -> String) -> Bool {
  if masks.isEmpty || source.nick.isEmpty { return false }
  let full = fold("\(source.nick)!\(source.user ?? "")@\(source.host ?? "")")
  let range = NSRange(location: 0, length: (full as NSString).length)
  return masks.contains { maskRegex(fold(normalizeMask($0))).firstMatch(in: full, range: range) != nil }
}

// ─── draft/multiline (multiline.ts) ──────────────────────────────

struct MultilineLimits: Equatable {
  var maxBytes: Int
  var maxLines: Int
}

/// One PRIVMSG in a batch; `concat` joins it to the previous one without a line break.
struct MultilinePart: Equatable {
  var text: String
  var concat: Bool
}

/// Parse a cap value like "max-bytes=4096,max-lines=24". max-bytes is mandatory.
func parseMultilineCap(_ value: String?) -> MultilineLimits? {
  var attrs: [String: String] = [:]
  for kv in (value ?? "").split(separator: ",") {
    let p = kv.split(separator: "=", maxSplits: 1)
    attrs[String(p[0])] = p.count > 1 ? String(p[1]) : ""
  }
  guard let maxBytes = attrs["max-bytes"].flatMap({ Int($0) }), maxBytes > 0 else { return nil }
  let maxLines = attrs["max-lines"].flatMap { Int($0) } ?? 0
  return MultilineLimits(maxBytes: maxBytes, maxLines: maxLines > 0 ? maxLines : Int.max)
}

/// Split text at byte limits without losing anything, since the pieces are
/// concatenated as-is: cuts go after a space when there's one near the end.
private func splitExact(_ text: String, _ maxBytes: Int) -> [String] {
  var out: [String] = []
  var rest = Substring(text)
  while rest.utf8.count > maxBytes {
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
    if let space = rest[..<cut].lastIndex(of: " ") {
      let after = rest.index(after: space)
      if Double(rest.distance(from: rest.startIndex, to: after)) > Double(chars) * 0.6 { cut = after }
    }
    out.append(String(rest[..<cut]))
    rest = rest[cut...]
  }
  out.append(String(rest))
  return out
}

/// The batches to send `text` in: each is a list of parts that fits the
/// server's limits, with over-long lines split into concatenated pieces.
func multilineBatches(_ text: String, limits: MultilineLimits, lineBytes: Int) -> [[MultilinePart]] {
  var t = text.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\r", with: "\n")
  while t.hasPrefix("\n") { t.removeFirst() }
  while t.hasSuffix("\n") { t.removeLast() }
  let parts: [MultilinePart] = t.split(separator: "\n", omittingEmptySubsequences: false).flatMap { line in
    line.isEmpty
      ? [MultilinePart(text: "", concat: false)]
      : splitExact(String(line), min(lineBytes, limits.maxBytes - 1)).enumerated().map {
        MultilinePart(text: $0.element, concat: $0.offset > 0)
      }
  }
  var batches: [[MultilinePart]] = []
  var cur: [MultilinePart] = []
  var size = 0
  for part in parts {
    // Count a line break for every part: conservative about how servers measure.
    let n = part.text.utf8.count + 1
    if !cur.isEmpty && (cur.count >= limits.maxLines || size + n > limits.maxBytes) {
      batches.append(cur)
      cur = []
      size = 0
    }
    // A batch can't open with a continuation; it starts a fresh line instead.
    cur.append(cur.isEmpty ? MultilinePart(text: part.text, concat: false) : part)
    size += n
  }
  if !cur.isEmpty { batches.append(cur) }
  // A batch of nothing but blank lines is invalid.
  return batches.filter { $0.contains { !$0.text.trimmingCharacters(in: .whitespaces).isEmpty } }
}

/// Join parts back into the text readers see.
func joinParts(_ parts: [MultilinePart]) -> String {
  parts.enumerated().map { ($0.offset > 0 && !$0.element.concat ? "\n" : "") + $0.element.text }.joined()
}

// ─── Aliases (aliases.ts) ────────────────────────────────────────

struct AliasVars {
  var nick: String
  var channel: String
  var network: String
}

/// Split a body into its `;`-separated commands. `\;` is a literal semicolon.
func splitCommands(_ body: String) -> [String] {
  var out: [String] = []
  var cur = ""
  var prev: Character?
  for c in body {
    if c == ";" && prev != "\\" {
      out.append(cur)
      cur = ""
    } else {
      cur.append(c)
    }
    prev = c
  }
  out.append(cur)
  return out.map { $0.replacingOccurrences(of: "\\;", with: ";").trimmingCharacters(in: .whitespaces) }.filter {
    !$0.isEmpty
  }
}

private let aliasVar = /\$(?:(\*)|([1-9])(-?)|(nick|channel|network)(?![A-Za-z]))/

/// Substitute `$1`…`$9`, `$2-` (from the 2nd on), `$*`, `$nick`, `$channel` and `$network`.
func expandCommand(_ cmd: String, args: [String], vars: AliasVars) -> String {
  let out = cmd.replacing(aliasVar) { m -> String in
    if m.output.1 != nil { return args.joined(separator: " ") }
    if let n = m.output.2.flatMap({ Int($0) }) {
      if m.output.3 == "-" { return args.dropFirst(n - 1).joined(separator: " ") }
      return n - 1 < args.count ? args[n - 1] : ""
    }
    switch m.output.4 {
    case "nick": return vars.nick
    case "channel": return vars.channel
    default: return vars.network
    }
  }
  return String(out.reversed().drop(while: \.isWhitespace).reversed())
}

/// `/J ` → `j`.
func aliasName(_ raw: String) -> String {
  var s = raw.trimmingCharacters(in: .whitespaces)
  while s.hasPrefix("/") { s.removeFirst() }
  return s.lowercased()
}

func aliasError(name: String, body: String) -> String? {
  if name.isEmpty { return "Give the alias a name" }
  if name.wholeMatch(of: /[a-z0-9_-]+/) == nil { return "Names use letters, digits, - and _ only" }
  if splitCommands(body).isEmpty { return "Add at least one command" }
  return nil
}

// ─── Nick colours (nickcolor.ts) ─────────────────────────────────

private let hueCache = Memo<Double>()

/// A stable hue in degrees for a nick; lightness and chroma come from the theme.
func nickHue(_ nick: String) -> Double {
  var key = nick.lowercased()
  while let l = key.last, "_`|^".contains(l) { key.removeLast() }
  return hueCache.value(key) {
    // FNV-1a over UTF-16 code units, as in the Svelte app, so colours match.
    var hash: UInt32 = 2_166_136_261
    for u in key.utf16 {
      hash ^= UInt32(u)
      hash = hash &* 16_777_619
    }
    return Double(hash % 360)
  }
}

// ─── Time (time.ts) ──────────────────────────────────────────────

func date(_ ms: Int) -> Date { Date(timeIntervalSince1970: Double(ms) / 1000) }

func formatClock(_ ms: Int, clock: Clock, seconds: Bool) -> String {
  let c = Calendar.current.dateComponents([.hour, .minute, .second], from: date(ms))
  var h = c.hour ?? 0
  let m = String(format: "%02d", c.minute ?? 0)
  let sec = seconds ? String(format: ":%02d", c.second ?? 0) : ""
  if clock == .h12 {
    let ap = h >= 12 ? "p" : "a"
    h = h % 12 == 0 ? 12 : h % 12
    return "\(h):\(m)\(sec)\(ap)"
  }
  return String(format: "%02d", h) + ":\(m)\(sec)"
}

func formatDay(_ ms: Int) -> String {
  let d = date(ms)
  let cal = Calendar.current
  if cal.isDateInToday(d) { return "Today" }
  if cal.isDateInYesterday(d) { return "Yesterday" }
  let sameYear = cal.component(.year, from: d) == cal.component(.year, from: Date())
  return d.formatted(
    sameYear ? .dateTime.weekday(.wide).month(.wide).day() : .dateTime.weekday(.wide).month(.wide).day().year())
}

func formatFull(_ ms: Int) -> String {
  date(ms).formatted(date: .abbreviated, time: .standard)
}

func formatDuration(_ secs: Int) -> String {
  var secs = secs
  var parts: [String] = []
  for (u, n) in [("d", 86400), ("h", 3600), ("m", 60), ("s", 1)] {
    if secs >= n {
      parts.append("\(secs / n)\(u)")
      secs %= n
    }
    if parts.count == 2 { break }
  }
  return parts.isEmpty ? "0s" : parts.joined(separator: " ")
}

// ─── Media (media.ts) ────────────────────────────────────────────

/// Messages that mention NSFW/NSFL get their images blurred until clicked.
func isNsfw(_ text: String) -> Bool {
  stripFormatting(text).firstMatch(of: /(?i)(^|[^a-z0-9])nsf[wl]([^a-z0-9]|$)/) != nil
}

func formatBytes(_ n: Int) -> String {
  if n < 1024 { return "\(n) B" }
  let units = ["KB", "MB", "GB"]
  var v = Double(n) / 1024
  var i = 0
  while v >= 1024 && i < units.count - 1 {
    v /= 1024
    i += 1
  }
  return (v < 10 ? String(format: "%.1f", v) : String(Int(v.rounded()))) + " " + units[i]
}

/// "Video · MP4 · 12 MB"
func mediaLabel(_ p: Preview) -> String {
  let mime = (p.mime ?? "").split(separator: "/", maxSplits: 1).map(String.init)
  let type = mime.first ?? ""
  let sub = mime.count > 1 ? mime[1] : ""
  let kind = type == "video" ? "Video" : type == "audio" ? "Audio" : type == "image" ? "Image" : "File"
  var parts = [kind]
  var fmt = sub
  for prefix in ["x-", "vnd."] where fmt.hasPrefix(prefix) { fmt.removeFirst(prefix.count) }
  fmt = String(fmt.split(whereSeparator: { "+;.".contains($0) }).first ?? "")
  if !fmt.isEmpty { parts.append(fmt == "mpeg" && type == "audio" ? "MP3" : fmt.uppercased()) }
  if let w = p.width, let h = p.height { parts.append("\(w)×\(h)") }
  if let s = p.size { parts.append(formatBytes(s)) }
  return parts.joined(separator: " · ")
}

/// The last path segment of a URL, decoded, for naming a file card.
func fileName(_ url: String) -> String {
  guard let u = URL(string: url) else { return url }
  return u.pathComponents.filter { $0 != "/" }.last ?? u.host ?? url
}
