// Pure-function tests: irc.ts, ignore.ts, format.ts, aliases.ts, multiline.ts,
// onboarding.ts and the media helpers (ignore/format/aliases/ircv3/onboarding/media .test.ts).

import Foundation
import Testing

@testable import Hearth

private let B = "\u{02}"
private let C = "\u{03}"
private let HEX = "\u{04}"
private let I = "\u{1d}"
private let U = "\u{1f}"
private let S = "\u{1e}"
private let M = "\u{11}"
private let R = "\u{16}"
private let O = "\u{0f}"

private func seg(_ s: String) -> [Segment] { parseFormatting(s) }
private func links(_ s: String) -> [String] { parseFormatting(s).compactMap(\.href) }
private func chans(_ s: String) -> [String] { parseFormatting(s).compactMap(\.channel) }

@MainActor @Suite("ignore masks")
struct IgnoreMaskTests {
  @Test("normalizes shorthand")
  func normalizesShorthand() {
    #expect(normalizeMask("nick") == "nick!*@*")
    #expect(normalizeMask("*@host") == "*!*@host")
    #expect(normalizeMask("nick!user") == "nick!user@*")
    #expect(normalizeMask(" a!b@c ") == "a!b@c")
  }

  @Test("matches with wildcards and case-folding")
  func matchesWildcards() {
    let fold = { (s: String) in casefold(s, "rfc1459") }
    #expect(isIgnored(["Troll[1]"], Source(nick: "troll{1}", user: "x", host: "y"), fold: fold))
    #expect(isIgnored(["*!*@*.bad.net"], Source(nick: "a", user: "b", host: "x.bad.net"), fold: fold))
    #expect(isIgnored(["a?c"], Source(nick: "abc"), fold: fold))
    #expect(!isIgnored(["a?c"], Source(nick: "abbc"), fold: fold))
    #expect(!isIgnored(["x.y"], Source(nick: "xzy"), fold: fold))
  }
}

@MainActor @Suite("splitMessage")
struct SplitMessageTests {
  @Test("splits on word boundaries within the byte budget")
  func splitsOnWords() {
    let text = String(String(repeating: "lorem ipsum dolor sit amet ", count: 20).dropLast())
    let parts = splitMessage(text, maxBytes: 100)
    #expect(parts.allSatisfy { $0.utf8.count <= 100 })
    #expect(parts.joined(separator: " ") == text)
  }

  @Test("never splits inside a multi-byte character")
  func multiByte() {
    let parts = splitMessage(String(repeating: "é", count: 100), maxBytes: 51)
    #expect(parts.joined() == String(repeating: "é", count: 100))
    #expect(parts.allSatisfy { $0.utf8.count <= 51 })
  }
}

@MainActor @Suite("mIRC formatting")
struct FormattingTests {
  @Test("toggles bold, italic, underline, strike and monospace")
  func toggles() {
    #expect(
      seg("a\(B)b\(I)c\(B)d\(I)\(U)e\(U)\(S)f\(S)\(M)g") == [
        Segment(text: "a"),
        Segment(text: "b", bold: true),
        Segment(text: "c", bold: true, italic: true),
        Segment(text: "d", italic: true),
        Segment(text: "e", underline: true),
        Segment(text: "f", strike: true),
        Segment(text: "g", mono: true),
      ])
  }

  @Test("resets everything with ^O")
  func reset() {
    #expect(
      seg("\(B)\(C)4bold red\(O)plain") == [Segment(text: "bold red", bold: true, fg: .palette(4)), Segment(text: "plain")])
  }

  @Test("parses basic colours with and without backgrounds")
  func basicColours() {
    #expect(
      seg("\(C)4red\(C)04,2red on blue\(C)3green keeps bg\(C)plain") == [
        Segment(text: "red", fg: .palette(4)),
        Segment(text: "red on blue", fg: .palette(4), bg: .palette(2)),
        Segment(text: "green keeps bg", fg: .palette(3), bg: .palette(2)),
        Segment(text: "plain"),
      ])
  }

  @Test("reads at most two digits, so trailing digits are text")
  func twoDigits() {
    #expect(seg("\(C)1234") == [Segment(text: "34", fg: .palette(12))])
    #expect(seg("\(C)4,123") == [Segment(text: "3", fg: .palette(4), bg: .palette(12))])
  }

  @Test("keeps a comma that is not followed by a colour")
  func comma() {
    #expect(seg("\(C)4,hello") == [Segment(text: ",hello", fg: .palette(4))])
    #expect(seg("\(C),4x") == [Segment(text: ",4x")])
  }

  @Test("maps the extended 16–98 palette and treats 99 as default")
  func extended() {
    #expect(
      seg("\(C)16a\(C)52b\(C)98c\(C)99d") == [
        Segment(text: "a", fg: .rgb(0x470000)),
        Segment(text: "b", fg: .rgb(0xff0000)),
        Segment(text: "c", fg: .rgb(0xffffff)),
        Segment(text: "d"),
      ])
    #expect(seg("\(C)88,99x") == [Segment(text: "x", fg: .rgb(0x000000))])
  }

  @Test("parses hex colours")
  func hex() {
    #expect(
      seg("\(HEX)FF8800orange\(HEX)00ff00,000000green on black\(HEX)reset") == [
        Segment(text: "orange", fg: .rgb(0xFF8800)),
        Segment(text: "green on black", fg: .rgb(0x00ff00), bg: .rgb(0x000000)),
        Segment(text: "reset"),
      ])
    // Not six hex digits: a plain reset, the rest is text.
    #expect(seg("\(HEX)12345x") == [Segment(text: "12345x")])
  }

  @Test("reverses colours")
  func reverse() {
    #expect(seg("\(R)rev") == [Segment(text: "rev", fg: .background, bg: .foreground)])
    #expect(seg("\(C)4,2\(R)rev") == [Segment(text: "rev", fg: .palette(2), bg: .palette(4))])
  }

  @Test("drops empty segments")
  func dropsEmpty() {
    #expect(seg("\(B)\(B)\(C)4\(C)") == [])
  }

  @Test("stripFormatting removes every control code")
  func strip() {
    #expect(stripFormatting("\(B)bold\(B) \(C)04,12col\(C) \(HEX)ff0000,00ff00hex \(I)\(U)\(S)\(M)\(R)x\(O)") == "bold col hex x")
    #expect(stripFormatting("\(C)99,99plain") == "plain")
  }
}

@MainActor @Suite("link detection")
struct LinkTests {
  @Test("finds http(s), www and mailto links")
  func finds() {
    #expect(
      links("see https://example.com/a?b=1&c=2#frag and http://x.org") == [
        "https://example.com/a?b=1&c=2#frag", "http://x.org",
      ])
    #expect(links("www.example.com/path") == ["https://www.example.com/path"])
    #expect(links("mail mailto:ann@example.com") == ["mailto:ann@example.com"])
  }

  @Test("trims trailing punctuation")
  func trims() {
    #expect(links("go to https://example.com.") == ["https://example.com"])
    #expect(links("https://example.com/a, then") == ["https://example.com/a"])
    #expect(links("really? https://example.com/q?!") == ["https://example.com/q"])
    #expect(links("\"https://example.com/quoted\"") == ["https://example.com/quoted"])
    #expect(links("<https://example.com/angle>") == ["https://example.com/angle"])
    #expect(links("[https://example.com/sq]") == ["https://example.com/sq"])
  }

  @Test("balances parentheses")
  func parens() {
    #expect(links("https://en.wikipedia.org/wiki/Foo_(bar)") == ["https://en.wikipedia.org/wiki/Foo_(bar)"])
    #expect(links("(see https://en.wikipedia.org/wiki/Foo_(bar))") == ["https://en.wikipedia.org/wiki/Foo_(bar)"])
    #expect(links("(https://example.com)") == ["https://example.com"])
    #expect(links("(https://example.com).") == ["https://example.com"])
  }

  @Test("keeps surrounding text and formatting around a link")
  func surrounding() {
    #expect(
      seg("hi \(B)https://a.io/x.\(B) ok") == [
        Segment(text: "hi "),
        Segment(text: "https://a.io/x", bold: true, href: "https://a.io/x"),
        Segment(text: ".", bold: true),
        Segment(text: " ok"),
      ])
  }

  @Test("needs a word boundary before the scheme")
  func boundary() {
    #expect(links("xhttps://nope.example") == [])
    #expect(links("notwww.example.com") == [])
  }

  @Test("firstLink returns the first previewable link")
  func first() {
    #expect(firstLink("mailto:a@b.c then (www.example.com).") == "https://www.example.com")
    #expect(firstLink("\(C)4https://red.example\(C)!") == "https://red.example")
    #expect(firstLink("no links here") == nil)
  }
}

@MainActor @Suite("channel links")
struct ChannelLinkTests {
  @Test("linkifies #channels at word starts")
  func wordStarts() {
    #expect(chans("join #hearth, #rust-lang and (#svelte)") == ["#hearth", "#rust-lang", "#svelte"])
    #expect(chans("#start of line") == ["#start"])
    #expect(chans("see #hearth.") == ["#hearth"])
  }

  @Test("skips mid-word hashes and URL fragments")
  func midWord() {
    #expect(chans("issue#12 c#") == [])
    #expect(chans("https://example.com/ #real https://x.io/#frag") == ["#real"])
    #expect(links("https://example.com/ #real https://x.io/#frag") == ["https://example.com/", "https://x.io/#frag"])
  }

  @Test("splits the text around the channel")
  func splits() {
    #expect(seg("go to #a now") == [Segment(text: "go to "), Segment(text: "#a", channel: "#a"), Segment(text: " now")])
  }
}

@MainActor @Suite("mentions")
struct MentionTests {
  @Test("matches whole words case-insensitively")
  func wholeWords() {
    #expect(mentions("Ann: hi", ["ann"]))
    #expect(mentions("hi @ann!", ["ann"]))
    #expect(!mentions("annabelle", ["ann"]))
    #expect(!mentions("ann-marie", ["ann"]))
    #expect(!mentions("joann", ["ann"]))
  }

  @Test("handles regex metacharacters and nick punctuation")
  func metachars() {
    #expect(mentions("hey [ann]", ["[ann]"]))
    #expect(mentions("ping ann|away", ["ann|away"]))
    #expect(!mentions("ping ann", ["ann|away"]))
    #expect(mentions("I use c++", ["c++"]))
  }

  @Test("ignores formatting and empty words")
  func formatting() {
    #expect(mentions("\(B)a\(B)nn", ["ann"]))
    #expect(!mentions("anything", [""]))
  }
}

@MainActor @Suite("highlight rules")
struct HighlightRuleTests {
  @Test("treats /pattern/flags as a regex, matched as written")
  func regexRules() {
    #expect(mentions("CI failed on main", [#"/\bci (failed|broke)\b/i"#]))
    #expect(!mentions("CI failed on main", [#"/\bci failed\b/"#]))
    #expect(mentions("deployed", ["/deploy(ed|ing)?/"]))
    // A stateful flag must not make every other call miss.
    let rule = "/x/g"
    #expect([1, 2, 3].map { _ in mentions("x", [rule]) } == [true, true, true])
  }

  @Test("reports which rule matched, skipping invalid ones")
  func matching() {
    #expect(matchingRule("the build is red", ["ann", "/(unclosed/", "/build/"]) == "/build/")
    #expect(matchingRule("\u{02}ann\u{02}: hi", ["bob", "ann"]) == "ann")
    #expect(matchingRule("nothing here", ["ann"]) == nil)
    // Invalid flags stay invalid even though g is dropped when compiling.
    #expect(matchingRule("foo", ["/foo/gg"]) == nil)
  }

  @Test("validates regex rules and accepts plain words")
  func validates() {
    #expect(ruleError("deploy") == nil)
    #expect(ruleError("/a|b/i") == nil)
    #expect(!(ruleError("/(unclosed/") ?? "").isEmpty)
    #expect(ruleError("/a/q")?.localizedCaseInsensitiveContains("flag") == true)
  }
}

@MainActor @Suite("alias expansion")
struct AliasExpansionTests {
  let vars = AliasVars(nick: "me", channel: "#a", network: "Testnet")

  @Test("substitutes positional, rest and context variables")
  func substitutes() {
    #expect(expandCommand("/msg $1 $2-", args: ["ann", "hi", "there"], vars: vars) == "/msg ann hi there")
    #expect(
      expandCommand("/say $* in $channel on $network as $nick", args: ["x", "y"], vars: vars)
        == "/say x y in #a on Testnet as me")
    #expect(expandCommand("/kick $1 $3", args: ["ann"], vars: vars) == "/kick ann")
    #expect(expandCommand("$nickname costs $5", args: [], vars: vars) == "$nickname costs")
  }

  @Test(#"splits on ; with \; as a literal"#)
  func splits() {
    #expect(splitCommands(#" /a ; /b x;; hi\; there "#) == ["/a", "/b x", "hi; there"])
  }

  @Test("validates names and bodies")
  func validates() {
    #expect(aliasError(name: "ns", body: "/msg NickServ $*") == nil)
    #expect(aliasError(name: "", body: "/x")?.contains("name") == true)
    #expect(aliasError(name: "n s", body: "/x")?.contains("letters") == true)
    #expect(aliasError(name: "ns", body: " ; ")?.contains("command") == true)
  }
}

@MainActor @Suite("draft/multiline helpers")
struct MultilineHelperTests {
  @Test("parses the cap value")
  func parsesCap() {
    #expect(parseMultilineCap("max-bytes=4096,max-lines=24") == MultilineLimits(maxBytes: 4096, maxLines: 24))
    #expect(parseMultilineCap("max-bytes=4096") == MultilineLimits(maxBytes: 4096, maxLines: Int.max))
    #expect(parseMultilineCap("max-lines=5") == nil)
    #expect(parseMultilineCap(nil) == nil)
  }

  @Test("splits long lines into concatenated parts without losing characters")
  func splitsLong() throws {
    let long = String(String(repeating: "word ", count: 40).dropLast())
    let batch = try #require(
      multilineBatches("\(long)\nshort", limits: MultilineLimits(maxBytes: 4096, maxLines: 10), lineBytes: 60).first)
    #expect(batch.count > 3)
    #expect(batch.allSatisfy { $0.text.utf8.count <= 60 })
    #expect(batch.filter { !$0.concat }.count == 2)
    #expect(joinParts(batch) == "\(long)\nshort")
  }

  @Test("respects max-lines and max-bytes by starting new batches")
  func respectsLimits() {
    let lines = multilineBatches("a\nb\nc\nd", limits: MultilineLimits(maxBytes: 4096, maxLines: 3), lineBytes: 400)
    #expect(lines.map(joinParts) == ["a\nb\nc", "d"])
    let bytes = multilineBatches("aaaa\nbbbb\ncccc", limits: MultilineLimits(maxBytes: 10, maxLines: 99), lineBytes: 400)
    #expect(bytes.map(joinParts) == ["aaaa\nbbbb", "cccc"])
    #expect(multilineBatches("\n\n", limits: MultilineLimits(maxBytes: 10, maxLines: 99), lineBytes: 400).isEmpty)
  }
}

@MainActor @Suite("onboarding")
struct OnboardingTests {
  @Test("nickError accepts ordinary and special-character nicks")
  func accepts() {
    for n in ["ember", "Ember_", "[away]", "a-b", "x^y", "`q", #"{z}|w\"#] { #expect(nickError(n) == nil, "\(n)") }
  }

  @Test("nickError rejects empty, spaced and badly-started nicks")
  func rejects() {
    #expect(nickError("")?.contains("Choose") == true)
    #expect(nickError("my nick")?.contains("spaces") == true)
    #expect(nickError("9lives")?.contains("digit") == true)
    #expect(nickError("-dash")?.contains("digit or a dash") == true)
  }

  @Test("nickError names the offending character")
  func namesChar() {
    #expect(nickError("ember!")?.contains("“!”") == true)
    #expect(nickError("émile")?.contains("“é”") == true)
  }

  @Test("nickError enforces the network NICKLEN")
  func nickLen() {
    #expect(nickError("abcdefghij", maxLen: 9, network: "EFnet")?.contains("9 characters or fewer on EFnet") == true)
    #expect(nickError("abcdefghi", maxLen: 9) == nil)
  }

  @Test("parseChannels adds # to bare names and drops duplicates")
  func channels() {
    #expect(parseChannels("#libera, rust  #Rust\n&local") == ["#libera", "#rust", "&local"])
    #expect(parseChannels("  ") == [])
  }

  private func title(_ raw: String, sasl: Bool = false) -> String {
    explainError(raw, host: "irc.example", port: 6697, tls: true, sasl: sasl).title
  }

  @Test("explainError explains common transport failures")
  func transport() {
    #expect(title("could not reach irc.example:6697: failed to lookup address information").contains("find irc.example"))
    #expect(title("could not reach irc.example:6697: Connection refused (os error 61)").contains("refused"))
    #expect(title("TLS handshake failed: invalid peer certificate: UnknownIssuer").contains("secure"))
    #expect(title("timed out connecting to irc.example:6697").contains("didn't answer"))
  }

  @Test("explainError tells SASL-required networks apart from bad credentials")
  func sasl() {
    #expect(title("Closing Link: (SASL access only)").contains("log in first"))
    #expect(title("SASL authentication failed", sasl: true).contains("not accepted"))
  }

  @Test("explainError falls back to a generic message")
  func generic() {
    #expect(title("something odd") == "Couldn't connect")
  }

  @Test("presets have TLS defaults on the standard port")
  func presetsTLS() {
    for p in presets where p.kind == .network {
      #expect(p.host.contains("."))
      #expect(p.tls)
      #expect(p.port == 6697)
    }
  }
}

@MainActor @Suite("media helpers")
struct MediaTests {
  @Test("spots NSFW markers as words")
  func nsfw() {
    #expect(isNsfw("NSFW: https://x.example/a.png"))
    #expect(isNsfw("(nsfl) look"))
    #expect(isNsfw("\u{02}nsfw\u{02} pic"))
    #expect(!isNsfw("https://x.example/unsfwx.png"))
    #expect(!isNsfw("safe for work"))
  }

  private func preview(kind: String, mime: String, size: Int?, width: Int? = nil, height: Int? = nil) -> Preview {
    Preview(
      url: "", kind: kind, title: nil, description: nil, site: nil, image: nil, width: width, height: height,
      animated: false, mime: mime, size: size)
  }

  @Test("describes files")
  func describes() {
    #expect(formatBytes(512) == "512 B")
    #expect(formatBytes(1536) == "1.5 KB")
    #expect(formatBytes(12 * 1024 * 1024) == "12 MB")
    #expect(mediaLabel(preview(kind: "media", mime: "video/mp4", size: 3 * 1024 * 1024)) == "Video · MP4 · 3.0 MB")
    #expect(mediaLabel(preview(kind: "media", mime: "audio/mpeg", size: nil)) == "Audio · MP3")
    #expect(
      mediaLabel(preview(kind: "image", mime: "image/png", size: 2048, width: 800, height: 600))
        == "Image · PNG · 800×600 · 2.0 KB")
    #expect(fileName("https://up.example/uploads/abc/My%20clip.webm?x=1") == "My clip.webm")
    #expect(fileName("https://up.example/") == "up.example")
  }
}
