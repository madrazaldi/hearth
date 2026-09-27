// The ⌘K switcher (QuickSwitcher.svelte): fuzzy-ish matching over every
// buffer, and an offer to join a #channel that isn't open yet.

import AppKit
import SwiftUI

struct QuickSwitcher: View {
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  @LocalState private var q = ""
  @LocalState private var sel = 0

  private enum Target {
    case buffer(Buffer)
    case join(Conn, String)
  }

  /// The id is worked out up front because Buffer and Conn ids are main-actor state.
  private struct Item: Identifiable {
    let id: String
    let target: Target

    @MainActor static func buffer(_ b: Buffer) -> Item { Item(id: "b\u{0}\(b.id)", target: .buffer(b)) }
    @MainActor static func join(_ c: Conn, _ name: String) -> Item { Item(id: "j\u{0}\(c.id)\u{0}\(name)", target: .join(c, name)) }
  }

  private var results: [Item] {
    let t = q.trimmingCharacters(in: .whitespaces)
    var scored: [(item: Item, score: Int, order: Int)] = []
    for (i, b) in app.allBuffers.enumerated() {
      let sc = switcherScore(b.kind == .server ? b.conn.name : b.name, t)
      if sc <= 0 { continue }
      let bonus = (b.highlights > 0 ? 50 : 0) + (b.unread > 0 ? 10 : 0) + (b === app.active ? -40 : 0)
      scored.append((.buffer(b), sc + bonus, i))
    }
    scored.sort { $0.score != $1.score ? $0.score > $1.score : $0.order < $1.order }
    let joins = joinOffers(t)
    return scored.prefix(40 - joins.count).map(\.item) + joins
  }

  /// A #channel that isn't open: offer it on the active network first, then
  /// on the other connected ones.
  private func joinOffers(_ t: String) -> [Item] {
    guard !t.isEmpty, !t.contains(where: \.isWhitespace) else { return [] }
    var conns: [Conn] = []
    if let c = app.active?.conn { conns.append(c) }
    conns += app.orderedConns.filter { c in c.status == .connected && !c.isControl && !conns.contains { $0 === c } }
    return conns.filter { $0.isChannel(t) && $0.buffer(t) == nil }.prefix(4).map { .join($0, t) }
  }

  var body: some View {
    let items = results
    VStack(spacing: 0) {
      HStack(spacing: 10) {
        Image(systemName: "magnifyingglass").foregroundStyle(p.text3)
        PanelField(
          placeholder: "Jump to a channel, conversation or network", text: $q, color: p.text,
          onMove: { move($0, in: items) }, onSubmit: { choose(sel, in: items) }, onCancel: close)
      }
      .padding(.horizontal, 16)
      .frame(height: 52)
      Divider()
      ScrollViewReader { proxy in
        ScrollView {
          LazyVStack(spacing: 0) {
            ForEach(Array(items.enumerated()), id: \.element.id) { i, item in
              row(item, selected: i == sel)
                .contentShape(Rectangle())
                .onTapGesture { choose(i, in: items) }
                .onHover { if $0 { sel = i } }
            }
            if items.isEmpty {
              Text("No matches")
                .foregroundStyle(p.text3)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
            }
          }
          .padding(6)
        }
        .onChange(of: sel) { _, i in
          if i < items.count { proxy.scrollTo(items[i].id) }
        }
      }
      Divider()
      HStack(spacing: 14) {
        PanelHint(keys: "↑↓", label: "navigate")
        PanelHint(keys: "↩", label: "open")
        PanelHint(keys: "esc", label: "close")
        Spacer()
        Text("Tip: type a #channel to join it")
      }
      .font(.system(size: 11.5))
      .foregroundStyle(p.text3)
      .padding(.horizontal, 14)
      .padding(.vertical, 8)
    }
    .frame(width: 560, height: 420)
    .background(p.raised)
    .onChange(of: q) { sel = 0 }
    .onExitCommand(perform: close)
  }

  @ViewBuilder
  private func row(_ item: Item, selected: Bool) -> some View {
    HStack(spacing: 10) {
      switch item.target {
      case .buffer(let b):
        Text(b.kind == .channel ? "#" : b.kind == .query ? "@" : "◆")
          .font(.system(size: 12, design: .monospaced))
          .foregroundStyle(p.text4)
          .frame(width: 14)
        Text(b.kind == .server ? b.conn.name : b.kind == .channel ? String(b.name.drop { $0 == "#" }) : b.name)
          .fontWeight(b.unread > 0 ? .medium : .regular)
          .foregroundStyle(b.unread > 0 || selected ? p.text : p.text2)
          .lineLimit(1)
          .truncationMode(.tail)
        if b.level == NotifyLevel.none {
          Image(systemName: "bell.slash").font(.system(size: 10)).foregroundStyle(p.text4)
        }
        Spacer(minLength: 8)
        if b.highlights > 0 {
          Badge(count: b.highlights, strong: true)
        } else if b.unread > 0 {
          Badge(count: b.unread, strong: false)
        }
        Text(b.kind == .server ? (b.conn.isControl ? "bouncer" : "network") : b.conn.name)
          .font(.system(size: 12))
          .foregroundStyle(p.text3)
          .lineLimit(1)
      case .join(let c, let name):
        Image(systemName: "plus")
          .font(.system(size: 11, weight: .semibold))
          .foregroundStyle(p.accent)
          .frame(width: 14)
        Text("Join \(name)").foregroundStyle(selected ? p.text : p.text2).lineLimit(1)
        Spacer(minLength: 8)
        Text(c.name).font(.system(size: 12)).foregroundStyle(p.text3).lineLimit(1)
      }
    }
    .font(.system(size: 13.5))
    .padding(.horizontal, 10)
    .frame(height: 34)
    .background(RoundedRectangle(cornerRadius: 7).fill(selected ? p.active : .clear))
    .accessibilityElement(children: .combine)
    .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
  }

  private func move(_ delta: Int, in items: [Item]) {
    guard !items.isEmpty else { return }
    sel = min(max(sel + delta, 0), items.count - 1)
  }

  private func choose(_ i: Int, in items: [Item]) {
    guard i < items.count else { return }
    switch items[i].target {
    case .buffer(let b): app.open(b)
    case .join(let c, let name): app.join(c, name)
    }
    close()
  }

  private func close() { app.switcherOpen = false }
}

/// Exact match, then prefix (ignoring #), then substring, then a subsequence
/// that rewards tight runs; 0 means no match.
func switcherScore(_ name: String, _ query: String) -> Int {
  if query.isEmpty { return 1 }
  let n = name.lowercased()
  let s = query.lowercased()
  if n == s { return 1000 }
  let bare = { (x: String) in String(x.drop { $0 == "#" }) }
  if bare(n).hasPrefix(bare(s)) { return 500 - n.count }
  if let r = n.range(of: s) { return 300 - n.distance(from: n.startIndex, to: r.lowerBound) }
  let chars = Array(n)
  var pos = 0
  var gaps = 0
  for ch in s {
    guard let at = chars[pos...].firstIndex(of: ch) else { return 0 }
    gaps += at - pos
    pos = at + 1
  }
  return 100 - gaps
}

/// A key and what it does, for the footers of the overlay panels.
struct PanelHint: View {
  var keys: String
  var label: String
  @Environment(\.palette) private var p

  var body: some View {
    HStack(spacing: 5) {
      KeyCap(text: keys)
      Text(label)
    }
  }
}

/// One key drawn as a keycap.
struct KeyCap: View {
  var text: String
  var size: CGFloat = 11
  @Environment(\.palette) private var p

  var body: some View {
    Text(text)
      .font(.system(size: size, weight: .medium))
      .foregroundStyle(p.text2)
      .padding(.horizontal, 5)
      .frame(minWidth: size + 9, minHeight: size + 7)
      .background(RoundedRectangle(cornerRadius: 4).fill(p.sunken))
      .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(p.lineStrong, lineWidth: 1))
  }
}

/// A borderless search field for the overlay panels. It's AppKit underneath
/// because a focused SwiftUI TextField doesn't reliably hand ↑/↓/↩/esc to
/// `onKeyPress`; the field editor's commands (which include ⌃N/⌃P) are exact.
struct PanelField: NSViewRepresentable {
  var placeholder: String
  @Binding var text: String
  var fontSize: CGFloat = 16
  var color: Color?
  var onMove: (Int) -> Void = { _ in }
  var onSubmit: () -> Void = {}
  var onCancel: () -> Void = {}

  final class Field: NSTextField {
    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      guard let window else { return }
      DispatchQueue.main.async { [weak self] in
        if let self, window.firstResponder !== self.currentEditor() { window.makeFirstResponder(self) }
      }
    }
  }

  final class Coordinator: NSObject, NSTextFieldDelegate {
    var parent: PanelField
    init(_ parent: PanelField) { self.parent = parent }

    func controlTextDidChange(_ note: Notification) {
      guard let f = note.object as? NSTextField else { return }
      parent.text = f.stringValue
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy sel: Selector) -> Bool {
      switch sel {
      case #selector(NSResponder.moveUp(_:)): parent.onMove(-1)
      case #selector(NSResponder.moveDown(_:)): parent.onMove(1)
      case #selector(NSResponder.insertNewline(_:)): parent.onSubmit()
      case #selector(NSResponder.cancelOperation(_:)), #selector(NSTextView.complete(_:)): parent.onCancel()
      default: return false
      }
      return true
    }
  }

  func makeCoordinator() -> Coordinator { Coordinator(self) }

  func makeNSView(context: Context) -> NSTextField {
    let f = Field()
    f.isBordered = false
    f.drawsBackground = false
    f.focusRingType = .none
    f.usesSingleLineMode = true
    f.lineBreakMode = .byTruncatingTail
    f.cell?.isScrollable = true
    f.cell?.wraps = false
    f.font = .systemFont(ofSize: fontSize)
    f.placeholderString = placeholder
    f.delegate = context.coordinator
    f.stringValue = text
    f.setContentHuggingPriority(.defaultLow, for: .horizontal)
    return f
  }

  func updateNSView(_ f: NSTextField, context: Context) {
    context.coordinator.parent = self
    if f.stringValue != text { f.stringValue = text }
    if let color { f.textColor = NSColor(color) }
  }
}
