import { afterEach, beforeEach, expect, test } from 'bun:test';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { defaultServer, normalizeServer, publicServer, Store } from './config';
import { useKeychain, type Keychain } from './secrets';

let dir: string;
let vault: Map<string, string>;
let failing = false;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'hearth-config-'));
  vault = new Map();
  failing = false;
  const k: Keychain = {
    async get(service, name) {
      if (failing) throw new Error('no keychain');
      return vault.get(`${service}/${name}`) ?? null;
    },
    async set(service, name, value) {
      if (failing) throw new Error('no keychain');
      vault.set(`${service}/${name}`, value);
    },
    async delete(service, name) {
      if (failing) throw new Error('no keychain');
      return vault.delete(`${service}/${name}`);
    },
  };
  useKeychain(k);
});

afterEach(() => {
  useKeychain(null);
  rmSync(dir, { recursive: true, force: true });
});

test('a missing file loads as an empty config', async () => {
  expect(await new Store(dir).load()).toEqual({ servers: [], settings: {} });
});

test('missing fields take their defaults', () => {
  const s = normalizeServer({ id: 'a', host: 'irc.example', nick: 'me' });
  expect(s).toEqual({ ...defaultServer(), id: 'a', host: 'irc.example', nick: 'me' });
  expect(s.port).toBe(6697);
  expect(s.tls).toBe(true);
  expect(s.autoconnect).toBe(true);
});

test('wrongly typed fields are rejected', () => {
  expect(() => normalizeServer({ port: '6667' })).toThrow();
  expect(() => normalizeServer({ port: 70000 })).toThrow();
  expect(() => normalizeServer({ tls: 'yes' })).toThrow();
  expect(() => normalizeServer({ autojoin: [1] })).toThrow();
  expect(() => normalizeServer({ sasl: 'scram' })).toThrow();
  expect(() => normalizeServer([])).toThrow();
});

test('the layout matches what the Rust engine wrote', async () => {
  writeFileSync(
    join(dir, 'config.json'),
    JSON.stringify({
      servers: [
        {
          id: 'abc123',
          name: 'Libera',
          host: 'irc.libera.chat',
          port: 6697,
          tls: true,
          acceptInvalidCerts: false,
          websocket: false,
          websocketPath: '/',
          nick: 'me',
          username: '',
          realname: '',
          password: '',
          sasl: 'plain',
          saslUsername: 'acct',
          saslPassword: '',
          autoconnect: true,
          autojoin: ['#a'],
          perform: [],
          clientCert: '',
        },
      ],
      settings: { theme: 'dark' },
    }),
  );
  const c = await new Store(dir).load();
  expect(c.servers[0].saslUsername).toBe('acct');
  expect(c.settings).toEqual({ theme: 'dark' });
});

test('save writes atomically with private permissions and no internal fields', async () => {
  const store = new Store(dir);
  const s = { ...defaultServer(), id: 'a', host: 'h', nick: 'n', password: 'pw', inKeychain: false };
  store.save({ servers: [s], settings: { x: 1 } });
  const text = readFileSync(join(dir, 'config.json'), 'utf8');
  expect(text).not.toContain('inKeychain');
  expect(JSON.parse(text).servers[0].password).toBe('pw'); // no keychain: stays in the file
  if (process.platform !== 'win32') expect(statSync(join(dir, 'config.json')).mode & 0o777).toBe(0o600);
  expect(existsSync(join(dir, 'config.json.tmp'))).toBe(false);
  expect(publicServer(s)).not.toHaveProperty('inKeychain');
});

test('passwords in the file move to the keychain on load', async () => {
  writeFileSync(join(dir, 'config.json'), JSON.stringify({ servers: [{ id: 'a', host: 'h', nick: 'n', password: 'pw', saslPassword: 'sp' }], settings: {} }));
  const store = new Store(dir);
  const c = await store.load();
  expect(c.servers[0].password).toBe('pw'); // still usable in memory
  expect(vault.get('app.hearth.irc/a:password')).toBe('pw');
  expect(vault.get('app.hearth.irc/a:sasl')).toBe('sp');
  const file = JSON.parse(readFileSync(join(dir, 'config.json'), 'utf8'));
  expect(file.servers[0].password).toBe('');
  expect(file.servers[0].saslPassword).toBe('');
  // A fresh load reads them back from the keychain.
  const again = await new Store(dir).load();
  expect(again.servers[0].password).toBe('pw');
  expect(again.servers[0].saslPassword).toBe('sp');
});

test('without a keychain the secrets stay in the file', async () => {
  failing = true;
  writeFileSync(join(dir, 'config.json'), JSON.stringify({ servers: [{ id: 'a', host: 'h', nick: 'n', password: 'pw' }], settings: {} }));
  const c = await new Store(dir).load();
  expect(c.servers[0].password).toBe('pw');
  expect(JSON.parse(readFileSync(join(dir, 'config.json'), 'utf8')).servers[0].password).toBe('pw');
});

test('an invalid file is backed up and replaced by an empty config', async () => {
  writeFileSync(join(dir, 'config.json'), '{ not json');
  expect(await new Store(dir).load()).toEqual({ servers: [], settings: {} });
  expect(readFileSync(join(dir, 'config.json.bak'), 'utf8')).toBe('{ not json');
});
