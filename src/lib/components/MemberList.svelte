<script lang="ts">
  import type { Buffer, Member } from '../state.svelte';
  import { nickHue } from '../nickcolor';
  import { app } from '../state.svelte';
  import Avatar from './Avatar.svelte';

  let { buf, onnick }: { buf: Buffer; onnick: (nick: string, e: MouseEvent) => void } = $props();

  let query = $state('');
  let cap = $state(400);

  const TITLES: Record<string, string> = {
    q: 'Owners',
    a: 'Admins',
    o: 'Operators',
    h: 'Half-operators',
    v: 'Voiced',
  };

  const groups = $derived.by(() => {
    const { modes, symbols } = buf.conn.prefix;
    const q = query.trim().toLowerCase();
    const out: { title: string; members: Member[] }[] = [];
    let current: { title: string; members: Member[] } | null = null;
    let shown = 0;
    for (const m of buf.sortedMembers) {
      if (q && !m.nick.toLowerCase().includes(q)) continue;
      if (shown >= cap) break;
      const sym = m.prefix[0] ?? '';
      const mode = sym ? modes[symbols.indexOf(sym)] : '';
      const title = sym ? (TITLES[mode] ?? `Mode ${mode}`) : 'Members';
      if (!current || current.title !== title) {
        current = { title, members: [] };
        out.push(current);
      }
      current.members.push(m);
      shown++;
    }
    return out;
  });

  const counts = $derived.by(() => {
    const c: Record<string, number> = {};
    const { modes, symbols } = buf.conn.prefix;
    for (const m of buf.members.values()) {
      const sym = m.prefix[0] ?? '';
      const t = sym ? (TITLES[modes[symbols.indexOf(sym)]] ?? `Mode ${modes[symbols.indexOf(sym)]}`) : 'Members';
      c[t] = (c[t] ?? 0) + 1;
    }
    return c;
  });
</script>

<aside>
  {#if buf.members.size > 30}
    <div class="search">
      <input class="input" bind:value={query} placeholder="Find member" />
    </div>
  {/if}
  <div class="list" onscroll={(e) => {
    const el = e.currentTarget;
    if (el.scrollTop + el.clientHeight > el.scrollHeight - 200 && cap < buf.members.size) cap += 400;
  }}>
    {#each groups as g (g.title)}
      <h3 class="eyebrow">{g.title} <span>{counts[g.title] ?? g.members.length}</span></h3>
      {#each g.members as m (m.nick)}
        <button
          class="member"
          class:away={m.away}
          onclick={(e) => onnick(m.nick, e)}
          title={m.user ? `${m.nick}!${m.user}@${m.host}${m.account ? ` · ${m.account}` : ''}` : m.nick}
        >
          <span class="prefix">{m.prefix[0] ?? ''}</span>
          {#if app.settings.loadAvatars}<Avatar conn={buf.conn} nick={m.nick} />{/if}
          <span class="nick" style="--h:{nickHue(m.nick)}">{m.nick}</span>
        </button>
      {/each}
    {/each}
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
    padding: 4px 8px 16px;
  }

  h3 {
    margin: 14px 8px 4px;
    display: flex;
    justify-content: space-between;
  }

  h3 span {
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
