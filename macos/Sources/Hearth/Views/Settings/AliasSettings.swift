import SwiftUI

/// User slash commands (AliasEditor.svelte).
struct AliasSettings: View {
  @Environment(AppModel.self) private var app
  @LocalState private var name = ""
  @LocalState private var commandText = ""
  @LocalState private var tried = false
  @FocusState private var nameFocused: Bool
  @Environment(\.palette) private var p

  private var key: String { aliasName(name) }
  private var error: String? { aliasError(name: key, body: commandText) }
  private var exists: Bool { app.settings.aliases[key] != nil }

  var body: some View {
    Form {
      Section {
        let entries = app.settings.aliases.sorted { $0.key < $1.key }
        if entries.isEmpty {
          SettingsNote("No aliases yet.")
        }
        ForEach(entries, id: \.key) { n, b in
          HStack(spacing: 10) {
            Button {
              name = n
              commandText = b
              tried = false
              nameFocused = true
            } label: {
              HStack(spacing: 10) {
                Text("/\(n)").font(.body.monospaced().weight(.medium))
                Text(b).font(.body.monospaced()).foregroundStyle(.secondary).lineLimit(1).truncationMode(.tail)
                Spacer(minLength: 0)
              }
              .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Edit")
            .accessibilityLabel("Edit alias /\(n): \(b)")
            if isBuiltinCommand(n) {
              Text("Replaces built-in")
                .font(.caption)
                .foregroundStyle(p.warn)
                .help("Replaces the built-in /\(n)")
            }
            Button {
              app.settings.aliases.removeValue(forKey: n)
            } label: {
              Image(systemName: "minus.circle")
            }
            .buttonStyle(.borderless)
            .help("Delete alias")
            .accessibilityLabel("Delete alias /\(n)")
          }
        }
      } header: {
        Text("Your commands")
      } footer: {
        SettingsNote(
          "They show up in command suggestions and Tab completion. For example `/j` → `/join $1`, `/ns` → `/msg NickServ $*`."
        )
      }

      Section {
        TextField("Name", text: $name, prompt: Text("/ns"))
          .font(.body.monospaced())
          .autocorrectionDisabled()
          .focused($nameFocused)
          .onSubmit(save)
        TextField("Commands", text: $commandText, prompt: Text("/msg NickServ $*"))
          .font(.body.monospaced())
          .autocorrectionDisabled()
          .onSubmit(save)
        if tried, let error {
          SettingsNote(error, .error)
        } else if !key.isEmpty && isBuiltinCommand(key) {
          SettingsNote(
            "/\(key) is a built-in command. This alias will replace it; inside the alias, /\(key) still runs the built-in.",
            .warning)
        }
        HStack {
          Spacer()
          if !name.isEmpty || !commandText.isEmpty {
            Button("Clear") {
              name = ""
              commandText = ""
              tried = false
            }
          }
          Button(exists ? "Update Alias" : "Add Alias", action: save)
            .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty || commandText.trimmingCharacters(in: .whitespaces).isEmpty)
        }
      } header: {
        Text(exists ? "Edit /\(key)" : "New alias")
      } footer: {
        SettingsNote(
          "`$1`…`$9` are arguments, `$2-` is the second onward, `$*` is all of them. `$nick`, `$channel` and `$network` fill in the obvious. Separate commands with `;` (`\\;` for a literal one). Lines without a leading `/` are sent as messages."
        )
      }
    }
    .formStyle(.grouped)
    .frame(width: 560, height: 520)
  }

  private func save() {
    tried = true
    if error != nil { return }
    app.settings.aliases[key] = commandText.trimmingCharacters(in: .whitespaces)
    name = ""
    commandText = ""
    tried = false
  }
}
