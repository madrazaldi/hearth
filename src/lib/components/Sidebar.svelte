<script lang="ts">
  import { app, type Buffer, type Conn } from '../state.svelte';
  import { api } from '../api';
  import Icon from './Icon.svelte';

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

  function openMenu(e: MouseEvent, conn?: Conn, buf?: Buffer) {
    e.preventDefault();
    menu = { x: e.clientX, y: e.clientY, conn, buf };
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
  {#each conn.channelBuffers as b (b.id)}
    <button
      class="buf"
      class:active={app.active === b}
      class:unread={b.unread > 0}
      class:parted={b.kind === 'channel' && !b.joined}
      onclick={() => app.open(b)}
      oncontextmenu={(e) => openMenu(e, conn, b)}
    >
      <span class="glyph">
        {#if b.kind === 'channel'}#{:else}<Icon name="user" size={12} stroke={1.6} />{/if}
      </span>
      <span class="label">{name(b)}</span>
      {#if b.highlights > 0}
        <span class="badge">{b.highlights}</span>
      {:else if b.typing.size > 0 && b.kind === 'query'}
        <span class="typing-dots"><i></i><i></i><i></i></span>
      {/if}
      <span
        class="close"
        role="button"
        tabindex="-1"
        title={b.kind === 'channel' ? 'Leave' : 'Close'}
        onclick={(e) => {
          e.stopPropagation();
          app.closeBuffer(b);
        }}
        onkeydown={() => {}}
      >
        <Icon name="x" size={12} />
      </span>
    </button>
  {/each}
{/snippet}

<svelte:window onclick={() => (menu = null)} onblur={() => (menu = null)} />

<aside class:mac={isMac}>
  <div class="top" data-tauri-drag-region>
    <button class="find" onclick={() => (app.switcherOpen = true)} title="Jump to…">
      <Icon name="search" size={14} />
      <span>Jump to…</span>
      <span class="kbd">{isMac ? '⌘' : 'Ctrl '}K</span>
    </button>
  </div>

  <nav>
    {#each app.orderedConns as conn (conn.id)}
      {@const label = statusLabel(conn)}
      {#if conn.isControl}
        <button
          class="group"
          class:active={app.active === conn.server}
          onclick={() => app.open(conn.server)}
          oncontextmenu={(e) => openMenu(e, conn)}
          title="Bouncer console"
        >
          <span class="eyebrow">{conn.name}</span>
          {#if label}<span class="state {conn.status}">{label}</span>{/if}
        </button>
        {#if conn.buffers.size}<div class="control-bufs">{@render buffers(conn)}</div>{/if}
      {:else}
        <section class:child={!!conn.parentId}>
          <div class="net-row">
            <button
              class="net"
              class:active={app.active === conn.server}
              onclick={() => app.open(conn.server)}
              oncontextmenu={(e) => openMenu(e, conn)}
            >
              <span class="dot {conn.status}"></span>
              <span class="net-name">{conn.name}</span>
              {#if label}<span class="state {conn.status}">{label}</span>{/if}
              {#if conn.collapsed}
                {@const hl = conn.channelBuffers.reduce((n, b) => n + b.highlights, 0)}
                {#if hl}<span class="badge">{hl}</span>{/if}
              {/if}
            </button>
            <button
              class="fold"
              class:folded={conn.collapsed}
              onclick={() => (conn.collapsed = !conn.collapsed)}
              aria-label={conn.collapsed ? 'Expand' : 'Collapse'}
            >
              <Icon name="down" size={13} />
            </button>
          </div>
          {#if !conn.collapsed}
            {@render buffers(conn)}
          {/if}
        </section>
      {/if}
    {/each}
  </nav>

  <footer>
    <button class="icon-btn" title="Add network" onclick={() => (app.settingsOpen = { tab: 'servers', serverId: 'new' })}>
      <Icon name="plus" />
    </button>
    <div class="me">
      {#if app.active}
        <span class="dot {app.active.conn.status}" class:away={app.active.conn.away}></span>
        <span class="me-nick">{app.active.conn.nick || '—'}</span>
        {#if app.active.conn.away}<span class="me-away">away</span>{/if}
      {/if}
    </div>
    <button class="icon-btn" title="Settings" onclick={() => (app.settingsOpen = { tab: 'appearance' })}>
      <Icon name="settings" />
    </button>
  </footer>

  <div class="grip" onpointerdown={resize} role="separator" aria-orientation="vertical"></div>
</aside>

{#if menu}
  <div class="menu" style="left:{menu.x}px; top:{menu.y}px" role="menu">
    {#if menu.buf}
      {@const b = menu.buf}
      <button onclick={() => app.markRead(b)}>Mark as read</button>
      {#if b.kind === 'channel' && !b.joined}
        <button onclick={() => app.join(b.conn, b.name)}>Rejoin</button>
      {/if}
      <button onclick={() => app.closeBuffer(b)}>{b.kind === 'channel' && b.joined ? 'Leave channel' : 'Close'}</button>
    {:else if menu.conn}
      {@const c = menu.conn}
      <button onclick={() => app.open(c.server)}>Open server log</button>
      <button onclick={() => api.reconnect(c.serverId)}>Reconnect</button>
      {#if c.status === 'disconnected' && !c.retryAt}
        <button onclick={() => api.connect(c.serverId)}>Connect</button>
      {:else}
        <button onclick={() => api.disconnect(c.serverId)}>Disconnect</button>
      {/if}
      <hr />
      <button onclick={() => (app.settingsOpen = { tab: 'servers', serverId: c.serverId })}>Edit server…</button>
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
    padding: 10px 10px 6px;
  }

  aside.mac .top {
    padding-top: 42px;
  }

  .find {
    width: 100%;
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

  section {
    margin-top: 10px;
  }

  section.child {
    margin-top: 6px;
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
    background: var(--accent);
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
    background: var(--accent);
    color: var(--accent-text);
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
    background: var(--accent);
    color: var(--accent-text);
  }

  .menu hr {
    border: 0;
    border-top: 1px solid var(--line);
    margin: 4px 0;
  }
</style>
