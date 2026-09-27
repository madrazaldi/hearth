// The Settings window (⌘,): the native counterpart of Settings.svelte.

import SwiftUI

/// Which row of the Servers list is showing.
enum ServerSelection: Hashable {
  case server(String)
  case new
}

struct SettingsView: View {
  @Environment(AppModel.self) private var app
  @LocalState private var tab = SettingsTab.general
  @LocalState private var serverSelection: ServerSelection?

  var body: some View {
    TabView(selection: $tab) {
      GeneralSettings()
        .tabItem { Label("General", systemImage: "gearshape") }
        .tag(SettingsTab.general)
      AppearanceSettings()
        .tabItem { Label("Appearance", systemImage: "paintpalette") }
        .tag(SettingsTab.appearance)
      NotificationSettings()
        .tabItem { Label("Notifications", systemImage: "bell.badge") }
        .tag(SettingsTab.notifications)
      PrivacySettings()
        .tabItem { Label("Privacy & Data", systemImage: "hand.raised") }
        .tag(SettingsTab.privacy)
      AliasSettings()
        .tabItem { Label("Aliases", systemImage: "terminal") }
        .tag(SettingsTab.aliases)
      ServerSettings(selection: $serverSelection)
        .tabItem { Label("Servers", systemImage: "server.rack") }
        .tag(SettingsTab.servers)
    }
    .onAppear(perform: applyRoute)
    .onChange(of: app.settingsRoute) { applyRoute() }
  }

  /// Follow a request from elsewhere in the app (a menu, the sidebar), then clear it
  /// so the same route can be asked for again.
  private func applyRoute() {
    guard let route = app.settingsRoute else { return }
    tab = route.tab
    if route.tab == .servers { serverSelection = route.serverId.map { .server($0) } ?? .new }
    app.settingsRoute = nil
  }
}
