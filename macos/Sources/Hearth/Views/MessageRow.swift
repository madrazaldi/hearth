// Rows in the scrollback: messages, events, collapsed join/part runs, WHOIS
// cards and MOTD blocks (MessageRow, EventRow, EventRun, WhoisCard, MotdBlock).

import SwiftUI

/// Widths of the "columns" layout gutters.
private let timeWidth: CGFloat = 50
private let nickWidth: CGFloat = 132

@MainActor private func clock(_ app: AppModel, _ t: Int) -> String {
  formatClock(t, clock: app.settings.clock, seconds: app.settings.showSeconds)
}

/// Opens a nick card for `hearth-nick:` links inside a row.
private struct NickLinks: ViewModifier {
  let buf: Buffer
  @LocalState private var nick: String?
  @Environment(\.openURL) private var parent

  func body(content: Content) -> some View {
    content
      .environment(\.openURL, OpenURLAction { url in
        if url.scheme == nickScheme {
          nick = url.path
          return .handled
        }
        parent(url)
        return .handled
      })
      .popover(item: $nick, arrowEdge: .leading) { n in NickCard(buf: buf, nick: n) { nick = nil } }
  }
}

extension String: @retroactive Identifiable {
  public var id: String { self }
}

private struct RowBackground: ViewModifier {
  var highlight: Bool
  var flash: Bool
  var hover: Bool
  @Environment(\.palette) private var p

  func body(content: Content) -> some View {
    content
      .background(flash ? p.accentSoft : highlight ? p.mention : hover ? p.hover : .clear)
      .overlay(alignment: .leading) { if highlight { Rectangle().fill(p.accent).frame(width: 2) } }
      .animation(.easeOut(duration: 0.4), value: flash)
  }
}

extension View {
  @ViewBuilder fileprivate func selectable(_ on: Bool) -> some View {
    if on { textSelection(.enabled) } else { self }
  }
}

/// Scrolling moves rows under a resting pointer, and each one gets a hover.
/// Waiting for the pointer to settle before showing the tools keeps a scroll
/// from rebuilding every row it passes.
@MainActor private var pendingHover: (id: Int, task: Task<Void, Never>)?

struct MessageRow: View {
  let line: Line
  let cont: Bool
  let buf: Buffer
  let flash: Bool
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  @LocalState private var hover = false
  /// Selectable text is costly to lay out, so a row only turns it on once
  /// the pointer has rested on it, and keeps it on so a selection survives
  /// the pointer leaving.
  @LocalState private var selectable = false
  @LocalState private var nickCard = false

  private var size: CGFloat { CGFloat(app.settings.fontSize) }

  var body: some View {
    let compact = app.settings.layout == .compact
    Group {
      if compact { compactBody } else { columnsBody }
    }
    .padding(.horizontal, 16)
    .padding(.top, cont ? 1 : compact ? 8 : 3)
    .padding(.bottom, 1)
    .frame(maxWidth: .infinity, alignment: .leading)
    .contentShape(Rectangle())
    .modifier(RowBackground(highlight: line.highlight, flash: flash, hover: hover))
    .overlay(alignment: .topTrailing) {
      if hover { MessageTools(line: line, buf: buf).padding(.trailing, 12).offset(y: -10) }
    }
    .onHover(perform: hovering)
    .contextMenu { MessageMenu(line: line, buf: buf) }
    .modifier(NickLinks(buf: buf))
  }

  private func hovering(_ inside: Bool) {
    if inside {
      pendingHover?.task.cancel()
      pendingHover = (line.id, Task {
        try? await Task.sleep(for: .milliseconds(100))
        if !Task.isCancelled {
          hover = true
          selectable = true
        }
      })
    } else {
      if pendingHover?.id == line.id { pendingHover?.task.cancel() }
      if hover { hover = false }
    }
  }

  private var nickButton: some View {
    // A tap, not a Button: buttons join the focus store, and every row
    // scrolling in or out then invalidates all the others.
    HStack(alignment: .firstTextBaseline, spacing: 0) {
      if let pre = line.prefix { Text(String(pre)).font(.system(size: size, weight: .medium)).foregroundStyle(p.text3) }
      NickText(nick: line.nick)
    }
    .onTapGesture { nickCard = true }
    .accessibilityAddTraits(.isButton)
    .accessibilityAction { nickCard = true }
    .popover(isPresented: $nickCard, arrowEdge: .bottom) { NickCard(buf: buf, nick: line.nick) { nickCard = false } }
  }

  private var columnsBody: some View {
    HStack(alignment: .firstTextBaseline, spacing: 10) {
      Text(clock(app, line.time))
        .font(.system(size: size - 2.5).monospacedDigit())
        .foregroundStyle(p.text4)
        .frame(width: timeWidth, alignment: .leading)
        .help(formatFull(line.time))
      // A ZStack, not a Group: continuation lines still need the gutter's width.
      ZStack(alignment: .trailing) {
        Color.clear.frame(height: 1)
        if line.type == .action {
          Text("*").foregroundStyle(p.text4)
        } else if !cont {
          HStack(spacing: 3) {
            if line.type == .notice { Text("–").foregroundStyle(p.text4) }
            nickButton
          }
        }
      }
      .frame(width: nickWidth, alignment: .trailing)
      .lineLimit(1)
      VStack(alignment: .leading, spacing: 4) {
        ReplyQuote(line: line, buf: buf)
        bodyText(action: line.type == .action)
        Extras(line: line, buf: buf)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
  }

  private var compactBody: some View {
    HStack(alignment: .top, spacing: 10) {
      if app.settings.loadAvatars {
        Group {
          if cont || line.type == .action { Color.clear } else { AvatarView(conn: buf.conn, nick: line.nick, size: 30) }
        }
        .frame(width: 30)
      }
      VStack(alignment: .leading, spacing: 3) {
        if !cont && line.type != .action {
          HStack(alignment: .firstTextBaseline, spacing: 7) {
            nickButton
            if let d = buf.conn.profile(line.nick).displayName, d != line.nick {
              Text(d).font(.system(size: size - 2)).foregroundStyle(p.text3)
            }
            if line.type == .notice {
              Text("notice").font(.system(size: 10, weight: .medium)).foregroundStyle(p.text3)
                .padding(.horizontal, 5).background(Capsule().fill(p.active))
            }
            Text(clock(app, line.time)).font(.system(size: size - 3).monospacedDigit()).foregroundStyle(p.text4)
              .help(formatFull(line.time))
          }
        }
        ReplyQuote(line: line, buf: buf)
        bodyText(action: line.type == .action)
        Extras(line: line, buf: buf)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
  }

  @ViewBuilder
  private func bodyText(action: Bool) -> some View {
    if line.redacted {
      Text("message deleted").italic().font(.system(size: size)).foregroundStyle(p.text4)
    } else {
      let text = ircAttributed(line.text, palette: p, size: size)
      Group {
        if action {
          var prefix = AttributedString(action ? "\(line.nick) " : "")
          let _ = {
            prefix.font = .system(size: size, weight: .semibold)
            prefix.foregroundColor = app.settings.nickColors ? p.nick(line.nick) : p.text
            prefix.link = nickURL(line.nick)
          }()
          Text(prefix + text).italic()
        } else {
          Text(text)
        }
      }
      .foregroundStyle(line.type == .notice ? p.text2 : p.text)
      .selectable(selectable)
      .fixedSize(horizontal: false, vertical: true)
    }
  }
}

/// "↪ nick: what they said" above a reply.
private struct ReplyQuote: View {
  let line: Line
  let buf: Buffer
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    if let parentId = line.replyTo, !line.redacted {
      let parent = buf.line(msgid: parentId)
      Button {
        if let parent { Task { await app.jumpTo(buf, time: parent.time, msgid: parent.msgid) } }
      } label: {
        HStack(spacing: 5) {
          Text("↪").foregroundStyle(p.text4)
          if let parent {
            NickText(nick: parent.nick, size: CGFloat(app.settings.fontSize) - 2)
            Text(parent.redacted ? "message deleted" : String(stripFormatting(parent.text).prefix(140)))
              .foregroundStyle(p.text3)
              .lineLimit(1)
          } else {
            Text("a message that isn't loaded").foregroundStyle(p.text4)
          }
        }
        .font(.system(size: CGFloat(app.settings.fontSize) - 2))
        .padding(.leading, 8)
        .overlay(alignment: .leading) { Rectangle().fill(p.lineStrong).frame(width: 2) }
      }
      .buttonStyle(.plain)
      .disabled(parent == nil)
      .help(parent != nil ? "Show the original message" : "")
    }
  }
}

/// Reactions and link previews under a message.
private struct Extras: View {
  let line: Line
  let buf: Buffer
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    if !line.redacted {
      if !line.reactions.isEmpty {
        HStack(spacing: 4) {
          ForEach(line.reactions, id: \.emoji) { r in
            let mine = r.nicks.contains(where: buf.conn.isMe)
            Button {
              app.react(buf, line, r.emoji)
            } label: {
              HStack(spacing: 3) {
                Text(r.emoji)
                Text("\(r.nicks.count)").font(.system(size: 11, weight: .medium).monospacedDigit())
              }
              .padding(.horizontal, 7)
              .padding(.vertical, 2)
              .background(Capsule().fill(mine ? p.accentSoft : p.hover))
              .overlay(Capsule().strokeBorder(mine ? p.accent.opacity(0.5) : p.line))
            }
            .buttonStyle(.plain)
            .disabled(mine || !app.canReact(buf, line))
            .help(r.nicks.joined(separator: ", "))
          }
        }
      }
      if app.settings.linkPreviews && buf.kind != .server, let url = firstLink(line.text) {
        LinkPreviewView(url: url, nsfw: isNsfw(line.text))
      }
    }
  }
}

// ─── Hover tools and the message menu ────────────────────────────

private let quickReactions = ["👍", "❤️", "😂", "😮", "🎉", "👀"]

private struct MessageTools: View {
  let line: Line
  let buf: Buffer
  @Environment(AppModel.self) private var app
  @Environment(ComposerBridge.self) private var composer: ComposerBridge?
  @Environment(\.palette) private var p
  @LocalState private var reacting = false

  var body: some View {
    HStack(spacing: 0) {
      if line.msgid != nil && buf.kind != .server && !line.redacted {
        tool("arrowshape.turn.up.left", "Reply") {
          buf.replyTo = line
          composer?.focus += 1
        }
      }
      if app.canReact(buf, line) && !line.redacted {
        tool("face.smiling", "React") { reacting = true }
          .popover(isPresented: $reacting) {
            HStack(spacing: 2) {
              ForEach(quickReactions, id: \.self) { e in
                Button(e) {
                  app.react(buf, line, e)
                  reacting = false
                }
                .buttonStyle(.plain)
                .font(.system(size: 20))
                .padding(4)
              }
            }
            .padding(6)
          }
      }
      tool("doc.on.doc", "Copy") { copy(line) }
      if app.canRedact(buf, line) && !line.redacted {
        tool("trash", "Delete") { app.redact(buf, line) }
      }
    }
    .padding(2)
    .background(RoundedRectangle(cornerRadius: 7).fill(p.raised))
    .overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(p.lineStrong))
    .shadow(color: .black.opacity(0.15), radius: 6, y: 2)
  }

  private func tool(_ icon: String, _ label: String, _ action: @escaping () -> Void) -> some View {
    Button(action: action) {
      Image(systemName: icon).font(.system(size: 12)).frame(width: 26, height: 22).contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .foregroundStyle(p.text2)
    .help(label)
    .accessibilityLabel(label)
  }
}

private func copy(_ line: Line) {
  let text = stripFormatting(line.text)
  NSPasteboard.general.clearContents()
  NSPasteboard.general.setString(line.type == .action ? "* \(line.nick) \(text)" : text, forType: .string)
}

private struct MessageMenu: View {
  let line: Line
  let buf: Buffer
  @Environment(AppModel.self) private var app
  @Environment(ComposerBridge.self) private var composer: ComposerBridge?

  var body: some View {
    if line.msgid != nil && buf.kind != .server && !line.redacted {
      Button("Reply") {
        buf.replyTo = line
        composer?.focus += 1
      }
    }
    if app.canReact(buf, line) && !line.redacted {
      Menu("React") {
        ForEach(quickReactions, id: \.self) { e in Button(e) { app.react(buf, line, e) } }
      }
    }
    Button("Copy Text") { copy(line) }
    if let url = firstLink(line.text) {
      Button("Copy Link") {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(url, forType: .string)
      }
    }
    if !line.nick.isEmpty && !line.isSelf {
      Divider()
      NickActions(buf: buf, nick: line.nick)
    }
    if app.canRedact(buf, line) && !line.redacted {
      Divider()
      Button("Delete Message", role: .destructive) { app.redact(buf, line) }
    }
  }
}

// ─── Events ──────────────────────────────────────────────────────

private let glyphs: [LineType: String] = [
  .join: "→", .part: "←", .quit: "←", .nick: "↻", .kick: "⨯", .mode: "±", .topic: "¶", .invite: "✉", .info: "·",
  .error: "!",
]

/// A join, part, topic change, … as one attributed line, with nicks linking to their cards.
@MainActor
private func eventText(_ l: Line, palette p: Palette, app: AppModel, size: CGFloat) -> AttributedString {
  func plain(_ s: String, faint: Bool = false) -> AttributedString {
    var a = AttributedString(s)
    a.font = .system(size: size)
    a.foregroundColor = faint ? p.text4 : p.text2
    return a
  }
  func nick(_ n: String) -> AttributedString {
    var a = AttributedString(n)
    a.font = .system(size: size, weight: .semibold)
    a.foregroundColor = app.settings.nickColors ? p.nick(n) : p.text
    a.link = nickURL(n)
    return a
  }
  func reason(_ s: String) -> AttributedString { s.isEmpty ? AttributedString() : plain(" (\(stripFormatting(s)))", faint: true) }
  switch l.type {
  case .join:
    let host = l.data?["host"] ?? ""
    return nick(l.nick) + plain(" joined") + (host.isEmpty || host == "@" ? AttributedString() : plain(" \(host)", faint: true))
  case .part: return nick(l.nick) + plain(" left") + reason(l.text)
  case .quit: return nick(l.nick) + plain(" quit") + reason(l.text)
  case .nick: return nick(l.nick) + plain(" is now ") + nick(l.target ?? "")
  case .kick: return nick(l.nick) + plain(" removed ") + nick(l.target ?? "") + reason(l.text)
  case .mode:
    var mode = AttributedString(l.text)
    mode.font = .system(size: size - 1, design: .monospaced)
    mode.foregroundColor = p.text2
    return (l.nick.isEmpty ? plain("Mode ") : nick(l.nick) + plain(" set mode ")) + mode
  case .topic: return nick(l.nick) + plain(" changed the topic to ") + ircAttributed(l.text, palette: p, size: size)
  case .invite: return nick(l.nick) + plain(" \(l.text)")
  case .error:
    var a = ircAttributed(l.text, palette: p, size: size)
    a.foregroundColor = p.danger
    return a
  default: return ircAttributed(l.text, palette: p, size: size)
  }
}

struct EventRow: View {
  let line: Line
  let buf: Buffer
  let flash: Bool
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    let size = CGFloat(app.settings.fontSize) - 1
    HStack(alignment: .firstTextBaseline, spacing: 10) {
      if app.settings.layout == .columns {
        Text(clock(app, line.time)).font(.system(size: size - 1.5).monospacedDigit()).foregroundStyle(p.text4)
          .frame(width: timeWidth, alignment: .leading).help(formatFull(line.time))
        Text(glyphs[line.type] ?? "·").foregroundStyle(line.type == .error ? p.danger : p.text4)
          .frame(width: nickWidth, alignment: .trailing)
      } else {
        Text(glyphs[line.type] ?? "·").foregroundStyle(line.type == .error ? p.danger : p.text4).frame(width: 14)
      }
      Text(eventText(line, palette: p, app: app, size: size))
        .textSelection(.enabled)
        .fixedSize(horizontal: false, vertical: true)
      if line.type == .invite, let chan = line.target {
        Button("Join") { app.join(buf.conn, chan) }.controlSize(.small)
      }
      Spacer(minLength: 0)
      if app.settings.layout == .compact {
        Text(clock(app, line.time)).font(.system(size: size - 2).monospacedDigit()).foregroundStyle(p.text4)
      }
    }
    .padding(.horizontal, 16)
    .padding(.vertical, 2)
    .modifier(RowBackground(highlight: line.highlight, flash: flash, hover: false))
    .modifier(NickLinks(buf: buf))
  }
}

struct EventRun: View {
  let lines: [Line]
  let buf: Buffer
  let open: Bool
  let toggle: () -> Void
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  private var summary: String {
    var joined: [String] = []
    var left: [String] = []
    var renamed: [String] = []
    for l in lines {
      if l.type == .join && !joined.contains(l.nick) {
        joined.append(l.nick)
      } else if (l.type == .part || l.type == .quit) && !left.contains(l.nick) {
        left.append(l.nick)
      } else if l.type == .nick {
        renamed.append("\(l.nick) → \(l.target ?? "")")
      }
    }
    func names(_ a: [String]) -> String {
      a.count <= 3 ? a.joined(separator: ", ") : "\(a.prefix(2).joined(separator: ", ")) and \(a.count - 2) others"
    }
    var parts: [String] = []
    if !joined.isEmpty { parts.append("\(names(joined)) joined") }
    if !left.isEmpty { parts.append("\(names(left)) left") }
    if !renamed.isEmpty { parts.append(renamed.count <= 2 ? renamed.joined(separator: ", ") : "\(renamed.count) nick changes") }
    return parts.joined(separator: " · ")
  }

  var body: some View {
    let size = CGFloat(app.settings.fontSize) - 1
    VStack(alignment: .leading, spacing: 2) {
      HStack(alignment: .firstTextBaseline, spacing: 10) {
        if app.settings.layout == .columns {
          Text(clock(app, lines[0].time)).font(.system(size: size - 1.5).monospacedDigit()).foregroundStyle(p.text4)
            .frame(width: timeWidth, alignment: .leading)
          Text("⇅").foregroundStyle(p.text4).frame(width: nickWidth, alignment: .trailing)
        } else {
          Text("⇅").foregroundStyle(p.text4).frame(width: 14)
        }
        // A tap rather than a Button, for the same reason as MessageRow's nick.
        HStack(spacing: 4) {
          Text(summary).foregroundStyle(p.text3).lineLimit(open ? nil : 1)
          Image(systemName: "chevron.right").font(.system(size: 9, weight: .semibold)).foregroundStyle(p.text4)
            .rotationEffect(.degrees(open ? 90 : 0))
        }
        .font(.system(size: size))
        .contentShape(Rectangle())
        .onTapGesture(perform: toggle)
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { toggle() }
        .help(open ? "Collapse" : "Show all")
        Spacer(minLength: 0)
      }
      if open {
        ForEach(lines) { l in
          HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(clock(app, l.time)).font(.system(size: size - 2).monospacedDigit()).foregroundStyle(p.text4)
            Text(glyphs[l.type] ?? "·").foregroundStyle(p.text4)
            Text(eventText(l, palette: p, app: app, size: size)).textSelection(.enabled)
          }
          .padding(.leading, app.settings.layout == .columns ? timeWidth + nickWidth + 20 : 24)
        }
      }
    }
    .padding(.horizontal, 16)
    .padding(.vertical, 2)
    .modifier(NickLinks(buf: buf))
  }
}

private struct BlockRow<Content: View>: View {
  let time: Int
  @ViewBuilder var content: Content
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    HStack(alignment: .top, spacing: 10) {
      if app.settings.layout == .columns {
        Text(clock(app, time)).font(.system(size: CGFloat(app.settings.fontSize) - 2.5).monospacedDigit())
          .foregroundStyle(p.text4).frame(width: timeWidth, alignment: .leading)
        Color.clear.frame(width: nickWidth, height: 1)
      }
      content.frame(maxWidth: 640, alignment: .leading)
      Spacer(minLength: 0)
    }
    .padding(.horizontal, 16)
    .padding(.vertical, 5)
  }
}

struct WhoisCard: View {
  let line: Line
  let buf: Buffer
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    let w = line.data ?? [:]
    BlockRow(time: line.time) {
      VStack(alignment: .leading, spacing: 8) {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
          NickText(nick: w["nick"] ?? line.nick)
          Text("\(w["user"] ?? "")@\(w["host"] ?? "")").font(.system(size: 11, design: .monospaced)).foregroundStyle(p.text3)
        }
        Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 14, verticalSpacing: 4) {
          field("Name", w["realname"]) { RichText(text: $0, plain: true, size: 12) }
          field("Account", w["account"]) { Text($0) }
          field("Server", w["server"]) { Text($0) }
          field("Channels", w["channels"]) { Text($0).font(.system(size: 11, design: .monospaced)) }
          field("Idle", w["idle"]) { idle in
            let since = w["signon"].flatMap { Int($0) }.map { " · online since \(formatFull($0 * 1000))" } ?? ""
            Text(formatDuration(Int(idle) ?? 0) + since)
          }
          field("Away", w["away"]) { Text($0) }
          field("Operator", w["oper"]) { Text($0) }
          field("Security", w["secure"]) { Text($0) }
          field("Actual", w["actual"]) { Text($0).font(.system(size: 11, design: .monospaced)) }
        }
        .font(.system(size: 12))
      }
      .textSelection(.enabled)
      .padding(12)
      .background(RoundedRectangle(cornerRadius: 9).fill(p.raised))
      .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(p.line))
    }
    .modifier(NickLinks(buf: buf))
  }

  @ViewBuilder
  private func field<V: View>(_ label: String, _ value: String?, @ViewBuilder _ content: (String) -> V) -> some View {
    if let value, !value.isEmpty {
      GridRow {
        Text(label).foregroundStyle(p.text3)
        content(value).foregroundStyle(p.text)
      }
    }
  }
}

struct MotdBlock: View {
  let line: Line
  let text: String
  let title: String?
  @Environment(\.palette) private var p
  @LocalState private var open: Bool

  init(line: Line, text: String, title: String?) {
    self.line = line
    self.text = text
    self.title = title
    // /help opens straight away; a connection's MOTD stays folded.
    _open = LocalState(initialValue: title != nil)
  }

  var body: some View {
    BlockRow(time: line.time) {
      DisclosureGroup(isExpanded: $open) {
        ScrollView(.horizontal) {
          RichText(text: text, size: 12)
            .font(.system(size: 12, design: .monospaced))
            .textSelection(.enabled)
            .fixedSize()
            .padding(.vertical, 6)
        }
      } label: {
        Text(title ?? "Message of the day").font(.system(size: 12, weight: .medium)).foregroundStyle(p.text2)
      }
    }
  }
}
