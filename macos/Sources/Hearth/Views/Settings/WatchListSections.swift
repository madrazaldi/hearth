import SwiftUI

/// The MONITOR watch list for each of a server's connections: the server
/// itself, or every network behind a bouncer (the control connection has none).
struct WatchListSections: View {
  @Environment(AppModel.self) private var app
  let serverId: String

  private var conns: [Conn] {
    app.orderedConns.filter { $0.serverId == serverId && !$0.isControl }
  }

  var body: some View {
    let list = conns
    ForEach(list) { conn in
      WatchListSection(conn: conn, titled: list.count > 1)
    }
  }
}

private struct WatchListSection: View {
  @Environment(AppModel.self) private var app
  let conn: Conn
  var titled: Bool
  @LocalState private var newNick = ""

  var body: some View {
    Section {
      ForEach(conn.watched, id: \.self) { nick in
        HStack(spacing: 8) {
          if conn.canMonitor {
            let online = conn.online[conn.fold(nick)]
            SettingsStatusDot(state: online == true ? .ok : .off)
              .help(online == true ? "Online" : online == false ? "Offline" : "Unknown")
          }
          Text(nick)
          Spacer()
          Button {
            app.unwatch(conn, nick)
          } label: {
            Image(systemName: "minus.circle")
          }
          .buttonStyle(.borderless)
          .help("Stop watching")
          .accessibilityLabel("Stop watching \(nick)")
        }
        .accessibilityElement(children: .combine)
      }
      HStack {
        TextField("Watch", text: $newNick, prompt: Text("nickname"))
          .labelsHidden()
          .autocorrectionDisabled()
          .onSubmit(add)
        Button("Watch", action: add)
          .disabled(newNick.trimmingCharacters(in: .whitespaces).isEmpty)
      }
      if conn.canMonitor {
        SettingsNote("Get told when these people come online or leave, in the sidebar and the server buffer.")
      } else if conn.status == .connected {
        SettingsNote("\(conn.name) doesn't support MONITOR, so Hearth can't tell when these people are online.", .warning)
      } else {
        SettingsNote("Get told when these people come online or leave, on servers that support MONITOR.")
      }
    } header: {
      Text(titled ? "Watch list · \(conn.name)" : "Watch list")
    }
  }

  private func add() {
    // A nick, never a mask or list: MONITOR takes bare nicknames.
    let nick = newNick.trimmingCharacters(in: .whitespaces)
    guard !nick.isEmpty, !nick.contains(where: { $0.isWhitespace || $0 == "," }) else { return }
    app.watch(conn, nick)
    newNick = ""
  }
}
