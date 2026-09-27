// The main window: the sidebar of networks and buffers, the open buffer, and
// the panels and toasts that float over them (App.svelte).

import SwiftUI

struct RootView: View {
  @Environment(AppModel.self) private var app

  var body: some View {
    @Bindable var app = app
    Group {
      if app.welcoming {
        WelcomeView { app.onboarding = false }
      } else {
        MainView()
      }
    }
    // Latched so the welcome flow can show connection progress after the
    // first server is saved, until the user moves on.
    .onChange(of: app.ready) { if app.ready && app.servers.isEmpty { app.onboarding = true } }
    .sheet(isPresented: $app.switcherOpen) { QuickSwitcher().environment(app).themed() }
    .sheet(isPresented: $app.searchOpen) { SearchPanel().environment(app).themed() }
    .sheet(isPresented: $app.shortcutsOpen) { ShortcutsHelp().environment(app).themed() }
    .sheet(item: $app.browserConn) { conn in ChannelBrowser(conn: conn).environment(app).themed() }
    .overlay(alignment: .bottom) { ToastStack() }
    .onChange(of: app.settingsRoute) { if app.settingsRoute != nil { openSettings() } }
  }
}

private struct MainView: View {
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    NavigationSplitView {
      SidebarView()
        .navigationSplitViewColumnWidth(min: 190, ideal: 240, max: 420)
    } detail: {
      if let buf = app.active {
        BufferView(buf: buf).id(buf.id)
      } else {
        Text(app.ready ? "Connecting…" : "")
          .foregroundStyle(p.text3)
          .frame(maxWidth: .infinity, maxHeight: .infinity)
          .background(p.bg)
      }
    }
  }
}

private struct ToastStack: View {
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    VStack(spacing: 8) {
      ForEach(app.toasts) { t in
        HStack(spacing: 10) {
          if t.kind == .error {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(p.danger)
          }
          Text(t.text).foregroundStyle(p.text).fixedSize(horizontal: false, vertical: true)
          if let action = t.action {
            Button(action.label) {
              app.dismissToast(t.id)
              action.run()
            }
            .buttonStyle(.borderedProminent)
            Button("Not now") { app.dismissToast(t.id) }
          }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(t.kind == .error ? p.danger.opacity(0.5) : p.lineStrong))
        .shadow(color: .black.opacity(0.25), radius: 16, y: 6)
        .onTapGesture { if t.action == nil { app.dismissToast(t.id) } }
        .transition(.move(edge: .bottom).combined(with: .opacity))
      }
    }
    .frame(maxWidth: 520)
    .padding(.bottom, 80)
    .animation(.snappy(duration: 0.2), value: app.toasts.map(\.id))
  }
}
