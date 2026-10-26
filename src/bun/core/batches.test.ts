// Batches, labeled responses, history routing and server-time stamping.

import { afterEach, expect, test } from 'bun:test';
import type { HistoryKind } from './client';
import { count, Harness, must, Peer } from './harness';
import { param, tag, type Message } from './message';
import { nowMs, parseMs } from './time';

const live: Harness[] = [];
afterEach(async () => {
  for (const h of live.splice(0)) await h.finish();
});

async function registered(caps: string): Promise<[Harness, Peer]> {
  const h = new Harness();
  live.push(h);
  h.mgr.connect(h.server());
  const p = await h.listener.accept();
  await p.register(caps);
  await h.event((m) => m.command === '422');
  return [h, p];
}

type Settled = { ok: Message[] } | { err: string };

/** Start a history request and return it with the line the server received. */
async function request(h: Harness, p: Peer, kind: HistoryKind, target: string, line: string): Promise<[Promise<Settled>, string]> {
  const prefix = line.split(' ')[0];
  const task: Promise<Settled> = h.mgr.history('t', kind, target, line).then(
    (ok) => ({ ok }),
    (e: Error) => ({ err: e.message }),
  );
  for (;;) {
    const l = await p.line();
    if (l.includes(prefix)) return [task, l];
  }
}

const labelOf = (line: string) => line.slice('@label='.length).split(' ')[0];

async function ok(task: Promise<Settled>): Promise<Message[]> {
  const r = await must('the history reply', task);
  if (!('ok' in r)) throw new Error(`request failed: ${r.err}`);
  return r.ok;
}

async function err(task: Promise<Settled>): Promise<string> {
  const r = await must('the history reply', task);
  if (!('err' in r)) throw new Error('request unexpectedly succeeded');
  return r.err;
}

const texts = (msgs: Message[]) => msgs.map((m) => param(m, 1));
const livePrivmsgs = (h: Harness) => count(h.events, (e) => e.type === 'message' && e.msg.command === 'PRIVMSG');

test('live messages without time are stamped', async () => {
  const [h, p] = await registered('server-time');
  const before = nowMs();
  await p.send(':x!u@h PRIVMSG #a :untimed');
  await p.send('@time=2020-02-02T02:02:02.020Z :x!u@h PRIVMSG #a :timed');
  const untimed = await h.event((m) => param(m, 1) === 'untimed');
  const after = nowMs();
  const stamped = parseMs(tag(untimed, 'time')!)!;
  expect(stamped).toBeGreaterThanOrEqual(before);
  expect(stamped).toBeLessThanOrEqual(after);
  const timed = await h.event((m) => param(m, 1) === 'timed');
  expect(tag(timed, 'time')).toBe('2020-02-02T02:02:02.020Z');

  // The local log uses the same stamp the UI saw.
  h.db.flush();
  const logged = h.db.history('t', '#a', null, 10).find((m) => param(m, 1) === 'untimed')!;
  expect(tag(logged, 'time')).toBe(tag(untimed, 'time'));
});

test('unlabeled history is matched by target', async () => {
  const [h, p] = await registered('batch draft/chathistory server-time');
  const [a, sent] = await request(h, p, 'messages', '#A', 'CHATHISTORY LATEST #A * 10');
  expect(sent).toBe('CHATHISTORY LATEST #A * 10'); // no label without labeled-response
  const [b] = await request(h, p, 'messages', '#b', 'CHATHISTORY LATEST #b * 10');

  // Replies come back out of order, and #a's in a different case.
  await p.send(':srv BATCH +1 chathistory #b');
  await p.send('@batch=1;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #b :for b');
  await p.send(':srv BATCH -1');
  await p.send(':srv BATCH +2 chathistory #a');
  await p.send('@batch=2;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #a :for a');
  await p.send(':srv BATCH -2');
  expect(texts(await ok(b))).toEqual(['for b']);
  expect(texts(await ok(a))).toEqual(['for a']);

  // History never shows up as live traffic.
  await p.assertQuiet();
  await h.event((m) => m.command === 'PING' && param(m, 0) === 'sync');
  expect(livePrivmsgs(h)).toBe(0);
});

test('unlabeled FAIL is matched by target', async () => {
  const [h, p] = await registered('batch draft/chathistory');
  const [a] = await request(h, p, 'messages', '#a', 'CHATHISTORY LATEST #a * 10');
  const [b] = await request(h, p, 'messages', '#b', 'CHATHISTORY LATEST #b * 10');
  await p.send(':srv FAIL CHATHISTORY INVALID_TARGET LATEST #b :No such channel');
  expect(await err(b)).toBe('No such channel');
  await p.send(':srv BATCH +1 chathistory #a');
  await p.send(':srv BATCH -1');
  expect(await ok(a)).toEqual([]);
});

test('targets and search batches route to their requests', async () => {
  const [h, p] = await registered('batch draft/chathistory soju.im/search');
  const [targets] = await request(
    h,
    p,
    'targets',
    '*',
    'CHATHISTORY TARGETS timestamp=2026-01-01T00:00:00.000Z timestamp=2026-02-01T00:00:00.000Z 50',
  );
  const [search] = await request(h, p, 'search', '*', 'SEARCH text=hello');

  await p.send(':srv BATCH +s soju.im/search');
  await p.send('@batch=s;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #a :hello world');
  await p.send(':srv BATCH -s');
  await p.send(':srv BATCH +t draft/chathistory-targets');
  await p.send('@batch=t :srv CHATHISTORY TARGETS #a 2026-01-05T00:00:00.000Z');
  await p.send('@batch=t :srv CHATHISTORY TARGETS bob 2026-01-04T00:00:00.000Z');
  await p.send(':srv BATCH -t');

  expect(texts(await ok(search))).toEqual(['hello world']);
  expect(texts(await ok(targets))).toEqual(['#a', 'bob']);
});

test('requests need the matching cap', async () => {
  const [h, p] = await registered('batch draft/chathistory');
  await expect(h.mgr.history('t', 'search', '*', 'SEARCH text=x')).rejects.toThrow('server does not support this request');
  // Nothing went to the server.
  await p.assertQuiet();
});

test('labeled ACK completes with no messages', async () => {
  const [h, p] = await registered('batch draft/chathistory labeled-response');
  const [a, sent] = await request(h, p, 'messages', '#a', 'CHATHISTORY LATEST #a * 10');
  await p.send(`@label=${labelOf(sent)} :srv ACK`);
  expect(await ok(a)).toEqual([]);
});

test('labeled history batch without a wrapper', async () => {
  const [h, p] = await registered('batch draft/chathistory labeled-response');
  const [a, sent] = await request(h, p, 'messages', '#a', 'CHATHISTORY LATEST #a * 10');
  // An unrelated live batch opening first must not steal the reply.
  await p.send(':srv BATCH +live chathistory #a');
  await p.send(`@label=${labelOf(sent)} :srv BATCH +h chathistory #a`);
  await p.send('@batch=h;time=2026-01-01T00:00:00.000Z;msgid=m1 :x!u@h PRIVMSG #a :old');
  await p.send('@batch=live;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #a :not ours');
  await p.send(':srv BATCH -h');
  await p.send(':srv BATCH -live');
  expect(texts(await ok(a))).toEqual(['old']);
  const liveMsg = await h.event((m) => param(m, 1) === 'not ours');
  expect(tag(liveMsg, 'batch')).toBe('live');
});

test('FAIL inside a history batch fails the request', async () => {
  const [h, p] = await registered('batch draft/chathistory labeled-response');
  const [a, sent] = await request(h, p, 'messages', '#a', 'CHATHISTORY LATEST #a * 10');
  await p.send(`@label=${labelOf(sent)} :srv BATCH +w labeled-response`);
  await p.send('@batch=w :srv FAIL CHATHISTORY MESSAGE_ERROR LATEST #a :Internal error');
  await p.send(':srv BATCH -w');
  expect(await err(a)).toBe('Internal error');
});

test('nested batches inside history stay in the reply', async () => {
  const [h, p] = await registered('batch draft/chathistory labeled-response draft/multiline');
  const [a, sent] = await request(h, p, 'messages', '#a', 'CHATHISTORY LATEST #a * 10');
  await p.send(`@label=${labelOf(sent)} :srv BATCH +w labeled-response`);
  await p.send('@batch=w :srv BATCH +h chathistory #a');
  await p.send('@batch=h;msgid=1;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #a :before');
  await p.send('@batch=h;msgid=2;time=2026-01-01T00:00:01.000Z :x!u@h BATCH +ml draft/multiline #a');
  await p.send('@batch=ml :x!u@h PRIVMSG #a :line one');
  await p.send('@batch=ml :x!u@h PRIVMSG #a :line two');
  // The inner batch ending must not complete the request early.
  await p.send('@batch=h :x!u@h BATCH -ml');
  await p.send('@batch=h;msgid=3;time=2026-01-01T00:00:02.000Z :x!u@h PRIVMSG #a :after');
  await p.send('@batch=w :srv BATCH -h');
  await p.send(':srv BATCH -w');

  const got = await ok(a);
  expect(texts(got)).toEqual(['before', 'line one\nline two', 'after']); // the multiline batch reads as one message
  expect(tag(got[1], 'msgid')).toBe('2');
  await p.assertQuiet();
  await h.event((m) => m.command === 'PING' && param(m, 0) === 'sync');
  expect(livePrivmsgs(h)).toBe(0); // history leaked out as live messages otherwise
  h.db.flush();
  expect(texts(h.db.history('t', '#a', null, 10))).toEqual(['before', 'line one\nline two', 'after']);
});

test('pending requests fail when the link drops', async () => {
  const [h, p] = await registered('batch draft/chathistory');
  const [a] = await request(h, p, 'messages', '#a', 'CHATHISTORY LATEST #a * 10');
  p.drop();
  expect(await err(a)).toBe('disconnected');
});

test('frontend labels follow nested batches', async () => {
  const [h, p] = await registered('batch labeled-response message-tags');
  h.mgr.send('t', ['@label=f1 WHO #a']);
  await p.expect('@label=f1 WHO');
  await p.send('@label=f1 :srv BATCH +r labeled-response');
  await p.send('@batch=r :srv BATCH +n netsplit a.net b.net');
  await p.send('@batch=n :x!u@h QUIT :a.net b.net');
  await p.send('@batch=r :srv BATCH -n');
  await p.send('@batch=r :srv 315 me #a :End of WHO');
  await p.send(':srv BATCH -r');
  await p.send(':y!u@h QUIT :bye');

  expect(tag(await h.event((m) => m.command === 'QUIT' && param(m, 0) === 'a.net b.net'), 'label')).toBe('f1');
  expect(tag(await h.event((m) => m.command === '315'), 'label')).toBe('f1');
  expect(tag(await h.event((m) => param(m, 0) === 'bye'), 'label')).toBeUndefined(); // label ends with the batch

  // An ACK for a frontend label reaches the UI.
  h.mgr.send('t', ['@label=f2 MODE #a +m']);
  await p.expect('@label=f2');
  await p.send('@label=f2 :srv ACK');
  expect(tag(await h.event((m) => m.command === 'ACK'), 'label')).toBe('f2');
});

test('unrelated batches pass through', async () => {
  const [h, p] = await registered('batch');
  await p.send(':srv BATCH +n netsplit a.net b.net');
  await p.send('@batch=n :x!u@h QUIT :a.net b.net');
  await p.send(':srv BATCH -n');
  const quit = await h.event((m) => m.command === 'QUIT');
  expect(tag(quit, 'batch')).toBe('n');
  expect(tag(quit, 'label')).toBeUndefined();
  await h.event((m) => m.command === 'BATCH' && param(m, 0) === '-n');
});
