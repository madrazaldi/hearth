import { afterEach, expect, test } from 'bun:test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { checkOpenable, commandHandlers, IRC_COMMANDS, PLATFORM_COMMANDS, type Platform } from './commands';
import { Core } from './core/app';

const dirs: string[] = [];
const cores: Core[] = [];
afterEach(() => {
  for (const c of cores.splice(0)) c.shutdown();
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

function fakePlatform(log: unknown[][] = []): Platform {
  return {
    openExternal: (url) => (log.push(['open', url]), true),
    notify: (t, b) => void log.push(['notify', t, b]),
    setBadge: (n) => void log.push(['badge', n]),
    version: async () => '9.9.9',
    checkForUpdate: async () => ({ version: '10.0.0' }),
    downloadUpdate: async (progress) => {
      progress({ event: 'Started', contentLength: 10 });
      progress({ event: 'Progress', chunkLength: 10 });
      progress({ event: 'Finished' });
    },
    applyUpdate: async () => void log.push(['apply']),
  };
}

async function setup(log: unknown[][] = [], pushed: unknown[] = []) {
  const dir = mkdtempSync(join(tmpdir(), 'hearth-cmd-'));
  dirs.push(dir);
  const core = await Core.open(dir, dir, () => {});
  cores.push(core);
  return commandHandlers(core, fakePlatform(log), (e, p) => void pushed.push([e, p]));
}

test('every command the UI calls has a handler', async () => {
  const h = await setup();
  for (const name of [...IRC_COMMANDS, ...PLATFORM_COMMANDS]) expect(typeof h[name]).toBe('function');
});

test('IRC commands reach the core', async () => {
  const h = await setup();
  expect(await h.get_config()).toEqual({ servers: [], settings: {} });
  await expect(h.connect({ server: 'nope' })).rejects.toThrow('unknown server');
});

test('only web and mail links are opened', async () => {
  const log: unknown[][] = [];
  const h = await setup(log);
  await h.open_url({ url: 'https://example.com/a b' });
  await h.open_url({ url: 'mailto:someone@example.com' });
  expect(log).toEqual([
    ['open', 'https://example.com/a%20b'],
    ['open', 'mailto:someone@example.com'],
  ]);
  for (const bad of ['file:///etc/passwd', 'javascript:alert(1)', 'x-apple.systempreferences:', 'smb://host/share', 'not a url']) {
    await expect(h.open_url({ url: bad })).rejects.toThrow();
  }
  expect(log).toHaveLength(2);
  expect(() => checkOpenable('ftp://example.com')).toThrow();
});

test('notifications, badge and version pass through', async () => {
  const log: unknown[][] = [];
  const h = await setup(log);
  await h.notify({ title: 'hi', body: 'there' });
  await h.set_badge({ count: 3 });
  await h.set_badge({ count: -2 });
  expect(log).toEqual([['notify', 'hi', 'there'], ['badge', 3], ['badge', 0]]);
  expect(await h.app_version()).toBe('9.9.9');
  await expect(h.notify({ title: 1 })).rejects.toThrow('invalid arguments');
});

test('update progress is pushed to the UI', async () => {
  const pushed: unknown[] = [];
  const h = await setup([], pushed);
  expect(await h.updater_check()).toEqual({ version: '10.0.0' });
  await h.updater_install();
  expect(pushed).toEqual([
    ['updater-progress', { event: 'Started', contentLength: 10 }],
    ['updater-progress', { event: 'Progress', chunkLength: 10 }],
    ['updater-progress', { event: 'Finished' }],
  ]);
  await h.relaunch();
});
