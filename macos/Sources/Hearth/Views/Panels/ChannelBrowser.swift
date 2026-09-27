// The /list channel browser (ChannelBrowser.svelte). Big networks list tens of
// thousands of channels, so rows are prepared once (topics stripped, names
// folded) and a Table only draws what's visible.

import SwiftUI

private struct ChannelRow: Identifiable {
  let name: String
  let fold: String
  let users: Int
  let topic: String
  let nameLower: String
  let topicLower: String
  var id: String { name }
}

/// Rows built so far. The list grows by appending while it loads, so only the
/// new tail needs preparing on each update. A class so updating it doesn't
/// invalidate the view.
private final class Prepared {
  var rows: [ChannelRow] = []
  var seen: Set<String> = []
  var consumed = 0
  var lastName: String?
}

private struct ListKey: Equatable {
  var count: Int
  var at: Int
  var loading: Bool
  var exists: Bool
}

struct ChannelBrowser: View {
  var conn: Conn
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  @LocalState private var q = ""
  @LocalState private var sortOrder = [KeyPathComparator(\ChannelRow.users, order: .reverse)]
  @LocalState private var prepared = Prepared()
  @LocalState private var shown: [ChannelRow] = []
  @LocalState private var selection: String?

  private var list: ChannelList? { conn.channelList }

  private var listKey: ListKey {
    ListKey(count: list?.items.count ?? 0, at: list?.at ?? 0, loading: list?.loading ?? false, exists: list != nil)
  }

  private enum Sort: Hashable { case users, name }

  private var sort: Binding<Sort> {
    Binding(
      get: { sortOrder.first?.keyPath == \ChannelRow.fold ? .name : .users },
      set: {
        sortOrder = $0 == .users ? [KeyPathComparator(\ChannelRow.users, order: .reverse)] : [KeyPathComparator(\ChannelRow.fold)]
      })
  }

  var body: some View {
    VStack(spacing: 0) {
      header
      HStack(spacing: 10) {
        HStack(spacing: 6) {
          Image(systemName: "line.3.horizontal.decrease").foregroundStyle(p.text3)
          PanelField(
            placeholder: "Filter by name or topic", text: $q, fontSize: 13, color: p.text, onMove: move,
            onSubmit: { if let selection { join(selection) } }, onCancel: close)
        }
        .padding(.horizontal, 8)
        .frame(height: 28)
        .background(RoundedRectangle(cornerRadius: 6).fill(p.sunken))
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(p.line))
        Picker("Sort by", selection: sort) {
          Text("Most users").tag(Sort.users)
          Text("Name").tag(Sort.name)
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .fixedSize()
      }
      .padding(.horizontal, 20)
      .padding(.bottom, 12)
      table
      Divider()
      HStack {
        Text("Double-click or press ↩ to join")
          .font(.system(size: 11.5))
          .foregroundStyle(p.text3)
        Spacer()
        Button("Done", action: close).keyboardShortcut(.cancelAction)
        Button(selectionJoined ? "Open" : "Join") { if let selection { join(selection) } }
          .keyboardShortcut(.defaultAction)
          .disabled(selection == nil)
      }
      .padding(.horizontal, 20)
      .padding(.vertical, 12)
    }
    .frame(width: 760, height: 560)
    .background(p.bg)
    .onAppear { app.browseChannels(conn) }
    .onChange(of: listKey, initial: true) { rebuild() }
    .onChange(of: q) { refilter(keepSelection: false) }
    .onChange(of: sortOrder) { refilter(keepSelection: true) }
    .onExitCommand(perform: close)
  }

  private var header: some View {
    HStack(alignment: .top, spacing: 8) {
      VStack(alignment: .leading, spacing: 2) {
        Text("Channels on \(conn.name)")
          .font(.system(size: 18, weight: .semibold))
          .foregroundStyle(p.text)
        HStack(spacing: 6) {
          if list?.loading == true { ProgressView().controlSize(.mini) }
          Text(status)
        }
        .font(.system(size: 12.5))
        .foregroundStyle(p.text3)
      }
      Spacer()
      Button {
        app.refreshChannelList(conn)
      } label: {
        Label("Refresh", systemImage: "arrow.clockwise")
      }
      .help("Fetch the channel list again")
      .keyboardShortcut("r")
      .disabled(list?.loading == true || conn.status != .connected)
    }
    .padding(.horizontal, 20)
    .padding(.top, 18)
    .padding(.bottom, 12)
  }

  private var status: String {
    guard let list else { return conn.status == .connected ? "" : "Not connected" }
    let total = prepared.rows.count
    if list.loading { return total > 0 ? "Loading… \(total.formatted()) so far" : "Loading…" }
    let match = q.trimmingCharacters(in: .whitespaces).isEmpty ? "" : " · \(shown.count.formatted()) match"
    return "\(total.formatted()) channel\(total == 1 ? "" : "s")\(match)"
  }

  private var table: some View {
    ScrollViewReader { proxy in
      Table(shown, selection: $selection, sortOrder: $sortOrder) {
        TableColumn("Channel", value: \ChannelRow.fold) { r in
          HStack(spacing: 5) {
            Text(r.name).fontWeight(.semibold).lineLimit(1)
            if conn.buffer(r.name)?.joined == true {
              Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 10))
                .foregroundStyle(p.accent)
                .help("Joined")
            }
          }
        }
        .width(min: 120, ideal: 190, max: 320)
        TableColumn("Users", value: \ChannelRow.users) { r in
          Text(r.users.formatted())
            .monospacedDigit()
            .foregroundStyle(p.text2)
            .frame(maxWidth: .infinity, alignment: .trailing)
        }
        .width(min: 48, ideal: 64, max: 96)
        TableColumn("Topic") { r in
          Text(r.topic).foregroundStyle(p.text2).lineLimit(1).help(r.topic)
        }
      }
      .contextMenu(forSelectionType: String.self) { ids in
        if let id = ids.first {
          Button(conn.buffer(id)?.joined == true ? "Open \(id)" : "Join \(id)") { join(id) }
        }
      } primaryAction: { ids in
        if let id = ids.first { join(id) }
      }
      .overlay {
        if shown.isEmpty && list?.loading != true {
          Text(empty).foregroundStyle(p.text3)
        }
      }
      .onChange(of: selection) { _, id in if let id { proxy.scrollTo(id) } }
    }
  }

  private var empty: String {
    if list == nil { return conn.status == .connected ? "No channels listed" : "Connect to see the channel list" }
    return q.trimmingCharacters(in: .whitespaces).isEmpty ? "No channels listed" : "No channels match"
  }

  private var selectionJoined: Bool { selection.flatMap { conn.buffer($0)?.joined } ?? false }

  private func rebuild() {
    let items = list?.items ?? []
    let pr = prepared
    // While loading, updates only append. A finished list is prepared afresh
    // once, since a refresh can change users and topics under the same names.
    let extends =
      list?.loading == true && pr.consumed <= items.count
      && (pr.consumed == 0 || items[pr.consumed - 1].name == pr.lastName)
    if !extends {
      pr.rows = []
      pr.seen = []
      pr.consumed = 0
    }
    for c in items[pr.consumed...] where pr.seen.insert(c.name).inserted {
      let topic = stripFormatting(c.topic)
      pr.rows.append(
        ChannelRow(
          name: c.name, fold: conn.fold(c.name), users: c.users, topic: topic, nameLower: c.name.lowercased(),
          topicLower: topic.lowercased()))
    }
    pr.consumed = items.count
    pr.lastName = items.last?.name
    refilter(keepSelection: true)
  }

  private func refilter(keepSelection: Bool) {
    let needle = q.trimmingCharacters(in: .whitespaces).lowercased()
    var rows =
      needle.isEmpty
      ? prepared.rows : prepared.rows.filter { $0.nameLower.contains(needle) || $0.topicLower.contains(needle) }
    let c = sortOrder.first
    let asc = c?.order == .forward
    if c?.keyPath == \ChannelRow.fold {
      rows.sort { asc ? $0.fold < $1.fold : $0.fold > $1.fold }
    } else {
      rows.sort { a, b in a.users != b.users ? (asc ? a.users < b.users : a.users > b.users) : a.fold < b.fold }
    }
    shown = rows
    // Keep ↩ useful from the filter field: the top match is always picked.
    if !keepSelection || selection.map({ id in !rows.contains { $0.id == id } }) ?? true {
      selection = rows.first?.id
    }
  }

  private func move(_ delta: Int) {
    guard !shown.isEmpty else { return }
    let i = selection.flatMap { id in shown.firstIndex { $0.id == id } } ?? -1
    selection = shown[min(max(i + delta, 0), shown.count - 1)].id
  }

  private func join(_ name: String) {
    app.join(conn, name)
    close()
  }

  private func close() { app.browserConn = nil }
}

extension AppModel {
  /// Fetch the channel list again even if the last one is recent.
  func refreshChannelList(_ conn: Conn) {
    guard conn.status == .connected, conn.channelList?.loading != true else { return }
    conn.channelList = ChannelList(loading: true, items: conn.channelList?.items ?? [], at: nowMs())
    conn.send("LIST")
  }
}
