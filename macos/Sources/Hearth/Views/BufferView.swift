// One open buffer: title and topic, messages, typing, composer and members
// (BufferView.svelte).

import SwiftUI

/// Lets rows and nick cards put text into the composer ("Mention", reply).
@MainActor
@Observable
final class ComposerBridge {
  /// Text to insert, with a sequence number so repeats still register.
  var insert: (text: String, seq: Int)?
  var focus = 0

  func insert(_ text: String) {
    insert = (text, (insert?.seq ?? 0) + 1)
  }
}

/// Links inside rows: channels join, nicks open a card.
let nickScheme = "hearth-nick"

func nickURL(_ nick: String) -> URL? {
  var c = URLComponents()
  c.scheme = nickScheme
  c.path = nick
  return c.url
}

struct BufferView: View {
  let buf: Buffer
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  @LocalState private var filter = ""
  @LocalState private var topicOpen = false
  @LocalState private var composer = ComposerBridge()

  private var conn: Conn { buf.conn }

  private var title: String { buf.kind == .server ? conn.name : buf.name }

  private var subtitle: String {
    switch buf.kind {
    case .server:
      if conn.isControl { return "Bouncer · \(conn.networks.count) network\(conn.networks.count == 1 ? "" : "s")" }
      if let netid = conn.netid {
        let attrs = conn.parentId.flatMap { app.conns[$0]?.networks[netid] }
        if let host = attrs?["host"], !host.isEmpty {
          return "\(host)\(attrs?["port"].map { ":" + $0 } ?? "") via bouncer"
        }
        return "via bouncer"
      }
      return conn.config.map { "\($0.host):\($0.port)\($0.tls ? " · TLS" : "")" } ?? ""
    case .query:
      let m = buf.member(buf.name)
      return m?.user.map { "\($0)@\(m?.host ?? "")" } ?? ""
    case .channel:
      if !buf.joined { return "not joined" }
      let n = buf.members.count
      let topic = stripFormatting(buf.topic)
      let count = n > 0 ? "\(n.formatted()) member\(n == 1 ? "" : "s")" : ""
      return [count, topic].filter { !$0.isEmpty }.joined(separator: " · ")
    }
  }

  private var showMembers: Binding<Bool> {
    Binding(
      get: { buf.kind == .channel && app.settings.showMembers },
      set: { if buf.kind == .channel { app.settings.showMembers = $0 } })
  }

  var body: some View {
    VStack(spacing: 0) {
      if topicOpen && !buf.topic.isEmpty { TopicPanel(buf: buf) }
      if conn.status != .connected && buf.kind != .server {
        Text(conn.status == .disconnected ? "Disconnected\(conn.error.map { " — " + $0 } ?? "")." : "Connecting to \(conn.name)…")
          .font(.system(size: 12))
          .foregroundStyle(conn.status == .disconnected ? p.danger : p.text3)
          .frame(maxWidth: .infinity)
          .padding(.vertical, 6)
          .background(conn.status == .disconnected ? p.danger.opacity(0.1) : p.hover)
      }
      MessageList(buf: buf, filter: filter)
      TypingBar(buf: buf)
      Composer(buf: buf)
    }
    .background(p.bg)
    .navigationTitle(title)
    .navigationSubtitle(subtitle)
    .searchable(text: $filter, placement: .toolbar, prompt: "Filter messages")
    .toolbar {
      if buf.kind == .channel && !buf.topic.isEmpty {
        ToolbarItem {
          Toggle(isOn: $topicOpen) { Label("Topic", systemImage: "text.quote") }
            .help(topicOpen ? "Hide the topic" : "Show the full topic")
        }
      }
      if buf.kind == .channel {
        ToolbarItem {
          Toggle(isOn: showMembers) { Label("Members", systemImage: "person.2") }
            .help("Members (⌘⇧M)")
        }
      }
    }
    .inspector(isPresented: showMembers) {
      MemberList(buf: buf)
        .inspectorColumnWidth(min: 180, ideal: 224, max: 340)
    }
    .environment(composer)
    .environment(\.openURL, OpenURLAction { url in
      if url.scheme == channelScheme {
        app.join(conn, url.path)
        return .handled
      }
      return .systemAction
    })
  }
}

private struct TopicPanel: View {
  let buf: Buffer
  @Environment(\.palette) private var p

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      RichText(text: buf.topic).textSelection(.enabled)
      if !buf.topicBy.isEmpty {
        Text("Set by \(buf.topicBy)\(buf.topicTime > 0 ? " · \(formatFull(buf.topicTime))" : "")")
          .font(.system(size: 11)).foregroundStyle(p.text4)
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .padding(.horizontal, 16)
    .padding(.vertical, 10)
    .background(p.raised)
    .overlay(alignment: .bottom) { Divider() }
  }
}

private struct TypingBar: View {
  let buf: Buffer
  @Environment(\.palette) private var p

  var body: some View {
    let typing = buf.typing.keys.sorted()
    HStack(spacing: 6) {
      if !typing.isEmpty {
        TypingDots()
        Group {
          switch typing.count {
          case 1: Text("**\(typing[0])** is typing")
          case 2: Text("**\(typing[0])** and **\(typing[1])** are typing")
          default: Text("\(typing.count) people are typing")
          }
        }
        .font(.system(size: 11))
        .foregroundStyle(p.text3)
      }
      Spacer()
    }
    .frame(height: 16)
    .padding(.horizontal, 16)
  }
}

// ─── Members ─────────────────────────────────────────────────────

struct MemberList: View {
  let buf: Buffer
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  @LocalState private var query = ""

  private struct Group: Identifiable {
    var id: String
    var members: [Member]
  }

  private var groups: [Group] {
    let syms = buf.conn.prefix.symbols
    let q = query.lowercased()
    var out: [Group] = []
    for m in buf.sortedMembers where q.isEmpty || m.nick.lowercased().contains(q) {
      let title: String
      switch m.prefix.first {
      case let s? where s == "+": title = "Voiced"
      case let s? where s == "%": title = "Half-operators"
      case let s? where syms.firstIndex(of: s).map({ $0 <= (syms.firstIndex(of: "@") ?? 0) }) ?? false: title = "Operators"
      case .some: title = "Members"
      case nil: title = "Members"
      }
      if out.last?.id == title {
        out[out.count - 1].members.append(m)
      } else {
        out.append(Group(id: title, members: [m]))
      }
    }
    return out
  }

  var body: some View {
    List {
      ForEach(groups) { g in
        Section("\(g.id) — \(g.members.count)") {
          ForEach(g.members) { m in MemberRow(buf: buf, member: m) }
        }
      }
    }
    .listStyle(.sidebar)
    .safeAreaInset(edge: .top, spacing: 0) {
      if buf.members.count > 12 {
        TextField("Find a member", text: $query)
          .textFieldStyle(.roundedBorder)
          .padding(.horizontal, 10)
          .padding(.vertical, 6)
      }
    }
    .overlay {
      if buf.members.isEmpty {
        Text(buf.joined ? "Loading members…" : "Not in the channel").foregroundStyle(p.text4).font(.system(size: 12))
      }
    }
  }
}

private struct MemberRow: View {
  let buf: Buffer
  let member: Member
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  @LocalState private var card = false

  var body: some View {
    HStack(spacing: 6) {
      if app.settings.loadAvatars { AvatarView(conn: buf.conn, nick: member.nick, size: 18) }
      Text(member.prefix.isEmpty ? " " : String(member.prefix.prefix(1)))
        .font(.system(size: 12, weight: .semibold, design: .monospaced))
        .foregroundStyle(p.accent)
        .frame(width: 10)
      NickText(nick: member.nick, weight: .regular, size: 13)
        .opacity(member.away ? 0.5 : 1)
      Spacer(minLength: 0)
    }
    .contentShape(Rectangle())
    .onTapGesture { card = true }
    .popover(isPresented: $card, arrowEdge: .leading) { NickCard(buf: buf, nick: member.nick) { card = false } }
    .contextMenu { NickActions(buf: buf, nick: member.nick) }
    .help(member.realname.map { stripFormatting($0) } ?? member.nick)
  }
}

/// Who someone is and what you can do about them (NickMenu.svelte).
struct NickCard: View {
  let buf: Buffer
  let nick: String
  var close: () -> Void
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    let conn = buf.conn
    let member = buf.member(nick)
    let profile = conn.profile(nick)
    let online = conn.online[conn.fold(nick)]
    let ignored = app.isIgnored(conn, Source(nick: nick, user: member?.user, host: member?.host))
    VStack(alignment: .leading, spacing: 10) {
      HStack(alignment: .top, spacing: 10) {
        if app.settings.loadAvatars && profile.avatar != nil { AvatarView(conn: conn, nick: nick, size: 40) }
        VStack(alignment: .leading, spacing: 3) {
          NickText(nick: nick, size: 15)
          if let d = profile.displayName, d != nick { Text(d).foregroundStyle(p.text2) }
          if let user = member?.user {
            Text("\(user)@\(member?.host ?? "")").font(.system(size: 11, design: .monospaced)).foregroundStyle(p.text3)
              .textSelection(.enabled)
          }
        }
      }
      let meta = [
        member.flatMap { $0.prefix.isEmpty ? nil : $0.prefix },
        member?.account.map { "signed in as \($0)" },
        member?.away == true ? "away" : nil,
        online.map { $0 ? "online" : "offline" },
        ignored ? "ignored" : nil,
      ].compactMap { $0 }
      if !meta.isEmpty { Text(meta.joined(separator: " · ")).font(.system(size: 11)).foregroundStyle(p.text3) }
      if let real = member?.realname, !real.isEmpty { RichText(text: real, plain: true, size: 12).foregroundStyle(p.text2) }
      Divider()
      VStack(alignment: .leading, spacing: 2) {
        NickActions(buf: buf, nick: nick, done: close)
      }
      .buttonStyle(CardButtonStyle())
    }
    .padding(14)
    .frame(width: 260, alignment: .leading)
  }
}

private struct CardButtonStyle: ButtonStyle {
  @Environment(\.palette) private var p
  @LocalState private var hover = false

  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding(.horizontal, 8)
      .padding(.vertical, 4)
      .background(RoundedRectangle(cornerRadius: 5).fill(hover || configuration.isPressed ? p.active : .clear))
      .contentShape(Rectangle())
      .onHover { hover = $0 }
  }
}

/// The actions for a nick, as buttons: used in context menus and the nick card.
struct NickActions: View {
  let buf: Buffer
  let nick: String
  var done: () -> Void = {}
  @Environment(AppModel.self) private var app
  @Environment(ComposerBridge.self) private var composer: ComposerBridge?

  var body: some View {
    let conn = buf.conn
    let isSelf = conn.isMe(nick)
    let member = buf.member(nick)
    let me = buf.member(conn.nick)
    let syms = conn.prefix.symbols
    let myRank = me?.prefix.first.map { conn.prefix.rank(of: $0) } ?? 99
    let opIndex = syms.firstIndex(of: "@") ?? 0
    let canModerate = buf.kind == .channel && myRank <= opIndex && myRank != 99
    let isOp = member?.prefix.contains("@") ?? false
    let isVoice = member?.prefix.contains("+") ?? false
    let src = Source(nick: nick, user: member?.user, host: member?.host)
    let act = { (f: () -> Void) in
      f()
      done()
    }
    if !isSelf {
      Button("Message") { act { app.query(conn, nick) } }
      if let composer { Button("Mention") { act { composer.insert("\(nick): ") } } }
    }
    Button("Whois") { act { conn.sendFrom(buf, "WHOIS \(nick) \(nick)") } }
    if !isSelf && conn.canMonitor {
      let watched = app.isWatched(conn, nick)
      Button(watched ? "Stop Watching" : "Tell Me When Online") {
        act { if watched { app.unwatch(conn, nick) } else { app.watch(conn, nick) } }
      }
    }
    if !isSelf {
      let ignored = app.isIgnored(conn, src)
      Button(ignored ? "Stop Ignoring" : "Ignore") { act { if ignored { _ = app.unignore(nick) } else { app.ignore(nick) } } }
    }
    if canModerate && !isSelf {
      Divider()
      Button(isOp ? "Remove Operator" : "Make Operator") { act { conn.send("MODE \(buf.name) \(isOp ? "-" : "+")o \(nick)") } }
      Button(isVoice ? "Remove Voice" : "Give Voice") { act { conn.send("MODE \(buf.name) \(isVoice ? "-" : "+")v \(nick)") } }
      Button("Kick", role: .destructive) { act { conn.send("KICK \(buf.name) \(nick)") } }
    }
  }
}

/// A draft/metadata-2 avatar fetched by the core, or the nick's initial.
struct AvatarView: View {
  let conn: Conn
  let nick: String
  var size: CGFloat
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  @LocalState private var image: NSImage?

  var body: some View {
    let url = conn.profile(nick).avatar
    ZStack {
      if let image {
        Image(nsImage: image).resizable().scaledToFill()
      } else {
        p.nick(nick).opacity(0.25)
        Text(nick.prefix(1).uppercased())
          .font(.system(size: size * 0.5, weight: .semibold))
          .foregroundStyle(p.nick(nick))
      }
    }
    .frame(width: size, height: size)
    .clipShape(RoundedRectangle(cornerRadius: size * 0.28))
    .task(id: url) {
      image = nil
      guard let url, let data = await app.avatar(url) else { return }
      image = imageFromDataURL(data)
    }
  }
}

func imageFromDataURL(_ s: String) -> NSImage? {
  guard let comma = s.firstIndex(of: ","), let data = Data(base64Encoded: String(s[s.index(after: comma)...])) else {
    return nil
  }
  return NSImage(data: data)
}
