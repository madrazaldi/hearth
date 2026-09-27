import SwiftUI

struct NotificationSettings: View {
  @Environment(AppModel.self) private var app
  @LocalState private var words = ""
  @LocalState private var loaded = false

  var body: some View {
    @Bindable var app = app
    Form {
      Section {
        Toggle(isOn: $app.settings.notifyHighlights) {
          SettingsLabel("Mentions", hint: "When someone says your nickname or a highlight word.")
        }
        Toggle("Private messages", isOn: $app.settings.notifyPrivate)
      } header: {
        Text("Notify me about")
      }

      Section {
        TextField("Highlight words", text: $words, prompt: Text("hearth, deploy"))
          .autocorrectionDisabled()
        let bad = settingsList(words, separators: ",").compactMap { w in ruleError(w).map { "\(w): \($0)" } }
        if bad.isEmpty {
          SettingsNote(
            "Comma-separated. Your nickname always counts. Words match whole-word, ignoring case; wrap a regular expression in slashes: `/pattern/i`. Add per-server words and exceptions in Servers.")
        } else {
          ForEach(bad, id: \.self) { SettingsNote("Invalid pattern: \($0)", .error) }
        }
      } header: {
        Text("Highlights")
      }

      Section {
        let overrides = app.settings.bufferNotify.sorted { $0.key < $1.key }
        if overrides.isEmpty {
          SettingsNote("Set a level for a single channel or conversation from its menu in the main window.")
        } else {
          ForEach(overrides, id: \.key) { key, level in
            HStack {
              VStack(alignment: .leading, spacing: 1) {
                Text(bufferName(key))
                Text(app.serverName(prefKey: key)).font(.callout).foregroundStyle(.secondary)
              }
              Spacer()
              Picker("Level", selection: levelBinding(key, level)) {
                Text("Everything").tag(NotifyLevel.all)
                Text("Mentions").tag(NotifyLevel.mentions)
                Text("Nothing").tag(NotifyLevel.none)
              }
              .labelsHidden()
              .fixedSize()
              Button {
                app.settings.bufferNotify.removeValue(forKey: key)
              } label: {
                Image(systemName: "minus.circle")
              }
              .buttonStyle(.borderless)
              .help("Use the default")
              .accessibilityLabel("Use the default for \(bufferName(key))")
            }
          }
        }
      } header: {
        Text("Per-conversation levels")
      }
    }
    .formStyle(.grouped)
    .frame(width: 560, height: 480)
    .onAppear {
      guard !loaded else { return }
      loaded = true
      words = app.settings.highlightWords.joined(separator: ", ")
    }
    .onChange(of: words) {
      // Keep what's saved until every rule is valid, rather than dropping the bad one.
      let list = settingsList(words, separators: ",")
      if list.allSatisfy({ ruleError($0) == nil }) && list != app.settings.highlightWords {
        app.settings.highlightWords = list
      }
    }
  }

  private func levelBinding(_ key: String, _ level: NotifyLevel) -> Binding<NotifyLevel> {
    Binding(get: { level }, set: { app.settings.bufferNotify[key] = $0 })
  }

  /// `serverId|netid|key` → the buffer part; an empty key is the server buffer.
  private func bufferName(_ prefKey: String) -> String {
    let key = prefKey.split(separator: "|", omittingEmptySubsequences: false).dropFirst(2).joined(separator: "|")
    return key.isEmpty ? "Server messages" : key
  }
}
