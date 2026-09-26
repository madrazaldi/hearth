<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { formatFull } from '../time';
  import Icon from './Icon.svelte';
  import RichText from './RichText.svelte';
  import MessageList from './MessageList.svelte';
  import Composer from './Composer.svelte';
  import MemberList from './MemberList.svelte';
  import NickMenu from './NickMenu.svelte';

  let { buf }: { buf: Buffer } = $props();

  let topicOpen = $state(false);
  let filter = $state('');
  let filterOpen = $state(false);
  let filterEl = $state<HTMLInputElement>();
  let nickMenu = $state<{ nick: string; x: number; y: number } | null>(null);

  const conn = $derived(buf.conn);
  const showMembers = $derived(app.settings.showMembers && buf.kind === 'channel');
  const typing = $derived([...buf.typing.keys()]);
  const title = $derived(buf.kind === 'server' ? conn.name : buf.name);

  const subtitle = $derived.by(() => {
    if (buf.kind === 'server') {
      const s = conn.config;
      if (conn.isControl) return `Bouncer · ${conn.networks.size} network${conn.networks.size === 1 ? '' : 's'}`;
      if (conn.netid) {
        const attrs = conn.parentId ? app.conns.get(conn.parentId)?.networks.get(conn.netid) : undefined;
        return attrs?.host ? `${attrs.host}${attrs.port ? ':' + attrs.port : ''} via bouncer` : 'via bouncer';
      }
      return s ? `${s.host}:${s.port}${s.tls ? ' · TLS' : ''}` : '';
    }
    if (buf.kind === 'query') {
      const m = buf.member(buf.name);
      return m?.user ? `${m.user}@${m.host}` : '';
    }
    return '';
  });

  function openNick(nick: string, e: MouseEvent) {
    e.stopPropagation();
    nickMenu = { nick, x: e.clientX, y: e.clientY };
  }

  function toggleFilter() {
    filterOpen = !filterOpen;
    if (!filterOpen) filter = '';
    else queueMicrotask(() => filterEl?.focus());
  }

  function onKey(e: KeyboardEvent) {
    const mod = navigator.userAgent.includes('Mac') ? e.metaKey : e.ctrlKey;
    if (mod && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      if (!filterOpen) toggleFilter();
      else filterEl?.select();
    }
  }
</script>

<svelte:window onkeydown={onKey} onclick={() => (nickMenu = null)} />

<div class="view" class:with-members={showMembers}>
  <header data-tauri-drag-region>
    <div class="title" data-tauri-drag-region>
      <h1>
        {#if buf.kind === 'channel'}<span class="hash">#</span>{buf.name.replace(/^#/, '')}{:else}{title}{/if}
      </h1>
      {#if buf.kind === 'channel'}
        {#if !buf.joined}
          <span class="pill">not joined</span>
        {:else if buf.members.size}
          <span class="count">{buf.members.size.toLocaleString()}</span>
        {/if}
      {/if}
      {#if buf.kind === 'channel' && buf.topic}
        <button class="topic" onclick={() => (topicOpen = !topicOpen)} title="Show full topic">
          <RichText text={buf.topic} plain />
        </button>
      {:else if subtitle}
        <span class="topic static">{subtitle}</span>
      {/if}
    </div>
    <div class="actions">
      {#if filterOpen}
        <div class="filter">
          <Icon name="search" size={13} />
          <input
            bind:this={filterEl}
            bind:value={filter}
            placeholder="Filter messages"
            onkeydown={(e) => e.key === 'Escape' && toggleFilter()}
          />
        </div>
      {:else}
        <button class="icon-btn" title="Filter messages" onclick={toggleFilter}><Icon name="search" /></button>
      {/if}
      {#if buf.kind === 'channel'}
        <button
          class="icon-btn"
          class:on={app.settings.showMembers}
          title="Members"
          onclick={() => app.updateSettings({ showMembers: !app.settings.showMembers })}
        >
          <Icon name="users" />
        </button>
      {/if}
    </div>
  </header>

  {#if topicOpen && buf.topic}
    <div class="topic-full selectable">
      <RichText text={buf.topic} />
      {#if buf.topicBy}
        <p class="topic-meta">Set by {buf.topicBy}{buf.topicTime ? ` · ${formatFull(buf.topicTime)}` : ''}</p>
      {/if}
    </div>
  {/if}

  <div class="body">
    <div class="center">
      {#if conn.status !== 'connected' && buf.kind !== 'server'}
        <div class="banner">
          {#if conn.status === 'disconnected'}
            Disconnected{conn.error ? ` — ${conn.error}` : ''}.
          {:else}
            Connecting to {conn.name}…
          {/if}
        </div>
      {/if}
      <MessageList {buf} {filter} onnick={openNick} />
      <div class="typing" class:on={typing.length > 0}>
        {#if typing.length}
          <span class="dots"><i></i><i></i><i></i></span>
          <span>
            {#if typing.length === 1}<b>{typing[0]}</b> is typing
            {:else if typing.length === 2}<b>{typing[0]}</b> and <b>{typing[1]}</b> are typing
            {:else}{typing.length} people are typing{/if}
          </span>
        {/if}
      </div>
      <Composer {buf} />
    </div>
    {#if showMembers}
      <MemberList {buf} onnick={openNick} />
    {/if}
  </div>
</div>

{#if nickMenu}
  <NickMenu {buf} nick={nickMenu.nick} x={nickMenu.x} y={nickMenu.y} onclose={() => (nickMenu = null)} />
{/if}

<style>
  .view {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  header {
    height: 52px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 12px 0 20px;
    border-bottom: 1px solid var(--line);
  }

  :global(html.mac) header {
    height: 52px;
  }

  .title {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: 10px;
  }

  h1 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    letter-spacing: -0.005em;
    white-space: nowrap;
  }

  .hash {
    color: var(--text-4);
    font-weight: 400;
    margin-right: 1px;
  }

  .count {
    font-size: 12px;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }

  .pill {
    font-size: 11px;
    color: var(--warn);
  }

  .topic {
    min-width: 0;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    color: var(--text-3);
    text-align: left;
    padding-left: 12px;
    border-left: 1px solid var(--line-strong);
  }

  .topic:hover {
    color: var(--text-2);
  }

  .topic.static {
    border-left: 0;
    padding-left: 0;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .filter {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    border-radius: 6px;
    background: var(--bg-sunken);
    box-shadow: 0 0 0 1px var(--line) inset;
    color: var(--text-3);
  }

  .filter input {
    width: 180px;
    border: 0;
    outline: none;
    background: none;
    font-size: 13px;
    user-select: text;
  }

  .topic-full {
    padding: 12px 20px 14px;
    border-bottom: 1px solid var(--line);
    font-size: 13.5px;
    color: var(--text-2);
    background: var(--bg-side);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .topic-meta {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--text-3);
  }

  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .center {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    position: relative;
  }

  .banner {
    padding: 7px 20px;
    font-size: 12.5px;
    color: var(--warn);
    background: color-mix(in oklab, var(--warn) 8%, transparent);
    border-bottom: 1px solid var(--line);
  }

  .typing {
    height: 20px;
    padding: 0 22px;
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    color: var(--text-3);
    opacity: 0;
    transition: opacity 150ms;
  }

  .typing.on {
    opacity: 1;
  }

  .typing b {
    font-weight: 500;
    color: var(--text-2);
  }

  .dots {
    display: flex;
    gap: 3px;
  }

  .dots i {
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--text-3);
    animation: bob 1.2s infinite;
  }

  .dots i:nth-child(2) {
    animation-delay: 0.15s;
  }
  .dots i:nth-child(3) {
    animation-delay: 0.3s;
  }

  @keyframes bob {
    0%,
    60%,
    100% {
      transform: none;
      opacity: 0.4;
    }
    30% {
      transform: translateY(-2px);
      opacity: 1;
    }
  }
</style>
