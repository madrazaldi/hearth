import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { Config, IrcEvent, IrcMessage, ServerConfig, Settings } from './types';

export const api = {
  getConfig: () => invoke<Config>('get_config'),
  saveServer: (server: ServerConfig) => invoke<ServerConfig>('save_server', { server }),
  deleteServer: (id: string) => invoke<void>('delete_server', { id }),
  saveSettings: (settings: Settings) => invoke<void>('save_settings', { settings }),
  connect: (server: string) => invoke<void>('connect', { server }),
  disconnect: (server: string, reason?: string) => invoke<void>('disconnect', { server, reason }),
  reconnect: (server: string) => invoke<void>('reconnect', { server }),
  reset: () => invoke<void>('reset'),
  send: (conn: string, lines: string[]) => invoke<void>('send', { conn, lines }),
  history: (conn: string, kind: 'messages' | 'targets', target: string, line: string) =>
    invoke<IrcMessage[]>('history', { conn, kind, target, line }),
  uploadFile: (server: string, url: string, path: string) => invoke<string>('upload_file', { server, url, path }),
  uploadBytes: (server: string, url: string, name: string, contentType: string, data: Uint8Array) =>
    invoke<string>('upload_bytes', { server, url, name, contentType, data: Array.from(data) }),
};

export function onIrc(cb: (events: IrcEvent[]) => void): Promise<UnlistenFn> {
  return listen<IrcEvent[]>('irc', (e) => cb(e.payload));
}
