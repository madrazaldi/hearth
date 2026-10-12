// Message search across the local FTS log and soju.im/search bouncers.

import { api } from './api';
import { casefold, escapeAttr, isoTime } from './irc';
import type { App, Buffer, Conn } from './state.svelte';
import type { SearchHit } from './types';

export interface SearchFilters {
  text: string;
  from?: string;
  in?: string;
  /** Exclusive bounds, in ms. */
  before?: number;
  after?: number;
}

export interface SearchPage {
  hits: SearchHit[];
  /** Pass as `before` for the next page; null when there's nothing older. */
  next: number | null;
}

/** A local day (YYYY-MM-DD, `today` or `yesterday`) as [start, end) in ms. */
function parseDay(v: string, now: number): [number, number] | null {
  const d = new Date(now);
  let y = d.getFullYear();
  let m = d.getMonth();
  let day = d.getDate();
  const ymd = /^(\d{4})-(\d{1,2})-(\d{1,2})$/.exec(v);
  if (ymd) [y, m, day] = [+ymd[1], +ymd[2] - 1, +ymd[3]];
  else if (v.toLowerCase() === 'yesterday') day--;
  else if (v.toLowerCase() !== 'today') return null;
  return [new Date(y, m, day).getTime(), new Date(y, m, day + 1).getTime()];
}

/** Split `from:`, `in:`, `before:`, `after:` and `on:` filters out of a query. */
export function parseQuery(q: string, now = Date.now()): SearchFilters {
  const out: SearchFilters = { text: '' };
  const words: string[] = [];
  for (const w of q.trim().split(/\s+/)) {
    const m = /^(from|in|before|after|on|during):(.+)$/i.exec(w);
    const key = m?.[1].toLowerCase();
    const day = m && key !== 'from' && key !== 'in' ? parseDay(m[2], now) : null;
    if (key === 'from') out.from = m![2].replace(/^@/, '');
    else if (key === 'in') out.in = m![2];
    else if (day && key === 'before') out.before = day[0];
    else if (day && key === 'after') out.after = day[1] - 1;
    else if (day) [out.after, out.before] = [day[0] - 1, day[1]];
    else if (w) words.push(w);
  }
  out.text = words.join(' ');
  return out;
}

const filtered = (f: SearchFilters) => !!(f.text || f.from || f.in || f.before !== undefined || f.after !== undefined);

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
  return out.sort((a, b) => hitTime(b) - hitTime(a));
}

export const hitTime = (h: SearchHit) => Date.parse(h.msg.tags.time ?? '') || 0;

/** One source's newest-first page; `full` when it may have older hits. */
type Result = { hits: SearchHit[]; full: boolean };

/**
 * Merge pages from several sources. A source that filled its page may have
 * more hits older than its oldest, so the merged page stops there, and the
 * next one starts from it.
 */
export function mergePages(results: Result[]): SearchPage {
  let cutoff = -Infinity;
  for (const r of results) if (r.full && r.hits.length) cutoff = Math.max(cutoff, Math.min(...r.hits.map(hitTime)));
  const hits = dedupeHits(results.flatMap((r) => r.hits));
  if (cutoff === -Infinity) return { hits, next: null };
  return { hits: hits.filter((h) => hitTime(h) >= cutoff), next: cutoff };
}

const REMOTE_LIMIT = 100;
const PER_TARGET_LIMIT = 50;

async function remoteSearch(c: Conn, f: SearchFilters, target: string | undefined, before: number | undefined): Promise<Result> {
  const limit = target ? PER_TARGET_LIMIT : REMOTE_LIMIT;
  let attrs = `limit=${limit}`;
  if (f.text) attrs += `;text=${escapeAttr(f.text)}`;
  if (f.from) attrs += `;from=${escapeAttr(f.from)}`;
  if (target) attrs += `;in=${escapeAttr(target)}`;
  if (before !== undefined) attrs += `;before=${isoTime(before)}`;
  if (f.after !== undefined) attrs += `;after=${isoTime(f.after)}`;
  const msgs = await api.history(c.id, 'search', '*', `SEARCH ${attrs}`);
  const hits = msgs.map((msg) => {
    const to = msg.params[0] ?? '';
    const peer = target ?? (c.isChannel(to) || !msg.source || c.isMe(msg.source.nick) ? to : msg.source.nick);
    return { network: c.id, target: c.fold(peer), msg };
  });
  return { hits, full: msgs.length >= limit };
}

/**
 * Search one bouncer network. Some (soju, for one) require `in`, so when a
 * search everywhere fails, ask each open buffer instead, from then on.
 */
async function searchConn(c: Conn, f: SearchFilters, target: string | undefined, before: number | undefined): Promise<Result[]> {
  const none = { hits: [], full: false };
  if (target) return [await remoteSearch(c, f, target, before).catch(() => none)];
  if (!c.searchPerTarget) {
    try {
      return [await remoteSearch(c, f, undefined, before)];
    } catch {
      c.searchPerTarget = true;
    }
  }
  const targets = [...c.buffers.values()].map((b) => b.name);
  return Promise.all(targets.map((t) => remoteSearch(c, f, t, before).catch(() => none)));
}

/**
 * Search the local log and, where supported, the bouncer: one page, newest
 * first, of hits older than `before` (from a previous page's `next`).
 */
export async function search(app: App, query: string, scope: { conn?: Conn; buf?: Buffer }, before?: number): Promise<SearchPage> {
  const f = parseQuery(query);
  if (!filtered(f)) return { hits: [], next: null };
  const upper = before === undefined ? f.before : Math.min(before, f.before ?? Infinity);
  const buf = scope.buf && scope.buf.kind !== 'server' ? scope.buf : undefined;
  const target = f.in ?? buf?.name;
  const tasks: Promise<Result[]>[] = [];
  if (app.settings.storeLogs) {
    const limit = 200;
    const key = target === undefined ? null : scope.conn ? scope.conn.fold(target) : casefold(target);
    const q = { text: f.text, network: scope.conn?.id ?? null, target: key, from: f.from ?? null, before: upper ?? null, after: f.after ?? null, limit };
    tasks.push(
      api
        .search(q)
        .then((hits) => [{ hits, full: hits.length >= limit }])
        .catch(() => []),
    );
  }
  const remote = scope.conn ? [scope.conn] : [...app.conns.values()];
  for (const c of remote) {
    if (!c.has('soju.im/search') || c.status !== 'connected' || c.isControl) continue;
    tasks.push(searchConn(c, f, target, upper));
  }
  return mergePages((await Promise.all(tasks)).flat());
}
