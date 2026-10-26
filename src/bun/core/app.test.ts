import { afterEach, beforeEach, expect, test } from 'bun:test';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Core } from './app';
import { defaultServer } from './config';
import { realSleep } from './harness';
import { useKeychain, type Keychain } from './secrets';

let dir: string;
let cores: Core[] = [];
const vault = new Map<string, string>();
const legacyVault = new Map<string, string>();

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'hearth-core-'));
  vault.clear();
  legacyVault.clear();
  const k: Keychain = {
    get: async (s, n) => vault.get(`${s}/${n}`) ?? null,
    set: async (s, n, v) => void vault.set(`${s}/${n}`, v),
    delete: async (s, n) => vault.delete(`${s}/${n}`),
    legacy: {
      get: async (s, n) => legacyVault.get(`${s}/${n}`) ?? null,
      delete: async (s, n) => void legacyVault.delete(`${s}/${n}`),
    },
  };
  useKeychain(k);
});

afterEach(async () => {
  const done = cores.splice(0);
  for (const c of done) c.shutdown();
  await realSleep(10);
  for (const c of done) c.close();
  useKeychain(null);
  rmSync(dir, { recursive: true, force: true });
});

async function open() {
  const core = await Core.open(dir, dir, () => {});
  cores.push(core);
  return core;
}

test('settings and config round trip', async () => {
  const core = await open();
  core.saveSettings({ storeLogs: false, theme: 'dark' });
  expect(core.getConfig().settings.theme).toBe('dark');
  expect(core.getConfig().servers).toEqual([]);
  expect(() => core.connect('nope')).toThrow('unknown server');
  expect(() => core.send('nope', [])).toThrow();
  expect(core.search({ text: 'x', limit: 5 })).toEqual([]);
});

test('saved buffers round trip', async () => {
  const core = await open();
  core.saveBuffers('c', [
    { name: '#Hearth', kind: 'channel', key: '' },
    { name: 'alice', kind: 'query', key: '' },
  ]);
  expect(core.loadBuffers('c')).toHaveLength(2);
});

test('saving a server fills in its id and name and keeps secrets out of the file', async () => {
  const core = await open();
  const saved = await core.saveServer({ ...defaultServer(), host: 'irc.example', nick: 'me', password: 'hunter2', sasl: 'plain', saslUsername: 'a', saslPassword: 'sp' });
  expect(saved.id).toMatch(/^[0-9a-f]{12}$/);
  expect(saved.name).toBe('irc.example');
  expect(saved).not.toHaveProperty('inKeychain');
  expect(vault.get(`app.hearth.irc/${saved.id}:password`)).toBe('hunter2');
  expect(vault.get(`app.hearth.irc/${saved.id}:sasl`)).toBe('sp');
  const onDisk = readFileSync(join(dir, 'config.json'), 'utf8');
  expect(onDisk).not.toContain('hunter2');
  expect(onDisk).not.toContain('"sp"');
  expect(core.getConfig().servers[0].password).toBe('hunter2');

  // A SASL password only counts for PLAIN.
  const other = await core.saveServer({ ...saved, sasl: 'none', saslPassword: 'ignored' });
  expect(other.saslPassword).toBe('');

  // Saving again updates in place.
  await core.saveServer({ ...saved, nick: 'renamed' });
  expect(core.getConfig().servers).toHaveLength(1);
  expect(core.getConfig().servers[0].nick).toBe('renamed');
});

test('a server needs a host', async () => {
  const core = await open();
  await expect(core.saveServer({ ...defaultServer(), host: '  ' })).rejects.toThrow('host is required');
});

test('deleting a server removes its secrets, logs and entry', async () => {
  const core = await open();
  const saved = await core.saveServer({ ...defaultServer(), host: 'h', nick: 'n', password: 'pw' });
  await core.deleteServer(saved.id);
  expect(core.getConfig().servers).toEqual([]);
  expect(vault.size).toBe(0);
  expect(JSON.parse(readFileSync(join(dir, 'config.json'), 'utf8')).servers).toEqual([]);
});

test('a delete during a save is not undone by it', async () => {
  const core = await open();
  const saved = await core.saveServer({ ...defaultServer(), id: 'x', host: 'h', nick: 'n', password: 'pw' });
  // Both start before either finishes its keychain work.
  const save = core.saveServer({ ...saved, password: 'pw2' });
  const del = core.deleteServer('x');
  await Promise.all([save, del]);
  expect(core.getConfig().servers).toEqual([]);
  expect(vault.size).toBe(0);
});

test('settings the backend acts on are applied', async () => {
  const core = await open();
  expect(core.closeToTray()).toBe(false);
  core.saveSettings({ closeToTray: true });
  expect(core.closeToTray()).toBe(true);
  // Avatars stay off unless the user opted in.
  await expect(core.invoke('avatar', { url: 'https://example.com/a.png' })).rejects.toThrow('avatars are turned off');
});

test('the config survives a restart', async () => {
  const a = await open();
  await a.saveServer({ ...defaultServer(), id: 'x', host: 'h', nick: 'n', password: 'pw' });
  a.saveSettings({ theme: 'light' });
  a.shutdown();
  const b = await open();
  expect(b.getConfig().servers[0].password).toBe('pw');
  expect(b.getConfig().settings).toEqual({ theme: 'light' });
});

test('secrets the Rust engine stored are found and moved over', async () => {
  const a = await open();
  await a.saveServer({ ...defaultServer(), id: 'x', host: 'h', nick: 'n', password: 'pw' });
  a.shutdown();
  for (const [k, v] of vault) legacyVault.set(k, v);
  vault.clear();
  const b = await open();
  expect(b.getConfig().servers[0].password).toBe('pw');
  expect(vault.get('app.hearth.irc/x:password')).toBe('pw');
  // The old entry is gone, so clearing the password sticks.
  expect(legacyVault.size).toBe(0);
  await b.saveServer({ ...b.getConfig().servers[0], password: '' });
  b.shutdown();
  const c = await open();
  expect(c.getConfig().servers[0].password).toBe('');
});

test('local history limits are clamped and results come back', async () => {
  const core = await open();
  expect(core.localHistory('c', '#a', null, null, 0)).toEqual([]);
  expect(core.localHistory('c', '#a', null, 5, 100000)).toEqual([]);
});

test('invoke dispatches by name and rejects with plain messages', async () => {
  const core = await open();
  expect(await core.invoke('get_config')).toEqual({ servers: [], settings: {} });
  expect(await core.invoke('reset')).toBeUndefined();
  const e = await core.invoke('connect', { server: 'nope' }).catch((e) => e);
  expect(e).toBeInstanceOf(Error);
  expect((e as Error).message).toBe('unknown server');
  await expect(core.invoke('no_such_command')).rejects.toThrow('unknown command');
  await expect(core.invoke('connect', {})).rejects.toThrow('invalid arguments');
});

test('certificates are generated under the config directory', async () => {
  const core = await open();
  const info = (await core.invoke('generate_cert', { server: 'my-server' })) as { path: string; sha256: string };
  expect(info.path).toBe(join(dir, 'certs', 'my-server.pem'));
  expect(existsSync(info.path)).toBe(true);
  expect(((await core.invoke('cert_info', { path: info.path })) as { sha256: string }).sha256).toBe(info.sha256);
});

test('uploads need a known server', async () => {
  const core = await open();
  await expect(core.invoke('upload_bytes', { server: 'nope', url: 'https://x', name: 'a.txt', contentType: '', data: 'aGk=' })).rejects.toThrow('unknown server');
});
