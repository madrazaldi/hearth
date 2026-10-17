<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { bufferLabel, modal } from '../a11y';

  let q = $state('');
  let sel = $state(0);
  let list = $state<HTMLDivElement>();

  function score(name: string, query: string): number {
    if (!query) return 1;
    const n = name.toLowerCase();
    const s = query.toLowerCase();
    if (n === s) return 1000;
    if (n.replace(/^#+/, '').startsWith(s.replace(/^#+/, ''))) return 500 - n.length;
    const i = n.indexOf(s);
    if (i >= 0) return 300 - i;
    // Subsequence match, rewarding tight runs.
    let pos = 0;
    let gaps = 0;
    for (const ch of s) {
      const at = n.indexOf(ch, pos);
      if (at < 0) return 0;
      gaps += at - pos;
      pos = at + 1;
    }
    return 100 - gaps;
  }

  type Result = { buf: Buffer; score: number } | { join: string; score: number };

  const results = $derived.by(() => {
    const out: Result[] = [];
    for (const b of app.allBuffers) {
      const label = b.kind === 'server' ? b.conn.name : b.name;
      const sc = score(label, q.trim());
      if (sc > 0) out.push({ buf: b, score: sc + (b.highlights ? 50 : 0) + (b.unread ? 10 : 0) + (b === app.active ? -40 : 0) });
    }
    out.sort((a, b) => b.score - a.score);
    const t = q.trim();
    const conn = app.active?.conn;
    if (conn && t && conn.isChannel(t) && !conn.buffer(t)) out.push({ join: t, score: 0 });
    return out.slice(0, 40);
  });

  $effect(() => {
    void q;
    sel = 0;
  });

  $effect(() => {
    const el = list?.querySelector('.sel') as HTMLElement | null;
    if (!el || !list) return;
    if (el.offsetTop < list.scrollTop) list.scrollTop = el.offsetTop;
    else if (el.offsetTop + el.offsetHeight > list.scrollTop + list.clientHeight)
      list.scrollTop = el.offsetTop + el.offsetHeight - list.clientHeight;
  });

  function label(b: Buffer): string {
    const name = b.kind === 'server' ? b.conn.name : b.name;
    const where = b.kind === 'server' ? (b.conn.isControl ? 'bouncer' : 'network') : b.conn.name;
    return `${bufferLabel({ name, kind: b.kind, unread: b.unread, highlights: b.highlights, muted: b.level === 'none' })}, ${where}`;
  }

  function choose(r: Result | undefined) {
    if (!r) return;
    if ('buf' in r) app.open(r.buf);
    else if (app.active) app.join(app.active.conn, r.join);
    app.switcherOpen = false;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'ArrowDown' || (e.key === 'n' && e.ctrlKey)) {
      e.preventDefault();
      sel = Math.min(sel + 1, results.length - 1);
    } else if (e.key === 'ArrowUp' || (e.key === 'p' && e.ctrlKey)) {
      e.preventDefault();
      sel = Math.max(sel - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      choose(results[sel]);
    }
  }
</script>

<div class="scrim" onclick={() => (app.switcherOpen = false)} role="presentation">
  <div
    class="palette"
    onclick={(e) => e.stopPropagation()}
    role="dialog"
    aria-modal="true"
    aria-label="Jump to"
    tabindex="-1"
    onkeydown={() => {}}
    use:modal={{ onclose: () => (app.switcherOpen = false) }}
  >
    <input
      bind:value={q}
      onkeydown={onKey}
      placeholder="Jump to a channel, conversation or network"
      spellcheck="false"
      role="combobox"
      aria-label="Jump to"
      aria-expanded={results.length > 0}
      aria-controls="switcher-results"
      aria-autocomplete="list"
      aria-activedescendant={results.length ? `switcher-${sel}` : undefined}
      data-autofocus
    />
    <div class="results" bind:this={list} id="switcher-results" role={results.length ? 'listbox' : undefined} aria-label={results.length ? 'Matches' : undefined}>
      {#each results as r, i ('buf' in r ? r.buf.id : 'join' + r.join)}
        <button
          class="result"
          class:sel={i === sel}
          id="switcher-{i}"
          role="option"
          aria-selected={i === sel}
          aria-label={'buf' in r ? label(r.buf) : undefined}
          tabindex="-1"
          onmouseenter={() => (sel = i)}
          onclick={() => choose(r)}
        >
          {#if 'buf' in r}
            {@const b = r.buf}
            <span class="glyph">{b.kind === 'channel' ? '#' : b.kind === 'query' ? '@' : '◆'}</span>
            <span class="name" class:unread={b.unread > 0}>
              {b.kind === 'server' ? b.conn.name : b.kind === 'channel' ? b.name.replace(/^#/, '') : b.name}
            </span>
            {#if b.highlights}<span class="badge">{b.highlights}</span>{/if}
            <span class="net">{b.kind === 'server' ? (b.conn.isControl ? 'bouncer' : 'network') : b.conn.name}</span>
          {:else}
            <span class="glyph">+</span>
            <span class="name">Join {r.join}</span>
            <span class="net">{app.active?.conn.name}</span>
          {/if}
        </button>
      {:else}
        <p class="none">No matches</p>
      {/each}
    </div>
    <footer>
      <span><span class="kbd">↑↓</span> navigate</span>
      <span><span class="kbd">↵</span> open</span>
      <span><span class="kbd">esc</span> close</span>
      <span class="hint">Tip: type a #channel to join it</span>
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
    padding-top: 16vh;
    animation: fade 120ms;
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  .palette {
    width: min(560px, 92vw);
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

  input {
    width: 100%;
    height: 52px;
    padding: 0 18px;
    border: 0;
    outline: none;
    background: none;
    font-size: 16px;
    border-bottom: 1px solid var(--line);
    user-select: text;
  }

  input::placeholder {
    color: var(--text-4);
  }

  .results {
    position: relative;
    max-height: 50vh;
    overflow-y: auto;
    padding: 6px;
  }

  .result {
    width: 100%;
    height: 34px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px;
    border-radius: 7px;
    text-align: left;
  }

  .result.sel {
    background: var(--bg-active);
  }

  .glyph {
    width: 14px;
    text-align: center;
    font-family: var(--font-mono);
    color: var(--text-4);
    font-size: 12px;
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-2);
  }

  .name.unread,
  .sel .name {
    color: var(--text);
  }

  .net {
    font-size: 12px;
    color: var(--text-3);
  }

  .badge {
    min-width: 18px;
    height: 17px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--brand);
    color: var(--brand-text);
    font-size: 11px;
    font-weight: 600;
    display: grid;
    place-items: center;
  }

  .none {
    margin: 0;
    padding: 18px 12px;
    color: var(--text-3);
    font-size: 13px;
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
