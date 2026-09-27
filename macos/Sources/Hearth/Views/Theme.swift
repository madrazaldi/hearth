// Colours from src/app.css, so the native app keeps Hearth's warm palette,
// plus nick colours computed in OKLCH like the Svelte app.

import AppKit
import SwiftUI

struct Palette {
  var bg: Color
  var sidebar: Color
  var raised: Color
  var sunken: Color
  var hover: Color
  var active: Color
  var line: Color
  var lineStrong: Color
  var text: Color
  var text2: Color
  var text3: Color
  var text4: Color
  var accent: Color
  var accentText: Color
  var accentSoft: Color
  var mention: Color
  var ok: Color
  var warn: Color
  var danger: Color
  /// mIRC colours 0–15, tuned per theme.
  var irc: [Color]
  var nickL: Double
  var nickC: Double

  static let dark = Palette(
    bg: Color(hex: 0x171513), sidebar: Color(hex: 0x121110), raised: Color(hex: 0x201d1b), sunken: Color(hex: 0x0f0e0d),
    hover: Color(hex: 0xfff4e6, alpha: 0.035), active: Color(hex: 0xfff4e6, alpha: 0.075),
    line: Color(hex: 0xfff0dc, alpha: 0.075), lineStrong: Color(hex: 0xfff0dc, alpha: 0.14),
    text: Color(hex: 0xece6dd), text2: Color(hex: 0xb7afa4), text3: Color(hex: 0x9f968b), text4: Color(hex: 0x8f877c),
    accent: Color(hex: 0xef8b4f), accentText: Color(hex: 0x1a0f08), accentSoft: Color(hex: 0xef8b4f, alpha: 0.14),
    mention: Color(hex: 0xef8b4f, alpha: 0.075), ok: Color(hex: 0x93c283), warn: Color(hex: 0xdcb866),
    danger: Color(hex: 0xe5705f),
    irc: [
      0xf2f2f2, 0x6b6b6b, 0x6f8fd6, 0x7fb069, 0xe5705f, 0xb26a52, 0xb58ad8, 0xe39b4c, 0xe3cf62, 0x98d67f, 0x5fb3b3,
      0x78d2d9, 0x82a6f2, 0xe188c6, 0x8c8c8c, 0xbdbdbd,
    ].map { Color(hex: $0) }, nickL: 0.8, nickC: 0.085)

  static let light = Palette(
    bg: Color(hex: 0xfbf9f6), sidebar: Color(hex: 0xf2eee8), raised: Color(hex: 0xffffff), sunken: Color(hex: 0xebe6de),
    hover: Color(hex: 0x462d14, alpha: 0.04), active: Color(hex: 0x462d14, alpha: 0.075),
    line: Color(hex: 0x462d14, alpha: 0.09), lineStrong: Color(hex: 0x462d14, alpha: 0.16),
    text: Color(hex: 0x221e1a), text2: Color(hex: 0x57504a), text3: Color(hex: 0x6b645b), text4: Color(hex: 0x736b62),
    accent: Color(hex: 0xb34d14), accentText: .white, accentSoft: Color(hex: 0xb34d14, alpha: 0.12),
    mention: Color(hex: 0xb34d14, alpha: 0.06), ok: Color(hex: 0x4d8a3c), warn: Color(hex: 0xa67b12),
    danger: Color(hex: 0xb3382a),
    irc: [
      0xffffff, 0x1e1e1e, 0x2f4fa8, 0x3f7a2c, 0xc2412f, 0x7c3b24, 0x7a3fa5, 0xc06a14, 0xa58a0a, 0x4b9a2d, 0x1f7f7f,
      0x1c8e99, 0x3864d0, 0xb0398d, 0x6f6f6f, 0xa8a8a8,
    ].map { Color(hex: $0) }, nickL: 0.5, nickC: 0.13)

  static func of(_ scheme: ColorScheme) -> Palette { scheme == .dark ? .dark : .light }

  func nick(_ nick: String) -> Color { oklch(l: nickL, c: nickC, h: nickHue(nick)) }

  func color(_ c: IrcColor) -> Color {
    switch c {
    case .palette(let n): irc[n]
    case .rgb(let v): Color(hex: v)
    case .background: bg
    case .foreground: text
    }
  }
}

extension Color {
  init(hex: UInt32, alpha: Double = 1) {
    self.init(
      .sRGB, red: Double((hex >> 16) & 0xff) / 255, green: Double((hex >> 8) & 0xff) / 255,
      blue: Double(hex & 0xff) / 255, opacity: alpha)
  }
}

/// OKLCH to sRGB (Björn Ottosson's OKLab), clamped into gamut.
func oklch(l: Double, c: Double, h: Double) -> Color {
  let a = c * cos(h * .pi / 180)
  let b = c * sin(h * .pi / 180)
  let l_ = pow(l + 0.3963377774 * a + 0.2158037573 * b, 3)
  let m_ = pow(l - 0.1055613458 * a - 0.0638541728 * b, 3)
  let s_ = pow(l - 0.0894841775 * a - 1.2914855480 * b, 3)
  func enc(_ x: Double) -> Double {
    let v = x <= 0.0031308 ? 12.92 * x : 1.055 * pow(x, 1 / 2.4) - 0.055
    return min(max(v, 0), 1)
  }
  return Color(
    .sRGB,
    red: enc(4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_),
    green: enc(-1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_),
    blue: enc(-0.0041960863 * l_ - 0.7034186147 * m_ + 1.7076147010 * s_))
}

private struct PaletteKey: EnvironmentKey {
  static let defaultValue = Palette.dark
}

extension EnvironmentValues {
  var palette: Palette {
    get { self[PaletteKey.self] }
    set { self[PaletteKey.self] = newValue }
  }
}

/// Puts the palette for the effective colour scheme into the environment.
struct Themed: ViewModifier {
  @Environment(\.colorScheme) private var scheme

  func body(content: Content) -> some View {
    content.environment(\.palette, .of(scheme)).tint(Palette.of(scheme).accent)
  }
}

extension View {
  func themed() -> some View { modifier(Themed()) }
}

extension Theme {
  var colorScheme: ColorScheme? {
    switch self {
    case .system: nil
    case .dark: .dark
    case .light: .light
    }
  }
}

/// A small rounded count, as on sidebar rows.
struct Badge: View {
  var count: Int
  var strong: Bool
  @Environment(\.palette) private var p

  var body: some View {
    Text(count > 999 ? "999+" : "\(count)")
      .font(.system(size: 10.5, weight: .semibold).monospacedDigit())
      .padding(.horizontal, 6)
      .frame(minWidth: 18, minHeight: 16)
      .foregroundStyle(strong ? p.accentText : p.text2)
      .background(Capsule().fill(strong ? p.accent : p.active))
  }
}
