// A buffer's scrollback (MessageList.svelte): sticks to the newest line while
// you're at the bottom, keeps its place while history loads above, and
// scrolls to lines that search or reply quotes ask for.

import SwiftUI

struct MessageList: View {
  let buf: Buffer
  let filter: String
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  @LocalState private var pinned = true
  @LocalState private var expanded: Set<String> = []
  @LocalState private var flash: Int?
  @LocalState private var topID: String?
  @LocalState private var resize = ResizeMemo()

  /// When the viewport shrinks (the composer grows, a banner appears) the
  /// bottom sentinel scrolls out of view without the reader scrolling away.
  /// Either event can arrive first, so each checks for the other.
  private final class ResizeMemo {
    var resizedAt = Date.distantPast
    var unpinnedAt = Date.distantPast
    func recent(_ d: Date) -> Bool { Date().timeIntervalSince(d) < 0.3 }
  }

  private var items: [TimelineItem] {
    var lines = buf.messages
    if app.settings.membership == .hide && buf.kind == .channel { lines = lines.filter { !$0.type.isMembership } }
    let q = filter.trimmingCharacters(in: .whitespaces).lowercased()
    if !q.isEmpty { lines = lines.filter { ($0.nick + " " + stripFormatting($0.text)).lowercased().contains(q) } }
    return buildTimeline(lines, divider: buf.divider, channel: buf.kind == .channel, membership: app.settings.membership)
  }

  var body: some View {
    let items = self.items
    ScrollViewReader { proxy in
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 0) {
          HistoryHeader(buf: buf, filtering: !filter.isEmpty)
          ForEach(items) { item in
            ItemView(
              item: item, buf: buf, flash: flash,
              expanded: expanded.contains(item.id),
              toggle: {
                if expanded.contains(item.id) { expanded.remove(item.id) } else { expanded.insert(item.id) }
              }
            )
            .id(item.id)
          }
          Color.clear.frame(height: 6).id(bottomID)
            .onAppear { pinned = true }
            .onDisappear {
              if resize.recent(resize.resizedAt) {
                proxy.scrollTo(bottomID, anchor: .bottom)
              } else {
                pinned = false
                resize.unpinnedAt = Date()
              }
            }
        }
        .scrollTargetLayout()
      }
      .scrollPosition(id: $topID, anchor: .top)
      .onGeometryChange(for: CGFloat.self, of: \.size.height) { _ in
        resize.resizedAt = Date()
        if pinned || resize.recent(resize.unpinnedAt) {
          pinned = true
          proxy.scrollTo(bottomID, anchor: .bottom)
        }
      }
      .defaultScrollAnchor(.bottom)
      .onChange(of: items.last?.id) {
        if pinned { proxy.scrollTo(bottomID, anchor: .bottom) }
        markRead()
      }
      .onChange(of: pinned) { markRead() }
      .onChange(of: app.focused) { markRead() }
      .onChange(of: buf.focus) {
        guard let f = buf.focus else { return }
        buf.focus = nil
        let line = buf.messages.first { f.msgid != nil ? $0.msgid == f.msgid : $0.time == f.time }
        guard let line, let item = items.first(where: { $0.contains(line.id) }) else { return }
        proxy.scrollTo(item.id, anchor: .center)
        flash = line.id
        Task {
          try? await Task.sleep(for: .seconds(1.8))
          if flash == line.id { flash = nil }
        }
      }
      .onAppear {
        proxy.scrollTo(bottomID, anchor: .bottom)
        markRead()
      }
      .overlay(alignment: .bottomTrailing) {
        if !pinned {
          Button {
            proxy.scrollTo(bottomID, anchor: .bottom)
          } label: {
            Label(buf.unread > 0 ? "\(buf.unread) new" : "Latest", systemImage: "arrow.down")
              .font(.system(size: 12, weight: .medium))
          }
          .buttonStyle(.borderedProminent)
          .controlSize(.small)
          .padding(14)
          .transition(.opacity)
        }
      }
    }
  }

  private let bottomID = "bottom"

  /// Mark as read while the newest messages are on screen.
  private func markRead() {
    if pinned && app.focused && filter.isEmpty { app.markRead(buf) }
  }
}

/// Loads older history when scrolled into view, and says when there is no more.
private struct HistoryHeader: View {
  let buf: Buffer
  let filtering: Bool
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    Group {
      switch buf.history {
      case .loading:
        ProgressView().controlSize(.small)
      case .done where buf.kind != .server:
        Text(buf.messages.isEmpty ? "No messages yet" : "Beginning of the conversation")
          .font(.system(size: 11))
          .foregroundStyle(p.text4)
      default:
        Color.clear
      }
    }
    .frame(maxWidth: .infinity, minHeight: 28)
    .onAppear {
      if buf.kind != .server && buf.history == .idle && !filtering {
        Task { await app.fetchHistory(buf, .before) }
      }
    }
  }
}

private struct ItemView: View {
  let item: TimelineItem
  let buf: Buffer
  let flash: Int?
  let expanded: Bool
  let toggle: () -> Void
  @Environment(\.palette) private var p

  var body: some View {
    switch item {
    case .day(_, _, let label):
      HStack(spacing: 10) {
        Rectangle().fill(p.line).frame(height: 1)
        Text(label).font(.system(size: 11, weight: .semibold)).foregroundStyle(p.text3)
        Rectangle().fill(p.line).frame(height: 1)
      }
      .padding(.horizontal, 16)
      .padding(.top, 14)
      .padding(.bottom, 6)
    case .unread:
      HStack(spacing: 8) {
        Rectangle().fill(p.accent.opacity(0.6)).frame(height: 1)
        Text("New").font(.system(size: 10, weight: .bold)).foregroundStyle(p.accent)
      }
      .padding(.horizontal, 16)
      .padding(.vertical, 4)
    case .line(let l, let cont):
      if l.type == .whois {
        WhoisCard(line: l, buf: buf)
      } else if let motd = l.data?["motd"] {
        MotdBlock(line: l, text: motd, title: l.data?["title"])
      } else if l.type.isTalk {
        MessageRow(line: l, cont: cont, buf: buf, flash: flash == l.id)
      } else {
        EventRow(line: l, buf: buf, flash: flash == l.id)
      }
    case .run(let lines):
      EventRun(lines: lines, buf: buf, open: expanded, toggle: toggle)
    }
  }
}
