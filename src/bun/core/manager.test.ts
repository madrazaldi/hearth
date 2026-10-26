// The connection manager: soju bouncer children, send limits, and actor
// lifetimes.

import { afterEach, expect, test } from 'bun:test';
import type { Cmd } from './client';
import { count, Harness, must, Peer, realSleep, recorder, waitEvent } from './harness';
import type { Event } from './events';
import { childId, Manager, MAX_BACKLOG } from './manager';
import { Db } from './store';

const BOUNCER_CAPS = 'batch soju.im/bouncer-networks soju.im/bouncer-networks-notify';

const live: Harness[] = [];
afterEach(async () => {
  for (const h of live.splice(0)) await h.finish();
});

function harness() {
  const h = new Harness();
  live.push(h);
  return h;
}

const added = (e: Event, id: string) => e.type === 'conn-added' && e.conn === id;
const removed = (e: Event, id: string) => e.type === 'conn-removed' && e.conn === id;
const after = (line: string) => line.slice(line.indexOf(' :') + 2);

/** Register a bouncer child: it binds to its network before ending CAP. */
async function registerChild(p: Peer, netid: string) {
  await p.expect('CAP LS');
  await p.send(`:srv CAP * LS :${BOUNCER_CAPS}`);
  const req = await p.expect('CAP REQ');
  expect(req).not.toContain('bouncer-networks-notify'); // children don't need network updates
  await p.send(`:srv CAP * ACK :${after(req)}`);
  expect(await p.line()).toBe(`BOUNCER BIND ${netid}`);
  expect(await p.line()).toBe('CAP END');
  await p.welcome();
}

test('bouncer networks come and go live', async () => {
  const h = harness();
  h.mgr.connect(h.server((s) => (s.autojoin = ['#a'])));

  const control = await h.listener.accept();
  await control.expect('CAP LS');
  await control.send(`:srv CAP * LS :${BOUNCER_CAPS}`);
  const req = await control.expect('CAP REQ');
  expect(req).toContain('soju.im/bouncer-networks-notify');
  await control.send(`:srv CAP * ACK :${after(req)}`);
  expect(await control.line()).toBe('CAP END');
  await control.welcome();
  // The control connection joins nothing; each network does its own.
  await control.assertQuiet();

  await control.send('@batch=b :srv BOUNCER NETWORK 42 name=Libera;state=connected');
  let child = await h.listener.accept();
  const ev = await waitEvent(h.events, 'the child to be added', (e) => added(e, 't/42'));
  if (ev.type !== 'conn-added') throw new Error('unreachable');
  expect([ev.server, ev.parent, ev.netid]).toEqual(['t', 't', '42']);
  await registerChild(child, '42');
  // The bouncer restores channels, so the child doesn't autojoin either.
  await child.assertQuiet();

  // Attribute updates for a known network don't spawn a second child.
  await control.send(':srv BOUNCER NETWORK 42 state=disconnected');
  await control.assertQuiet();
  expect(await h.listener.tryAccept(200)).toBeNull();
  expect(count(h.events, (e) => added(e, 't/42'))).toBe(1);

  // Children log under their own id; the control connection logs nothing.
  await child.send(':x!u@h PRIVMSG #chan :on libera');
  await control.send(':x!u@h PRIVMSG #chan :on control');
  await child.assertQuiet();
  await control.assertQuiet();
  h.db.flush();
  expect(h.db.history('t/42', '#chan', null, 10)).toHaveLength(1);
  expect(h.db.history('t', '#chan', null, 10)).toEqual([]);

  // Only the control connection manages networks.
  await child.send(':srv BOUNCER NETWORK 99 name=Other');
  await child.assertQuiet();
  expect(await h.listener.tryAccept(200)).toBeNull();

  // Removal quits the child exactly once.
  await control.send(':srv BOUNCER NETWORK 42 *');
  expect(await child.closed()).toEqual(['QUIT :Hearth']);
  await waitEvent(h.events, 'the child to be removed', (e) => removed(e, 't/42'));
  await control.assertQuiet();
  await realSleep(50);
  expect(count(h.events, (e) => removed(e, 't/42'))).toBe(1);
  expect(() => h.mgr.send('t/42', ['PRIVMSG #a :x'])).toThrow();

  // Re-adding it brings up a fresh child.
  await control.send(':srv BOUNCER NETWORK 42 name=Libera');
  child = await h.listener.accept();
  await registerChild(child, '42');

  // Disconnecting the server takes its children down too.
  h.mgr.disconnect('t', 'bye');
  expect(await control.closed()).toEqual(['QUIT :bye']);
  expect((await child.closed()).at(-1)).toBe('QUIT :bye');
});

test('child ids nest under the server', () => {
  expect(childId('srv', '42')).toBe('srv/42');
});

test('sends are sanitised and bounded', async () => {
  const h = harness();
  h.mgr.connect(h.server());
  const p = await h.listener.accept();
  await p.register('');
  await h.event((m) => m.command === '422');

  expect(() => h.mgr.send('nope', ['PING x'])).toThrow('no such connection');

  // Embedded line breaks can't smuggle in a second command; blank lines are dropped.
  h.mgr.send('t', ['PRIVMSG #a :one\r\nQUIT :x', '', '   ', 'PRIVMSG #a :two']);
  expect(await p.line()).toBe('PRIVMSG #a :one  QUIT :x');
  expect(await p.line()).toBe('PRIVMSG #a :two');

  // A flood far beyond what the pacing can drain is refused, not queued.
  h.mgr.send('t', Array.from({ length: 400 }, (_, i) => `PRIVMSG #a :${i}`));
  const refused = await must(
    'the backlog cap to kick in',
    (async () => {
      for (;;) {
        try {
          h.mgr.send('t', ['PRIVMSG #a :more']);
        } catch (e) {
          return (e as Error).message;
        }
        await realSleep(5);
      }
    })(),
  );
  expect(refused).toContain('too many lines');
});

test('reconnect restarts the session', async () => {
  const h = harness();
  h.mgr.connect(h.server());
  const p = await h.listener.accept();
  await p.register('');
  await h.event((m) => m.command === '422');
  h.mgr.reconnect('t');
  expect(await p.closed()).toEqual(['QUIT :Reconnecting']);
  const p2 = await h.listener.accept();
  await p2.expect('CAP LS');
});

test('a finished actor leaves its successor alone', async () => {
  const h = harness();
  h.mgr.connect(h.server());
  const old = await h.listener.accept();
  await old.expect('USER');

  // Replace the connection before the old actor has wound down.
  h.mgr.disconnect('t');
  h.mgr.connect(h.server());
  const fresh = await h.listener.accept();
  expect(await old.closed()).toEqual(['QUIT :Hearth']);
  await fresh.register('');
  await h.event((m) => m.command === '422');

  await realSleep(50);
  expect(count(h.events, (e) => removed(e, 't'))).toBe(1); // only the explicit disconnect
  h.mgr.send('t', ['PRIVMSG #a :still here']);
  expect(await fresh.line()).toBe('PRIVMSG #a :still here');
});

test('connecting twice keeps one connection', async () => {
  const h = harness();
  h.mgr.connect(h.server());
  h.mgr.connect(h.server());
  const p = await h.listener.accept();
  await p.expect('USER');
  expect(await h.listener.tryAccept(200)).toBeNull();
  expect(count(h.events, (e) => added(e, 't'))).toBe(1);
});

// ---- Unit tests against stand-in connections ----

function fake(m: Manager, id: string, generation: number, server = 's') {
  const cmds: Cmd[] = [];
  const backlog = m.adopt(id, { command: (c) => cmds.push(c) }, server, generation);
  return { cmds, backlog };
}

function bare() {
  const db = Db.memory();
  const { emitter } = recorder();
  return { m: new Manager(emitter, db), db };
}

test('send is refused at the backlog cap', () => {
  const { m, db } = bare();
  const { cmds, backlog } = fake(m, 's', 1);
  backlog.n = MAX_BACKLOG - 1;
  m.send('s', ['A\rB\nC', ' ', 'D']);
  expect(cmds).toEqual([{ type: 'send', lines: ['A B C', 'D'] }]);
  backlog.n = MAX_BACKLOG;
  expect(() => m.send('s', ['E'])).toThrow();
  expect(cmds).toHaveLength(1);
  db.close();
});

test('stale actors do not remove their successor', () => {
  const { m, db } = bare();
  fake(m, 's', 2);
  m.actorFinished('s', 1);
  expect(m.has('s')).toBe(true);
  m.actorFinished('s', 2);
  expect(m.has('s')).toBe(false);
  db.close();
});

test('disconnect takes children along', () => {
  const { m, db } = bare();
  const parent = fake(m, 's', 1);
  const child = fake(m, childId('s', '7'), 2);
  const other = fake(m, 'o', 3, 'o');
  m.disconnect('s', 'bye');
  expect(parent.cmds).toEqual([{ type: 'quit', reason: 'bye' }]);
  expect(child.cmds).toEqual([{ type: 'quit', reason: 'bye' }]);
  expect(other.cmds).toEqual([]);
  expect(m.has('o')).toBe(true);
  db.close();
});
