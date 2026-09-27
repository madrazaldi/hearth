import { describe, expect, it } from 'vitest';
import { buildItems, itemIndexOf } from './timeline';
import { Estimates, indexAt, offsets, rangeFor } from './virtual';
import type { Line, LineType } from './types';

let id = 0;
const day = new Date(2026, 8, 1, 12).getTime();
const line = (type: LineType, nick: string, mins: number, extra: Partial<Line> = {}): Line => ({
  id: ++id,
  time: day + mins * 60_000,
  type,
  nick,
  text: 'hi',
  self: false,
  highlight: false,
  ...extra,
});

describe('timeline', () => {
  const opts = { divider: 0, channel: true, membership: 'collapse' as const };

  it('groups consecutive lines and collapses membership runs', () => {
    const lines = [
      line('privmsg', 'a', 0),
      line('privmsg', 'a', 1),
      line('join', 'b', 2),
      line('quit', 'c', 2),
      line('privmsg', 'a', 3),
      line('privmsg', 'a', 10),
    ];
    const items = buildItems(lines, opts);
    expect(items.map((i) => i.kind)).toEqual(['day', 'line', 'line', 'run', 'line', 'line']);
    expect(items.map((i) => (i.kind === 'line' ? i.cont : null))).toEqual([null, false, true, null, false, false]);
    expect(itemIndexOf(items, lines[3])).toBe(3);
    expect(buildItems(lines, { ...opts, membership: 'show' }).filter((i) => i.kind === 'run')).toHaveLength(0);
  });

  it('places day and unread dividers', () => {
    const lines = [line('privmsg', 'a', 0), line('privmsg', 'a', 1), line('privmsg', 'a', 24 * 60)];
    const items = buildItems(lines, { ...opts, divider: lines[0].time });
    expect(items.map((i) => i.kind)).toEqual(['day', 'line', 'unread', 'line', 'day', 'line']);
    // A divider breaks grouping.
    expect(items[3].kind === 'line' && items[3].cont).toBe(false);
  });

  it('starts a new group at a reply', () => {
    const items = buildItems([line('privmsg', 'a', 0), line('privmsg', 'a', 1, { replyTo: 'm1' })], opts);
    expect(items.map((i) => (i.kind === 'line' ? i.cont : null))).toEqual([null, false, false]);
  });
});

describe('virtual offsets', () => {
  const offs = offsets(4, (i) => [10, 20, 30, 40][i]);

  it('sums heights', () => {
    expect([...offs]).toEqual([0, 10, 30, 60, 100]);
  });

  it('finds the item at a position', () => {
    expect(indexAt(offs, -5)).toBe(0);
    expect(indexAt(offs, 0)).toBe(0);
    expect(indexAt(offs, 10)).toBe(1);
    expect(indexAt(offs, 59.5)).toBe(2);
    expect(indexAt(offs, 1000)).toBe(3);
  });

  it('covers a window', () => {
    expect(rangeFor(offs, 15, 35)).toEqual([1, 3]);
    expect(rangeFor(offs, -100, 1000)).toEqual([0, 4]);
    expect(rangeFor(offsets(0, () => 0), 0, 100)).toEqual([0, 0]);
  });

  it('averages measured heights per category', () => {
    const e = new Estimates({ msg: 28 });
    expect(e.get('msg')).toBe(28);
    expect(e.get('other')).toBe(24);
    e.add('msg', 20);
    e.add('msg', 40);
    expect(e.get('msg')).toBe(30);
    e.add('msg', 60, 40); // re-measured
    expect(e.get('msg')).toBe(40);
  });
});
