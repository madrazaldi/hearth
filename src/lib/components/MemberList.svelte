<script lang="ts">
  import { tick } from 'svelte';
  import type { Buffer, Member } from '../state.svelte';
  import { nickHue } from '../nickcolor';
  import { app } from '../state.svelte';
  import { memberLabel, stepIndex } from '../a11y';
  import { offsets, rangeFor } from '../virtual';
  import Avatar from './Avatar.svelte';

  let { buf, onnick }: { buf: Buffer; onnick: (nick: string, e: MouseEvent) => void } = $props();

  // Fixed row heights, so the list can be virtualized without measuring.
  const HEAD_H = 36;
  const ROW_H = 26;
  const OVERSCAN = 400;

  let query = $state('');
  let el = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let viewH = $state(600);
  /** The member with the roving tabindex, by nick. */
  let focusNick = $state<string | null>(null);

  const TITLES: Record<string, string> = {
    q: 'Owners',
    a: 'Admins',
    o: 'Operators',
    h: 'Half-operators',
    v: 'Voiced',
  };

  type Row = { kind: 'head'; title: string } | { kind: 'member'; m: Member; pos: number };

  const titleOf = (m: Member) => {
    const { modes, symbols } = buf.conn.prefix;
    const sym = m.prefix[0] ?? '';
    const mode = sym ? modes[symbols.indexOf(sym)] : '';
    return sym ? (TITLES[mode] ?? `Mode ${mode}`) : 'Members';
  };

  /** Group headers and members in display order, plus where each member row is. */
  const model = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const rows: Row[] = [];
    const members: number[] = [];
    let title = '';
    for (const m of buf.sortedMembers) {
      if (q && !m.nick.toLowerCase().includes(q)) continue;
      const t = titleOf(m);
      if (t !== title) {
        rows.push({ kind: 'head', title: t });
        title = t;
      }
      members.push(rows.length);
      rows.push({ kind: 'member', m, pos: members.length });
    }
    return { rows, members };
  });

  const counts = $derived.by(() => {
    const c: Record<string, number> = {};
    for (const m of buf.members.values()) {
      const t = titleOf(m);
      c[t] = (c[t] ?? 0) + 1;
    }
    return c;
  });

  const offs = $derived(offsets(model.rows.length, (i) => (model.rows[i].kind === 'head' ? HEAD_H : ROW_H)));
  const range = $derived(rangeFor(offs, scrollTop - OVERSCAN, scrollTop + viewH + OVERSCAN));

  const focusRow = $derived.by(() => {
    const { rows, members } = model;
    if (focusNick) for (const i of members) if ((rows[i] as { m: Member }).m.nick === focusNick) return i;
    return members[0] ?? -1;
  });

  /** Rows to render: what's in view, and the focused one wherever it is so focus survives scrolling. */
  const shown = $derived.by(() => {
    const out: number[] = [];
    for (let i = range[0]; i < range[1]; i++) out.push(i);
    if (focusRow >= 0 && (focusRow < range[0] || focusRow >= range[1])) out.push(focusRow);
    return out;
  });

  const rowKey = (i: number) => {
    const r = model.rows[i];
    return r.kind === 'head' ? 'h' + r.title : 'm' + r.m.nick;
  };

  $effect(() => {
    if (!el) return;
    const ro = new ResizeObserver(() => el && (viewH = el.clientHeight));
    ro.observe(el);
    return () => ro.disconnect();
  });

  async function focusMember(row: number) {
    const r = model.rows[row];
    if (!el || r?.kind !== 'member') return;
    focusNick = r.m.nick;
    // Scroll just enough to show it.
    if (offs[row] < el.scrollTop) el.scrollTop = offs[row] - (row > 0 && model.rows[row - 1].kind === 'head' ? HEAD_H : 0);
    else if (offs[row + 1] > el.scrollTop + el.clientHeight) el.scrollTop = offs[row + 1] - el.clientHeight;
    await tick();
    el.querySelector<HTMLElement>(`[data-nick="${CSS.escape(r.m.nick)}"]`)?.focus({ preventScroll: true });
  }

  function onKey(e: KeyboardEvent) {
    const { members } = model;
    const at = members.indexOf(focusRow);
    const to = stepIndex(e.key, at, members.length, Math.max(1, Math.floor(viewH / ROW_H) - 1));
    if (to === null) return;
    e.preventDefault();
    focusMember(members[to]);
  }
</script>

<aside aria-label="Members">
  {#if buf.members.size > 30}
    <div class="search">
      <input class="input" bind:value={query} placeholder="Find member" aria-label="Find member" />
    </div>
  {/if}
  <div
    class="list"
    bind:this={el}
    onscroll={() => el && (scrollTop = el.scrollTop)}
    role="list"
    aria-label="{model.members.length.toLocaleString()} members"
  >
    <div class="canvas" style="height:{offs[model.rows.length]}px">
      {#each shown as i (rowKey(i))}
        {@const r = model.rows[i]}
        {#if r.kind === 'head'}
          <div class="head eyebrow" style="top:{offs[i]}px" aria-hidden="true">{r.title} <span>{counts[r.title] ?? 0}</span></div>
        {:else}
          {@const m = r.m}
          <div class="row" style="top:{offs[i]}px" role="listitem" aria-setsize={model.members.length} aria-posinset={r.pos}>
            <button
              class="member focus-inset"
              class:away={m.away}
              data-nick={m.nick}
              tabindex={i === focusRow ? 0 : -1}
              aria-label={memberLabel(m, buf.conn.prefix, app.settings.loadAvatars ? buf.conn.profile(m.nick).displayName : undefined)}
              onfocus={() => (focusNick = m.nick)}
              onkeydown={onKey}
              onclick={(e) => onnick(m.nick, e)}
              title={m.user ? `${m.nick}!${m.user}@${m.host}${m.account ? ` · ${m.account}` : ''}` : m.nick}
            >
              <span class="prefix">{m.prefix[0] ?? ''}</span>
              {#if app.settings.loadAvatars}<Avatar conn={buf.conn} nick={m.nick} />{/if}
              <span class="nick" style="--h:{nickHue(m.nick)}">{m.nick}</span>
            </button>
          </div>
        {/if}
      {/each}
    </div>
    {#if !buf.members.size}
      <p class="empty">{buf.joined ? 'Loading members…' : 'Not joined'}</p>
    {/if}
  </div>
</aside>

<style>
  aside {
    width: var(--members-w);
    flex: none;
    border-left: 1px solid var(--line);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .search {
    padding: 10px 10px 4px;
  }

  .search .input {
    height: 28px;
    font-size: 12.5px;
  }

  .list {
    flex: 1;
    overflow-y: auto;
    padding: 0 8px 16px;
  }

  .canvas {
    position: relative;
  }

  .head,
  .row {
    position: absolute;
    left: 0;
    right: 0;
  }

  .head {
    height: 36px;
    margin: 0;
    padding: 0 8px 4px;
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
  }

  .head span {
    font-weight: 500;
    color: var(--text-4);
    font-variant-numeric: tabular-nums;
  }

  .member {
    width: 100%;
    height: 26px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 8px 0 4px;
    border-radius: 6px;
    font-size: 13px;
    text-align: left;
  }

  .member:hover {
    background: var(--bg-hover);
  }

  .prefix {
    width: 12px;
    flex: none;
    text-align: center;
    font-family: var(--font-mono);
    font-size: 11.5px;
    color: var(--text-3);
  }

  .nick {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .away .nick,
  .away :global(.avatar) {
    opacity: 0.42;
  }

  .member :global(.avatar) {
    margin-right: 3px;
  }

  .empty {
    color: var(--text-3);
    font-size: 12.5px;
    padding: 8px;
  }
</style>
