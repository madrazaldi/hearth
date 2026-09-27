import type { Line } from './types';
import { formatDay } from './time';

export type Item =
  | { kind: 'day'; key: string; time: number; label: string }
  | { kind: 'unread'; key: string; time: number }
  | { kind: 'line'; key: string; time: number; line: Line; cont: boolean }
  | { kind: 'run'; key: string; time: number; lines: Line[] };

export const MEMBERSHIP = new Set(['join', 'part', 'quit', 'nick']);
const TALK = new Set(['privmsg', 'action', 'notice']);

export interface TimelineOpts {
  /** Lines newer than this (and not ours) get a "New" divider before them; 0 for none. */
  divider: number;
  /** Membership changes are only grouped in channels. */
  channel: boolean;
  membership: 'show' | 'collapse' | 'hide';
}

/** Turn a buffer's lines into rows: day and unread dividers, grouped runs of joins/parts. */
export function buildItems(lines: Line[], o: TimelineOpts): Item[] {
  const out: Item[] = [];
  let prev: Line | null = null;
  let run: Line[] = [];
  let dividerPlaced = !o.divider;
  // Local-day bounds of the current day, so we only touch Date at day changes.
  let dayStart = Infinity;
  let dayEnd = -Infinity;

  const flush = () => {
    if (!run.length) return;
    if (run.length === 1 || o.membership === 'show')
      for (const l of run) out.push({ kind: 'line', key: 'l' + l.id, time: l.time, line: l, cont: false });
    else out.push({ kind: 'run', key: 'r' + run[0].id, time: run[0].time, lines: run });
    run = [];
  };

  for (const l of lines) {
    if (l.time < dayStart || l.time >= dayEnd) {
      const d = new Date(l.time);
      dayStart = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
      dayEnd = new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1).getTime();
      flush();
      out.push({ kind: 'day', key: 'd' + l.id, time: l.time, label: formatDay(l.time) });
      prev = null;
    }
    if (!dividerPlaced && l.time > o.divider && !l.self) {
      flush();
      out.push({ kind: 'unread', key: 'u', time: l.time });
      dividerPlaced = true;
      prev = null;
    }
    if (o.channel && MEMBERSHIP.has(l.type)) {
      run.push(l);
      prev = l;
      continue;
    }
    flush();
    const cont =
      !!prev &&
      TALK.has(l.type) &&
      prev.type === l.type &&
      l.type !== 'action' &&
      prev.nick === l.nick &&
      l.time - prev.time < 5 * 60_000;
    out.push({ kind: 'line', key: 'l' + l.id, time: l.time, line: l, cont });
    prev = l;
  }
  flush();
  return out;
}

/** Index of the item that shows `line`, including collapsed runs. */
export function itemIndexOf(items: Item[], line: Line): number {
  return items.findIndex((it) => (it.kind === 'line' ? it.line.id === line.id : it.kind === 'run' && it.lines.includes(line)));
}
