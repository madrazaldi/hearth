// IRCv3 extensions (ircv3.test.ts): draft/multiline, standard-replies,
// draft/channel-rename, draft/account-registration and draft/metadata-2.
// The pure multiline helper tests live in PureTests.swift.

import Foundation
import Testing

@testable import Hearth

private let ML = ["batch", "draft/multiline", "message-tags"]

extension Harness {
  func inChannel(_ caps: [String] = [], _ capValues: [String: String] = [:]) async -> Buffer {
    await connect(caps: caps, capValues: capValues)
    await feed(":me!u@h JOIN #a", ":srv 353 me = #a :@me ann", ":srv 366 me #a :End")
    sent = []
    app.open(buf("#a"))
    await settle()
    return buf("#a")
  }

  /// Sent lines with generated batch refs and labels replaced, for comparison.
  var normalized: [String] {
    sent.map { $0.replacing(/ml\d+/, with: "ML").replacing(/label=f\d+/, with: "label=L") }
  }

  func label(_ i: Int = 0) -> String? { sent[i].firstMatch(of: /label=(f\d+)/).map { String($0.output.1) } }
}

@MainActor @Suite("draft/multiline")
struct MultilineTests {
  @Test("sends composer text with line breaks as one batch, keeping blank lines")
  func oneBatch() async throws {
    let h = Harness()
    let a = await h.inChannel(
      ML + ["labeled-response", "echo-message"], ["draft/multiline": "max-bytes=4096,max-lines=24"])
    _ = await h.run("first\n\nthird\n", a, replyTo: "parent1")
    #expect(
      h.normalized == [
        "@label=L;+draft/reply=parent1 BATCH +ML draft/multiline #a",
        "@batch=ML PRIVMSG #a :first",
        "@batch=ML PRIVMSG #a :",
        "@batch=ML PRIVMSG #a :third",
        "BATCH -ML",
      ])
    // With echo-message, the message shows up once, when the server echoes it.
    #expect(a.messages.isEmpty)
    let label = try #require(h.label())
    await h.feed("@label=\(label);msgid=e1 :me!u@h PRIVMSG #a :first\n\nthird")
    #expect(h.texts("#a") == ["first\n\nthird"])
    a.merge([a.messages[0]])
    #expect(a.messages.count == 1)
  }

  @Test("shows our own batch locally without echo-message and splits at max-lines")
  func localEcho() async {
    let h = Harness()
    let a = await h.inChannel(ML, ["draft/multiline": "max-bytes=4096,max-lines=2"])
    _ = await h.run("one\ntwo\nthree", a)
    #expect(
      h.normalized == [
        "BATCH +ML draft/multiline #a",
        "@batch=ML PRIVMSG #a :one",
        "@batch=ML PRIVMSG #a :two",
        "BATCH -ML",
        "BATCH +ML draft/multiline #a",
        "@batch=ML PRIVMSG #a :three",
        "BATCH -ML",
      ])
    #expect(h.texts("#a") == ["one\ntwo", "three"])
    #expect(a.messages.allSatisfy { $0.isSelf })
  }

  @Test("marks continuations of a long line with multiline-concat")
  func concat() async {
    let h = Harness()
    let a = await h.inChannel(ML, ["draft/multiline": "max-bytes=8192"])
    let long = String(repeating: "x", count: 700)
    _ = await h.run("\(long)\nend", a)
    let body = h.sent.filter { $0.contains("PRIVMSG") }
    #expect(body.count == 3)
    let second = body.count > 1 ? body[1] : ""
    let concatTagged = second.firstMatch(of: #/^@batch=ml\d+;draft/multiline-concat PRIVMSG/#) != nil
    #expect(concatTagged)
    #expect(body.map { String($0.split(separator: " :", maxSplits: 1)[1]) }.joined() == long + "end")
  }

  @Test("falls back to one PRIVMSG per line without the cap or for a single line")
  func fallback() async {
    let h = Harness()
    let a = await h.inChannel(["batch"])
    _ = await h.run("one\n\ntwo", a)
    #expect(h.sent == ["PRIVMSG #a :one", "PRIVMSG #a :two"])
    h.sent = []
    h.conn.caps = Set(ML)
    h.conn.capValues = ["draft/multiline": "max-bytes=4096"]
    _ = await h.run("just one\n", a)
    #expect(h.sent == ["PRIVMSG #a :just one"])
  }

  @Test("receives a combined multiline message as one line with its breaks")
  func receives() async {
    let h = Harness()
    _ = await h.inChannel(ML)
    await h.feed("@msgid=m1 :ann!a@h PRIVMSG #a :hello\nme: are you there?")
    let line = h.lines("#a")[0]
    #expect(line.text == "hello\nme: are you there?")
    #expect(line.highlight)
  }
}

@MainActor @Suite("standard replies")
struct StandardReplyTests {
  @Test("routes labeled replies to the buffer that sent the command")
  func labeled() async throws {
    let h = Harness()
    let a = await h.inChannel(["labeled-response"])
    h.app.open(h.conn.server)
    _ = await h.run("/topic", a)
    let label = try #require(h.label())
    await h.feed("@label=\(label) FAIL TOPIC NEED_MORE_PRIVS #a :You're not a channel operator")
    let l = h.lines("#a").last
    #expect(l?.type == .error && l?.text == "TOPIC failed: You're not a channel operator")
    #expect(h.conn.server.messages.isEmpty)
  }

  @Test("files unlabeled replies under a buffer named in the context")
  func context() async {
    let h = Harness()
    _ = await h.inChannel()
    h.app.open(h.conn.server)
    await h.feed("FAIL JOIN CHANNEL_FULL #a :Channel is full")
    #expect(h.lines("#a").last?.text == "JOIN failed: Channel is full")
  }

  @Test("falls back to the active buffer, NOTEs to the server buffer")
  func fallback() async {
    let h = Harness()
    let a = await h.inChannel()
    await h.feed(
      "WARN REHASH CERTS_EXPIRED :Certificate expires soon", "FAIL * ACCOUNT_REQUIRED",
      "NOTE * OPER_MESSAGE :Maintenance tonight")
    #expect(a.messages.suffix(2).map(\.type) == [.info, .error])
    #expect(a.messages.suffix(2).map(\.text) == ["Warning (REHASH): Certificate expires soon", "Account required"])
    let l = h.conn.server.messages.last
    #expect(l?.type == .info && l?.text == "Maintenance tonight")
  }

  @Test("keeps quiet about history failures and toasts bouncer ones")
  func quiet() async {
    let h = Harness()
    let a = await h.inChannel()
    await h.feed("FAIL CHATHISTORY INVALID_TARGET #a :nope", "FAIL BOUNCER INVALID_NETID 7 :Unknown network")
    #expect(a.messages.isEmpty)
    #expect(h.toasts == ["Bouncer: Unknown network"])
  }
}

@MainActor @Suite("draft/channel-rename")
struct ChannelRenameTests {
  @Test("moves the buffer with its members, history, unread state and settings")
  func moves() async {
    let h = Harness()
    let a = await h.inChannel(["draft/channel-rename"])
    await h.feed(":ann!a@h PRIVMSG #a :before")
    a.readMarker = 42
    h.app.setNotify(a, .all)
    await h.feed(":ann!a@h RENAME #a #b :cleaning up")
    #expect(h.conn.buffer("#a") == nil)
    let b = h.buf("#b")
    #expect(b === a)
    #expect(b.name == "#b")
    #expect(b.sortedMembers.map(\.nick) == ["me", "ann"])
    #expect(b.readMarker == 42)
    #expect(h.texts("#b") == ["before", "ann renamed #a to #b (cleaning up)"])
    #expect(h.app.settings.bufferNotify == ["c||#b": .all])
    #expect(h.app.active === b)
    await h.feed(":ann!a@h PRIVMSG #b :after")
    #expect(h.texts("#b").last == "after")
  }

  @Test("replaces a stale buffer that already had the new name")
  func replacesStale() async {
    let h = Harness()
    _ = await h.inChannel()
    h.conn.ensure("#b", .channel)
    await h.feed(":op!o@h RENAME #a #b")
    #expect(Array(h.conn.buffers.keys) == ["#b"])
    #expect(h.buf("#b").joined)
  }

  @Test("/rename needs the cap and takes an optional source channel")
  func renameCommand() async {
    let h = Harness()
    let a = await h.inChannel()
    _ = await h.run("/rename #new", a)
    #expect(h.sent == [])
    #expect(h.toasts.last?.contains("does not support renaming") == true)
    h.conn.caps = ["draft/channel-rename"]
    _ = await h.run("/rename #new tidier name", a)
    _ = await h.run("/rename #a #new", h.conn.server)
    #expect(h.sent == ["RENAME #a #new :tidier name", "RENAME #a #new"])
  }
}

@MainActor @Suite("draft/account-registration")
struct AccountRegistrationTests {
  private let reg = ["draft/account-registration", "labeled-response"]

  private func setup(_ capValue: String = "custom-account-name") async -> (Harness, Buffer) {
    let h = Harness()
    var server = ServerConfig()
    server.id = "c"
    server.host = "irc.example"
    h.app.servers = [server]
    let a = await h.inChannel(reg, ["draft/account-registration": capValue])
    return (h, a)
  }

  @Test("registers, then offers to save the login for SASL")
  func registers() async throws {
    let (h, a) = await setup()
    _ = await h.run("/register me@example.com hunter2", a)
    #expect(h.normalized == ["@label=L REGISTER * me@example.com :hunter2"])
    let label = try #require(h.label())
    await h.feed("@label=\(label) REGISTER SUCCESS me :Account created")
    #expect(a.messages.last?.text == "Account me registered")
    let offer = try #require(h.app.toasts.last)
    #expect(offer.action?.label == "Save login")
    offer.action?.run()
    for _ in 0..<50 where h.backend.calls("save_server").isEmpty { await settle() }
    let saved = try #require(h.backend.calls("save_server").first?["server"] as? [String: Any])
    #expect(saved["id"] as? String == "c")
    #expect(saved["sasl"] as? String == "plain")
    #expect(saved["saslUsername"] as? String == "me")
    #expect(saved["saslPassword"] as? String == "hunter2")
    #expect(h.conn.registration == nil)
  }

  @Test("walks through verification and reports failures")
  func verification() async {
    let (h, a) = await setup()
    _ = await h.run("/register * hunter2", a)
    await h.feed("REGISTER VERIFICATION_REQUIRED me :Check your email")
    #expect(a.messages.last?.text == "Check your email — then type /verify me <code>")
    _ = await h.run("/verify me 123", a)
    await h.feed("FAIL VERIFY INVALID_CODE me :That code is wrong")
    #expect(a.messages.last?.type == .error)
    #expect(a.messages.last?.text == "Verification failed: That code is wrong")
    #expect(h.conn.registration != nil)
    await h.feed("VERIFY SUCCESS me :Welcome")
    #expect(a.messages.last?.text == "Account me verified")
    #expect(h.app.toasts.last?.action != nil)
  }

  @Test("forgets the password when registration is refused")
  func refused() async {
    let (h, a) = await setup()
    _ = await h.run("/register * hunter2", a)
    await h.feed("FAIL REGISTER ACCOUNT_EXISTS me :Already registered")
    #expect(a.messages.last?.text == "Registration failed: Already registered")
    #expect(h.conn.registration == nil)
    #expect(!h.app.toasts.contains { $0.action != nil })
  }

  @Test("checks usage and email-required, and never keeps the command for recall")
  func emailRequired() async {
    let (h, a) = await setup("email-required")
    _ = await h.run("/register * hunter2", a)
    #expect(h.sent == [])
    #expect(h.toasts.last?.contains("needs an email") == true)
    h.app.inputHistory = ["/register a@b.c hunter2", "/verify me 1", "hello"]
    #expect(h.app.inputHistory == ["hello"])
  }
}

@MainActor @Suite("draft/metadata-2")
struct MetadataTests {
  @Test("tracks avatar and display-name per user")
  func tracks() async {
    let h = Harness()
    _ = await h.inChannel(["draft/metadata-2"])
    await h.feed(
      ":srv METADATA ann avatar * :https://img.example/ann.png",
      ":srv 761 me ann display-name * :Ann Example",
      ":srv 761 me #a url * :https://example.com"
    )
    #expect(h.conn.profile("ANN").avatar == "https://img.example/ann.png")
    #expect(h.conn.profile("ANN").displayName == "Ann Example")
    await h.feed(":ann!a@h NICK anna")
    #expect(h.conn.profile("anna").displayName == "Ann Example")
    #expect(h.conn.profile("ann").avatar == nil && h.conn.profile("ann").displayName == nil)
    await h.feed(":srv METADATA anna avatar *", ":srv 766 me anna display-name :key not set")
    #expect(h.conn.profile("anna").avatar == nil && h.conn.profile("anna").displayName == nil)
    await h.feed(":srv METADATA anna avatar * :https://img.example/2.png", ":anna!a@h QUIT :bye")
    #expect(h.conn.profile("anna").avatar == nil && h.conn.profile("anna").displayName == nil)
    #expect(h.conn.server.messages.isEmpty)
  }

  @Test("retries a sync the server postponed")
  func retriesSync() async {
    let h = Harness()
    _ = await h.inChannel(["draft/metadata-2"])
    await h.feed(":srv 774 me #a 3")
    await sleep(ms: 2500)
    #expect(h.sent == [])
    await sleep(ms: 1000)
    #expect(h.sent == ["METADATA #a SYNC"])
  }

  @Test("loads avatars through the backend only when allowed")
  func avatars() async {
    let h = Harness()
    #expect(await h.app.avatar("https://img.example/a.png") == nil)
    #expect(h.backend.calls("avatar").isEmpty)
    h.app.settings.loadAvatars = true
    #expect(await h.app.avatar("https://img.example/a.png")?.hasPrefix("data:image/png;base64,") == true)
    _ = await h.app.avatar("https://img.example/a.png")
    #expect(h.backend.calls("avatar").count == 1)
  }
}
