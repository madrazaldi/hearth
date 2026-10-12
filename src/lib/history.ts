// Backfilling buffers: CHATHISTORY when the server has it, else the local log.

import { api } from './api';
import { isoTime, msgTime } from './irc';
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
  if (buf.history === 'loading') return buf.pending ?? undefined;
  if (mode === 'before' && buf.history === 'done') return;
  buf.history = 'loading';
  buf.pending = load(app, buf, mode, remote).finally(() => (buf.pending = null));
  return buf.pending;
}

async function load(app: App, buf: Buffer, mode: 'latest' | 'before' | 'after', remote: boolean) {
  const conn = buf.conn;
  const max = parseInt(conn.isupport.CHATHISTORY || '100') || 100;
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

/**
 * A stretch of a buffer's history that hasn't been loaded: lines newer than
 * `after` and older than `before` may be missing. Both are times of loaded
 * lines. Jumping to an old message leaves one between it and the present.
 */
export interface Gap {
  after: number;
  before: number;
}

/**
 * The gaps once a contiguous window of history, from `w0` to `w1`, has been
 * merged into `lines`. The window fills whatever gaps it covers; if it touches
 * no loaded line and no gap, it's an island and gets a gap to its neighbour.
 */
export function addWindow(gaps: Gap[], lines: Line[], w0: number, w1: number): Gap[] {
  const out: Gap[] = [];
  let touches = lines.some((l) => l.time >= w0 && l.time <= w1);
  for (const g of gaps) {
    if (g.before <= w0 || g.after >= w1) {
      out.push(g);
      continue;
    }
    touches = true;
    if (g.after < w0) out.push({ after: g.after, before: w0 });
    if (w1 < g.before) out.push({ after: w1, before: g.before });
  }
  if (!touches) {
    const prev = lines.findLast((l) => l.time < w0);
    const next = lines.find((l) => l.time > w1);
    if (prev) out.push({ after: prev.time, before: w0 });
    if (next) out.push({ after: w1, before: next.time });
  }
  return out.sort((a, b) => a.after - b.after);
}

/** Merge a contiguous window of history and update the buffer's gaps. */
function applyWindow(app: App, buf: Buffer, msgs: IrcMessage[], w0: number, w1: number) {
  buf.gaps = addWindow(buf.gaps, buf.messages, w0, w1);
  applyHistory(app, buf, msgs);
}

/** Load history around a message so it can be scrolled to. */
export async function loadAround(app: App, buf: Buffer, time: number, msgid?: string) {
  try {
    let msgs: IrcMessage[] = [];
    const conn = buf.conn;
    if (conn.has('draft/chathistory') && conn.status === 'connected') {
      // soju and others only take timestamps (MSGREFTYPES), and every hit has one.
      const byId = msgid && (conn.isupport.MSGREFTYPES ?? '').split(',').includes('msgid');
      const at = byId ? `msgid=${msgid}` : `timestamp=${isoTime(time)}`;
      msgs = await api.history(conn.id, 'messages', buf.name, `CHATHISTORY AROUND ${buf.name} ${at} 100`);
    } else if (app.settings.storeLogs) {
      const [before, after] = await Promise.all([
        api.localHistory(conn.id, buf.key, { before: time + 1, limit: 60 }),
        api.localHistory(conn.id, buf.key, { after: time, limit: 60 }),
      ]);
      msgs = [...before, ...after];
    }
    if (!msgs.length) return;
    applyWindow(app, buf, msgs, msgTime(msgs[0].tags), msgTime(msgs[msgs.length - 1].tags));
  } catch (e) {
    console.warn('jump', e);
  }
}

/** Whether `fillGap` can do anything right now. */
export function canFill(app: App, buf: Buffer): boolean {
  return (buf.conn.has('draft/chathistory') && buf.conn.status === 'connected') || app.settings.storeLogs;
}

/**
 * Load a page of the lines missing in `gap`, from its older edge (`after`,
 * when scrolling down into it) or its newer one (`before`, scrolling up).
 */
export async function fillGap(app: App, buf: Buffer, gap: Gap, from: 'after' | 'before') {
  if (buf.fillingGap || Date.now() < buf.gapRetryAt || !canFill(app, buf)) return;
  const conn = buf.conn;
  buf.fillingGap = true;
  try {
    let msgs: IrcMessage[];
    let done: boolean;
    if (conn.has('draft/chathistory') && conn.status === 'connected') {
      const limit = Math.min(200, parseInt(conn.isupport.CHATHISTORY || '100') || 100);
      const [a, b] = from === 'after' ? [gap.after, gap.before] : [gap.before, gap.after];
      msgs = await api.history(conn.id, 'messages', buf.name, `CHATHISTORY BETWEEN ${buf.name} timestamp=${isoTime(a)} timestamp=${isoTime(b)} ${limit}`);
      done = msgs.length < limit;
    } else {
      const limit = 200;
      const raw = await api.localHistory(conn.id, buf.key, from === 'after' ? { after: gap.after, limit } : { before: gap.before, limit });
      msgs = raw.filter((m) => {
        const t = msgTime(m.tags);
        return t > gap.after && t < gap.before;
      });
      done = raw.length < limit || msgs.length < raw.length;
    }
    let w0 = gap.after;
    let w1 = gap.before;
    // A page that doesn't move past the edge (all on one timestamp) would
    // be fetched again forever; treat it as the whole gap.
    if (!done && msgs.length) {
      if (from === 'after') w1 = Math.max(msgTime(msgs[msgs.length - 1].tags), gap.after + 1);
      else w0 = Math.min(msgTime(msgs[0].tags), gap.before - 1);
    }
    applyWindow(app, buf, msgs, w0, w1);
  } catch (e) {
    buf.gapRetryAt = Date.now() + 5000;
    console.warn('gap', buf.name, e);
  } finally {
    buf.fillingGap = false;
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
