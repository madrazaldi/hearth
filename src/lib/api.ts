import { invoke, listen, type Unlisten } from './ipc';
import type { CertInfo, Config, IrcEvent, IrcMessage, Preview, SavedBuffer, SearchHit, SearchQuery, ServerConfig, Settings } from './types';

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
  search: (query: SearchQuery) => invoke<SearchHit[]>('search', { query }),
  loadBuffers: (conn: string) => invoke<SavedBuffer[]>('load_buffers', { conn }),
  saveBuffers: (conn: string, buffers: SavedBuffer[]) => invoke<void>('save_buffers', { conn, buffers }),
  clearLogs: () => invoke<void>('clear_logs'),
  linkPreview: (url: string) => invoke<Preview>('link_preview', { url }),
  /** A metadata avatar URL, fetched by the backend, as a data: URL. */
  avatar: (url: string) => invoke<string>('avatar', { url }),
  generateCert: (server: string) => invoke<CertInfo>('generate_cert', { server }),
  certInfo: (path: string) => invoke<CertInfo>('cert_info', { path }),
  uploadFile: (server: string, url: string, path: string) => invoke<string>('upload_file', { server, url, path }),
  /** File contents travel base64-encoded, which is far smaller on the bridge than a list of numbers. */
  uploadBytes: (server: string, url: string, name: string, contentType: string, data: Uint8Array) =>
    invoke<string>('upload_bytes', { server, url, name, contentType, data: toBase64(data) }),
};

function toBase64(data: Uint8Array): string {
  let out = '';
  for (let i = 0; i < data.length; i += 0x8000) out += String.fromCharCode(...data.subarray(i, i + 0x8000));
  return btoa(out);
}

export function onIrc(cb: (events: IrcEvent[]) => void): Promise<Unlisten> {
  return listen('irc', (events) => cb(events as IrcEvent[]));
}
