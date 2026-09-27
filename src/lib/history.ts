// Backfilling buffers: CHATHISTORY when the server has it, else the local log.

import { api } from './api';
import { isoTime } from './irc';
import { historyLine } from './incoming';
import type { App, Buffer, Conn } from './state.svelte';
import type { IrcMessage, Line } from './types';

function lineKey(l: Line): string {
  return l.msgid ?? `${l.time}|${l.type}|${l.nick}|${l.text}`;
}

/**
 * `existing` plus the lines of `incoming` it doesn't already have, sorted by
 * time. Returns null when nothing is new.
 */
export function mergeLines(existing: Line[], incoming: Line[]): Line[] | null {
  if (!incoming.length) return null;
  const seen = new Set(existing.map(lineKey));
  // Lines without a msgid can differ by a few ms between the live copy and
  // the logged one (our own messages without echo-message), so also match
  // same-content lines that are close in time.
  const loose = new Map<string, number[]>();
  for (const l of existing) {
    if (l.msgid) continue;
    const k = `${l.type}|${l.nick}|${l.text}`;
    const times = loose.get(k);
    if (times) times.push(l.time);
    else loose.set(k, [l.time]);
  }
  const fresh = incoming.filter((l) => {
    if (seen.has(lineKey(l))) return false;
    if (l.msgid) return true;
    const times = loose.get(`${l.type}|${l.nick}|${l.text}`);
    return !times?.some((t) => Math.abs(t - l.time) < 3000);
  });
  if (!fresh.length) return null;
  return [...existing, ...fresh].sort((a, b) => a.time - b.time || a.id - b.id);
}

/** The CHATHISTORY command to page `buf` in `mode`, and the limit it asks for. */
export function chathistoryRequest(buf: Buffer, mode: 'latest' | 'before' | 'after', max: number) {
  const first = buf.messages[0];
  const last = buf.messages[buf.messages.length - 1];
  if (mode === 'after' && last) {
    const limit = Math.min(1000, max);
    return { limit, line: `CHATHISTORY AFTER ${buf.name} timestamp=${isoTime(last.time)} ${limit}` };
  }
  const limit = Math.min(100, max);
  if (mode === 'before' && first) {
    return { limit, line: `CHATHISTORY BEFORE ${buf.name} timestamp=${isoTime(first.time)} ${limit}` };
  }
  return { limit, line: `CHATHISTORY LATEST ${buf.name} * ${limit}` };
}

export async function fetchHistory(app: App, buf: Buffer, mode: 'latest' | 'before' | 'after') {
  const conn = buf.conn;
  if (buf.kind === 'server') return;
  const remote = conn.has('draft/chathistory') && conn.status === 'connected';
  // The local log only fills gaps before what we have: anything newer
  // arrived live in this session.
  if (!remote && (!app.settings.storeLogs || mode === 'after')) return;
  if (buf.history === 'loading' || (mode === 'before' && buf.history === 'done')) return;
  const max = parseInt(conn.isupport.CHATHISTORY || '100') || 100;
  buf.history = 'loading';
  try {
    let msgs: IrcMessage[];
    let limit: number;
    if (remote) {
      const req = chathistoryRequest(buf, mode, max);
      limit = req.limit;
      msgs = await api.history(conn.id, 'messages', buf.name, req.line);
    } else {
      limit = 100;
      msgs = await api.localHistory(conn.id, buf.key, { before: mode === 'before' ? buf.messages[0]?.time : undefined, limit });
    }
    applyHistory(app, buf, msgs);
    buf.history = mode !== 'after' && msgs.length < limit ? 'done' : 'idle';
  } catch (e) {
    buf.history = 'idle';
    console.warn('history', buf.name, e);
  }
}

/** Load history around a message so it can be scrolled to. */
export async function loadAround(app: App, buf: Buffer, time: number, msgid?: string) {
  try {
    let msgs: IrcMessage[] = [];
    if (buf.conn.has('draft/chathistory') && buf.conn.status === 'connected') {
      const at = msgid ? `msgid=${msgid}` : `timestamp=${isoTime(time)}`;
      msgs = await api.history(buf.conn.id, 'messages', buf.name, `CHATHISTORY AROUND ${buf.name} ${at} 100`);
    } else if (app.settings.storeLogs) {
      const [before, after] = await Promise.all([
        api.localHistory(buf.conn.id, buf.key, { before: time + 1, limit: 60 }),
        api.localHistory(buf.conn.id, buf.key, { after: time, limit: 60 }),
      ]);
      msgs = [...before, ...after];
    }
    applyHistory(app, buf, msgs);
  } catch (e) {
    console.warn('jump', e);
  }
}

function applyHistory(app: App, buf: Buffer, msgs: IrcMessage[]) {
  const lines = msgs.map((m) => historyLine(app, buf.conn, m)).filter((l): l is Line => !!l);
  buf.merge(lines);
  for (const m of msgs) {
    const emoji = m.tags['+draft/react'];
    const parent = m.tags['+draft/reply'];
    if (m.command === 'TAGMSG' && emoji && parent && m.source && !app.isIgnored(buf.conn, m.source)) {
      buf.react(parent, emoji, m.source.nick);
    }
  }
}

/** Open query buffers for recent conversations the server remembers. */
export async function fetchTargets(app: App, conn: Conn) {
  if (!conn.has('draft/chathistory') || conn.isControl) return;
  const now = Date.now();
  const line = `CHATHISTORY TARGETS timestamp=${isoTime(now + 60000)} timestamp=${isoTime(now - 14 * 86400000)} 50`;
  try {
    const msgs = await api.history(conn.id, 'targets', '*', line);
    for (const m of msgs) {
      const target = m.params[1];
      if (!target || conn.isChannel(target) || conn.isMe(target)) continue;
      const b = conn.ensure(target, 'query');
      if (!b.messages.length) app.fetchHistory(b, 'latest');
    }
  } catch (e) {
    console.warn('targets', e);
  }
}
