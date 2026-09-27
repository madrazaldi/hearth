// A buffer's lines as rows: day and unread dividers, grouped runs of
// joins/parts (src/lib/timeline.ts).

import Foundation

enum TimelineItem: Identifiable, Equatable {
  case day(key: String, time: Int, label: String)
  case unread(time: Int)
  case line(Line, cont: Bool)
  case run([Line])

  var id: String {
    switch self {
    case .day(let key, _, _): key
    case .unread: "u"
    case .line(let l, _): "l\(l.id)"
    case .run(let lines): "r\(lines[0].id)"
    }
  }

  var time: Int {
    switch self {
    case .day(_, let t, _), .unread(let t): t
    case .line(let l, _): l.time
    case .run(let lines): lines[0].time
    }
  }

  func contains(_ lineId: Int) -> Bool {
    switch self {
    case .line(let l, _): l.id == lineId
    case .run(let lines): lines.contains { $0.id == lineId }
    default: false
    }
  }
}

/// Turn a buffer's lines into rows. `divider`: lines newer than this (and not
/// ours) get a "New" divider before them; 0 for none. Membership changes are
/// only grouped in channels.
func buildTimeline(_ lines: [Line], divider: Int, channel: Bool, membership: Membership) -> [TimelineItem] {
  var out: [TimelineItem] = []
  var prev: Line?
  var run: [Line] = []
  var dividerPlaced = divider == 0
  var dayStart = Int.max
  var dayEnd = Int.min
  let cal = Calendar.current

  func flush() {
    guard !run.isEmpty else { return }
    if run.count == 1 || membership == .show {
      out += run.map { .line($0, cont: false) }
    } else {
      out.append(.run(run))
    }
    run = []
  }

  for l in lines {
    if l.time < dayStart || l.time >= dayEnd {
      let start = cal.startOfDay(for: date(l.time))
      dayStart = Int(start.timeIntervalSince1970 * 1000)
      dayEnd = Int((cal.date(byAdding: .day, value: 1, to: start) ?? start).timeIntervalSince1970 * 1000)
      flush()
      out.append(.day(key: "d\(l.id)", time: l.time, label: formatDay(l.time)))
      prev = nil
    }
    if !dividerPlaced && l.time > divider && !l.isSelf {
      flush()
      out.append(.unread(time: l.time))
      dividerPlaced = true
      prev = nil
    }
    if channel && l.type.isMembership {
      run.append(l)
      prev = l
      continue
    }
    flush()
    // A reply shows its quote under a full header, so it starts a new group.
    let cont =
      prev.map { p in
        l.replyTo == nil && l.type.isTalk && p.type == l.type && l.type != .action && p.nick == l.nick
          && l.time - p.time < 5 * 60_000
      } ?? false
    out.append(.line(l, cont: cont))
    prev = l
  }
  flush()
  return out
}
