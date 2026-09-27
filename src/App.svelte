<script lang="ts">
  import { onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { app } from './lib/state.svelte';
  import { updater } from './lib/updater.svelte';
  import Sidebar from './lib/components/Sidebar.svelte';
  import BufferView from './lib/components/BufferView.svelte';
  import QuickSwitcher from './lib/components/QuickSwitcher.svelte';
  import SearchPanel from './lib/components/SearchPanel.svelte';
  import ChannelBrowser from './lib/components/ChannelBrowser.svelte';
  import Settings from './lib/components/Settings.svelte';
  import Welcome from './lib/components/Welcome.svelte';

  const isMac = navigator.userAgent.includes('Mac');
  let systemDark = $state(window.matchMedia('(prefers-color-scheme: dark)').matches);
  let bootError = $state<string | null>(null);
  // Latched so the welcome flow can show connection progress after the first
  // server is saved, until the user moves on.
  let onboarding = $state(false);
  $effect(() => {
    if (app.ready && app.servers.length === 0) onboarding = true;
  });

  const theme = $derived(app.settings.theme === 'system' ? (systemDark ? 'dark' : 'light') : app.settings.theme);

  $effect(() => {
    const root = document.documentElement;
    root.dataset.theme = theme;
    root.style.setProperty('--fs', `${app.settings.fontSize}px`);
    root.style.setProperty('--sidebar-w', `${app.settings.sidebarWidth}px`);
    root.classList.toggle('mac', isMac);
    root.classList.toggle('no-nick-colors', !app.settings.nickColors);
  });

  // Dock badge: mentions and private messages waiting.
  const attention = $derived(app.allBuffers.reduce((n, b) => n + b.highlights, 0));
  $effect(() => {
    const n = attention;
    getCurrentWindow()
      .setBadgeCount(n > 0 ? n : undefined)
      .catch(() => {});
  });

  onMount(() => {
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const onScheme = () => (systemDark = mq.matches);
    mq.addEventListener('change', onScheme);
    const onFocus = () => app.setFocused(true);
    const onBlur = () => app.setFocused(false);
    window.addEventListener('focus', onFocus);
    window.addEventListener('blur', onBlur);
    app
      .init()
      .then(() => updater.start())
      .catch((e) => (bootError = String(e)));
    return () => {
      mq.removeEventListener('change', onScheme);
      window.removeEventListener('focus', onFocus);
      window.removeEventListener('blur', onBlur);
    };
  });

  function onKey(e: KeyboardEvent) {
    const mod = isMac ? e.metaKey : e.ctrlKey;
    if (mod && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      app.switcherOpen = !app.switcherOpen;
    } else if (mod && e.shiftKey && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      e.stopPropagation();
      app.searchOpen = !app.searchOpen;
    } else if (mod && e.key === ',') {
      e.preventDefault();
      app.settingsOpen = app.settingsOpen ? null : { tab: 'appearance' };
    } else if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      app.step(e.key === 'ArrowUp' ? -1 : 1, e.shiftKey);
    } else if (mod && /^[1-9]$/.test(e.key)) {
      e.preventDefault();
      const list = app.allBuffers.filter((b) => b.kind !== 'server');
      const b = list[parseInt(e.key) - 1];
      if (b) app.open(b);
    } else if (mod && e.key.toLowerCase() === 'w' && app.active && app.active.kind !== 'server') {
      e.preventDefault();
      app.closeBuffer(app.active);
    } else if (mod && e.shiftKey && e.key.toLowerCase() === 'm') {
      e.preventDefault();
      app.updateSettings({ showMembers: !app.settings.showMembers });
    }
  }
</script>

<svelte:window onkeydown={onKey} />

{#if bootError}
  <div class="boot-error">
    <p class="eyebrow">Hearth failed to start</p>
    <pre class="mono selectable">{bootError}</pre>
  </div>
{:else if app.ready && (app.servers.length === 0 || onboarding)}
  <Welcome ondone={() => (onboarding = false)} />
{:else}
  <div class="shell">
    <Sidebar />
    <main>
      {#if app.active}
        {#key app.active.id}
          <BufferView buf={app.active} />
        {/key}
      {:else}
        <div class="empty" data-tauri-drag-region>
          <p>{app.ready ? 'Connecting…' : ''}</p>
        </div>
      {/if}
    </main>
  </div>
{/if}

{#if app.switcherOpen}
  <QuickSwitcher />
{/if}
{#if app.searchOpen}
  <SearchPanel />
{/if}
{#if app.browserConn}
  <ChannelBrowser conn={app.browserConn} />
{/if}
{#if app.settingsOpen}
  <Settings />
{/if}

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div class="toast" class:error={t.kind === 'error'} class:has-action={t.action}>
      {t.text}
      {#if t.action}
        {@const action = t.action}
        <button class="btn primary" onclick={() => (app.dismissToast(t.id), action.run())}>{action.label}</button>
        <button class="btn ghost" onclick={() => app.dismissToast(t.id)}>Not now</button>
      {/if}
    </div>
  {/each}
</div>

<style>
  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr);
    height: 100%;
  }

  main {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }

  .empty {
    flex: 1;
    display: grid;
    place-items: center;
    color: var(--text-3);
  }

  .boot-error {
    padding: 64px;
  }

  .boot-error pre {
    white-space: pre-wrap;
    color: var(--danger);
  }

  .toasts {
    position: fixed;
    bottom: 88px;
    left: 50%;
    transform: translateX(-50%);
    display: grid;
    gap: 8px;
    z-index: 100;
    pointer-events: none;
  }

  .toast {
    background: var(--bg-raised);
    box-shadow: var(--shadow);
    padding: 8px 14px;
    border-radius: 8px;
    font-size: 13px;
    color: var(--text);
    animation: rise 180ms var(--ease);
  }

  .toast.has-action {
    display: flex;
    align-items: center;
    gap: 10px;
    pointer-events: auto;
  }

  .toast.error {
    color: var(--danger);
  }

  @keyframes rise {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
</style>
