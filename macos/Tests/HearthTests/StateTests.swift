// AppModel, Conn and Buffer behaviour (state.test.ts) and per-network
// highlight rules (highlights.test.ts).

import Foundation
import Testing

@testable import Hearth

/// The `buffers` argument of the last save_buffers call.
@MainActor
private func lastSaved(_ b: FakeBackend) throws -> (conn: String, buffers: [SavedBuffer])? {
  guard let args = b.calls("save_buffers").last else { return nil }
  let data = try JSONSerialization.data(withJSONObject: args["buffers"] ?? [])
  return (args["conn"] as? String ?? "", try JSONDecoder().decode([SavedBuffer].self, from: data))
}

@MainActor @Suite("messages")
struct MessageTests {
  @Test("files channel and private messages and flags mentions")
  func files() async {
    let h = Harness()
    await h.connect()
    await h.feed(":me!u@h JOIN #hearth", ":ann!a@h PRIVMSG #hearth :hello there", ":bob!b@h PRIVMSG #hearth :me: ping")
    await h.feed(":cat!c@h PRIVMSG me :psst")
    let chan = h.buf("#hearth")
    #expect(chan.messages.map(\.nick) == ["ann", "bob"])
    #expect(chan.messages.map(\.highlight) == [false, true])
    #expect(h.buf("cat").kind == .query)
    #expect(h.texts("cat") == ["psst"])
  }

  @Test("drops messages from ignored masks")
  func ignored() async {
    let h = Harness()
    await h.connect()
    h.app.settings.ignores = ["spammer", "*!*@bad.host"]
    await h.feed(
      ":me!u@h JOIN #a", ":Spammer!x@y PRIVMSG #a :buy", ":eve!e@bad.host PRIVMSG #a :hi", ":ann!a@ok PRIVMSG #a :fine")
    await h.feed("@+typing=active :spammer!x@y TAGMSG #a")
    #expect(h.texts("#a") == ["fine"])
    #expect(h.buf("#a").typing.isEmpty)
  }

  @Test("attaches reactions and replies")
  func reactionsAndReplies() async {
    let h = Harness()
    await h.connect(caps: ["message-tags"])
    await h.feed(":me!u@h JOIN #a", "@msgid=m1 :ann!a@h PRIVMSG #a :ship it?")
    await h.feed(
      "@+draft/react=👍;+draft/reply=m1 :bob!b@h TAGMSG #a", "@+draft/react=👍;+draft/reply=m1 :bob!b@h TAGMSG #a")
    await h.feed("@msgid=m2;+draft/reply=m1 :cat!c@h PRIVMSG #a :yes")
    let msgs = h.lines("#a")
    #expect(msgs[0].reactions == [Reaction(emoji: "👍", nicks: ["bob"])])
    #expect(msgs[1].replyTo == "m1")
  }

  @Test("sends replies and reactions with tags")
  func sendsTags() async {
    let h = Harness()
    await h.connect(caps: ["message-tags"])
    await h.feed(":me!u@h JOIN #a", "@msgid=m1 :ann!a@h PRIVMSG #a :hi")
    let buf = h.buf("#a")
    h.app.sendMessage(buf, "hello", kind: "PRIVMSG", action: false, replyTo: "m1")
    h.app.react(buf, buf.messages[0], "🎉")
    await settle()
    #expect(h.sent.contains("@+draft/reply=m1 PRIVMSG #a :hello"))
    #expect(h.sent.contains("@+draft/react=🎉;+draft/reply=m1 TAGMSG #a"))
    // Without echo-message the reaction shows up locally at once.
    #expect(buf.messages[0].reactions == [Reaction(emoji: "🎉", nicks: ["me"])])
  }
}

@MainActor @Suite("line cap")
struct LineCapTests {
  @Test("trims the oldest lines in a chunk once past the cap")
  func trims() async {
    let h = Harness()
    await h.connect()
    await h.feed(":me!u@h JOIN #big")
    let buf = h.buf("#big")
    let t = nowMs()
    buf.messages = (0..<20_000).map { Line(type: .privmsg, time: t + $0, nick: "a", text: String($0)) }
    buf.history = .done
    buf.push(Line(type: .privmsg, time: t + 20_000, nick: "a", text: "last"))
    #expect(buf.messages.count == 18_000)
    #expect(buf.messages.first?.text == "2001")
    #expect(buf.messages.last?.text == "last")
    // Older lines can be fetched again.
    #expect(buf.history == .idle)
  }
}

@MainActor @Suite("routing")
struct RoutingTests {
  @Test("sends labeled replies to the buffer that asked")
  func labeledWhois() async throws {
    let h = Harness()
    await h.connect(caps: ["labeled-response"])
    await h.feed(":me!u@h JOIN #a", ":me!u@h JOIN #b")
    let a = h.buf("#a")
    h.app.open(h.buf("#b"))
    h.conn.sendFrom(a, "WHOIS ann")
    await settle()
    let label = try #require(h.sent.last?.wholeMatch(of: #/@label=(\S+) WHOIS ann/#)?.output.1)
    await h.feed("@label=\(label) :srv 311 me ann a h * :Ann", "@label=\(label) :srv 318 me ann :End")
    #expect(a.messages.last?.type == .whois)
    #expect(!h.buf("#b").messages.contains { $0.type == .whois })
  }

  @Test("collects LIST replies into the channel browser")
  func list() async {
    let h = Harness()
    await h.connect()
    await h.feed(
      ":srv 321 me Channel :Users Name", ":srv 322 me #a 12 :topic a", ":srv 322 me #b 3 :topic b", ":srv 323 me :End")
    #expect(h.conn.channelList?.loading == false)
    #expect(h.conn.channelList?.items.map(\.name) == ["#a", "#b"])
    #expect(h.conn.channelList?.items.map(\.users) == [12, 3])
    #expect(h.conn.server.messages.isEmpty)
  }

  @Test("tracks MONITOR presence and announces watched nicks")
  func monitor() async {
    let h = Harness()
    await h.connect(isupport: ["MONITOR": "100"])
    h.app.settings.watch = ["c|": ["Ann"]]
    await h.feed(":srv 730 me :ann!a@h,bob!b@h")
    #expect(h.conn.online["ann"] == true)
    #expect(h.conn.server.messages.isEmpty)
    await h.feed(":srv 731 me :ann")
    #expect(h.conn.online["ann"] == false)
    #expect(h.conn.server.messages.last?.text == "ann is offline")
  }
}

@MainActor @Suite("buffers")
struct BufferTests {
  @Test("restores saved buffers and rejoins with keys")
  func restores() async {
    let h = Harness()
    h.backend.respondOnce(
      "load_buffers",
      with: [
        ["name": "#open", "kind": "channel", "key": ""],
        ["name": "#locked", "kind": "channel", "key": "sekrit"],
        ["name": "ann", "kind": "query", "key": ""],
      ])
    await h.connect()
    await sleep(ms: 2000)
    #expect(h.buf("ann").kind == .query)
    #expect(h.sent.contains("JOIN #locked,#open sekrit"))
  }

  @Test("saves the buffer list and forgets parted channels")
  func saves() async throws {
    let h = Harness()
    await h.connect()
    h.app.join(h.conn, "#keyed", key: "k1")
    await h.feed(":me!u@h JOIN #keyed", ":me!u@h JOIN #gone", ":me!u@h PART #gone")
    await sleep(ms: 600)
    let saved = try #require(try lastSaved(h.backend))
    #expect(saved.conn == "c")
    #expect(saved.buffers == [SavedBuffer(name: "#keyed", kind: "channel", key: "k1")])
  }

  @Test("falls back to the local log without chathistory")
  func localLog() async throws {
    let h = Harness()
    await h.connect()
    await h.feed(":me!u@h JOIN #a", "@time=2026-01-01T00:00:10.000Z :ann!a@h PRIVMSG #a :live")
    let buf = h.buf("#a")
    h.backend.respondOnce(
      "local_history",
      with: [
        json(
          IrcMessage(
            tags: ["time": "2026-01-01T00:00:01.000Z"], source: Source(nick: "bob", user: "b", host: "h"),
            command: "PRIVMSG", params: ["#a", "old"]))
      ])
    buf.history = .idle
    await h.app.fetchHistory(buf, .before)
    let args = try #require(h.backend.calls("local_history").last)
    #expect(args["conn"] as? String == "c")
    #expect(args["target"] as? String == "#a")
    #expect(args["before"] as? Int == ms("2026-01-01T00:00:10.000Z"))
    #expect(args["after"] is NSNull)
    #expect(args["limit"] as? Int == 100)
    #expect(h.texts("#a") == ["old", "live"])
    #expect(buf.history == .done)
  }

  @Test("dedupes history lines that match live ones closely in time")
  func dedupes() async {
    let h = Harness()
    await h.connect()
    await h.feed(":me!u@h JOIN #a")
    let buf = h.buf("#a")
    h.app.sendMessage(buf, "mine")
    let l = buf.messages[0]
    // A different line (new id) with the same content, logged 5 ms earlier.
    buf.merge([Line(type: l.type, time: l.time - 5, nick: l.nick, text: l.text, isSelf: l.isSelf)])
    #expect(buf.messages.count == 1)
  }
}

@MainActor @Suite("notification levels")
struct NotificationLevelTests {
  @Test("mutes and escalates per buffer")
  func mutesAndEscalates() async {
    let h = Harness()
    await h.connect()
    await h.feed(":me!u@h JOIN #a", ":ann!a@h PRIVMSG #a :chatter", ":ann!a@h PRIVMSG #a :me: look")
    let buf = h.buf("#a")
    buf.readMarker = 0
    #expect(buf.highlights == 1)
    h.app.settings.bufferNotify = [buf.prefKey: .all]
    #expect(buf.highlights == 2)
    h.app.settings.bufferNotify = [buf.prefKey: NotifyLevel.none]
    #expect(buf.highlights == 0)
    h.app.focused = false
    h.notified = []
    await h.feed(":ann!a@h PRIVMSG #a :me: again")
    #expect(h.notified.isEmpty)
    h.app.settings.bufferNotify = [buf.prefKey: .all]
    await h.feed(":ann!a@h PRIVMSG #a :anything")
    #expect(h.notified.count == 1)
  }
}

@MainActor @Suite("network highlight rules")
struct NetworkHighlightTests {
  private func setup() async -> Harness {
    let h = Harness()
    h.app.focused = false
    await h.connect()
    await h.feed(":me!u@h JOIN #a")
    return h
  }

  private func highlights(_ h: Harness) -> [String] {
    h.lines("#a").map { "\($0.text)=\($0.highlight)" }
  }

  @Test("adds words and regexes for this server to the nick and global words")
  func addsWords() async {
    let h = await setup()
    h.app.settings.highlightWords = ["hearth"]
    h.app.settings.networkHighlights = [
      "c": NetworkHighlights(words: ["deploy", #"/\bci (failed|broke)\b/i"#], never: [])
    ]
    await h.feed(
      ":ann!a@h PRIVMSG #a :me: ping",
      ":ann!a@h PRIVMSG #a :hearth is nice",
      ":ann!a@h PRIVMSG #a :deploy done",
      ":ann!a@h PRIVMSG #a :CI broke again",
      ":ann!a@h PRIVMSG #a :redeploying"
    )
    #expect(
      highlights(h) == [
        "me: ping=true", "hearth is nice=true", "deploy done=true", "CI broke again=true", "redeploying=false",
      ])
    #expect(h.notified.count == 4)
  }

  @Test("doesn't apply another server's rules")
  func otherServer() async {
    let h = await setup()
    h.app.settings.networkHighlights = ["other": NetworkHighlights(words: ["deploy"], never: [])]
    await h.feed(":ann!a@h PRIVMSG #a :deploy done")
    #expect(highlights(h) == ["deploy done=false"])
  }

  @Test("never highlights listed nicks or masks, even for our nick")
  func never() async {
    let h = await setup()
    h.app.settings.networkHighlights = ["c": NetworkHighlights(words: [], never: ["GitBot", "*!*@relay.example"])]
    await h.feed(
      ":gitbot!g@h PRIVMSG #a :me pushed 3 commits",
      ":bridge!b@relay.example PRIVMSG #a :<x> me: hi",
      ":ann!a@h PRIVMSG #a :me: hi"
    )
    #expect(highlights(h) == ["me pushed 3 commits=false", "<x> me: hi=false", "me: hi=true"])
    #expect(h.notified.count == 1)
  }

  @Test("applies to replayed history too")
  func history() async {
    let h = await setup()
    h.app.settings.networkHighlights = ["c": NetworkHighlights(words: ["deploy"], never: ["gitbot"])]
    let line = { (raw: String) in h.app.historyLine(h.conn, msg(raw))?.highlight }
    #expect(line(":ann!a@h PRIVMSG #a :deploy now") == true)
    #expect(line(":GitBot!g@h PRIVMSG #a :deploy now") == false)
  }
}
