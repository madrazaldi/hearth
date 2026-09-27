// Keyboard and screen reader helpers: focus management for dialogs and
// menus, roving-focus index math, and the accessible names of rows.

import { stripFormatting } from './format';
import type { Line } from './types';

const FOCUSABLE =
  'a[href], button:not(:disabled), input:not(:disabled):not([type="hidden"]), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

/** Tabbable elements inside `root`, in DOM order. */
export function focusables(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((e) => e.tabIndex >= 0 && !e.hidden && e.getClientRects().length > 0);
}

/**
 * Where Tab should go inside a trap of `count` elements, or null to let the
 * browser move normally. `at` is the focused element's index, -1 if none of them.
 */
export function wrapTab(count: number, at: number, back: boolean): number | null {
  if (!count) return null;
  if (at < 0) return back ? count - 1 : 0;
  if (back && at === 0) return count - 1;
  if (!back && at === count - 1) return 0;
  return null;
}

/**
 * The index a list-navigation key moves to from `i` in a list of `count`, or
 * null for keys that don't navigate. `page` is how many rows PageUp/Down skip.
 */
export function stepIndex(key: string, i: number, count: number, page = 10): number | null {
  if (!count) return null;
  const clamp = (n: number) => Math.max(0, Math.min(count - 1, n));
  switch (key) {
    case 'ArrowDown':
      return clamp(i + 1);
    case 'ArrowUp':
      return clamp(i < 0 ? count - 1 : i - 1);
    case 'Home':
      return 0;
    case 'End':
      return count - 1;
    case 'PageDown':
      return clamp(i + page);
    case 'PageUp':
      return clamp(i - page);
  }
  return null;
}

/** Restores focus to whatever had it before, unless focus has since moved on purpose. */
function restorer(node: HTMLElement) {
  const prev = document.activeElement as HTMLElement | null;
  return () => {
    const now = document.activeElement;
    if (now && now !== document.body && !node.contains(now)) return;
    if (prev?.isConnected && prev !== document.body) prev.focus({ preventScroll: true });
    // The opener is gone (say, a menu item that opened this): the composer is home.
    else window.dispatchEvent(new CustomEvent('hearth:focus-composer'));
  };
}

/**
 * Svelte action for modal dialogs: focuses the first `[data-autofocus]` or
 * tabbable element, keeps Tab inside, closes on Escape, and puts focus back
 * where it was when the dialog goes away.
 */
export function modal(node: HTMLElement, opts: { onclose: () => void }) {
  let o = opts;
  const restore = restorer(node);
  const first = node.querySelector<HTMLElement>('[data-autofocus]') ?? focusables(node)[0] ?? node;
  if (!node.contains(document.activeElement)) first.focus({ preventScroll: true });

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      // Leave Escape to an open menu or suggestion list inside the dialog.
      if (e.defaultPrevented) return;
      e.preventDefault();
      e.stopPropagation();
      o.onclose();
    } else if (e.key === 'Tab') {
      const els = focusables(node);
      const to = wrapTab(els.length, els.indexOf(document.activeElement as HTMLElement), e.shiftKey);
      if (to === null) return;
      e.preventDefault();
      els[to].focus();
    }
  }

  node.addEventListener('keydown', onKey);
  return {
    update: (next: { onclose: () => void }) => (o = next),
    destroy() {
      node.removeEventListener('keydown', onKey);
      restore();
    },
  };
}

/**
 * Svelte action for popup menus (`role="menu"`): arrow keys move between
 * items, Escape and Tab close it, and focus returns to the opener.
 */
export function menu(node: HTMLElement, opts: { onclose: () => void; horizontal?: boolean }) {
  let o = opts;
  const restore = restorer(node);
  const items = () => [...node.querySelectorAll<HTMLElement>('[role^="menuitem"]:not(:disabled)')];
  if (!node.contains(document.activeElement)) (items()[0] ?? node).focus({ preventScroll: true });

  function onKey(e: KeyboardEvent) {
    const els = items();
    const at = els.indexOf(document.activeElement as HTMLElement);
    const key = o.horizontal ? ({ ArrowLeft: 'ArrowUp', ArrowRight: 'ArrowDown' } as Record<string, string>)[e.key] ?? e.key : e.key;
    if (key === 'Escape' || key === 'Tab') {
      if (key === 'Escape') e.preventDefault();
      e.stopPropagation();
      o.onclose();
      return;
    }
    let to = stepIndex(key, at, els.length, els.length);
    if (to === null) return;
    // Menus wrap around.
    if (key === 'ArrowDown' && at === els.length - 1) to = 0;
    else if (key === 'ArrowUp' && at === 0) to = els.length - 1;
    e.preventDefault();
    e.stopPropagation();
    els[to]?.focus();
  }

  node.addEventListener('keydown', onKey);
  return {
    update: (next: typeof opts) => (o = next),
    destroy() {
      node.removeEventListener('keydown', onKey);
      restore();
    },
  };
}

const plural = (n: number, one: string, many = one + 's') => `${n} ${n === 1 ? one : many}`;

/** Accessible name of a sidebar entry: its name and what's waiting in it. */
export function bufferLabel(b: {
  name: string;
  kind: 'server' | 'channel' | 'query';
  unread: number;
  highlights: number;
  muted: boolean;
  joined?: boolean;
  online?: boolean;
  typing?: boolean;
}): string {
  const parts = [b.name];
  if (b.kind === 'query' && b.online !== undefined) parts.push(b.online ? 'online' : 'offline');
  if (b.kind === 'channel' && b.joined === false) parts.push('not joined');
  if (b.highlights) parts.push(plural(b.highlights, b.kind === 'query' ? 'new message' : 'mention'));
  if (b.unread && !(b.kind === 'query' && b.highlights)) parts.push(`${b.unread} unread`);
  if (b.muted) parts.push('muted');
  if (b.typing) parts.push('typing');
  return parts.join(', ');
}

/** Singular names for channel prefix modes, keyed by mode letter. */
const RANKS: Record<string, string> = { q: 'owner', a: 'admin', o: 'operator', h: 'half-operator', v: 'voiced' };

/** Accessible name of a member list row: nick, rank and away status. */
export function memberLabel(m: { nick: string; prefix: string; away: boolean }, prefix: { modes: string; symbols: string }, displayName?: string): string {
  const parts = [m.nick];
  if (displayName && displayName !== m.nick) parts.push(displayName);
  const sym = m.prefix[0];
  if (sym) {
    const mode = prefix.modes[prefix.symbols.indexOf(sym)];
    parts.push(RANKS[mode] ?? `mode ${mode}`);
  }
  if (m.away) parts.push('away');
  return parts.join(', ');
}

/** Accessible name of a message row: who, when, and what they said. */
export function lineLabel(l: Line, clock: string, replyNick?: string): string {
  const text = l.redacted ? 'message deleted' : stripFormatting(l.text);
  let who = l.nick;
  if (l.type === 'notice') who += ' (notice)';
  let s = l.type === 'action' ? `${clock}, * ${l.nick} ${text}` : `${who}, ${clock}: ${text}`;
  if (replyNick) s += `, replying to ${replyNick}`;
  const r = l.reactions && !l.redacted ? Object.entries(l.reactions) : [];
  if (r.length) s += `. Reactions: ${r.map(([e, n]) => `${e} ${n.length}`).join(', ')}`;
  return s;
}

const TALK = new Set(['privmsg', 'notice', 'action']);

/**
 * What to announce for lines appended after `prevLast`: live messages from
 * others, never history. Returns short phrases, or a count for a burst.
 */
export function announcements(lines: Line[], prevLast: Line | undefined, now: number, max = 3): string[] {
  let from = lines.length;
  if (prevLast) {
    const i = lines.lastIndexOf(prevLast);
    // Only look at what's after the old last line; anything else is a history
    // load or a trim, not something that just happened.
    if (i >= 0) from = i + 1;
    else for (from = lines.length; from > 0 && lines[from - 1].time > prevLast.time; ) from--;
  }
  // A minute's slack for server clocks; older lines are catch-up history.
  const fresh = lines.slice(from).filter((l) => TALK.has(l.type) && !l.self && now - l.time < 60_000);
  if (fresh.length > max) return [`${fresh.length} new messages`];
  return fresh.map((l) => {
    const text = stripFormatting(l.text).slice(0, 300);
    return l.type === 'action' ? `* ${l.nick} ${text}` : `${l.nick}: ${text}`;
  });
}

const EMOJI_NAMES: Record<string, string> = {
  '👍': 'thumbs up',
  '❤️': 'heart',
  '😂': 'laughing',
  '🎉': 'party',
  '😮': 'surprised',
  '😢': 'sad',
  '👀': 'eyes',
  '✅': 'check mark',
};

export function emojiName(e: string): string {
  return EMOJI_NAMES[e] ?? e;
}
