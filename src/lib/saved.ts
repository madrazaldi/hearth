// Saved buffers: which channels and queries to reopen after a restart.

import { api } from './api';
import type { App, Buffer, Conn } from './state.svelte';

export type SavedBuffer = { name: string; kind: 'channel' | 'query'; key: string };

/** Queries, plus channels we mean to be in. */
export function savedList(buffers: Iterable<Buffer>): SavedBuffer[] {
  return [...buffers]
    .filter((b) => b.kind === 'query' || (b.kind === 'channel' && b.wantJoined))
    .map((b) => ({ name: b.name, kind: b.kind as 'channel' | 'query', key: b.joinKey }));
}

export function saveBuffersSoon(conn: Conn, timers: Map<string, ReturnType<typeof setTimeout>>) {
  if (!conn.buffersLoaded || conn.isControl) return;
  clearTimeout(timers.get(conn.id));
  timers.set(
    conn.id,
    setTimeout(() => {
      api.saveBuffers(conn.id, savedList(conn.buffers.values())).catch((e) => console.warn('save buffers', e));
    }, 500),
  );
}

export async function restoreBuffers(app: App, conn: Conn) {
  try {
    const saved = await api.loadBuffers(conn.id);
    for (const s of saved) {
      // Bouncers keep their own channel list; only restore conversations there.
      if (s.kind === 'channel' && conn.netid) continue;
      const b = conn.ensure(s.name, s.kind);
      if (s.kind === 'channel') {
        b.wantJoined = true;
        b.joinKey = s.key;
      } else if (!b.messages.length) app.fetchHistory(b, 'latest');
      app.maybeRestoreActive(conn, b.key);
    }
  } catch (e) {
    console.warn('restore buffers', e);
  }
  conn.buffersLoaded = true;
}

/** JOIN commands for `chans`, ten per line. */
export function rejoinCommands(chans: { name: string; joinKey: string }[]): string[] {
  const out: string[] = [];
  for (let i = 0; i < chans.length; i += 10) {
    const group = chans.slice(i, i + 10);
    // Keyed channels must come first so keys line up by position.
    const keyed = group.filter((b) => b.joinKey);
    const names = [...keyed, ...group.filter((b) => !b.joinKey)].map((b) => b.name).join(',');
    out.push(keyed.length ? `JOIN ${names} ${keyed.map((b) => b.joinKey).join(',')}` : `JOIN ${names}`);
  }
  return out;
}
