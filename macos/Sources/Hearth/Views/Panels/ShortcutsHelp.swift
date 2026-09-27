// The keyboard shortcut list (ShortcutsHelp.svelte, shortcuts.ts). Keep it in
// sync with HearthCommands and the buffer and composer key handlers.

import SwiftUI

struct KeyboardShortcutGroup {
  struct Item {
    /// Alternative combos, each drawn as one keycap.
    var keys: [String]
    var label: String
  }
  var title: String
  var items: [Item]
}

let keyboardShortcutGroups: [KeyboardShortcutGroup] = [
  KeyboardShortcutGroup(
    title: "Navigation",
    items: [
      .init(keys: ["⌘K"], label: "Jump to a buffer, or join a #channel"),
      .init(keys: ["⌥↑", "⌥↓"], label: "Previous / next buffer"),
      .init(keys: ["⌥⇧↑", "⌥⇧↓"], label: "Previous / next unread"),
      .init(keys: ["⌘1–9"], label: "Go to channel by position"),
      .init(keys: ["⌘⇧W"], label: "Close or leave the buffer"),
    ]),
  KeyboardShortcutGroup(
    title: "Panels",
    items: [
      .init(keys: ["⌘F"], label: "Filter messages in this buffer"),
      .init(keys: ["⌘⇧F"], label: "Search all messages"),
      .init(keys: ["⌘⇧M"], label: "Show or hide the member list"),
      .init(keys: ["⌘,"], label: "Settings"),
      .init(keys: ["⌘/"], label: "Keyboard shortcuts"),
      .init(keys: ["esc"], label: "Close a panel, cancel a reply"),
    ]),
  KeyboardShortcutGroup(
    title: "Composer",
    items: [
      .init(keys: ["⇥", "⇧⇥"], label: "Complete a nick, channel, command or alias"),
      .init(keys: ["↑", "↓"], label: "Input history"),
      .init(keys: ["⇧↩"], label: "New line"),
    ]),
]

/// The groups of shortcuts, for the help sheet and anywhere else that lists them.
struct KeyboardShortcutList: View {
  @Environment(\.palette) private var p

  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      ForEach(keyboardShortcutGroups, id: \.title) { g in
        VStack(alignment: .leading, spacing: 8) {
          Text(g.title.uppercased())
            .font(.system(size: 11, weight: .semibold))
            .tracking(0.6)
            .foregroundStyle(p.text3)
          Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 9) {
            ForEach(g.items, id: \.label) { s in
              GridRow {
                HStack(spacing: 4) {
                  ForEach(Array(s.keys.enumerated()), id: \.offset) { i, combo in
                    if i > 0 { Text("/").font(.system(size: 12)).foregroundStyle(p.text4) }
                    KeyCap(text: combo, size: 12)
                  }
                }
                .frame(width: 140, alignment: .leading)
                Text(s.label).foregroundStyle(p.text2)
              }
              .accessibilityElement(children: .combine)
            }
          }
          .font(.system(size: 13.5))
        }
      }
    }
  }
}

struct ShortcutsHelp: View {
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      Text("Keyboard Shortcuts")
        .font(.system(size: 18, weight: .semibold))
        .foregroundStyle(p.text)
        .padding(.bottom, 18)
      ScrollView {
        KeyboardShortcutList().frame(maxWidth: .infinity, alignment: .leading)
      }
      .scrollBounceBehavior(.basedOnSize)
      HStack {
        Spacer()
        Button("Done", action: close)
          .keyboardShortcut(.defaultAction)
      }
      .padding(.top, 16)
      // Esc closes too, without a second visible button.
      .background {
        Button("Close", action: close).keyboardShortcut(.cancelAction).opacity(0).accessibilityHidden(true)
      }
    }
    .padding(.horizontal, 26)
    .padding(.top, 22)
    .padding(.bottom, 20)
    .frame(width: 500, height: 560)
    .background(p.bg)
  }

  private func close() { app.shortcutsOpen = false }
}
