// MONITOR presence: watched nicks (saved per network) plus open queries.

import type { App, Conn } from './state.svelte';

const utf8 = new TextEncoder();

/** MONITOR + commands for `nicks`, keeping each line's target list within 400 bytes. */
export function monitorCommands(nicks: string[], limit = Infinity): string[] {
  const out: string[] = [];
  let chunk: string[] = [];
  let bytes = 0;
  const flush = () => {
    if (chunk.length) out.push(`MONITOR + ${chunk.join(',')}`);
    chunk = [];
    bytes = 0;
  };
  for (const n of nicks.slice(0, limit)) {
    const size = utf8.encode(n).length;
    if (chunk.length && bytes + 1 + size > 400) flush();
    bytes += (chunk.length ? 1 : 0) + size;
    chunk.push(n);
  }
  flush();
  return out;
}

export function isWatched(conn: Conn, nick: string): boolean {
  return conn.watched.some((n) => conn.fold(n) === conn.fold(nick));
}

/** How many nicks the server lets us MONITOR. */
const monitorLimit = (conn: Conn) => parseInt(conn.isupport.MONITOR) || Infinity;

/** Everything we MONITOR: watched nicks and open conversations, by folded nick. */
function monitored(conn: Conn): Map<string, string> {
  const nicks = new Map<string, string>();
  for (const n of conn.watched) nicks.set(conn.fold(n), n);
  for (const b of conn.buffers.values()) if (b.kind === 'query') nicks.set(conn.fold(b.name), b.name);
  return nicks;
}

export function watch(app: App, conn: Conn, nick: string) {
  if (isWatched(conn, nick)) return;
  if (conn.canMonitor) {
    const subs = monitored(conn);
    if (!subs.has(conn.fold(nick)) && subs.size >= monitorLimit(conn)) {
      app.toast(`Can't watch ${nick}: ${conn.name} allows ${monitorLimit(conn)} nicknames`, 'error');
      return;
    }
  }
  app.updateSettings({ watch: { ...app.settings.watch, [conn.prefKey]: [...conn.watched, nick] } });
  if (conn.canMonitor) conn.send(`MONITOR + ${nick}`);
}

export function unwatch(app: App, conn: Conn, nick: string) {
  const rest = conn.watched.filter((n) => conn.fold(n) !== conn.fold(nick));
  app.updateSettings({ watch: { ...app.settings.watch, [conn.prefKey]: rest } });
  if (conn.canMonitor && !conn.buffer(nick)) {
    conn.send(`MONITOR - ${nick}`);
    conn.online.delete(conn.fold(nick));
  }
}

/** Subscribe to presence for watched nicks and open conversations. */
export function syncMonitor(conn: Conn) {
  if (!conn.canMonitor) return;
  conn.online.clear();
  for (const line of monitorCommands([...monitored(conn).values()], monitorLimit(conn))) conn.send(line);
}
