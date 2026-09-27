import SwiftUI

struct GeneralSettings: View {
  @Environment(AppModel.self) private var app

  var body: some View {
    @Bindable var app = app
    Form {
      Section {
        Toggle(isOn: $app.settings.closeToTray) {
          SettingsLabel(
            "Keep running when the window is closed",
            hint: "Hearth stays connected in the menu bar. Quit from the menu bar icon or with ⌘Q.")
        }
        Picker(selection: $app.settings.autoAwayMinutes) {
          Text("Never").tag(0)
          Text("After 10 minutes").tag(10)
          Text("After 30 minutes").tag(30)
          Text("After 1 hour").tag(60)
          // Keep a value set by hand in config.json selectable.
          if ![0, 10, 30, 60].contains(app.settings.autoAwayMinutes) {
            Text("After \(app.settings.autoAwayMinutes) minutes").tag(app.settings.autoAwayMinutes)
          }
        } label: {
          SettingsLabel(
            "Mark me away",
            hint: "When Hearth has been in the background this long. You're marked back when you return.")
        }
      }
      Section {
        Toggle(isOn: $app.settings.spellcheck) {
          SettingsLabel(
            "Check spelling as I type",
            hint: "Underlines misspelled words in the message box. Autocorrect stays off.")
        }
      }
    }
    .formStyle(.grouped)
    .frame(width: 560, height: 280)
  }
}
