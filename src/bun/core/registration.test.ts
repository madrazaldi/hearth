// Registration: CAP negotiation, SASL, nick fallback and what follows the
// welcome.

import { afterEach, expect, test } from 'bun:test';
import { createHash, createHmac, pbkdf2Sync } from 'node:crypto';
import type { ServerConfig } from './config';
import { b64decode, b64encode, Harness, Peer, waitEvent } from './harness';
import { param } from './message';

const live: Harness[] = [];
afterEach(async () => {
  for (const h of live.splice(0)) await h.finish();
});

async function start(configure?: (s: ServerConfig) => void): Promise<[Harness, Peer]> {
  const h = new Harness();
  live.push(h);
  h.mgr.connect(h.server(configure));
  return [h, await h.listener.accept()];
}

const plain = (user: string, password: string) => (s: ServerConfig) => {
  s.sasl = 'plain';
  s.saslUsername = user;
  s.saslPassword = password;
};

const capsOf = (line: string) => new Set(line.slice(line.indexOf(' :') + 2).split(/\s+/).filter(Boolean));
const after = (line: string) => line.slice(line.indexOf(' :') + 2);

async function registeredNick(h: Harness): Promise<string> {
  const e = await waitEvent(h.events, 'registration', (e) => e.type === 'registered');
  return e.type === 'registered' ? e.nick : '';
}

/** The capability set carried by the latest Caps event, once it satisfies `pred`. */
async function capsEvent(h: Harness, pred: (caps: string[]) => boolean): Promise<string[]> {
  const e = await waitEvent(h.events, 'a caps event', (e) => e.type === 'caps' && pred(e.caps));
  return e.type === 'caps' ? e.caps : [];
}

test('opening lines carry pass and user', async () => {
  const [, p] = await start((s) => {
    s.password = 'serverpw';
    s.username = 'ident';
    s.realname = 'Real Name';
  });
  expect(await p.line()).toBe('CAP LS 302');
  expect(await p.line()).toBe('PASS serverpw');
  expect(await p.line()).toBe('NICK me');
  expect(await p.line()).toBe('USER ident 0 * :Real Name');
});

test('multiline CAP LS requests only wanted caps', async () => {
  const [h, p] = await start();
  await p.expect('USER');
  await p.send(':srv CAP * LS * :account-notify away-notify batch cap-notify chghost draft/chathistory');
  await p.send(':srv CAP * LS * :echo-message extended-join invite-notify labeled-response message-tags');
  // Nothing is requested until the last line of the listing.
  await p.assertQuiet();
  await p.send(':srv CAP * LS :multi-prefix server-time setname sasl=PLAIN znc.in/self-message draft/unknown=1');
  const req = await p.expect('CAP REQ');
  expect(req.length).toBeLessThan(512);
  const want = new Set(
    ('account-notify away-notify batch cap-notify chghost draft/chathistory echo-message extended-join ' +
      'invite-notify labeled-response message-tags multi-prefix server-time setname').split(' '),
  );
  // sasl is left out: no credentials are configured.
  expect(capsOf(req)).toEqual(want);

  // CAP END waits for the ACK.
  await p.assertQuiet();
  await p.send(`:srv CAP me ACK :${after(req)}`);
  expect(await p.line()).toBe('CAP END');
  await p.welcome();
  const e = await waitEvent(h.events, 'registration', (e) => e.type === 'registered');
  if (e.type !== 'registered') throw new Error('unreachable');
  expect(new Set(e.caps)).toEqual(want);
});

test('every wanted cap is requested in lines under the limit', async () => {
  const [, p] = await start(plain('acct', 'pw'));
  await p.expect('CAP LS');
  const all =
    'account-notify account-tag away-notify batch cap-notify chghost draft/account-registration ' +
    'draft/channel-rename draft/chathistory draft/event-playback draft/extended-monitor ' +
    'draft/message-redaction draft/metadata-2 draft/multiline draft/read-marker echo-message ' +
    'extended-join extended-monitor invite-notify labeled-response message-tags multi-prefix sasl ' +
    'server-time setname soju.im/bouncer-networks soju.im/bouncer-networks-notify soju.im/search ' +
    'standard-replies userhost-in-names';
  await p.send(`:srv CAP * LS :${all}`);
  const want = new Set(all.split(' '));
  const got = new Set<string>();
  let reqs = 0;
  while (got.size !== want.size) {
    const req = await p.expect('CAP REQ');
    expect(req.length).toBeLessThan(512);
    for (const c of capsOf(req)) got.add(c);
    reqs++;
  }
  expect(got).toEqual(want);
  expect(reqs).toBe(2); // the list is split, not sent one cap at a time
});

test('NAK still ends negotiation', async () => {
  const [h, p] = await start();
  await p.expect('CAP LS');
  await p.send(':srv CAP * LS :server-time batch');
  const req = await p.expect('CAP REQ');
  await p.send(`:srv CAP * NAK :${after(req)}`);
  expect(await p.line()).toBe('CAP END');
  await p.welcome();
  await registeredNick(h);
});

test('no caps at all ends immediately', async () => {
  const [, p] = await start(plain('acct', 'pw'));
  await p.expect('USER');
  await p.send(':srv CAP * LS :');
  expect(await p.line()).toBe('CAP END');
});

test('cap-notify adds and removes caps', async () => {
  const [h, p] = await start(plain('acct', 'pw'));
  await p.register('cap-notify server-time');
  await registeredNick(h);

  // New caps are requested, except sasl once registered.
  await p.send(':srv CAP me NEW :away-notify sasl=PLAIN made-up');
  expect(await p.line()).toBe('CAP REQ :away-notify');
  await p.send(':srv CAP me ACK :away-notify');
  expect(await capsEvent(h, (c) => c.includes('away-notify'))).toEqual(['away-notify', 'cap-notify', 'server-time']);
  await p.assertQuiet();

  // Offering an enabled cap again doesn't request it twice.
  await p.send(':srv CAP me NEW :server-time');
  await p.assertQuiet();

  await p.send(':srv CAP me DEL :away-notify');
  expect(await capsEvent(h, (c) => c.length === 2)).toEqual(['cap-notify', 'server-time']);

  // A removal can also arrive as an ACK with a '-' prefix.
  await p.send(':srv CAP me ACK :-server-time');
  expect(await capsEvent(h, (c) => c.length === 1)).toEqual(['cap-notify']);
  // Registration is over: none of this sends CAP END again.
  await p.assertQuiet();
});

test('ISUPPORT tokens are parsed and unescaped', async () => {
  const [h, p] = await start();
  await p.register('');
  await p.send(String.raw`:srv 005 me NETWORK=Foo\x20Net STATUSMSG=@+ EXCEPTS :are supported`);
  await p.send(':srv 005 me -EXCEPTS NICKLEN=9 :are supported');
  const e = await waitEvent(h.events, 'isupport', (e) => e.type === 'isupport' && 'NICKLEN' in e.isupport);
  if (e.type !== 'isupport') throw new Error('unreachable');
  expect(e.isupport.NETWORK).toBe('Foo Net');
  expect(e.isupport.STATUSMSG).toBe('@+');
  expect(e.isupport.CASEMAPPING).toBe('ascii'); // earlier lines are kept
  expect('EXCEPTS' in e.isupport).toBe(false); // negated tokens are removed
});

test('nick in use falls back during registration only', async () => {
  const [h, p] = await start((s) => {
    s.autojoin = ['#a'];
    s.perform = ['PRIVMSG NickServ :hi from $nick'];
  });
  await p.expect('CAP LS');
  await p.send(':srv CAP * LS :');
  await p.expect('CAP END');
  await p.send(':srv 433 * me :Nickname is already in use');
  expect(await p.line()).toBe('NICK me_');
  await p.send(':srv 432 * me_ :Erroneous nickname');
  expect(await p.line()).toBe('NICK me__');
  await p.send(':srv 001 me__ :Welcome');
  await p.send(':srv 422 me__ :No MOTD');
  expect(await registeredNick(h)).toBe('me__');
  expect(await p.line()).toBe('JOIN #a');
  expect(await p.line()).toBe('PRIVMSG NickServ :hi from me__');

  // Once registered, a failed /nick is the user's business.
  await p.send(':srv 433 me__ taken :Nickname is already in use');
  await p.assertQuiet();
});

test('nick fallback at the length limit still changes the nick', async () => {
  const long = 'abcdefghijklmnopqrstuvwxyz0123';
  const [, p] = await start((s) => (s.nick = long));
  await p.expect('CAP LS');
  await p.send(':srv CAP * LS :');
  await p.expect('CAP END');
  const seen = [long];
  for (let i = 0; i < 3; i++) {
    await p.send(`:srv 433 * ${seen.at(-1)} :Nickname is already in use`);
    const nick = (await p.expect('NICK ')).slice('NICK '.length);
    expect([...nick].length).toBeLessThanOrEqual(30);
    expect(seen).not.toContain(nick);
    seen.push(nick);
  }
});

test('autojoin is sent in groups of ten', async () => {
  const chans = Array.from({ length: 12 }, (_, i) => `#c${i}`);
  const [, p] = await start((s) => (s.autojoin = chans));
  await p.register('');
  expect(await p.line()).toBe(`JOIN ${chans.slice(0, 10).join(',')}`);
  expect(await p.line()).toBe(`JOIN ${chans.slice(10).join(',')}`);
  // A later MOTD end (e.g. /motd) doesn't join again.
  await p.send(':srv 376 me :End of MOTD');
  await p.assertQuiet();
});

async function saslStart(p: Peer, ls: string) {
  await p.expect('CAP LS');
  await p.send(`:srv CAP * LS :${ls}`);
  const req = await p.expect('CAP REQ');
  expect(capsOf(req).has('sasl')).toBe(true);
  await p.send(`:srv CAP * ACK :${after(req)}`);
}

test('PLAIN payload is split into 400-byte chunks', async () => {
  // 10 bytes of "acct\0acct\0" plus 290 of password: exactly 400 base64 bytes.
  const password = 'p'.repeat(290);
  const [, p] = await start(plain('acct', password));
  await saslStart(p, 'sasl');
  expect(await p.line()).toBe('AUTHENTICATE PLAIN');
  await p.send('AUTHENTICATE +');
  const chunk = await p.line();
  expect(chunk.length).toBe('AUTHENTICATE '.length + 400);
  // A final chunk of exactly 400 bytes must be followed by an empty one.
  expect(await p.line()).toBe('AUTHENTICATE +');
  expect(b64decode(chunk.slice('AUTHENTICATE '.length)).toString()).toBe(`acct\0acct\0${password}`);
  await p.send(':srv 900 me me!u@h acct :You are now logged in as acct');
  await p.send(':srv 903 me :SASL authentication successful');
  expect(await p.line()).toBe('CAP END');
});

test('SASL failures end negotiation and register anyway', async () => {
  for (const numeric of ['902', '904', '905', '906']) {
    const [h, p] = await start(plain('acct', 'pw'));
    await saslStart(p, 'sasl=PLAIN');
    expect(await p.line()).toBe('AUTHENTICATE PLAIN');
    await p.send('AUTHENTICATE +');
    await p.expect('AUTHENTICATE ');
    await p.send(`:srv ${numeric} me :SASL failed`);
    expect(await p.line()).toBe('CAP END');
    await p.welcome();
    await registeredNick(h);
    // The failure reaches the UI.
    await h.event((m) => m.command === numeric);
    await h.finish();
    live.pop();
  }
});

test('SCRAM failure without PLAIN offered gives up', async () => {
  const [, p] = await start(plain('acct', 'pw'));
  await saslStart(p, 'sasl=SCRAM-SHA-256');
  expect(await p.line()).toBe('AUTHENTICATE SCRAM-SHA-256');
  await p.send('AUTHENTICATE +');
  await p.expect('AUTHENTICATE ');
  await p.send(':srv 908 me SCRAM-SHA-256 :are available SASL mechanisms');
  await p.send(':srv 904 me :SASL authentication failed');
  expect(await p.line()).toBe('CAP END');
});

test('PLAIN fallback is tried only once', async () => {
  const [, p] = await start(plain('acct', 'pw'));
  await saslStart(p, 'sasl=SCRAM-SHA-256,PLAIN');
  expect(await p.line()).toBe('AUTHENTICATE SCRAM-SHA-256');
  await p.send(':srv 904 me :SASL authentication failed');
  expect(await p.line()).toBe('AUTHENTICATE PLAIN');
  await p.send('AUTHENTICATE +');
  await p.expect('AUTHENTICATE ');
  await p.send(':srv 904 me :SASL authentication failed');
  expect(await p.line()).toBe('CAP END');
});

test('SASL without a mechanism list uses PLAIN', async () => {
  const [, p] = await start(plain('', 'pw'));
  await saslStart(p, 'sasl');
  expect(await p.line()).toBe('AUTHENTICATE PLAIN');
  await p.send('AUTHENTICATE +');
  const line = await p.line();
  // With no account name the nick is used.
  expect(b64decode(line.slice('AUTHENTICATE '.length)).toString()).toBe('me\0me\0pw');
});

test('SASL is skipped without credentials', async () => {
  const [, p] = await start(plain('acct', ''));
  await p.expect('USER');
  await p.send(':srv CAP * LS :sasl=PLAIN server-time');
  expect(await p.line()).toBe('CAP REQ :server-time');
  await p.send(':srv CAP * ACK :server-time');
  expect(await p.line()).toBe('CAP END');
});

test('EXTERNAL sends an empty response', async () => {
  const [, p] = await start((s) => {
    s.sasl = 'external';
    // A password is ignored: the certificate is the credential.
    s.saslPassword = 'pw';
    s.clientCert = '/nonexistent/cert.pem';
  });
  await saslStart(p, 'sasl=EXTERNAL,SCRAM-SHA-256,PLAIN');
  expect(await p.line()).toBe('AUTHENTICATE EXTERNAL');
  await p.send('AUTHENTICATE +');
  expect(await p.line()).toBe('AUTHENTICATE +');
  await p.send(':srv 903 me :SASL authentication successful');
  expect(await p.line()).toBe('CAP END');
});

const hmac = (key: Uint8Array, data: string | Uint8Array) => createHmac('sha256', key).update(data).digest();

/** Server side of a SCRAM-SHA-256 exchange: whether the client's proof checks out, and the server-final message. */
function scramServer(password: string, clientFirst: string, serverFirst: string, clientFinal: string): [boolean, string] {
  const bare = clientFirst.replace(/^n,,/, '');
  const attr = (msg: string, k: string) => msg.split(',').find((kv) => kv.startsWith(`${k}=`))!.slice(k.length + 1);
  const salt = b64decode(attr(serverFirst, 's'));
  const salted = pbkdf2Sync(password, salt, Number(attr(serverFirst, 'i')), 32, 'sha256');
  const clientKey = hmac(salted, 'Client Key');
  const storedKey = createHash('sha256').update(clientKey).digest();
  const cut = clientFinal.lastIndexOf(',p=');
  const withoutProof = clientFinal.slice(0, cut);
  const proof = clientFinal.slice(cut + 3);
  const auth = `${bare},${serverFirst},${withoutProof}`;
  const signature = hmac(storedKey, auth);
  const expected = Buffer.from(clientKey.map((b, i) => b ^ signature[i]));
  const serverSig = hmac(hmac(salted, 'Server Key'), auth);
  return [b64decode(proof).equals(expected), `v=${b64encode(serverSig)}`];
}

/** Run SCRAM up to the server-final message, which `tamper` may alter. */
async function scramExchange(p: Peer, tamper: (f: string) => string) {
  await saslStart(p, 'sasl=SCRAM-SHA-256,PLAIN');
  expect(await p.line()).toBe('AUTHENTICATE SCRAM-SHA-256');
  await p.send('AUTHENTICATE +');
  const first = b64decode((await p.line()).slice('AUTHENTICATE '.length)).toString();
  const cnonce = first.split(',r=')[1];
  const serverFirst = `r=${cnonce}srvnonce,s=${b64encode('salty')},i=4096`;
  await p.send(`AUTHENTICATE ${b64encode(serverFirst)}`);
  const fin = b64decode((await p.line()).slice('AUTHENTICATE '.length)).toString();
  const [ok, serverFinal] = scramServer('correct horse', first, serverFirst, fin);
  expect(ok).toBe(true); // client proof must verify
  await p.send(`AUTHENTICATE ${b64encode(tamper(serverFinal))}`);
}

test('SCRAM completes with mutual authentication', async () => {
  const [, p] = await start(plain('acct', 'correct horse'));
  await scramExchange(p, (f) => f);
  // The server proved itself; the client's last word is empty.
  expect(await p.line()).toBe('AUTHENTICATE +');
  await p.send(':srv 903 me :SASL authentication successful');
  expect(await p.line()).toBe('CAP END');
});

test('SCRAM aborts on a forged server signature', async () => {
  const [h, p] = await start(plain('acct', 'correct horse'));
  await scramExchange(p, () => `v=${b64encode(new Uint8Array(32))}`);
  expect(await p.line()).toBe('AUTHENTICATE *');
  const fail = await h.event((m) => m.command === 'FAIL');
  expect(param(fail, 0)).toBe('AUTHENTICATE');
  expect(param(fail, 2)).toContain('signature');
  await p.send(':srv 906 me :SASL authentication aborted');
  expect(await p.line()).toBe('CAP END');
});

test('SCRAM aborts on a foreign nonce', async () => {
  const [, p] = await start(plain('acct', 'pw'));
  await saslStart(p, 'sasl=SCRAM-SHA-256');
  await p.expect('AUTHENTICATE SCRAM');
  await p.send('AUTHENTICATE +');
  await p.expect('AUTHENTICATE ');
  await p.send(`AUTHENTICATE ${b64encode(`r=notyours,s=${b64encode('salt')},i=4096`)}`);
  expect(await p.line()).toBe('AUTHENTICATE *');
  await p.send(':srv 906 me :SASL authentication aborted');
  expect(await p.line()).toBe('CAP END');
});
