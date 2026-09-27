// Networks and their buffers (Sidebar.svelte), as a native source list.

import SwiftUI

struct SidebarView: View {
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  private var selection: Binding<String?> {
    Binding(
      get: { app.active?.id },
      set: { id in
        guard let id, let buf = app.allBuffers.first(where: { $0.id == id }) else { return }
        app.open(buf)
      })
  }

  var body: some View {
    List(selection: selection) {
      ForEach(app.orderedConns) { conn in
        ConnSection(conn: conn)
      }
    }
    .listStyle(.sidebar)
    .safeAreaInset(edge: .top, spacing: 0) {
      Button {
        app.switcherOpen = true
      } label: {
        HStack(spacing: 6) {
          Image(systemName: "magnifyingglass")
          Text("Jump to…")
          Spacer()
          Text("⌘K").font(.system(size: 11)).foregroundStyle(p.text4)
        }
        .foregroundStyle(p.text3)
        .padding(.horizontal, 9)
        .padding(.vertical, 5)
        .background(RoundedRectangle(cornerRadius: 7).fill(p.hover))
        .overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(p.line))
        .contentShape(Rectangle())
      }
      .buttonStyle(.plain)
      .help("Jump to a buffer (⌘K)")
      .padding(.horizontal, 10)
      .padding(.bottom, 6)
    }
    .safeAreaInset(edge: .bottom, spacing: 0) { SidebarFooter() }
    .toolbar {
      ToolbarItem {
        Button {
          app.searchOpen = true
        } label: {
          Label("Search Messages", systemImage: "text.magnifyingglass")
        }
        .help("Search messages (⌘⇧F)")
      }
    }
  }
}

private struct ConnSection: View {
  let conn: Conn
  @Environment(AppModel.self) private var app

  var body: some View {
    if conn.isControl {
      // A bouncer's own connection is a console; its networks follow as their own rows.
      ConnRow(conn: conn).tag(conn.server.id)
      ForEach(conn.channelBuffers) { b in BufferRow(buf: b).tag(b.id) }
    } else {
      DisclosureGroup(isExpanded: Binding(get: { !conn.collapsed }, set: { conn.collapsed = !$0 })) {
        ForEach(conn.channelBuffers) { b in BufferRow(buf: b).tag(b.id) }
      } label: {
        ConnRow(conn: conn).tag(conn.server.id)
      }
    }
  }
}

/// "retrying in 12s", ticking.
private struct StatusLabel: View {
  let conn: Conn
  @Environment(\.palette) private var p

  var body: some View {
    TimelineView(.periodic(from: .now, by: 1)) { ctx in
      if let text = label(now: ctx.date) {
        Text(text).font(.system(size: 11)).foregroundStyle(conn.status == .disconnected ? p.danger : p.text4)
      }
    }
  }

  private func label(now: Date) -> String? {
    switch conn.status {
    case .connected: return nil
    case .disconnected:
      if conn.retryAt > 0 {
        return "retrying in \(max(0, Int((Double(conn.retryAt) / 1000 - now.timeIntervalSince1970).rounded())))s"
      }
      return "offline"
    default: return "connecting"
    }
  }
}

struct StatusDot: View {
  let status: ConnStatus
  var away = false
  @Environment(\.palette) private var p

  var body: some View {
    Circle()
      .fill(away ? p.warn : status == .connected ? p.ok : status == .disconnected ? p.danger : p.text4)
      .frame(width: 7, height: 7)
  }
}

private struct ConnRow: View {
  let conn: Conn
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    HStack(spacing: 7) {
      if conn.isControl {
        Text(conn.name.uppercased()).font(.system(size: 10.5, weight: .semibold)).tracking(0.6).foregroundStyle(p.text3)
      } else {
        StatusDot(status: conn.status)
        Text(conn.name).font(.system(size: 13, weight: .semibold)).lineLimit(1)
      }
      StatusLabel(conn: conn)
      Spacer(minLength: 0)
      if conn.collapsed, case let hl = conn.channelBuffers.reduce(0, { $0 + $1.highlights }), hl > 0 {
        Badge(count: hl, strong: true)
      }
    }
    .help(conn.isControl ? "Bouncer console" : conn.name)
    .contextMenu { ConnMenu(conn: conn) }
  }
}

struct ConnMenu: View {
  let conn: Conn
  @Environment(AppModel.self) private var app

  var body: some View {
    Button("Open Server Log") { app.open(conn.server) }
    if conn.isControl {
      Button("Manage Networks…") { app.settingsRoute = SettingsRoute(tab: .servers, serverId: conn.serverId) }
    } else if conn.status == .connected {
      Button("Browse Channels…") { app.browseChannels(conn) }
    }
    Button("Reconnect") { app.reconnectServer(conn.serverId) }
    if conn.status == .disconnected && conn.retryAt == 0 {
      Button("Connect") { app.connectServer(conn.serverId) }
    } else {
      Button("Disconnect") { app.disconnectServer(conn.serverId) }
    }
    Divider()
    Button("Edit Server…") { app.settingsRoute = SettingsRoute(tab: .servers, serverId: conn.serverId) }
  }
}

private struct BufferRow: View {
  let buf: Buffer
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    let conn = buf.conn
    let unread = buf.unread > 0
    let parted = buf.kind == .channel && !buf.joined
    HStack(spacing: 6) {
      ZStack(alignment: .bottomTrailing) {
        if buf.kind == .channel {
          Text("#").font(.system(size: 13, weight: .medium)).foregroundStyle(p.text4)
        } else {
          Image(systemName: "person").font(.system(size: 11)).foregroundStyle(p.text4)
        }
        if buf.kind == .query, let online = conn.online[conn.fold(buf.name)] {
          Circle().fill(online ? p.ok : p.text4).frame(width: 6, height: 6).offset(x: 3, y: 1)
        }
      }
      .frame(width: 14)
      Text(buf.kind == .channel ? String(buf.name.drop { $0 == "#" }) : buf.name)
        .font(.system(size: 13, weight: unread ? .semibold : .regular))
        .foregroundStyle(parted ? p.text4 : unread ? p.text : p.text2)
        .lineLimit(1)
      Spacer(minLength: 0)
      if buf.level == NotifyLevel.none {
        Image(systemName: "bell.slash").font(.system(size: 11)).foregroundStyle(p.text4).help("Muted")
      } else if buf.highlights > 0 {
        Badge(count: buf.highlights, strong: true)
      } else if buf.kind == .query && !buf.typing.isEmpty {
        TypingDots()
      }
    }
    .contextMenu { BufferMenu(buf: buf) }
  }
}

struct BufferMenu: View {
  let buf: Buffer
  @Environment(AppModel.self) private var app

  var body: some View {
    let conn = buf.conn
    Button("Mark as Read") { app.markRead(buf) }
    if buf.kind == .channel && !buf.joined {
      Button("Rejoin") { app.join(conn, buf.name) }
    }
    if buf.kind == .query && conn.canMonitor {
      let watched = app.isWatched(conn, buf.name)
      Button(watched ? "Stop Watching" : "Tell Me When Online") {
        if watched { app.unwatch(conn, buf.name) } else { app.watch(conn, buf.name) }
      }
    }
    Divider()
    Picker(
      "Notify Me About",
      selection: Binding(get: { buf.level }, set: { app.setNotify(buf, $0) })
    ) {
      Text("Default").tag(NotifyLevel?.none)
      Text("All Messages").tag(NotifyLevel?.some(.all))
      Text("Mentions Only").tag(NotifyLevel?.some(.mentions))
      Text("Nothing (Mute)").tag(NotifyLevel?.some(.none))
    }
    Divider()
    Button(buf.kind == .channel && buf.joined ? "Leave Channel" : "Close") { app.closeBuffer(buf) }
  }
}

struct TypingDots: View {
  @Environment(\.palette) private var p

  var body: some View {
    TimelineView(.animation(minimumInterval: 0.2)) { ctx in
      let phase = Int(ctx.date.timeIntervalSinceReferenceDate * 3) % 3
      HStack(spacing: 2) {
        ForEach(0..<3) { i in
          Circle().fill(p.text3).frame(width: 4, height: 4).opacity(i == phase ? 1 : 0.35)
        }
      }
    }
    .accessibilityLabel("typing")
  }
}

private struct SidebarFooter: View {
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    HStack(spacing: 8) {
      Button {
        app.settingsRoute = SettingsRoute(tab: .servers, serverId: nil)
      } label: {
        Image(systemName: "plus")
      }
      .buttonStyle(.borderless)
      .help("Add network")
      if let c = app.active?.conn {
        StatusDot(status: c.status, away: c.away)
        Text(c.nick.isEmpty ? "—" : c.nick).font(.system(size: 12, weight: .medium)).lineLimit(1)
        if c.away { Text("away").font(.system(size: 11)).foregroundStyle(p.text4) }
      }
      Spacer()
      SettingsLink {
        Image(systemName: "gearshape")
      }
      .buttonStyle(.borderless)
      .help("Settings (⌘,)")
    }
    .foregroundStyle(p.text2)
    .padding(.horizontal, 12)
    .padding(.vertical, 8)
    .overlay(alignment: .top) { Divider() }
  }
}
