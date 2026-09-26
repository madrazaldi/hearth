<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { app, type Buffer } from '../state.svelte';
  import { stripFormatting } from '../format';
  import { nickHue } from '../nickcolor';
  import { dayKey, formatClock, formatDay, formatDuration, formatFull } from '../time';
  import type { Line } from '../types';
  import RichText from './RichText.svelte';
  import Icon from './Icon.svelte';

  let { buf, filter = '', onnick }: { buf: Buffer; filter?: string; onnick: (nick: string, e: MouseEvent) => void } = $props();

  type Item =
    | { kind: 'day'; key: string; label: string }
    | { kind: 'unread'; key: string }
    | { kind: 'line'; key: string; line: Line; cont: boolean }
    | { kind: 'run'; key: string; lines: Line[] };

  const MEMBERSHIP = new Set(['join', 'part', 'quit', 'nick']);
  const TALK = new Set(['privmsg', 'action', 'notice']);
  const PAGE = 150;

  let el = $state<HTMLDivElement>();
  let limit = $state(PAGE);
  let atBottom = $state(true);
  const expanded = new SvelteSet<string>();

  const s = $derived(app.settings);

  const source = $derived.by(() => {
    let lines = buf.messages;
    if (s.membership === 'hide' && buf.kind === 'channel') lines = lines.filter((l) => !MEMBERSHIP.has(l.type));
    const q = filter.trim().toLowerCase();
    if (q) lines = lines.filter((l) => (l.nick + ' ' + stripFormatting(l.text)).toLowerCase().includes(q));
    return lines;
  });

  const items = $derived.by(() => {
    const lines = source.slice(-limit);
    const out: Item[] = [];
    let prev: Line | null = null;
    let run: Line[] = [];
    let dividerPlaced = !buf.divider;

    const flush = () => {
      if (!run.length) return;
      if (run.length === 1 || s.membership === 'show') for (const l of run) out.push({ kind: 'line', key: 'l' + l.id, line: l, cont: false });
      else out.push({ kind: 'run', key: 'r' + run[0].id, lines: run });
      run = [];
    };

    for (const l of lines) {
      if (!prev || dayKey(prev.time) !== dayKey(l.time)) {
        flush();
        out.push({ kind: 'day', key: 'd' + l.id, label: formatDay(l.time) });
        prev = null;
      }
      if (!dividerPlaced && l.time > buf.divider && !l.self) {
        flush();
        out.push({ kind: 'unread', key: 'u' });
        dividerPlaced = true;
        prev = null;
      }
      if (MEMBERSHIP.has(l.type) && buf.kind === 'channel') {
        run.push(l);
        prev = l;
        continue;
      }
      flush();
      const cont =
        !!prev &&
        TALK.has(l.type) &&
        prev.type === l.type &&
        l.type !== 'action' &&
        prev.nick === l.nick &&
        l.time - prev.time < 5 * 60_000;
      out.push({ kind: 'line', key: 'l' + l.id, line: l, cont });
      prev = l;
    }
    flush();
    return out;
  });

  // ── Scroll anchoring (WebKit has no overflow-anchor) ─────────
  let snap = { h: 0, top: 0, stick: true, first: '' };

  $effect.pre(() => {
    void items;
    if (!el) return;
    snap = { h: el.scrollHeight, top: el.scrollTop, stick: isAtBottom(), first: firstLineKey() };
  });

  $effect(() => {
    void items;
    if (!el) return;
    if (snap.stick) el.scrollTop = el.scrollHeight;
    else if (snap.first && firstLineKey() !== snap.first) el.scrollTop = snap.top + (el.scrollHeight - snap.h);
  });

  function firstLineKey() {
    const first = items.find((i) => i.kind === 'line' || i.kind === 'run');
    return first?.key ?? '';
  }

  function isAtBottom() {
    return !!el && el.scrollHeight - el.scrollTop - el.clientHeight < 24;
  }

  // Mark as read while the newest messages are on screen.
  $effect(() => {
    void buf.messages;
    if (atBottom && app.focused && !filter) app.markRead(buf);
  });

  async function onScroll() {
    if (!el) return;
    atBottom = isAtBottom();
    app.atBottom = atBottom;
    if (el.scrollTop < 400) {
      if (limit < source.length) limit += PAGE;
      else if (buf.history !== 'done' && buf.history !== 'loading' && !filter) {
        await app.fetchHistory(buf, 'before');
        if (source.length > limit) limit += PAGE;
      }
    }
  }

  function toBottom() {
    limit = PAGE;
    tick().then(() => el && el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' }));
  }

  onMount(() => {
    if (!el) return;
    const divider = el.querySelector('.unread-divider');
    if (divider) {
      // Not scrollIntoView: it would also scroll the overflow-hidden page shell.
      const d = divider as HTMLElement;
      el.scrollTop = d.offsetTop - el.clientHeight / 3;
      atBottom = isAtBottom();
    } else el.scrollTop = el.scrollHeight;
    const ro = new ResizeObserver(() => {
      if (atBottom && el) el.scrollTop = el.scrollHeight;
    });
    ro.observe(el);
    if (el.scrollHeight <= el.clientHeight) onScroll();
    return () => ro.disconnect();
  });

  const clock = (t: number) => formatClock(t, s);

  function summarize(lines: Line[]): string {
    const joined: string[] = [];
    const left: string[] = [];
    const renamed: string[] = [];
    for (const l of lines) {
      if (l.type === 'join' && !joined.includes(l.nick)) joined.push(l.nick);
      else if ((l.type === 'part' || l.type === 'quit') && !left.includes(l.nick)) left.push(l.nick);
      else if (l.type === 'nick') renamed.push(`${l.nick} → ${l.target}`);
    }
    const names = (a: string[]) => (a.length <= 3 ? a.join(', ') : `${a.slice(0, 2).join(', ')} and ${a.length - 2} others`);
    const parts: string[] = [];
    if (joined.length) parts.push(`${names(joined)} joined`);
    if (left.length) parts.push(`${names(left)} left`);
    if (renamed.length) parts.push(renamed.length <= 2 ? renamed.join(', ') : `${renamed.length} nick changes`);
    return parts.join(' · ');
  }

  const glyph: Record<string, string> = {
    join: '→',
    part: '←',
    quit: '←',
    nick: '↻',
    kick: '⨯',
    mode: '±',
    topic: '¶',
    invite: '✉',
    info: '·',
    error: '!',
  };
</script>

{#snippet nickBtn(nick: string, self = false)}
  <button class="nick" class:me={self} style="--h:{nickHue(nick)}" onclick={(e) => onnick(nick, e)}>{nick}</button>
{/snippet}

{#snippet event(l: Line)}
  {#if l.type === 'join'}
    {@render nickBtn(l.nick)} joined {#if l.data?.host && l.data.host !== '@'}<span class="faint">{l.data.host}</span>{/if}
  {:else if l.type === 'part'}
    {@render nickBtn(l.nick)} left{#if l.text}<span class="faint"> ({stripFormatting(l.text)})</span>{/if}
  {:else if l.type === 'quit'}
    {@render nickBtn(l.nick)} quit{#if l.text}<span class="faint"> ({stripFormatting(l.text)})</span>{/if}
  {:else if l.type === 'nick'}
    {@render nickBtn(l.nick)} is now {@render nickBtn(l.target ?? '')}
  {:else if l.type === 'kick'}
    {@render nickBtn(l.nick)} removed {@render nickBtn(l.target ?? '')}{#if l.text}<span class="faint"> ({stripFormatting(l.text)})</span>{/if}
  {:else if l.type === 'mode'}
    {#if l.nick}{@render nickBtn(l.nick)} set mode{:else}Mode{/if} <span class="mono">{l.text}</span>
  {:else if l.type === 'topic'}
    {@render nickBtn(l.nick)} changed the topic to <span class="quote"><RichText text={l.text} /></span>
  {:else if l.type === 'invite'}
    {@render nickBtn(l.nick)} {l.text}
    {#if l.target}<button class="inline-action" onclick={() => app.join(buf.conn, l.target!)}>Join</button>{/if}
  {:else}
    <RichText text={l.text} />
  {/if}
{/snippet}

{#snippet body(l: Line)}
  {#if l.redacted}
    <span class="faint"><i>message deleted</i></span>
  {:else}
    <span class="body"><RichText text={l.text} /></span>
  {/if}
{/snippet}

<div class="scroller layout-{s.layout}" bind:this={el} onscroll={onScroll}>
  <div class="spacer"></div>
  {#if buf.history === 'loading'}
    <div class="loading"><span></span>Loading history</div>
  {:else if buf.history === 'done' && buf.kind !== 'server' && limit >= source.length}
    <div class="start">
      <p class="eyebrow">Beginning of history</p>
      <p>{buf.kind === 'channel' ? buf.name : buf.kind === 'query' ? `Your conversation with ${buf.name}` : ''}</p>
    </div>
  {/if}

  {#each items as it (it.key)}
    {#if it.kind === 'day'}
      <div class="day"><span>{it.label}</span></div>
    {:else if it.kind === 'unread'}
      <div class="unread-divider"><span>New</span></div>
    {:else if it.kind === 'run'}
      {@const open = expanded.has(it.key)}
      <div class="row event run" class:open>
        <time title={formatFull(it.lines[0].time)}>{clock(it.lines[0].time)}</time>
        <span class="who"><span class="glyph">⇅</span></span>
        <div class="text">
          <button
            class="run-toggle"
            onclick={() => (open ? expanded.delete(it.key) : expanded.add(it.key))}
            title={open ? 'Collapse' : 'Show all'}
          >
            {summarize(it.lines)}
            <span class="chev" class:open><Icon name="chevron" size={11} /></span>
          </button>
          {#if open}
            <div class="run-lines">
              {#each it.lines as l (l.id)}
                <div class="run-line">
                  <time>{clock(l.time)}</time>
                  <span class="glyph">{glyph[l.type]}</span>
                  <span>{@render event(l)}</span>
                </div>
              {/each}
            </div>
          {/if}
        </div>
      </div>
    {:else}
      {@const l = it.line}
      {#if l.type === 'privmsg' || l.type === 'notice' || l.type === 'action'}
        <div
          class="row msg {l.type}"
          class:cont={it.cont}
          class:hl={l.highlight}
          class:self={l.self}
        >
          <time title={formatFull(l.time)}>{clock(l.time)}</time>
          {#if s.layout === 'columns'}
            <span class="who">
              {#if l.type === 'action'}<span class="glyph">*</span>
              {:else if !it.cont}
                {#if l.type === 'notice'}<span class="notice-mark">–</span>{/if}{@render nickBtn(l.nick, l.self)}
              {/if}
            </span>
            <div class="text selectable">
              {#if l.type === 'action'}{@render nickBtn(l.nick, l.self)}{' '}{/if}{@render body(l)}
            </div>
          {:else}
            <div class="text selectable">
              {#if !it.cont && l.type !== 'action'}
                <div class="head">
                  {@render nickBtn(l.nick, l.self)}
                  {#if l.type === 'notice'}<span class="tag">notice</span>{/if}
                  <span class="head-time" title={formatFull(l.time)}>{clock(l.time)}</span>
                </div>
              {/if}
              {#if l.type === 'action'}<span class="glyph">*</span>{' '}{@render nickBtn(l.nick, l.self)}{' '}{/if}{@render body(l)}
            </div>
          {/if}
        </div>
      {:else if l.type === 'whois' && l.data}
        {@const w = l.data}
        <div class="row block">
          <time title={formatFull(l.time)}>{clock(l.time)}</time>
          <span class="who"></span>
          <div class="card selectable">
            <div class="card-head">
              {@render nickBtn(w.nick)}
              <span class="faint mono">{w.user}@{w.host}</span>
            </div>
            <dl>
              {#if w.realname}<dt>Name</dt><dd><RichText text={w.realname} /></dd>{/if}
              {#if w.account}<dt>Account</dt><dd>{w.account}</dd>{/if}
              {#if w.server}<dt>Server</dt><dd>{w.server}</dd>{/if}
              {#if w.channels}<dt>Channels</dt><dd class="mono small">{w.channels}</dd>{/if}
              {#if w.idle}<dt>Idle</dt><dd>{formatDuration(parseInt(w.idle))}{#if w.signon}<span class="faint"> · online since {formatFull(parseInt(w.signon) * 1000)}</span>{/if}</dd>{/if}
              {#if w.away}<dt>Away</dt><dd>{w.away}</dd>{/if}
              {#if w.oper}<dt>Operator</dt><dd>{w.oper}</dd>{/if}
              {#if w.secure}<dt>Security</dt><dd>{w.secure}</dd>{/if}
              {#if w.actual}<dt>Actual</dt><dd class="mono small">{w.actual}</dd>{/if}
            </dl>
          </div>
        </div>
      {:else if l.data?.motd !== undefined}
        <div class="row block">
          <time title={formatFull(l.time)}>{clock(l.time)}</time>
          <span class="who"></span>
          <details class="motd" open={l.data.title !== undefined || buf.messages.length < 40}>
            <summary>{l.data.title ?? 'Message of the day'}</summary>
            <pre class="selectable"><RichText text={l.data.motd} /></pre>
          </details>
        </div>
      {:else}
        <div class="row event {l.type}" class:hl={l.highlight}>
          <time title={formatFull(l.time)}>{clock(l.time)}</time>
          <span class="who"><span class="glyph">{glyph[l.type] ?? '·'}</span></span>
          <div class="text selectable">{@render event(l)}</div>
        </div>
      {/if}
    {/if}
  {/each}
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
    display: flex;
    flex-direction: column;
    padding: 0 0 6px;
  }

  .spacer {
    flex: 1;
  }

  :global(html) .scroller {
    font-size: var(--fs);
  }

  .row {
    display: grid;
    grid-template-columns: var(--time-w) var(--who-w) minmax(0, 1fr);
    column-gap: 12px;
    padding: 1px 20px 1px 14px;
    position: relative;
    line-height: 1.5;
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .msg {
    padding-top: 3px;
    padding-bottom: 1px;
  }

  .msg.cont {
    padding-top: 0;
  }

  time {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-4);
    text-align: right;
    padding-top: 0.2em;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .msg.cont time {
    visibility: hidden;
  }

  .msg.cont:hover time {
    visibility: visible;
  }

  .who {
    text-align: right;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .text {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .body {
    white-space: pre-wrap;
  }

  .nick {
    font-weight: 500;
    cursor: pointer;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    vertical-align: bottom;
  }

  .nick:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .who .nick {
    font-weight: 600;
  }

  .notice-mark {
    color: var(--text-4);
    margin-right: 4px;
  }

  .notice .text {
    color: var(--text-2);
  }

  .action .text {
    font-style: italic;
    color: var(--text-2);
  }

  .action .text :global(.nick) {
    font-style: normal;
  }

  .glyph {
    font-family: var(--font-mono);
    color: var(--text-4);
    font-size: 12px;
  }

  .hl {
    background: var(--mention);
    box-shadow: inset 2px 0 0 var(--accent);
  }

  .hl:hover {
    background: color-mix(in oklab, var(--mention) 100%, var(--bg-hover));
  }

  /* Events: joins, modes, topics … */
  .event {
    font-size: 0.9em;
    color: var(--text-3);
    padding-top: 2px;
    padding-bottom: 2px;
  }

  .event .nick {
    font-weight: 500;
    font-size: inherit;
  }

  .event.error .text {
    color: var(--danger);
  }

  .event.error .glyph {
    color: var(--danger);
  }

  .faint {
    color: var(--text-4);
  }

  .quote {
    color: var(--text-2);
  }

  .inline-action {
    margin-left: 8px;
    font-size: 12px;
    color: var(--accent);
  }

  .inline-action:hover {
    text-decoration: underline;
  }

  .run-toggle {
    text-align: left;
    color: var(--text-3);
    display: inline;
  }

  .run-toggle:hover {
    color: var(--text-2);
  }

  .chev {
    display: inline-flex;
    vertical-align: middle;
    margin-left: 4px;
    color: var(--text-4);
    transition: transform 150ms var(--ease);
  }

  .chev.open {
    transform: rotate(90deg);
  }

  .run-lines {
    margin: 4px 0 2px;
    display: grid;
    gap: 1px;
  }

  .run-line {
    display: flex;
    gap: 8px;
    align-items: baseline;
  }

  .run-line time {
    padding: 0;
  }

  /* Day and unread separators */
  .day {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 18px 20px 8px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-3);
  }

  .day::before,
  .day::after {
    content: '';
    flex: 1;
    height: 1px;
    background: var(--line);
  }

  .unread-divider {
    position: relative;
    height: 1px;
    margin: 10px 20px 8px 14px;
    background: var(--accent);
    opacity: 0.8;
  }

  .unread-divider span {
    position: absolute;
    right: 0;
    top: -8px;
    padding: 0 6px;
    background: var(--bg);
    color: var(--accent);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    line-height: 16px;
  }

  /* Cards: whois, motd */
  .block {
    padding-top: 6px;
    padding-bottom: 6px;
  }

  .card,
  .motd {
    max-width: 640px;
    border-radius: 8px;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
    padding: 10px 14px;
    font-size: 13px;
  }

  .card-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-bottom: 8px;
  }

  .card-head .nick {
    font-weight: 600;
    font-size: 14px;
  }

  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 3px 16px;
    margin: 0;
  }

  dt {
    color: var(--text-3);
  }

  dd {
    margin: 0;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }

  .small {
    font-size: 12px;
  }

  .motd summary {
    color: var(--text-3);
    font-size: 12px;
    font-weight: 500;
    cursor: default;
    list-style: none;
  }

  .motd summary::before {
    content: '▸ ';
    color: var(--text-4);
  }

  .motd[open] summary::before {
    content: '▾ ';
  }

  .motd pre {
    margin: 8px 0 2px;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-2);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
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
  .layout-compact .row {
    grid-template-columns: var(--time-w) minmax(0, 1fr);
  }

  .layout-compact .row .who {
    display: none;
  }

  .layout-compact .msg time {
    visibility: hidden;
  }

  .layout-compact .msg:hover time {
    visibility: visible;
  }

  .layout-compact .msg:not(.cont) {
    padding-top: 10px;
  }

  .layout-compact .msg:not(.cont):not(.action) time {
    padding-top: 1.75em;
  }

  .layout-compact .head {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }

  .layout-compact .head .nick {
    font-weight: 600;
  }

  .head-time {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-4);
  }

  .tag {
    font-size: 10.5px;
    color: var(--text-3);
    padding: 0 5px;
    border-radius: 4px;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
  }

  .layout-compact .loading,
  .layout-compact .start {
    padding-left: calc(14px + var(--time-w) + 12px);
  }

  .layout-compact .event .who {
    display: none;
  }
</style>
