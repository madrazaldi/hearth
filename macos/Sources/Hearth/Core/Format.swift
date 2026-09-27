// mIRC formatting codes, link detection and highlight rules (src/lib/format.ts).

import Foundation

enum IrcColor: Equatable, Hashable {
  /// One of the 16 themed base colours.
  case palette(Int)
  case rgb(UInt32)
  /// The buffer's own background or text colour, for reverse video.
  case background, foreground
}

struct Segment: Equatable {
  var text: String
  var bold = false
  var italic = false
  var underline = false
  var strike = false
  var mono = false
  var fg: IrcColor?
  var bg: IrcColor?
  var href: String?
  var channel: String?
}

// Colours 16–98 from the extended mIRC table.
private let extended: [UInt32] = [
  0x470000, 0x472100, 0x474700, 0x324700, 0x004700, 0x00472c, 0x004747, 0x002747, 0x000047, 0x2e0047, 0x470047, 0x47002a,
  0x740000, 0x743a00, 0x747400, 0x517400, 0x007400, 0x007449, 0x007474, 0x004074, 0x000074, 0x4b0074, 0x740074, 0x740045,
  0xb50000, 0xb56300, 0xb5b500, 0x7db500, 0x00b500, 0x00b571, 0x00b5b5, 0x0063b5, 0x0000b5, 0x7500b5, 0xb500b5, 0xb5006b,
  0xff0000, 0xff8c00, 0xffff00, 0xb2ff00, 0x00ff00, 0x00ffa0, 0x00ffff, 0x008cff, 0x0000ff, 0xa500ff, 0xff00ff, 0xff0098,
  0xff5959, 0xffb459, 0xffff71, 0xcfff60, 0x6fff6f, 0x65ffc9, 0x6dffff, 0x59b4ff, 0x5959ff, 0xc459ff, 0xff66ff, 0xff59bc,
  0xff9c9c, 0xffd39c, 0xffff9c, 0xe2ff9c, 0x9cff9c, 0x9cffdb, 0x9cffff, 0x9cd3ff, 0x9c9cff, 0xdc9cff, 0xff9cff, 0xff94d3,
  0x000000, 0x131313, 0x282828, 0x363636, 0x4d4d4d, 0x656565, 0x818181, 0x9f9f9f, 0xbcbcbc, 0xe2e2e2, 0xffffff,
]

private func color(_ n: Int) -> IrcColor? {
  if (0...15).contains(n) { return .palette(n) }
  if (16...98).contains(n) { return .rgb(extended[n - 16]) }
  return nil
}

private let formatCodes = /\x03(\d{1,2}(,\d{1,2})?)?|\x04([0-9a-fA-F]{6}(,[0-9a-fA-F]{6})?)?|[\x02\x0f\x11\x16\x1d\x1e\x1f]/

func stripFormatting(_ s: String) -> String {
  guard s.unicodeScalars.contains(where: { $0.value < 0x20 }) else { return s }
  return s.replacing(formatCodes, with: "")
}

/// Up to `max` of `chars` (digits or hex) at the start of `s`.
private func leading(_ s: Substring.UnicodeScalarView, max: Int, hex: Bool) -> String {
  var out = ""
  for c in s.prefix(max) {
    let ok = hex ? c.properties.isASCIIHexDigit : ("0"..."9").contains(c)
    guard ok else { break }
    out.unicodeScalars.append(c)
  }
  return out
}

func parseFormatting(_ input: String) -> [Segment] {
  var out: [Segment] = []
  var cur = Segment(text: "")
  func push() {
    if !cur.text.isEmpty { out.append(cur) }
    cur.text = ""
  }
  let scalars = Substring(input).unicodeScalars
  var i = scalars.startIndex
  while i < scalars.endIndex {
    let c = scalars[i]
    switch c.value {
    case 0x02: push(); cur.bold.toggle()
    case 0x1d: push(); cur.italic.toggle()
    case 0x1f: push(); cur.underline.toggle()
    case 0x1e: push(); cur.strike.toggle()
    case 0x11: push(); cur.mono.toggle()
    case 0x16:
      push()
      (cur.fg, cur.bg) = (cur.bg ?? .background, cur.fg ?? .foreground)
    case 0x0f:
      push()
      cur = Segment(text: "")
    case 0x03:
      push()
      var j = scalars.index(after: i)
      let fg = leading(scalars[j...], max: 2, hex: false)
      if fg.isEmpty {
        cur.fg = nil
        cur.bg = nil
      } else {
        j = scalars.index(j, offsetBy: fg.unicodeScalars.count)
        cur.fg = color(Int(fg)!)
        if j < scalars.endIndex, scalars[j] == ",", case let bg = leading(scalars[scalars.index(after: j)...], max: 2, hex: false),
          !bg.isEmpty
        {
          cur.bg = color(Int(bg)!)
          j = scalars.index(j, offsetBy: 1 + bg.unicodeScalars.count)
        }
      }
      i = j
      continue
    case 0x04:
      push()
      var j = scalars.index(after: i)
      let fg = leading(scalars[j...], max: 6, hex: true)
      if fg.unicodeScalars.count < 6 {
        cur.fg = nil
        cur.bg = nil
      } else {
        j = scalars.index(j, offsetBy: 6)
        cur.fg = .rgb(UInt32(fg, radix: 16)!)
        if j < scalars.endIndex, scalars[j] == ",", case let bg = leading(scalars[scalars.index(after: j)...], max: 6, hex: true),
          bg.unicodeScalars.count == 6
        {
          cur.bg = .rgb(UInt32(bg, radix: 16)!)
          j = scalars.index(j, offsetBy: 7)
        }
      }
      i = j
      continue
    default:
      cur.text.unicodeScalars.append(c)
    }
    i = scalars.index(after: i)
  }
  push()
  return out.flatMap(linkify)
}

private let urlRe = try! NSRegularExpression(
  pattern: #"\b(?:https?://|www\.)[^\s<>"'\x00-\x1f]+|\bmailto:[^\s<>"']+"#, options: [.caseInsensitive])
private let chanRe = try! NSRegularExpression(pattern: #"(^|[\s(])(#[^\s,\x07()]{1,64})"#)

private func trimUrl(_ url: String) -> String {
  var url = url
  // Drop trailing punctuation, but keep a closing paren that balances one in the URL.
  while let last = url.last,
    ".,;:!?'\"]>".contains(last) || (last == ")" && url.filter { $0 == "(" }.count < url.filter { $0 == ")" }.count)
  {
    url.removeLast()
  }
  return url
}

private func linkify(_ seg: Segment) -> [Segment] {
  let text = seg.text as NSString
  var found: [(range: NSRange, href: String?, channel: String?)] = []
  let all = NSRange(location: 0, length: text.length)
  for m in urlRe.matches(in: seg.text, range: all) {
    let url = trimUrl(text.substring(with: m.range))
    let href = url.lowercased().hasPrefix("www.") ? "https://" + url : url
    found.append((NSRange(location: m.range.location, length: (url as NSString).length), href, nil))
  }
  for m in chanRe.matches(in: seg.text, range: all) {
    var name = text.substring(with: m.range(at: 2))
    while let l = name.last, ".,;:!?'\"".contains(l) { name.removeLast() }
    let r = NSRange(location: m.range(at: 2).location, length: (name as NSString).length)
    if !found.contains(where: { NSIntersectionRange($0.range, r).length > 0 }) { found.append((r, nil, name)) }
  }
  if found.isEmpty { return [seg] }
  found.sort { $0.range.location < $1.range.location }
  var out: [Segment] = []
  var pos = 0
  for f in found {
    if f.range.location > pos {
      var s = seg
      s.text = text.substring(with: NSRange(location: pos, length: f.range.location - pos))
      out.append(s)
    }
    var s = seg
    s.text = text.substring(with: f.range)
    s.href = f.href
    s.channel = f.channel
    out.append(s)
    pos = f.range.location + f.range.length
  }
  if pos < text.length {
    var s = seg
    s.text = text.substring(from: pos)
    out.append(s)
  }
  return out
}

// ─── Highlight rules ─────────────────────────────────────────────

private let regexRule = try! NSRegularExpression(pattern: #"^/(.+)/([a-z]*)$"#, options: [.dotMatchesLineSeparators])
private let ruleCache = Memo<NSRegularExpression?>(limit: 500)

/// `/pattern/flags` rules as (pattern, flags); nil for a plain word.
private func regexParts(_ rule: String) -> (String, String)? {
  let ns = rule as NSString
  guard let m = regexRule.firstMatch(in: rule, range: NSRange(location: 0, length: ns.length)) else { return nil }
  return (ns.substring(with: m.range(at: 1)), ns.substring(with: m.range(at: 2)))
}

/// JavaScript regex flags as NSRegularExpression options. g and y only make
/// JS `.test()` stateful, so they're accepted and ignored.
private func options(_ flags: String) throws -> NSRegularExpression.Options {
  var opts: NSRegularExpression.Options = []
  var seen = Set<Character>()
  for f in flags {
    guard seen.insert(f).inserted else { throw CoreError(message: "Invalid flags supplied to RegExp constructor '\(flags)'") }
    switch f {
    case "i": opts.insert(.caseInsensitive)
    case "m": opts.insert(.anchorsMatchLines)
    case "s": opts.insert(.dotMatchesLineSeparators)
    case "g", "y", "u", "d", "v": break
    default: throw CoreError(message: "Invalid flags supplied to RegExp constructor '\(flags)'")
    }
  }
  return opts
}

private func compileRule(_ rule: String) -> NSRegularExpression? {
  ruleCache.value(rule) {
    if let (pattern, flags) = regexParts(rule) {
      return try? NSRegularExpression(pattern: pattern, options: options(flags))
    }
    let escaped = NSRegularExpression.escapedPattern(for: rule)
    return try? NSRegularExpression(pattern: "(^|[^\\w-])\(escaped)($|[^\\w-])", options: [.caseInsensitive])
  }
}

/// Why a highlight rule is invalid, or nil. `/pattern/flags` rules are regexes; anything else is a word.
func ruleError(_ rule: String) -> String? {
  guard let (pattern, flags) = regexParts(rule.trimmingCharacters(in: .whitespaces)) else { return nil }
  do {
    _ = try NSRegularExpression(pattern: pattern, options: options(flags))
    return nil
  } catch let e as CoreError {
    return e.message
  } catch {
    return "Invalid regular expression"
  }
}

/// The first rule that matches: words match whole-word and case-insensitively, `/regex/flags` as written.
func matchingRule(_ text: String, _ rules: [String]) -> String? {
  let plain = stripFormatting(text)
  let range = NSRange(location: 0, length: (plain as NSString).length)
  for r in rules where !r.isEmpty {
    if compileRule(r)?.firstMatch(in: plain, range: range) != nil { return r }
  }
  return nil
}

func mentions(_ text: String, _ rules: [String]) -> Bool { matchingRule(text, rules) != nil }

/// The first http(s) link in a message, for previews.
func firstLink(_ text: String) -> String? {
  let plain = stripFormatting(text)
  let ns = plain as NSString
  for m in urlRe.matches(in: plain, range: NSRange(location: 0, length: ns.length)) {
    let url = trimUrl(ns.substring(with: m.range))
    if url.lowercased().hasPrefix("mailto:") { continue }
    return url.lowercased().hasPrefix("www.") ? "https://" + url : url
  }
  return nil
}
