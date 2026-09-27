// runInput: slash commands and plain text from the composer (commands.test.ts),
// and running user aliases (aliases.test.ts; expansion is in PureTests.swift).

import Foundation
import Testing

@testable import Hearth

extension Harness {
  /// Connect, join #a with a NAMES list and clear `sent` (commands.test.ts `inChannel`).
  func joined(caps: [String] = [], isupport: [String: String] = [:]) async -> Buffer {
    await connect(caps: caps, isupport: isupport)
    await feed(":me!u@h JOIN #a", ":srv 353 me = #a :@me ann", ":srv 366 me #a :End")
    sent = []
    return buf("#a")
  }
}

@MainActor @Suite("channel commands")
struct ChannelCommandTests {
  @Test("/join adds a missing prefix, passes keys and opens the channel on JOIN")
  func join() async {
    let h = Harness()
    await h.connect()
    _ = await h.run("/join hearth", h.conn.server)
    _ = await h.run("/j #locked sekrit", h.conn.server)
    #expect(h.sent == ["JOIN #hearth", "JOIN #locked sekrit"])
    await h.feed(":me!u@h JOIN #locked")
    #expect(h.buf("#locked").joinKey == "sekrit")
    #expect(h.app.active === h.buf("#locked"))
  }

  @Test("/join of a channel we are in just switches to it")
  func joinSwitches() async {
    let h = Harness()
    let a = await h.joined()
    h.app.open(h.conn.server)
    _ = await h.run("/join #A", h.conn.server)
    #expect(h.sent == [])
    #expect(h.app.active === a)
  }

  @Test("/part with and without an explicit channel")
  func part() async {
    let h = Harness()
    _ = await h.joined()
    _ = await h.run("/part see you all")
    _ = await h.run("/part #other bye now")
    _ = await h.run("/leave")
    #expect(h.sent == ["PART #a :see you all", "PART #other :bye now", "PART #a"])
  }

  @Test("/topic shows or sets the topic")
  func topic() async {
    let h = Harness()
    _ = await h.joined(caps: ["labeled-response"])
    _ = await h.run("/topic")
    _ = await h.run("/topic new: topic here")
    #expect(h.sent.first?.wholeMatch(of: /@label=\S+ TOPIC #a/) != nil)
    #expect(h.sent.count > 1 && h.sent[1] == "TOPIC #a :new: topic here")
  }

  @Test("/kick, /ban, /unban, /op and friends target the current channel")
  func kickBanOp() async {
    let h = Harness()
    _ = await h.joined()
    for input in [
      "/kick ann", "/kick ann stop that now", "/ban ann", "/ban *!*@bad.host", "/ban troll!*", "/unban *!*@bad.host",
      "/op ann bob", "/devoice ann",
    ] {
      _ = await h.run(input)
    }
    #expect(
      h.sent == [
        "KICK #a ann",
        "KICK #a ann :stop that now",
        "MODE #a +b ann!*@*",
        "MODE #a +b *!*@bad.host",
        "MODE #a +b troll!*",
        "MODE #a -b *!*@bad.host",
        "MODE #a +oo ann bob",
        "MODE #a -v ann",
      ])
  }

  @Test("/mode on the channel, another channel or ourselves")
  func mode() async {
    let h = Harness()
    _ = await h.joined()
    for input in ["/mode +m", "/mode", "/mode #b +i", "/mode me +i"] { _ = await h.run(input) }
    #expect(h.sent == ["MODE #a +m", "MODE #a", "MODE #b +i", "MODE me +i"])
  }

  @Test("channel-only commands fail with usage in a query")
  func channelOnly() async {
    let h = Harness()
    await h.connect()
    let q = h.conn.ensure("ann", .query)
    #expect(await h.run("/kick bob", q) == false)
    #expect(await h.run("/topic hi", q) == false)
    #expect(await h.run("/me", q) == false)
    #expect(h.sent == [])
    #expect(
      h.toasts == [
        "Usage: This command needs a channel", "Usage: This command needs a channel", "Usage: /me <action>",
      ])
  }
}

@MainActor @Suite("messaging commands")
struct MessagingCommandTests {
  @Test("/msg sends to a nick and opens a quiet query")
  func msg() async {
    let h = Harness()
    _ = await h.joined()
    #expect(await h.run("/msg ann hello there"))
    #expect(h.sent == ["PRIVMSG ann :hello there"])
    #expect(h.lines("ann").map(\.text) == ["hello there"])
    #expect(h.lines("ann").map(\.isSelf) == [true])
    #expect(await h.run("/msg ann") == false)
    #expect(h.toasts.contains("Usage: /msg <target> <text>"))
  }

  @Test("/msg with echo-message waits for the echo")
  func msgEcho() async {
    let h = Harness()
    _ = await h.joined(caps: ["echo-message"])
    _ = await h.run("/m ann hi")
    #expect(h.lines("ann").isEmpty)
  }

  @Test("/me sends a CTCP ACTION")
  func me() async {
    let h = Harness()
    let a = await h.joined()
    _ = await h.run("/me waves")
    #expect(h.sent == ["PRIVMSG #a :\u{01}ACTION waves\u{01}"])
    let l = a.messages.last
    #expect(l?.type == .action && l?.text == "waves" && l?.isSelf == true)
  }

  @Test("/notice and /ctcp")
  func noticeCtcp() async {
    let h = Harness()
    _ = await h.joined()
    for input in ["/notice ann heads up", "/ctcp ann version", "/ctcp ann ping 123"] { _ = await h.run(input) }
    #expect(
      h.sent == ["NOTICE ann :heads up", "PRIVMSG ann :\u{01}VERSION\u{01}", "PRIVMSG ann :\u{01}PING 123\u{01}"])
  }

  @Test("/query opens a conversation and optionally sends")
  func query() async {
    let h = Harness()
    _ = await h.joined()
    _ = await h.run("/q bob hey")
    #expect(h.app.active === h.buf("bob"))
    #expect(h.sent == ["PRIVMSG bob :hey"])
  }
}

@MainActor @Suite("client-side commands")
struct ClientCommandTests {
  @Test("/notify sets the buffer level; /mute and /unmute are shortcuts")
  func notify() async {
    let h = Harness()
    let a = await h.joined()
    _ = await h.run("/notify ALL")
    #expect(h.app.settings.bufferNotify[a.prefKey] == .all)
    _ = await h.run("/mute")
    #expect(h.app.settings.bufferNotify[a.prefKey] == NotifyLevel.none)
    _ = await h.run("/unmute")
    #expect(h.app.settings.bufferNotify[a.prefKey] == nil)
    #expect(await h.run("/notify loud") == false)
    #expect(await h.run("/notify all", h.conn.server) == false)
  }

  @Test("/ignore and /unignore manage masks")
  func ignore() async {
    let h = Harness()
    let a = await h.joined()
    _ = await h.run("/ignore troll")
    _ = await h.run("/ignore *@spam.host")
    _ = await h.run("/ignore troll")
    #expect(h.app.settings.ignores == ["troll!*@*", "*!*@spam.host"])
    await h.feed(":Troll!t@x PRIVMSG #a :hi")
    #expect(a.messages.isEmpty)
    _ = await h.run("/ignore")
    #expect(a.messages.last?.text == "Ignoring: troll!*@*, *!*@spam.host")
    _ = await h.run("/unignore troll")
    _ = await h.run("/unignore nobody")
    #expect(h.app.settings.ignores == ["*!*@spam.host"])
    #expect(Array(h.toasts.suffix(2)) == ["No longer ignoring troll", "nobody was not ignored"])
  }

  @Test("/clear empties the buffer")
  func clear() async {
    let h = Harness()
    let a = await h.joined()
    await h.feed(":ann!a@h PRIVMSG #a :one")
    _ = await h.run("/clear")
    #expect(a.messages.isEmpty)
    #expect(h.sent == [])
  }

  @Test("/whois in a query defaults to the peer")
  func whois() async {
    let h = Harness()
    await h.connect()
    let q = h.conn.ensure("ann", .query)
    await settle()
    h.sent = []
    _ = await h.run("/whois", q)
    _ = await h.run("/w bob", q)
    #expect(h.sent == ["WHOIS ann ann", "WHOIS bob bob"])
  }
}

@MainActor @Suite("raw and unknown commands")
struct RawCommandTests {
  @Test("sends unknown commands verbatim with the name upper-cased")
  func unknown() async {
    let h = Harness()
    _ = await h.joined()
    #expect(await h.run("/cs op #a me"))
    _ = await h.run("/knock #secret please let me in")
    _ = await h.run("/lusers")
    #expect(h.sent == ["CS op #a me", "KNOCK #secret please let me in", "LUSERS"])
  }

  @Test("/raw and /quote pass the line through")
  func raw() async {
    let h = Harness()
    await h.connect()
    _ = await h.run("/raw PRIVMSG ann :hi  there", h.conn.server)
    _ = await h.run("/quote WHO #a", h.conn.server)
    #expect(h.sent == ["PRIVMSG ann :hi  there", "WHO #a"])
  }

  @Test("a double slash sends a literal slash")
  func doubleSlash() async {
    let h = Harness()
    _ = await h.joined()
    _ = await h.run("//shrug")
    #expect(h.sent == ["PRIVMSG #a :/shrug"])
  }
}

@MainActor @Suite("plain text")
struct PlainTextTests {
  @Test("ignores blank input and refuses to chat in the server buffer")
  func blank() async {
    let h = Harness()
    await h.connect()
    #expect(await h.run("   ", h.conn.server) == false)
    #expect(await h.run("hello", h.conn.server) == false)
    #expect(h.sent == [])
    #expect(h.toasts.first?.contains("server buffer") == true)
  }

  @Test("sends each non-empty line separately, replying only with the first")
  func lines() async {
    let h = Harness()
    _ = await h.joined(caps: ["message-tags"])
    _ = await h.run("one\n\n  \ntwo\nthree", h.buf("#a"), replyTo: "m1")
    #expect(h.sent == ["@+draft/reply=m1 PRIVMSG #a :one", "PRIVMSG #a :two", "PRIVMSG #a :three"])
  }

  @Test("splits long messages on word boundaries within the line limit")
  func splitsLong() async {
    let h = Harness()
    let a = await h.joined()
    let words = (0..<200).map { "word\($0)" }
    let text = words.joined(separator: " ")
    _ = await h.run(text)
    #expect(h.sent.count > 1)
    let parts = h.sent.map { String($0.dropFirst("PRIVMSG #a :".count)) }
    let prefix = ":me!\(String(repeating: "u", count: 10))@\(String(repeating: "h", count: 63)) PRIVMSG #a :"
    for p in parts {
      #expect((prefix + p).utf8.count <= 510)
      #expect(!p.hasPrefix(" ") && !p.hasSuffix(" "))
    }
    // Nothing lost, nothing broken mid-word.
    #expect(parts.joined(separator: " ") == text)
    #expect(parts.flatMap { $0.split(separator: " ").map(String.init) } == words)
    #expect(a.messages.map(\.text) == parts)
  }

  @Test("splits multi-byte text without breaking characters")
  func splitsMultiByte() async {
    let h = Harness()
    _ = await h.joined()
    let text = String(repeating: "é", count: 500)
    _ = await h.run(text)
    let parts = h.sent.map { String($0.dropFirst("PRIVMSG #a :".count)) }
    #expect(parts.count > 1)
    #expect(parts.joined() == text)
    for p in parts { #expect(p.utf8.count <= 400) }
  }
}

@MainActor @Suite("running aliases")
struct RunningAliasTests {
  private func setup() async -> Harness {
    let h = Harness()
    await h.connect(isupport: ["NETWORK": "Testnet"])
    await h.feed(":me!u@h JOIN #a")
    h.sent = []
    return h
  }

  @Test("runs each command with arguments filled in")
  func runs() async {
    let h = await setup()
    h.app.settings.aliases = ["ns": "/msg NickServ $*", "greet": "hello $1!; /me waves at $1"]
    #expect(await h.run("/ns identify hunter2"))
    #expect(await h.run("/GREET ann"))
    #expect(
      h.sent == [
        "PRIVMSG NickServ :identify hunter2", "PRIVMSG #a :hello ann!", "PRIVMSG #a :\u{01}ACTION waves at ann\u{01}",
      ])
  }

  @Test("keeps ; inside arguments intact")
  func semicolons() async {
    let h = await setup()
    h.app.settings.aliases = ["say": "/msg #a $*"]
    _ = await h.run("/say a;b")
    #expect(h.sent == ["PRIVMSG #a :a;b"])
  }

  @Test("can call other aliases")
  func chains() async {
    let h = await setup()
    h.app.settings.aliases = ["j": "/join #$1", "both": "/j one; /j two"]
    _ = await h.run("/both")
    #expect(h.sent == ["JOIN #one", "JOIN #two"])
  }

  @Test("overrides a built-in but can still call it from inside")
  func overrides() async {
    let h = await setup()
    h.app.settings.aliases = ["join": "/join #$1; /msg ChanServ op #$1"]
    _ = await h.run("/join rust")
    #expect(h.sent == ["JOIN #rust", "PRIVMSG ChanServ :op #rust"])
  }

  @Test("stops recursion and runaway fan-out")
  func runaway() async {
    let h = await setup()
    h.app.settings.aliases = ["a": "/b", "b": "/a"]
    #expect(await h.run("/a") == false)
    #expect(h.toasts.contains("Alias /a calls itself"))
    h.app.toasts = []
    h.app.settings.aliases = [
      "x": "/y $1; /y $1; /y $1; /y $1", "y": "/z $1; /z $1; /z $1; /z $1",
      "z": "/raw PING $1; /raw PING $1; /raw PING $1; /raw PING $1",
    ]
    _ = await h.run("/x 1")
    #expect(h.sent.count <= 50)
    #expect(h.toasts.contains("Alias /x runs too many commands"))
    #expect(h.sent.allSatisfy { $0 == "PING 1" })
  }

  @Test("lists aliases ahead of the built-ins they replace")
  func lists() {
    let infos = commandInfos(["join": "/join #$1", "ns": "/msg NickServ $*"])
    #expect(infos.prefix(2).map(\.usage) == ["/join", "/ns"])
    #expect(infos.filter { $0.name == "join" }.count == 1)
    #expect(infos.contains { $0.name == "part" })
  }
}
