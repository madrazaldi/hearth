<script lang="ts">
  import { app, type Buffer, type Conn } from '../state.svelte';
  import { api } from '../api';
  import type { NotifyLevel } from '../types';
  import Icon from './Icon.svelte';
  import { tick } from 'svelte';
  import { bufferLabel, menu as menuNav, stepIndex } from '../a11y';

  const LEVELS: { value: NotifyLevel | 'default'; label: string }[] = [
    { value: 'default', label: 'Default' },
    { value: 'all', label: 'All messages' },
    { value: 'mentions', label: 'Mentions only' },
    { value: 'none', label: 'Nothing (mute)' },
  ];

  const isMac = navigator.userAgent.includes('Mac');
  let menu = $state<{ x: number; y: number; conn?: Conn; buf?: Buffer } | null>(null);

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

  let menuEl = $state<HTMLDivElement>();
  let menuPos = $state({ left: 0, top: 0 });
  $effect(() => {
    if (!menu || !menuEl) return;
    const r = menuEl.getBoundingClientRect();
    menuPos = {
      left: Math.min(menu.x, window.innerWidth - r.width - 8),
      top: Math.max(8, Math.min(menu.y, window.innerHeight - r.height - 8)),
    };
  });

  function openMenu(e: MouseEvent, conn?: Conn, buf?: Buffer) {
    e.preventDefault();
    e.stopPropagation();
    menuPos = { left: 0, top: 0 };
    menu = { x: e.clientX, y: e.clientY, conn, buf };
  }

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
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      menuPos = { left: 0, top: 0 };
      menu = { x: r.left + 24, y: r.top + 24, conn: n.conn, buf: n.buf };
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

  function name(b: Buffer) {
    return b.kind === 'channel' ? b.name.replace(/^#/, '') : b.name;
  }
</script>

{#snippet buffers(conn: Conn)}
  <ul role="group">
    {#each conn.channelBuffers as b (b.id)}
      {@const id = 'b' + b.id}
      <li
        class="buf focus-inset"
        class:active={app.active === b}
        class:unread={b.unread > 0}
        class:parted={b.kind === 'channel' && !b.joined}
        class:muted={b.level === 'none'}
        role="treeitem"
        aria-level={2}
        aria-selected={app.active === b}
        aria-label={bufLabel(conn, b)}
        tabindex={roving === id ? 0 : -1}
        data-tid={tid(id)}
        onfocus={() => (focusId = id)}
        onclick={(e) => {
          e.stopPropagation();
          app.open(b);
        }}
        onkeydown={(e) => onItemKey(e, { id, conn, buf: b })}
        oncontextmenu={(e) => openMenu(e, conn, b)}
      >
        <span class="glyph" aria-hidden="true">
          {#if b.kind === 'channel'}#{:else}<Icon name="user" size={12} stroke={1.6} />{/if}
          {#if b.kind === 'query' && conn.online.has(conn.fold(b.name))}
            <i class="presence" class:online={conn.online.get(conn.fold(b.name))}></i>
          {/if}
        </span>
        <span class="label">{name(b)}</span>
        {#if b.level === 'none'}
          <span class="mute" title="Muted"><Icon name="bellOff" size={12} /></span>
        {:else if b.highlights > 0}
          <span class="badge">{b.highlights}</span>
        {:else if b.typing.size > 0 && b.kind === 'query'}
          <span class="typing-dots"><i></i><i></i><i></i></span>
        {/if}
        <!-- Mouse-only shortcut; from the keyboard it's in the context menu and on Cmd/Ctrl+W. -->
        <span
          class="close"
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
    {/each}
  </ul>
{/snippet}

<svelte:window onclick={() => (menu = null)} onblur={() => (menu = null)} />

<aside class:mac={isMac} aria-label="Sidebar">
  <div class="top" data-tauri-drag-region>
    <button class="find" onclick={() => (app.switcherOpen = true)} title="Jump to…">
      <Icon name="search" size={14} />
      <span>Jump to…</span>
      <span class="kbd" aria-hidden="true">{isMac ? '⌘' : 'Ctrl '}K</span>
    </button>
    <button
      class="icon-btn"
      title="Search messages ({isMac ? '⌘⇧F' : 'Ctrl+Shift+F'})"
      aria-label="Search messages"
      onclick={() => (app.searchOpen = true)}
    >
      <Icon name="list" size={15} />
    </button>
  </div>

  <nav aria-label="Networks and channels">
    <ul class="tree" role="tree" aria-label="Networks and channels" bind:this={tree}>
      {#each app.orderedConns as conn (conn.id)}
        {@const label = statusLabel(conn)}
        {@const id = 'n' + conn.id}
        {@const kids = conn.channelBuffers.length > 0}
        <li
          class="net-item"
          class:control={conn.isControl}
          class:child={!!conn.parentId}
          role="treeitem"
          aria-level={1}
          aria-selected={app.active === conn.server}
          aria-expanded={kids ? conn.isControl || !conn.collapsed : undefined}
          aria-label={netLabel(conn, label)}
          tabindex={roving === id ? 0 : -1}
          data-tid={tid(id)}
          onfocus={(e) => e.target === e.currentTarget && (focusId = id)}
          onclick={() => app.open(conn.server)}
          onkeydown={(e) => onItemKey(e, { id, conn })}
          oncontextmenu={(e) => openMenu(e, conn)}
        >
          {#if conn.isControl}
            <div class="group" class:active={app.active === conn.server} title="Bouncer console">
              <span class="eyebrow">{conn.name}</span>
              {#if label}<span class="state {conn.status}">{label}</span>{/if}
            </div>
            {#if conn.buffers.size}<div class="control-bufs">{@render buffers(conn)}</div>{/if}
          {:else}
            <div class="net-row">
              <div class="net" class:active={app.active === conn.server}>
                <span class="dot {conn.status}"></span>
                <span class="net-name">{conn.name}</span>
                {#if label}<span class="state {conn.status}">{label}</span>{/if}
                {#if conn.collapsed}
                  {@const hl = conn.channelBuffers.reduce((n, b) => n + b.highlights, 0)}
                  {#if hl}<span class="badge">{hl}</span>{/if}
                {/if}
              </div>
              <!-- Left/Right arrows fold from the keyboard. -->
              <span
                class="fold"
                class:folded={conn.collapsed}
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
      {/each}
    </ul>
  </nav>

  <footer>
    <button
      class="icon-btn"
      title="Add network"
      aria-label="Add network"
      onclick={() => (app.settingsOpen = { tab: 'servers', serverId: 'new' })}
    >
      <Icon name="plus" />
    </button>
    <div class="me">
      {#if app.active}
        {@const c = app.active.conn}
        <span class="dot {c.status}" class:away={c.away}></span>
        <span class="me-nick">{c.nick || '—'}</span>
        {#if c.away}<span class="me-away">away</span>{/if}
        <span class="sr-only">, {c.status}</span>
      {/if}
    </div>
    <button class="icon-btn" title="Settings" aria-label="Settings" onclick={() => (app.settingsOpen = { tab: 'appearance' })}>
      <Icon name="settings" />
    </button>
  </footer>

  <div
    class="grip"
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

{#if menu}
  {@const close = () => (menu = null)}
  <div
    class="menu"
    bind:this={menuEl}
    style="left:{menuPos.left || menu.x}px; top:{menuPos.top || menu.y}px"
    role="menu"
    tabindex="-1"
    aria-label={menu.buf ? menu.buf.name : menu.conn?.name}
    use:menuNav={{ onclose: close }}
  >
    {#if menu.buf}
      {@const b = menu.buf}
      <button role="menuitem" onclick={() => app.markRead(b)}>Mark as read</button>
      {#if b.kind === 'channel' && !b.joined}
        <button role="menuitem" onclick={() => app.join(b.conn, b.name)}>Rejoin</button>
      {/if}
      {#if b.kind === 'query' && b.conn.canMonitor}
        {@const watched = app.isWatched(b.conn, b.name)}
        <button role="menuitem" onclick={() => (watched ? app.unwatch(b.conn, b.name) : app.watch(b.conn, b.name))}>
          {watched ? 'Stop watching' : 'Tell me when online'}
        </button>
      {/if}
      <hr />
      <div role="group" aria-labelledby="notify-label">
        <p class="menu-label" id="notify-label">Notify me about</p>
        {#each LEVELS as l (l.value)}
          <button class="check" role="menuitemradio" aria-checked={b.level === l.value} onclick={() => app.setNotify(b, l.value)}>
            <span class="tick">{#if b.level === l.value}<Icon name="check" size={13} />{/if}</span>{l.label}
          </button>
        {/each}
      </div>
      <hr />
      <button role="menuitem" onclick={() => app.closeBuffer(b)}>{b.kind === 'channel' && b.joined ? 'Leave channel' : 'Close'}</button>
    {:else if menu.conn}
      {@const c = menu.conn}
      <button role="menuitem" onclick={() => app.open(c.server)}>Open server log</button>
      {#if c.isControl}
        <button role="menuitem" onclick={() => (app.settingsOpen = { tab: 'servers', serverId: c.serverId })}>Manage networks…</button>
      {:else if c.status === 'connected'}
        <button role="menuitem" onclick={() => app.browseChannels(c)}>Browse channels…</button>
      {/if}
      <button role="menuitem" onclick={() => api.reconnect(c.serverId)}>Reconnect</button>
      {#if c.status === 'disconnected' && !c.retryAt}
        <button role="menuitem" onclick={() => api.connect(c.serverId)}>Connect</button>
      {:else}
        <button role="menuitem" onclick={() => api.disconnect(c.serverId)}>Disconnect</button>
      {/if}
      <hr />
      <button role="menuitem" onclick={() => (app.settingsOpen = { tab: 'servers', serverId: c.serverId })}>Edit server…</button>
    {/if}
  </div>
{/if}

<style>
  aside {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--bg-side);
    border-right: 1px solid var(--line);
  }

  .top {
    display: flex;
    gap: 4px;
    padding: 10px 10px 6px;
  }

  aside.mac .top {
    padding-top: 42px;
  }

  .find {
    flex: 1;
    min-width: 0;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px 0 10px;
    border-radius: var(--radius);
    color: var(--text-3);
    font-size: 13px;
    box-shadow: 0 0 0 1px var(--line) inset;
    transition: background 120ms, color 120ms;
  }

  .find:hover {
    background: var(--bg-hover);
    color: var(--text-2);
  }

  .find span:first-of-type {
    flex: 1;
    text-align: left;
  }

  nav {
    flex: 1;
    overflow-y: auto;
    padding: 6px 8px 16px;
  }

  .group {
    display: flex;
    align-items: baseline;
    gap: 8px;
    width: 100%;
    padding: 14px 8px 4px;
    text-align: left;
  }

  .group:hover .eyebrow,
  .group.active .eyebrow {
    color: var(--text);
  }

  .tree,
  .tree :global(ul) {
    list-style: none;
    margin: 0;
    padding: 0;
  }

  .net-item {
    margin-top: 10px;
  }

  .net-item.child {
    margin-top: 6px;
  }

  .net-item.control {
    margin-top: 0;
  }

  /* The item holds its channels too; ring just its own row. */
  .net-item:focus-visible {
    outline: none;
  }

  .net-item:focus-visible > .net-row > .net,
  .net-item:focus-visible > .group {
    outline: 2px solid var(--brand);
    outline-offset: -2px;
  }

  .grip:focus-visible {
    outline: none;
    background: var(--brand);
    opacity: 0.6;
  }

  .net-row {
    display: flex;
    align-items: center;
  }

  .net {
    flex: 1;
    min-width: 0;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 8px;
    border-radius: 6px;
    font-weight: 600;
    font-size: 13px;
    color: var(--text-2);
  }

  .net:hover {
    color: var(--text);
  }

  .net.active {
    background: var(--bg-active);
    color: var(--text);
  }

  .net-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .fold {
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    color: var(--text-4);
    border-radius: 5px;
    opacity: 0;
    transition: opacity 120ms, transform 160ms var(--ease);
  }

  .net-row:hover .fold {
    opacity: 1;
  }

  .fold:hover {
    color: var(--text-2);
  }

  .fold.folded {
    opacity: 1;
    transform: rotate(-90deg);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex: none;
    background: var(--text-4);
  }

  .dot.connected {
    background: var(--ok);
    opacity: 0.8;
  }

  .dot.connecting,
  .dot.registering {
    background: var(--warn);
    animation: pulse 1.2s ease-in-out infinite;
  }

  .dot.disconnected {
    background: var(--danger);
  }

  .dot.away {
    background: var(--warn);
  }

  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }

  .state {
    margin-left: auto;
    font-size: 11px;
    font-weight: 400;
    color: var(--text-3);
    white-space: nowrap;
  }

  .state.disconnected {
    color: var(--danger);
  }

  .buf {
    position: relative;
    width: 100%;
    height: 28px;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 6px 0 10px;
    border-radius: 6px;
    color: var(--text-3);
    font-size: 13.5px;
    transition: background 80ms, color 80ms;
  }

  .buf:hover {
    background: var(--bg-hover);
    color: var(--text-2);
  }

  .buf.unread {
    color: var(--text);
    font-weight: 500;
  }

  .buf.active {
    background: var(--bg-active);
    color: var(--text);
  }

  .buf.active::before {
    content: '';
    position: absolute;
    left: 0;
    top: 7px;
    bottom: 7px;
    width: 2px;
    border-radius: 2px;
    background: var(--brand);
  }

  .buf.muted {
    opacity: 0.6;
  }

  .buf.muted.unread {
    font-weight: 400;
  }

  .mute {
    display: grid;
    place-items: center;
    color: var(--text-4);
    padding-right: 3px;
  }

  .buf:hover .mute {
    display: none;
  }

  .glyph {
    position: relative;
  }

  .presence {
    position: absolute;
    right: -2px;
    bottom: -1px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-4);
    box-shadow: 0 0 0 1.5px var(--bg-side);
  }

  .presence.online {
    background: var(--ok);
  }

  .menu-label {
    margin: 2px 10px 2px;
    font-size: 11px;
    color: var(--text-3);
  }

  .menu .check {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .tick {
    width: 14px;
    display: grid;
    place-items: center;
  }

  .buf.parted .label {
    opacity: 0.55;
    font-style: italic;
  }

  .glyph {
    width: 14px;
    flex: none;
    display: grid;
    place-items: center;
    font-family: var(--font-mono);
    font-size: 13px;
    color: var(--text-4);
    font-weight: 400;
  }

  .buf.active .glyph,
  .buf.unread .glyph {
    color: var(--text-3);
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
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
    font-variant-numeric: tabular-nums;
  }

  .close {
    display: none;
    width: 20px;
    height: 20px;
    place-items: center;
    border-radius: 4px;
    color: var(--text-3);
  }

  .buf:hover .close {
    display: grid;
  }

  .buf:hover .badge,
  .buf:hover .typing-dots {
    display: none;
  }

  .close:hover {
    background: var(--bg-active);
    color: var(--text);
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
    background: var(--text-3);
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

  footer {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 8px;
    border-top: 1px solid var(--line);
  }

  .me {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 6px;
    font-size: 12.5px;
    color: var(--text-2);
  }

  .me-nick {
    font-family: var(--font-mono);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .me-away {
    font-size: 11px;
    color: var(--warn);
  }

  .grip {
    position: absolute;
    top: 0;
    right: -3px;
    width: 6px;
    height: 100%;
    cursor: col-resize;
    z-index: 5;
  }

  .menu {
    position: fixed;
    z-index: 60;
    min-width: 180px;
    padding: 4px;
    background: var(--bg-raised);
    border-radius: 8px;
    box-shadow: var(--shadow);
    display: grid;
  }

  .menu button {
    text-align: left;
    padding: 6px 10px;
    border-radius: 5px;
    font-size: 13px;
  }

  .menu button:hover {
    background: var(--brand);
    color: var(--brand-text);
  }

  .menu hr {
    border: 0;
    border-top: 1px solid var(--line);
    margin: 4px 0;
  }
</style>
