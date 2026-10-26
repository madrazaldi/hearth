// The desktop shell: the Bun process Electrobun starts. It owns the window
// (Chromium/CEF), the tray, the application menu and the IRC engine in
// ./core, and answers the UI's commands over Electrobun RPC.

import Electrobun, { ApplicationMenu, BrowserView, BrowserWindow, Tray, Updater, Utils, type RPCSchema } from 'electrobun/main';
import type { PushEvents, UpdaterProgress } from '../shared/rpc';
import { commandHandlers, type Platform } from './commands';
import { Core } from './core/app';
import { appDirs } from './core/paths';

type HearthRPC = {
  bun: RPCSchema<{ requests: Record<string, { params: Record<string, unknown> | undefined; response: unknown }>; messages: Record<string, never> }>;
  webview: RPCSchema<{ requests: Record<string, never>; messages: { push: { event: keyof PushEvents; payload: unknown } } }>;
};

const isMac = process.platform === 'darwin';

// A long-lived client should survive a stray rejection from a socket.
process.on('unhandledRejection', (e) => console.error('unhandled rejection:', e));
process.on('uncaughtException', (e) => console.error('uncaught exception:', e));

let quitting = false;
let updating = false;
let tray: Tray | null = null;

// ---- Window ----

const DEV_SERVER = 'http://localhost:1420';

/** In development, use the Vite dev server (hot reload) when it is running; otherwise the built UI. */
async function mainViewUrl(): Promise<string> {
  if ((await Updater.localInfo.channel()) === 'dev') {
    try {
      await fetch(DEV_SERVER, { method: 'HEAD', signal: AbortSignal.timeout(500) });
      console.log(`Using the Vite dev server at ${DEV_SERVER}`);
      return DEV_SERVER;
    } catch {}
  }
  return 'views://mainview/index.html';
}

let rpc: ReturnType<typeof BrowserView.defineRPC<HearthRPC>>;

function push<K extends keyof PushEvents>(event: K, payload: PushEvents[K]) {
  try {
    rpc?.send('push', { event, payload });
  } catch (e) {
    console.error(`could not push ${event}:`, e);
  }
}

// ---- Desktop integration the UI asks for ----

const platform: Platform = {
  openExternal: (url) => Utils.openExternal(url),
  notify: (title, body) => Utils.showNotification({ title, body }),
  // There is no dock badge API, so the count rides on the tray icon's title where the platform shows one.
  setBadge: (count) => tray?.setTitle(count > 0 ? String(count) : ''),
  version: async () => (await Updater.getLocalInfo()).version,
  async checkForUpdate() {
    const info = await Updater.checkForUpdate();
    if (info.error) throw new Error(info.error);
    return info.updateAvailable ? { version: info.version } : null;
  },
  async downloadUpdate(progress: (e: UpdaterProgress) => void) {
    let started = false;
    let seen = 0;
    Updater.onStatusChange((entry) => {
      const d = entry.details;
      if (entry.status !== 'download-progress' || !d) return;
      if (!started) {
        started = true;
        progress({ event: 'Started', contentLength: d.totalBytes });
      }
      const done = d.bytesDownloaded ?? 0;
      progress({ event: 'Progress', chunkLength: Math.max(0, done - seen) });
      seen = done;
    });
    try {
      await Updater.downloadUpdate();
      const info = Updater.updateInfo();
      if (info.error) throw new Error(info.error);
      if (!info.updateReady) throw new Error('the update could not be prepared');
      progress({ event: 'Finished' });
    } finally {
      Updater.onStatusChange(null);
    }
  },
  async applyUpdate() {
    // Applying quits and relaunches by itself; say goodbye to the servers first.
    updating = true;
    await core.shutdownGracefully();
    await Updater.applyUpdate();
    updating = false;
    const info = Updater.updateInfo();
    if (info.error) throw new Error(info.error);
  },
};

// ---- Boot ----

const dirs = appDirs();
const core = await Core.open(dirs.config, dirs.data, (events) => push('irc', events));

rpc = BrowserView.defineRPC<HearthRPC>({
  // History requests, uploads and the like take as long as the network does.
  maxRequestTime: Infinity,
  handlers: { requests: commandHandlers(core, platform, push) as never, messages: {} },
});

const win = new BrowserWindow({
  title: 'Hearth',
  url: await mainViewUrl(),
  frame: { width: 1240, height: 800 },
  renderer: 'cef',
  // macOS draws the traffic lights over the UI; elsewhere the system title bar stays.
  titleBarStyle: isMac ? 'hiddenInset' : 'default',
  trafficLightOffset: { x: 4, y: 11 },
  rpc,
});

function showMain() {
  win.show();
  win.unminimize();
  win.activate();
}

function quit() {
  quitting = true;
  Utils.quit();
}

// Close to tray: hide instead of closing when the user asked for it.
win.on('will-close', (event) => {
  const e = event as { response?: { allow: boolean } };
  if (!quitting && core.closeToTray()) {
    e.response = { allow: false };
    win.hide();
  }
});

Electrobun.app.on('reopen', showMain);

// Quit politely: let the QUIT lines reach the servers before the process goes.
let farewell = false;
Electrobun.events.on('before-quit', (event) => {
  if (farewell || updating) return;
  farewell = true;
  (event as { response?: { allow: boolean } }).response = { allow: false };
  core
    .shutdownGracefully()
    .catch((e) => console.error('shutdown failed:', e))
    .finally(() => {
      quitting = true;
      Utils.quit();
    });
});

// ---- Menus and tray ----

if (isMac) {
  ApplicationMenu.setApplicationMenu([
    {
      label: 'Hearth',
      submenu: [{ role: 'about' }, { type: 'separator' }, { role: 'hide' }, { role: 'hideOthers' }, { role: 'showAll' }, { type: 'separator' }, { role: 'quit' }],
    },
    {
      label: 'Edit',
      submenu: [{ role: 'undo' }, { role: 'redo' }, { type: 'separator' }, { role: 'cut' }, { role: 'copy' }, { role: 'paste' }, { role: 'selectAll' }],
    },
    {
      label: 'Window',
      submenu: [{ role: 'minimize' }, { role: 'zoom' }, { role: 'close' }, { type: 'separator' }, { role: 'toggleFullScreen' }],
    },
  ]);
}

tray = new Tray({
  title: '',
  image: isMac ? 'views://tray/tray-template.png' : 'views://tray/icon.png',
  template: isMac,
  width: isMac ? 18 : 22,
  height: isMac ? 18 : 22,
});
tray.setMenu([
  { type: 'normal', label: 'Show Hearth', action: 'show' },
  { type: 'divider' },
  { type: 'normal', label: 'Quit Hearth', action: 'quit' },
]);
tray.on('tray-clicked', (event) => {
  const action = (event as { data?: { action?: string } }).data?.action;
  if (action === 'show') showMain();
  else if (action === 'quit') quit();
});

console.log('Hearth started');
