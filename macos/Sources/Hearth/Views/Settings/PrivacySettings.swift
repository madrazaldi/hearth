import SwiftUI

struct PrivacySettings: View {
  @Environment(AppModel.self) private var app
  @LocalState private var confirmClear = false
  @LocalState private var clearing = false
  @LocalState private var newIgnore = ""

  var body: some View {
    @Bindable var app = app
    Form {
      Section {
        Toggle(isOn: $app.settings.storeLogs) {
          SettingsLabel(
            "Save messages on this Mac",
            hint: "Keeps scrollback across restarts and makes messages searchable. Stored only on this device.")
        }
        LabeledContent {
          Button("Delete Saved Messages…", role: .destructive) { confirmClear = true }
            .disabled(clearing)
        } label: {
          SettingsLabel("Saved messages", hint: "Deleting them doesn't affect history kept by a server or bouncer.")
        }
      }

      Section {
        Toggle(isOn: $app.settings.linkPreviews) {
          SettingsLabel(
            "Show link previews",
            hint:
              "Hearth fetches linked pages and images itself, so those sites see your IP address. Links to local or private addresses are never fetched."
          )
        }
        Toggle(isOn: $app.settings.inlineImages) {
          SettingsLabel(
            "Show inline images",
            hint:
              "Image links appear under the message. Images in messages that mention NSFW stay blurred until clicked. Video and audio are only described."
          )
        }
        .disabled(!app.settings.linkPreviews)
        Toggle(isOn: $app.settings.autoplayAnimations) {
          SettingsLabel(
            "Play animations automatically",
            hint:
              "Otherwise animated images play while the pointer is over them. Never when your system asks for reduced motion."
          )
        }
        .disabled(!app.settings.linkPreviews)
        Toggle(isOn: $app.settings.loadAvatars) {
          SettingsLabel(
            "Show profile pictures",
            hint:
              "People can publish an avatar on servers that support it. Hearth fetches the image itself, so its host sees your IP address."
          )
        }
      } header: {
        Text("Media")
      }

      Section {
        ForEach(app.settings.ignores, id: \.self) { mask in
          HStack {
            Text(mask).font(.body.monospaced()).textSelection(.enabled)
            Spacer()
            Button {
              _ = app.unignore(mask)
            } label: {
              Image(systemName: "minus.circle")
            }
            .buttonStyle(.borderless)
            .help("Stop ignoring")
            .accessibilityLabel("Stop ignoring \(mask)")
          }
        }
        HStack {
          TextField("Ignore", text: $newIgnore, prompt: Text("nick or nick!user@host"))
            .labelsHidden()
            .font(.body.monospaced())
            .autocorrectionDisabled()
            .onSubmit(addIgnore)
          Button("Ignore", action: addIgnore)
            .disabled(newIgnore.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        SettingsNote("Hides their messages, notices, invites and reactions everywhere. `*` and `?` are wildcards.")
      } header: {
        Text("Ignored people")
      }
    }
    .formStyle(.grouped)
    .frame(width: 560, height: 580)
    .confirmationDialog("Delete all saved messages?", isPresented: $confirmClear) {
      Button("Delete", role: .destructive, action: clearLogs)
    } message: {
      Text("Scrollback and search on this Mac start over. History kept by a server or bouncer isn't affected.")
    }
  }

  private func addIgnore() {
    let mask = newIgnore.trimmingCharacters(in: .whitespaces)
    if mask.isEmpty { return }
    app.ignore(mask)
    newIgnore = ""
  }

  private func clearLogs() {
    clearing = true
    Task {
      defer { clearing = false }
      do {
        try await app.backend.clearLogs()
        app.toast("Saved messages deleted")
      } catch {
        app.toast(String(describing: error), .error)
      }
    }
  }
}
