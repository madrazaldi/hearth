<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { app, type Buffer } from '../state.svelte';
  import { stripFormatting } from '../format';
  import { buildItems, itemIndexOf, MEMBERSHIP, type Item } from '../timeline';
  import { Estimates, indexAt, offsets as prefixSums, rangeFor } from '../virtual';
  import Icon from './Icon.svelte';
  import Divider from './Divider.svelte';
  import EventRow from './EventRow.svelte';
  import EventRun from './EventRun.svelte';
  import MessageRow from './MessageRow.svelte';
  import { closeMenus } from './MessageTools.svelte';
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

  const items = $derived(buildItems(source, { divider: buf.divider, channel: buf.kind === 'channel', membership: s.membership }));
  const index = $derived(new Map(items.map((it, i) => [it.key, i])));

  // ── Virtualization ───────────────────────────────────────────
  // Measured heights by item key; anything unmeasured uses a per-category
  // average. `hv` bumps whenever `heights` changes.
  const heights = new Map<string, number>();
  const est = new Estimates({ day: 41, unread: 19, run: 25, msg: 28, cont: 22, event: 24, block: 160 });
  let hv = $state(0);

  function category(it: Item): string {
    if (it.kind !== 'line') return it.kind;
    const t = it.line.type;
    if (t === 'privmsg' || t === 'notice' || t === 'action') return it.cont ? 'cont' : 'msg';
    return t === 'whois' || it.line.data?.motd !== undefined ? 'block' : 'event';
  }

  const offs = $derived.by(() => {
    void hv;
    return prefixSums(items.length, (i) => heights.get(items[i].key) ?? est.get(category(items[i])));
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
      const old = heights.get(key);
      if (old !== undefined && Math.abs(old - h) < 0.5) continue;
      heights.set(key, h);
      est.add(category(items[i]), h, old);
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
    void offs;
    void buf.history;
    untrack(() => {
      if (measure()) hv++;
      else restore();
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
  }

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
      const line = source.find((l) => (f.msgid ? l.msgid === f.msgid : l.time === f.time));
      if (!line || !el) return;
      const i = itemIndexOf(items, line);
      if (i < 0) return;
      scrollToItem(i, el.clientHeight / 2);
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
    } else el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' });
  }

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

<div class="scroller layout-{s.layout}" bind:this={el} onscroll={onScroll}>
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
    {#each visible as it (it.key)}
      <div class="item" data-key={it.key} use:observe>
        {#if it.kind === 'day'}
          <Divider label={it.label} />
        {:else if it.kind === 'unread'}
          <Divider />
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
      </div>
    {/each}
  </div>
  <div class="end"></div>
</div>

{#if !atBottom}
  <button class="jump" onclick={toBottom}>
    <Icon name="arrowDown" size={14} />
    {buf.unread > 0 ? `${buf.unread} new` : 'Latest'}
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

  .loading span {
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
