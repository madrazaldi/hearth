import SwiftUI

struct AppearanceSettings: View {
  @Environment(AppModel.self) private var app

  private static let sizes = [(13, "Small"), (14, "Medium"), (15, "Large"), (16, "Extra Large")]

  var body: some View {
    @Bindable var app = app
    Form {
      Section {
        Picker("Appearance", selection: $app.settings.theme) {
          Text("System").tag(Theme.system)
          Text("Light").tag(Theme.light)
          Text("Dark").tag(Theme.dark)
        }
        .pickerStyle(.segmented)
        Picker(selection: $app.settings.layout) {
          Text("Columns").tag(Layout.columns)
          Text("Compact").tag(Layout.compact)
        } label: {
          SettingsLabel("Message layout", hint: "Columns aligns nicks in a gutter; compact stacks them above messages.")
        }
        Picker("Text size", selection: $app.settings.fontSize) {
          ForEach(Self.sizes, id: \.0) { Text($0.1).tag($0.0) }
          if !Self.sizes.contains(where: { $0.0 == app.settings.fontSize }) {
            Text("\(app.settings.fontSize) pt").tag(app.settings.fontSize)
          }
        }
      }
      Section("Messages") {
        Picker("Clock", selection: $app.settings.clock) {
          Text("24-hour").tag(Clock.h24)
          Text("12-hour").tag(Clock.h12)
        }
        Toggle("Show seconds in timestamps", isOn: $app.settings.showSeconds)
        Picker(selection: $app.settings.membership) {
          Text("Show").tag(Membership.show)
          Text("Collapse").tag(Membership.collapse)
          Text("Hide").tag(Membership.hide)
        } label: {
          SettingsLabel("Joins, parts and quits", hint: "Collapse folds consecutive events into one line.")
        }
        Toggle("Colored nicknames", isOn: $app.settings.nickColors)
      }
      Section("Channels") {
        Toggle("Show the member list", isOn: $app.settings.showMembers)
        Toggle("Show when others are typing", isOn: $app.settings.showTyping)
        Toggle("Let others see when I'm typing", isOn: $app.settings.sendTyping)
      }
    }
    .formStyle(.grouped)
    .frame(width: 560, height: 520)
  }
}
