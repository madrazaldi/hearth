// The first-run flow (Welcome.svelte): pick a network, a nickname, maybe an
// account and some channels, then connect while showing what's happening.

import AppKit
import SwiftUI

/// The Hearth logo (assets/logo/hearth-mark.svg): an arch-shaped speech bubble
/// around a flame. The holes wind the other way, so it fills correctly with
/// either fill rule.
struct HearthMark: Shape {
  func path(in rect: CGRect) -> Path {
    var p = Path()
    // The bubble, clockwise.
    p.move(to: CGPoint(x: 256, y: 844))
    p.addLine(to: CGPoint(x: 256, y: 470))
    p.addRelativeArc(center: CGPoint(x: 520, y: 470), radius: 264, startAngle: .degrees(180), delta: .degrees(180))
    p.addLine(to: CGPoint(x: 784, y: 720))
    p.addRelativeArc(center: CGPoint(x: 736, y: 720), radius: 48, startAngle: .zero, delta: .degrees(90))
    p.addLine(to: CGPoint(x: 384, y: 768))
    p.closeSubpath()
    // Its inside, counter-clockwise.
    p.move(to: CGPoint(x: 688, y: 680))
    p.addLine(to: CGPoint(x: 688, y: 470))
    p.addRelativeArc(center: CGPoint(x: 520, y: 470), radius: 168, startAngle: .zero, delta: .degrees(-180))
    p.addLine(to: CGPoint(x: 352, y: 680))
    p.closeSubpath()
    // The flame, clockwise.
    p.move(to: CGPoint(x: 438, y: 602))
    p.addCurve(to: CGPoint(x: 500, y: 466), control1: CGPoint(x: 438, y: 540), control2: CGPoint(x: 474, y: 500))
    p.addCurve(to: CGPoint(x: 542, y: 418), control1: CGPoint(x: 516, y: 446), control2: CGPoint(x: 530, y: 436))
    p.addCurve(to: CGPoint(x: 594, y: 602), control1: CGPoint(x: 552, y: 478), control2: CGPoint(x: 594, y: 526))
    p.addRelativeArc(center: CGPoint(x: 516, y: 602), radius: 78, startAngle: .zero, delta: .degrees(180))
    p.closeSubpath()
    // The flame's core, counter-clockwise.
    p.move(to: CGPoint(x: 478, y: 642))
    p.addRelativeArc(center: CGPoint(x: 516, y: 642), radius: 38, startAngle: .degrees(180), delta: .degrees(-180))
    p.addCurve(to: CGPoint(x: 526, y: 558), control1: CGPoint(x: 554, y: 612), control2: CGPoint(x: 534, y: 592))
    p.addCurve(to: CGPoint(x: 478, y: 642), control1: CGPoint(x: 502, y: 588), control2: CGPoint(x: 478, y: 610))
    p.closeSubpath()

    // The SVG's viewBox is 240 190 560 670; fit it into `rect`, centred.
    let s = min(rect.width / 560, rect.height / 670)
    let t = CGAffineTransform(
      translationX: rect.minX + (rect.width - 560 * s) / 2, y: rect.minY + (rect.height - 670 * s) / 2
    )
    .scaledBy(x: s, y: s)
    .translatedBy(x: -240, y: -190)
    return p.applying(t)
  }
}

private enum Step: Hashable {
  case network, server, nick, account, channels, custom, connect

  var label: String {
    switch self {
    case .network: "Network"
    case .server, .custom: "Server"
    case .nick: "Nickname"
    case .account: "Account"
    case .channels: "Channels"
    case .connect: "Connect"
    }
  }
}

private enum Field: Hashable {
  case name, host, port, nick, username, realname, password, account, secret, cert, channels
}

private struct Failure: Equatable {
  var raw: String
  var sasl: Bool
}

/// What the connection progress depends on, to react when any of it changes.
private struct Progress: Equatable {
  var status: ConnStatus?
  var error: String?
  var saving: Bool
  var connecting: Bool
}

private func flow(for preset: NetworkPreset?) -> [Step] {
  guard let preset else { return [.network] }
  switch preset.kind {
  case .custom: return [.network, .custom, .connect]
  case .bouncer: return [.network, .server, .nick, .account, .connect]
  case .network: return [.network, .nick] + (preset.sasl == true ? [.account] : []) + [.channels, .connect]
  }
}

struct WelcomeView: View {
  var onDone: () -> Void
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  @LocalState private var step = Step.network
  @LocalState private var preset: NetworkPreset?
  @LocalState private var s = ServerConfig()
  /// Set once saved, so retries update the same server instead of adding another.
  @LocalState private var savedId = ""
  @LocalState private var nickTouched = false
  @LocalState private var useAccount = false
  @LocalState private var accountError = ""
  @LocalState private var serverError = ""
  @LocalState private var picked: [String] = []
  @LocalState private var extra = ""
  @LocalState private var netSel = 0
  @LocalState private var hovered: String?

  @LocalState private var saving = false
  /// Line ids are sequential, so anything above this belongs to the current attempt.
  @LocalState private var baseline = 0
  /// The core has picked up this attempt, so a disconnect belongs to it.
  @LocalState private var live = false
  @LocalState private var failure: Failure?

  @FocusState private var focus: Field?
  @FocusState private var listFocused: Bool

  private var steps: [Step] { flow(for: preset) }
  private var index: Int { steps.firstIndex(of: step) ?? 0 }
  private var nickLen: Int { preset?.nickLen ?? 30 }
  private var nickProblem: String? {
    nickError(s.nick.trimmingCharacters(in: .whitespaces), maxLen: nickLen, network: preset?.label ?? "this network")
  }
  private var netName: String {
    preset?.kind == .network ? preset!.label : s.name.isEmpty ? s.host : s.name
  }
  private var isBouncer: Bool { preset?.kind == .bouncer }
  private var networks: [NetworkPreset] { presets.filter { $0.kind == .network } }

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 0) {
        HearthMark()
          .fill(p.accent)
          .frame(width: 40, height: 48)
          .padding(.bottom, 18)
          .accessibilityHidden(true)
        Text("Hearth")
          .font(.system(size: 30, weight: .semibold))
          .tracking(-0.7)
        Text("A quiet place for IRC. Pick a network to begin — you can add more, or a bouncer, any time.")
          .font(.system(size: 15))
          .foregroundStyle(p.text2)
          .frame(maxWidth: 400, alignment: .leading)
          .fixedSize(horizontal: false, vertical: true)
          .padding(.top, 8)
          .padding(.bottom, 24)
        if steps.count > 1 {
          stepBar.padding(.bottom, 12)
        }
        VStack(alignment: .leading, spacing: 16) { content }
          .padding(22)
          .frame(maxWidth: .infinity, alignment: .leading)
          .background(RoundedRectangle(cornerRadius: 14).fill(p.sidebar))
          .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(p.line))
          .id(step)
      }
      .frame(maxWidth: 560, alignment: .leading)
      .padding(.horizontal, 24)
      .padding(.top, 64)
      .padding(.bottom, 48)
      .frame(maxWidth: .infinity)
    }
    .foregroundStyle(p.text)
    .textFieldStyle(.roundedBorder)
    .controlSize(.large)
    .background {
      ZStack(alignment: .top) {
        p.bg
        EllipticalGradient(colors: [p.accent.opacity(0.07), .clear], center: .top, endRadiusFraction: 0.5)
          .frame(height: 420)
      }
      .ignoresSafeArea()
    }
    .onAppear(perform: focusFirst)
    .onChange(of: step) { focusFirst() }
    .onChange(of: focus) { old, _ in
      if old == .nick && focus != .nick { nickTouched = !s.nick.isEmpty }
    }
    .onChange(of: progress) { track() }
  }

  // ─── Chrome ──────────────────────────────────────────────────

  private var stepBar: some View {
    HStack(spacing: 16) {
      ForEach(Array(steps.enumerated()), id: \.element) { i, f in
        HStack(spacing: 6) {
          ZStack {
            if i < index {
              Circle().fill(p.accentSoft)
              Image(systemName: "checkmark").font(.system(size: 8.5, weight: .bold)).foregroundStyle(p.accent)
            } else if i == index {
              Circle().fill(p.text)
              Text("\(i + 1)").font(.system(size: 10.5, weight: .medium)).foregroundStyle(p.bg)
            } else {
              Circle().strokeBorder(p.lineStrong)
              Text("\(i + 1)").font(.system(size: 10.5))
            }
          }
          .frame(width: 18, height: 18)
          Text(f.label)
        }
        .foregroundStyle(i == index ? p.text : i < index ? p.text2 : p.text3)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(f.label), step \(i + 1) of \(steps.count)\(i == index ? ", current" : i < index ? ", done" : "")")
      }
    }
    .font(.system(size: 12.5))
  }

  private func title(_ text: String, optional: Bool = false) -> some View {
    HStack(alignment: .firstTextBaseline, spacing: 6) {
      Text(text).font(.system(size: 16, weight: .semibold))
      if optional { Text("optional").font(.system(size: 12)).foregroundStyle(p.text3) }
    }
    .accessibilityAddTraits(.isHeader)
  }

  private func field<C: View>(_ label: String, @ViewBuilder _ control: () -> C) -> some View {
    VStack(alignment: .leading, spacing: 5) {
      Text(label).font(.system(size: 12)).foregroundStyle(p.text2)
      control()
    }
  }

  private func switchRow(_ label: String, hint: String? = nil, isOn: Binding<Bool>) -> some View {
    Toggle(isOn: isOn) {
      VStack(alignment: .leading, spacing: 2) {
        Text(label).font(.system(size: 13.5))
        if let hint { Text(hint).font(.system(size: 12)).foregroundStyle(p.text3) }
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .toggleStyle(.switch)
    .controlSize(.small)
  }

  private func note(_ text: String, icon: String = "info.circle", color: Color? = nil) -> some View {
    Label {
      Text(text).fixedSize(horizontal: false, vertical: true)
    } icon: {
      Image(systemName: icon)
    }
    .font(.system(size: 12))
    .foregroundStyle(color ?? p.text3)
  }

  private func errorText(_ text: String) -> some View {
    Text(text).font(.system(size: 13)).foregroundStyle(p.danger)
  }

  private func actions(_ label: String, _ submit: @escaping () -> Void) -> some View {
    HStack {
      Button("Back", action: back).keyboardShortcut(.cancelAction)
      Spacer()
      Button(label, action: submit)
        .keyboardShortcut(.defaultAction)
        .buttonStyle(.borderedProminent)
    }
    .padding(.top, 4)
  }

  // ─── Steps ───────────────────────────────────────────────────

  @ViewBuilder
  private var content: some View {
    switch step {
    case .network: networkStep
    case .server: serverStep
    case .nick: nickStep
    case .account: accountStep
    case .channels: channelsStep
    case .custom: customStep
    case .connect: connectStep
    }
  }

  private var networkStep: some View {
    VStack(alignment: .leading, spacing: 0) {
      title("Choose a network").padding(.bottom, 16)
      VStack(spacing: 0) {
        ForEach(Array(networks.enumerated()), id: \.element.id) { i, n in
          if i > 0 { Rectangle().fill(p.line).frame(height: 1) }
          Button { choose(n) } label: {
            HStack(spacing: 12) {
              Text(n.label).font(.system(size: 13.5, weight: .semibold)).foregroundStyle(p.text)
                .frame(width: 104, alignment: .leading)
              Text(n.blurb).font(.system(size: 12.5)).foregroundStyle(p.text2)
                .fixedSize(horizontal: false, vertical: true)
              Spacer(minLength: 8)
              Text(n.host).font(.system(size: 11, design: .monospaced)).foregroundStyle(p.text3)
              Image(systemName: "chevron.right").font(.system(size: 10, weight: .semibold)).foregroundStyle(p.text3)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 11)
            .background(netSel == i && (listFocused || hovered == n.id) ? p.hover : .clear)
            .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .onHover { inside in
            hovered = inside ? n.id : hovered == n.id ? nil : hovered
            if inside { netSel = i }
          }
          .accessibilityLabel("\(n.label), \(n.blurb)")
        }
      }
      .background(p.bg)
      .clipShape(RoundedRectangle(cornerRadius: 10))
      .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(listFocused ? p.accent.opacity(0.6) : p.line))
      .focusable()
      .focused($listFocused)
      .focusEffectDisabled()
      .onKeyPress(.upArrow) {
        netSel = max(netSel - 1, 0)
        return .handled
      }
      .onKeyPress(.downArrow) {
        netSel = min(netSel + 1, networks.count - 1)
        return .handled
      }
      .onKeyPress(.return) {
        choose(networks[netSel])
        return .handled
      }
      Text("Or connect to your own")
        .font(.system(size: 12))
        .foregroundStyle(p.text3)
        .padding(.top, 18)
        .padding(.bottom, 8)
      HStack(alignment: .top, spacing: 8) {
        ForEach(presets.filter { $0.kind != .network }) { n in
          Button { choose(n) } label: {
            HStack(alignment: .top, spacing: 10) {
              Image(systemName: n.kind == .bouncer ? "square.stack.3d.up" : "slider.horizontal.3")
                .font(.system(size: 14))
                .foregroundStyle(p.text3)
                .frame(width: 16)
              VStack(alignment: .leading, spacing: 2) {
                Text(n.label).font(.system(size: 13.5, weight: .semibold)).foregroundStyle(p.text)
                Text(n.blurb).font(.system(size: 12.5)).foregroundStyle(p.text2)
                  .fixedSize(horizontal: false, vertical: true)
              }
              Spacer(minLength: 0)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 11)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(RoundedRectangle(cornerRadius: 10).fill(hovered == n.id ? p.hover : .clear))
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(hovered == n.id ? p.lineStrong : p.line))
            .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .onHover { inside in hovered = inside ? n.id : hovered == n.id ? nil : hovered }
          .accessibilityLabel("\(n.label), \(n.blurb)")
        }
      }
      .fixedSize(horizontal: false, vertical: true)
    }
  }

  private var serverStep: some View {
    Group {
      title("Where is your bouncer?")
      HStack(alignment: .top, spacing: 12) {
        field("Host") {
          TextField("Host", text: $s.host, prompt: Text("bouncer.example.org"))
            .font(.system(size: 13, design: .monospaced))
            .focused($focus, equals: .host)
        }
        field("Port") {
          TextField("Port", value: $s.port, format: .number.grouping(.never))
            .font(.system(size: 13, design: .monospaced))
            .frame(width: 96)
            .focused($focus, equals: .port)
        }
      }
      switchRow("Use TLS", hint: "Almost every bouncer supports it; the usual port is 6697.", isOn: $s.tls)
      if !serverError.isEmpty { errorText(serverError) }
      actions("Continue", submitServer)
    }
  }

  private var nickStep: some View {
    Group {
      title("Choose a nickname")
      VStack(alignment: .leading, spacing: 5) {
        field("Nickname") {
          TextField("Nickname", text: $s.nick, prompt: Text("ember"))
            .autocorrectionDisabled()
            .focused($focus, equals: .nick)
        }
        if nickTouched, let nickProblem {
          Text(nickProblem).font(.system(size: 12)).foregroundStyle(p.danger)
        } else {
          Text(verbatim: "How others see you. Letters, digits and - _ [ ] { } \\ | ^ `, up to \(nickLen) characters.")
            .font(.system(size: 12))
            .foregroundStyle(p.text3)
        }
      }
      if let preset, preset.kind == .network, preset.sasl == false, let n = preset.accountNote {
        note(n)
      }
      actions("Continue", submitNick)
    }
  }

  private var accountStep: some View {
    Group {
      title(isBouncer ? "Log in to your bouncer" : "Log in to an account", optional: !isBouncer)
      if let n = preset?.accountNote {
        Text(n).font(.system(size: 13)).foregroundStyle(p.text2).fixedSize(horizontal: false, vertical: true)
      }
      if !isBouncer {
        switchRow("I have an account", hint: "Skip this if you haven't registered your nickname.", isOn: $useAccount)
      }
      if useAccount {
        HStack(alignment: .top, spacing: 12) {
          field(isBouncer ? "Username" : "Account name") {
            TextField("Username", text: $s.saslUsername, prompt: Text(s.nick))
              .textContentType(.username)
              .autocorrectionDisabled()
              .focused($focus, equals: .account)
          }
          field("Password") {
            SecureField("Password", text: $s.saslPassword)
              .textContentType(.password)
              .focused($focus, equals: .secret)
          }
        }
        note(
          "Kept in your system keychain, never in Hearth's config file. Sent with SCRAM-SHA-256 when the server supports it.",
          icon: "lock")
      } else if preset?.kind == .network {
        note("No account yet? Connect first, then register with /msg NickServ REGISTER password email.")
      }
      if !accountError.isEmpty { errorText(accountError) }
      actions(isBouncer ? "Connect" : "Continue", submitAccount)
    }
    .onChange(of: useAccount) { if useAccount { focusSoon(.secret) } }
  }

  private var channelsStep: some View {
    Group {
      title("Join some channels", optional: true)
      if let chans = preset?.channels, !chans.isEmpty {
        VStack(alignment: .leading, spacing: 8) {
          Text("Suggested on \(netName)").font(.system(size: 12)).foregroundStyle(p.text2)
          ForEach(chans, id: \.self) { c in
            switchRow(
              c, hint: "The network's official help and community channel.",
              isOn: Binding(
                get: { picked.contains(c) },
                set: { on in picked = on ? picked + [c] : picked.filter { $0 != c } }))
          }
        }
      }
      VStack(alignment: .leading, spacing: 5) {
        field("Other channels") {
          TextField("Other channels", text: $extra, prompt: Text("#rust, #linux"))
            .font(.system(size: 13, design: .monospaced))
            .autocorrectionDisabled()
            .focused($focus, equals: .channels)
        }
        Text("Separate with spaces or commas. You can join more later with /join or the channel browser.")
          .font(.system(size: 12))
          .foregroundStyle(p.text3)
      }
      actions("Connect", { connect() })
    }
  }

  @ViewBuilder
  private var customStep: some View {
    title("Custom server")
    Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 14) {
      GridRow {
        field("Host") {
          TextField("Host", text: $s.host, prompt: Text("irc.example.org"))
            .font(.system(size: 13, design: .monospaced))
            .focused($focus, equals: .host)
        }
        field("Port") {
          TextField("Port", value: $s.port, format: .number.grouping(.never))
            .font(.system(size: 13, design: .monospaced))
            .frame(width: 96)
            .focused($focus, equals: .port)
        }
      }
      GridRow {
        field("Name") {
          TextField("Name", text: $s.name, prompt: Text(s.host.isEmpty ? "Optional" : s.host))
            .focused($focus, equals: .name)
        }
        .gridCellColumns(2)
      }
    }
    switchRow("Use TLS", hint: "Almost every server supports it; the usual port is 6697.", isOn: $s.tls)
    if s.tls {
      switchRow(
        "Accept invalid certificates", hint: "Only for servers you trust, such as self-signed ones.",
        isOn: $s.acceptInvalidCerts)
    }
    Divider().overlay(p.line)
    VStack(alignment: .leading, spacing: 5) {
      field("Nickname") {
        TextField("Nickname", text: $s.nick, prompt: Text("ember"))
          .autocorrectionDisabled()
          .focused($focus, equals: .nick)
      }
      if nickTouched, let nickProblem {
        Text(nickProblem).font(.system(size: 12)).foregroundStyle(p.danger)
      }
    }
    HStack(alignment: .top, spacing: 12) {
      field("Username") {
        TextField("Username", text: $s.username, prompt: Text(s.nick.isEmpty ? "Optional" : s.nick))
          .autocorrectionDisabled()
          .focused($focus, equals: .username)
      }
      field("Real name") {
        TextField("Real name", text: $s.realname, prompt: Text("Optional"))
          .focused($focus, equals: .realname)
      }
    }
    field("Server password") {
      SecureField("Server password", text: $s.password, prompt: Text("Optional"))
        .focused($focus, equals: .password)
    }
    Divider().overlay(p.line)
    field("Log in with") {
      Picker("Log in with", selection: $s.sasl) {
        Text("Nothing").tag(SaslMechanism.none)
        Text("Account and password").tag(SaslMechanism.plain)
        Text("Client certificate").tag(SaslMechanism.external)
      }
      .pickerStyle(.segmented)
      .labelsHidden()
      .controlSize(.regular)
    }
    if s.sasl == .plain {
      HStack(alignment: .top, spacing: 12) {
        field("Account name") {
          TextField("Account name", text: $s.saslUsername, prompt: Text(s.nick))
            .autocorrectionDisabled()
            .focused($focus, equals: .account)
        }
        field("Password") {
          SecureField("Password", text: $s.saslPassword)
            .focused($focus, equals: .secret)
        }
      }
      note("Kept in your system keychain, never in Hearth's config file.", icon: "lock")
    } else if s.sasl == .external {
      field("Certificate") {
        HStack {
          TextField("Certificate", text: $s.clientCert, prompt: Text("PEM file with certificate and key"))
            .font(.system(size: 12, design: .monospaced))
            .focused($focus, equals: .cert)
          Button("Choose…", action: chooseCert).controlSize(.regular)
        }
      }
      note("You can also generate one in Settings → Servers after connecting.")
    }
    Divider().overlay(p.line)
    VStack(alignment: .leading, spacing: 5) {
      field("Channels to join") {
        TextField("Channels to join", text: $extra, prompt: Text("#rust, #linux"))
          .font(.system(size: 13, design: .monospaced))
          .autocorrectionDisabled()
          .focused($focus, equals: .channels)
      }
      Text("Separate with spaces or commas.").font(.system(size: 12)).foregroundStyle(p.text3)
    }
    if !serverError.isEmpty { errorText(serverError) }
    actions("Connect", submitCustom)
  }

  // ─── Connecting ──────────────────────────────────────────────

  private var conn: Conn? { savedId.isEmpty ? nil : app.conns[savedId] }

  /// "saving" or a ConnStatus raw value.
  private var status: String { saving ? "saving" : conn?.status.rawValue ?? "connecting" }

  private var progress: Progress {
    Progress(status: conn?.status, error: conn?.error, saving: saving, connecting: step == .connect)
  }

  private var saslFailed: String? {
    conn?.server.messages.first { $0.type == .error && $0.text.hasPrefix("SASL:") && $0.id > baseline }?.text
  }

  private var nickNote: String? {
    guard let conn, conn.status == .connected, !conn.nick.isEmpty, conn.fold(conn.nick) != conn.fold(s.nick) else {
      return nil
    }
    return "“\(s.nick)” was taken, so you're “\(conn.nick)” for now."
  }

  private var stages: [(key: String, label: String)] {
    [
      ("saving", s.sasl == .plain ? "Saving settings, password to your keychain" : "Saving settings"),
      ("connecting", "Reaching \(s.host)\(s.tls ? " securely" : "")"),
      ("registering", s.sasl == .plain ? "Logging in to your account" : "Signing on"),
      ("connected", isBouncer ? "Connected to your bouncer" : "Connected to \(netName)"),
    ]
  }

  @ViewBuilder
  private var connectStep: some View {
    let done = status == "connected"
    title(failure != nil ? "Couldn't connect" : done ? "You're in" : "Connecting to \(netName)")
    if let failure {
      let friendly = explainError(failure.raw, host: s.host, port: s.port, tls: s.tls, sasl: failure.sasl)
      VStack(alignment: .leading, spacing: 4) {
        Text(friendly.title).fontWeight(.semibold).foregroundStyle(p.danger)
        Text(friendly.hint).font(.system(size: 13)).foregroundStyle(p.text2)
        DisclosureGroup("Details") {
          Text(failure.raw)
            .font(.system(size: 11.5, design: .monospaced))
            .textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.top, 4)
        }
        .font(.system(size: 12))
        .foregroundStyle(p.text3)
        .controlSize(.small)
        .padding(.top, 6)
      }
      .padding(.horizontal, 14)
      .padding(.vertical, 12)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(RoundedRectangle(cornerRadius: 10).fill(p.danger.opacity(0.09)))
      .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(p.danger.opacity(0.3)))
      .accessibilityElement(children: .contain)
      HStack {
        Button("Back", action: editAfterFailure).keyboardShortcut(.cancelAction)
        Spacer()
        Button("Try Again") { connect() }
          .keyboardShortcut(.defaultAction)
          .buttonStyle(.borderedProminent)
      }
    } else {
      let order = ["saving", "connecting", "registering", "connected"]
      let at = order.firstIndex(of: status == "disconnected" ? "connecting" : status) ?? 1
      VStack(alignment: .leading, spacing: 10) {
        ForEach(Array(stages.enumerated()), id: \.element.key) { i, st in
          let finished = i < at || done
          HStack(spacing: 10) {
            ZStack {
              if finished {
                Circle().fill(p.accentSoft)
                Image(systemName: "checkmark").font(.system(size: 9, weight: .bold)).foregroundStyle(p.accent)
              } else if i == at {
                ProgressView().controlSize(.small).scaleEffect(0.8)
              } else {
                Circle().strokeBorder(p.lineStrong)
              }
            }
            .frame(width: 20, height: 20)
            Text(st.label)
          }
          .font(.system(size: 13.5))
          .foregroundStyle(i == at && !done ? p.text : finished ? p.text2 : p.text3)
          .accessibilityElement(children: .ignore)
          .accessibilityLabel("\(st.label)\(finished ? ", done" : i == at ? ", in progress" : "")")
        }
      }
      if done {
        if let nickNote { note(nickNote, color: p.warn) }
        if saslFailed != nil {
          note(
            "Your account login failed, so you're connected without it. Check the password in Settings.",
            color: p.warn)
        }
        if !s.autojoin.isEmpty && preset?.kind == .network {
          Text("Joining \(s.autojoin.joined(separator: ", ")).").font(.system(size: 13)).foregroundStyle(p.text2)
        }
        HStack {
          Spacer()
          Button("Start Chatting", action: onDone)
            .keyboardShortcut(.defaultAction)
            .buttonStyle(.borderedProminent)
        }
      }
    }
  }

  // ─── Actions ─────────────────────────────────────────────────

  private func choose(_ n: NetworkPreset) {
    var c = ServerConfig()
    c.name = n.kind == .network ? n.label : ""
    c.host = n.host
    c.port = n.port
    c.tls = n.tls
    c.nick = s.nick
    s = c
    preset = n
    // A failed attempt was already saved; don't leave it behind when switching networks.
    if !savedId.isEmpty {
      let id = savedId
      Task { try? await app.deleteServer(id) }
    }
    savedId = ""
    useAccount = n.kind == .bouncer
    picked = n.channels
    extra = ""
    nickTouched = false
    serverError = ""
    accountError = ""
    failure = nil
    step = flow(for: n)[1]
  }

  private func next() { step = steps[min(index + 1, steps.count - 1)] }
  private func back() { step = steps[max(index - 1, 0)] }

  private func submitServer() {
    s.host = s.host.trimmingCharacters(in: .whitespaces)
    serverError =
      s.host.isEmpty ? "Enter the address of your bouncer." : (1...65535).contains(s.port) ? "" : "Enter a port from 1 to 65535."
    if serverError.isEmpty { next() }
  }

  private func submitNick() {
    nickTouched = true
    s.nick = s.nick.trimmingCharacters(in: .whitespaces)
    if nickProblem == nil { next() } else { focus = .nick }
  }

  private func submitAccount() {
    accountError = ""
    if useAccount && s.saslPassword.isEmpty {
      accountError = isBouncer ? "Enter your password." : "Enter your password, or turn off “I have an account”."
      focus = .secret
      return
    }
    if preset?.kind == .network { next() } else { connect() }
  }

  private func submitCustom() {
    s.host = s.host.trimmingCharacters(in: .whitespaces)
    s.nick = s.nick.trimmingCharacters(in: .whitespaces)
    nickTouched = true
    serverError =
      s.host.isEmpty
      ? "Enter the server's address."
      : !(1...65535).contains(s.port)
        ? "Enter a port from 1 to 65535."
        : nickProblem != nil
          ? "Choose a valid nickname."
          : s.sasl == .plain && s.saslPassword.isEmpty
            ? "Enter your account password, or log in with nothing."
            : s.sasl == .external && s.clientCert.isEmpty ? "Choose a client certificate." : ""
    if !serverError.isEmpty { return }
    if s.sasl == .plain && s.saslUsername.trimmingCharacters(in: .whitespaces).isEmpty { s.saslUsername = s.nick }
    s.autojoin = parseChannels(extra)
    connect()
  }

  private func chooseCert() {
    let panel = NSOpenPanel()
    panel.canChooseDirectories = false
    panel.allowsMultipleSelection = false
    panel.message = "Choose a PEM file with your client certificate and its key."
    if panel.runModal() == .OK, let url = panel.url { s.clientCert = url.path }
  }

  private func connect() {
    if preset?.kind != .custom {
      s.sasl = useAccount ? .plain : .none
      if !useAccount { s.saslPassword = "" }
      if useAccount && s.saslUsername.trimmingCharacters(in: .whitespaces).isEmpty { s.saslUsername = s.nick }
      if preset?.kind == .network { s.autojoin = parseChannels((picked + [extra]).joined(separator: " ")) }
    }
    if !savedId.isEmpty { s.id = savedId }
    step = .connect
    failure = nil
    live = false
    saving = true
    baseline = nextSeq()
    let cfg = s
    Task {
      do {
        let saved = try await app.saveServer(cfg, connectNow: true)
        savedId = saved.id
        s.id = saved.id
      } catch {
        failure = Failure(raw: String(describing: error), sasl: false)
      }
      saving = false
    }
  }

  /// Watch the attempt: once the core has started it, a disconnect with an
  /// error is its failure.
  private func track() {
    guard step == .connect, !saving, failure == nil, let conn else { return }
    if conn.status == .connecting || conn.status == .registering {
      live = true
    } else if live, conn.status == .disconnected, let error = conn.error {
      failure = Failure(raw: [saslFailed, error].compactMap { $0 }.joined(separator: " — "), sasl: s.sasl == .plain)
      // Stop the backoff loop; the user decides whether to retry or edit.
      let id = savedId
      Task { try? await app.backend.disconnect(id) }
    }
  }

  /// Send the user back to the step that can fix the problem.
  private func editAfterFailure() {
    let auth = failure?.raw.firstMatch(of: /(?i)sasl|authenticat/) != nil
    failure = nil
    if auth && steps.contains(.account) {
      step = .account
    } else if steps.contains(.server) {
      step = .server
    } else {
      step = steps[max(steps.count - 2, 0)]
    }
  }

  // ─── Focus ───────────────────────────────────────────────────

  private func focusFirst() {
    switch step {
    case .network:
      DispatchQueue.main.async { listFocused = true }
    case .server, .custom: focusSoon(.host)
    case .nick: focusSoon(.nick)
    case .account: focusSoon(isBouncer ? .account : useAccount ? .secret : nil)
    case .channels: focusSoon(.channels)
    case .connect: focusSoon(nil)
    }
  }

  /// After the step's views exist, which is a runloop turn after the change.
  private func focusSoon(_ f: Field?) {
    DispatchQueue.main.async { focus = f }
  }
}
