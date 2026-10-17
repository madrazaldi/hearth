<script lang="ts">
  import { app, type Buffer, type Conn } from '../state.svelte';
  import { api } from '../api';
  import type { NotifyLevel } from '../types';
  import Icon from './Icon.svelte';
  import { tick } from 'svelte';
  import { bufferLabel, stepIndex } from '../a11y';
  import { cn } from '$lib/utils';
  import { Button } from '$lib/components/ui/button';
  import { Badge } from '$lib/components/ui/badge';
  import { Kbd } from '$lib/components/ui/kbd';
  import * as ContextMenu from '$lib/components/ui/context-menu';
  import * as Tooltip from '$lib/components/ui/tooltip';

  const LEVELS: { value: NotifyLevel | 'default'; label: string }[] = [
    { value: 'default', label: 'Default' },
    { value: 'all', label: 'All messages' },
    { value: 'mentions', label: 'Mentions only' },
    { value: 'none', label: 'Nothing (mute)' },
  ];

  const isMac = navigator.userAgent.includes('Mac');

  function statusLabel(c: Conn): string {
    if (c.status === 'connected') return '';
    if (c.status === 'disconnected') {
      if (c.retryAt) return `retrying in ${Math.max(0, Math.round((c.retryAt - now) / 1000))}s`;
      return 'offline';
    }
    return 'connecting';
  }

  let now = $state(Date.now());
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(t);
  });

  // ── Tree keyboard navigation ─────────────────────────────────
  // One tabbable item (roving tabindex); arrows move, Left/Right fold.
  type Node = { id: string; conn: Conn; buf?: Buffer };
  let tree = $state<HTMLUListElement>();
  let focusId = $state<string | null>(null);

  const nodes = $derived.by(() => {
    const out: Node[] = [];
    for (const conn of app.orderedConns) {
      out.push({ id: 'n' + conn.id, conn });
      if (conn.isControl || !conn.collapsed) for (const buf of conn.channelBuffers) out.push({ id: 'b' + buf.id, conn, buf });
    }
    return out;
  });

  const roving = $derived.by(() => {
    if (focusId && nodes.some((n) => n.id === focusId)) return focusId;
    const a = app.active;
    const at = a && nodes.find((n) => (n.buf ?? n.conn.server) === a);
    return at ? at.id : nodes[0]?.id;
  });

  const tid = (id: string) => encodeURIComponent(id);

  async function focusNode(id: string) {
    focusId = id;
    await tick();
    tree?.querySelector<HTMLElement>(`[data-tid="${CSS.escape(tid(id))}"]`)?.focus();
  }

  function onItemKey(e: KeyboardEvent, n: Node) {
    // Items nest (a network holds its channels); only the focused one acts.
    if (e.target !== e.currentTarget) return;
    const i = nodes.findIndex((x) => x.id === n.id);
    const foldable = !n.buf && !n.conn.isControl;
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      app.open(n.buf ?? n.conn.server);
    } else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      e.preventDefault();
      e.stopPropagation();
      // The menu is bits-ui's context menu, which opens from a contextmenu event.
      const el = e.currentTarget as HTMLElement;
      const r = el.getBoundingClientRect();
      el.dispatchEvent(
        new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: r.left + 24, clientY: r.top + 24 }),
      );
    } else if (e.key === 'ArrowRight') {
      e.preventDefault();
      if (foldable && n.conn.collapsed) n.conn.collapsed = false;
      else if (!n.buf && nodes[i + 1]?.buf) focusNode(nodes[i + 1].id);
    } else if (e.key === 'ArrowLeft') {
      e.preventDefault();
      if (n.buf) focusNode('n' + n.conn.id);
      else if (foldable && !n.conn.collapsed) n.conn.collapsed = true;
    } else {
      const to = stepIndex(e.key, i, nodes.length);
      if (to === null) return;
      e.preventDefault();
      focusNode(nodes[to].id);
    }
  }

  function netLabel(conn: Conn, status: string): string {
    const parts = [conn.name, conn.isControl ? 'bouncer' : 'network'];
    if (status) parts.push(status);
    if (conn.collapsed && !conn.isControl) {
      const hl = conn.channelBuffers.reduce((n, b) => n + b.highlights, 0);
      if (hl) parts.push(`${hl} mention${hl === 1 ? '' : 's'}`);
    }
    return parts.join(', ');
  }

  function bufLabel(conn: Conn, b: Buffer): string {
    return bufferLabel({
      name: b.name,
      kind: b.kind,
      unread: b.unread,
      highlights: b.highlights,
      muted: b.level === 'none',
      joined: b.joined,
      online: b.kind === 'query' ? conn.online.get(conn.fold(b.name)) : undefined,
      typing: b.kind === 'query' && b.typing.size > 0,
    });
  }

  function resizeKey(e: KeyboardEvent) {
    const d = e.key === 'ArrowRight' ? 16 : e.key === 'ArrowLeft' ? -16 : 0;
    if (!d) return;
    e.preventDefault();
    app.updateSettings({ sidebarWidth: Math.min(420, Math.max(190, app.settings.sidebarWidth + d)) });
  }

  function resize(e: PointerEvent) {
    const start = e.clientX;
    const w0 = app.settings.sidebarWidth;
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const move = (ev: PointerEvent) =>
      app.updateSettings({ sidebarWidth: Math.round(Math.min(420, Math.max(190, w0 + ev.clientX - start))) });
    const up = () => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
  }

  function dotClass(status: string, away = false) {
    return cn(
      'size-1.5 flex-none rounded-full',
      away
        ? 'bg-warn'
        : status === 'connected'
          ? 'bg-ok/80'
          : status === 'connecting' || status === 'registering'
            ? 'animate-pulse bg-warn'
            : status === 'disconnected'
              ? 'bg-destructive'
              : 'bg-muted-foreground/50',
    );
  }

  function name(b: Buffer) {
    return b.kind === 'channel' ? b.name.replace(/^#/, '') : b.name;
  }
</script>


{#snippet iconBtn(label: string, icon: string, onclick: () => void, shortcut?: string)}
  <Tooltip.Root>
    <Tooltip.Trigger>
      {#snippet child({ props })}
        <Button {...props} variant="ghost" size="icon-sm" class="text-muted-foreground" aria-label={label} {onclick}>
          <Icon name={icon} size={15} />
        </Button>
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Content side="top">
      {label}
      {#if shortcut}<Kbd>{shortcut}</Kbd>{/if}
    </Tooltip.Content>
  </Tooltip.Root>
{/snippet}

{#snippet buffers(conn: Conn)}
  <ul role="group" class="m-0 list-none p-0">
    {#each conn.channelBuffers as b (b.id)}
      {@const id = 'b' + b.id}
      {@const active = app.active === b}
      {@const muted = b.level === 'none'}
      <ContextMenu.Root>
        <ContextMenu.Trigger>
          {#snippet child({ props })}
            <li
              {...props}
              class={cn(
                'focus-inset group/buf relative flex h-7 w-full items-center gap-2 rounded-md pr-1.5 pl-2.5 text-[13.5px] text-sidebar-foreground/65 transition-colors select-none',
                'hover:bg-sidebar-accent/60 hover:text-sidebar-foreground data-[state=open]:bg-sidebar-accent/60',
                b.unread > 0 && 'font-medium text-sidebar-foreground',
                active && 'bg-sidebar-accent text-sidebar-accent-foreground',
                muted && 'opacity-60',
                muted && b.unread > 0 && 'font-normal',
              )}
              role="treeitem"
              aria-level={2}
              aria-selected={active}
              aria-label={bufLabel(conn, b)}
              tabindex={roving === id ? 0 : -1}
              data-tid={tid(id)}
              onfocus={() => (focusId = id)}
              onclick={(e) => {
                e.stopPropagation();
                app.open(b);
              }}
              onkeydown={(e) => onItemKey(e, { id, conn, buf: b })}
            >
              {#if active}
                <span class="absolute inset-y-2 left-0 w-0.5 rounded-full bg-brand" aria-hidden="true"></span>
              {/if}
              <span
                class={cn(
                  'relative grid w-3.5 flex-none place-items-center font-mono text-[13px] font-normal text-muted-foreground/70',
                  (active || b.unread > 0) && 'text-muted-foreground',
                )}
                aria-hidden="true"
              >
                {#if b.kind === 'channel'}#{:else}<Icon name="user" size={12} stroke={1.6} />{/if}
                {#if b.kind === 'query' && conn.online.has(conn.fold(b.name))}
                  <i
                    class={cn(
                      'absolute -right-0.5 -bottom-px size-1.5 rounded-full ring-[1.5px] ring-sidebar',
                      conn.online.get(conn.fold(b.name)) ? 'bg-ok' : 'bg-muted-foreground/60',
                    )}
                  ></i>
                {/if}
              </span>
              <span
                class={cn(
                  'min-w-0 flex-1 truncate text-left',
                  b.kind === 'channel' && !b.joined && 'italic opacity-55',
                )}>{name(b)}</span
              >
              {#if muted}
                <span class="grid place-items-center pr-0.5 text-muted-foreground/70 group-hover/buf:hidden" title="Muted">
                  <Icon name="bellOff" size={12} />
                </span>
              {:else if b.highlights > 0}
                <Badge class="h-4.5 min-w-4.5 bg-brand px-1.5 text-[11px] font-semibold text-(--brand-text) tabular-nums group-hover/buf:hidden">
                  {b.highlights}
                </Badge>
              {:else if b.typing.size > 0 && b.kind === 'query'}
                <span class="typing-dots group-hover/buf:hidden"><i></i><i></i><i></i></span>
              {/if}
              <!-- Mouse-only shortcut; from the keyboard it's in the context menu and on Cmd/Ctrl+W. -->
              <span
                class="hidden size-5 place-items-center rounded text-muted-foreground group-hover/buf:grid hover:bg-sidebar-accent hover:text-foreground"
                role="button"
                tabindex="-1"
                aria-hidden="true"
                title={b.kind === 'channel' ? 'Leave' : 'Close'}
                onclick={(e) => {
                  e.stopPropagation();
                  app.closeBuffer(b);
                }}
                onkeydown={() => {}}
              >
                <Icon name="x" size={12} />
              </span>
            </li>
          {/snippet}
        </ContextMenu.Trigger>
        <ContextMenu.Content class="min-w-48" aria-label={b.name}>
          <ContextMenu.Item onSelect={() => app.markRead(b)}>Mark as read</ContextMenu.Item>
          {#if b.kind === 'channel' && !b.joined}
            <ContextMenu.Item onSelect={() => app.join(b.conn, b.name)}>Rejoin</ContextMenu.Item>
          {/if}
          {#if b.kind === 'query' && b.conn.canMonitor}
            {@const watched = app.isWatched(b.conn, b.name)}
            <ContextMenu.Item onSelect={() => (watched ? app.unwatch(b.conn, b.name) : app.watch(b.conn, b.name))}>
              {watched ? 'Stop watching' : 'Tell me when online'}
            </ContextMenu.Item>
          {/if}
          <ContextMenu.Separator />
          <ContextMenu.Group>
            <ContextMenu.GroupHeading>Notify me about</ContextMenu.GroupHeading>
            <ContextMenu.RadioGroup value={b.level} onValueChange={(v) => app.setNotify(b, v as NotifyLevel | 'default')}>
              {#each LEVELS as l (l.value)}
                <ContextMenu.RadioItem value={l.value} closeOnSelect>{l.label}</ContextMenu.RadioItem>
              {/each}
            </ContextMenu.RadioGroup>
          </ContextMenu.Group>
          <ContextMenu.Separator />
          <ContextMenu.Item onSelect={() => app.closeBuffer(b)}>
            {b.kind === 'channel' && b.joined ? 'Leave channel' : 'Close'}
          </ContextMenu.Item>
        </ContextMenu.Content>
      </ContextMenu.Root>
    {/each}
  </ul>
{/snippet}

<aside
  class="relative flex min-h-0 flex-col border-r border-sidebar-border bg-sidebar text-sidebar-foreground"
  aria-label="Sidebar"
>
  <div class={cn('flex gap-1 px-2.5 pb-1.5', isMac ? 'pt-10.5' : 'pt-2.5')} data-tauri-drag-region>
    <Button
      variant="outline"
      size="sm"
      class="h-8 min-w-0 flex-1 justify-start gap-2 bg-transparent pr-1.5 pl-2.5 text-[13px] font-normal text-muted-foreground hover:text-foreground"
      onclick={() => (app.switcherOpen = true)}
    >
      <Icon name="search" size={14} />
      <span class="flex-1 text-left">Jump to…</span>
      <Kbd aria-hidden="true">{isMac ? '⌘' : 'Ctrl '}K</Kbd>
    </Button>
    {@render iconBtn('Search messages', 'list', () => (app.searchOpen = true), isMac ? '⌘⇧F' : 'Ctrl+Shift+F')}
  </div>

  <nav class="flex-1 overflow-y-auto px-2 pt-1.5 pb-4" aria-label="Networks and channels">
    <ul class="m-0 list-none p-0" role="tree" aria-label="Networks and channels" bind:this={tree}>
      {#each app.orderedConns as conn (conn.id)}
        {@const label = statusLabel(conn)}
        {@const id = 'n' + conn.id}
        {@const kids = conn.channelBuffers.length > 0}
        {@const active = app.active === conn.server}
        <ContextMenu.Root>
          <ContextMenu.Trigger>
            {#snippet child({ props })}
              <!-- The item holds its channels too; the ring is drawn on just its own row. -->
              <li
                {...props}
                class={cn('group/net select-none outline-none', conn.isControl ? 'mt-0' : conn.parentId ? 'mt-1.5' : 'mt-2.5')}
                role="treeitem"
                aria-level={1}
                aria-selected={active}
                aria-expanded={kids ? conn.isControl || !conn.collapsed : undefined}
                aria-label={netLabel(conn, label)}
                tabindex={roving === id ? 0 : -1}
                data-tid={tid(id)}
                onfocus={(e) => e.target === e.currentTarget && (focusId = id)}
                onclick={() => app.open(conn.server)}
                onkeydown={(e) => onItemKey(e, { id, conn })}
              >
                {#if conn.isControl}
                  <div
                    class={cn(
                      'flex items-baseline gap-2 rounded-md px-2 pt-3.5 pb-1 text-left outline-2 -outline-offset-2 outline-transparent group-focus-visible/net:outline-ring',
                      '[&_.eyebrow]:transition-colors hover:[&_.eyebrow]:text-foreground',
                      active && '[&_.eyebrow]:text-foreground',
                    )}
                    title="Bouncer console"
                  >
                    <span class="eyebrow">{conn.name}</span>
                    {#if label}
                      <span class={cn('ml-auto text-[11px] whitespace-nowrap text-muted-foreground', conn.status === 'disconnected' && 'text-destructive')}>{label}</span>
                    {/if}
                  </div>
                  {#if conn.buffers.size}<div class="pt-0.5">{@render buffers(conn)}</div>{/if}
                {:else}
                  <div class="group/row flex items-center">
                    <div
                      class={cn(
                        'flex h-7 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-[13px] font-semibold text-sidebar-foreground/80 transition-colors',
                        'outline-2 -outline-offset-2 outline-transparent group-focus-visible/net:outline-ring',
                        'hover:bg-sidebar-accent/50 hover:text-sidebar-foreground group-data-[state=open]/net:bg-sidebar-accent/50',
                        active && 'bg-sidebar-accent text-sidebar-accent-foreground',
                      )}
                    >
                      <span class={dotClass(conn.status)}></span>
                      <span class="truncate">{conn.name}</span>
                      {#if label}
                        <span class={cn('ml-auto text-[11px] font-normal whitespace-nowrap text-muted-foreground', conn.status === 'disconnected' && 'text-destructive')}>{label}</span>
                      {/if}
                      {#if conn.collapsed}
                        {@const hl = conn.channelBuffers.reduce((n, b) => n + b.highlights, 0)}
                        {#if hl}
                          <Badge class="h-4.5 min-w-4.5 bg-brand px-1.5 text-[11px] font-semibold text-(--brand-text) tabular-nums">{hl}</Badge>
                        {/if}
                      {/if}
                    </div>
                    <!-- Left/Right arrows fold from the keyboard. -->
                    <span
                      class={cn(
                        'grid size-5.5 place-items-center rounded text-muted-foreground/70 opacity-0 transition-[opacity,transform] duration-150 group-hover/row:opacity-100 hover:text-foreground',
                        conn.collapsed && '-rotate-90 opacity-100',
                      )}
                      role="button"
                      tabindex="-1"
                      aria-hidden="true"
                      onclick={(e) => {
                        e.stopPropagation();
                        conn.collapsed = !conn.collapsed;
                      }}
                      onkeydown={() => {}}
                    >
                      <Icon name="down" size={13} />
                    </span>
                  </div>
                  {#if !conn.collapsed && kids}
                    {@render buffers(conn)}
                  {/if}
                {/if}
              </li>
            {/snippet}
          </ContextMenu.Trigger>
          <ContextMenu.Content class="min-w-48" aria-label={conn.name}>
            <ContextMenu.Item onSelect={() => app.open(conn.server)}>Open server log</ContextMenu.Item>
            {#if conn.isControl}
              <ContextMenu.Item onSelect={() => (app.settingsOpen = { tab: 'servers', serverId: conn.serverId })}>
                Manage networks…
              </ContextMenu.Item>
            {:else if conn.status === 'connected'}
              <ContextMenu.Item onSelect={() => app.browseChannels(conn)}>Browse channels…</ContextMenu.Item>
            {/if}
            <ContextMenu.Item onSelect={() => api.reconnect(conn.serverId)}>Reconnect</ContextMenu.Item>
            {#if conn.status === 'disconnected' && !conn.retryAt}
              <ContextMenu.Item onSelect={() => api.connect(conn.serverId)}>Connect</ContextMenu.Item>
            {:else}
              <ContextMenu.Item onSelect={() => api.disconnect(conn.serverId)}>Disconnect</ContextMenu.Item>
            {/if}
            <ContextMenu.Separator />
            <ContextMenu.Item onSelect={() => (app.settingsOpen = { tab: 'servers', serverId: conn.serverId })}>
              Edit server…
            </ContextMenu.Item>
          </ContextMenu.Content>
        </ContextMenu.Root>
      {/each}
    </ul>
  </nav>

  <footer class="flex items-center gap-1 border-t border-sidebar-border p-2">
    {@render iconBtn('Add network', 'plus', () => (app.settingsOpen = { tab: 'servers', serverId: 'new' }))}
    <div class="flex min-w-0 flex-1 items-center gap-2 px-1.5 text-[12.5px] text-muted-foreground">
      {#if app.active}
        {@const c = app.active.conn}
        <span class={dotClass(c.status, c.away)}></span>
        <span class="truncate font-mono text-xs">{c.nick || '—'}</span>
        {#if c.away}<span class="text-[11px] text-warn">away</span>{/if}
        <span class="sr-only">, {c.status}</span>
      {/if}
    </div>
    {@render iconBtn('Settings', 'settings', () => (app.settingsOpen = { tab: 'appearance' }))}
  </footer>

  <div
    class="absolute top-0 -right-0.75 z-5 h-full w-1.5 cursor-col-resize transition-colors hover:bg-brand/20 focus-visible:bg-brand/60 focus-visible:outline-none"
    onpointerdown={resize}
    onkeydown={resizeKey}
    role="slider"
    aria-orientation="horizontal"
    aria-label="Sidebar width"
    aria-valuenow={app.settings.sidebarWidth}
    aria-valuemin={190}
    aria-valuemax={420}
    tabindex="0"
  ></div>
</aside>

<style>
  .typing-dots {
    display: flex;
    gap: 2px;
    padding-right: 4px;
  }

  .typing-dots i {
    width: 3px;
    height: 3px;
    border-radius: 50%;
    background: var(--muted-foreground);
    animation: blink 1.2s infinite;
  }

  .typing-dots i:nth-child(2) {
    animation-delay: 0.2s;
  }
  .typing-dots i:nth-child(3) {
    animation-delay: 0.4s;
  }

  @keyframes blink {
    0%,
    60%,
    100% {
      opacity: 0.25;
    }
    30% {
      opacity: 1;
    }
  }
</style>
