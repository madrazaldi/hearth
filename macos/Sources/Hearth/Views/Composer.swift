// The message box (Composer.svelte): Enter sends, Shift+Enter breaks the line,
// Tab completes commands, channels and nicks, `:name` suggests emoji, ↑/↓ walk the
// input history, and pasted or dropped files go to the server's FILEHOST.

import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct Composer: View {
  let buf: Buffer
  @Environment(AppModel.self) private var app
  @Environment(ComposerBridge.self) private var bridge: ComposerBridge?
  @Environment(\.palette) private var p
  @LocalState private var text = ""
  @LocalState private var height: CGFloat = 22
  @LocalState private var cmdIndex = 0
  @LocalState private var uploading = 0
  @LocalState private var dragging = false
  @LocalState private var focusSeq = 0
  @LocalState private var caret = 0
  @LocalState private var emojiIndex = 0
  // The colon position whose suggestions Escape closed.
  @LocalState private var emojiDismissed = -1

  private var conn: Conn { buf.conn }
  private var filehost: String { conn.isupport["soju.im/FILEHOST"] ?? conn.isupport["FILEHOST"] ?? "" }

  private var placeholder: String {
    if buf.kind == .server { return "Type /help for commands" }
    if buf.kind == .channel && !buf.joined { return "You're not in \(buf.name)" }
    return "Message \(buf.name)"
  }

  private var cmdMatches: [CommandInfo] {
    guard text.hasPrefix("/"), case let q = text.dropFirst().lowercased(),
      q.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "_" || $0 == "-" })
    else { return [] }
    return Array(commandInfos(app.settings.aliases).filter { $0.name.hasPrefix(q) }.prefix(7))
  }

  struct EmojiMatch {
    let start: Int
    let query: String
    let matches: [Emoji]
  }

  private func emojiMatch(_ cmds: [CommandInfo]) -> EmojiMatch? {
    guard cmds.isEmpty else { return nil }
    let ns = text as NSString
    guard let q = Emojis.query(ns.substring(to: min(caret, ns.length))), q.start != emojiDismissed else { return nil }
    let matches = Emojis.search(q.query)
    return matches.isEmpty ? nil : EmojiMatch(start: q.start, query: q.query, matches: matches)
  }

  var body: some View {
    let matches = cmdMatches
    let emoji = emojiMatch(matches)
    VStack(spacing: 0) {
      if !matches.isEmpty {
        CommandSuggestions(matches: matches, index: min(cmdIndex, matches.count - 1)) { text = "/\($0.name) " }
      }
      if let emoji {
        EmojiSuggestions(matches: emoji.matches, index: min(emojiIndex, emoji.matches.count - 1)) {
          pickEmoji($0, from: emoji.start)
          focusSeq += 1
        }
      }
      if let reply = buf.replyTo {
        HStack(spacing: 6) {
          Image(systemName: "arrowshape.turn.up.left").foregroundStyle(p.text4)
          Text("Replying to").foregroundStyle(p.text3)
          NickText(nick: reply.nick, size: 12)
          Text(stripFormatting(reply.text)).foregroundStyle(p.text3).lineLimit(1)
          Spacer()
          Button {
            buf.replyTo = nil
          } label: {
            Image(systemName: "xmark.circle.fill")
          }
          .buttonStyle(.plain)
          .foregroundStyle(p.text4)
          .help("Cancel reply (Esc)")
        }
        .font(.system(size: 12))
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
      }
      HStack(alignment: .bottom, spacing: 8) {
        if !filehost.isEmpty {
          Button(action: pickFile) {
            Image(systemName: "paperclip").font(.system(size: 14))
          }
          .buttonStyle(.plain)
          .foregroundStyle(p.text3)
          .help("Upload a file")
          .padding(.bottom, 3)
        }
        ZStack(alignment: .topLeading) {
          if text.isEmpty {
            Text(placeholder).foregroundStyle(p.text4).padding(.leading, 5).allowsHitTesting(false)
          }
          ComposerField(
            text: $text, height: $height, focusSeq: focusSeq, insert: bridge?.insert,
            spellcheck: app.settings.spellcheck,
            fontSize: CGFloat(app.settings.fontSize),
            textColor: NSColor(p.text),
            keys: keyHandler(matches, emoji),
            onChange: changed,
            onCaret: { if caret != $0 { caret = $0 } },
            onFiles: { files in upload(files) }
          )
          .frame(height: height)
        }
        .font(.system(size: CGFloat(app.settings.fontSize)))
        if uploading > 0 {
          ProgressView().controlSize(.small).help("Uploading").padding(.bottom, 2)
        }
      }
      .padding(.horizontal, 12)
      .padding(.vertical, 9)
      .background(RoundedRectangle(cornerRadius: 10).fill(p.sunken))
      .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(dragging ? p.accent : p.lineStrong))
    }
    .padding(.horizontal, 14)
    .padding(.bottom, 12)
    .onDrop(of: [.fileURL], isTargeted: $dragging) { providers in
      for provider in providers {
        _ = provider.loadObject(ofClass: URL.self) { url, _ in
          guard let url else { return }
          Task { @MainActor in upload([url]) }
        }
      }
      return true
    }
    .onAppear {
      text = buf.draft
      focusSeq += 1
    }
    .onChange(of: text) {
      buf.draft = text
      cmdIndex = 0
    }
    .onChange(of: emoji.map { "\($0.start):\($0.query)" }) { emojiIndex = 0 }
    .onChange(of: bridge?.focus) { focusSeq += 1 }
    .onChange(of: app.composerFocus) { focusSeq += 1 }
    .onChange(of: buf.replyTo?.id) { if buf.replyTo != nil { focusSeq += 1 } }
  }

  private func changed(_ tv: NSTextView) {
    memo.tv = tv
    history = -1
    let ns = tv.string as NSString
    let pos = min(tv.selectedRange().location, ns.length)
    let before = ns.substring(to: pos)
    if !tv.hasMarkedText(), let sc = Emojis.typed(before) {
      replace(tv, NSRange(location: sc.start, length: pos - sc.start), with: sc.char)
    } else if Emojis.query(before) == nil {
      emojiDismissed = -1
    }
    if text.hasPrefix("/") || text.isEmpty { app.sendTyping(buf, .done) } else { app.sendTyping(buf, .active) }
  }

  // Held outside view state: they change on every keystroke and nothing renders them.
  @LocalState private var memo = Memo()
  private final class Memo {
    var history = -1
    var completion: Completion?
    weak var tv: NSTextView?
  }
  private var history: Int {
    get { memo.history }
    nonmutating set { memo.history = newValue }
  }

  private func keyHandler(_ matches: [CommandInfo], _ emoji: EmojiMatch?) -> (ComposerKey, NSTextView) -> Bool {
    { key, tv in
      memo.tv = tv
      if key != .tab && key != .backtab { memo.completion = nil }
      if let emoji {
        let i = min(emojiIndex, emoji.matches.count - 1)
        switch key {
        case .up, .down:
          emojiIndex = (i + (key == .up ? -1 : 1) + emoji.matches.count) % emoji.matches.count
          return true
        // Enter still sends a lone `:D` or `:P`; Tab always picks.
        case .tab, .backtab:
          pickEmoji(emoji.matches[i], from: emoji.start)
          return true
        case .enter where emoji.query.count > 1:
          pickEmoji(emoji.matches[i], from: emoji.start)
          return true
        case .escape:
          emojiDismissed = emoji.start
          return true
        default: break
        }
      }
      if !matches.isEmpty {
        let i = min(cmdIndex, matches.count - 1)
        switch key {
        case .up, .down:
          cmdIndex = (i + (key == .up ? -1 : 1) + matches.count) % matches.count
          return true
        case .tab, .backtab:
          text = "/\(matches[i].name) "
          return true
        case .enter where matches[i].name != String(text.dropFirst()):
          text = "/\(matches[i].name) "
          return true
        default: break
        }
      }
      switch key {
      case .enter:
        submit()
        return true
      case .tab, .backtab:
        return complete(tv, backwards: key == .backtab)
      case .up:
        let caret = tv.selectedRange().location
        let before = (text as NSString).substring(to: min(caret, (text as NSString).length))
        if before.contains("\n") { return false }
        if history + 1 < app.inputHistory.count && (text.isEmpty || history >= 0) {
          let h = history + 1
          text = app.inputHistory[h]
          history = h
          return true
        }
        return false
      case .down:
        let end = NSMaxRange(tv.selectedRange())
        let after = (text as NSString).substring(from: min(end, (text as NSString).length))
        if history < 0 || after.contains("\n") { return false }
        let h = history - 1
        text = h >= 0 ? app.inputHistory[h] : ""
        history = h
        return true
      case .escape:
        if buf.replyTo != nil {
          buf.replyTo = nil
          return true
        }
        return false
      }
    }
  }

  private func submit() {
    let value = text
    guard !value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
    let replyTo = value.hasPrefix("/") && !value.hasPrefix("//") ? nil : buf.replyTo?.msgid
    Task {
      if await app.runInput(buf, value, replyTo: replyTo) {
        app.inputHistory = Array(([value] + app.inputHistory.filter { $0 != value }).prefix(100))
        if replyTo != nil { buf.replyTo = nil }
        if text == value { text = "" }
        history = -1
        app.sendTyping(buf, .done)
      }
    }
  }

  // ─── Tab completion ──────────────────────────────────────────

  struct Completion {
    var start: Int
    var end: Int
    var options: [String]
    var index: Int
    var suffix: String
  }

  private func recentSpeakers() -> [String] {
    var seen: [String] = []
    for l in buf.messages.suffix(400).reversed() where !l.nick.isEmpty && !l.isSelf && !seen.contains(l.nick) {
      seen.append(l.nick)
      if seen.count >= 50 { break }
    }
    return seen
  }

  /// False when there's nothing to complete, so Tab can move focus instead.
  private func complete(_ tv: NSTextView, backwards: Bool) -> Bool {
    let ns = text as NSString
    let caret = tv.selectedRange().location
    if var c = memo.completion, caret == c.end {
      c.index = (c.index + (backwards ? -1 : 1) + c.options.count) % c.options.count
      apply(&c, tv)
      return true
    }
    let before = ns.substring(to: min(caret, ns.length))
    let start = (before.rangeOfCharacter(from: .whitespacesAndNewlines, options: .backwards)?.upperBound)
      .map { before.utf16.distance(from: before.utf16.startIndex, to: $0) } ?? 0
    let word = (before as NSString).substring(from: start)
    if word.isEmpty { return false }
    let lw = word.lowercased()
    var options: [String] = []
    var suffix = " "
    if start == 0 && word.hasPrefix("/") {
      options = commandInfos(app.settings.aliases).filter { $0.name.hasPrefix(lw.dropFirst()) }.map { "/" + $0.name }
    } else if conn.isChannel(word) {
      options = conn.buffers.values.filter { $0.kind == .channel && $0.name.lowercased().hasPrefix(lw) }.map(\.name)
        .sorted()
    } else {
      var all = buf.members.values.map(\.nick)
      if buf.kind == .query { all.append(buf.name) }
      var seen = Set<String>()
      options = (recentSpeakers() + all.sorted { $0.localizedCompare($1) == .orderedAscending })
        .filter { seen.insert($0).inserted && $0.lowercased().hasPrefix(lw) && !conn.isMe($0) }
      if start == 0 { suffix = ": " }
    }
    if options.isEmpty { return false }
    var c = Completion(start: start, end: caret, options: options, index: backwards ? options.count - 1 : 0, suffix: suffix)
    apply(&c, tv)
    return true
  }

  private func apply(_ c: inout Completion, _ tv: NSTextView) {
    let ns = text as NSString
    let insert = c.options[c.index] + c.suffix
    var after = ns.substring(from: min(c.end, ns.length))
    if after.hasPrefix(": ") { after.removeFirst(2) } else if after.hasPrefix(" ") { after.removeFirst() }
    text = ns.substring(to: c.start) + insert + after
    c.end = c.start + (insert as NSString).length
    memo.completion = c
    tv.string = text
    tv.setSelectedRange(NSRange(location: c.end, length: 0))
  }

  // ─── Emoji ───────────────────────────────────────────────────

  private func pickEmoji(_ e: Emoji, from start: Int) {
    guard let tv = memo.tv else { return }
    let ns = tv.string as NSString
    let end = min(tv.selectedRange().location, ns.length)
    guard start <= end else { return }
    var range = NSRange(location: start, length: end - start)
    if end < ns.length && ns.character(at: end) == 0x20 { range.length += 1 }
    replace(tv, range, with: e.char + " ")
  }

  /// Edits through the text system so undo can take it back.
  private func replace(_ tv: NSTextView, _ range: NSRange, with s: String) {
    guard tv.shouldChangeText(in: range, replacementString: s) else { return }
    tv.replaceCharacters(in: range, with: s)
    tv.didChangeText()
    tv.setSelectedRange(NSRange(location: range.location + (s as NSString).length, length: 0))
  }

  // ─── Uploads ─────────────────────────────────────────────────

  private func insertText(_ s: String) {
    let sep = !text.isEmpty && !text.hasSuffix(" ") ? " " : ""
    text += sep + s
    focusSeq += 1
  }

  private func pickFile() {
    let panel = NSOpenPanel()
    panel.allowsMultipleSelection = true
    panel.canChooseDirectories = false
    if panel.runModal() == .OK { upload(panel.urls) }
  }

  private func upload(_ files: [ComposerFile]) {
    guard !filehost.isEmpty else {
      app.toast("This server does not accept file uploads", .error)
      return
    }
    for f in files {
      uploading += 1
      Task {
        defer { uploading -= 1 }
        do {
          let url: String
          switch f {
          case .path(let u):
            url = try await app.backend.uploadFile(server: conn.serverId, url: filehost, path: u.path)
          case .data(let d, let name, let type):
            url = try await app.backend.uploadBytes(
              server: conn.serverId, url: filehost, name: name, contentType: type, data: d)
          }
          insertText(url)
        } catch {
          app.toast("Upload failed: \(error.localizedDescription)", .error)
        }
      }
    }
  }

  private func upload(_ urls: [URL]) { upload(urls.map { ComposerFile.path($0) }) }
}

enum ComposerFile {
  case path(URL)
  case data(Data, name: String, type: String)
}

enum ComposerKey { case enter, tab, backtab, up, down, escape }

private struct EmojiSuggestions: View {
  let matches: [Emoji]
  let index: Int
  let pick: (Emoji) -> Void
  @Environment(\.palette) private var p

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      ForEach(Array(matches.enumerated()), id: \.element.name) { i, e in
        Button {
          pick(e)
        } label: {
          HStack(spacing: 10) {
            Text(e.char).font(.system(size: 15)).frame(width: 20)
            Text(":\(e.name):").font(.system(size: 12, weight: .medium, design: .monospaced)).foregroundStyle(p.text)
            Spacer(minLength: 0)
          }
          .padding(.horizontal, 10)
          .padding(.vertical, 4)
          .background(RoundedRectangle(cornerRadius: 6).fill(i == index ? p.active : .clear))
          .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
      }
    }
    .padding(4)
    .background(RoundedRectangle(cornerRadius: 9).fill(p.raised))
    .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(p.line))
    .padding(.bottom, 6)
  }
}

private struct CommandSuggestions: View {
  let matches: [CommandInfo]
  let index: Int
  let pick: (CommandInfo) -> Void
  @Environment(\.palette) private var p

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      ForEach(Array(matches.enumerated()), id: \.element.id) { i, c in
        Button {
          pick(c)
        } label: {
          HStack(spacing: 10) {
            Text(c.usage).font(.system(size: 12, weight: .medium, design: .monospaced)).foregroundStyle(p.text)
            Spacer(minLength: 12)
            Text(c.help).font(.system(size: 11.5)).foregroundStyle(p.text3).lineLimit(1)
          }
          .padding(.horizontal, 10)
          .padding(.vertical, 5)
          .background(RoundedRectangle(cornerRadius: 6).fill(i == index ? p.active : .clear))
          .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
      }
    }
    .padding(4)
    .background(RoundedRectangle(cornerRadius: 9).fill(p.raised))
    .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(p.line))
    .padding(.bottom, 6)
  }
}

// ─── The AppKit text view ────────────────────────────────────────

private struct ComposerField: NSViewRepresentable {
  @Binding var text: String
  @Binding var height: CGFloat
  var focusSeq: Int
  var insert: (text: String, seq: Int)?
  var spellcheck: Bool
  var fontSize: CGFloat
  var textColor: NSColor
  var keys: (ComposerKey, NSTextView) -> Bool
  var onChange: (NSTextView) -> Void
  var onCaret: (Int) -> Void
  var onFiles: ([ComposerFile]) -> Void

  func makeCoordinator() -> Coordinator { Coordinator(self) }

  func makeNSView(context: Context) -> NSScrollView {
    let scroll = NSScrollView()
    scroll.drawsBackground = false
    scroll.hasVerticalScroller = true
    scroll.autohidesScrollers = true
    scroll.borderType = .noBorder
    let tv = ComposerTextView()
    tv.delegate = context.coordinator
    tv.onFiles = { onFiles($0) }
    tv.drawsBackground = false
    tv.isRichText = false
    tv.allowsUndo = true
    tv.importsGraphics = false
    tv.isAutomaticQuoteSubstitutionEnabled = false
    tv.isAutomaticDashSubstitutionEnabled = false
    tv.isAutomaticTextReplacementEnabled = false
    tv.isAutomaticSpellingCorrectionEnabled = false
    tv.textContainerInset = NSSize(width: 0, height: 2)
    tv.textContainer?.lineFragmentPadding = 5
    tv.isVerticallyResizable = true
    tv.isHorizontallyResizable = false
    tv.autoresizingMask = [.width]
    tv.textContainer?.widthTracksTextView = true
    tv.string = text
    scroll.documentView = tv
    context.coordinator.tv = tv
    return scroll
  }

  func updateNSView(_ scroll: NSScrollView, context: Context) {
    let c = context.coordinator
    c.parent = self
    guard let tv = c.tv else { return }
    tv.onFiles = { onFiles($0) }
    tv.font = .systemFont(ofSize: fontSize)
    tv.textColor = textColor
    tv.insertionPointColor = textColor
    tv.isContinuousSpellCheckingEnabled = spellcheck
    if tv.string != text {
      tv.string = text
      tv.setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
    }
    if let insert, insert.seq != c.insertSeq {
      c.insertSeq = insert.seq
      tv.insertText(insert.text, replacementRange: tv.selectedRange())
      focus(tv)
    }
    if focusSeq != c.focusSeq {
      c.focusSeq = focusSeq
      focus(tv)
    }
    DispatchQueue.main.async { c.measure() }
  }

  private func focus(_ tv: NSTextView) {
    DispatchQueue.main.async { tv.window?.makeFirstResponder(tv) }
  }

  @MainActor
  final class Coordinator: NSObject, NSTextViewDelegate {
    var parent: ComposerField
    weak var tv: ComposerTextView?
    var focusSeq = 0
    var insertSeq = 0

    init(_ parent: ComposerField) {
      self.parent = parent
      self.insertSeq = parent.insert?.seq ?? 0
    }

    func textDidChange(_ notification: Notification) {
      guard let tv else { return }
      parent.text = tv.string
      parent.onChange(tv)
      measure()
    }

    func textViewDidChangeSelection(_ notification: Notification) {
      guard let tv else { return }
      // Also fires while updateNSView sets the text, where SwiftUI state can't change.
      let loc = tv.selectedRange().location
      DispatchQueue.main.async { [parent] in parent.onCaret(loc) }
    }

    /// Grow with the text up to about ten lines, then scroll.
    func measure() {
      guard let tv, let lm = tv.layoutManager, let tc = tv.textContainer else { return }
      lm.ensureLayout(for: tc)
      let h = lm.usedRect(for: tc).height + tv.textContainerInset.height * 2
      let line = (tv.font ?? .systemFont(ofSize: 13)).boundingRectForFont.height
      let clamped = min(max(h, line + 4), line * 10)
      if abs(clamped - parent.height) > 0.5 { parent.height = clamped }
    }

    func textView(_ textView: NSTextView, doCommandBy sel: Selector) -> Bool {
      let key: ComposerKey?
      switch sel {
      case #selector(NSResponder.insertNewline(_:)):
        // Shift+Enter (and ⌥Enter) add a line; plain Enter sends.
        let flags = NSApp.currentEvent?.modifierFlags ?? []
        key = flags.contains(.shift) || flags.contains(.option) ? nil : .enter
        if key == nil {
          textView.insertNewlineIgnoringFieldEditor(nil)
          return true
        }
      case #selector(NSResponder.insertTab(_:)): key = .tab
      case #selector(NSResponder.insertBacktab(_:)): key = .backtab
      case #selector(NSResponder.moveUp(_:)): key = .up
      case #selector(NSResponder.moveDown(_:)): key = .down
      case #selector(NSResponder.cancelOperation(_:)): key = .escape
      default: key = nil
      }
      guard let key else { return false }
      if textView.hasMarkedText() { return false }
      if parent.keys(key, textView) { return true }
      // Nothing to complete: let Tab move focus like it does elsewhere.
      if key == .tab { textView.window?.selectNextKeyView(nil); return true }
      if key == .backtab { textView.window?.selectPreviousKeyView(nil); return true }
      return false
    }
  }
}

/// Takes pasted images and dropped files for upload instead of inserting them.
final class ComposerTextView: NSTextView {
  var onFiles: (([ComposerFile]) -> Void)?

  override func paste(_ sender: Any?) {
    let pb = NSPasteboard.general
    if let urls = pb.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL],
      !urls.isEmpty
    {
      onFiles?(urls.map { .path($0) })
      return
    }
    if pb.string(forType: .string) == nil, let png = pngData(pb) {
      onFiles?([.data(png, name: "pasted.png", type: "image/png")])
      return
    }
    pasteAsPlainText(sender)
  }

  private func pngData(_ pb: NSPasteboard) -> Data? {
    if let d = pb.data(forType: .png) { return d }
    guard let img = NSImage(pasteboard: pb), let tiff = img.tiffRepresentation,
      let rep = NSBitmapImageRep(data: tiff)
    else { return nil }
    return rep.representation(using: .png, properties: [:])
  }

  override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
    let pb = sender.draggingPasteboard
    if let urls = pb.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL],
      !urls.isEmpty
    {
      onFiles?(urls.map { .path($0) })
      return true
    }
    return super.performDragOperation(sender)
  }
}
