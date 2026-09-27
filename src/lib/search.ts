// Message search across the local FTS log and soju.im/search bouncers.

import { api } from './api';
import { escapeAttr } from './irc';
import type { App, Buffer, Conn } from './state.svelte';
import type { SearchHit } from './types';

/** Drop hits found by both the log and the bouncer, then sort newest first. */
export function dedupeHits(all: SearchHit[]): SearchHit[] {
  const seen = new Set<string>();
  const out: SearchHit[] = [];
  for (const h of all) {
    const k = h.msg.tags.msgid ? `${h.network}|${h.msg.tags.msgid}` : `${h.network}|${h.target}|${h.msg.tags.time}|${h.msg.params[1]}`;
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(h);
  }
  const t = (h: SearchHit) => Date.parse(h.msg.tags.time ?? '') || 0;
  return out.sort((a, b) => t(b) - t(a));
}

function remoteSearch(c: Conn, q: string, target: Buffer | undefined): Promise<SearchHit[]> {
  let attrs = `text=${escapeAttr(q)};limit=100`;
  if (target) attrs += `;in=${escapeAttr(target.name)}`;
  return api
    .history(c.id, 'search', '*', `SEARCH ${attrs}`)
    .then((msgs) =>
      msgs.map((msg) => {
        const to = msg.params[0] ?? '';
        const peer = c.isChannel(to) || !msg.source || c.isMe(msg.source.nick) ? to : msg.source.nick;
        return { network: c.id, target: c.fold(peer), msg };
      }),
    )
    .catch(() => []);
}

export async function search(app: App, query: string, scope: { conn?: Conn; buf?: Buffer }): Promise<SearchHit[]> {
  const q = query.trim();
  if (!q) return [];
  const target = scope.buf && scope.buf.kind !== 'server' ? scope.buf : undefined;
  const tasks: Promise<SearchHit[]>[] = [];
  if (app.settings.storeLogs) {
    tasks.push(api.search(q, scope.conn?.id ?? null, target?.key ?? null, 200).catch(() => []));
  }
  const remote = scope.conn ? [scope.conn] : [...app.conns.values()];
  for (const c of remote) {
    if (!c.has('soju.im/search') || c.status !== 'connected' || c.isControl) continue;
    tasks.push(remoteSearch(c, q, target));
  }
  return dedupeHits((await Promise.all(tasks)).flat());
}
