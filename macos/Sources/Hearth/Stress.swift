// HEARTH_STRESS=1 opens #stress, a channel with about 10k lines and 2k
// members, for checking scrolling performance (the Svelte mock's ?stress).
// HEARTH_STRESS=live also posts a line every 3 s. HEARTH_STRESS=bench scrolls
// the whole buffer, prints frame timings and quits. Compare instruction
// counts rather than times, which move with thermal throttling, and keep the
// pointer in the same place between runs, since rows under it get a hover.
// Pair it with HEARTH_DATA_DIR so the stress connection sits beside an empty
// profile.

import AppKit

@MainActor
func startStress(_ app: AppModel, mode: String) {
  let id = "stress"
  // In memory only: servers are saved one at a time, and this one never is.
  var server = ServerConfig()
  server.id = id
  server.name = "Stress"
  app.servers.append(server)
  app.handleEvents([
    .connAdded(conn: id, server: id, parent: nil, netid: nil),
    .isupport(conn: id, isupport: ["NETWORK": "Stress", "CHANTYPES": "#", "PREFIX": "(ov)@+"]),
    .registered(conn: id, nick: "ember", caps: ["message-tags", "server-time"], capValues: [:]),
  ])
  guard let conn = app.conns[id] else { return }
  conn.status = .connected
  let buf = conn.ensure("#stress", .channel)
  buf.joined = true
  buf.topic = "10k lines for checking scroll performance"
  var members: [String: Member] = [:]
  for (i, n) in stressNicks.prefix(2000).enumerated() {
    members[conn.fold(n)] = Member(nick: n, prefix: i < 12 ? "@" : i < 60 ? "+" : "", away: false)
  }
  buf.members = members
  buf.messages = (0..<10_000).reversed().compactMap { stressLine($0) }
  buf.history = .done
  app.open(buf)

  if mode == "live" {
    Task {
      var n = 0
      while true {
        try? await Task.sleep(for: .seconds(3))
        n += 1
        let nick = stressNicks[1 + n % 50]
        buf.push(Line(type: .privmsg, nick: nick, text: "live \(n) " + words(n, count: 4 + n % 12)))
      }
    }
  } else if mode == "bench" {
    Task {
      try? await Task.sleep(for: .seconds(2))
      await benchScroll()
      fflush(stdout)
      NSApp.terminate(nil)
    }
  }
}

private let stressNicks: [String] = (0..<2000).map { i in
  let a = ["ash", "birch", "cedar", "dune", "elm", "fern", "gale", "heath", "iris", "juno", "kite", "lark", "moss", "nova", "onyx", "pike"]
  return i == 0 ? "ember" : "\(a[i % a.count])\(i % 7 == 0 ? "_" : "")\(i / a.count == 0 ? "" : String(i / a.count))"
}

private let stressWords =
  "the a scroll anchor websocket parser buffer render frame layout paint row height measure virtual list prepend history bouncer cap sasl batch label nick channel topic kettle tea patch review merge ship release build tokio actor webkit"
  .split(separator: " ").map(String.init)

/// A small deterministic generator, so line `i` looks the same on every run.
private struct Rng {
  var state: UInt64
  init(_ seed: Int) { state = UInt64(truncatingIfNeeded: seed) &* 0x9E37_79B9_7F4A_7C15 | 1 }
  mutating func next() -> Double {
    state ^= state << 13
    state ^= state >> 7
    state ^= state << 17
    return Double(state % 1_000_000) / 1_000_000
  }
}

private func words(_ seed: Int, count: Int) -> String {
  var r = Rng(seed)
  return (0..<count).map { _ in stressWords[Int(r.next() * Double(stressWords.count))] }.joined(separator: " ")
}

private let stressEnd = Int(Date().timeIntervalSince1970 * 1000) - 100 * 60_000

/// Line `i` counts back from the newest (0).
@MainActor
private func stressLine(_ i: Int) -> Line? {
  var r = Rng(i)
  let nick = stressNicks[1 + Int(r.next() * r.next() * 400)]
  let time = stressEnd - i * 31_000 - (i % 5) * 1000
  let k = r.next()
  if k < 0.05 {
    let types: [LineType] = [.join, .part, .quit]
    return Line(type: types[Int(r.next() * 3)], time: time, nick: nick, text: k < 0.017 ? "" : "bye", msgid: "s\(i)")
  }
  var text = words(i &* 31, count: 3 + Int(r.next() * (r.next() < 0.1 ? 90 : 18)))
  var highlight = false
  if r.next() < 0.03 {
    text = "ember: " + text
    highlight = true
  }
  if r.next() < 0.02 { text += " https://hearth.example/notes/\(i)" }
  if i % 41 == 7 { text += " https://img.example/s/\(i).png" }
  if i % 23 == 3 { text = "\u{02}bold\u{02} \u{03}4red\u{03} " + text + " in #hearth" }
  let type: LineType = k < 0.08 ? .notice : k < 0.12 ? .action : .privmsg
  var l = Line(
    type: type, time: time, nick: nick, text: type == .action ? text : "#\(i) \(text)", highlight: highlight,
    msgid: "s\(i)", replyTo: r.next() < 0.03 ? "s\(i + 1 + Int(r.next() * 4))" : nil)
  if r.next() < 0.02 { l.reactions = [Reaction(emoji: "👍", nicks: [stressNicks[7]])] }
  return l
}

/// Scrolls the message list from the bottom to the top at a steady speed and
/// reports how long each frame took.
@MainActor
private func benchScroll() async {
  guard let window = NSApp.windows.first(where: { $0.isVisible && $0.contentView != nil }),
    let scroll = largestScrollView(in: window.contentView!)
  else {
    print("bench: no scroll view")
    return
  }
  let clip = scroll.contentView
  var stamps: [CFTimeInterval] = []
  let start = CACurrentMediaTime()
  let cpuStart = clock_gettime_nsec_np(CLOCK_THREAD_CPUTIME_ID)
  let insStart = instructions()
  var y = clip.bounds.origin.y
  // 40 px per step, about 2400 pt/s at 60 Hz: a fast trackpad fling.
  for _ in 0..<500 {
    y = max(0, y - 40)
    clip.scroll(to: NSPoint(x: 0, y: y))
    scroll.reflectScrolledClipView(clip)
    // Let SwiftUI lay out and draw before the next step.
    try? await Task.sleep(for: .milliseconds(1))
    CATransaction.flush()
    window.displayIfNeeded()
    stamps.append(CACurrentMediaTime())
  }
  let total = CACurrentMediaTime() - start
  let cpu = Double(clock_gettime_nsec_np(CLOCK_THREAD_CPUTIME_ID) - cpuStart) / 1e6
  let ins = Double(instructions() - insStart) / 1e9
  let frames = zip(stamps.dropFirst(), stamps).map { ($0 - $1) * 1000 }.sorted()
  let avg = frames.reduce(0, +) / Double(frames.count)
  let p95 = frames[Int(Double(frames.count) * 0.95)]
  let p99 = frames[Int(Double(frames.count) * 0.99)]
  let slow = frames.filter { $0 > 16.7 }.count
  print(
    String(
      format: "bench: %.2fG instructions · cpu %.0fms · %d steps in %.2fs · avg %.2fms · p95 %.2fms · p99 %.2fms · max %.2fms · >16.7ms %d",
      ins, cpu, stamps.count, total, avg, p95, p99, frames.last ?? 0, slow))
}

@MainActor
private func largestScrollView(in view: NSView) -> NSScrollView? {
  var best: NSScrollView?
  func walk(_ v: NSView) {
    if let s = v as? NSScrollView, s.documentView != nil,
      (best?.documentView?.frame.height ?? 0) < s.documentView!.frame.height
    {
      best = s
    }
    v.subviews.forEach(walk)
  }
  walk(view)
  return best
}

/// Instructions retired by the whole process, which unlike time doesn't move with the clock speed.
private func instructions() -> UInt64 {
  var info = rusage_info_v4()
  let ok = withUnsafeMutablePointer(to: &info) {
    $0.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) { proc_pid_rusage(getpid(), RUSAGE_INFO_V4, $0) }
  }
  return ok == 0 ? info.ri_instructions : 0
}
