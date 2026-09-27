// Behavioural tests for Incoming.swift (incoming.test.ts): raw IRC lines in, state out.

import Foundation
import Testing

@testable import Hearth

private let X1 = "\u{01}"

@MainActor @Suite("NICK")
struct NickTests {
  @Test("renames another user in every shared channel and their query")
  func renamesEverywhere() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "@me ann bob")
    await h.joinWith("#b", "me +ann")
    await h.joinWith("#c", "me bob")
    await h.feed(":ann!a@h PRIVMSG me :hi")
    await h.feed(":ann!a@h NICK anna")

    #expect(h.nicks("#a") == ["@me", "anna", "bob"])
    #expect(h.nicks("#b") == ["+anna", "me"])
    #expect(h.lines("#a").map(\.type) == [.nick])
    #expect(h.lines("#a")[0].nick == "ann")
    #expect(h.lines("#a")[0].target == "anna")
    #expect(h.lines("#b").map(\.type) == [.nick])
    #expect(h.lines("#c").isEmpty)

    // The private query follows the rename and is filed under the new name.
    #expect(h.conn.buffer("ann") == nil)
    let q = h.buf("anna")
    #expect(q.name == "anna")
    #expect(q.kind == .query)
    #expect(q.messages.map(\.type) == [.privmsg, .nick])
    await h.feed(":anna!a@h PRIVMSG me :still me")
    #expect(h.texts("anna") == ["hi", "", "still me"])
  }

  @Test("doesn't merge a query into an existing one for the new nick")
  func noMerge() async {
    let h = Harness()
    await h.connect()
    await h.feed(":ann!a@h PRIVMSG me :one", ":anna!a@h PRIVMSG me :two")
    await h.feed(":ann!a@h NICK anna")
    #expect(h.buf("ann").name == "ann")
    #expect(h.texts("anna") == ["two"])
  }

  @Test("updates the query name on a case-only nick change")
  func caseOnly() async {
    let h = Harness()
    await h.connect()
    await h.feed(":ann!a@h PRIVMSG me :hi", ":ann!a@h NICK Ann")
    #expect(h.buf("ann").name == "Ann")
    #expect(h.buf("Ann").member("ann")?.nick == "Ann")
  }

  @Test("tracks our own nick change everywhere")
  func ownNick() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "@me ann")
    await h.feed(":ann!a@h PRIVMSG me :hi")
    await h.feed(":me!u@h NICK me2")
    #expect(h.conn.nick == "me2")
    #expect(h.nicks("#a") == ["@me2", "ann"])
    let last = h.lines("#a").last
    #expect(last?.type == .nick && last?.isSelf == true && last?.target == "me2")
    // Queries and the server buffer hear about it too.
    #expect(h.lines("ann").map(\.type) == [.privmsg, .nick])
    #expect(h.conn.server.messages.last?.type == .nick && h.conn.server.messages.last?.target == "me2")
    // Mentions now use the new nick.
    await h.feed(":ann!a@h PRIVMSG #a :me2: yo", ":ann!a@h PRIVMSG #a :me: old")
    #expect(h.lines("#a").suffix(2).map(\.highlight) == [true, false])
  }
}

@MainActor @Suite("KICK, PART, QUIT")
struct KickPartQuitTests {
  @Test("kicking someone else removes only them")
  func kickOther() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "@me ann bob")
    await h.feed(":me!u@h KICK #a bob :bye")
    #expect(h.nicks("#a") == ["@me", "ann"])
    #expect(h.buf("#a").joined)
    let l = h.lines("#a").last!
    #expect(l.type == .kick && l.nick == "me" && l.target == "bob" && l.text == "bye" && !l.highlight)
  }

  @Test("being kicked leaves the channel and highlights")
  func kickedSelf() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me @ann")
    await h.feed(":ann!a@h KICK #a me :out")
    #expect(!h.buf("#a").joined)
    #expect(h.buf("#a").members.isEmpty)
    let l = h.lines("#a").last!
    #expect(l.type == .kick && l.target == "me" && l.text == "out" && l.highlight)
  }

  @Test("PART removes the member; our own PART clears the list")
  func part() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me ann bob")
    await h.feed(":ann!a@h PART #a :later")
    #expect(h.nicks("#a") == ["bob", "me"])
    let l = h.lines("#a").last!
    #expect(l.type == .part && l.nick == "ann" && l.text == "later")
    await h.feed(":me!u@h PART #a")
    #expect(!h.buf("#a").joined)
    #expect(h.buf("#a").members.isEmpty)
  }

  @Test("QUIT only shows in buffers shared with the user")
  func quitShared() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me ann")
    await h.joinWith("#b", "me ann")
    await h.joinWith("#c", "me bob")
    await h.feed(":ann!a@h PRIVMSG me :hi")
    await h.feed("@+typing=active :ann!a@h TAGMSG #a")
    await h.feed(":ann!a@h QUIT :Quit: gone")
    let l = h.lines("#a").last!
    #expect(l.type == .quit && l.nick == "ann" && l.text == "Quit: gone")
    #expect(h.lines("#b").map(\.type) == [.quit])
    #expect(h.lines("#c").isEmpty)
    #expect(h.lines("ann").map(\.type) == [.privmsg, .quit])
    #expect(h.buf("#a").member("ann") == nil)
    #expect(h.buf("#a").typing["ann"] == nil)
  }

  @Test("QUIT reaches a query even when the member list is empty")
  func quitQuery() async {
    let h = Harness()
    await h.connect()
    h.conn.ensure("ann", .query)
    await h.feed(":ann!a@h QUIT :bye")
    #expect(h.lines("ann").map(\.type) == [.quit])
  }
}

@MainActor @Suite("MODE")
struct ModeTests {
  @Test("adds and removes prefixes with multi-prefix, keeping rank order")
  func prefixes() async {
    let h = Harness()
    await h.connect(caps: ["multi-prefix"], isupport: ["PREFIX": "(qaohv)~&@%+"])
    await h.joinWith("#a", "me ann +bob")
    await h.feed(":srv MODE #a +v ann", ":srv MODE #a +o ann")
    #expect(h.buf("#a").member("ann")?.prefix == "@+")
    await h.feed(":srv MODE #a +q ann")
    #expect(h.buf("#a").member("ann")?.prefix == "~@+")
    await h.feed(":srv MODE #a -o ann")
    #expect(h.buf("#a").member("ann")?.prefix == "~+")
    await h.feed(":srv MODE #a +vv ann bob")
    #expect(h.buf("#a").member("bob")?.prefix == "+")
    #expect(h.buf("#a").member("ann")?.prefix == "~+")
  }

  @Test("applies several modes with mixed signs and arguments in one line")
  func mixed() async {
    let h = Harness()
    await h.connect(isupport: ["CHANMODES": "beI,k,l,imnpst"])
    await h.joinWith("#a", "me @ann bob +cat")
    await h.feed(":ann!a@h MODE #a +ov-v+lk-o bob bob cat 50 sekrit ann")
    // bob gets both +o and +v; cat loses voice; ann deops herself.
    #expect(h.buf("#a").member("bob")?.prefix == "@+")
    #expect(h.buf("#a").member("cat")?.prefix == "")
    #expect(h.buf("#a").member("ann")?.prefix == "")
    #expect(h.buf("#a").joinKey == "sekrit")
    let l = h.lines("#a").last!
    #expect(l.type == .mode && l.nick == "ann" && l.text == "+ov-v+lk-o bob bob cat 50 sekrit ann")
    #expect(h.nicks("#a") == ["@+bob", "ann", "cat", "me"])
  }

  @Test("doesn't consume an argument for -l, but does for bans")
  func minusL() async {
    let h = Harness()
    await h.connect(isupport: ["CHANMODES": "beI,k,l,imnpst"])
    await h.joinWith("#a", "me ann")
    await h.feed(":srv MODE #a -l+b+v *!*@spam ann")
    #expect(h.buf("#a").member("ann")?.prefix == "+")
  }

  @Test("forgets the key on -k and ignores a hidden key")
  func key() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "@me")
    await h.feed(":srv MODE #a +k real")
    #expect(h.buf("#a").joinKey == "real")
    await h.feed(":srv MODE #a +k *")
    #expect(h.buf("#a").joinKey == "real")
    await h.feed(":srv MODE #a -k *")
    #expect(h.buf("#a").joinKey == "")
  }

  @Test("records channel modes from 324 and user modes on us")
  func rpl324() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "@me")
    await h.feed(":srv 324 me #a +ntkl hunter2 10")
    #expect(h.buf("#a").modes == "+ntkl hunter2 10")
    #expect(h.buf("#a").joinKey == "hunter2")
    await h.feed(":me MODE me :+iw")
    let l = h.conn.server.messages.last
    #expect(l?.type == .mode && l?.text == "+iw")
  }

  @Test("parses multi-prefix NAMES and ranks members")
  func multiPrefixNames() async {
    let h = Harness()
    await h.connect(caps: ["multi-prefix"], isupport: ["PREFIX": "(qaohv)~&@%+"])
    await h.joinWith("#a", "+zed @+ann ~@bob %cat me")
    #expect(h.buf("#a").member("ann")?.prefix == "@+")
    #expect(h.nicks("#a") == ["~@bob", "@+ann", "%cat", "+zed", "me"])
  }
}

@MainActor @Suite("IRCv3 membership updates")
struct MembershipTests {
  @Test("extended-join records account and realname")
  func extendedJoin() async {
    let h = Harness()
    await h.connect(caps: ["extended-join"])
    await h.joinWith("#a", "me")
    await h.feed(":ann!a@host.example JOIN #a annacct :Ann Example", ":bob!b@h JOIN #a * :Bob")
    let ann = h.buf("#a").member("ann")
    #expect(ann?.account == "annacct" && ann?.realname == "Ann Example" && ann?.user == "a" && ann?.host == "host.example")
    #expect(h.buf("#a").member("bob")?.account == nil)
    #expect(h.lines("#a").map(\.type) == [.join, .join])
    #expect(h.lines("#a").map(\.nick) == ["ann", "bob"])
    #expect(h.lines("#a").map { $0.data?["host"] } == ["a@host.example", "b@h"])
  }

  @Test("userhost-in-names fills user and host")
  func userhostInNames() async {
    let h = Harness()
    await h.connect(caps: ["userhost-in-names"])
    await h.joinWith("#a", "@me!u@h ann!a@ann.host")
    let ann = h.buf("#a").member("ann")
    #expect(ann?.nick == "ann" && ann?.user == "a" && ann?.host == "ann.host" && ann?.prefix == "")
    #expect(h.buf("#a").member("me")?.prefix == "@")
  }

  @Test("keeps away and account across a NAMES refresh")
  func namesRefresh() async {
    let h = Harness()
    await h.connect(caps: ["away-notify", "account-notify"])
    await h.joinWith("#a", "me ann")
    await h.feed(":ann!a@h AWAY :lunch", ":ann!a@h ACCOUNT annacct")
    await h.feed(":srv 353 me = #a :me @ann", ":srv 366 me #a :End")
    let ann = h.buf("#a").member("ann")
    #expect(ann?.prefix == "@" && ann?.away == true && ann?.account == "annacct")
  }

  @Test("chghost, setname, account-notify and away-notify patch every channel")
  func patchEveryChannel() async {
    let h = Harness()
    await h.connect(caps: ["chghost", "setname", "account-notify", "away-notify"])
    await h.joinWith("#a", "me ann")
    await h.joinWith("#b", "me ann")
    await h.feed(":ann!a@old CHGHOST newuser new.host", ":ann!newuser@new.host SETNAME :Ann Two")
    await h.feed(":ann!a@h ACCOUNT annacct", ":ann!a@h AWAY :brb")
    for c in ["#a", "#b"] {
      let m = h.buf(c).member("ann")
      #expect(m?.user == "newuser" && m?.host == "new.host" && m?.realname == "Ann Two")
      #expect(m?.account == "annacct" && m?.away == true)
    }
    await h.feed(":ann!a@h ACCOUNT *", ":ann!a@h AWAY")
    #expect(h.buf("#a").member("ann")?.account == nil)
    #expect(h.buf("#a").member("ann")?.away == false)
    // None of these produce chat lines.
    #expect(h.lines("#a").isEmpty)
  }

  @Test("tracks our own away state")
  func ownAway() async {
    let h = Harness()
    await h.connect()
    await h.feed(":srv 306 me :You have been marked as being away")
    #expect(h.conn.away)
    await h.feed(":srv 305 me :You are no longer marked as being away")
    #expect(!h.conn.away)
  }
}

@MainActor @Suite("TOPIC and NAMES")
struct TopicNamesTests {
  @Test("stores the topic from 332/333 without a line")
  func topicNumerics() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me")
    await h.feed(":srv 332 me #a :Welcome to #a", ":srv 333 me #a ann!a@h 1700000000")
    #expect(h.buf("#a").topic == "Welcome to #a")
    #expect(h.buf("#a").topicBy == "ann")
    #expect(h.buf("#a").topicTime == 1_700_000_000_000)
    #expect(h.lines("#a").isEmpty)
    await h.feed(":srv 331 me #a :No topic is set")
    #expect(h.buf("#a").topic == "")
  }

  @Test("a live TOPIC updates state and adds a line")
  func liveTopic() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me")
    await h.feed("@time=2026-01-02T03:04:05.000Z :bob!b@h TOPIC #a :new topic")
    #expect(h.buf("#a").topic == "new topic")
    #expect(h.buf("#a").topicBy == "bob")
    #expect(h.buf("#a").topicTime == ms("2026-01-02T03:04:05.000Z"))
    let l = h.lines("#a").last
    #expect(l?.type == .topic && l?.nick == "bob" && l?.text == "new topic")
  }

  @Test("accumulates NAMES across 353s and swaps on 366")
  func accumulates() async {
    let h = Harness()
    await h.connect()
    await h.feed(":me!u@h JOIN #a", ":srv 353 me = #a :me ann", ":srv 353 me = #a :@bob")
    // Nothing visible until the list is complete.
    #expect(h.buf("#a").members.isEmpty)
    await h.feed(":srv 366 me #a :End")
    #expect(h.nicks("#a") == ["@bob", "ann", "me"])
    // A refresh replaces the list rather than adding to it.
    await h.feed(":srv 353 me = #a :me", ":srv 366 me #a :End")
    #expect(h.nicks("#a") == ["me"])
  }

  @Test("ignores NAMES for channels we have no buffer for")
  func noBuffer() async {
    let h = Harness()
    await h.connect()
    await h.feed(":srv 353 me = #nope :ann bob", ":srv 366 me #nope :End")
    #expect(h.conn.buffer("#nope") == nil)
  }
}

@MainActor @Suite("echo-message, labels, redaction")
struct EchoLabelRedactionTests {
  @Test("shows our message once when the server echoes it")
  func echoOnce() async {
    let h = Harness()
    await h.connect(caps: ["echo-message", "message-tags"])
    await h.joinWith("#a", "me")
    h.app.sendMessage(h.buf("#a"), "hello")
    await settle()
    #expect(h.sent.contains("PRIVMSG #a :hello"))
    #expect(h.lines("#a").isEmpty)
    await h.feed("@msgid=e1 :me!u@h PRIVMSG #a :hello")
    #expect(h.lines("#a").map(\.text) == ["hello"])
    #expect(h.lines("#a")[0].isSelf)
    #expect(h.lines("#a")[0].msgid == "e1")
  }

  @Test("files echoed private messages under the recipient")
  func echoPrivate() async {
    let h = Harness()
    await h.connect(caps: ["echo-message"])
    await h.feed(":me!u@h PRIVMSG ann :psst")
    #expect(h.buf("ann").kind == .query)
    let l = h.lines("ann")[0]
    #expect(l.isSelf && l.text == "psst" && !l.highlight)
    #expect(h.notified.isEmpty)
  }

  @Test("reactions wait for the echo")
  func reactionsWait() async {
    let h = Harness()
    await h.connect(caps: ["echo-message", "message-tags"])
    await h.joinWith("#a", "me")
    await h.feed("@msgid=m1 :ann!a@h PRIVMSG #a :ship it")
    h.app.react(h.buf("#a"), h.lines("#a")[0], "🚀")
    await settle()
    #expect(h.lines("#a")[0].reactions.isEmpty)
    await h.feed("@+draft/react=🚀;+draft/reply=m1 :me!u@h TAGMSG #a")
    await h.feed("@+draft/react=🚀;+draft/reply=m1 :Me!u@h TAGMSG #a")
    #expect(h.lines("#a")[0].reactions == [Reaction(emoji: "🚀", nicks: ["me"])])
  }

  @Test("routes labeled replies (and labeled errors) to the asking buffer")
  func labeledErrors() async throws {
    let h = Harness()
    await h.connect(caps: ["labeled-response"])
    await h.joinWith("#a", "me")
    await h.joinWith("#b", "me")
    h.app.open(h.buf("#b"))
    h.conn.sendFrom(h.buf("#a"), "TOPIC #zzz")
    await settle()
    let label = try #require(h.sent.last?.firstMatch(of: #/^@label=(\S+) /#)?.output.1)
    await h.feed("@label=\(label) :srv 403 me #zzz :No such channel")
    let l = h.lines("#a").last
    #expect(l?.type == .error && l?.text == "#zzz No such channel")
    #expect(h.lines("#b").isEmpty)
  }

  @Test("marks redacted messages in channels and queries")
  func redacted() async {
    let h = Harness()
    await h.connect(caps: ["draft/message-redaction", "message-tags"])
    await h.joinWith("#a", "me")
    await h.feed("@msgid=m1 :ann!a@h PRIVMSG #a :oops", "@msgid=m2 :ann!a@h PRIVMSG #a :fine")
    await h.feed("@+draft/react=👍;+draft/reply=m1 :bob!b@h TAGMSG #a")
    await h.feed(":ann!a@h REDACT #a m1 :typo")
    #expect(h.lines("#a").map(\.msgid) == ["m1", "m2"])
    #expect(h.lines("#a").map(\.redacted) == [true, false])
    #expect(h.lines("#a")[0].reactions.isEmpty)

    await h.feed("@msgid=p1 :ann!a@h PRIVMSG me :secret")
    await h.feed(":ann!a@h REDACT me p1")
    #expect(h.lines("ann")[0].redacted)
  }

  @Test("lets ops and authors redact")
  func canRedact() async {
    let h = Harness()
    await h.connect(caps: ["draft/message-redaction"])
    await h.joinWith("#a", "me ann")
    await h.feed("@msgid=m1 :ann!a@h PRIVMSG #a :x", "@msgid=m2 :me!u@h PRIVMSG #a :mine")
    let theirs = h.lines("#a")[0]
    let mine = h.lines("#a")[1]
    #expect(!h.app.canRedact(h.buf("#a"), theirs))
    #expect(h.app.canRedact(h.buf("#a"), mine))
    await h.feed(":srv MODE #a +o me")
    #expect(h.app.canRedact(h.buf("#a"), theirs))
    h.app.redact(h.buf("#a"), theirs)
    await settle()
    #expect(h.sent.last == "REDACT #a m1")
  }
}

@MainActor @Suite("typing and read markers")
struct TypingReadMarkerTests {
  @Test("tracks typing notifications per buffer")
  func tracksTyping() async {
    let h = Harness()
    await h.connect(caps: ["message-tags"])
    await h.joinWith("#a", "me ann bob")
    await h.feed("@+typing=active :ann!a@h TAGMSG #a", "@+typing=paused :bob!b@h TAGMSG #a")
    let typing = h.buf("#a").typing
    #expect(typing.keys.sorted() == ["ann", "bob"])
    #expect(typing["bob"]! - typing["ann"]! > 20_000)
    await h.feed("@+typing=done :bob!b@h TAGMSG #a")
    #expect(h.buf("#a").typing["bob"] == nil)
    // Sending a message clears the indicator.
    await h.feed(":ann!a@h PRIVMSG #a :done typing")
    #expect(h.buf("#a").typing.isEmpty)
  }

  @Test("files private typing under the sender and skips our own")
  func privateTyping() async {
    let h = Harness()
    await h.connect(caps: ["message-tags"])
    await h.feed(":ann!a@h PRIVMSG me :hi", "@+typing=active :ann!a@h TAGMSG me")
    #expect(h.buf("ann").typing["ann"] != nil)
    await h.joinWith("#a", "me")
    await h.feed("@+typing=active :me!u@h TAGMSG #a")
    #expect(h.buf("#a").typing.isEmpty)
  }

  @Test("respects the showTyping setting")
  func showTyping() async {
    let h = Harness()
    await h.connect(caps: ["message-tags"])
    h.app.settings.showTyping = false
    await h.joinWith("#a", "me ann")
    await h.feed("@+typing=active :ann!a@h TAGMSG #a")
    #expect(h.buf("#a").typing.isEmpty)
  }

  @Test("sends typing only when allowed and throttled")
  func sendsTyping() async {
    let h = Harness()
    await h.connect(caps: ["message-tags"], isupport: ["CLIENTTAGDENY": "*,-typing"])
    await h.joinWith("#a", "me")
    h.sent = []
    h.app.sendTyping(h.buf("#a"), .active)
    h.app.sendTyping(h.buf("#a"), .active)
    h.app.sendTyping(h.buf("#a"), .done)
    await settle()
    #expect(h.sent == ["@+typing=active TAGMSG #a", "@+typing=done TAGMSG #a"])
    h.sent = []
    h.conn.isupport["CLIENTTAGDENY"] = "typing"
    h.app.sendTyping(h.buf("#a"), .active)
    await settle()
    #expect(h.sent == [])
  }

  @Test("moves the read marker forward only")
  func readMarkerForward() async {
    let h = Harness()
    await h.connect(caps: ["draft/read-marker"])
    await h.joinWith("#a", "me")
    #expect(h.buf("#a").readMarker == 0)
    await h.feed(":srv MARKREAD #a timestamp=2026-01-01T00:00:10.000Z")
    #expect(h.buf("#a").readMarker == ms("2026-01-01T00:00:10.000Z"))
    await h.feed(":srv MARKREAD #a timestamp=2026-01-01T00:00:05.000Z", ":srv MARKREAD #a *")
    #expect(h.buf("#a").readMarker == ms("2026-01-01T00:00:10.000Z"))
  }

  @Test("counts unread lines after the marker and syncs markRead")
  func unreadAndMarkRead() async {
    let h = Harness()
    await h.connect(caps: ["draft/read-marker"])
    await h.joinWith("#a", "me")
    await h.feed(
      "@time=2026-01-01T00:00:01.000Z :ann!a@h PRIVMSG #a :one", "@time=2026-01-01T00:00:02.000Z :ann!a@h PRIVMSG #a :two")
    await h.feed(":srv MARKREAD #a timestamp=2026-01-01T00:00:01.000Z")
    #expect(h.buf("#a").unread == 1)
    h.app.markRead(h.buf("#a"))
    #expect(h.buf("#a").unread == 0)
    await sleep(ms: 500)
    #expect(h.sent.last == "MARKREAD #a timestamp=2026-01-01T00:00:02.000Z")
  }
}

@MainActor @Suite("PRIVMSG, NOTICE and CTCP")
struct PrivmsgNoticeCtcpTests {
  @Test("shows ACTIONs as action lines")
  func actions() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me ann")
    await h.feed(":ann!a@h PRIVMSG #a :\(X1)ACTION waves at me\(X1)", ":ann!a@h PRIVMSG #a :\(X1)ACTION\(X1)")
    #expect(h.lines("#a").map(\.type) == [.action, .action])
    #expect(h.lines("#a").map(\.text) == ["waves at me", ""])
    #expect(h.lines("#a").map(\.highlight) == [true, false])
  }

  @Test("answers CTCP requests and reports replies without chat lines")
  func ctcp() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me ann")
    h.app.open(h.buf("#a"))
    await h.feed(
      ":ann!a@h PRIVMSG me :\(X1)PING 12345\(X1)", ":ann!a@h PRIVMSG #a :\(X1)VERSION\(X1)",
      ":ann!a@h PRIVMSG me :\(X1)FOO\(X1)")
    #expect(h.sent.count == 2)
    #expect(h.sent.first == "NOTICE ann :\(X1)PING 12345\(X1)")
    #expect(h.sent.last?.wholeMatch(of: #/NOTICE ann :\x01VERSION .+\x01/#) != nil)
    await h.feed(":ann!a@h NOTICE me :\(X1)VERSION irssi\(X1)")
    #expect(
      h.texts("#a") == [
        "CTCP PING request from ann",
        "CTCP VERSION request from ann",
        "CTCP FOO request from ann",
        "CTCP VERSION reply from ann: irssi",
      ])
    #expect(h.conn.buffer("ann") == nil)
  }

  @Test("rate-limits CTCP replies")
  func ctcpFlood() async {
    let h = Harness()
    await h.connect()
    for i in 0..<10 { await h.feed(":x\(i)!u@h PRIVMSG me :\(X1)VERSION\(X1)") }
    #expect(h.sent.filter { $0.hasPrefix("NOTICE") }.count == 3)
    #expect(h.conn.server.messages.contains { $0.text.contains("flood") })
  }

  @Test("routes server notices to the server buffer")
  func serverNotices() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me")
    h.app.open(h.buf("#a"))
    await h.feed(
      ":irc.example.net NOTICE * :*** Looking up your hostname", ":irc.example.net NOTICE me :server says hi",
      "NOTICE me :sourceless")
    #expect(h.conn.server.messages.map(\.text) == ["*** Looking up your hostname", "server says hi", "sourceless"])
    #expect(h.lines("#a").isEmpty)
  }

  @Test("routes user notices to an open query, else the active buffer")
  func userNotices() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me")
    h.app.open(h.buf("#a"))
    await h.feed(":NickServ!s@services. NOTICE me :Please identify")
    #expect(h.conn.buffer("NickServ") == nil)
    let l = h.lines("#a").last
    #expect(l?.type == .notice && l?.nick == "NickServ" && l?.text == "Please identify")
    await h.feed(":ann!a@h PRIVMSG me :hi", ":ann!a@h NOTICE me :fyi")
    #expect(h.lines("ann").map(\.type) == [.privmsg, .notice])
    await h.feed(":ann!a@h NOTICE #a :channel notice")
    #expect(h.lines("#a").last?.type == .notice && h.lines("#a").last?.text == "channel notice")
  }

  @Test("strips STATUSMSG prefixes")
  func statusmsg() async {
    let h = Harness()
    await h.connect(isupport: ["STATUSMSG": "@+"])
    await h.joinWith("#a", "me")
    await h.feed(":ann!a@h NOTICE @#a :ops only")
    #expect(h.texts("#a") == ["ops only"])
    #expect(h.conn.buffer("@#a") == nil)
  }

  @Test("highlights on nick and configured words as whole words")
  func highlights() async {
    let h = Harness()
    await h.connect()
    h.app.settings.highlightWords = ["hearth", "c++"]
    await h.joinWith("#a", "me ann")
    await h.feed(
      ":ann!a@h PRIVMSG #a :ME, look",
      ":ann!a@h PRIVMSG #a :meme time",
      ":ann!a@h PRIVMSG #a :I love Hearth!",
      ":ann!a@h PRIVMSG #a :hearthstone",
      ":ann!a@h PRIVMSG #a :learning c++ today",
      ":ann!a@h PRIVMSG #a :\u{02}me\u{02} in bold"
    )
    #expect(h.lines("#a").map(\.highlight) == [true, false, true, false, true, true])
    // Our own messages never highlight.
    await h.feed(":me!u@h PRIVMSG #a :hearth me")
    #expect(h.lines("#a").last?.highlight == false)
  }

  @Test("notifies for highlights and private messages when not looking")
  func notifies() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me ann")
    h.app.focused = false
    await h.feed(":ann!a@h PRIVMSG #a :chatter", ":ann!a@h PRIVMSG #a :me: ping", ":ann!a@h PRIVMSG me :dm")
    #expect(h.notified.map { String($0.title.split(separator: " · ")[0]) } == ["ann in #a", "ann"])
  }

  @Test("drops everything from ignored masks, case-insensitively")
  func ignored() async {
    let h = Harness()
    await h.connect(isupport: ["CASEMAPPING": "rfc1459"])
    h.app.settings.ignores = ["troll[1]!*@*", "*!*@*.spam.example"]
    await h.joinWith("#a", "me")
    h.app.open(h.buf("#a"))
    await h.feed(
      ":TROLL{1}!x@y PRIVMSG #a :hi",
      ":bot!b@node.spam.example PRIVMSG me :buy now",
      ":bot!b@node.spam.example INVITE me #spam",
      ":bot!b@node.spam.example NOTICE me :notice spam",
      ":ok!o@good.example PRIVMSG #a :real"
    )
    #expect(h.texts("#a") == ["real"])
    #expect(h.conn.buffer("bot") == nil)
  }

  @Test("announces invites")
  func invites() async {
    let h = Harness()
    await h.connect()
    await h.feed(":ann!a@h INVITE me #party", ":ann!a@h INVITE bob #party")
    #expect(h.conn.server.messages.map(\.text) == ["invited you to #party", "invited bob to #party"])
    #expect(h.conn.server.messages.map(\.highlight) == [true, false])
  }
}

@MainActor @Suite("ISUPPORT")
struct IsupportTests {
  @Test("folds nicks and channels with rfc1459 casemapping")
  func rfc1459() async {
    let h = Harness()
    await h.connect(isupport: ["CASEMAPPING": "rfc1459"])
    await h.feed(":me!u@h JOIN #Chan[x]", ":Ann[away]!a@h JOIN #chan{X}", ":ann{AWAY}!a@h PRIVMSG #CHAN{x} :hi")
    #expect(h.conn.buffers.count == 1)
    let b = h.buf("#chan{x}")
    #expect(b.name == "#Chan[x]")
    #expect(b.member("ANN{away}")?.nick == "Ann[away]")
    #expect(h.texts("#Chan[x]") == ["", "hi"])
    await h.feed(":ann~!a@h PRIVMSG me :tilde", ":ANN^!a@h PRIVMSG me :caret")
    #expect(h.texts("ann^") == ["tilde", "caret"])
    #expect(h.conn.isMe("ME"))
  }

  @Test("ascii casemapping keeps brackets distinct")
  func ascii() async {
    let h = Harness()
    await h.connect(isupport: ["CASEMAPPING": "ascii"])
    await h.feed(":a[1]!a@h PRIVMSG me :x", ":A{1}!a@h PRIVMSG me :y")
    #expect(h.texts("a[1]") == ["x"])
    #expect(h.texts("a{1}") == ["y"])
  }

  @Test("uses CHANTYPES to tell channels from nicks")
  func chantypes() async {
    let h = Harness()
    await h.connect(isupport: ["CHANTYPES": "#&"])
    await h.feed(":me!u@h JOIN &local", ":ann!a@h PRIVMSG &local :hey")
    #expect(h.buf("&local").kind == .channel)
    #expect(h.texts("&local") == ["hey"])
    #expect(!h.conn.isChannel("!odd"))
    #expect(!h.conn.isChannel(""))
  }

  @Test("defaults PREFIX when it is missing or malformed")
  func prefixDefault() async {
    let h = Harness()
    await h.connect(isupport: ["PREFIX": "(ohv)@+"])
    #expect(h.conn.prefix == PrefixMap(modes: Array("ov"), symbols: Array("@+")))
    h.conn.isupport["PREFIX"] = "(yqaohv)!~&@%+"
    #expect(h.conn.prefix == PrefixMap(modes: Array("yqaohv"), symbols: Array("!~&@%+")))
  }
}

@MainActor @Suite("numerics")
struct NumericTests {
  @Test("routes errors about a buffer to that buffer")
  func errorsToBuffer() async {
    let h = Harness()
    await h.connect()
    await h.joinWith("#a", "me")
    await h.joinWith("#b", "me")
    await h.feed(":ann!a@h PRIVMSG me :hi")
    h.app.open(h.buf("#b"))
    await h.feed(":srv 404 me #a :Cannot send to channel")
    await h.feed(":srv 482 me #a :You're not channel operator")
    await h.feed(":srv 401 me ann :No such nick/channel")
    #expect(h.lines("#a").suffix(2).map(\.type) == [.error, .error])
    #expect(h.lines("#a").suffix(2).map(\.text) == ["#a Cannot send to channel", "#a You're not channel operator"])
    #expect(h.lines("ann").last?.type == .error && h.lines("ann").last?.text == "ann No such nick/channel")
    #expect(h.lines("#b").isEmpty)
  }

  @Test("falls back to the active buffer, then the server buffer")
  func fallback() async {
    let h = Harness()
    await h.connect()
    await h.feed(":srv 403 me #nowhere :No such channel")
    let l = h.conn.server.messages.last
    #expect(l?.type == .error && l?.text == "#nowhere No such channel")
    await h.joinWith("#a", "me")
    h.app.open(h.buf("#a"))
    await h.feed(":srv 401 me ghost :No such nick/channel", ":srv 421 me FOO :Unknown command")
    #expect(h.texts("#a") == ["ghost No such nick/channel", "FOO Unknown command"])
  }

  @Test("reports nick collisions and SASL failures")
  func nickAndSasl() async {
    let h = Harness()
    await h.connect()
    await h.feed(":srv 433 me taken :Nickname is already in use", ":srv 904 me :SASL authentication failed")
    #expect(h.conn.server.messages.map(\.type) == [.error, .error])
    #expect(
      h.conn.server.messages.map(\.text) == ["Nickname taken is already in use", "SASL: SASL authentication failed"])
  }

  @Test("collects the MOTD into one block, dropping the 375 header")
  func motd() async {
    let h = Harness()
    await h.connect()
    await h.feed(
      ":srv 375 me :- srv Message of the day", ":srv 372 me :- line one", ":srv 372 me :-line two", ":srv 376 me :End")
    #expect(h.conn.server.messages.count == 1)
    #expect(h.conn.server.messages.first?.data == ["motd": "line one\nline two"])
  }

  @Test("assembles WHOIS replies into one card")
  func whois() async {
    let h = Harness()
    await h.connect()
    await h.feed(
      ":srv 311 me ann a host.example * :Ann Example",
      ":srv 312 me ann irc.example :The Server",
      ":srv 319 me ann :@#a",
      ":srv 319 me ann :+#b",
      ":srv 330 me ann annacct :is logged in as",
      ":srv 301 me ann :lunch",
      ":srv 318 me ann :End of /WHOIS list."
    )
    let card = h.conn.server.messages.last!
    #expect(card.type == .whois)
    let expected = [
      "nick": "ann", "user": "a", "host": "host.example", "realname": "Ann Example",
      "server": "irc.example — The Server", "channels": "@#a +#b", "account": "annacct", "away": "lunch",
    ]
    for (k, v) in expected { #expect(card.data?[k] == v, "\(k)") }
  }

  @Test("shows FAIL for the asking buffer and hides CHATHISTORY failures")
  func failReplies() async {
    let h = Harness()
    await h.connect()
    await h.feed(
      "FAIL CHATHISTORY INVALID_TARGET #a :nope", "FAIL NICK ERRONEOUS_NICKNAME * :Bad nick", "NOTE SERVER RESTART :soon")
    #expect(h.conn.server.messages.map(\.type) == [.error, .info])
    #expect(h.conn.server.messages.map(\.text) == ["NICK failed: Bad nick", "soon"])
  }
}
