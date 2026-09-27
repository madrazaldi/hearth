<script lang="ts">
  import { app, type Buffer, type Conn } from '../state.svelte';
  import { parseCtcp } from '../irc';
  import { stripFormatting } from '../format';
  import { nickHue } from '../nickcolor';
  import type { SearchHit } from '../types';
  import Segmented from './Segmented.svelte';
  import { modal } from '../a11y';

  const origin = app.active;
  let q = $state(app.searchQuery);
  let scope = $state<'all' | 'network' | 'buffer'>('all');
  let hits = $state<SearchHit[]>([]);
  let busy = $state(false);
  let sel = $state(0);
  let list = $state<HTMLDivElement>();
  let run = 0;

  const remoteAvailable = $derived([...app.conns.values()].some((c) => c.has('soju.im/search')));
  const canSearch = $derived(app.settings.storeLogs || remoteAvailable);

  $effect(() => {
    const query = q;
    const sc = scope;
    const id = ++run;
    if (!query.trim()) {
      hits = [];
      busy = false;
      return;
    }
    busy = true;
    const t = setTimeout(async () => {
      const where: { conn?: Conn; buf?: Buffer } =
        sc === 'buffer' && origin ? { conn: origin.conn, buf: origin } : sc === 'network' && origin ? { conn: origin.conn } : {};
      const res = await app.search(query, where);
      if (id !== run) return;
      hits = res;
      sel = 0;
      busy = false;
    }, 180);
    return () => clearTimeout(t);
  });

  $effect(() => {
    void sel;
    const el = list?.querySelector('.sel') as HTMLElement | null;
    el?.scrollIntoView({ block: 'nearest' });
  });

  function close() {
    app.searchOpen = false;
    app.searchQuery = '';
  }

  function where(h: SearchHit): { conn: Conn | undefined; name: string } {
    const conn = app.conns.get(h.network);
    const to = h.msg.params[0] ?? h.target;
    const nick = h.msg.source?.nick ?? '';
    if (!conn) return { conn, name: to };
    const name = conn.isChannel(to) || !nick || conn.isMe(nick) ? to : nick;
    return { conn, name: conn.fold(name) === h.target ? name : h.target };
  }

  function text(h: SearchHit): string {
    const raw = h.msg.params[1] ?? '';
    const ctcp = parseCtcp(raw);
    return stripFormatting(ctcp ? ctcp.args : raw);
  }

  function time(h: SearchHit): number {
    return Date.parse(h.msg.tags.time ?? '') || 0;
  }

  function choose(h: SearchHit | undefined) {
    if (!h) return;
    const { conn, name } = where(h);
    if (!conn) return app.toast('That network is not connected', 'error');
    const buf = conn.buffer(name) ?? conn.ensure(name);
    close();
    app.jumpTo(buf, time(h), h.msg.tags.msgid);
  }

  /** Split text into plain and matched parts for highlighting. */
  function marked(s: string): { t: string; m: boolean }[] {
    const words = q
      .trim()
      .split(/\s+/)
      .map((w) => w.replace(/[^\p{L}\p{N}#@]/gu, ''))
      .filter(Boolean)
      .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
    if (!words.length) return [{ t: s, m: false }];
    const re = new RegExp(`(${words.join('|')})`, 'giu');
    return s.split(re).map((t, i) => ({ t, m: i % 2 === 1 }));
  }

  function onKey(e: KeyboardEvent) {
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
        bind:value={q}
        onkeydown={onKey}
        placeholder="Search messages"
        spellcheck="false"
        role="combobox"
        aria-label="Search messages"
        aria-expanded={hits.length > 0}
        aria-controls="search-results"
        aria-autocomplete="list"
        aria-activedescendant={hits.length ? `search-hit-${sel}` : undefined}
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
    <div class="results" bind:this={list} id="search-results" role={hits.length ? 'listbox' : undefined} aria-label={hits.length ? 'Results' : undefined}>
      {#if !canSearch}
        <p class="none">
          Search uses messages saved on this computer, which is turned off.
          <button class="link" onclick={() => (app.settingsOpen = { tab: 'privacy' })}>Change in Settings</button>
        </p>
      {:else}
        {#each hits as h, i (h.network + (h.msg.tags.msgid ?? i))}
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
              <span class="time">{stamp(time(h))}</span>
            </div>
            <div class="line">
              <b class="nick" style="--h:{nickHue(nick)}">{nick}</b>
              <span class="txt">{#each marked(text(h)) as part, j (j)}{#if part.m}<mark>{part.t}</mark>{:else}{part.t}{/if}{/each}</span>
            </div>
          </button>
        {:else}
          {#if q.trim() && !busy}<p class="none">No messages found</p>{/if}
        {/each}
      {/if}
    </div>
    <footer>
      <span><span class="kbd">↑↓</span> navigate</span>
      <span><span class="kbd">↵</span> jump to message</span>
      <span><span class="kbd">esc</span> close</span>
      <span class="hint" aria-live="polite">{busy ? 'Searching…' : hits.length ? `${hits.length} result${hits.length === 1 ? '' : 's'}` : ''}</span>
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
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  mark {
    background: var(--accent-soft);
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
    color: var(--accent);
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
