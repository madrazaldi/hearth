// Every command the UI can invoke over the bridge: the IRC API from
// core/app.ts plus the desktop integration the shell provides. Kept free of
// Electrobun imports so it can be tested under plain `bun test`.

import type { UpdaterProgress } from '../shared/rpc';
import type { Core } from './core/app';

export interface UpdateOffer {
  version: string;
  notes?: string;
}

/** What the shell provides to the UI besides IRC. */
export interface Platform {
  openExternal(url: string): boolean;
  notify(title: string, body: string): void;
  setBadge(count: number): void;
  version(): Promise<string>;
  checkForUpdate(): Promise<UpdateOffer | null>;
  /** Download and stage the update, reporting progress as it goes. */
  downloadUpdate(progress: (e: UpdaterProgress) => void): Promise<void>;
  /** Apply the staged update and restart. Resolves only if that did not happen. */
  applyUpdate(): Promise<void>;
}

/** Every IRC command, by name. */
export const IRC_COMMANDS = [
  'get_config',
  'save_server',
  'delete_server',
  'save_settings',
  'connect',
  'disconnect',
  'reconnect',
  'reset',
  'send',
  'history',
  'local_history',
  'search',
  'load_buffers',
  'save_buffers',
  'clear_logs',
  'link_preview',
  'avatar',
  'generate_cert',
  'cert_info',
  'upload_file',
  'upload_bytes',
] as const;

export const PLATFORM_COMMANDS = ['open_url', 'notify', 'set_badge', 'app_version', 'updater_check', 'updater_install', 'relaunch'] as const;

export type Handlers = Record<string, (params?: Record<string, unknown>) => Promise<unknown>>;

/** Schemes a chat message may ask the system to open. */
const OPENABLE = new Set(['http:', 'https:', 'mailto:', 'tel:']);

export function checkOpenable(raw: string): URL {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new Error('not a valid URL');
  }
  if (!OPENABLE.has(url.protocol)) throw new Error(`refusing to open a ${url.protocol} link`);
  return url;
}

export function commandHandlers(core: Core, platform: Platform, push: (event: 'updater-progress', payload: UpdaterProgress) => void): Handlers {
  const handlers: Handlers = {};
  for (const name of IRC_COMMANDS) handlers[name] = (params = {}) => core.invoke(name, params);

  const text = (p: Record<string, unknown> | undefined, k: string) => {
    const v = p?.[k];
    if (typeof v !== 'string') throw new Error(`invalid arguments: ${k} must be a string`);
    return v;
  };

  handlers.open_url = async (p) => {
    const url = checkOpenable(text(p, 'url'));
    if (!platform.openExternal(url.href)) throw new Error('could not open the link');
  };
  handlers.notify = async (p) => platform.notify(text(p, 'title'), text(p, 'body'));
  handlers.set_badge = async (p) => platform.setBadge(Math.max(0, Math.trunc(Number(p?.count) || 0)));
  handlers.app_version = () => platform.version();
  handlers.updater_check = () => platform.checkForUpdate();
  handlers.updater_install = () => platform.downloadUpdate((e) => push('updater-progress', e));
  handlers.relaunch = async () => platform.applyUpdate();
  return handlers;
}
