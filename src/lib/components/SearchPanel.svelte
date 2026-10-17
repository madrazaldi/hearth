<script module lang="ts">
  import type { Buffer as Buf } from '../state.svelte';
  import type { SearchHit as Hit } from '../types';

  /** The last search, kept so reopening after a jump picks up where it left off. */
  let last: { q: string; origin: Buf | null; scope: 'all' | 'network' | 'buffer'; hits: Hit[]; next: number | null; sel: number } | null =
    null;
</script>

<script lang="ts">
  import { tick } from 'svelte';
  import { app, type Buffer, type Conn } from '../state.svelte';
  import { parseCtcp } from '../irc';
  import { stripFormatting } from '../format';
  import { nickHue } from '../nickcolor';
  import { dedupeHits, hitTime, parseQuery } from '../search';
  import type { SearchHit } from '../types';
  import Segmented from './Segmented.svelte';
  import { modal } from '../a11y';

  const resumed = last && app.searchQuery && last.q === app.searchQuery ? last : null;
  const origin = resumed ? resumed.origin : app.active;
  let q = $state(app.searchQuery);
  let scope = $state<'all' | 'network' | 'buffer'>(resumed?.scope ?? 'all');
  let hits = $state<SearchHit[]>(resumed?.hits ?? []);
  let next = $state<number | null>(resumed?.next ?? null);
  let busy = $state(false);
  let more = $state(false);
  let sel = $state(resumed?.sel ?? 0);
  let sugSel = $state(0);
  let list = $state<HTMLDivElement>();
  let input = $state<HTMLInputElement>();
  let run = 0;
  let skip = !!resumed;

  const remoteAvailable = $derived([...app.conns.values()].some((c) => c.has('soju.im/search')));
  const canSearch = $derived(app.settings.storeLogs || remoteAvailable);

  function scoped(sc: typeof scope): { conn?: Conn; buf?: Buffer } {
    return sc === 'buffer' && origin ? { conn: origin.conn, buf: origin } : sc === 'network' && origin ? { conn: origin.conn } : {};
  }

  $effect(() => {
    const query = q;
    const sc = scope;
    const id = ++run;
    if (skip) {
      skip = false;
      return;
    }
    more = false;
    if (!query.trim()) {
      hits = [];
      next = null;
      busy = false;
      return;
    }
    busy = true;
    const t = setTimeout(async () => {
      const page = await app.search(query, scoped(sc));
      if (id !== run) return;
      hits = page.hits;
      next = page.next;
      sel = 0;
      busy = false;
      list?.scrollTo({ top: 0 });
    }, 180);
    return () => clearTimeout(t);
  });

  /** Fetch the next page of older hits. */
  async function loadMore() {
    if (more || busy || next === null) return;
    const id = run;
    more = true;
    const page = await app.search(q, scoped(scope), next);
    if (id !== run) return;
    hits = dedupeHits([...hits, ...page.hits]);
    next = page.next;
    more = false;
  }

  function onListScroll() {
    if (list && list.scrollTop + list.clientHeight > list.scrollHeight - 240) loadMore();
  }

  $effect(() => {
    void sel;
    const el = list?.querySelector('.sel') as HTMLElement | null;
    el?.scrollIntoView({ block: 'nearest' });
    if (sel >= hits.length - 5) loadMore();
  });

  // Resuming: show the query selected, so typing starts a new one.
  $effect(() => {
    if (resumed) input?.select();
  });

  function close() {
    app.searchOpen = false;
    app.searchQuery = q;
  }

  // ── from: and in: suggestions ─────────────────────────────────
  const token = $derived(/(?:^|\s)(from|in):(\S*)$/i.exec(q));

  const suggestions = $derived.by((): string[] => {
    if (!token) return [];
    const kind = token[1].toLowerCase();
    const part = token[2].toLowerCase();
    const conns = scope !== 'all' && origin ? [origin.conn] : [...app.conns.values()];
    const names = new Map<string, string>();
    const add = (n: string) => n && !names.has(n.toLowerCase()) && names.set(n.toLowerCase(), n);
    if (kind === 'in') {
      if (origin && origin.kind !== 'server') add(origin.name);
      for (const c of conns) for (const b of c.buffers.values()) add(b.name);
    } else {
      // People who spoke here recently first, then everyone else in view.
      if (origin) for (let i = origin.messages.length - 1, n = 0; i >= 0 && n < 400; i--, n++) add(origin.messages[i].nick);
      if (origin) for (const m of origin.sortedMembers) add(m.nick);
      for (const c of conns) for (const b of c.buffers.values()) if (b.kind === 'query') add(b.name);
    }
    const all = [...names.values()].filter((n) => n.toLowerCase().startsWith(part));
    return all.length === 1 && all[0].toLowerCase() === part ? [] : all.slice(0, 8);
  });

  $effect(() => {
    void suggestions;
    sugSel = 0;
  });

  function complete(name: string) {
    if (!token) return;
    q = q.slice(0, q.length - token[2].length) + name + ' ';
    input?.focus();
  }

  function addFilter(prefix: string) {
    q = (q.trim() ? q.trim() + ' ' : '') + prefix;
    tick().then(() => {
      input?.focus();
      input?.setSelectionRange(q.length, q.length);
    });
  }

  // ── Results ───────────────────────────────────────────────────
  function where(h: SearchHit): { conn: Conn | undefined; name: string } {
    const conn = app.conns.get(h.network);
    const to = h.msg.params[0] ?? h.target;
    const nick = h.msg.source?.nick ?? '';
    if (!conn) return { conn, name: to };
    const name = conn.isChannel(to) || !nick || conn.isMe(nick) ? to : nick;
    return { conn, name: conn.fold(name) === h.target ? name : (conn.buffer(h.target)?.name ?? h.target) };
  }

  function text(h: SearchHit): string {
    const raw = h.msg.params[1] ?? '';
    const ctcp = parseCtcp(raw);
    return stripFormatting(ctcp ? ctcp.args : raw);
  }

  function choose(h: SearchHit | undefined) {
    if (!h) return;
    const { conn, name } = where(h);
    if (!conn) return app.toast('That network is not connected', 'error');
    const buf = conn.buffer(name) ?? conn.ensure(name);
    last = { q, origin, scope, hits, next, sel: Math.max(0, hits.indexOf(h)) };
    close();
    app.jumpTo(buf, hitTime(h), h.msg.tags.msgid);
  }

  /** Split text into plain and matched parts for highlighting. */
  function marked(s: string): { t: string; m: boolean }[] {
    const words = parseQuery(q)
      .text.split(/\s+/)
      .map((w) => w.replace(/[^\p{L}\p{N}#@]/gu, ''))
      .filter(Boolean)
      .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
    if (!words.length) return [{ t: s, m: false }];
    const re = new RegExp(`(${words.join('|')})`, 'giu');
    return s.split(re).map((t, i) => ({ t, m: i % 2 === 1 }));
  }

  function onKey(e: KeyboardEvent) {
    if (suggestions.length) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        sugSel = (sugSel + (e.key === 'ArrowDown' ? 1 : suggestions.length - 1)) % suggestions.length;
        return;
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        complete(suggestions[sugSel]);
        return;
      }
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      sel = Math.min(sel + 1, hits.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      sel = Math.max(sel - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      choose(hits[sel]);
    }
  }

  const stamp = (t: number) => new Date(t).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });

  const FILTERS = [
    { prefix: 'from:', hint: 'nick', label: 'Messages from someone' },
    { prefix: 'in:', hint: 'channel', label: 'In one channel or conversation' },
    { prefix: 'on:', hint: '2026-10-12', label: 'On a day; also today or yesterday' },
    { prefix: 'before:', hint: 'date', label: 'Before a day' },
    { prefix: 'after:', hint: 'date', label: 'After a day' },
  ];
</script>

<div class="scrim" onclick={close} role="presentation">
  <div
    class="palette"
    onclick={(e) => e.stopPropagation()}
    role="dialog"
    aria-modal="true"
    aria-label="Search messages"
    tabindex="-1"
    onkeydown={() => {}}
    use:modal={{ onclose: close }}
  >
    <div class="head">
      <input
        bind:this={input}
        bind:value={q}
        onkeydown={onKey}
        placeholder="Search messages"
        spellcheck="false"
        role="combobox"
        aria-label="Search messages"
        aria-expanded={hits.length > 0 || suggestions.length > 0}
        aria-controls="search-results"
        aria-autocomplete="list"
        aria-activedescendant={suggestions.length ? `search-sug-${sugSel}` : hits.length ? `search-hit-${sel}` : undefined}
        data-autofocus
      />
      {#if origin}
        <Segmented label="Search in"
          bind:value={scope}
          options={[
            { value: 'all', label: 'Everywhere' },
            { value: 'network', label: origin.conn.name },
            ...(origin.kind !== 'server' ? [{ value: 'buffer' as const, label: origin.name }] : []),
          ]}
        />
      {/if}
    </div>
    <div
      class="results"
      bind:this={list}
      onscroll={onListScroll}
      id="search-results"
      role={hits.length || suggestions.length ? 'listbox' : undefined}
      aria-label={suggestions.length ? 'Suggestions' : hits.length ? 'Results' : undefined}
    >
      {#if !canSearch}
        <p class="none">
          Search uses messages saved on this computer, which is turned off.
          <button class="link" onclick={() => (app.settingsOpen = { tab: 'privacy' })}>Change in Settings</button>
        </p>
      {:else if suggestions.length}
        {#each suggestions as name, i (name)}
          <button
            class="sug"
            class:sel={i === sugSel}
            id="search-sug-{i}"
            role="option"
            aria-selected={i === sugSel}
            tabindex="-1"
            onmouseenter={() => (sugSel = i)}
            onclick={() => complete(name)}
          >
            <span class="dim">{token?.[1].toLowerCase()}:</span><b style={token?.[1].toLowerCase() === 'from' ? `--h:${nickHue(name)}` : ''} class:nick={token?.[1].toLowerCase() === 'from'}>{name}</b>
          </button>
        {/each}
      {:else if !q.trim()}
        <p class="eyebrow">Narrow it down</p>
        {#each FILTERS as f (f.prefix)}
          <button class="filter" tabindex="-1" onclick={() => addFilter(f.prefix)}>
            <span class="code">{f.prefix}<span class="dim">{f.hint}</span></span>
            <span class="dim">{f.label}</span>
          </button>
        {/each}
      {:else}
        {#each hits as h, i (h.network + '|' + h.target + '|' + (h.msg.tags.msgid ?? h.msg.tags.time) + '|' + i)}
          {@const w = where(h)}
          {@const nick = h.msg.source?.nick ?? ''}
          <button
            class="hit"
            class:sel={i === sel}
            id="search-hit-{i}"
            role="option"
            aria-selected={i === sel}
            tabindex="-1"
            disabled={!w.conn}
            onmouseenter={() => (sel = i)}
            onclick={() => choose(h)}
          >
            <div class="meta">
              <span class="buf">{w.name}</span>
              {#if w.conn}<span class="net">{w.conn.name}</span>{/if}
              <span class="time">{stamp(hitTime(h))}</span>
            </div>
            <div class="line">
              <b class="nick" style="--h:{nickHue(nick)}">{nick}</b>
              <span class="txt">{#each marked(text(h)) as part, j (j)}{#if part.m}<mark>{part.t}</mark>{:else}{part.t}{/if}{/each}</span>
            </div>
          </button>
        {:else}
          {#if !busy}<p class="none">No messages found</p>{/if}
        {/each}
        {#if more}<p class="none">Loading older messages…</p>{/if}
      {/if}
    </div>
    <footer>
      <span><span class="kbd">↑↓</span> navigate</span>
      <span><span class="kbd">↵</span> {suggestions.length ? 'complete' : 'jump to message'}</span>
      <span><span class="kbd">esc</span> close</span>
      <span class="hint" aria-live="polite">
        {busy ? 'Searching…' : hits.length ? `${hits.length}${next !== null ? '+' : ''} result${hits.length === 1 ? '' : 's'}` : ''}
      </span>
    </footer>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 80;
    background: rgb(0 0 0 / 0.28);
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
    animation: fade 120ms;
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  .palette {
    width: min(680px, 94vw);
    background: var(--bg-raised);
    border-radius: 12px;
    box-shadow: var(--shadow);
    overflow: hidden;
    animation: drop 160ms var(--ease);
  }

  @keyframes drop {
    from {
      transform: translateY(-6px);
      opacity: 0;
    }
  }

  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-right: 12px;
    border-bottom: 1px solid var(--line);
  }

  input {
    flex: 1;
    min-width: 0;
    height: 52px;
    padding: 0 18px;
    border: 0;
    outline: none;
    background: none;
    font-size: 16px;
    user-select: text;
  }

  input::placeholder {
    color: var(--text-4);
  }

  .results {
    max-height: 56vh;
    overflow-y: auto;
    padding: 6px;
  }

  .hit {
    width: 100%;
    display: grid;
    gap: 2px;
    padding: 7px 10px;
    border-radius: 7px;
    text-align: left;
  }

  .hit.sel {
    background: var(--bg-active);
  }

  .hit:disabled {
    opacity: 0.5;
  }

  .meta {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 11.5px;
    color: var(--text-3);
  }

  .buf {
    font-weight: 600;
    color: var(--text-2);
  }

  .time {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }

  .line {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 13.5px;
    min-width: 0;
  }

  .line b {
    font-weight: 600;
    flex: none;
  }

  .txt {
    color: var(--text-2);
    overflow: hidden;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .sug,
  .filter {
    width: 100%;
    display: flex;
    align-items: baseline;
    gap: 10px;
    padding: 7px 10px;
    border-radius: 7px;
    text-align: left;
    font-size: 13.5px;
  }

  .sug.sel,
  .filter:hover {
    background: var(--bg-active);
  }

  .sug b {
    font-weight: 600;
  }

  .dim {
    color: var(--text-3);
  }

  .code {
    font-family: var(--font-mono);
    font-size: 12.5px;
    min-width: 150px;
  }

  .eyebrow {
    margin: 4px 10px 6px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-3);
  }

  mark {
    background: var(--brand-soft);
    color: var(--text);
    border-radius: 2px;
  }

  .none {
    margin: 0;
    padding: 18px 12px;
    color: var(--text-3);
    font-size: 13px;
  }

  .link {
    color: var(--brand);
  }

  footer {
    display: flex;
    gap: 14px;
    padding: 8px 14px;
    border-top: 1px solid var(--line);
    font-size: 11.5px;
    color: var(--text-3);
  }

  .hint {
    margin-left: auto;
  }
</style>
