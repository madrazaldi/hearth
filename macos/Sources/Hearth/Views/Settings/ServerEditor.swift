// One server's settings (ServerForm.svelte), followed by the parts of the
// Svelte server page that apply immediately: bouncer networks, highlight
// rules and the MONITOR watch list.

import AppKit
import SwiftUI

struct ServerEditor: View {
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  /// Nil when adding a server.
  let original: ServerConfig?
  @Binding var dirty: Bool
  var onSaved: (ServerConfig) -> Void
  var onCancel: () -> Void
  var onRemove: () -> Void

  @LocalState private var draft: ServerConfig
  @LocalState private var autojoinText: String
  @LocalState private var performText: String
  @LocalState private var saving = false
  @LocalState private var error: String?
  @LocalState private var cert: CertInfo?
  @LocalState private var certError: String?

  init(
    original: ServerConfig?, dirty: Binding<Bool>, onSaved: @escaping (ServerConfig) -> Void,
    onCancel: @escaping () -> Void, onRemove: @escaping () -> Void
  ) {
    self.original = original
    _dirty = dirty
    self.onSaved = onSaved
    self.onCancel = onCancel
    self.onRemove = onRemove
    let s = original ?? ServerConfig()
    _draft = LocalState(initialValue: s)
    _autojoinText = LocalState(initialValue: s.autojoin.joined(separator: ", "))
    _performText = LocalState(initialValue: s.perform.joined(separator: "\n"))
  }

  private var isNew: Bool { original == nil }
  private var conn: Conn? { original.flatMap { app.conns[$0.id] } }

  /// The draft with the free-text fields parsed, as it would be saved.
  private var edited: ServerConfig {
    var s = draft
    s.autojoin = parseChannels(autojoinText)
    s.perform = settingsList(performText, separators: "\n")
    return s
  }

  private var hasChanges: Bool { edited != (original ?? ServerConfig()) }

  var body: some View {
    VStack(spacing: 0) {
      Form {
        if let original { statusSection(original) }
        serverSection
        identitySection
        authSection
        connectSection
        if let original {
          if let conn, conn.isControl {
            BouncerNetworksSection(conn: conn)
          }
          NetworkHighlightsSection(serverId: original.id, nick: original.nick)
          WatchListSections(serverId: original.id)
        }
      }
      .formStyle(.grouped)
      Divider()
      footer
    }
    .onChange(of: hasChanges, initial: true) { dirty = hasChanges }
  }

  // ─── Sections ───────────────────────────────────────────────

  private func statusSection(_ s: ServerConfig) -> some View {
    Section {
      LabeledContent {
        HStack {
          if conn?.status == nil || conn?.status == .disconnected {
            Button("Connect") { app.connectServer(s.id) }
          } else {
            Button("Reconnect") { app.reconnectServer(s.id) }
            Button("Disconnect") { app.disconnectServer(s.id) }
          }
        }
      } label: {
        HStack(spacing: 8) {
          SettingsStatusDot(state: .init(conn))
          VStack(alignment: .leading, spacing: 2) {
            Text(statusText).font(.headline)
            if let e = conn?.error, conn?.status == .disconnected {
              Text(e).font(.callout).foregroundStyle(p.danger).lineLimit(3).textSelection(.enabled)
            } else if conn?.isControl == true {
              Text("soju bouncer · \(conn?.networks.count ?? 0) networks").font(.callout).foregroundStyle(.secondary)
            }
          }
        }
      }
    }
  }

  private var statusText: String {
    guard let conn else { return "Not connected" }
    switch conn.status {
    case .connected: return conn.nick.isEmpty ? "Connected" : "Connected as \(conn.nick)"
    case .connecting: return "Connecting…"
    case .registering: return "Signing in…"
    case .disconnected:
      if conn.retryAt > nowMs() { return "Disconnected · retrying" }
      return "Disconnected"
    }
  }

  private var serverSection: some View {
    Section {
      TextField("Name", text: $draft.name, prompt: Text(draft.host.isEmpty ? "Libera.Chat" : draft.host))
      TextField("Host", text: $draft.host, prompt: Text("irc.libera.chat"))
        .font(.body.monospaced())
        .autocorrectionDisabled()
      TextField("Port", value: $draft.port, format: .number.grouping(.never), prompt: Text(draft.tls ? "6697" : "6667"))
        .font(.body.monospaced())
      Picker("Transport", selection: Binding(get: { draft.websocket }, set: setTransport)) {
        Text("IRC").tag(false)
        Text("WebSocket").tag(true)
      }
      .pickerStyle(.segmented)
      if draft.websocket {
        TextField("WebSocket path", text: $draft.websocketPath, prompt: Text("/socket"))
          .font(.body.monospaced())
          .autocorrectionDisabled()
      }
      Toggle("Use TLS", isOn: Binding(get: { draft.tls }, set: setTls))
      Toggle(isOn: $draft.acceptInvalidCerts) {
        SettingsLabel("Accept invalid certificates", hint: "Only for servers you trust with self-signed certificates.")
      }
      .disabled(!draft.tls)
    } header: {
      Text("Server")
    }
  }

  private var identitySection: some View {
    Section {
      TextField("Nickname", text: $draft.nick)
        .autocorrectionDisabled()
      if !draft.nick.isEmpty, let e = nickError(draft.nick) {
        SettingsNote(e, .warning)
      }
      TextField("Username", text: $draft.username, prompt: Text(draft.nick.isEmpty ? "Optional" : draft.nick))
        .autocorrectionDisabled()
      TextField("Real name", text: $draft.realname, prompt: Text(draft.nick.isEmpty ? "Optional" : draft.nick))
      SecureField("Server password", text: $draft.password, prompt: Text("Usually empty"))
    } header: {
      Text("Identity")
    }
  }

  private var authSection: some View {
    Section {
      Picker("Sign in with", selection: $draft.sasl) {
        Text("None").tag(SaslMechanism.none)
        Text("Password").tag(SaslMechanism.plain)
        Text("Certificate").tag(SaslMechanism.external)
      }
      .pickerStyle(.segmented)
      switch draft.sasl {
      case .none:
        EmptyView()
      case .plain:
        TextField("Account", text: $draft.saslUsername, prompt: Text(draft.nick))
          .autocorrectionDisabled()
        SecureField("Password", text: $draft.saslPassword)
        SettingsNote(
          "Uses SCRAM-SHA-256 when the server offers it, so the password never crosses the wire. Stored in your keychain.")
      case .external:
        certificateRows
      }
    } header: {
      Text("Authentication")
    }
  }

  @ViewBuilder
  private var certificateRows: some View {
    LabeledContent("Client certificate") {
      HStack {
        TextField("Client certificate", text: $draft.clientCert, prompt: Text("/path/to/cert.pem"))
          .labelsHidden()
          .font(.body.monospaced())
          .autocorrectionDisabled()
        Button("Choose…", action: chooseCert)
        Button("Generate", action: generateCert)
          .help("Create a new certificate and key for this server")
      }
    }
    .task(id: draft.clientCert) { await loadCert() }
    if let cert {
      VStack(alignment: .leading, spacing: 4) {
        HStack {
          Text("SHA-512 fingerprint").font(.callout)
          Spacer()
          Button("Copy") {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(cert.sha512, forType: .string)
          }
          .controlSize(.small)
        }
        Text(cert.sha512)
          .font(.caption.monospaced())
          .foregroundStyle(.secondary)
          .textSelection(.enabled)
          .fixedSize(horizontal: false, vertical: true)
      }
      SettingsNote(
        "Register it with the network's services while connected. On Libera.Chat and most Atheme networks: `/msg NickServ CERT ADD`"
      )
    } else if let certError {
      SettingsNote(certError, .error)
    } else {
      SettingsNote("A PEM file holding both the certificate and its private key.")
    }
  }

  private var connectSection: some View {
    Section {
      Toggle("Connect on launch", isOn: $draft.autoconnect)
      VStack(alignment: .leading, spacing: 6) {
        TextField("Channels to join", text: $autojoinText, prompt: Text("#hearth, #rust"))
          .font(.body.monospaced())
          .autocorrectionDisabled()
        SettingsNote("Ignored when connecting through a bouncer, which restores channels itself.")
      }
      VStack(alignment: .leading, spacing: 6) {
        Text("Commands after connecting")
        TextEditor(text: $performText)
          .font(.body.monospaced())
          .autocorrectionDisabled()
          .frame(height: 64)
          .scrollContentBackground(.hidden)
          .padding(4)
          .background(RoundedRectangle(cornerRadius: 5).fill(Color(nsColor: .textBackgroundColor)))
          .overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(Color(nsColor: .separatorColor)))
          .accessibilityLabel("Commands after connecting")
        SettingsNote("One raw IRC line each, such as `MODE $nick +x`. `$nick` expands to your nickname.")
      }
    } header: {
      Text("On connect")
    }
  }

  private var footer: some View {
    HStack(spacing: 8) {
      if !isNew {
        Button("Remove Server…", role: .destructive, action: onRemove)
      }
      if let error {
        Text(error)
          .foregroundStyle(p.danger)
          .font(.callout)
          .lineLimit(2)
          .frame(maxWidth: .infinity, alignment: .leading)
      } else {
        Spacer()
      }
      if saving { ProgressView().controlSize(.small) }
      if isNew {
        Button("Cancel", action: onCancel)
        Button("Add & Connect") { submit(reconnect: true) }
          .buttonStyle(.borderedProminent)
          .disabled(saving)
      } else {
        Button("Revert", action: revert).disabled(!hasChanges || saving)
        Button("Save") { submit(reconnect: false) }
          .disabled(!hasChanges || saving)
        Button("Save & Reconnect") { submit(reconnect: true) }
          .buttonStyle(.borderedProminent)
          .disabled(saving)
          .help("Save, then reconnect so the changes take effect now")
      }
    }
    .padding(.horizontal, 20)
    .padding(.vertical, 12)
  }

  // ─── Actions ────────────────────────────────────────────────

  /// Follow the transport's default port unless one was typed.
  private func setTransport(_ ws: Bool) {
    draft.websocket = ws
    if ws && (draft.port == 6697 || draft.port == 6667) { draft.port = draft.tls ? 443 : 80 }
    if !ws && (draft.port == 443 || draft.port == 80) { draft.port = draft.tls ? 6697 : 6667 }
  }

  private func setTls(_ on: Bool) {
    draft.tls = on
    if !draft.websocket && draft.port == (on ? 6667 : 6697) { draft.port = on ? 6697 : 6667 }
    if draft.websocket && draft.port == (on ? 80 : 443) { draft.port = on ? 443 : 80 }
  }

  private func revert() {
    let s = original ?? ServerConfig()
    draft = s
    autojoinText = s.autojoin.joined(separator: ", ")
    performText = s.perform.joined(separator: "\n")
    error = nil
  }

  private func submit(reconnect: Bool) {
    error = nil
    var s = edited
    s.host = s.host.trimmingCharacters(in: .whitespaces)
    s.nick = s.nick.trimmingCharacters(in: .whitespaces)
    s.clientCert = s.clientCert.trimmingCharacters(in: .whitespaces)
    if s.host.isEmpty { return error = "A host is required." }
    if s.nick.isEmpty { return error = "Choose a nickname." }
    if !(1...65535).contains(s.port) { return error = "Choose a port between 1 and 65535." }
    if s.sasl == .plain && s.saslUsername.isEmpty { s.saslUsername = s.nick }
    if s.sasl == .external && s.clientCert.isEmpty { return error = "Choose or generate a client certificate." }
    if s.sasl == .external && !s.tls { return error = "Certificate authentication needs TLS." }
    saving = true
    Task {
      defer { saving = false }
      do {
        let saved = try await app.saveServer(s, connectNow: reconnect)
        draft = saved
        autojoinText = saved.autojoin.joined(separator: ", ")
        performText = saved.perform.joined(separator: "\n")
        onSaved(saved)
      } catch {
        self.error = String(describing: error)
      }
    }
  }

  private func loadCert() async {
    cert = nil
    certError = nil
    let path = draft.clientCert.trimmingCharacters(in: .whitespaces)
    if path.isEmpty { return }
    // Typing a path fires this per keystroke; wait for a pause.
    try? await Task.sleep(for: .milliseconds(300))
    if Task.isCancelled { return }
    do {
      let info = try await app.backend.certInfo(path)
      if !Task.isCancelled { cert = info }
    } catch {
      if !Task.isCancelled { certError = String(describing: error) }
    }
  }

  private func chooseCert() {
    let panel = NSOpenPanel()
    panel.title = "Choose a Client Certificate"
    panel.message = "A PEM file with the certificate and its private key."
    panel.prompt = "Choose"
    panel.canChooseFiles = true
    panel.canChooseDirectories = false
    panel.allowsMultipleSelection = false
    panel.showsHiddenFiles = true
    if panel.runModal() == .OK, let url = panel.url { draft.clientCert = url.path }
  }

  private func generateCert() {
    certError = nil
    // An unsaved server has no id yet; the core only uses it to name the file.
    let id = draft.id.isEmpty ? "new-\(UUID().uuidString.prefix(8).lowercased())" : draft.id
    Task {
      do {
        let info = try await app.backend.generateCert(id)
        draft.clientCert = info.path
      } catch {
        certError = String(describing: error)
      }
    }
  }
}
