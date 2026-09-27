// Unread and highlight counts, and the read marker (draft/read-marker).

import { isoTime } from './irc';
import type { Buffer } from './state.svelte';
import type { Line, LineType } from './types';

const COUNTED: LineType[] = ['privmsg', 'action', 'notice'];

/** Messages from others after `readMarker`. */
export function countUnread(messages: Line[], readMarker: number): number {
  let n = 0;
  for (let i = messages.length - 1; i >= 0; i--) {
    const l = messages[i];
    if (l.time <= readMarker) break;
    if (!l.self && COUNTED.includes(l.type)) n++;
  }
  return n;
}

/** Highlights after `readMarker`; with `everything`, every message counts. */
export function countHighlights(messages: Line[], readMarker: number, everything: boolean): number {
  let n = 0;
  for (let i = messages.length - 1; i >= 0; i--) {
    const l = messages[i];
    if (l.time <= readMarker) break;
    if (!l.self && (l.highlight || (everything && COUNTED.includes(l.type)))) n++;
  }
  return n;
}

export function markRead(buf: Buffer) {
  const last = buf.messages[buf.messages.length - 1];
  if (!last || last.time <= buf.readMarker) return;
  buf.readMarker = last.time;
  if (buf.kind === 'server' || !buf.conn.has('draft/read-marker')) return;
  // Debounced so scrolling through a burst sends one MARKREAD.
  if (buf.markTimer) clearTimeout(buf.markTimer);
  buf.markTimer = setTimeout(() => {
    buf.markTimer = null;
    buf.conn.send(`MARKREAD ${buf.name} timestamp=${isoTime(buf.readMarker)}`);
  }, 400);
}
