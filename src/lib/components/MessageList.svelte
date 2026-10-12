<script lang="ts">
  import { onMount, tick, untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { app, type Buffer } from '../state.svelte';
  import { stripFormatting } from '../format';
  import { announcements, lineLabel } from '../a11y';
  import { formatClock } from '../time';
  import type { Line } from '../types';
  import { buildItems, itemIndexOf, MEMBERSHIP, type Item } from '../timeline';
  import { canFill } from '../history';
  import { Estimates, indexAt, offsets as prefixSums, rangeFor } from '../virtual';
  import Icon from './Icon.svelte';
  import Divider from './Divider.svelte';
  import EventRow from './EventRow.svelte';
  import EventRun from './EventRun.svelte';
  import MessageRow from './MessageRow.svelte';
  import { askDelete, canReply, closeMenus, copyLine, openReact, startReply } from './MessageTools.svelte';
  import MotdBlock from './MotdBlock.svelte';
  import WhoisCard from './WhoisCard.svelte';

  let { buf, filter = '', onnick }: { buf: Buffer; filter?: string; onnick: (nick: string, e: MouseEvent) => void } = $props();

  // Rendered beyond each edge of the viewport, so fast scrolls and the
  // smooth "Latest" scroll mostly land on measured rows.
  const OVERSCAN = 1000;

  let el = $state<HTMLDivElement>();
  let list = $state<HTMLDivElement>();
  let atBottom = $state(true);
  const expanded = new SvelteSet<string>();
  let flash = $state<number | null>(null);

  const s = $derived(app.settings);

  const source = $derived.by(() => {
    let lines = buf.messages;
    if (s.membership === 'hide' && buf.kind === 'channel') lines = lines.filter((l) => !MEMBERSHIP.has(l.type));
    const q = filter.trim().toLowerCase();
    if (q) lines = lines.filter((l) => (l.nick + ' ' + stripFormatting(l.text)).toLowerCase().includes(q));
    return lines;
  });

  const items = $derived(
    buildItems(source, { divider: buf.divider, channel: buf.kind === 'channel', membership: s.membership, gaps: buf.gaps }),
  );
  const index = $derived(new Map(items.map((it, i) => [it.key, i])));

  // ── Virtualization ───────────────────────────────────────────
  // Measured heights by item key; anything unmeasured uses a per-category
  // average. `hv` bumps whenever `heights` changes. The category is kept with
  // the height because a row can move between `msg` and `cont` on a rebuild.
  const heights = new Map<string, { h: number; cat: string }>();
  const est = new Estimates({ day: 41, unread: 19, run: 25, msg: 28, cont: 22, event: 24, block: 160, gap: 44 });
  let hv = $state(0);

  function category(it: Item): string {
    if (it.kind !== 'line') return it.kind;
    const t = it.line.type;
    if (t === 'privmsg' || t === 'notice' || t === 'action') return it.cont ? 'cont' : 'msg';
    return t === 'whois' || it.line.data?.motd !== undefined ? 'block' : 'event';
  }

  const offs = $derived.by(() => {
    void hv;
    return prefixSums(items.length, (i) => heights.get(items[i].key)?.h ?? est.get(category(items[i])));
  });

  // Scroll position and viewport height, in scroller coordinates; `base` is
  // where the item list starts inside the scroller (below the history header).
  // Starts far down so the first render is the newest rows.
  let viewTop = $state(1e12);
  let viewH = $state(0);
  let base = $state(0);

  const range = $derived(rangeFor(offs, viewTop - base - OVERSCAN, viewTop - base + viewH + OVERSCAN));
  // Split out so a scroll that keeps the same rows doesn't re-render.
  const start = $derived(range[0]);
  const end = $derived(range[1]);
  const visible = $derived(items.slice(start, end));

  /** Read the rendered rows' heights; true if anything changed. */
  function measure(): boolean {
    if (!list) return false;
    let changed = false;
    for (const node of list.children as HTMLCollectionOf<HTMLElement>) {
      const key = node.dataset.key;
      if (!key) continue;
      const i = index.get(key);
      if (i === undefined) continue;
      const h = node.getBoundingClientRect().height;
      const cat = category(items[i]);
      const old = heights.get(key);
      const same = old?.cat === cat;
      if (same && Math.abs(old!.h - h) < 0.5) continue;
      heights.set(key, { h, cat });
      est.add(cat, h, same ? old!.h : undefined);
      changed = true;
    }
    base = list.offsetTop;
    // Drop lines that have left the buffer.
    if (heights.size > 2 * items.length + 1000) for (const k of heights.keys()) if (!index.has(k)) heights.delete(k);
    return changed;
  }

  // ── Scroll anchoring ─────────────────────────────────────────
  // WebKit has no overflow-anchor, and rows change height as they're first
  // measured, so we keep our own anchor: while pinned we stick to the
  // bottom, otherwise an item stays at the same distance from the viewport
  // top across prepends, measurements and resizes.
  let pinned = true;
  let anchor: { key: string; time: number; delta: number } | null = null;
  /** Where restore() last scrolled to, to recognise the scroll event it causes. */
  let ours = -1;

  function isAtBottom() {
    return !!el && el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  }

  function anchorIndex(a: NonNullable<typeof anchor>): number {
    const i = index.get(a.key);
    if (i !== undefined) return i;
    // The item is gone (filtered, or a run/day key changed): use the first
    // item at or after its time.
    let lo = 0;
    let hi = items.length - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (items[mid].time < a.time) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  function scrollTo(top: number) {
    if (!el) return;
    el.scrollTop = top;
    ours = el.scrollTop;
  }

  function restore() {
    if (!el) return;
    if (pinned) {
      const max = el.scrollHeight - el.clientHeight;
      if (el.scrollTop < max - 0.5) scrollTo(max);
    } else if (anchor && items.length) {
      const target = base + offs[anchorIndex(anchor)] - anchor.delta;
      // Only when it moved, so a smooth scroll isn't interrupted.
      if (Math.abs(el.scrollTop - target) >= 1) scrollTo(target);
    }
    viewTop = el.scrollTop;
  }

  function capture() {
    if (!el || !items.length) return (anchor = null);
    const i = indexAt(offs, el.scrollTop - base);
    anchor = { key: items[i].key, time: items[i].time, delta: base + offs[i] - el.scrollTop };
  }

  // Sitting at the bottom when new lines come in (say after a jump that
  // clamped there) means following them.
  $effect.pre(() => {
    void items;
    untrack(() => {
      if (el && !pinned && isAtBottom()) {
        pinned = true;
        anchor = null;
      }
    });
  });

  // After every render: measure what's on screen, and once heights have
  // settled put the anchor back. Runs before paint, so corrections don't flicker.
  $effect(() => {
    void visible;
    void stray;
    void offs;
    void buf.history;
    untrack(() => {
      if (measure()) hv++;
      else restore();
      syncTabStops();
      if (holdFocus && focusKey) {
        if (!index.has(focusKey) && items.length) focusKey = items[anchorIndex({ key: focusKey, time: focusTime, delta: 0 })].key;
        if (!document.activeElement || document.activeElement === document.body) rowEl(focusKey)?.focus({ preventScroll: true });
      }
    });
  });

  function onScroll() {
    if (!el) return;
    viewTop = el.scrollTop;
    // Our own scroll can be dispatched after the content grew again (a link
    // preview loaded); that's not the user leaving the bottom. The slack
    // covers the browser re-clamping a fractional scrollHeight.
    if (Math.abs(el.scrollTop - ours) >= 2) {
      ours = -1;
      pinned = isAtBottom();
      if (pinned) anchor = null;
      else capture();
    }
    atBottom = pinned || isAtBottom();
    app.atBottom = atBottom;
    if (el.scrollTop - base < OVERSCAN && buf.history !== 'done' && buf.history !== 'loading' && !filter) app.fetchHistory(buf, 'before');
    fillGaps();
  }

  /**
   * Load into any gap that's rendered (on screen or within the overscan),
   * from the side we're looking at it from, so lines grow away from the view.
   */
  function fillGaps() {
    if (!el || filter || !buf.gaps.length) return;
    for (let i = start; i < end; i++) {
      const it = items[i];
      if (it.kind !== 'gap') continue;
      const below = base + offs[i] > el.scrollTop + el.clientHeight / 2;
      app.fillGap(buf, it.gap, below ? 'after' : 'before');
      return;
    }
  }

  // Also when a gap comes into range without a scroll: after a jump, or as a fill lands.
  $effect(() => {
    void start;
    void end;
    void buf.gaps;
    untrack(fillGaps);
  });

  // Mark as read while the newest messages are on screen.
  $effect(() => {
    void buf.messages;
    if (atBottom && app.focused && !filter) app.markRead(buf);
  });

  /** Put item `i` `delta` px below the viewport top and keep it there. */
  function scrollToItem(i: number, delta: number) {
    if (!el) return;
    pinned = false;
    anchor = { key: items[i].key, time: items[i].time, delta };
    restore();
    atBottom = isAtBottom();
    app.atBottom = atBottom;
  }

  // Scroll to a line requested through buf.focus (search results, reply quotes).
  $effect(() => {
    const f = buf.focus;
    if (!f) return;
    untrack(() => {
      buf.focus = null;
      // The exact line can be missing (deleted, or past the server's history);
      // land on the nearest one after it instead, without the flash.
      const exact = source.find((l) => (f.msgid && l.msgid ? l.msgid === f.msgid : l.time === f.time));
      const line = exact ?? source.find((l) => l.time >= f.time) ?? source[source.length - 1];
      if (!line || !el) return;
      const i = itemIndexOf(items, line);
      if (i < 0) return;
      scrollToItem(i, el.clientHeight / 2);
      if (!exact) return;
      flash = line.id;
      setTimeout(() => flash === line.id && (flash = null), 1800);
    });
  });

  /** Expand or collapse a run in place, growing downward like any other content. */
  function toggleRun(key: string) {
    const i = index.get(key);
    if (el && i !== undefined) scrollToItem(i, base + offs[i] - el.scrollTop);
    if (expanded.has(key)) expanded.delete(key);
    else expanded.add(key);
  }

  function toBottom() {
    if (!el) return;
    anchor = null;
    // A long smooth scroll would render every row on the way; jump instead.
    if (el.scrollHeight - el.scrollTop - el.clientHeight > el.clientHeight) {
      pinned = true;
      restore();
    } else el.scrollTo({ top: el.scrollHeight, behavior: reducedMotion() ? 'auto' : 'smooth' });
  }

  const reducedMotion = () => window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  // ── Keyboard navigation ──────────────────────────────────────
  // Rows use a roving tabindex: the list itself is the tab stop until a row
  // has been focused, then that row is. The focused row is always rendered,
  // even scrolled out of the window, so focus isn't dropped.
  let focusKey = $state<string | null>(null);
  /** Its time, to land nearby if the row goes away (a run or day key changes on prepend). */
  let focusTime = 0;
  const focusIndex = $derived(focusKey === null ? -1 : (index.get(focusKey) ?? -1));
  /** The focused row, when it's outside the rendered window. */
  const stray = $derived(focusIndex >= 0 && (focusIndex < start || focusIndex >= end) ? items[focusIndex] : null);
  /** Row shortcuts (r, e, c, Delete) only after keyboard focus, so typing after a click still reaches the composer. */
  let viaKeyboard = false;
  /** Focus was on a row; if that row re-mounts, give it focus back. */
  let holdFocus = false;

  const talk = (it: Item): Line | null =>
    it.kind === 'line' && (it.line.type === 'privmsg' || it.line.type === 'notice' || it.line.type === 'action') ? it.line : null;

  function rowLabel(it: Item): string | undefined {
    const l = talk(it);
    if (!l) return undefined;
    const parent = l.replyTo && !l.redacted ? buf.byMsgid.get(l.replyTo) : undefined;
    return lineLabel(l, formatClock(l.time, s), parent?.nick);
  }

  /**
   * Links, nicks and buttons inside rows are only tab stops in the focused
   * row, so Tab crosses the list in one step instead of through every nick.
   */
  function syncTabStops() {
    if (!el) return;
    for (const row of el.querySelectorAll<HTMLElement>('[role="article"]')) {
      const on = row.dataset.key === focusKey;
      for (const c of row.querySelectorAll<HTMLElement>('a[href], button, input, summary, [data-rove]')) {
        if (on && c.hasAttribute('data-rove')) {
          c.removeAttribute('tabindex');
          c.removeAttribute('data-rove');
        } else if (!on && !c.hasAttribute('tabindex')) {
          c.tabIndex = -1;
          c.setAttribute('data-rove', '');
        }
      }
    }
  }

  $effect(() => {
    void focusKey;
    tick().then(syncTabStops);
  });

  const rowEl = (key: string) => el?.querySelector<HTMLElement>(`[role="article"][data-key="${CSS.escape(key)}"]`);

  /** Scroll just enough to show item `i`. */
  function reveal(i: number) {
    if (!el) return;
    const top = base + offs[i];
    const bottom = base + offs[i + 1];
    if (top < el.scrollTop) scrollTo(top - 8);
    else if (bottom > el.scrollTop + el.clientHeight) scrollTo(Math.min(top - 8, bottom - el.clientHeight + 8));
    else return;
    // Settle the anchor and window now rather than on the scroll event, so
    // restore() doesn't pull us back and the row mounts once in its final place.
    pinned = isAtBottom();
    if (pinned) anchor = null;
    else capture();
    atBottom = pinned;
    app.atBottom = atBottom;
    viewTop = el.scrollTop;
  }

  async function focusItem(i: number) {
    if (i < 0 || i >= items.length) return;
    focusKey = items[i].key;
    focusTime = items[i].time;
    viaKeyboard = true;
    reveal(i);
    await tick();
    rowEl(items[i].key)?.focus({ preventScroll: true });
  }

  function onKey(e: KeyboardEvent) {
    if (!el || e.metaKey || e.ctrlKey || e.altKey || !items.length) return;
    const t = e.target as HTMLElement;
    const onRow = t.getAttribute('role') === 'article';
    if (t !== el && !onRow) {
      // A control inside a row: Escape goes back to the row.
      if (e.key === 'Escape' && !e.defaultPrevented) {
        const row = t.closest<HTMLElement>('[role="article"]');
        if (row) {
          e.preventDefault();
          row.focus();
        }
      }
      return;
    }
    const i = onRow ? (index.get(t.dataset.key ?? '') ?? -1) : -1;
    const top = el.scrollTop - base;
    let to: number | null = null;
    switch (e.key) {
      case 'ArrowDown':
        to = i < 0 ? indexAt(offs, top) : i + 1;
        break;
      case 'ArrowUp':
        to = i < 0 ? indexAt(offs, top + el.clientHeight - 1) : i - 1;
        break;
      case 'Home':
        to = 0;
        break;
      case 'End':
        to = items.length - 1;
        break;
      case 'PageDown':
      case 'PageUp':
        if (i < 0) return;
        to = indexAt(offs, offs[i] + (e.key === 'PageDown' ? 1 : -1) * el.clientHeight * 0.8);
        break;
    }
    if (to !== null) {
      e.preventDefault();
      focusItem(Math.max(0, Math.min(items.length - 1, to)));
      return;
    }
    if (!onRow || !viaKeyboard) return;
    const it = items[i];
    if (it.kind === 'run' && (e.key === 'Enter' || e.key === ' ')) {
      e.preventDefault();
      toggleRun(it.key);
      return;
    }
    const l = talk(it);
    if (!l) return;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    if (key === 'r' && canReply(buf, l)) startReply(buf, l);
    else if (key === 'e' && app.canReact(buf, l)) openReact(l);
    else if (key === 'c' && !l.redacted) copyLine(l);
    else if (key === 'Delete' && app.canRedact(buf, l) && !l.redacted) {
      askDelete(l);
      tick().then(() => rowEl(it.key)?.querySelector<HTMLElement>('.tools .wide')?.focus());
    } else return;
    e.preventDefault();
  }

  function onFocusIn(e: FocusEvent) {
    const t = e.target as HTMLElement;
    if (t.getAttribute('role') !== 'article') return;
    focusKey = t.dataset.key ?? null;
    focusTime = items[index.get(focusKey ?? '') ?? -1]?.time ?? focusTime;
    // Programmatic focus from our own key handling counts as keyboard too.
    viaKeyboard ||= t.matches(':focus-visible');
    holdFocus = true;
  }

  function onFocusOut(e: FocusEvent) {
    const t = e.target as HTMLElement;
    const to = e.relatedTarget as Node | null;
    if (to && el?.contains(to)) return;
    // A row that unmounts (virtualization) drops focus without the user
    // asking; anything else is them moving on.
    queueMicrotask(() => {
      if (t.isConnected) holdFocus = false;
    });
  }

  // ── Screen reader announcements ──────────────────────────────
  // The rows are virtualized, so they can't be a live region themselves:
  // scrolling would announce every row it mounts. New live lines are
  // echoed into a separate log instead, only while following the bottom.
  let lastSeen = untrack(() => buf.messages[buf.messages.length - 1]);
  let spoken = $state<{ id: number; text: string }[]>([]);
  let spokenSeq = 0;
  $effect(() => {
    const lines = buf.messages;
    untrack(() => {
      const prev = lastSeen;
      lastSeen = lines[lines.length - 1];
      if (!atBottom || filter) return;
      const said = announcements(lines, prev, Date.now());
      if (said.length) spoken = [...spoken, ...said.map((text) => ({ id: ++spokenSeq, text }))].slice(-5);
    });
  });

  /** Observe rows for size changes we didn't cause: previews, images, expanded runs. */
  let rowObserver: ResizeObserver | undefined;
  function observe(node: HTMLElement) {
    // Deferred to the next frame: rows mounted from inside an observer
    // callback would otherwise trip the "ResizeObserver loop" error.
    const raf = requestAnimationFrame(() => rowObserver?.observe(node));
    return { destroy: () => (cancelAnimationFrame(raf), rowObserver?.unobserve(node)) };
  }

  onMount(() => {
    if (!el) return;
    viewH = el.clientHeight;
    const u = index.get('u');
    // A jump requested before we mounted takes precedence.
    if (u !== undefined && !anchor) scrollToItem(u, el.clientHeight / 3);
    else restore();

    rowObserver = new ResizeObserver(() => measure() && hv++);
    let width = el.clientWidth;
    const ro = new ResizeObserver(() => {
      if (!el) return;
      viewH = el.clientHeight;
      // Rewrapping changes every row's height; forget them so offscreen rows
      // go back to estimates rather than stale measurements.
      if (el.clientWidth !== width) {
        width = el.clientWidth;
        heights.clear();
        est.clear();
      }
      hv++;
    });
    ro.observe(el);
    if (el.scrollHeight <= el.clientHeight) onScroll();
    return () => {
      ro.disconnect();
      rowObserver?.disconnect();
    };
  });

  // Row heights depend on the layout and font size.
  $effect(() => {
    void s.layout;
    void s.fontSize;
    untrack(() => {
      heights.clear();
      est.clear();
      hv++;
    });
  });
</script>

<svelte:window onclick={closeMenus} />

{#snippet row(it: Item)}
  {#if it.kind === 'day'}
    <Divider label={it.label} />
  {:else if it.kind === 'unread'}
    <Divider />
  {:else if it.kind === 'gap'}
    <div class="gap">
      {#if canFill(app, buf)}<span class="spin"></span>Loading messages{:else}Messages here aren't loaded. Reconnect to load them.{/if}
    </div>
  {:else if it.kind === 'run'}
    <EventRun lines={it.lines} {buf} open={expanded.has(it.key)} {onnick} ontoggle={() => toggleRun(it.key)} />
  {:else}
    {@const l = it.line}
    {#if l.type === 'privmsg' || l.type === 'notice' || l.type === 'action'}
      <MessageRow {l} cont={it.cont} {buf} flash={flash === l.id} {onnick} />
    {:else if l.type === 'whois' && l.data}
      <WhoisCard {l} {onnick} />
    {:else if l.data?.motd !== undefined}
      <MotdBlock {l} open={l.data.title !== undefined || buf.messages.length < 40} />
    {:else}
      <EventRow {l} {buf} flash={flash === l.id} {onnick} />
    {/if}
  {/if}
{/snippet}

<!-- The WAI-ARIA feed pattern: a focusable feed of focusable articles, which
     svelte's linter doesn't know are interactive. -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="scroller layout-{s.layout}"
  bind:this={el}
  onscroll={onScroll}
  onkeydown={onKey}
  onfocusin={onFocusIn}
  onfocusout={onFocusOut}
  onpointerdown={() => (viaKeyboard = false)}
  role="feed"
  aria-label="Messages in {buf.kind === 'server' ? buf.conn.name : buf.name}"
  aria-busy={buf.history === 'loading'}
  tabindex={focusIndex >= 0 ? -1 : 0}
>
  <div class="spacer"></div>
  {#if buf.history === 'loading'}
    <div class="loading"><span></span>Loading history</div>
  {:else if buf.history === 'done' && buf.kind !== 'server'}
    <div class="start">
      <p class="eyebrow">Beginning of history</p>
      <p>{buf.kind === 'channel' ? buf.name : buf.kind === 'query' ? `Your conversation with ${buf.name}` : ''}</p>
    </div>
  {/if}

  <div class="items" bind:this={list} style="padding-top:{offs[start]}px;padding-bottom:{offs[items.length] - offs[end]}px">
    {#each visible as it, j (it.key)}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div
        class="item focus-inset"
        data-key={it.key}
        role="article"
        aria-label={rowLabel(it)}
        aria-posinset={start + j + 1}
        aria-setsize={buf.history === 'done' ? items.length : -1}
        tabindex={it.key === focusKey ? 0 : -1}
        use:observe
      >
        {@render row(it)}
      </div>
    {/each}
  </div>
  {#if stray}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="item stray focus-inset"
      style="top:{base + offs[focusIndex]}px"
      data-key={stray.key}
      role="article"
      aria-label={rowLabel(stray)}
      aria-posinset={focusIndex + 1}
      aria-setsize={buf.history === 'done' ? items.length : -1}
      tabindex="0"
    >
      {@render row(stray)}
    </div>
  {/if}
  <div class="end"></div>
</div>

<div class="sr-only" role="log" aria-live="polite" aria-label="New messages">
  {#each spoken as m (m.id)}<p>{m.text}</p>{/each}
</div>

{#if !atBottom}
  <button class="jump" onclick={toBottom}>
    <Icon name="arrowDown" size={14} />
    {buf.unread > 0 ? `${buf.unread} new` : buf.gaps.length ? 'Jump to present' : 'Latest'}
  </button>
{/if}

<style>
  .scroller {
    position: relative;
    --time-w: 52px;
    --who-w: 132px;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    /* Anchoring is done by hand (see restore); the browser's would fight it. */
    overflow-anchor: none;
    display: flex;
    flex-direction: column;
    padding: 0 0 6px;
  }

  .spacer {
    flex: 1;
  }

  /* Own formatting context, so row margins count toward the measured height. */
  .item {
    display: flow-root;
  }

  .item.stray {
    position: absolute;
    left: 0;
    right: 0;
  }

  :global(html) .scroller {
    font-size: var(--fs);
  }

  /* ── Row structure, shared by the row components ──────────── */
  .scroller :global(.row) {
    display: grid;
    grid-template-columns: var(--time-w) var(--who-w) minmax(0, 1fr);
    column-gap: 12px;
    padding: 1px 20px 1px 14px;
    position: relative;
    line-height: 1.5;
  }

  .scroller :global(.row:hover) {
    background: var(--bg-hover);
  }

  .scroller :global(.msg) {
    padding-top: 3px;
    padding-bottom: 1px;
  }

  .scroller :global(.msg.cont) {
    padding-top: 0;
  }

  .scroller :global(time) {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-4);
    text-align: right;
    padding-top: 0.2em;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .scroller :global(.msg.cont time) {
    visibility: hidden;
  }

  .scroller :global(.msg.cont:hover time) {
    visibility: visible;
  }

  .scroller :global(.who) {
    text-align: right;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .scroller :global(.text) {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .scroller :global(.nick) {
    font-weight: 500;
    cursor: pointer;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    vertical-align: bottom;
  }

  .scroller :global(.nick:hover) {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .scroller :global(.who .nick) {
    font-weight: 600;
  }

  .scroller :global(.notice .text) {
    color: var(--text-2);
  }

  .scroller :global(.action .text) {
    font-style: italic;
    color: var(--text-2);
  }

  .scroller :global(.action .text .nick) {
    font-style: normal;
  }

  .scroller :global(.glyph) {
    font-family: var(--font-mono);
    color: var(--text-4);
    font-size: 12px;
  }

  .scroller :global(.hl) {
    background: var(--mention);
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .scroller :global(.hl:hover) {
    background: color-mix(in oklab, var(--mention) 100%, var(--bg-hover));
  }

  .scroller :global(.flash) {
    animation: flash 1.8s var(--ease);
  }

  /* Events: joins, modes, topics … */
  .scroller :global(.event) {
    font-size: 0.9em;
    color: var(--text-3);
    padding-top: 2px;
    padding-bottom: 2px;
  }

  .scroller :global(.event .nick) {
    font-weight: 500;
    font-size: inherit;
  }

  .scroller :global(.event.error .text) {
    color: var(--danger);
  }

  .scroller :global(.event.error .glyph) {
    color: var(--danger);
  }

  .scroller :global(.faint) {
    color: var(--text-4);
  }

  .scroller :global(.run-line time) {
    padding: 0;
  }

  /* Cards: whois, motd */
  .scroller :global(.block) {
    padding-top: 6px;
    padding-bottom: 6px;
  }

  .scroller :global(.card-head .nick) {
    font-weight: 600;
    font-size: 14px;
  }

  @keyframes flash {
    0%,
    30% {
      background: var(--accent-soft);
    }
  }

  .loading,
  .start {
    padding: 28px 20px 12px calc(14px + var(--time-w) + var(--who-w) + 24px);
    color: var(--text-3);
    font-size: 13px;
  }

  .start p {
    margin: 0 0 4px;
  }

  .start p:last-child {
    font-size: 18px;
    font-weight: 600;
    color: var(--text);
    letter-spacing: -0.01em;
  }

  .loading {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .gap {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 20px 12px calc(14px + var(--time-w) + var(--who-w) + 24px);
    border-block: 1px dashed var(--line);
    color: var(--text-3);
    font-size: 13px;
  }

  .loading span,
  .gap .spin {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 1.5px solid var(--line-strong);
    border-top-color: var(--accent);
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .end {
    height: 4px;
  }

  .jump {
    position: absolute;
    right: 20px;
    bottom: 96px;
    height: 28px;
    padding: 0 12px 0 10px;
    display: flex;
    align-items: center;
    gap: 6px;
    border-radius: 14px;
    background: var(--bg-raised);
    box-shadow: var(--shadow);
    font-size: 12px;
    font-weight: 500;
    color: var(--text-2);
    z-index: 4;
    animation: rise 160ms var(--ease);
  }

  .jump:hover {
    color: var(--text);
  }

  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }

  /* ── Compact layout: nick above grouped lines ─────────────── */
  .scroller.layout-compact :global(.row) {
    grid-template-columns: var(--time-w) minmax(0, 1fr);
  }

  .scroller.layout-compact :global(.row .who) {
    display: none;
  }

  .scroller.layout-compact :global(.msg time) {
    visibility: hidden;
  }

  .scroller.layout-compact :global(.msg:hover time) {
    visibility: visible;
  }

  .scroller.layout-compact :global(.msg:not(.cont)) {
    padding-top: 10px;
  }

  .scroller.layout-compact :global(.msg:not(.cont):not(.action) time) {
    padding-top: 1.75em;
  }

  .scroller.layout-compact :global(.head) {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .scroller.layout-compact :global(.head .nick) {
    font-weight: 600;
  }

  .scroller.layout-compact :global(.event .who) {
    display: none;
  }

  .layout-compact .loading,
  .layout-compact .start {
    padding-left: calc(14px + var(--time-w) + 12px);
  }
</style>
