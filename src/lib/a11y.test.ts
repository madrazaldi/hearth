import { describe, expect, it } from 'vitest';
import { announcements, bufferLabel, lineLabel, memberLabel, stepIndex, wrapTab } from './a11y';
import type { Line } from './types';

let id = 0;
const now = Date.now();
const line = (nick: string, text: string, extra: Partial<Line> = {}): Line => ({
  id: ++id,
  time: now - 1000,
  type: 'privmsg',
  nick,
  text,
  self: false,
  highlight: false,
  ...extra,
});

describe('stepIndex', () => {
  it('moves and clamps', () => {
    expect(stepIndex('ArrowDown', 0, 3)).toBe(1);
    expect(stepIndex('ArrowDown', 2, 3)).toBe(2);
    expect(stepIndex('ArrowUp', 0, 3)).toBe(0);
    expect(stepIndex('Home', 2, 3)).toBe(0);
    expect(stepIndex('End', 0, 3)).toBe(2);
    expect(stepIndex('PageDown', 1, 30, 10)).toBe(11);
    expect(stepIndex('PageUp', 5, 30, 10)).toBe(0);
  });

  it('starts from the end when nothing is focused and going up', () => {
    expect(stepIndex('ArrowUp', -1, 5)).toBe(4);
    expect(stepIndex('ArrowDown', -1, 5)).toBe(0);
  });

  it('ignores other keys and empty lists', () => {
    expect(stepIndex('a', 0, 3)).toBeNull();
    expect(stepIndex('ArrowDown', 0, 0)).toBeNull();
  });
});

describe('wrapTab', () => {
  it('wraps at the ends only', () => {
    expect(wrapTab(3, 2, false)).toBe(0);
    expect(wrapTab(3, 0, true)).toBe(2);
    expect(wrapTab(3, 1, false)).toBeNull();
    expect(wrapTab(3, 1, true)).toBeNull();
  });

  it('pulls focus in from outside', () => {
    expect(wrapTab(3, -1, false)).toBe(0);
    expect(wrapTab(3, -1, true)).toBe(2);
    expect(wrapTab(0, -1, false)).toBeNull();
  });
});

describe('labels', () => {
  it('names buffers with their counts', () => {
    expect(bufferLabel({ name: '#hearth', kind: 'channel', unread: 4, highlights: 1, muted: false, joined: true })).toBe(
      '#hearth, 1 mention, 4 unread',
    );
    expect(bufferLabel({ name: '#old', kind: 'channel', unread: 0, highlights: 0, muted: true, joined: false })).toBe('#old, not joined, muted');
    expect(bufferLabel({ name: 'bob', kind: 'query', unread: 2, highlights: 2, muted: false, online: true, typing: true })).toBe(
      'bob, online, 2 new messages, typing',
    );
  });

  it('names members with rank and away', () => {
    const prefix = { modes: 'ov', symbols: '@+' };
    expect(memberLabel({ nick: 'alice', prefix: '@+', away: false }, prefix)).toBe('alice, operator');
    expect(memberLabel({ nick: 'bob', prefix: '+', away: true }, prefix, 'Bob B')).toBe('bob, Bob B, voiced, away');
    expect(memberLabel({ nick: 'carol', prefix: '', away: false }, prefix)).toBe('carol');
  });

  it('names message rows', () => {
    expect(lineLabel(line('alice', '\x02hi\x02 there'), '12:03')).toBe('alice, 12:03: hi there');
    expect(lineLabel(line('alice', 'waves', { type: 'action' }), '12:03')).toBe('12:03, * alice waves');
    expect(lineLabel(line('bot', 'x', { type: 'notice', redacted: true }), '09:00')).toBe('bot (notice), 09:00: message deleted');
    expect(lineLabel(line('a', 'yes', { reactions: { '👍': ['b', 'c'] } }), '1:00p', 'bob')).toBe(
      'a, 1:00p: yes, replying to bob. Reactions: 👍 2',
    );
  });
});

describe('announcements', () => {
  it('announces only lines appended after the old last one', () => {
    const a = line('a', 'one');
    const b = line('b', 'two');
    const old = line('c', 'history', { time: now - 3600_000 });
    expect(announcements([old, a, b], a, now)).toEqual(['b: two']);
    // A history load prepends; nothing new at the end.
    expect(announcements([old, a, b], b, now)).toEqual([]);
  });

  it('skips our own lines, events and catch-up', () => {
    const a = line('a', 'one');
    const lines = [a, line('me', 'mine', { self: true }), line('x', '', { type: 'join' }), line('b', 'late', { time: now - 3600_000 })];
    expect(announcements(lines, a, now)).toEqual([]);
  });

  it('summarises a burst', () => {
    const a = line('a', 'one');
    const more = [1, 2, 3, 4].map((i) => line('n' + i, 'm' + i));
    expect(announcements([a, ...more], a, now)).toEqual(['4 new messages']);
  });

  it('falls back to time when the old last line was trimmed', () => {
    const gone = line('a', 'gone', { time: now - 5000 });
    expect(announcements([line('b', 'new')], gone, now)).toEqual(['b: new']);
  });

  it('formats actions', () => {
    const a = line('a', 'one');
    expect(announcements([a, line('b', 'waves', { type: 'action' })], a, now)).toEqual(['* b waves']);
  });
});
