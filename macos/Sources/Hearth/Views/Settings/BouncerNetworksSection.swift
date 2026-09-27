import SwiftUI

/// Networks on a soju bouncer (BouncerNetworks.svelte). Edits go straight to
/// the bouncer, which answers with BOUNCER NETWORK updates to `conn.networks`.
struct BouncerNetworksSection: View {
  let conn: Conn
  @LocalState private var editing: BouncerNetworkDraft?
  @LocalState private var removing: String?

  private var networks: [(id: String, attrs: [String: String])] {
    conn.networks.map { ($0.key, $0.value) }.sorted {
      ($0.attrs["name"] ?? $0.id).localizedStandardCompare($1.attrs["name"] ?? $1.id) == .orderedAscending
    }
  }

  var body: some View {
    Section {
      if networks.isEmpty {
        SettingsNote("No networks yet.")
      }
      ForEach(networks, id: \.id) { net in
        BouncerNetworkRow(
          netid: net.id, attrs: net.attrs,
          onEdit: { editing = BouncerNetworkDraft(netid: net.id, attrs: net.attrs) },
          onRemove: { removing = net.id })
      }
    } header: {
      HStack {
        Text("Networks on this bouncer")
        Spacer()
        Button("Add Network…") { editing = BouncerNetworkDraft() }
          .controlSize(.small)
          .disabled(conn.status != .connected)
      }
    }
    .sheet(item: $editing) { draft in
      BouncerNetworkSheet(draft: draft) { saved in
        conn.saveBouncerNetwork(saved)
        editing = nil
      } onCancel: {
        editing = nil
      }
    }
    .confirmationDialog(
      "Remove \(removing.map(name) ?? "network") from the bouncer?",
      isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }), presenting: removing
    ) { netid in
      Button("Remove", role: .destructive) {
        conn.deleteBouncerNetwork(netid)
        removing = nil
      }
    } message: { _ in
      Text("The bouncer disconnects from it and deletes its settings and history.")
    }
  }

  private func name(_ netid: String) -> String {
    let a = conn.networks[netid] ?? [:]
    return [a["name"], a["host"]].compactMap { $0 }.first { !$0.isEmpty } ?? netid
  }
}

private struct BouncerNetworkRow: View {
  let netid: String
  let attrs: [String: String]
  var onEdit: () -> Void
  var onRemove: () -> Void

  private var state: SettingsStatusDot.Kind {
    switch attrs["state"] {
    case "connected": .ok
    case "connecting": .busy
    default: attrs["error"].map { $0.isEmpty ? .off : .failed } ?? .off
    }
  }

  private var detail: String {
    var s = attrs["host"] ?? ""
    if let port = attrs["port"], !port.isEmpty { s += ":\(port)" }
    if attrs["tls"] != "0" { s += " · TLS" }
    if let e = attrs["error"], !e.isEmpty { s += " · \(e)" }
    return s
  }

  var body: some View {
    HStack(spacing: 8) {
      SettingsStatusDot(state: state)
      VStack(alignment: .leading, spacing: 1) {
        Text([attrs["name"], attrs["host"]].compactMap { $0 }.first { !$0.isEmpty } ?? netid)
        Text(detail).font(.caption.monospaced()).foregroundStyle(.secondary).lineLimit(2)
      }
      Spacer()
      Button("Edit…", action: onEdit)
      Button(role: .destructive, action: onRemove) {
        Image(systemName: "minus.circle")
      }
      .buttonStyle(.borderless)
      .help("Remove from the bouncer")
      .accessibilityLabel("Remove network")
    }
  }
}

private struct BouncerNetworkSheet: View {
  @LocalState var draft: BouncerNetworkDraft
  var onSave: (BouncerNetworkDraft) -> Void
  var onCancel: () -> Void
  @LocalState private var error: String?
  @Environment(\.palette) private var p

  init(draft: BouncerNetworkDraft, onSave: @escaping (BouncerNetworkDraft) -> Void, onCancel: @escaping () -> Void) {
    _draft = LocalState(initialValue: draft)
    self.onSave = onSave
    self.onCancel = onCancel
  }

  var body: some View {
    VStack(spacing: 0) {
      Form {
        Section {
          TextField("Name", text: $draft.name, prompt: Text("Libera.Chat"))
          TextField("Host", text: $draft.host, prompt: Text("irc.libera.chat"))
            .font(.body.monospaced())
            .autocorrectionDisabled()
          TextField("Port", text: $draft.port, prompt: Text(draft.tls ? "6697" : "6667"))
            .font(.body.monospaced())
          Toggle("Use TLS", isOn: $draft.tls)
        } header: {
          Text(draft.netid == nil ? "Add a network" : "Edit network")
        }
        Section {
          TextField("Nickname", text: $draft.nickname, prompt: Text("Bouncer default"))
            .autocorrectionDisabled()
          TextField("Username", text: $draft.username, prompt: Text("Bouncer default"))
            .autocorrectionDisabled()
          SecureField(
            "Server password", text: $draft.pass, prompt: Text(draft.netid == nil ? "Usually empty" : "Unchanged"))
        }
      }
      .formStyle(.grouped)
      HStack {
        if let error { Text(error).foregroundStyle(p.danger).font(.callout) }
        Spacer()
        Button("Cancel", role: .cancel, action: onCancel)
          .keyboardShortcut(.cancelAction)
        Button(draft.netid == nil ? "Add Network" : "Save", action: save)
          .keyboardShortcut(.defaultAction)
      }
      .padding(.horizontal, 20)
      .padding(.bottom, 16)
    }
    .frame(width: 440)
  }

  private func save() {
    if draft.host.trimmingCharacters(in: .whitespaces).isEmpty { return error = "A host is required." }
    if let port = Int(draft.port.trimmingCharacters(in: .whitespaces)), !(1...65535).contains(port) {
      return error = "Choose a port between 1 and 65535."
    }
    onSave(draft)
  }
}
