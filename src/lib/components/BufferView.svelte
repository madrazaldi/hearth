<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { formatFull } from '../time';
  import Icon from './Icon.svelte';
  import RichText from './RichText.svelte';
  import MessageList from './MessageList.svelte';
  import Composer from './Composer.svelte';
  import MemberList from './MemberList.svelte';
  import NickMenu from './NickMenu.svelte';
  import { Button } from '$lib/components/ui/button';
  import { Badge } from '$lib/components/ui/badge';
  import { Kbd } from '$lib/components/ui/kbd';
  import * as Tooltip from '$lib/components/ui/tooltip';
  import * as InputGroup from '$lib/components/ui/input-group';

  let { buf }: { buf: Buffer } = $props();

  let topicOpen = $state(false);
  let filter = $state('');
  let filterOpen = $state(false);
  let filterEl = $state<HTMLInputElement | null>(null);
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
    // From the keyboard there's no pointer position; open it by the element.
    const r = e.detail === 0 && e.currentTarget instanceof Element ? e.currentTarget.getBoundingClientRect() : null;
    nickMenu = { nick, x: r ? r.left : e.clientX, y: r ? r.bottom : e.clientY };
  }

  function toggleFilter() {
    filterOpen = !filterOpen;
    if (!filterOpen) filter = '';
    else queueMicrotask(() => filterEl?.focus());
  }

  function onKey(e: KeyboardEvent) {
    const mod = navigator.userAgent.includes('Mac') ? e.metaKey : e.ctrlKey;
    if (mod && !e.shiftKey && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      if (!filterOpen) toggleFilter();
      else filterEl?.select();
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="flex h-full min-h-0 flex-col">
  <header class="flex h-12 flex-none items-center gap-3 border-b pr-3 pl-5" data-drag-region>
    <div class="flex min-w-0 flex-1 items-center gap-2.5" data-drag-region>
      <h1 class="m-0 text-[15px] font-semibold whitespace-nowrap">
        {#if buf.kind === 'channel'}<span class="mr-px font-normal text-muted-foreground">#</span>{buf.name.replace(/^#/, '')}{:else}{title}{/if}
      </h1>
      {#if buf.kind === 'channel'}
        {#if !buf.joined}
          <Badge variant="outline" class="text-warn">not joined</Badge>
        {:else if buf.members.size}
          <Badge variant="secondary" class="tabular-nums">{buf.members.size.toLocaleString()}</Badge>
        {/if}
      {/if}
      {#if buf.kind === 'channel' && buf.topic}
        <button
          class="min-w-0 flex-1 truncate border-l pl-3 text-left text-[13px] text-muted-foreground hover:text-foreground"
          onclick={() => (topicOpen = !topicOpen)}
          title="Show full topic"
          aria-expanded={topicOpen}
        >
          <RichText text={buf.topic} plain />
        </button>
      {:else if subtitle}
        <span class="min-w-0 flex-1 truncate text-[13px] text-muted-foreground" data-drag-region>{subtitle}</span>
      {/if}
    </div>
    <div class="flex items-center gap-0.5">
      {#if filterOpen}
        <InputGroup.Root class="h-7 w-56">
          <InputGroup.Addon><Icon name="search" size={13} /></InputGroup.Addon>
          <InputGroup.Input
            bind:ref={filterEl}
            bind:value={filter}
            placeholder="Filter messages"
            aria-label="Filter messages"
            class="h-7 text-[13px] md:text-[13px]"
            onkeydown={(e) => e.key === 'Escape' && toggleFilter()}
          />
        </InputGroup.Root>
      {:else}
        <Tooltip.Root>
          <Tooltip.Trigger>
            {#snippet child({ props })}
              <Button {...props} variant="ghost" size="icon-sm" class="text-muted-foreground" aria-label="Filter messages" onclick={toggleFilter}>
                <Icon name="search" />
              </Button>
            {/snippet}
          </Tooltip.Trigger>
          <Tooltip.Content side="bottom">Filter messages <Kbd>{navigator.userAgent.includes('Mac') ? '⌘' : 'Ctrl'}F</Kbd></Tooltip.Content>
        </Tooltip.Root>
      {/if}
      {#if buf.kind === 'channel'}
        <Tooltip.Root>
          <Tooltip.Trigger>
            {#snippet child({ props })}
              <Button
                {...props}
                variant="ghost"
                size="icon-sm"
                class={app.settings.showMembers ? 'bg-muted text-foreground' : 'text-muted-foreground'}
                aria-label="Members"
                aria-pressed={app.settings.showMembers}
                onclick={() => app.updateSettings({ showMembers: !app.settings.showMembers })}
              >
                <Icon name="users" />
              </Button>
            {/snippet}
          </Tooltip.Trigger>
          <Tooltip.Content side="bottom">Members</Tooltip.Content>
        </Tooltip.Root>
      {/if}
    </div>
  </header>

  {#if topicOpen && buf.topic}
    <div class="selectable border-b bg-sidebar px-5 pt-3 pb-3.5 text-[13.5px] break-words whitespace-pre-wrap text-foreground/85">
      <RichText text={buf.topic} />
      {#if buf.topicBy}
        <p class="mt-2 mb-0 text-xs text-muted-foreground">Set by {buf.topicBy}{buf.topicTime ? ` · ${formatFull(buf.topicTime)}` : ''}</p>
      {/if}
    </div>
  {/if}

  <div class="flex min-h-0 flex-1">
    <div class="relative flex min-w-0 flex-1 flex-col">
      {#if conn.status !== 'connected' && buf.kind !== 'server'}
        <div class="border-b bg-warn/10 px-5 py-1.5 text-[12.5px] text-warn" role="status">
          {#if conn.status === 'disconnected'}
            Disconnected{conn.error ? ` — ${conn.error}` : ''}.
          {:else}
            Connecting to {conn.name}…
          {/if}
        </div>
      {/if}
      <MessageList {buf} {filter} onnick={openNick} />
      <div class="flex h-5 items-center gap-2 px-[22px] text-xs text-muted-foreground transition-opacity duration-150 {typing.length ? 'opacity-100' : 'opacity-0'}">
        {#if typing.length}
          <span class="dots"><i></i><i></i><i></i></span>
          <span>
            {#if typing.length === 1}<b class="font-medium text-foreground/80">{typing[0]}</b> is typing
            {:else if typing.length === 2}<b class="font-medium text-foreground/80">{typing[0]}</b> and <b class="font-medium text-foreground/80">{typing[1]}</b> are typing
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
  .dots {
    display: flex;
    gap: 3px;
  }

  .dots i {
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: var(--muted-foreground);
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
