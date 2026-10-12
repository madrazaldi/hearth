import { beforeEach, describe, expect, it, vi } from 'vitest';
import { connect, feed, mockApi, msg } from './test-utils';
import type { IrcMessage, Line, SearchHit } from './types';

vi.mock('./api', () => ({ api: mockApi, onIrc: vi.fn(async () => () => {}) }));
vi.mock('./notify', () => ({ notify: vi.fn() }));

const { App } = await import('./state.svelte');
const { addWindow } = await import('./history');
const { buildItems } = await import('./timeline');
const { mergePages, parseQuery } = await import('./search');

let app: InstanceType<typeof App>;
let id = 0;

beforeEach(() => {
  vi.clearAllMocks();
  app = new App();
});

const conn = () => app.conns.get('c')!;
const T = (s: number) => Date.parse('2026-01-01T00:00:00.000Z') + s * 1000;
const iso = (s: number) => new Date(T(s)).toISOString();
const at = (s: number, text: string, target = '#a'): IrcMessage => msg(`@time=${iso(s)} :ann!a@h PRIVMSG ${target} :${text}`);
const line = (s: number): Line => ({ id: ++id, type: 'privmsg', time: T(s), nick: 'ann', text: `t${s}`, self: false, highlight: false });

describe('addWindow', () => {
  const lines = [line(100), line(101), line(102)];

  it('leaves a gap between an old window and the present', () => {
    expect(addWindow([], lines, T(10), T(20))).toEqual([{ after: T(20), before: T(100) }]);
  });

  it('needs no gap when the window overlaps what is loaded', () => {
    expect(addWindow([], lines, T(50), T(101))).toEqual([]);
  });

  it('shrinks or splits the gaps it covers', () => {
    const gap = { after: T(20), before: T(100) };
    expect(addWindow([gap], lines, T(20), T(40))).toEqual([{ after: T(40), before: T(100) }]);
    expect(addWindow([gap], lines, T(60), T(100))).toEqual([{ after: T(20), before: T(60) }]);
    expect(addWindow([gap], lines, T(40), T(60))).toEqual([
      { after: T(20), before: T(40) },
      { after: T(60), before: T(100) },
    ]);
    expect(addWindow([gap], lines, T(20), T(100))).toEqual([]);
  });
});

describe('gaps in the timeline', () => {
  it('puts a gap row before the first line past it, then repeats the day', () => {
    const items = buildItems([line(10), line(20), line(100)], {
      divider: 0,
      channel: true,
      membership: 'collapse',
      gaps: [{ after: T(20), before: T(100) }],
    });
    expect(items.map((it) => it.kind)).toEqual(['day', 'line', 'line', 'gap', 'day', 'line']);
  });
});

describe('jumping to a message', () => {
  const history = () => connect(app, { caps: ['draft/chathistory', 'server-time'], isupport: { CHATHISTORY: '1000' } });

  it('loads around an old message by timestamp and fills the gap to the present', async () => {
    history();
    feed(app, ':me!u@h JOIN #a', `@time=${iso(1000)} :bob!b@h PRIVMSG #a :now`);
    const buf = conn().buffer('#a')!;
    mockApi.history.mockResolvedValueOnce([at(10, 'before'), at(20, 'target'), at(30, 'after')]);
    // soju has no msgids; its MSGREFTYPES is timestamp only.
    await app.jumpTo(buf, T(20), 'id-from-another-server');
    expect(mockApi.history).toHaveBeenLastCalledWith('c', 'messages', '#a', `CHATHISTORY AROUND #a timestamp=${iso(20)} 100`);
    expect(buf.focus).toMatchObject({ time: T(20) });
    expect(buf.gaps).toEqual([{ after: T(30), before: T(1000) }]);

    // Scrolling down into the gap pages forward until it closes.
    mockApi.history.mockResolvedValueOnce(Array.from({ length: 200 }, (_, i) => at(31 + i, `fill ${i}`)));
    await app.fillGap(buf, buf.gaps[0], 'after');
    expect(mockApi.history).toHaveBeenLastCalledWith('c', 'messages', '#a', `CHATHISTORY BETWEEN #a timestamp=${iso(30)} timestamp=${iso(1000)} 200`);
    expect(buf.gaps).toEqual([{ after: T(230), before: T(1000) }]);
    mockApi.history.mockResolvedValueOnce([at(500, 'last of it')]);
    await app.fillGap(buf, buf.gaps[0], 'after');
    expect(buf.gaps).toEqual([]);
    expect(buf.messages.length).toBe(3 + 200 + 1 + 1);
  });

  it('pages up into a gap from the present', async () => {
    history();
    feed(app, ':me!u@h JOIN #a', `@time=${iso(1000)} :bob!b@h PRIVMSG #a :now`);
    const buf = conn().buffer('#a')!;
    buf.gaps = [{ after: T(30), before: T(1000) }];
    mockApi.history.mockResolvedValueOnce([at(900, 'recent')]);
    await app.fillGap(buf, buf.gaps[0], 'before');
    expect(mockApi.history).toHaveBeenLastCalledWith('c', 'messages', '#a', `CHATHISTORY BETWEEN #a timestamp=${iso(1000)} timestamp=${iso(30)} 200`);
    expect(buf.gaps).toEqual([]);
  });

  it('uses msgid when the server takes it', async () => {
    connect(app, { caps: ['draft/chathistory'], isupport: { MSGREFTYPES: 'msgid,timestamp' } });
    feed(app, ':me!u@h JOIN #a');
    await app.jumpTo(conn().buffer('#a')!, T(20), 'abc');
    expect(mockApi.history).toHaveBeenLastCalledWith('c', 'messages', '#a', 'CHATHISTORY AROUND #a msgid=abc 100');
  });

  it("waits for a new buffer's latest lines before loading around", async () => {
    history();
    let release!: (m: IrcMessage[]) => void;
    mockApi.history.mockImplementationOnce(() => new Promise((r) => (release = r)));
    const buf = conn().ensure('#new');
    const jump = app.jumpTo(buf, T(20));
    release([at(1000, 'latest', '#new')]);
    mockApi.history.mockResolvedValueOnce([at(20, 'old', '#new')]);
    await jump;
    expect(mockApi.history.mock.calls.map((c) => (c as unknown[])[3])).toEqual([
      'CHATHISTORY LATEST #new * 100',
      `CHATHISTORY AROUND #new timestamp=${iso(20)} 100`,
    ]);
    expect(buf.gaps).toEqual([{ after: T(20), before: T(1000) }]);
  });
});

describe('search', () => {
  const now = new Date(2026, 9, 12, 15, 0).getTime();

  it('parses filters out of the query', () => {
    expect(parseQuery('deploy from:@ann in:#ops failed', now)).toEqual({ text: 'deploy failed', from: 'ann', in: '#ops' });
    const day = new Date(2026, 9, 1).getTime();
    const next = new Date(2026, 9, 2).getTime();
    expect(parseQuery('on:2026-10-01', now)).toMatchObject({ after: day - 1, before: next });
    expect(parseQuery('before:2026-10-01 after:2026-10-01', now)).toMatchObject({ before: day, after: next - 1 });
    expect(parseQuery('on:yesterday', now).before).toBe(new Date(2026, 9, 12).getTime());
    expect(parseQuery('before:soon http://x', now)).toEqual({ text: 'before:soon http://x' });
  });

  const hit = (s: number, network = 'c'): SearchHit => ({ network, target: '#a', msg: at(s, `m${s}`) });

  it('stops a merged page where a full source runs out', () => {
    const page = mergePages([
      { hits: [hit(100), hit(50)], full: true },
      { hits: [hit(90), hit(10)], full: false },
    ]);
    expect(page.hits.map((h) => h.msg.params[1])).toEqual(['m100', 'm90', 'm50']);
    expect(page.next).toBe(T(50));
    expect(mergePages([{ hits: [hit(1)], full: false }]).next).toBeNull();
  });

  it('searches buffer by buffer when the bouncer requires in=', async () => {
    connect(app, { caps: ['soju.im/search'] });
    feed(app, ':me!u@h JOIN #a', ':me!u@h JOIN #b');
    app.settings.storeLogs = false;
    mockApi.history.mockImplementation(async (...args: unknown[]) => {
      const line = String(args[3]);
      if (!line.includes(';in=')) throw new Error('The in parameter is mandatory');
      return line.includes('in=#b') ? [at(5, 'deploy', '#b')] : [];
    });
    const page = await app.search('deploy from:ann', {});
    expect(page.hits.map((h) => [h.target, h.msg.params[1]])).toEqual([['#b', 'deploy']]);
    const lines = mockApi.history.mock.calls.map((c) => (c as unknown[])[3]);
    expect(lines).toEqual([
      'SEARCH limit=100;text=deploy;from=ann',
      'SEARCH limit=50;text=deploy;from=ann;in=#a',
      'SEARCH limit=50;text=deploy;from=ann;in=#b',
    ]);
    // Remembered: the next search goes straight to the buffers.
    await app.search('x', {});
    expect(mockApi.history).toHaveBeenCalledTimes(5);
    mockApi.history.mockReset();
  });
});
