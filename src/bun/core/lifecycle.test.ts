// Reconnects, backoff, keepalive and send pacing, on a clock that only moves
// when a test advances it.

import { afterEach, expect, test } from 'bun:test';
import { Direct, isStatus, must, Peer, realSleep, retryDelays, secs, waitCount, waitEvent } from './harness';

const live: Direct[] = [];
function start(configure?: ConstructorParameters<typeof Direct>[0], bind: string | null = null) {
  const d = new Direct(configure, bind);
  live.push(d);
  return d;
}
afterEach(async () => {
  for (const d of live.splice(0)) {
    if (!d.finished) d.command({ type: 'quit' });
    d.dispose();
  }
});

/** Wait for the actor to report the session ended, returning its error. */
async function disconnected(d: Direct, nth: number): Promise<string | null> {
  const evs = await waitCount(d.events, nth, (e) => isStatus(e, 't', 'disconnected'));
  const e = evs[nth - 1];
  return e.type === 'status' ? e.error : null;
}

/** The connection must not come back before `delay` seconds, and must right after. */
async function expectRetryAfter(d: Direct, delay: number): Promise<Peer> {
  await d.advance(delay * 1000 - 10, 1000);
  expect(await d.listener.tryAccept(100)).toBeNull();
  await d.advance(10);
  return d.listener.accept();
}

test('reconnects with backoff and rejoins channels', async () => {
  const d = start((s) => {
    s.autojoin = ['#a', ' ', '#b'];
    s.perform = ['/MODE $nick +i'];
  });

  // Failures before registration back off exponentially: 2s, 4s, 8s.
  let p = await d.listener.accept();
  await p.expect('CAP LS');
  p.drop();
  expect(await disconnected(d, 1)).not.toBeNull();
  p = await expectRetryAfter(d, 2);
  p.drop();
  await disconnected(d, 2);
  p = await expectRetryAfter(d, 4);
  p.drop();
  await disconnected(d, 3);
  p = await expectRetryAfter(d, 8);

  await p.register('');
  expect(await p.expect('JOIN')).toBe('JOIN #a,#b');
  expect(await p.line()).toBe('MODE me +i');
  await waitEvent(d.events, 'connected', (e) => isStatus(e, 't', 'connected'));

  // Losing a registered session resets the backoff, and the channels are
  // joined again on the new link.
  await p.send('ERROR :Closing link (Ping timeout)');
  expect(await p.closed()).toEqual([]);
  expect(await disconnected(d, 4)).toBe('Closing link (Ping timeout)');
  p = await expectRetryAfter(d, 2);
  await p.register('');
  expect(await p.expect('JOIN')).toBe('JOIN #a,#b');

  // Channels the user joined by hand are rejoined by the frontend, keys and
  // all; the actor must pass that through on the new link.
  d.send('JOIN #secret,#open hunter2');
  expect(await p.expect('JOIN')).toBe('JOIN #secret,#open hunter2');

  expect(retryDelays(d.events)).toEqual([2, 4, 8, 2]);
  await d.quit();
});

test('backoff is capped at two minutes', async () => {
  const d = start();
  const delays = [2, 4, 8, 16, 32, 64, 120, 120];
  for (const [i, delay] of delays.entries()) {
    (await d.listener.accept()).drop();
    await disconnected(d, i + 1);
    await d.advance(secs(delay));
  }
  expect(retryDelays(d.events)).toEqual(delays);
  await d.quit();
});

test('requests made while offline fail fast', async () => {
  const d = start();
  (await d.listener.accept()).drop();
  await disconnected(d, 1);

  const reply = new Promise<{ ok: unknown } | { err: string }>((resolve) =>
    d.command({ type: 'history', kind: 'messages', target: '#a', line: 'CHATHISTORY LATEST #a * 10', reply: resolve }),
  );
  expect(await must('history reply', reply)).toEqual({ err: 'not connected' });
  await d.quit();
});

test('quit during backoff stops the actor', async () => {
  const d = start();
  (await d.listener.accept()).drop();
  await disconnected(d, 1);
  d.command({ type: 'quit' });
  await d.stopped();
  await d.advance(secs(10), 1000);
  expect(await d.listener.tryAccept(100)).toBeNull();
});

test('manual reconnect skips the backoff', async () => {
  const d = start();
  let p = await d.listener.accept();
  await p.register('');

  d.command({ type: 'reconnect' });
  expect((await p.closed()).at(-1)).toBe('QUIT :Reconnecting');
  // No clock movement needed: a requested reconnect is immediate.
  p = await d.listener.accept();
  await p.expect('CAP LS');
  expect(await disconnected(d, 1)).toBeNull();
  expect(retryDelays(d.events)).toEqual([null]);

  // During a backoff, Reconnect cuts the wait short.
  p.drop();
  await disconnected(d, 2);
  d.command({ type: 'reconnect' });
  await d.listener.accept();
  await d.quit();
});

test('keepalive pings and times out', async () => {
  const d = start();
  let p = await d.listener.accept();
  await p.register('');
  await d.handled('422');

  // Silence for more than 90s draws a PING (checked every 15s).
  await d.advance(secs(90), secs(15));
  expect(await p.tryLine(100)).toBeNull();
  await d.advance(secs(15), secs(15));
  expect(await p.line()).toBe('PING :hearth');

  // Any traffic counts as an answer and keeps the link up.
  await p.send(':srv PONG srv :hearth');
  await d.handled('PONG');
  await d.advance(secs(90), secs(15));
  expect(await p.tryLine(100)).toBeNull();
  await d.advance(secs(15), secs(15));
  expect(await p.line()).toBe('PING :hearth');

  // No answer at all: the link is dropped once 150s have passed.
  await d.advance(secs(45), secs(15));
  expect(await p.tryLine(100)).toBeNull();
  await d.advance(secs(15), secs(15));
  expect(await p.closed()).toEqual([]);
  expect(await disconnected(d, 1)).toBe('ping timeout');

  // ...and reconnects after the reset backoff.
  p = await expectRetryAfter(d, 2);
  await p.expect('CAP LS');
  await d.quit();
});

test('registration times out', async () => {
  const d = start();
  const p = await d.listener.accept();
  await p.expect('USER');
  // The server keeps talking but never welcomes us.
  await d.advance(secs(45), secs(15));
  await p.send(':srv NOTICE * :*** Looking up your hostname');
  await d.handled('NOTICE');
  await d.advance(secs(15), secs(15));
  expect(await p.tryLine(100)).toBeNull();
  await d.advance(secs(15), secs(15));
  await p.closed();
  expect(await disconnected(d, 1)).toBe('registration timed out');
  expect(retryDelays(d.events)).toEqual([2]);
  await d.quit();
});

test('direct connections are paced', async () => {
  const d = start();
  const p = await d.listener.accept();
  await p.register('');
  await d.handled('422');
  // Let the token bucket refill after registration.
  await d.advance(secs(10), secs(5));

  const lines = Array.from({ length: 12 }, (_, i) => `PRIVMSG #a :${i}`);
  d.command({ type: 'send', lines });
  // A burst of eight goes out at once...
  for (let i = 0; i < 8; i++) expect(await p.line()).toBe(`PRIVMSG #a :${i}`);
  expect(await p.tryLine(100)).toBeNull();
  expect(d.backlog.n).toBe(4);
  // ...then one every 400ms.
  for (let i = 8; i < 12; i++) {
    await d.advance(390);
    expect(await p.tryLine(50)).toBeNull();
    await d.advance(10);
    expect(await p.line()).toBe(`PRIVMSG #a :${i}`);
  }
  await must('backlog drained', (async () => { while (d.backlog.n !== 0) await realSleep(2); })());
  await d.quit();
});

test('bouncer children are not paced', async () => {
  const d = start(undefined, '7');
  const p = await d.listener.accept();
  await p.expect('CAP LS');
  await p.send(':srv CAP * LS :batch');
  await p.expect('CAP REQ');
  await p.send(':srv CAP * ACK :batch');
  expect(await p.line()).toBe('BOUNCER BIND 7');
  expect(await p.line()).toBe('CAP END');
  await p.welcome();

  d.command({ type: 'send', lines: Array.from({ length: 30 }, (_, i) => `PRIVMSG #a :${i}`) });
  // All of it arrives without the clock moving at all.
  for (let i = 0; i < 30; i++) expect(await p.line()).toBe(`PRIVMSG #a :${i}`);
  await d.quit();
});

test('quit is flushed with its reason', async () => {
  const d = start();
  const p = await d.listener.accept();
  await p.register('');
  d.command({ type: 'quit', reason: 'see you' });
  expect(await p.closed()).toEqual(['QUIT :see you']);
  await d.stopped();
  // No reconnect was scheduled.
  await realSleep(50);
  expect(retryDelays(d.events)).toEqual([]);
});

test('a quit the user sends does not reconnect', async () => {
  const d = start();
  const p = await d.listener.accept();
  await p.register('labeled-response');
  await d.handled('422');

  // `/raw QUIT` goes out labeled; the server answers with ERROR and hangs up.
  d.send('@label=f1 QUIT :done for today');
  expect(await p.expect('@label=f1')).toBe('@label=f1 QUIT :done for today');
  await p.send('ERROR :Closing Link: me (Quit: done for today)');
  p.drop();
  await d.stopped();
  await d.advance(secs(10), secs(1));
  expect(await d.listener.tryAccept(100)).toBeNull();
  expect(retryDelays(d.events)).toEqual([]);
});

test('an unlabeled quit closes without retrying', async () => {
  const d = start();
  const p = await d.listener.accept();
  await p.register('');
  await d.handled('422');

  // Any spelling of QUIT stops the writer after it; that isn't a failure.
  d.send('quit :bye', 'PRIVMSG #a :never sent');
  expect(await p.closed()).toEqual(['quit :bye']);
  await d.stopped();
  expect(retryDelays(d.events)).toEqual([]);
});
