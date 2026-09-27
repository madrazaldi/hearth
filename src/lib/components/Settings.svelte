<script lang="ts">
  import { app } from '../state.svelte';
  import { api } from '../api';
  import { defaultServer, type ServerConfig, type Settings } from '../types';
  import Icon from './Icon.svelte';
  import Switch from './Switch.svelte';
  import Segmented from './Segmented.svelte';
  import ServerForm from './ServerForm.svelte';
  import BouncerNetworks from './BouncerNetworks.svelte';
  import { updater } from '../updater.svelte';

  const isMac = navigator.userAgent.includes('Mac');
  const mod = isMac ? '⌘' : 'Ctrl';

  const tabs = [
    { id: 'servers', label: 'Servers', icon: 'server' },
    { id: 'general', label: 'General', icon: 'settings' },
    { id: 'appearance', label: 'Appearance', icon: 'moon' },
    { id: 'notifications', label: 'Notifications', icon: 'bell' },
    { id: 'privacy', label: 'Privacy & data', icon: 'lock' },
    { id: 'shortcuts', label: 'Shortcuts', icon: 'layers' },
  ];

  const tab = $derived(app.settingsOpen?.tab ?? 'appearance');
  const editing = $derived(app.settingsOpen?.serverId);
  const editServer = $derived<ServerConfig | null>(
    editing === 'new' ? defaultServer() : (app.servers.find((s) => s.id === editing) ?? null),
  );
  let words = $state(app.settings.highlightWords.join(', '));
  let newIgnore = $state('');
  let confirmClear = $state(false);
  const bouncer = $derived(editing && editing !== 'new' ? app.conns.get(editing) : undefined);

  async function clearLogs() {
    confirmClear = false;
    try {
      await api.clearLogs();
      app.toast('Saved messages deleted');
    } catch (e) {
      app.toast(String(e), 'error');
    }
  }

  function addIgnore(e: SubmitEvent) {
    e.preventDefault();
    if (newIgnore.trim()) app.ignore(newIgnore);
    newIgnore = '';
  }

  function set<K extends keyof Settings>(key: K) {
    return {
      get: () => app.settings[key],
      set: (v: Settings[K]) => app.updateSettings({ [key]: v } as Partial<Settings>),
    };
  }

  const theme = set('theme');
  const layout = set('layout');
  const fontSize = set('fontSize');
  const clock = set('clock');
  const membership = set('membership');
  const autoAway = set('autoAwayMinutes');

  function go(t: string, serverId?: string) {
    app.settingsOpen = { tab: t, serverId };
  }

  function close() {
    app.settingsOpen = null;
  }

  function statusOf(id: string) {
    const c = app.conns.get(id);
    if (!c) return 'offline';
    return c.status;
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && close()} />

<div class="scrim" onclick={close} role="presentation">
  <div class="sheet" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Settings" tabindex="-1" onkeydown={() => {}}>
    <nav>
      <p class="eyebrow">Settings</p>
      {#each tabs as t (t.id)}
        <button class:on={tab === t.id} onclick={() => go(t.id)}>
          <Icon name={t.icon} size={15} />{t.label}
        </button>
      {/each}
      <div class="about">
        <p>Hearth <span>{updater.current || '0.1.0'}</span></p>
      </div>
    </nav>

    <section>
      <button class="icon-btn close" onclick={close} aria-label="Close"><Icon name="x" /></button>

      {#if tab === 'servers'}
        {#if editServer}
          <h2>{editing === 'new' ? 'Add a server' : `Edit ${editServer.name}`}</h2>
          {#key editing}
            <ServerForm
              server={editServer}
              submitLabel={editing === 'new' ? 'Add & connect' : 'Save & reconnect'}
              oncancel={() => go('servers')}
              ondelete={editing !== 'new'
                ? async () => {
                    await app.deleteServer(editServer.id);
                    go('servers');
                  }
                : undefined}
              onsave={async (s) => {
                await app.saveServer(s, true);
                go('servers');
              }}
            />
          {/key}
          {#if bouncer?.isControl}
            <BouncerNetworks conn={bouncer} />
          {/if}
        {:else}
          <h2>Servers</h2>
          <p class="lede">Networks and bouncers Hearth connects to. A soju bouncer expands into all of its networks automatically.</p>
          <div class="servers">
            {#each app.servers as s (s.id)}
              {@const st = statusOf(s.id)}
              <div class="server">
                <span class="dot {st}"></span>
                <div class="server-main">
                  <div class="server-name">{s.name}</div>
                  <div class="server-host mono">{s.websocket ? (s.tls ? 'wss://' : 'ws://') : ''}{s.host}:{s.port}{s.websocket ? s.websocketPath : ''}</div>
                </div>
                {#if st === 'offline' || st === 'disconnected'}
                  <button class="btn ghost" onclick={() => api.connect(s.id)}>Connect</button>
                {:else}
                  <button class="btn ghost" onclick={() => api.disconnect(s.id)}>Disconnect</button>
                {/if}
                <button class="btn" onclick={() => go('servers', s.id)}>Edit</button>
              </div>
            {/each}
          </div>
          <button class="btn primary add" onclick={() => go('servers', 'new')}><Icon name="plus" size={14} />Add server</button>
        {/if}
      {:else if tab === 'general'}
        <h2>General</h2>
        <div class="rows">
          <Switch
            bind:checked={() => app.settings.closeToTray, (v) => app.updateSettings({ closeToTray: v })}
            label="Keep running when the window is closed"
            hint="Hearth stays connected in the menu bar or system tray. Quit from the tray icon."
          />
          <div class="row">
            <div>
              <div class="label">Mark me away</div>
              <div class="hint">When Hearth has been in the background this long. You're marked back when you return.</div>
            </div>
            <Segmented
              bind:value={autoAway.get, autoAway.set}
              options={[
                { value: 0, label: 'Never' },
                { value: 10, label: '10 min' },
                { value: 30, label: '30 min' },
                { value: 60, label: '1 hour' },
              ]}
            />
          </div>
          <Switch
            bind:checked={() => app.settings.checkUpdates, (v) => app.updateSettings({ checkUpdates: v })}
            label="Check for updates automatically"
            hint="Looks for a new release when Hearth starts and every few hours. Nothing is installed without asking."
          />
          <div class="row">
            <div>
              <div class="label">Hearth {updater.current}</div>
              <div class="hint" class:error={updater.phase === 'error'}>
                {#if updater.phase === 'checking'}
                  Checking for updates…
                {:else if updater.phase === 'current'}
                  You're up to date.
                {:else if updater.phase === 'available'}
                  Version {updater.version} is available.
                {:else if updater.phase === 'installing'}
                  Installing {updater.version}{updater.progress != null ? ` · ${Math.round(updater.progress * 100)}%` : '…'}
                {:else if updater.phase === 'error'}
                  Couldn't update: {updater.error}
                {:else}
                  Updates come from GitHub releases and are verified before installing.
                {/if}
              </div>
            </div>
            {#if updater.phase === 'available' || updater.phase === 'installing'}
              <button class="btn primary" disabled={updater.busy} onclick={() => updater.installAndRestart()}>Install and restart</button>
            {:else}
              <button class="btn" disabled={updater.busy} onclick={() => updater.check()}>Check now</button>
            {/if}
          </div>
        </div>
      {:else if tab === 'privacy'}
        <h2>Privacy & data</h2>
        <div class="rows">
          <Switch
            bind:checked={() => app.settings.storeLogs, (v) => app.updateSettings({ storeLogs: v })}
            label="Save messages on this computer"
            hint="Keeps scrollback across restarts and makes messages searchable. Stored only on this device."
          />
          <div class="row">
            <div>
              <div class="label">Saved messages</div>
              <div class="hint">Deleting them doesn't affect history kept by a server or bouncer.</div>
            </div>
            {#if confirmClear}
              <button class="btn danger" onclick={clearLogs}>Delete all?</button>
              <button class="btn ghost" onclick={() => (confirmClear = false)}>Cancel</button>
            {:else}
              <button class="btn ghost danger" onclick={() => (confirmClear = true)}>Delete saved messages</button>
            {/if}
          </div>
          <Switch
            bind:checked={() => app.settings.linkPreviews, (v) => app.updateSettings({ linkPreviews: v })}
            label="Show link previews"
            hint="Hearth fetches linked pages and images itself, so those sites see your IP address. Links to local or private addresses are never fetched."
          />
          <div class="field">
            <span>Ignored people</span>
            {#if app.settings.ignores.length}
              <div class="ignores">
                {#each app.settings.ignores as mask (mask)}
                  <div class="ignore">
                    <span class="mono">{mask}</span>
                    <button class="icon-btn" title="Stop ignoring" onclick={() => app.unignore(mask)}><Icon name="x" size={13} /></button>
                  </div>
                {/each}
              </div>
            {/if}
            <form class="add-ignore" onsubmit={addIgnore}>
              <input class="input mono" bind:value={newIgnore} placeholder="nick or nick!user@host" spellcheck="false" />
              <button class="btn" type="submit" disabled={!newIgnore.trim()}>Ignore</button>
            </form>
            <small>Hides their messages, notices, invites and reactions everywhere. <code>*</code> and <code>?</code> are wildcards.</small>
          </div>
        </div>
      {:else if tab === 'appearance'}
        <h2>Appearance</h2>
        <div class="rows">
          <div class="row">
            <div><div class="label">Theme</div></div>
            <Segmented
              bind:value={theme.get, theme.set}
              options={[
                { value: 'system', label: 'System' },
                { value: 'light', label: 'Light' },
                { value: 'dark', label: 'Dark' },
              ]}
            />
          </div>
          <div class="row">
            <div>
              <div class="label">Message layout</div>
              <div class="hint">Columns aligns nicks in a gutter; compact stacks them above messages.</div>
            </div>
            <Segmented
              bind:value={layout.get, layout.set}
              options={[
                { value: 'columns', label: 'Columns' },
                { value: 'compact', label: 'Compact' },
              ]}
            />
          </div>
          <div class="row">
            <div><div class="label">Text size</div></div>
            <Segmented
              bind:value={fontSize.get, fontSize.set}
              options={[
                { value: 13, label: 'S' },
                { value: 14, label: 'M' },
                { value: 15, label: 'L' },
                { value: 16, label: 'XL' },
              ]}
            />
          </div>
          <div class="row">
            <div><div class="label">Clock</div></div>
            <Segmented
              bind:value={clock.get, clock.set}
              options={[
                { value: '24h', label: '24-hour' },
                { value: '12h', label: '12-hour' },
              ]}
            />
          </div>
          <div class="row">
            <div>
              <div class="label">Joins, parts and quits</div>
              <div class="hint">Collapse folds consecutive events into one line.</div>
            </div>
            <Segmented
              bind:value={membership.get, membership.set}
              options={[
                { value: 'show', label: 'Show' },
                { value: 'collapse', label: 'Collapse' },
                { value: 'hide', label: 'Hide' },
              ]}
            />
          </div>
          <Switch bind:checked={() => app.settings.showSeconds, (v) => app.updateSettings({ showSeconds: v })} label="Show seconds in timestamps" />
          <Switch bind:checked={() => app.settings.nickColors, (v) => app.updateSettings({ nickColors: v })} label="Colored nicknames" />
          <Switch
            bind:checked={() => app.settings.showTyping, (v) => app.updateSettings({ showTyping: v })}
            label="Show when others are typing"
          />
          <Switch
            bind:checked={() => app.settings.sendTyping, (v) => app.updateSettings({ sendTyping: v })}
            label="Let others see when I'm typing"
          />
        </div>
      {:else if tab === 'notifications'}
        <h2>Notifications</h2>
        <div class="rows">
          <Switch
            bind:checked={() => app.settings.notifyHighlights, (v) => app.updateSettings({ notifyHighlights: v })}
            label="Mentions"
            hint="When someone says your nickname or a highlight word."
          />
          <Switch
            bind:checked={() => app.settings.notifyPrivate, (v) => app.updateSettings({ notifyPrivate: v })}
            label="Private messages"
          />
          <label class="field">
            <span>Highlight words</span>
            <input
              class="input"
              bind:value={words}
              onchange={() => app.updateSettings({ highlightWords: words.split(',').map((w) => w.trim()).filter(Boolean) })}
              placeholder="hearth, deploy"
            />
            <small>Comma-separated. Your nickname always counts.</small>
          </label>
        </div>
      {:else if tab === 'shortcuts'}
        <h2>Shortcuts</h2>
        <dl class="keys">
          <dt><span class="kbd">{mod} K</span></dt><dd>Jump to buffer</dd>
          <dt><span class="kbd">⌥ ↑</span> <span class="kbd">⌥ ↓</span></dt><dd>Previous / next buffer</dd>
          <dt><span class="kbd">⌥ ⇧ ↑</span> <span class="kbd">⌥ ⇧ ↓</span></dt><dd>Previous / next unread</dd>
          <dt><span class="kbd">{mod} 1–9</span></dt><dd>Go to channel by position</dd>
          <dt><span class="kbd">{mod} F</span></dt><dd>Filter messages</dd>
          <dt><span class="kbd">{mod} ⇧ F</span></dt><dd>Search all messages</dd>
          <dt><span class="kbd">{mod} W</span></dt><dd>Close buffer</dd>
          <dt><span class="kbd">{mod} ⇧ M</span></dt><dd>Toggle member list</dd>
          <dt><span class="kbd">{mod} ,</span></dt><dd>Settings</dd>
          <dt><span class="kbd">Tab</span></dt><dd>Complete nick, channel or command</dd>
          <dt><span class="kbd">↑</span> <span class="kbd">↓</span></dt><dd>Input history</dd>
          <dt><span class="kbd">⇧ ↵</span></dt><dd>New line</dd>
        </dl>
      {/if}
    </section>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 90;
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
    width: min(860px, 94vw);
    height: min(640px, 88vh);
    display: grid;
    grid-template-columns: 200px 1fr;
    background: var(--bg);
    border-radius: 14px;
    box-shadow: var(--shadow);
    overflow: hidden;
    animation: rise 180ms var(--ease);
  }

  @keyframes rise {
    from {
      transform: translateY(8px) scale(0.995);
      opacity: 0;
    }
  }

  nav {
    background: var(--bg-side);
    border-right: 1px solid var(--line);
    padding: 20px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  nav .eyebrow {
    margin: 0 10px 10px;
  }

  nav button {
    height: 32px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 10px;
    border-radius: 7px;
    font-size: 13.5px;
    color: var(--text-2);
    text-align: left;
  }

  nav button:hover {
    background: var(--bg-hover);
    color: var(--text);
  }

  nav button.on {
    background: var(--bg-active);
    color: var(--text);
  }

  .about {
    margin-top: auto;
    padding: 0 10px;
    font-size: 12px;
    color: var(--text-3);
  }

  .about span {
    color: var(--text-4);
    font-family: var(--font-mono);
  }

  section {
    position: relative;
    overflow-y: auto;
    padding: 28px 36px 36px;
    user-select: none;
  }

  .close {
    position: absolute;
    top: 14px;
    right: 14px;
  }

  h2 {
    margin: 0 0 6px;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.015em;
  }

  .lede {
    margin: 0 0 20px;
    color: var(--text-3);
    font-size: 13px;
    max-width: 52ch;
  }

  h2 + :global(form),
  h2 + .rows,
  h2 + .keys {
    margin-top: 20px;
  }

  .servers {
    display: grid;
    border-radius: 10px;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
    overflow: hidden;
  }

  .server {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 14px;
  }

  .server + .server {
    border-top: 1px solid var(--line);
  }

  .server-main {
    flex: 1;
    min-width: 0;
  }

  .server-name {
    font-weight: 500;
  }

  .server-host {
    font-size: 12px;
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-4);
  }

  .dot.connected {
    background: var(--ok);
  }

  .dot.connecting,
  .dot.registering {
    background: var(--warn);
  }

  .dot.disconnected {
    background: var(--danger);
  }

  .add {
    margin-top: 16px;
  }

  .rows {
    display: grid;
    gap: 10px;
    max-width: 560px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 6px 0;
  }

  .row > div:first-child {
    flex: 1;
  }

  .label {
    font-size: 13.5px;
  }

  .hint {
    font-size: 12px;
    color: var(--text-3);
  }

  .hint.error {
    color: var(--danger);
  }

  .ignores {
    display: grid;
    border-radius: 8px;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
    overflow: hidden;
  }

  .ignore {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 3px 4px 3px 12px;
    font-size: 12.5px;
  }

  .ignore + .ignore {
    border-top: 1px solid var(--line);
  }

  .add-ignore {
    display: flex;
    gap: 8px;
  }

  .add-ignore .input {
    flex: 1;
  }

  code {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }

  .keys {
    display: grid;
    grid-template-columns: 180px 1fr;
    gap: 10px 16px;
    font-size: 13.5px;
  }

  .keys dt {
    display: flex;
    gap: 4px;
  }

  .keys dd {
    margin: 0;
    color: var(--text-2);
  }
</style>
