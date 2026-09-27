// A row in a buffer (types.ts `Line`).

import Foundation

enum LineType: String, Sendable {
  case privmsg, action, notice, join, part, quit, kick, nick, mode, topic, invite, info, error, whois

  /// Lines that count as conversation for unread counts.
  var isTalk: Bool { self == .privmsg || self == .action || self == .notice }
  var isMembership: Bool { self == .join || self == .part || self == .quit || self == .nick }
}

struct Reaction: Equatable, Sendable {
  var emoji: String
  var nicks: [String]
}

private var seq = 0

/// A fresh id, shared by lines, labels and toasts as in the Svelte app.
@MainActor func nextSeq() -> Int {
  seq += 1
  return seq
}

struct Line: Identifiable, Equatable, Sendable {
  let id: Int
  var time: Int
  var type: LineType
  var nick = ""
  var text = ""
  var isSelf = false
  var highlight = false
  var msgid: String?
  /// Extra subject: kicked nick, new nick, invite channel, …
  var target: String?
  var account: String?
  var redacted = false
  /// WHOIS fields, MOTD text (`motd`, `title`), standard-reply `code`, join `host`.
  var data: [String: String]?
  /// msgid of the message this one replies to (+draft/reply).
  var replyTo: String?
  /// +draft/react, in the order they arrived.
  var reactions: [Reaction] = []

  @MainActor
  init(
    type: LineType, time: Int? = nil, nick: String = "", text: String = "", isSelf: Bool = false,
    highlight: Bool = false, msgid: String? = nil, target: String? = nil, account: String? = nil,
    data: [String: String]? = nil, replyTo: String? = nil
  ) {
    self.id = nextSeq()
    self.time = time ?? nowMs()
    self.type = type
    self.nick = nick
    self.text = text
    self.isSelf = isSelf
    self.highlight = highlight
    self.msgid = msgid
    self.target = target
    self.account = account
    self.data = data
    self.replyTo = replyTo
  }
}
