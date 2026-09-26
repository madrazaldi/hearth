<script lang="ts">
  import { onMount } from 'svelte';
  import { app, type Conn, type ChannelInfo } from '../state.svelte';
  import { stripFormatting } from '../format';
  import Segmented from './Segmented.svelte';
  import Icon from './Icon.svelte';

  let { conn }: { conn: Conn } = $props();

  const SHOWN = 300;
  let q = $state('');
  let sort = $state<'users' | 'name'>('users');
  let input = $state<HTMLInputElement>();

  const list = $derived(conn.channelList);

  const filtered = $derived.by(() => {
    const items = list?.items ?? [];
    const needle = q.trim().toLowerCase();
    const match = needle
      ? items.filter((c) => c.name.toLowerCase().includes(needle) || stripFormatting(c.topic).toLowerCase().includes(needle))
      : items;
    const sorted = [...match];
    if (sort === 'users') sorted.sort((a, b) => b.users - a.users || a.name.localeCompare(b.name));
    else sorted.sort((a, b) => conn.fold(a.name).localeCompare(conn.fold(b.name)));
    return sorted;
  });

  function close() {
    app.browserConn = null;
  }

  function join(c: ChannelInfo) {
    const existing = conn.buffer(c.name);
    if (existing?.joined) app.open(existing);
    else app.join(conn, c.name);
    close();
  }

  function refresh() {
    conn.channelList = null;
    app.browseChannels(conn);
  }

  onMount(() => input?.focus());
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && close()} />

<div class="scrim" onclick={close} role="presentation">
  <div class="sheet" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Channels" tabindex="-1" onkeydown={() => {}}>
    <header>
      <div>
        <h2>Channels on {conn.name}</h2>
        <p>
          {#if list?.loading}
            Loading… {list.items.length ? `${list.items.length.toLocaleString()} so far` : ''}
          {:else if list}
            {list.items.length.toLocaleString()} channels{q ? ` · ${filtered.length.toLocaleString()} match` : ''}
          {:else}
            Not connected
          {/if}
        </p>
      </div>
      <button class="icon-btn" title="Refresh" onclick={refresh} disabled={list?.loading}><Icon name="refresh" /></button>
      <button class="icon-btn" title="Close" onclick={close}><Icon name="x" /></button>
    </header>
    <div class="tools">
      <input class="input" bind:this={input} bind:value={q} placeholder="Filter by name or topic" spellcheck="false" />
      <Segmented
        bind:value={sort}
        options={[
          { value: 'users', label: 'Most users' },
          { value: 'name', label: 'Name' },
        ]}
      />
    </div>
    <div class="rows">
      {#each filtered.slice(0, SHOWN) as c (c.name)}
        {@const joined = conn.buffer(c.name)?.joined}
        <div class="row">
          <div class="main">
            <div class="name">{c.name}</div>
            {#if c.topic}<div class="topic">{stripFormatting(c.topic)}</div>{/if}
          </div>
          <span class="users" title="Users">{c.users.toLocaleString()}</span>
          <button class="btn" class:primary={!joined} onclick={() => join(c)}>{joined ? 'Open' : 'Join'}</button>
        </div>
      {:else}
        {#if !list?.loading}<p class="none">{q ? 'No channels match' : 'No channels listed'}</p>{/if}
      {/each}
      {#if filtered.length > SHOWN}
        <p class="none">Showing {SHOWN} of {filtered.length.toLocaleString()}. Narrow the filter to see more.</p>
      {/if}
    </div>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 85;
    background: rgb(0 0 0 / 0.35);
    display: grid;
    place-items: center;
    animation: fade 120ms;
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  .sheet {
    width: min(760px, 94vw);
    height: min(640px, 88vh);
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border-radius: 14px;
    box-shadow: var(--shadow);
    overflow: hidden;
  }

  header {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 18px 14px 10px 22px;
  }

  header > div {
    flex: 1;
  }

  h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  header p {
    margin: 2px 0 0;
    font-size: 12.5px;
    color: var(--text-3);
  }

  .tools {
    display: flex;
    gap: 10px;
    padding: 0 22px 12px;
    border-bottom: 1px solid var(--line);
  }

  .tools .input {
    flex: 1;
  }

  .rows {
    flex: 1;
    overflow-y: auto;
    padding: 6px 12px 16px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 8px 10px;
    border-radius: 8px;
  }

  .row:hover {
    background: var(--bg-hover);
  }

  .main {
    flex: 1;
    min-width: 0;
  }

  .name {
    font-weight: 600;
    font-size: 13.5px;
  }

  .topic {
    font-size: 12.5px;
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .users {
    font-size: 12px;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
    min-width: 48px;
    text-align: right;
  }

  .none {
    color: var(--text-3);
    font-size: 13px;
    padding: 12px 10px;
    margin: 0;
  }
</style>
