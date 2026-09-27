// IRCv3 extensions: draft/multiline, standard-replies, draft/channel-rename,
// draft/account-registration and draft/metadata-2.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { connect, feed, mockApi, sent } from './test-utils';
import { defaultServer } from './types';

vi.mock('./api', () => ({ api: mockApi, onIrc: vi.fn(async () => () => {}) }));
vi.mock('./notify', () => ({ notify: vi.fn() }));

const { App } = await import('./state.svelte');
const { runInput } = await import('./commands');
const { multilineBatches, parseMultilineCap, joinParts } = await import('./multiline');

let app: InstanceType<typeof App>;

beforeEach(() => {
  vi.clearAllMocks();
  sent.length = 0;
  app = new App();
});

afterEach(() => {
  vi.useRealTimers();
});

const conn = () => app.conns.get('c')!;
const buf = (name: string) => conn().buffer(name)!;
const texts = (name: string) => buf(name).messages.map((l) => l.text);

function inChannel(caps: string[] = [], capValues: Record<string, string> = {}) {
  connect(app, { caps, capValues });
  feed(app, ':me!u@h JOIN #a', ':srv 353 me = #a :@me ann', ':srv 366 me #a :End');
  sent.length = 0;
  app.open(buf('#a'));
  return buf('#a');
}

/** Replace generated batch refs and labels so sent lines can be compared. */
const normalized = () =>
  sent.map((l) => l.replace(/ml\d+/g, 'ML').replace(/label=f\d+/g, 'label=L'));

describe('draft/multiline', () => {
  const ML = ['batch', 'draft/multiline', 'message-tags'];

  it('parses the cap value', () => {
    expect(parseMultilineCap('max-bytes=4096,max-lines=24')).toEqual({ maxBytes: 4096, maxLines: 24 });
    expect(parseMultilineCap('max-bytes=4096')).toEqual({ maxBytes: 4096, maxLines: Infinity });
    expect(parseMultilineCap('max-lines=5')).toBeNull();
    expect(parseMultilineCap(undefined)).toBeNull();
  });

  it('splits long lines into concatenated parts without losing characters', () => {
    const long = 'word '.repeat(40).trim();
    const [batch] = multilineBatches(`${long}\nshort`, { maxBytes: 4096, maxLines: 10 }, 60);
    expect(batch.length).toBeGreaterThan(3);
    expect(batch.every((p) => new TextEncoder().encode(p.text).length <= 60)).toBe(true);
    expect(batch.filter((p) => !p.concat).length).toBe(2);
    expect(joinParts(batch)).toBe(`${long}\nshort`);
  });

  it('respects max-lines and max-bytes by starting new batches', () => {
    const lines = multilineBatches('a\nb\nc\nd', { maxBytes: 4096, maxLines: 3 }, 400);
    expect(lines.map(joinParts)).toEqual(['a\nb\nc', 'd']);
    const bytes = multilineBatches('aaaa\nbbbb\ncccc', { maxBytes: 10, maxLines: 99 }, 400);
    expect(bytes.map(joinParts)).toEqual(['aaaa\nbbbb', 'cccc']);
    expect(multilineBatches('\n\n', { maxBytes: 10, maxLines: 99 }, 400)).toEqual([]);
  });

  it('sends composer text with line breaks as one batch, keeping blank lines', async () => {
    const a = inChannel([...ML, 'labeled-response', 'echo-message'], { 'draft/multiline': 'max-bytes=4096,max-lines=24' });
    await runInput(app, a, 'first\n\nthird\n', 'parent1');
    expect(normalized()).toEqual([
      '@label=L;+draft/reply=parent1 BATCH +ML draft/multiline #a',
      '@batch=ML PRIVMSG #a :first',
      '@batch=ML PRIVMSG #a :',
      '@batch=ML PRIVMSG #a :third',
      'BATCH -ML',
    ]);
    // With echo-message, the message shows up once, when the server echoes it.
    expect(a.messages).toEqual([]);
    const label = sent[0].match(/label=(f\d+)/)![1];
    feed(app, `@label=${label};msgid=e1 :me!u@h PRIVMSG #a :first\n\nthird`);
    expect(texts('#a')).toEqual(['first\n\nthird']);
    a.merge([{ ...a.messages[0], id: -1 }]);
    expect(a.messages).toHaveLength(1);
  });

  it('shows our own batch locally without echo-message and splits at max-lines', async () => {
    const a = inChannel(ML, { 'draft/multiline': 'max-bytes=4096,max-lines=2' });
    await runInput(app, a, 'one\ntwo\nthree');
    expect(normalized()).toEqual([
      'BATCH +ML draft/multiline #a',
      '@batch=ML PRIVMSG #a :one',
      '@batch=ML PRIVMSG #a :two',
      'BATCH -ML',
      'BATCH +ML draft/multiline #a',
      '@batch=ML PRIVMSG #a :three',
      'BATCH -ML',
    ]);
    expect(texts('#a')).toEqual(['one\ntwo', 'three']);
    expect(a.messages.every((l) => l.self)).toBe(true);
  });

  it('marks continuations of a long line with multiline-concat', async () => {
    const a = inChannel(ML, { 'draft/multiline': 'max-bytes=8192' });
    const long = 'x'.repeat(700);
    await runInput(app, a, `${long}\nend`);
    const body = sent.filter((l) => l.includes('PRIVMSG'));
    expect(body.length).toBe(3);
    expect(body[1]).toMatch(/^@batch=ml\d+;draft\/multiline-concat PRIVMSG/);
    expect(body.map((l) => l.split(' :')[1]).join('')).toBe(long + 'end');
  });

  it('falls back to one PRIVMSG per line without the cap or for a single line', async () => {
    const a = inChannel(['batch']);
    await runInput(app, a, 'one\n\ntwo');
    expect(sent).toEqual(['PRIVMSG #a :one', 'PRIVMSG #a :two']);
    sent.length = 0;
    conn().caps = new Set([...ML]);
    conn().capValues = { 'draft/multiline': 'max-bytes=4096' };
    await runInput(app, a, 'just one\n');
    expect(sent).toEqual(['PRIVMSG #a :just one']);
  });

  it('receives a combined multiline message as one line with its breaks', () => {
    inChannel(ML);
    feed(app, '@msgid=m1 :ann!a@h PRIVMSG #a :hello\nme: are you there?');
    const [line] = buf('#a').messages;
    expect(line.text).toBe('hello\nme: are you there?');
    expect(line.highlight).toBe(true);
  });
});

describe('standard replies', () => {
  it('routes labeled replies to the buffer that sent the command', async () => {
    const a = inChannel(['labeled-response']);
    app.open(conn().server);
    await runInput(app, a, '/topic');
    const label = sent[0].match(/label=(f\d+)/)![1];
    feed(app, `@label=${label} FAIL TOPIC NEED_MORE_PRIVS #a :You're not a channel operator`);
    expect(buf('#a').messages.at(-1)).toMatchObject({ type: 'error', text: "TOPIC failed: You're not a channel operator" });
    expect(conn().server.messages).toEqual([]);
  });

  it('files unlabeled replies under a buffer named in the context', () => {
    inChannel();
    app.open(conn().server);
    feed(app, 'FAIL JOIN CHANNEL_FULL #a :Channel is full');
    expect(buf('#a').messages.at(-1)?.text).toBe('JOIN failed: Channel is full');
  });

  it('falls back to the active buffer, NOTEs to the server buffer', () => {
    const a = inChannel();
    feed(app, 'WARN REHASH CERTS_EXPIRED :Certificate expires soon', 'FAIL * ACCOUNT_REQUIRED', 'NOTE * OPER_MESSAGE :Maintenance tonight');
    expect(a.messages.slice(-2).map((l) => [l.type, l.text])).toEqual([
      ['info', 'Warning (REHASH): Certificate expires soon'],
      ['error', 'Account required'],
    ]);
    expect(conn().server.messages.at(-1)).toMatchObject({ type: 'info', text: 'Maintenance tonight' });
  });

  it('keeps quiet about history failures and toasts bouncer ones', () => {
    const a = inChannel();
    feed(app, 'FAIL CHATHISTORY INVALID_TARGET #a :nope', 'FAIL BOUNCER INVALID_NETID 7 :Unknown network');
    expect(a.messages).toEqual([]);
    expect(app.toasts.map((t) => t.text)).toEqual(['Bouncer: Unknown network']);
  });
});

describe('draft/channel-rename', () => {
  it('moves the buffer with its members, history, unread state and settings', () => {
    const a = inChannel(['draft/channel-rename']);
    feed(app, ':ann!a@h PRIVMSG #a :before');
    a.readMarker = 42;
    app.setNotify(a, 'all');
    feed(app, ':ann!a@h RENAME #a #b :cleaning up');
    expect(conn().buffer('#a')).toBeUndefined();
    const b = buf('#b');
    expect(b).toBe(a);
    expect(b.name).toBe('#b');
    expect(b.sortedMembers.map((m) => m.nick)).toEqual(['me', 'ann']);
    expect(b.readMarker).toBe(42);
    expect(texts('#b')).toEqual(['before', 'ann renamed #a to #b (cleaning up)']);
    expect(app.settings.bufferNotify).toEqual({ 'c||#b': 'all' });
    expect(app.active).toBe(b);
    feed(app, ':ann!a@h PRIVMSG #b :after');
    expect(texts('#b').at(-1)).toBe('after');
  });

  it('replaces a stale buffer that already had the new name', () => {
    inChannel();
    conn().ensure('#b', 'channel');
    feed(app, ':op!o@h RENAME #a #b');
    expect([...conn().buffers.keys()]).toEqual(['#b']);
    expect(buf('#b').joined).toBe(true);
  });

  it('/rename needs the cap and takes an optional source channel', async () => {
    const a = inChannel();
    await runInput(app, a, '/rename #new');
    expect(sent).toEqual([]);
    expect(app.toasts.at(-1)?.text).toMatch(/does not support renaming/);
    conn().caps = new Set(['draft/channel-rename']);
    await runInput(app, a, '/rename #new tidier name');
    await runInput(app, conn().server, '/rename #a #new');
    expect(sent).toEqual(['RENAME #a #new :tidier name', 'RENAME #a #new']);
  });
});

describe('draft/account-registration', () => {
  const REG = ['draft/account-registration', 'labeled-response'];

  function setup(capValue = 'custom-account-name') {
    app.servers = [{ ...defaultServer(), id: 'c', host: 'irc.example' }];
    mockApi.saveServer.mockImplementation(async (s: unknown) => s);
    return inChannel(REG, { 'draft/account-registration': capValue });
  }

  it('registers, then offers to save the login for SASL', async () => {
    const a = setup();
    await runInput(app, a, '/register me@example.com hunter2');
    expect(normalized()).toEqual(['@label=L REGISTER * me@example.com :hunter2']);
    const label = sent[0].match(/label=(f\d+)/)![1];
    feed(app, `@label=${label} REGISTER SUCCESS me :Account created`);
    expect(a.messages.at(-1)?.text).toBe('Account me registered');
    const offer = app.toasts.at(-1)!;
    expect(offer.action?.label).toBe('Save login');
    offer.action!.run();
    await vi.waitFor(() => expect(mockApi.saveServer).toHaveBeenCalled());
    expect(mockApi.saveServer.mock.calls[0][0]).toMatchObject({ id: 'c', sasl: 'plain', saslUsername: 'me', saslPassword: 'hunter2' });
    expect(conn().registration).toBeNull();
  });

  it('walks through verification and reports failures', async () => {
    const a = setup();
    await runInput(app, a, '/register * hunter2');
    feed(app, 'REGISTER VERIFICATION_REQUIRED me :Check your email');
    expect(a.messages.at(-1)?.text).toBe('Check your email — then type /verify me <code>');
    await runInput(app, a, '/verify me 123');
    feed(app, 'FAIL VERIFY INVALID_CODE me :That code is wrong');
    expect(a.messages.at(-1)).toMatchObject({ type: 'error', text: 'Verification failed: That code is wrong' });
    expect(conn().registration).not.toBeNull();
    feed(app, 'VERIFY SUCCESS me :Welcome');
    expect(a.messages.at(-1)?.text).toBe('Account me verified');
    expect(app.toasts.at(-1)?.action).toBeDefined();
  });

  it('forgets the password when registration is refused', async () => {
    const a = setup();
    await runInput(app, a, '/register * hunter2');
    feed(app, 'FAIL REGISTER ACCOUNT_EXISTS me :Already registered');
    expect(a.messages.at(-1)?.text).toBe('Registration failed: Already registered');
    expect(conn().registration).toBeNull();
    expect(app.toasts.some((t) => t.action)).toBe(false);
  });

  it('checks usage and email-required, and never keeps the command for recall', async () => {
    const a = setup('email-required');
    await runInput(app, a, '/register * hunter2');
    expect(sent).toEqual([]);
    expect(app.toasts.at(-1)?.text).toMatch(/needs an email/);
    app.inputHistory = ['/register a@b.c hunter2', '/verify me 1', 'hello'];
    expect(app.inputHistory).toEqual(['hello']);
  });
});

describe('draft/metadata-2', () => {
  it('tracks avatar and display-name per user', () => {
    inChannel(['draft/metadata-2']);
    feed(
      app,
      ':srv METADATA ann avatar * :https://img.example/ann.png',
      ':srv 761 me ann display-name * :Ann Example',
      ':srv 761 me #a url * :https://example.com',
    );
    expect(conn().profile('ANN')).toEqual({ avatar: 'https://img.example/ann.png', displayName: 'Ann Example' });
    feed(app, ':ann!a@h NICK anna');
    expect(conn().profile('anna').displayName).toBe('Ann Example');
    expect(conn().profile('ann')).toEqual({});
    feed(app, ':srv METADATA anna avatar *', ':srv 766 me anna display-name :key not set');
    expect(conn().profile('anna')).toEqual({});
    feed(app, ':srv METADATA anna avatar * :https://img.example/2.png', ':anna!a@h QUIT :bye');
    expect(conn().profile('anna')).toEqual({});
    expect(conn().server.messages).toEqual([]);
  });

  it('retries a sync the server postponed', () => {
    vi.useFakeTimers();
    inChannel(['draft/metadata-2']);
    feed(app, ':srv 774 me #a 3');
    vi.advanceTimersByTime(2900);
    expect(sent).toEqual([]);
    vi.advanceTimersByTime(200);
    expect(sent).toEqual(['METADATA #a SYNC']);
  });

  it('loads avatars through the backend only when allowed', async () => {
    expect(await app.avatar('https://img.example/a.png')).toBeNull();
    expect(mockApi.avatar).not.toHaveBeenCalled();
    app.settings.loadAvatars = true;
    expect(await app.avatar('https://img.example/a.png')).toMatch(/^data:image\/png;base64,/);
    await app.avatar('https://img.example/a.png');
    expect(mockApi.avatar).toHaveBeenCalledTimes(1);
  });
});
