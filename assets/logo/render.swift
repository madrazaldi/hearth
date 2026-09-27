// Rasterizes an SVG to one or more RGBA PNGs using AppKit's built-in SVG
// support (macOS 14+), so the icon set builds without extra tooling.
// Usage: swift render.swift <in.svg> <size>:<out.png> [<size>:<out.png> ...]
import AppKit

let args = CommandLine.arguments.dropFirst()
guard let input = args.first, args.count >= 2 else {
  FileHandle.standardError.write("usage: render.swift in.svg size:out.png ...\n".data(using: .utf8)!)
  exit(2)
}
guard let image = NSImage(contentsOf: URL(fileURLWithPath: input)) else {
  FileHandle.standardError.write("cannot load \(input)\n".data(using: .utf8)!)
  exit(1)
}

for spec in args.dropFirst() {
  let parts = spec.split(separator: ":", maxSplits: 1)
  guard parts.count == 2, let px = Int(parts[0]) else { fatalError("bad spec \(spec)") }
  let out = String(parts[1])
  let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: px, pixelsHigh: px, bitsPerSample: 8,
    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
    bytesPerRow: 0, bitsPerPixel: 0)!
  // Point size == pixel size so the SVG is drawn at exactly 1x into the bitmap.
  rep.size = NSSize(width: px, height: px)
  NSGraphicsContext.saveGraphicsState()
  let ctx = NSGraphicsContext(bitmapImageRep: rep)!
  NSGraphicsContext.current = ctx
  ctx.imageInterpolation = .high
  NSColor.clear.set()
  NSRect(x: 0, y: 0, width: px, height: px).fill(using: .copy)
  // Fit non-square artwork (the glyph-only mark) centered, without stretching.
  let s = Double(px) / max(image.size.width, image.size.height)
  let w = image.size.width * s, h = image.size.height * s
  let dst = NSRect(x: (Double(px) - w) / 2, y: (Double(px) - h) / 2, width: w, height: h)
  image.draw(in: dst, from: .zero, operation: .sourceOver, fraction: 1)
  ctx.flushGraphics()
  NSGraphicsContext.restoreGraphicsState()
  let data = rep.representation(using: .png, properties: [:])!
  try! data.write(to: URL(fileURLWithPath: out))
}
