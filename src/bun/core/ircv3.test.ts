// IRCv3 extensions the actor handles itself: draft/multiline batches,
// draft/metadata-2 subscriptions, draft/channel-rename logs and cap values.

import { afterEach, expect, test } from 'bun:test';
import { count, Harness, Peer, waitEvent } from './harness';
import { param, tag } from './message';

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

const privmsgs = (h: Harness) => count(h.events, (e) => e.type === 'message' && e.msg.command === 'PRIVMSG');

test('multiline batches arrive as one message', async () => {
  const [h, p] = await registered('batch draft/multiline message-tags');
  await p.send('@msgid=m1;time=2026-01-01T00:00:00.000Z;+draft/reply=p0 :x!u@h BATCH +ml draft/multiline #a');
  await p.send('@batch=ml :x!u@h PRIVMSG #a :first line');
  await p.send('@batch=ml :x!u@h PRIVMSG #a :second ');
  await p.send('@batch=ml;draft/multiline-concat :x!u@h PRIVMSG #a :line, continued');
  await p.send('@batch=ml :x!u@h PRIVMSG #a :');
  await p.send('@batch=ml :x!u@h PRIVMSG #a :after a blank');
  await p.send(':x!u@h BATCH -ml');

  const m = await h.event((m) => m.command === 'PRIVMSG');
  expect(m.params).toEqual(['#a', 'first line\nsecond line, continued\n\nafter a blank']);
  expect(tag(m, 'msgid')).toBe('m1');
  expect(tag(m, 'time')).toBe('2026-01-01T00:00:00.000Z');
  expect(tag(m, '+draft/reply')).toBe('p0');
  expect(tag(m, 'batch')).toBeUndefined();
  expect(m.source?.nick).toBe('x');
  await p.assertQuiet();
  await h.event((m) => m.command === 'PING' && param(m, 0) === 'sync');
  expect(privmsgs(h)).toBe(1); // the parts must not leak out one by one
  expect(count(h.events, (e) => e.type === 'message' && e.msg.command === 'BATCH')).toBe(0);

  h.db.flush();
  const logged = h.db.history('t', '#a', null, 10);
  expect(logged).toHaveLength(1);
  expect(param(logged[0], 1)).toBe('first line\nsecond line, continued\n\nafter a blank');
});

test('echoed multiline keeps its label', async () => {
  const [h, p] = await registered('batch draft/multiline echo-message labeled-response message-tags');
  h.mgr.send('t', [
    '@label=f1 BATCH +out draft/multiline #a',
    '@batch=out PRIVMSG #a :one',
    '@batch=out PRIVMSG #a :two',
    'BATCH -out',
  ]);
  await p.expect('BATCH -out');
  // The label sits on the echoed batch itself...
  await p.send('@label=f1;msgid=e1 :me!u@h BATCH +e draft/multiline #a');
  await p.send('@batch=e :me!u@h PRIVMSG #a :one');
  await p.send('@batch=e :me!u@h PRIVMSG #a :two');
  await p.send(':me!u@h BATCH -e');
  let m = await h.event((m) => tag(m, 'msgid') === 'e1');
  expect([param(m, 1), tag(m, 'label')]).toEqual(['one\ntwo', 'f1']);

  // ...or on a labeled-response batch around it.
  await p.send('@label=f2 :srv BATCH +r labeled-response');
  await p.send('@batch=r;msgid=e2 :me!u@h BATCH +e2 draft/multiline #a');
  await p.send('@batch=e2 :me!u@h PRIVMSG #a :three');
  await p.send('@batch=e2 :me!u@h PRIVMSG #a :four');
  await p.send('@batch=r :me!u@h BATCH -e2');
  await p.send(':srv BATCH -r');
  m = await h.event((m) => tag(m, 'msgid') === 'e2');
  expect([param(m, 1), tag(m, 'label')]).toEqual(['three\nfour', 'f2']);
});

test('outgoing multiline is logged as one message', async () => {
  const [h, p] = await registered('batch draft/multiline');
  h.mgr.send('t', [
    'BATCH +out draft/multiline #a',
    '@batch=out PRIVMSG #a :hello',
    '@batch=out;draft/multiline-concat PRIVMSG #a : world',
    '@batch=out PRIVMSG #a :bye',
    'BATCH -out',
  ]);
  await p.expect('BATCH -out');
  h.db.flush();
  const logged = h.db.history('t', '#a', null, 10);
  expect(logged).toHaveLength(1);
  expect(param(logged[0], 1)).toBe('hello world\nbye');
  expect(logged[0].source?.nick).toBe('me');
});

test('metadata keys are subscribed after registration', async () => {
  const h = new Harness();
  live.push(h);
  h.mgr.connect(h.server());
  const p = await h.listener.accept();
  await p.expect('CAP LS');
  await p.send(':srv CAP * LS :draft/metadata-2=before-connect,max-subs=10 draft/multiline=max-bytes=4096,max-lines=24');
  await p.expect('CAP REQ');
  await p.send(':srv CAP * ACK :draft/metadata-2 draft/multiline');
  await p.expect('CAP END');
  await p.welcome();
  expect(await p.expect('METADATA')).toBe('METADATA * SUB avatar display-name');

  // The frontend needs the multiline limits to build batches.
  const ev = await waitEvent(h.events, 'registration', (e) => e.type === 'registered');
  if (ev.type !== 'registered') throw new Error('unreachable');
  expect(ev.capValues['draft/multiline']).toBe('max-bytes=4096,max-lines=24');
  expect(ev.capValues['draft/metadata-2']).toBe('before-connect,max-subs=10');
});

test('no metadata subscription without the cap', async () => {
  const [, p] = await registered('batch');
  await p.assertQuiet();
});

test('renamed channels take their log along', async () => {
  const [h, p] = await registered('draft/channel-rename');
  await p.send(':x!u@h PRIVMSG #Old :before');
  await p.send(':op!u@h RENAME #Old #New :tidying up');
  await p.send(':x!u@h PRIVMSG #new :after');
  const rename = await h.event((m) => m.command === 'RENAME');
  expect(rename.params).toEqual(['#Old', '#New', 'tidying up']);
  await h.event((m) => param(m, 1) === 'after');
  h.db.flush();
  expect(h.db.history('t', '#old', null, 10)).toEqual([]);
  expect(h.db.history('t', '#new', null, 10).map((m) => param(m, 1))).toEqual(['before', 'after']);
});
