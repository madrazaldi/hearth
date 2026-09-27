// mIRC-formatted text as a SwiftUI Text, with clickable links and channels.

import SwiftUI

/// Channel names in messages link here; `BufferView` joins them.
let channelScheme = "hearth-channel"

func channelURL(_ name: String) -> URL? {
  var c = URLComponents()
  c.scheme = channelScheme
  c.path = name
  return c.url
}

func ircAttributed(_ text: String, palette p: Palette, size: CGFloat, plain: Bool = false) -> AttributedString {
  var out = AttributedString()
  for seg in parseFormatting(text) {
    var run = AttributedString(seg.text)
    var font = Font.system(size: size, weight: seg.bold ? .semibold : .regular, design: seg.mono ? .monospaced : .default)
    if seg.italic { font = font.italic() }
    run.font = font
    if let fg = seg.fg { run.foregroundColor = p.color(fg) }
    if let bg = seg.bg { run.backgroundColor = p.color(bg) }
    if seg.underline { run.underlineStyle = .single }
    if seg.strike { run.strikethroughStyle = .single }
    if !plain {
      if let href = seg.href, let url = URL(string: href) {
        run.link = url
        run.foregroundColor = p.accent
      } else if let chan = seg.channel, let url = channelURL(chan) {
        run.link = url
        run.foregroundColor = p.accent
      }
    }
    out += run
  }
  return out
}

struct RichText: View {
  var text: String
  /// No links, for single-line summaries like the topic bar.
  var plain = false
  var size: CGFloat?
  @Environment(\.palette) private var p
  @Environment(AppModel.self) private var app

  var body: some View {
    Text(ircAttributed(text, palette: p, size: size ?? CGFloat(app.settings.fontSize), plain: plain))
  }
}

/// A nick in its stable colour (or plain text colour when nick colours are off).
struct NickText: View {
  var nick: String
  var weight: Font.Weight = .semibold
  var size: CGFloat?
  @Environment(\.palette) private var p
  @Environment(AppModel.self) private var app

  var body: some View {
    Text(nick)
      .font(.system(size: size ?? CGFloat(app.settings.fontSize), weight: weight))
      .foregroundStyle(app.settings.nickColors ? p.nick(nick) : p.text)
  }
}
