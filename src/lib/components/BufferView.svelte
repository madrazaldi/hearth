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
  import * as DropdownMenu from '$lib/components/ui/dropdown-menu';
  import Avatar from './Avatar.svelte';
  import { cn } from '$lib/utils';
  import type { NotifyLevel } from '../types';

  const LEVELS: { value: NotifyLevel | 'default'; label: string }[] = [
    { value: 'default', label: 'Default' },
    { value: 'all', label: 'All messages' },
    { value: 'mentions', label: 'Mentions only' },
    { value: 'none', label: 'Nothing (mute)' },
  ];
  const mod = navigator.userAgent.includes('Mac') ? '⌘' : 'Ctrl+';

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
  <header class="flex h-14 flex-none items-center gap-3 border-b pr-3 pl-4" data-drag-region>
    <div class="flex min-w-0 flex-1 items-center gap-3" data-drag-region>
      <span
        class={cn(
          'grid size-8 flex-none place-items-center rounded-lg border bg-muted/60 text-muted-foreground',
          buf.kind === 'query' && 'border-0 bg-transparent',
        )}
        aria-hidden="true"
      >
        {#if buf.kind === 'channel'}<Icon name="hash" size={16} stroke={2.2} />
        {:else if buf.kind === 'query'}<Avatar {conn} nick={buf.name} size={32} />
        {:else if conn.isControl}<Icon name="layers" size={16} />
        {:else}<Icon name="terminal" size={16} />{/if}
      </span>
      <div class="grid min-w-0 flex-1 leading-tight" data-drag-region>
        <div class="flex min-w-0 items-center gap-2">
          <h1 class="m-0 truncate text-[15px] font-semibold tracking-tight">
            {buf.kind === 'channel' ? buf.name.replace(/^#/, '') : title}
          </h1>
          {#if buf.kind === 'channel' && !buf.joined}
            <Badge variant="outline" class="h-4.5 border-warn/40 px-1.5 text-[10.5px] text-warn">not joined</Badge>
          {:else if buf.kind === 'query' && conn.online.has(conn.fold(buf.name))}
            {@const on = conn.online.get(conn.fold(buf.name))}
            <span class="flex items-center gap-1 text-[11.5px] text-muted-foreground">
              <i class="size-1.5 rounded-full {on ? 'bg-ok' : 'bg-muted-foreground/60'}"></i>{on ? 'online' : 'offline'}
            </span>
          {/if}
          {#if buf.level === 'none'}<span class="text-muted-foreground" title="Muted"><Icon name="bellOff" size={12} /></span>{/if}
        </div>
        {#if buf.kind === 'channel' && buf.topic}
          <button
            class="mt-0.5 min-w-0 truncate text-left text-[12.5px] text-muted-foreground hover:text-foreground"
            onclick={() => (topicOpen = !topicOpen)}
            title="Show full topic"
            aria-expanded={topicOpen}
          >
            <RichText text={buf.topic} plain />
          </button>
        {:else if subtitle}
          <span class="mt-0.5 min-w-0 truncate text-[12.5px] text-muted-foreground" data-drag-region>{subtitle}</span>
        {:else if buf.kind === 'channel'}
          <span class="mt-0.5 text-[12.5px] text-muted-foreground/70" data-drag-region>No topic</span>
        {/if}
      </div>
    </div>
    <div class="flex items-center gap-1">
      {#if filterOpen}
        <InputGroup.Root class="h-8 w-60 rounded-lg">
          <InputGroup.Addon><Icon name="search" size={13} /></InputGroup.Addon>
          <InputGroup.Input
            bind:ref={filterEl}
            bind:value={filter}
            placeholder="Filter messages"
            aria-label="Filter messages"
            class="h-8 text-[13px] md:text-[13px]"
            onkeydown={(e) => e.key === 'Escape' && toggleFilter()}
          />
          <InputGroup.Addon align="inline-end">
            <InputGroup.Button size="icon-xs" aria-label="Close filter" onclick={toggleFilter}><Icon name="x" size={13} /></InputGroup.Button>
          </InputGroup.Addon>
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
          <Tooltip.Content side="bottom">Filter messages <Kbd>{mod}F</Kbd></Tooltip.Content>
        </Tooltip.Root>
      {/if}
      {#if buf.kind === 'channel'}
        <Tooltip.Root>
          <Tooltip.Trigger>
            {#snippet child({ props })}
              <Button
                {...props}
                variant="ghost"
                size="sm"
                class={cn('h-8 gap-1.5 px-2 tabular-nums', app.settings.showMembers ? 'bg-muted text-foreground' : 'text-muted-foreground')}
                aria-label="Members, {buf.members.size}"
                aria-pressed={app.settings.showMembers}
                onclick={() => app.updateSettings({ showMembers: !app.settings.showMembers })}
              >
                <Icon name="users" size={15} />
                {#if buf.members.size}<span class="text-[12.5px] font-medium">{buf.members.size.toLocaleString()}</span>{/if}
              </Button>
            {/snippet}
          </Tooltip.Trigger>
          <Tooltip.Content side="bottom">{app.settings.showMembers ? 'Hide' : 'Show'} members <Kbd>{mod}⇧M</Kbd></Tooltip.Content>
        </Tooltip.Root>
      {/if}
      {#if buf.kind !== 'server'}
        <DropdownMenu.Root>
          <DropdownMenu.Trigger>
            {#snippet child({ props })}
              <Button {...props} variant="ghost" size="icon-sm" class="text-muted-foreground data-[state=open]:bg-muted" aria-label="More actions">
                <Icon name="more" />
              </Button>
            {/snippet}
          </DropdownMenu.Trigger>
          <DropdownMenu.Content align="end" class="w-56">
            <DropdownMenu.Item onSelect={() => app.markRead(buf)}><Icon name="read" size={15} />Mark as read</DropdownMenu.Item>
            {#if buf.kind === 'channel' && buf.topic}
              <DropdownMenu.Item onSelect={() => (topicOpen = !topicOpen)}><Icon name="info" size={15} />{topicOpen ? 'Hide' : 'Show'} full topic</DropdownMenu.Item>
            {/if}
            {#if buf.kind === 'channel' && !buf.joined}
              <DropdownMenu.Item onSelect={() => app.join(conn, buf.name)}><Icon name="refresh" size={15} />Rejoin</DropdownMenu.Item>
            {/if}
            {#if buf.kind === 'query' && conn.canMonitor}
              {@const watched = app.isWatched(conn, buf.name)}
              <DropdownMenu.Item onSelect={() => (watched ? app.unwatch(conn, buf.name) : app.watch(conn, buf.name))}>
                <Icon name="eye" size={15} />{watched ? 'Stop watching' : 'Tell me when online'}
              </DropdownMenu.Item>
            {/if}
            <DropdownMenu.Separator />
            <DropdownMenu.Sub>
              <DropdownMenu.SubTrigger><Icon name={buf.level === 'none' ? 'bellOff' : 'bell'} size={15} />Notifications</DropdownMenu.SubTrigger>
              <DropdownMenu.SubContent class="w-44">
                <DropdownMenu.RadioGroup value={buf.level} onValueChange={(v) => app.setNotify(buf, v as NotifyLevel | 'default')}>
                  {#each LEVELS as l (l.value)}
                    <DropdownMenu.RadioItem value={l.value} closeOnSelect>{l.label}</DropdownMenu.RadioItem>
                  {/each}
                </DropdownMenu.RadioGroup>
              </DropdownMenu.SubContent>
            </DropdownMenu.Sub>
            <DropdownMenu.Separator />
            <DropdownMenu.Item variant="destructive" onSelect={() => app.closeBuffer(buf)}>
              <Icon name="x" size={15} />{buf.kind === 'channel' && buf.joined ? 'Leave channel' : 'Close conversation'}
            </DropdownMenu.Item>
          </DropdownMenu.Content>
        </DropdownMenu.Root>
      {/if}
    </div>
  </header>

  {#if topicOpen && buf.topic}
    <div class="selectable border-b bg-muted/30 px-5 pt-3 pb-3.5 text-[13.5px] break-words whitespace-pre-wrap text-foreground/85 animate-in fade-in-0 slide-in-from-top-1 duration-150">
      <RichText text={buf.topic} />
      {#if buf.topicBy}
        <p class="mt-2 mb-0 text-xs text-muted-foreground">Set by {buf.topicBy}{buf.topicTime ? ` · ${formatFull(buf.topicTime)}` : ''}</p>
      {/if}
    </div>
  {/if}

  <div class="flex min-h-0 flex-1">
    <div class="relative flex min-w-0 flex-1 flex-col">
      {#if conn.status !== 'connected' && buf.kind !== 'server'}
        <div class="flex items-center gap-2 border-b bg-warn/8 px-5 py-1.5 text-[12.5px] text-warn" role="status">
          <span class="size-1.5 rounded-full bg-current {conn.status === 'disconnected' ? '' : 'animate-pulse'}"></span>
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
