// Message search (SearchPanel.svelte): the local log plus soju.im/search,
// newest first. Choosing a hit opens its buffer and scrolls to the message.

import SwiftUI

private enum SearchScope: Hashable { case all, network, buffer }

private struct SearchKey: Equatable {
  var query: String
  var scope: SearchScope
  var ready: Bool
}

struct SearchPanel: View {
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  /// The buffer that was open when search started; scopes are relative to it.
  @LocalState private var origin: Buffer?
  @LocalState private var ready = false
  @LocalState private var q = ""
  @LocalState private var scope = SearchScope.all
  @LocalState private var hits: [Hit] = []
  @LocalState private var busy = false
  @LocalState private var sel = 0

  private struct Hit: Identifiable {
    let id: Int
    let conn: Conn?
    /// The buffer name, in its original case when we can tell.
    let name: String
    let time: Int
    let msgid: String?
    let nick: String
    let text: String
    let action: Bool
  }

  private var canSearch: Bool { app.settings.storeLogs || app.conns.values.contains { $0.has("soju.im/search") } }

  var body: some View {
    VStack(spacing: 0) {
      HStack(spacing: 10) {
        Image(systemName: "magnifyingglass").foregroundStyle(p.text3)
        PanelField(
          placeholder: "Search messages", text: $q, color: p.text, onMove: move, onSubmit: { choose(sel) },
          onCancel: close)
        if let origin {
          Picker("Search in", selection: $scope) {
            Text("Everywhere").tag(SearchScope.all)
            Text(origin.conn.name).tag(SearchScope.network)
            if origin.kind != .server { Text(origin.name).tag(SearchScope.buffer) }
          }
          .pickerStyle(.segmented)
          .labelsHidden()
          .fixedSize()
        }
      }
      .padding(.horizontal, 16)
      .frame(height: 52)
      Divider()
      results
      Divider()
      HStack(spacing: 14) {
        PanelHint(keys: "↑↓", label: "navigate")
        PanelHint(keys: "↩", label: "jump to message")
        PanelHint(keys: "esc", label: "close")
        Spacer()
        if busy {
          ProgressView().controlSize(.mini)
          Text("Searching…")
        } else if !hits.isEmpty {
          Text("\(hits.count) result\(hits.count == 1 ? "" : "s")")
        }
      }
      .font(.system(size: 11.5))
      .foregroundStyle(p.text3)
      .padding(.horizontal, 14)
      .frame(height: 32)
    }
    .frame(width: 680, height: 520)
    .background(p.raised)
    .onAppear {
      guard !ready else { return }
      origin = app.active
      q = app.searchQuery
      ready = true
    }
    .task(id: SearchKey(query: q, scope: scope, ready: ready)) { await run() }
    .onExitCommand(perform: close)
  }

  @ViewBuilder
  private var results: some View {
    if !canSearch {
      VStack(alignment: .leading, spacing: 8) {
        Text("Search uses messages saved on this computer, which is turned off.")
        Button("Change in Settings…") {
          close()
          app.settingsRoute = SettingsRoute(tab: .privacy, serverId: nil)
          openSettings()
        }
        .buttonStyle(.link)
      }
      .font(.system(size: 13))
      .foregroundStyle(p.text3)
      .padding(18)
      .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    } else {
      ScrollViewReader { proxy in
        ScrollView {
          LazyVStack(spacing: 0) {
            ForEach(hits) { h in
              row(h, selected: h.id == sel)
                .contentShape(Rectangle())
                .onTapGesture { choose(h.id) }
                .onHover { if $0 { sel = h.id } }
            }
            if hits.isEmpty {
              Text(q.trimmingCharacters(in: .whitespaces).isEmpty ? "" : busy ? "" : "No messages found")
                .font(.system(size: 13))
                .foregroundStyle(p.text3)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
            }
          }
          .padding(6)
        }
        .onChange(of: sel) { _, i in proxy.scrollTo(i) }
      }
    }
  }

  private func row(_ h: Hit, selected: Bool) -> some View {
    VStack(alignment: .leading, spacing: 3) {
      HStack(alignment: .firstTextBaseline, spacing: 8) {
        Text(h.name).fontWeight(.semibold).foregroundStyle(p.text2)
        if let c = h.conn { Text(c.name) }
        Spacer(minLength: 8)
        Text(h.time > 0 ? date(h.time).formatted(date: .abbreviated, time: .shortened) : "")
          .monospacedDigit()
      }
      .font(.system(size: 11.5))
      .foregroundStyle(p.text3)
      .lineLimit(1)
      HStack(alignment: .firstTextBaseline, spacing: 8) {
        NickText(nick: h.action ? "* \(h.nick)" : h.nick, size: 13.5)
          .lineLimit(1)
          .fixedSize()
        Text(marked(h.text))
          .foregroundStyle(p.text2)
          .lineLimit(2)
          .truncationMode(.tail)
          .frame(maxWidth: .infinity, alignment: .leading)
      }
    }
    .padding(.horizontal, 10)
    .padding(.vertical, 7)
    .background(RoundedRectangle(cornerRadius: 7).fill(selected ? p.active : .clear))
    .opacity(h.conn == nil ? 0.5 : 1)
    .accessibilityElement(children: .combine)
    .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
  }

  /// The query's words, as the Svelte version picks them out for highlighting.
  private var words: [String] {
    q.split(whereSeparator: \.isWhitespace)
      .map { $0.filter { $0.isLetter || $0.isNumber || $0 == "#" || $0 == "@" } }
      .filter { !$0.isEmpty }
  }

  /// Formatted text with the query's words marked.
  private func marked(_ text: String) -> AttributedString {
    var a = ircAttributed(text, palette: p, size: 13.5, plain: true)
    for w in words {
      var from = a.startIndex
      while from < a.endIndex, let r = a[from...].range(of: w, options: [.caseInsensitive, .diacriticInsensitive]) {
        a[r].backgroundColor = p.accentSoft
        a[r].foregroundColor = p.text
        from = r.upperBound
      }
    }
    return a
  }

  private func run() async {
    guard ready else { return }
    let query = q
    if query.trimmingCharacters(in: .whitespaces).isEmpty {
      hits = []
      busy = false
      return
    }
    busy = true
    try? await Task.sleep(for: .milliseconds(180))
    if Task.isCancelled { return }
    let conn = scope == .all ? nil : origin?.conn
    let buffer = scope == .buffer ? origin : nil
    let res = await app.search(query, conn: conn, buffer: buffer)
    if Task.isCancelled { return }
    hits = res.enumerated().map { prepare($1, id: $0) }
    sel = 0
    busy = false
  }

  private func prepare(_ h: SearchHit, id: Int) -> Hit {
    let conn = app.conns[h.network]
    let to = h.msg.param(0) ?? h.target
    let nick = h.msg.source?.nick ?? ""
    var name = to
    if let conn {
      let peer = conn.isChannel(to) || nick.isEmpty || conn.isMe(nick) ? to : nick
      name = conn.fold(peer) == h.target ? peer : h.target
    }
    let raw = h.msg.param(1) ?? ""
    var text = parseCtcp(raw)?.args ?? raw
    var action = parseCtcp(raw)?.command == "ACTION"
    if let conn, let line = app.historyLine(conn, h.msg) {
      text = line.text
      action = line.type == .action
    }
    return Hit(
      id: id, conn: conn, name: name, time: h.msg.tags["time"].flatMap(parseIso) ?? 0, msgid: h.msg.tags["msgid"],
      nick: nick, text: text, action: action)
  }

  private func move(_ delta: Int) {
    guard !hits.isEmpty else { return }
    sel = min(max(sel + delta, 0), hits.count - 1)
  }

  private func choose(_ i: Int) {
    guard i < hits.count else { return }
    let h = hits[i]
    guard let conn = h.conn else {
      app.toast("That network is not connected", .error)
      return
    }
    let buf = conn.buffer(h.name) ?? conn.ensure(h.name)
    close()
    Task { await app.jumpTo(buf, time: h.time, msgid: h.msgid) }
  }

  private func close() {
    app.searchOpen = false
    app.searchQuery = ""
  }
}
