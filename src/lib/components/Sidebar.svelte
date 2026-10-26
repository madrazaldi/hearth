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
  import * as DropdownMenu from '$lib/components/ui/dropdown-menu';
  import Avatar from './Avatar.svelte';
  import { nickHue } from '../nickcolor';
  import logo from '../../assets/logo.png';

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
      'flex-none rounded-full',
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

  /** A network's tile: its first letter, tinted by the same hash as nicks. */
  function initial(s: string) {
    return [...s.replace(/^[^\p{L}\p{N}]+/u, '')][0]?.toUpperCase() ?? '?';
  }

  function presence(c: Conn): string {
    if (c.status === 'connected') return c.away ? 'Away' : 'Online';
    if (c.status === 'disconnected') return 'Offline';
    return 'Connecting…';
  }

  const mod = isMac ? '⌘' : 'Ctrl+';

</script>


{#snippet iconBtn(label: string, icon: string, onclick: () => void, shortcut?: string)}
  <Tooltip.Root>
    <Tooltip.Trigger>
      {#snippet child({ props })}
        <Button
          {...props}
          variant="ghost"
          size="icon-sm"
          class="text-sidebar-foreground/60 hover:bg-sidebar-accent hover:text-sidebar-foreground"
          aria-label={label}
          {onclick}
        >
          <Icon name={icon} size={15} />
        </Button>
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Content side="bottom">
      {label}
      {#if shortcut}<Kbd>{shortcut}</Kbd>{/if}
    </Tooltip.Content>
  </Tooltip.Root>
{/snippet}

{#snippet badge(n: number)}
  <Badge class="h-4.5 min-w-4.5 rounded-full bg-brand px-1.5 text-[10.5px] font-semibold text-brand-text tabular-nums">{n}</Badge>
{/snippet}

{#snippet buffers(conn: Conn)}
  <ul role="group" class="m-0 mt-0.5 grid list-none gap-px p-0">
    {#each conn.channelBuffers as b (b.id)}
      {@const id = 'b' + b.id}
      {@const active = app.active === b}
      {@const muted = b.level === 'none'}
      {@const unread = b.unread > 0 && !muted}
      <ContextMenu.Root>
        <ContextMenu.Trigger>
          {#snippet child({ props })}
            <li
              {...props}
              class={cn(
                'focus-inset group/buf relative flex h-7.5 w-full items-center gap-2 rounded-md pr-1 pl-1.5 text-[13.5px] text-sidebar-foreground/62 transition-colors select-none',
                'hover:bg-sidebar-accent/70 hover:text-sidebar-foreground data-[state=open]:bg-sidebar-accent/70',
                unread && 'font-semibold text-sidebar-foreground',
                active && 'bg-sidebar-accent font-medium text-sidebar-accent-foreground shadow-xs',
                muted && 'opacity-55',
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
              <span
                class={cn(
                  'relative grid size-5 flex-none place-items-center text-sidebar-foreground/45',
                  (active || unread) && 'text-sidebar-foreground/80',
                  active && 'text-brand',
                )}
                aria-hidden="true"
              >
                {#if b.kind === 'channel'}
                  <Icon name="hash" size={14} stroke={2.2} />
                {:else}
                  <Avatar {conn} nick={b.name} size={18} />
                  {#if conn.online.has(conn.fold(b.name))}
                    <i
                      class={cn(
                        'absolute -right-px -bottom-px size-2 rounded-full ring-2 ring-sidebar',
                        conn.online.get(conn.fold(b.name)) ? 'bg-ok' : 'bg-muted-foreground/60',
                      )}
                    ></i>
                  {/if}
                {/if}
              </span>
              <span class={cn('min-w-0 flex-1 truncate text-left', b.kind === 'channel' && !b.joined && 'italic opacity-55')}>{name(b)}</span>
              {#if muted}
                <span class="grid place-items-center pr-1 text-muted-foreground/70 group-hover/buf:hidden" title="Muted">
                  <Icon name="bellOff" size={12} />
                </span>
              {:else if b.highlights > 0}
                <span class="group-hover/buf:hidden">{@render badge(b.highlights)}</span>
              {:else if b.typing.size > 0 && b.kind === 'query'}
                <span class="typing-dots group-hover/buf:hidden"><i></i><i></i><i></i></span>
              {/if}
              <!-- Mouse-only shortcut; from the keyboard it's in the context menu and on Cmd/Ctrl+W. -->
              <span
                class="hidden size-5.5 place-items-center rounded-md text-muted-foreground group-hover/buf:grid hover:bg-sidebar-border hover:text-foreground"
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
          <ContextMenu.Item onSelect={() => app.markRead(b)}><Icon name="read" size={15} />Mark as read</ContextMenu.Item>
          {#if b.kind === 'channel' && !b.joined}
            <ContextMenu.Item onSelect={() => app.join(b.conn, b.name)}><Icon name="refresh" size={15} />Rejoin</ContextMenu.Item>
          {/if}
          {#if b.kind === 'query' && b.conn.canMonitor}
            {@const watched = app.isWatched(b.conn, b.name)}
            <ContextMenu.Item onSelect={() => (watched ? app.unwatch(b.conn, b.name) : app.watch(b.conn, b.name))}>
              <Icon name="eye" size={15} />{watched ? 'Stop watching' : 'Tell me when online'}
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
          <ContextMenu.Item variant="destructive" onSelect={() => app.closeBuffer(b)}>
            <Icon name="x" size={15} />{b.kind === 'channel' && b.joined ? 'Leave channel' : 'Close'}
          </ContextMenu.Item>
        </ContextMenu.Content>
      </ContextMenu.Root>
    {/each}
  </ul>
{/snippet}

<aside class="relative flex min-h-0 flex-col bg-sidebar text-sidebar-foreground" aria-label="Sidebar">
  <div class={cn('flex flex-col gap-2.5 px-2.5 pb-2', isMac ? 'pt-10' : 'pt-2.5')} data-drag-region>
    <div class="flex h-8 items-center gap-2 pl-1" data-drag-region>
      <img class="size-6 rounded-md shadow-sm" src={logo} width="24" height="24" alt="" draggable="false" />
      <span class="text-[14px] font-semibold tracking-tight">Hearth</span>
      <div class="ml-auto flex items-center">
        {@render iconBtn('Search messages', 'list', () => (app.searchOpen = true), isMac ? '⌘⇧F' : 'Ctrl+Shift+F')}
        {@render iconBtn('Add network', 'plus', () => (app.settingsOpen = { tab: 'servers', serverId: 'new' }))}
      </div>
    </div>
    <Button
      variant="outline"
      size="sm"
      class="h-8 min-w-0 justify-start gap-2 rounded-lg border-sidebar-border bg-sidebar-accent/40 pr-1.5 pl-2.5 text-[13px] font-normal text-sidebar-foreground/55 shadow-none hover:bg-sidebar-accent hover:text-sidebar-foreground dark:bg-sidebar-accent/40 dark:hover:bg-sidebar-accent"
      onclick={() => (app.switcherOpen = true)}
    >
      <Icon name="search" size={14} />
      <span class="flex-1 text-left">Jump to…</span>
      <Kbd aria-hidden="true">{isMac ? '⌘' : 'Ctrl '}K</Kbd>
    </Button>
  </div>

  <nav class="flex-1 overflow-y-auto px-2.5 pt-1 pb-4" aria-label="Networks and channels">
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
                class={cn('group/net select-none outline-none', conn.isControl ? 'mt-0' : 'mt-3')}
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
                      'flex h-7 items-center gap-1.5 rounded-md px-1.5 text-[11.5px] font-medium tracking-wide text-sidebar-foreground/50 uppercase outline-2 -outline-offset-2 outline-transparent transition-colors group-focus-visible/net:outline-ring',
                      'hover:text-sidebar-foreground',
                      active && 'text-sidebar-foreground',
                    )}
                    title="Bouncer console"
                  >
                    <Icon name="layers" size={12} />
                    <span class="truncate">{conn.name}</span>
                    <span class={cn(dotClass(conn.status), 'size-1.5')}></span>
                    {#if label}
                      <span class={cn('ml-auto text-[11px] tracking-normal whitespace-nowrap normal-case', conn.status === 'disconnected' && 'text-destructive')}>{label}</span>
                    {/if}
                  </div>
                  {#if conn.buffers.size}{@render buffers(conn)}{/if}
                {:else}
                  <div
                    class={cn(
                      'group/row flex h-8 min-w-0 items-center gap-2 rounded-md pr-1 pl-1.5 text-[13px] font-semibold text-sidebar-foreground/85 transition-colors',
                      'outline-2 -outline-offset-2 outline-transparent group-focus-visible/net:outline-ring',
                      'hover:bg-sidebar-accent/70 hover:text-sidebar-foreground group-data-[state=open]/net:bg-sidebar-accent/70',
                      active && 'bg-sidebar-accent text-sidebar-accent-foreground shadow-xs',
                    )}
                  >
                    <span
                      class="nettile relative grid size-5 flex-none place-items-center rounded-[5px] text-[10.5px] font-bold"
                      style="--h:{nickHue(conn.name)}"
                      aria-hidden="true"
                    >
                      {initial(conn.name)}
                      <i class={cn(dotClass(conn.status), 'absolute -right-0.75 -bottom-0.75 size-2 ring-2 ring-sidebar')}></i>
                    </span>
                    <span class="truncate">{conn.name}</span>
                    {#if label}
                      <span class={cn('ml-auto text-[11px] font-normal whitespace-nowrap text-muted-foreground', conn.status === 'disconnected' && 'text-destructive')}>{label}</span>
                    {/if}
                    {#if conn.collapsed}
                      {@const hl = conn.channelBuffers.reduce((n, b) => n + b.highlights, 0)}
                      {#if hl}<span class={label ? '' : 'ml-auto'}>{@render badge(hl)}</span>{/if}
                    {/if}
                    <!-- Left/Right arrows fold from the keyboard. -->
                    <span
                      class={cn(
                        'grid size-5.5 flex-none place-items-center rounded-md text-muted-foreground/70 opacity-0 transition-[opacity,transform] duration-150 group-hover/row:opacity-100 hover:bg-sidebar-border hover:text-foreground',
                        !label && !(conn.collapsed && conn.channelBuffers.some((b) => b.highlights)) && 'ml-auto',
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
          <ContextMenu.Content class="min-w-52" aria-label={conn.name}>
            <ContextMenu.Item onSelect={() => app.open(conn.server)}><Icon name="terminal" size={15} />Open server log</ContextMenu.Item>
            {#if conn.isControl}
              <ContextMenu.Item onSelect={() => (app.settingsOpen = { tab: 'servers', serverId: conn.serverId })}>
                <Icon name="layers" size={15} />Manage networks…
              </ContextMenu.Item>
            {:else if conn.status === 'connected'}
              <ContextMenu.Item onSelect={() => app.browseChannels(conn)}><Icon name="compass" size={15} />Browse channels…</ContextMenu.Item>
            {/if}
            <ContextMenu.Separator />
            <ContextMenu.Item onSelect={() => api.reconnect(conn.serverId)}><Icon name="refresh" size={15} />Reconnect</ContextMenu.Item>
            {#if conn.status === 'disconnected' && !conn.retryAt}
              <ContextMenu.Item onSelect={() => api.connect(conn.serverId)}><Icon name="plug" size={15} />Connect</ContextMenu.Item>
            {:else}
              <ContextMenu.Item onSelect={() => api.disconnect(conn.serverId)}><Icon name="unplug" size={15} />Disconnect</ContextMenu.Item>
            {/if}
            <ContextMenu.Separator />
            <ContextMenu.Item onSelect={() => (app.settingsOpen = { tab: 'servers', serverId: conn.serverId })}>
              <Icon name="edit" size={15} />Edit server…
            </ContextMenu.Item>
          </ContextMenu.Content>
        </ContextMenu.Root>
      {/each}
    </ul>
  </nav>

  <footer class="flex items-center gap-1 border-t border-sidebar-border p-2">
    {#if app.active}
      {@const c = app.active.conn}
      <DropdownMenu.Root>
        <DropdownMenu.Trigger>
          {#snippet child({ props })}
            <button
              {...props}
              class="flex min-w-0 flex-1 items-center gap-2.5 rounded-lg p-1.5 text-left transition-colors hover:bg-sidebar-accent data-[state=open]:bg-sidebar-accent"
              aria-label="{c.nick || 'You'} on {c.name}, {presence(c)}. Account menu"
            >
              <span class="relative flex">
                <Avatar conn={c} nick={c.nick || '?'} size={30} />
                <i class={cn(dotClass(c.status, c.away), 'absolute -right-0.5 -bottom-0.5 size-2.5 ring-2 ring-sidebar')}></i>
              </span>
              <span class="grid min-w-0 flex-1 leading-tight">
                <span class="truncate text-[13px] font-semibold">{c.nick || '—'}</span>
                <span class="truncate text-[11.5px] text-muted-foreground">{presence(c)} · {c.name}</span>
              </span>
              <span class="text-muted-foreground"><Icon name="updown" size={14} /></span>
            </button>
          {/snippet}
        </DropdownMenu.Trigger>
        <DropdownMenu.Content side="top" align="start" sideOffset={6} class="w-60">
          <DropdownMenu.Label class="flex items-center gap-2.5 py-1.5 font-normal">
            <Avatar conn={c} nick={c.nick || '?'} size={32} />
            <span class="grid min-w-0 leading-tight">
              <span class="truncate text-[13px] font-semibold">{c.nick || '—'}</span>
              <span class="truncate text-xs text-muted-foreground">{c.name}</span>
            </span>
          </DropdownMenu.Label>
          <DropdownMenu.Separator />
          {#if c.status === 'connected' && !c.isControl}
            <DropdownMenu.Item onSelect={() => c.send(c.away ? 'AWAY' : 'AWAY :Away')}>
              <Icon name="coffee" size={15} />{c.away ? "I'm back" : 'Set yourself away'}
            </DropdownMenu.Item>
          {/if}
          <DropdownMenu.Item onSelect={() => (app.settingsOpen = { tab: 'servers', serverId: 'new' })}>
            <Icon name="plus" size={15} />Add network…
          </DropdownMenu.Item>
          <DropdownMenu.Item onSelect={() => (app.searchOpen = true)}>
            <Icon name="list" size={15} />Search messages
            <DropdownMenu.Shortcut>{mod}⇧F</DropdownMenu.Shortcut>
          </DropdownMenu.Item>
          <DropdownMenu.Item onSelect={() => (app.shortcutsOpen = true)}>
            <Icon name="keyboard" size={15} />Keyboard shortcuts
            <DropdownMenu.Shortcut>{mod}/</DropdownMenu.Shortcut>
          </DropdownMenu.Item>
          <DropdownMenu.Separator />
          <DropdownMenu.Item onSelect={() => (app.settingsOpen = { tab: 'appearance' })}>
            <Icon name="gear" size={15} />Settings
            <DropdownMenu.Shortcut>{mod},</DropdownMenu.Shortcut>
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Root>
    {:else}
      <div class="flex-1"></div>
    {/if}
    {@render iconBtn('Settings', 'gear', () => (app.settingsOpen = { tab: 'appearance' }), `${mod},`)}
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
  .nettile {
    color: oklch(var(--nick-l) var(--nick-c) var(--h));
    background: color-mix(in oklch, oklch(var(--nick-l) var(--nick-c) var(--h)) 22%, var(--sidebar-accent));
  }

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
