// runInput: slash commands and plain text from the composer.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { connect, feed, mockApi, sent } from './test-utils';

vi.mock('./api', () => ({ api: mockApi, onIrc: vi.fn(async () => () => {}) }));
vi.mock('./notify', () => ({ notify: vi.fn() }));

const { App } = await import('./state.svelte');
const { runInput } = await import('./commands');

let app: InstanceType<typeof App>;

beforeEach(() => {
  vi.clearAllMocks();
  sent.length = 0;
  app = new App();
});

const conn = () => app.conns.get('c')!;
const buf = (name: string) => conn().buffer(name)!;
const toasts = () => app.toasts.map((t) => t.text);

function inChannel(opts: Parameters<typeof connect>[1] = {}) {
  connect(app, opts);
  feed(app, ':me!u@h JOIN #a', ':srv 353 me = #a :@me ann', ':srv 366 me #a :End');
  sent.length = 0;
  return buf('#a');
}

const run = (input: string, b = buf('#a'), replyTo?: string) => runInput(app, b, input, replyTo);

describe('channel commands', () => {
  it('/join adds a missing prefix, passes keys and opens the channel on JOIN', async () => {
    connect(app);
    await run('/join hearth', conn().server);
    await run('/j #locked sekrit', conn().server);
    expect(sent).toEqual(['JOIN #hearth', 'JOIN #locked sekrit']);
    feed(app, ':me!u@h JOIN #locked');
    expect(buf('#locked').joinKey).toBe('sekrit');
    expect(app.active).toBe(buf('#locked'));
  });

  it('/join of a channel we are in just switches to it', async () => {
    const a = inChannel();
    app.open(conn().server);
    await run('/join #A', conn().server);
    expect(sent).toEqual([]);
    expect(app.active).toBe(a);
  });

  it('/part with and without an explicit channel', async () => {
    inChannel();
    await run('/part see you all');
    await run('/part #other bye now');
    await run('/leave');
    expect(sent).toEqual(['PART #a :see you all', 'PART #other :bye now', 'PART #a']);
  });

  it('/topic shows or sets the topic', async () => {
    inChannel({ caps: ['labeled-response'] });
    await run('/topic');
    await run('/topic new: topic here');
    expect(sent[0]).toMatch(/^@label=\S+ TOPIC #a$/);
    expect(sent[1]).toBe('TOPIC #a :new: topic here');
  });

  it('/kick, /ban, /unban, /op and friends target the current channel', async () => {
    inChannel();
    await run('/kick ann');
    await run('/kick ann stop that now');
    await run('/ban ann');
    await run('/ban *!*@bad.host');
    await run('/ban troll!*');
    await run('/unban *!*@bad.host');
    await run('/op ann bob');
    await run('/devoice ann');
    expect(sent).toEqual([
      'KICK #a ann',
      'KICK #a ann :stop that now',
      'MODE #a +b ann!*@*',
      'MODE #a +b *!*@bad.host',
      'MODE #a +b troll!*',
      'MODE #a -b *!*@bad.host',
      'MODE #a +oo ann bob',
      'MODE #a -v ann',
    ]);
  });

  it('/mode on the channel, another channel or ourselves', async () => {
    inChannel();
    await run('/mode +m');
    await run('/mode');
    await run('/mode #b +i');
    await run('/mode me +i');
    expect(sent).toEqual(['MODE #a +m', 'MODE #a', 'MODE #b +i', 'MODE me +i']);
  });

  it('channel-only commands fail with usage in a query', async () => {
    connect(app);
    const q = conn().ensure('ann', 'query');
    expect(await run('/kick bob', q)).toBe(false);
    expect(await run('/topic hi', q)).toBe(false);
    expect(await run('/me', q)).toBe(false);
    expect(sent).toEqual([]);
    expect(toasts()).toEqual(['Usage: This command needs a channel', 'Usage: This command needs a channel', 'Usage: /me <action>']);
  });
});

describe('messaging commands', () => {
  it('/msg sends to a nick and opens a quiet query', async () => {
    inChannel();
    expect(await run('/msg ann hello there')).toBe(true);
    expect(sent).toEqual(['PRIVMSG ann :hello there']);
    expect(buf('ann').messages.map((l) => [l.text, l.self])).toEqual([['hello there', true]]);
    expect(await run('/msg ann')).toBe(false);
    expect(toasts()).toContain('Usage: /msg <target> <text>');
  });

  it('/msg with echo-message waits for the echo', async () => {
    inChannel({ caps: ['echo-message'] });
    await run('/m ann hi');
    expect(buf('ann').messages).toEqual([]);
  });

  it('/me sends a CTCP ACTION', async () => {
    const a = inChannel();
    await run('/me waves');
    expect(sent).toEqual(['PRIVMSG #a :\x01ACTION waves\x01']);
    expect(a.messages.at(-1)).toMatchObject({ type: 'action', text: 'waves', self: true });
  });

  it('/notice and /ctcp', async () => {
    inChannel();
    await run('/notice ann heads up');
    await run('/ctcp ann version');
    await run('/ctcp ann ping 123');
    expect(sent).toEqual(['NOTICE ann :heads up', 'PRIVMSG ann :\x01VERSION\x01', 'PRIVMSG ann :\x01PING 123\x01']);
  });

  it('/query opens a conversation and optionally sends', async () => {
    inChannel();
    await run('/q bob hey');
    expect(app.active).toBe(buf('bob'));
    expect(sent).toEqual(['PRIVMSG bob :hey']);
  });
});

describe('client-side commands', () => {
  it('/notify sets the buffer level; /mute and /unmute are shortcuts', async () => {
    const a = inChannel();
    await run('/notify ALL');
    expect(app.settings.bufferNotify[a.prefKey]).toBe('all');
    await run('/mute');
    expect(app.settings.bufferNotify[a.prefKey]).toBe('none');
    await run('/unmute');
    expect(a.prefKey in app.settings.bufferNotify).toBe(false);
    expect(await run('/notify loud')).toBe(false);
    expect(await run('/notify all', conn().server)).toBe(false);
  });

  it('/ignore and /unignore manage masks', async () => {
    const a = inChannel();
    await run('/ignore troll');
    await run('/ignore *@spam.host');
    await run('/ignore troll');
    expect(app.settings.ignores).toEqual(['troll!*@*', '*!*@spam.host']);
    feed(app, ':Troll!t@x PRIVMSG #a :hi');
    expect(a.messages).toEqual([]);
    await run('/ignore');
    expect(a.messages.at(-1)!.text).toBe('Ignoring: troll!*@*, *!*@spam.host');
    await run('/unignore troll');
    await run('/unignore nobody');
    expect(app.settings.ignores).toEqual(['*!*@spam.host']);
    expect(toasts().slice(-2)).toEqual(['No longer ignoring troll', 'nobody was not ignored']);
  });

  it('/clear empties the buffer', async () => {
    const a = inChannel();
    feed(app, ':ann!a@h PRIVMSG #a :one');
    await run('/clear');
    expect(a.messages).toEqual([]);
    expect(sent).toEqual([]);
  });

  it('/whois in a query defaults to the peer', async () => {
    connect(app);
    const q = conn().ensure('ann', 'query');
    sent.length = 0;
    await run('/whois', q);
    await run('/w bob', q);
    expect(sent).toEqual(['WHOIS ann ann', 'WHOIS bob bob']);
  });
});

describe('raw and unknown commands', () => {
  it('sends unknown commands verbatim with the name upper-cased', async () => {
    inChannel();
    expect(await run('/cs op #a me')).toBe(true);
    await run('/knock #secret please let me in');
    await run('/lusers');
    expect(sent).toEqual(['CS op #a me', 'KNOCK #secret please let me in', 'LUSERS']);
  });

  it('/raw and /quote pass the line through', async () => {
    connect(app);
    await run('/raw PRIVMSG ann :hi  there', conn().server);
    await run('/quote WHO #a', conn().server);
    expect(sent).toEqual(['PRIVMSG ann :hi  there', 'WHO #a']);
  });

  it('a double slash sends a literal slash', async () => {
    inChannel();
    await run('//shrug');
    expect(sent).toEqual(['PRIVMSG #a :/shrug']);
  });
});

describe('plain text', () => {
  it('ignores blank input and refuses to chat in the server buffer', async () => {
    connect(app);
    expect(await run('   ', conn().server)).toBe(false);
    expect(await run('hello', conn().server)).toBe(false);
    expect(sent).toEqual([]);
    expect(toasts()[0]).toMatch(/server buffer/);
  });

  it('sends each non-empty line separately, replying only with the first', async () => {
    inChannel({ caps: ['message-tags'] });
    await run('one\n\n  \ntwo\nthree', buf('#a'), 'm1');
    expect(sent).toEqual(['@+draft/reply=m1 PRIVMSG #a :one', 'PRIVMSG #a :two', 'PRIVMSG #a :three']);
  });

  it('splits long messages on word boundaries within the line limit', async () => {
    const a = inChannel();
    const words = Array.from({ length: 200 }, (_, i) => `word${i}`);
    const text = words.join(' ');
    await run(text);
    expect(sent.length).toBeGreaterThan(1);
    const parts = sent.map((l) => l.replace(/^PRIVMSG #a :/, ''));
    for (const p of parts) {
      expect(new TextEncoder().encode(`:me!${'u'.repeat(10)}@${'h'.repeat(63)} PRIVMSG #a :${p}`).length).toBeLessThanOrEqual(510);
      expect(p.startsWith(' ') || p.endsWith(' ')).toBe(false);
    }
    // Nothing lost, nothing broken mid-word.
    expect(parts.join(' ')).toBe(text);
    expect(parts.flatMap((p) => p.split(' '))).toEqual(words);
    expect(a.messages.map((l) => l.text)).toEqual(parts);
  });

  it('splits multi-byte text without breaking characters', async () => {
    inChannel();
    const text = 'é'.repeat(500);
    await run(text);
    const parts = sent.map((l) => l.replace(/^PRIVMSG #a :/, ''));
    expect(parts.length).toBeGreaterThan(1);
    expect(parts.join('')).toBe(text);
    for (const p of parts) expect(new TextEncoder().encode(p).length).toBeLessThanOrEqual(400);
  });
});
