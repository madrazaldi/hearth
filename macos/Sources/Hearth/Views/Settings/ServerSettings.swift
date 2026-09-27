import SwiftUI

/// Servers: a list on the left, the selected server's form on the right.
struct ServerSettings: View {
  @Environment(AppModel.self) private var app
  @Binding var selection: ServerSelection?
  /// The open form has edits that switching away would lose.
  @LocalState private var dirty = false
  @LocalState private var pendingSelection: ServerSelection??
  @LocalState private var removing: ServerConfig?

  var body: some View {
    HStack(spacing: 0) {
      sidebar.frame(width: 210)
      Divider()
      detail.frame(maxWidth: .infinity, maxHeight: .infinity)
    }
    .frame(width: 800, height: 620)
    .onAppear {
      if selection == nil, let first = app.servers.first { selection = .server(first.id) }
    }
    .confirmationDialog(
      "Remove \(removing?.settingsName ?? "server")?",
      isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }), presenting: removing
    ) { server in
      Button("Remove", role: .destructive) { remove(server) }
    } message: { server in
      Text("Hearth disconnects from \(server.settingsName) and forgets its settings and saved password.")
    }
    .alert(
      "Discard unsaved changes?",
      isPresented: Binding(get: { pendingSelection != nil }, set: { if !$0 { pendingSelection = nil } })
    ) {
      Button("Discard", role: .destructive) {
        if let next = pendingSelection {
          dirty = false
          selection = next
        }
        pendingSelection = nil
      }
      Button("Keep Editing", role: .cancel) { pendingSelection = nil }
    } message: {
      Text("Your edits to this server haven't been saved.")
    }
  }

  /// Selection that asks before throwing away edits.
  private var guardedSelection: Binding<ServerSelection?> {
    Binding(
      get: { selection },
      set: { next in
        if next == selection { return }
        if dirty { pendingSelection = .some(next) } else { selection = next }
      })
  }

  private var sidebar: some View {
    List(selection: guardedSelection) {
      ForEach(app.servers) { s in
        ServerRow(server: s, conn: app.conns[s.id])
          .tag(ServerSelection.server(s.id))
          .contextMenu { serverMenu(s) }
      }
      if selection == .new {
        Label("New Server", systemImage: "plus.circle")
          .foregroundStyle(.secondary)
          .tag(ServerSelection.new)
      }
    }
    .listStyle(.sidebar)
    .safeAreaInset(edge: .bottom, spacing: 0) {
      VStack(spacing: 0) {
        Divider()
        HStack(spacing: 2) {
          Button {
            guardedSelection.wrappedValue = .new
          } label: {
            Image(systemName: "plus").frame(width: 22, height: 20)
          }
          .help("Add a server")
          .accessibilityLabel("Add server")
          Button {
            if case let .server(id) = selection { removing = app.servers.first { $0.id == id } }
          } label: {
            Image(systemName: "minus").frame(width: 22, height: 20)
          }
          .disabled(selectedServer == nil)
          .help("Remove the selected server")
          .accessibilityLabel("Remove server")
          Spacer()
        }
        .buttonStyle(.borderless)
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
      }
      .background(.bar)
    }
  }

  @ViewBuilder
  private func serverMenu(_ s: ServerConfig) -> some View {
    let status = app.conns[s.id]?.status ?? .disconnected
    if status == .disconnected {
      Button("Connect") { app.connectServer(s.id) }
    } else {
      Button("Disconnect") { app.disconnectServer(s.id) }
    }
    Button("Reconnect") { app.reconnectServer(s.id) }
    Divider()
    Button("Remove…", role: .destructive) { removing = s }
  }

  private var selectedServer: ServerConfig? {
    guard case let .server(id) = selection else { return nil }
    return app.servers.first { $0.id == id }
  }

  @ViewBuilder
  private var detail: some View {
    switch selection {
    case .new:
      ServerEditor(original: nil, dirty: $dirty, onSaved: saved, onCancel: cancelNew, onRemove: {})
        .id("\u{0}new")
    case .server:
      if let s = selectedServer {
        ServerEditor(original: s, dirty: $dirty, onSaved: saved, onCancel: {}, onRemove: { removing = s })
          .id(s.id)
      } else {
        placeholder
      }
    case nil:
      placeholder
    }
  }

  private var placeholder: some View {
    VStack(spacing: 12) {
      Image(systemName: "server.rack").font(.system(size: 36, weight: .light)).foregroundStyle(.tertiary)
      Text(app.servers.isEmpty ? "No servers yet" : "Select a server").font(.title3).foregroundStyle(.secondary)
      Text("Networks and bouncers Hearth connects to. A soju bouncer expands into all of its networks automatically.")
        .multilineTextAlignment(.center)
        .foregroundStyle(.secondary)
        .frame(maxWidth: 360)
      Button("Add Server…") { selection = .new }
    }
    .padding()
  }

  private func saved(_ s: ServerConfig) {
    dirty = false
    selection = .server(s.id)
  }

  private func cancelNew() {
    dirty = false
    selection = app.servers.first.map { .server($0.id) }
  }

  private func remove(_ s: ServerConfig) {
    removing = nil
    Task {
      do {
        try await app.deleteServer(s.id)
        if selection == .server(s.id) {
          dirty = false
          selection = app.servers.first.map { .server($0.id) }
        }
      } catch {
        app.toast("Couldn't remove \(s.settingsName): \(error)", .error)
      }
    }
  }
}

extension ServerConfig {
  var settingsName: String { name.isEmpty ? host : name }

  /// The address as typed into a client: `wss://host:443/path`, `host:6697`.
  var settingsAddress: String {
    let scheme = websocket ? (tls ? "wss://" : "ws://") : ""
    return "\(scheme)\(host):\(port)\(websocket ? websocketPath : "")"
  }
}

private struct ServerRow: View {
  var server: ServerConfig
  var conn: Conn?

  var body: some View {
    HStack(spacing: 8) {
      SettingsStatusDot(state: .init(conn))
      VStack(alignment: .leading, spacing: 1) {
        Text(server.settingsName).lineLimit(1)
        Text(server.settingsAddress)
          .font(.caption.monospaced())
          .foregroundStyle(.secondary)
          .lineLimit(1)
          .truncationMode(.middle)
      }
    }
    .padding(.vertical, 2)
    .accessibilityElement(children: .combine)
    .accessibilityValue(conn.map { $0.status.rawValue } ?? "offline")
  }
}
