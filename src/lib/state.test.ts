import { beforeEach, describe, expect, it, vi } from 'vitest';
import { connect, feed, mockApi, sent } from './test-utils';

vi.mock('./api', () => ({ api: mockApi, onIrc: vi.fn(async () => () => {}) }));
vi.mock('./notify', () => ({ notify: vi.fn() }));

const { App } = await import('./state.svelte');
const { notify } = await import('./notify');

let app: InstanceType<typeof App>;

beforeEach(() => {
  vi.clearAllMocks();
  sent.length = 0;
  app = new App();
});

const conn = () => app.conns.get('c')!;
const texts = (name: string) => conn().buffer(name)!.messages.map((l) => l.text);

describe('messages', () => {
  it('files channel and private messages and flags mentions', () => {
    connect(app);
    feed(app, ':me!u@h JOIN #hearth', ':ann!a@h PRIVMSG #hearth :hello there', ':bob!b@h PRIVMSG #hearth :me: ping');
    feed(app, ':cat!c@h PRIVMSG me :psst');
    const chan = conn().buffer('#hearth')!;
    expect(chan.messages.map((l) => [l.nick, l.highlight])).toEqual([
      ['ann', false],
      ['bob', true],
    ]);
    expect(conn().buffer('cat')!.kind).toBe('query');
    expect(texts('cat')).toEqual(['psst']);
  });

  it('drops messages from ignored masks', () => {
    connect(app);
    app.settings.ignores = ['spammer', '*!*@bad.host'];
    feed(app, ':me!u@h JOIN #a', ':Spammer!x@y PRIVMSG #a :buy', ':eve!e@bad.host PRIVMSG #a :hi', ':ann!a@ok PRIVMSG #a :fine');
    feed(app, '@+typing=active :spammer!x@y TAGMSG #a');
    expect(texts('#a')).toEqual(['fine']);
    expect(conn().buffer('#a')!.typing.size).toBe(0);
  });

  it('rate-limits CTCP replies', () => {
    connect(app);
    for (let i = 0; i < 10; i++) feed(app, `:x${i}!u@h PRIVMSG me :\x01VERSION\x01`);
    expect(sent.filter((l) => l.startsWith('NOTICE')).length).toBe(3);
    expect(conn().server.messages.some((l) => l.text.includes('flood'))).toBe(true);
  });

  it('attaches reactions and replies', () => {
    connect(app, { caps: ['message-tags'] });
    feed(app, ':me!u@h JOIN #a', '@msgid=m1 :ann!a@h PRIVMSG #a :ship it?');
    feed(app, '@+draft/react=👍;+draft/reply=m1 :bob!b@h TAGMSG #a', '@+draft/react=👍;+draft/reply=m1 :bob!b@h TAGMSG #a');
    feed(app, '@msgid=m2;+draft/reply=m1 :cat!c@h PRIVMSG #a :yes');
    const [first, second] = conn().buffer('#a')!.messages;
    expect(first.reactions).toEqual({ '👍': ['bob'] });
    expect(second.replyTo).toBe('m1');
  });

  it('sends replies and reactions with tags', () => {
    connect(app, { caps: ['message-tags'] });
    feed(app, ':me!u@h JOIN #a', '@msgid=m1 :ann!a@h PRIVMSG #a :hi');
    const buf = conn().buffer('#a')!;
    app.sendMessage(buf, 'hello', 'PRIVMSG', false, 'm1');
    app.react(buf, buf.messages[0], '🎉');
    expect(sent).toContain('@+draft/reply=m1 PRIVMSG #a :hello');
    expect(sent).toContain('@+draft/react=🎉;+draft/reply=m1 TAGMSG #a');
    // Without echo-message the reaction shows up locally at once.
    expect(buf.messages[0].reactions).toEqual({ '🎉': ['me'] });
  });
});

describe('routing', () => {
  it('sends labeled replies to the buffer that asked', () => {
    connect(app, { caps: ['labeled-response'] });
    feed(app, ':me!u@h JOIN #a', ':me!u@h JOIN #b');
    const a = conn().buffer('#a')!;
    app.open(conn().buffer('#b')!);
    conn().sendFrom(a, 'WHOIS ann');
    const label = /^@label=(\S+) WHOIS ann$/.exec(sent.at(-1)!)![1];
    feed(app, `@label=${label} :srv 311 me ann a h * :Ann`, `@label=${label} :srv 318 me ann :End`);
    expect(a.messages.at(-1)!.type).toBe('whois');
    expect(conn().buffer('#b')!.messages.some((l) => l.type === 'whois')).toBe(false);
  });

  it('collects LIST replies into the channel browser', () => {
    connect(app);
    feed(app, ':srv 321 me Channel :Users Name', ':srv 322 me #a 12 :topic a', ':srv 322 me #b 3 :topic b', ':srv 323 me :End');
    expect(conn().channelList!.loading).toBe(false);
    expect(conn().channelList!.items.map((c) => [c.name, c.users])).toEqual([
      ['#a', 12],
      ['#b', 3],
    ]);
    expect(conn().server.messages).toEqual([]);
  });

  it('tracks MONITOR presence and announces watched nicks', () => {
    connect(app, { isupport: { MONITOR: '100' } });
    app.settings.watch = { 'c|': ['Ann'] };
    feed(app, ':srv 730 me :ann!a@h,bob!b@h');
    expect(conn().online.get('ann')).toBe(true);
    expect(conn().server.messages).toEqual([]);
    feed(app, ':srv 731 me :ann');
    expect(conn().online.get('ann')).toBe(false);
    expect(conn().server.messages.at(-1)!.text).toBe('ann is offline');
  });
});

describe('buffers', () => {
  it('restores saved buffers and rejoins with keys', async () => {
    vi.useFakeTimers();
    mockApi.loadBuffers.mockResolvedValueOnce([
      { name: '#open', kind: 'channel', key: '' },
      { name: '#locked', kind: 'channel', key: 'sekrit' },
      { name: 'ann', kind: 'query', key: '' },
    ]);
    connect(app);
    await vi.advanceTimersByTimeAsync(2000);
    expect(conn().buffer('ann')!.kind).toBe('query');
    expect(sent).toContain('JOIN #locked,#open sekrit');
    vi.useRealTimers();
  });

  it('saves the buffer list and forgets parted channels', async () => {
    vi.useFakeTimers();
    connect(app);
    await vi.advanceTimersByTimeAsync(0);
    app.join(conn(), '#keyed', 'k1');
    feed(app, ':me!u@h JOIN #keyed', ':me!u@h JOIN #gone', ':me!u@h PART #gone');
    await vi.advanceTimersByTimeAsync(600);
    expect(mockApi.saveBuffers).toHaveBeenLastCalledWith('c', [{ name: '#keyed', kind: 'channel', key: 'k1' }]);
    vi.useRealTimers();
  });

  it('falls back to the local log without chathistory', async () => {
    connect(app);
    feed(app, ':me!u@h JOIN #a', '@time=2026-01-01T00:00:10.000Z :ann!a@h PRIVMSG #a :live');
    const buf = conn().buffer('#a')!;
    mockApi.localHistory.mockResolvedValueOnce([
      { tags: { time: '2026-01-01T00:00:01.000Z' }, source: { nick: 'bob', user: 'b', host: 'h' }, command: 'PRIVMSG', params: ['#a', 'old'] },
    ]);
    buf.history = 'idle';
    await app.fetchHistory(buf, 'before');
    expect(mockApi.localHistory).toHaveBeenCalledWith('c', '#a', { before: Date.parse('2026-01-01T00:00:10.000Z'), limit: 100 });
    expect(texts('#a')).toEqual(['old', 'live']);
    expect(buf.history).toBe('done');
  });

  it('dedupes history lines that match live ones closely in time', () => {
    connect(app);
    feed(app, ':me!u@h JOIN #a');
    const buf = conn().buffer('#a')!;
    app.sendMessage(buf, 'mine');
    const t = buf.messages[0].time;
    buf.merge([{ ...buf.messages[0], id: 999, time: t - 5 }]);
    expect(buf.messages.length).toBe(1);
  });
});

describe('notification levels', () => {
  it('mutes and escalates per buffer', () => {
    connect(app);
    feed(app, ':me!u@h JOIN #a', ':ann!a@h PRIVMSG #a :chatter', ':ann!a@h PRIVMSG #a :me: look');
    const buf = conn().buffer('#a')!;
    buf.readMarker = 0;
    expect(buf.highlights).toBe(1);
    app.settings.bufferNotify = { [buf.prefKey]: 'all' };
    expect(buf.highlights).toBe(2);
    app.settings.bufferNotify = { [buf.prefKey]: 'none' };
    expect(buf.highlights).toBe(0);
    app.focused = false;
    vi.mocked(notify).mockClear();
    feed(app, ':ann!a@h PRIVMSG #a :me: again');
    expect(notify).not.toHaveBeenCalled();
    app.settings.bufferNotify = { [buf.prefKey]: 'all' };
    feed(app, ':ann!a@h PRIVMSG #a :anything');
    expect(notify).toHaveBeenCalledTimes(1);
  });
});
