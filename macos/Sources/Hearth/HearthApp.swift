// The native macOS frontend. It drives the same Rust core as the Tauri build
// (crates/hearth-ffi), with the Svelte app's logic ported to Swift in Model/.

import AppKit
import SwiftUI
import UserNotifications

@main
struct HearthApp: App {
  @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
  @LocalState private var boot = Boot.shared

  var body: some Scene {
    Window("Hearth", id: "main") {
      Group {
        if let app = boot.app {
          RootView()
            .environment(app)
            .preferredColorScheme(app.settings.theme.colorScheme)
        } else if let error = boot.error {
          BootError(message: error)
        } else {
          Color.clear
        }
      }
      .themed()
      .frame(minWidth: 720, minHeight: 480)
    }
    .defaultSize(width: 1240, height: 800)
    .commands { if let app = boot.app { HearthCommands(app: app) } }

    SwiftUI.Settings {
      if let app = boot.app {
        SettingsView()
          .environment(app)
          .preferredColorScheme(app.settings.theme.colorScheme)
          .themed()
      }
    }

    MenuBarExtra(isInserted: boot.showsMenuBarItem) {
      MenuBarMenu()
    } label: {
      Image(nsImage: menuBarIcon)
    }
  }
}

/// Starts the core once, and keeps the app model alive for the whole process
/// (connections outlive the window when Hearth keeps running in the menu bar).
@MainActor
@Observable
final class Boot {
  var app: AppModel?
  var error: String?
  @ObservationIgnored private(set) var core: RustCore?
  /// SwiftUI builds this before the app delegate exists, so whichever comes
  /// second connects the two.
  @ObservationIgnored static private(set) var current: Boot?
  static let shared = Boot()

  private init() {
    do {
      let core = try RustCore()
      let app = AppModel(backend: core)
      self.app = app
      self.core = core
      Boot.current = self
      AppDelegate.shared?.attach(app: app, core: core)
      Task {
        await app.start()
        if let mode = ProcessInfo.processInfo.environment["HEARTH_STRESS"] { startStress(app, mode: mode) }
      }
    } catch {
      self.error = String(describing: error)
    }
  }

  var showsMenuBarItem: Binding<Bool> {
    Binding(get: { self.app?.settings.closeToTray ?? false }, set: { _ in })
  }
}

private let menuBarIcon: NSImage = {
  let image =
    Bundle.main.image(forResource: "MenuBarIcon")
    ?? NSImage(systemSymbolName: "flame", accessibilityDescription: "Hearth")!
  image.isTemplate = true
  image.size = NSSize(width: 18, height: 18)
  return image
}()

private struct MenuBarMenu: View {
  @Environment(\.openWindow) private var openWindow

  var body: some View {
    Button("Show Hearth") {
      NSApp.activate(ignoringOtherApps: true)
      openWindow(id: "main")
    }
    Divider()
    Button("Quit Hearth") { NSApp.terminate(nil) }.keyboardShortcut("q")
  }
}

private struct BootError: View {
  var message: String
  @Environment(\.palette) private var p

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      Text("Hearth failed to start").font(.headline)
      Text(message).font(.system(.body, design: .monospaced)).textSelection(.enabled).foregroundStyle(p.text2)
    }
    .padding(40)
    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
  }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, UNUserNotificationCenterDelegate {
  static private(set) var shared: AppDelegate?
  private var app: AppModel?
  private var core: RustCore?
  private var badgeTask: Task<Void, Never>?

  override init() {
    super.init()
    AppDelegate.shared = self
    if let boot = Boot.current, let app = boot.app, let core = boot.core { attach(app: app, core: core) }
  }

  func attach(app: AppModel, core: RustCore) {
    self.app = app
    self.core = core
    // UNUserNotificationCenter needs a bundle; `swift run` has none.
    if Bundle.main.bundleIdentifier != nil {
      let center = UNUserNotificationCenter.current()
      center.delegate = self
      center.requestAuthorization(options: [.alert, .sound, .badge]) { _, _ in }
      app.notify = { title, body, buffer in
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        content.userInfo = ["buffer": buffer]
        center.add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
      }
    }
    trackBadge()
  }

  /// Keep the dock badge on the number of mentions waiting.
  private func trackBadge() {
    guard let app else { return }
    withObservationTracking {
      let n = app.totalHighlights
      NSApp?.dockTile.badgeLabel = n > 0 ? "\(n)" : nil
    } onChange: {
      Task { @MainActor [weak self] in self?.trackBadge() }
    }
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    NotificationCenter.default.addObserver(
      forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
    ) { _ in MainActor.assumeIsolated { AppDelegate.shared?.app?.setFocused(true) } }
    NotificationCenter.default.addObserver(
      forName: NSApplication.didResignActiveNotification, object: nil, queue: .main
    ) { _ in MainActor.assumeIsolated { AppDelegate.shared?.app?.setFocused(false) } }
    trackBadge()
  }

  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    !(app?.settings.closeToTray ?? false)
  }

  func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    guard let app, let core else { return .terminateNow }
    Task {
      await app.flushSettings()
      core.close()
      sender.reply(toApplicationShouldTerminate: true)
    }
    return .terminateLater
  }

  nonisolated func userNotificationCenter(
    _ center: UNUserNotificationCenter, willPresent notification: UNNotification
  ) async -> UNNotificationPresentationOptions {
    [.banner, .sound]
  }

  nonisolated func userNotificationCenter(
    _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse
  ) async {
    guard let id = response.notification.request.content.userInfo["buffer"] as? String else { return }
    await MainActor.run {
      NSApp.activate(ignoringOtherApps: true)
      guard let app = self.app, let buf = app.allBuffers.first(where: { $0.id == id }) else { return }
      app.open(buf)
    }
  }
}

struct HearthCommands: Commands {
  let app: AppModel
  @Environment(\.openSettings) private var openSettings

  var body: some Commands {
    CommandGroup(replacing: .newItem) {
      Button("Add Server…") {
        app.settingsRoute = SettingsRoute(tab: .servers, serverId: nil)
        // The main window, and RootView's observer, may be closed to the tray.
        NSApp.activate(ignoringOtherApps: true)
        openSettings()
      }
    }
    CommandGroup(after: .textEditing) {
      Divider()
      Button("Search Messages…") { app.searchOpen.toggle() }.keyboardShortcut("f", modifiers: [.command, .shift])
    }
    CommandMenu("Go") {
      Button("Jump to…") { app.switcherOpen.toggle() }.keyboardShortcut("k")
      Divider()
      Button("Previous Buffer") { app.step(-1) }.keyboardShortcut(.upArrow, modifiers: .option)
      Button("Next Buffer") { app.step(1) }.keyboardShortcut(.downArrow, modifiers: .option)
      Button("Previous Unread") { app.step(-1, unreadOnly: true) }.keyboardShortcut(.upArrow, modifiers: [.option, .shift])
      Button("Next Unread") { app.step(1, unreadOnly: true) }.keyboardShortcut(.downArrow, modifiers: [.option, .shift])
      Divider()
      ForEach(1..<10) { n in
        Button("Channel \(n)") {
          let list = app.allBuffers.filter { $0.kind != .server }
          if n <= list.count { app.open(list[n - 1]) }
        }
        .keyboardShortcut(KeyEquivalent(Character("\(n)")))
      }
    }
    CommandMenu("Conversation") {
      Button(app.active?.kind == .channel ? "Leave Channel" : "Close Conversation") {
        if let b = app.active { app.closeBuffer(b) }
      }
      .keyboardShortcut("w", modifiers: [.command, .shift])
      .disabled(app.active == nil || app.active?.kind == .server)
      Button(app.settings.showMembers ? "Hide Members" : "Show Members") { app.settings.showMembers.toggle() }
        .keyboardShortcut("m", modifiers: [.command, .shift])
      Button("Browse Channels…") { if let c = app.active?.conn { app.browseChannels(c) } }
        .disabled(app.active == nil)
      Divider()
      Button("Mark All as Read") {
        for b in app.allBuffers { app.markRead(b) }
      }
    }
    CommandGroup(replacing: .help) {
      Button("Keyboard Shortcuts") { app.shortcutsOpen.toggle() }.keyboardShortcut("/")
    }
  }
}
