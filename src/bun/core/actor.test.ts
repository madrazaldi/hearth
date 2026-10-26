// Drives a real connection actor against a scripted IRC server on localhost.

import { afterEach, expect, test } from 'bun:test';
import { createServer, type Server } from 'node:net';
import { b64decode, Harness, must, Peer, realSleep } from './harness';
import { param, tag } from './message';

const live: Harness[] = [];
const servers: Server[] = [];
afterEach(async () => {
  for (const h of live.splice(0)) await h.finish();
  for (const s of servers.splice(0)) s.close();
});

function harness() {
  const h = new Harness();
  live.push(h);
  return h;
}

test('SCRAM falls back to PLAIN', async () => {
  const h = harness();
  h.mgr.connect(
    h.server((s) => {
      s.sasl = 'plain';
      s.saslUsername = 'acct';
      s.saslPassword = 'pw';
    }),
  );

  const p = await h.listener.accept();
  await p.expect('CAP LS');
  await p.send(':srv CAP * LS :sasl=PLAIN,SCRAM-SHA-256 server-time');
  const req = await p.expect('CAP REQ');
  expect(req).toContain('sasl');
  expect(req).toContain('server-time');
  await p.send(':srv CAP * ACK :sasl server-time');
  expect(await p.expect('AUTHENTICATE')).toBe('AUTHENTICATE SCRAM-SHA-256');
  await p.send('AUTHENTICATE +');
  const first = await p.expect('AUTHENTICATE');
  const decoded = b64decode(first.slice('AUTHENTICATE '.length)).toString();
  expect(decoded.startsWith('n,,n=acct,r=')).toBe(true);
  await p.send(':srv 904 me :SASL authentication failed');
  expect(await p.expect('AUTHENTICATE')).toBe('AUTHENTICATE PLAIN');
  await p.send('AUTHENTICATE +');
  const plain = await p.expect('AUTHENTICATE');
  expect(b64decode(plain.slice('AUTHENTICATE '.length)).toString()).toBe('acct\0acct\0pw');
  await p.send(':srv 903 me :SASL authentication successful');
  await p.expect('CAP END');
});

test('STS upgrades plaintext to TLS', async () => {
  const h = harness();
  const hello = Promise.withResolvers<Buffer>();
  const tls = createServer((sock) => {
    sock.on('error', () => {});
    sock.once('data', (d) => hello.resolve(Buffer.from(d)));
  });
  servers.push(tls);
  await new Promise<void>((r) => tls.listen(0, '127.0.0.1', r));
  const tlsPort = (tls.address() as { port: number }).port;
  h.mgr.connect(h.server());

  const p = await h.listener.accept();
  await p.expect('CAP LS');
  await p.send(`:srv CAP * LS :sts=port=${tlsPort},duration=300 server-time`);

  // The client must hang up without requesting caps and reconnect over TLS.
  const first = await must('the TLS reconnect', hello.promise);
  expect(first[0]).toBe(0x16); // TLS handshake record
  expect(first[1]).toBe(0x03);
  // A policy learned over plaintext must not be persisted.
  expect(h.db.stsPolicy('127.0.0.1')).toBeNull();
});

test('labeled history and frontend labels', async () => {
  const h = harness();
  h.mgr.connect(h.server());
  const p = await h.listener.accept();
  await p.register('batch draft/chathistory labeled-response message-tags server-time');
  await realSleep(100);

  // Two requests in flight; the replies arrive in the opposite order.
  const a = h.mgr.history('t', 'messages', '#a', 'CHATHISTORY LATEST #a * 10');
  const reqA = await p.expect('@label=');
  const b = h.mgr.history('t', 'messages', '#b', 'CHATHISTORY LATEST #b * 10');
  b.catch(() => {});
  const reqB = await p.expect('@label=');
  const label = (l: string) => l.split(' ')[0].slice('@label='.length);
  const [la, lb] = [label(reqA), label(reqB)];

  await p.send(`@label=${lb} :srv FAIL CHATHISTORY INVALID_TARGET LATEST #b :No such channel`);
  await p.send(`@label=${la} :srv BATCH +w labeled-response`);
  await p.send('@batch=w :srv BATCH +h chathistory #a');
  await p.send('@batch=h;msgid=1;time=2026-01-01T00:00:00.000Z :x!u@h PRIVMSG #a :one');
  await p.send('@batch=h;msgid=2;time=2026-01-01T00:00:01.000Z :x!u@h PRIVMSG #a :two');
  await p.send('@batch=w :srv BATCH -h');
  await p.send(':srv BATCH -w');

  const gotA = await a;
  expect(gotA).toHaveLength(2);
  expect(param(gotA[1], 1)).toBe('two');
  await expect(b).rejects.toThrow('No such channel');

  // History with msgids is backfilled into the local log.
  h.db.flush();
  expect(h.db.history('t', '#a', null, 10)).toHaveLength(2);

  // A labeled response to a line the frontend sent keeps its label, even for
  // messages nested in the batch.
  h.mgr.send('t', ['@label=f7 WHOIS x']);
  await p.expect('@label=f7 WHOIS');
  await p.send('@label=f7 :srv BATCH +r labeled-response');
  await p.send('@batch=r :srv 311 me x u h * :Real');
  await p.send(':srv BATCH -r');
  const whois = await h.event((m) => m.command === '311');
  expect(tag(whois, 'label')).toBe('f7');
});

test('logs live and outgoing messages', async () => {
  const h = harness();
  h.mgr.connect(h.server());
  const p: Peer = await h.listener.accept();
  await p.register('');

  await p.send(':x!u@h PRIVMSG #Chan :hello channel');
  await p.send(':x!u@h PRIVMSG me :hello you');
  await p.send(':x!u@h PRIVMSG me :\x01VERSION\x01');
  await p.send(':srv.example NOTICE me :server notice');
  await h.event((m) => m.command === 'NOTICE');
  h.mgr.send('t', ['PRIVMSG x :reply']);
  await p.expect('PRIVMSG x');

  h.db.flush();
  expect(h.db.history('t', '#chan', null, 10)).toHaveLength(1); // channel names are case-folded
  const query = h.db.history('t', 'x', null, 10);
  expect(query).toHaveLength(2); // incoming and our own reply, no CTCP
  expect(query[1].source?.nick).toBe('me');
  expect(h.db.history('t', 'srv.example', null, 10)).toEqual([]);
});
