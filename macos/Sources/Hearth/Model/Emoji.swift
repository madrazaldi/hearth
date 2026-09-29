// Emoji shortcodes (src/lib/emoji.ts): `:name` suggestions in the composer and
// `:name:` expansion. Offsets are UTF-16, like NSTextView's ranges.

import Foundation

struct Emoji: Equatable {
  let name: String
  let char: String
}

enum Emojis {
  static let all: [Emoji] = emojiData.split(separator: "\n").map { l in
    let parts = l.split(separator: " ", maxSplits: 1)
    return Emoji(name: String(parts[0]), char: String(parts[1]))
  }

  private static let byName = Dictionary(all.map { ($0.name, $0.char) }, uniquingKeysWith: { a, _ in a })

  static func named(_ name: String) -> String? { byName[name.lowercased()] }

  /// The `:name` being typed at the end of `before`, with where its colon is.
  static func query(_ before: String) -> (start: Int, query: String)? {
    let ns = before as NSString
    return token(ns, end: ns.length).map { ($0.start, $0.name.lowercased()) }
  }

  /// Names starting with the query first, then names containing it, each in gemoji order.
  static func search(_ query: String, limit: Int = 7) -> [Emoji] {
    let q = query.lowercased()
    var out = all.filter { $0.name.hasPrefix(q) }
    if out.count < limit { out += all.filter { !$0.name.hasPrefix(q) && $0.name.contains(q) } }
    return Array(out.prefix(limit))
  }

  /// A complete `:name:` just typed at the end of `before`, as its emoji and where it starts.
  static func typed(_ before: String) -> (start: Int, char: String)? {
    let ns = before as NSString
    guard ns.length > 0, ns.character(at: ns.length - 1) == colon,
      let t = token(ns, end: ns.length - 1), let char = named(t.name)
    else { return nil }
    return (t.start, char)
  }

  private static let colon = UInt16(UInt8(ascii: ":"))

  /// `:name` ending at `end`. The colon has to start a word, so `12:30`, URLs and `nick:` don't count.
  private static func token(_ s: NSString, end: Int) -> (start: Int, name: String)? {
    var i = end
    while i > 0, isNameChar(s.character(at: i - 1)) { i -= 1 }
    guard i < end, i > 0, s.character(at: i - 1) == colon else { return nil }
    let start = i - 1
    if start > 0, !(Unicode.Scalar(s.character(at: start - 1)).map(CharacterSet.whitespacesAndNewlines.contains) ?? false) {
      return nil
    }
    return (start, s.substring(with: NSRange(location: i, length: end - i)))
  }

  private static func isNameChar(_ c: UInt16) -> Bool {
    switch c {
    case 0x30...0x39, 0x41...0x5A, 0x61...0x7A, 0x5F, 0x2B, 0x2D: true  // 0-9 A-Z a-z _ + -
    default: false
    }
  }
}
