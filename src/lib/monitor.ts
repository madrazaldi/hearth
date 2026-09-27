// MONITOR presence: watched nicks (saved per network) plus open queries.

import type { App, Conn } from './state.svelte';

/** MONITOR + commands for `nicks`, keeping each line's target list under 400 bytes. */
export function monitorCommands(nicks: string[], limit = Infinity): string[] {
  const out: string[] = [];
  let chunk: string[] = [];
  const flush = () => {
    if (chunk.length) out.push(`MONITOR + ${chunk.join(',')}`);
    chunk = [];
  };
  for (const n of nicks.slice(0, limit)) {
    if (chunk.join(',').length + n.length > 400) flush();
    chunk.push(n);
  }
  flush();
  return out;
}

export function isWatched(conn: Conn, nick: string): boolean {
  return conn.watched.some((n) => conn.fold(n) === conn.fold(nick));
}

export function watch(app: App, conn: Conn, nick: string) {
  if (isWatched(conn, nick)) return;
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
  const limit = parseInt(conn.isupport.MONITOR) || Infinity;
  const nicks = new Map<string, string>();
  for (const n of conn.watched) nicks.set(conn.fold(n), n);
  for (const b of conn.buffers.values()) if (b.kind === 'query') nicks.set(conn.fold(b.name), b.name);
  conn.online.clear();
  for (const line of monitorCommands([...nicks.values()], limit)) conn.send(line);
}
