import { beforeEach, describe, expect, it, vi } from 'vitest';
import { connect, feed, mockApi, sent } from './test-utils';
import type { Line, SearchHit } from './types';

vi.mock('./api', () => ({ api: mockApi, onIrc: vi.fn(async () => () => {}) }));
vi.mock('./notify', () => ({ notify: vi.fn() }));

const { App } = await import('./state.svelte');
const { mergeLines } = await import('./history');
const { dedupeHits } = await import('./search');
const { monitorCommands } = await import('./monitor');
const { rejoinCommands } = await import('./saved');
const { countHighlights, countUnread } = await import('./unread');

let app: InstanceType<typeof App>;
let id = 0;

beforeEach(() => {
  vi.clearAllMocks();
  sent.length = 0;
  app = new App();
});

const line = (p: Partial<Line>): Line => ({ id: ++id, type: 'privmsg', time: 1000, nick: 'ann', text: 'hi', self: false, highlight: false, ...p });
const conn = () => app.conns.get('c')!;

describe('mergeLines', () => {
  it('returns null when nothing is new', () => {
    const a = line({ msgid: 'm1' });
    expect(mergeLines([a], [])).toBeNull();
    expect(mergeLines([a], [{ ...a, id: 99, text: 'edited copy' }])).toBeNull();
  });

  it('dedupes msgid-less lines by content within 3 s', () => {
    const live = line({ time: 10_000 });
    expect(mergeLines([live], [line({ time: 12_999 })])).toBeNull();
    expect(mergeLines([live], [line({ time: 13_000 })])!.length).toBe(2);
    expect(mergeLines([live], [line({ time: 10_000, text: 'other' })])!.length).toBe(2);
  });

  it('keeps lines with a new msgid even if their text matches', () => {
    const live = line({ time: 10_000 });
    expect(mergeLines([live], [line({ time: 10_001, msgid: 'm2' })])!.length).toBe(2);
  });

  it('sorts by time, then insertion order', () => {
    const a = line({ time: 5, text: 'a' });
    const b = line({ time: 1, text: 'b' });
    const c = line({ time: 5, text: 'c' });
    expect(mergeLines([a], [c, b])!.map((l) => l.text)).toEqual(['b', 'a', 'c']);
  });
});

describe('unread counts', () => {
  const msgs = [
    line({ time: 1 }),
    line({ time: 2, highlight: true }),
    line({ time: 3, type: 'join' }),
    line({ time: 4, self: true, highlight: true }),
    line({ time: 5, type: 'notice' }),
  ];

  it('counts messages from others after the marker', () => {
    expect(countUnread(msgs, 0)).toBe(3);
    expect(countUnread(msgs, 2)).toBe(1);
    expect(countUnread(msgs, 5)).toBe(0);
  });

  it('counts highlights, or everything when asked', () => {
    expect(countHighlights(msgs, 0, false)).toBe(1);
    expect(countHighlights(msgs, 0, true)).toBe(3);
    expect(countHighlights(msgs, 2, false)).toBe(0);
  });

  it('marks read and sends a debounced MARKREAD', async () => {
    vi.useFakeTimers();
    connect(app, { caps: ['draft/read-marker'] });
    feed(app, ':me!u@h JOIN #a', '@time=2026-01-01T00:00:00.000Z :ann!a@h PRIVMSG #a :one');
    const buf = conn().buffer('#a')!;
    expect(buf.unread).toBe(1);
    app.markRead(buf);
    expect(buf.unread).toBe(0);
    await vi.advanceTimersByTimeAsync(500);
    expect(sent).toContain('MARKREAD #a timestamp=2026-01-01T00:00:00.000Z');
    vi.useRealTimers();
  });
});

describe('dedupeHits', () => {
  const hit = (network: string, tags: Record<string, string>, text = 'x'): SearchHit => ({
    network,
    target: '#a',
    msg: { tags, source: null, command: 'PRIVMSG', params: ['#a', text] },
  });

  it('drops duplicates by msgid or content and sorts newest first', () => {
    const out = dedupeHits([
      hit('c', { msgid: 'm1', time: '2026-01-01T00:00:01Z' }),
      hit('c', { msgid: 'm1', time: '2026-01-01T00:00:01Z' }),
      hit('d', { msgid: 'm1', time: '2026-01-01T00:00:03Z' }),
      hit('c', { time: '2026-01-01T00:00:02Z' }),
      hit('c', { time: '2026-01-01T00:00:02Z' }),
    ]);
    expect(out.map((h) => [h.network, h.msg.tags.time])).toEqual([
      ['d', '2026-01-01T00:00:03Z'],
      ['c', '2026-01-01T00:00:02Z'],
      ['c', '2026-01-01T00:00:01Z'],
    ]);
  });
});

describe('monitorCommands', () => {
  it('chunks long lists and honours the server limit', () => {
    const nicks = Array.from({ length: 60 }, (_, i) => `nickname${String(i).padStart(2, '0')}`);
    const lines = monitorCommands(nicks);
    expect(lines.length).toBeGreaterThan(1);
    expect(lines.every((l) => l.length - 'MONITOR + '.length <= 400)).toBe(true);
    expect(lines.flatMap((l) => l.slice(10).split(','))).toEqual(nicks);
    expect(monitorCommands(['a', 'b', 'c'], 2)).toEqual(['MONITOR + a,b']);
    expect(monitorCommands([])).toEqual([]);
  });

  it('counts commas and UTF-8 bytes against the 400-byte limit', () => {
    expect(monitorCommands(['a'.repeat(200), 'b'.repeat(199)])).toHaveLength(1);
    expect(monitorCommands(['a'.repeat(200), 'b'.repeat(200)])).toHaveLength(2);
    // 134 three-byte characters are 402 bytes but only 134 UTF-16 units.
    expect(monitorCommands(['ä'.repeat(100), 'ö'.repeat(99)])).toHaveLength(1);
    expect(monitorCommands(['中'.repeat(100), '文'.repeat(34)])).toHaveLength(2);
  });

  it('refuses to watch past the server limit', () => {
    connect(app, { isupport: { MONITOR: '1' } });
    app.watch(conn(), 'ann');
    sent.length = 0;
    app.watch(conn(), 'bob');
    expect(conn().watched).toEqual(['ann']);
    expect(sent).not.toContain('MONITOR + bob');
  });

  it('subscribes watched nicks and open queries on connect', async () => {
    app.settings.watch = { 'c|': ['Ann'] };
    mockApi.loadBuffers.mockResolvedValueOnce([{ name: 'bob', kind: 'query', key: '' }]);
    connect(app, { isupport: { MONITOR: '100' } });
    await vi.waitFor(() => expect(sent).toContain('MONITOR + Ann,bob'));
  });
});

describe('rejoinCommands', () => {
  it('puts keyed channels first and groups by ten', () => {
    expect(rejoinCommands([{ name: '#a', joinKey: '' }, { name: '#b', joinKey: 'k' }])).toEqual(['JOIN #b,#a k']);
    const many = Array.from({ length: 12 }, (_, i) => ({ name: `#c${i}`, joinKey: '' }));
    expect(rejoinCommands(many).length).toBe(2);
  });
});
