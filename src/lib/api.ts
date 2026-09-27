import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { CertInfo, Config, IrcEvent, IrcMessage, Preview, SavedBuffer, SearchHit, ServerConfig, Settings } from './types';

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
  history: (conn: string, kind: 'messages' | 'targets' | 'search', target: string, line: string) =>
    invoke<IrcMessage[]>('history', { conn, kind, target, line }),
  /** From the local log; `target` is the case-folded buffer name. */
  localHistory: (conn: string, target: string, opts: { before?: number; after?: number; limit: number }) =>
    invoke<IrcMessage[]>('local_history', { conn, target, before: opts.before ?? null, after: opts.after ?? null, limit: opts.limit }),
  search: (query: string, network: string | null, target: string | null, limit = 200) =>
    invoke<SearchHit[]>('search', { query, network, target, limit }),
  loadBuffers: (conn: string) => invoke<SavedBuffer[]>('load_buffers', { conn }),
  saveBuffers: (conn: string, buffers: SavedBuffer[]) => invoke<void>('save_buffers', { conn, buffers }),
  clearLogs: () => invoke<void>('clear_logs'),
  linkPreview: (url: string) => invoke<Preview>('link_preview', { url }),
  /** A metadata avatar URL, fetched by the backend, as a data: URL. */
  avatar: (url: string) => invoke<string>('avatar', { url }),
  generateCert: (server: string) => invoke<CertInfo>('generate_cert', { server }),
  certInfo: (path: string) => invoke<CertInfo>('cert_info', { path }),
  uploadFile: (server: string, url: string, path: string) => invoke<string>('upload_file', { server, url, path }),
  uploadBytes: (server: string, url: string, name: string, contentType: string, data: Uint8Array) =>
    invoke<string>('upload_bytes', { server, url, name, contentType, data: Array.from(data) }),
};

export function onIrc(cb: (events: IrcEvent[]) => void): Promise<UnlistenFn> {
  return listen<IrcEvent[]>('irc', (e) => cb(e.payload));
}
