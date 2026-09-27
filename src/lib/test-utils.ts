// Test harness: an App wired to a mocked backend, fed scripted IRC events.

import { vi } from 'vitest';
import type { IrcEvent, IrcMessage } from './types';

export const sent: string[] = [];

export const mockApi = {
  getConfig: vi.fn(async () => ({ servers: [], settings: null })),
  saveServer: vi.fn(),
  deleteServer: vi.fn(),
  saveSettings: vi.fn(async () => {}),
  connect: vi.fn(async () => {}),
  disconnect: vi.fn(async () => {}),
  reconnect: vi.fn(async () => {}),
  reset: vi.fn(async () => {}),
  send: vi.fn(async (_conn: string, lines: string[]) => {
    sent.push(...lines);
  }),
  history: vi.fn(async (): Promise<IrcMessage[]> => []),
  localHistory: vi.fn(async (): Promise<IrcMessage[]> => []),
  search: vi.fn(async () => []),
  loadBuffers: vi.fn(async (): Promise<{ name: string; kind: 'channel' | 'query'; key: string }[]> => []),
  saveBuffers: vi.fn(async () => {}),
  clearLogs: vi.fn(),
  linkPreview: vi.fn(),
  avatar: vi.fn(async (url: string) => `data:image/png;base64,${btoa(url)}`),
  generateCert: vi.fn(),
  certInfo: vi.fn(),
  uploadFile: vi.fn(),
  uploadBytes: vi.fn(),
};

/** Parse a raw IRC line into the shape the backend emits. */
export function msg(line: string): IrcMessage {
  let rest = line;
  const tags: Record<string, string> = {};
  if (rest.startsWith('@')) {
    const i = rest.indexOf(' ');
    for (const t of rest.slice(1, i).split(';')) {
      const [k, v = ''] = t.split('=');
      tags[k] = v.replace(/\\s/g, ' ').replace(/\\:/g, ';');
    }
    rest = rest.slice(i + 1);
  }
  let source: IrcMessage['source'] = null;
  if (rest.startsWith(':')) {
    const i = rest.indexOf(' ');
    const [nickUser, host] = rest.slice(1, i).split('@');
    const [nick, user] = nickUser.split('!');
    source = { nick, user, host };
    rest = rest.slice(i + 1);
  }
  const trailingAt = rest.indexOf(' :');
  const head = trailingAt >= 0 ? rest.slice(0, trailingAt) : rest;
  const params = head.split(' ').filter(Boolean);
  const command = params.shift()!.toUpperCase();
  if (trailingAt >= 0) params.push(rest.slice(trailingAt + 2));
  return { tags, source, command, params };
}

export function connect(
  app: { handleEvents: (e: IrcEvent[]) => void },
  opts: { id?: string; caps?: string[]; capValues?: Record<string, string>; isupport?: Record<string, string>; nick?: string } = {},
) {
  const conn = opts.id ?? 'c';
  app.handleEvents([
    { type: 'conn-added', conn, server: conn, parent: null, netid: null },
    { type: 'registered', conn, nick: opts.nick ?? 'me', caps: opts.caps ?? [], capValues: opts.capValues },
    { type: 'isupport', conn, isupport: { CHANTYPES: '#', PREFIX: '(ov)@+', CASEMAPPING: 'ascii', ...(opts.isupport ?? {}) } },
    { type: 'status', conn, status: 'connected', error: null, retryIn: null },
  ]);
}

export function feed(app: { handleEvents: (e: IrcEvent[]) => void }, ...lines: string[]) {
  app.handleEvents(lines.map((l) => ({ type: 'message' as const, conn: 'c', msg: msg(l) })));
}
