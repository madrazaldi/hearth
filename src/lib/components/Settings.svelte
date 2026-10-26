<script lang="ts">
  import { app } from '../state.svelte';
  import { api } from '../api';
  import { defaultServer, type ServerConfig, type Settings } from '../types';
  import Icon from './Icon.svelte';
  import Switch from './Switch.svelte';
  import Segmented from './Segmented.svelte';
  import ChoiceCards from './ChoiceCards.svelte';
  import ServerForm from './ServerForm.svelte';
  import BouncerNetworks from './BouncerNetworks.svelte';
  import NetworkHighlights from './NetworkHighlights.svelte';
  import AliasEditor from './AliasEditor.svelte';
  import ShortcutList from './ShortcutList.svelte';
  import { updater } from '../updater.svelte';
  import * as Dialog from '$lib/components/ui/dialog';
  import * as Card from '$lib/components/ui/card';
  import * as Field from '$lib/components/ui/field';
  import { Button } from '$lib/components/ui/button';
  import { Badge } from '$lib/components/ui/badge';
  import { Input } from '$lib/components/ui/input';
  import { Separator } from '$lib/components/ui/separator';

  const tabs = [
    { id: 'servers', label: 'Servers', icon: 'server' },
    { id: 'general', label: 'General', icon: 'gear' },
    { id: 'appearance', label: 'Appearance', icon: 'moon' },
    { id: 'notifications', label: 'Notifications', icon: 'bell' },
    { id: 'aliases', label: 'Aliases', icon: 'terminal' },
    { id: 'privacy', label: 'Privacy & data', icon: 'lock' },
    { id: 'shortcuts', label: 'Shortcuts', icon: 'keyboard' },
  ];

  const uid = $props.id();
  const tab = $derived(app.settingsOpen?.tab ?? 'appearance');
  const editing = $derived(app.settingsOpen?.serverId);
  const editServer = $derived<ServerConfig | null>(
    editing === 'new' ? defaultServer() : (app.servers.find((s) => s.id === editing) ?? null),
  );
  let words = $state(app.settings.highlightWords.join(', '));
  let newIgnore = $state('');
  let confirmClear = $state(false);
  let nav = $state<HTMLElement | null>(null);
  const bouncer = $derived(editing && editing !== 'new' ? app.conns.get(editing) : undefined);

  const title = $derived.by(() => {
    if (tab === 'servers' && editServer) return editing === 'new' ? 'Add a server' : `Edit ${editServer.name}`;
    return tabs.find((t) => t.id === tab)?.label ?? 'Settings';
  });

  const descriptions: Record<string, string> = {
    servers: 'Networks and bouncers Hearth connects to. A soju bouncer expands into all of its networks automatically.',
    general: 'Startup behavior, presence and updates.',
    appearance: 'Theme, message layout and how the conversation is displayed.',
    notifications: 'Choose what gets your attention.',
    privacy: 'What Hearth stores on this computer and what it fetches on your behalf.',
    shortcuts: 'Keyboard shortcuts available throughout Hearth.',
  };

  // Status colours are listed in full so Tailwind sees the class names.
  const dots: Record<string, string> = {
    connected: 'bg-ok',
    connecting: 'bg-warn',
    registering: 'bg-warn',
    disconnected: 'bg-destructive',
  };

  const rows = 'flex flex-col divide-y divide-border rounded-xl border bg-card *:px-4 *:py-3.5';

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

{#snippet heading(text: string, hint?: string)}
  <div class="mt-7 mb-3 grid gap-0.5 first:mt-0">
    <h3 class="m-0 text-[13.5px] font-semibold">{text}</h3>
    {#if hint}<p class="m-0 text-xs text-muted-foreground">{hint}</p>{/if}
  </div>
{/snippet}

{#snippet mini(t: string)}
  <span class="mini flex size-full gap-1 p-1" data-t={t}>
    <span class="mini-side flex w-1/4 flex-col gap-1 pt-1"><i></i><i class="on"></i><i></i><i></i></span>
    <span class="mini-panel flex flex-1 flex-col justify-end gap-1 rounded-[3px] p-1.5"><i class="w-3/4"></i><i class="w-1/2"></i><i class="box"></i></span>
  </span>
{/snippet}

{#snippet themePreview(v: string)}
  <span class="relative block h-[72px] overflow-hidden rounded-md">
    {#if v === 'system'}
      <span class="absolute inset-0 flex">{@render mini('light')}</span>
      <span class="absolute inset-0 flex [clip-path:polygon(100%_0,100%_100%,0_100%)]">{@render mini('dark')}</span>
    {:else}
      <span class="absolute inset-0 flex">{@render mini(v)}</span>
    {/if}
  </span>
{/snippet}

{#snippet layoutPreview(v: string)}
  <span class="lp flex h-[72px] flex-col justify-center gap-1.5 rounded-md bg-muted/50 px-2.5">
    {#if v === 'cozy'}
      {#each [0, 1] as k (k)}
        <span class="flex gap-1.5"><b class="size-4"></b><span class="grid flex-1 content-start gap-1"><i class="nick w-1/3"></i><i class={k ? 'w-1/2' : 'w-5/6'}></i></span></span>
      {/each}
    {:else if v === 'columns'}
      {#each [0, 1, 2, 3] as k (k)}
        <span class="flex items-center gap-1.5"><i class="w-2.5 opacity-60"></i><i class="nick w-4"></i><i class={k % 2 ? 'w-1/2' : 'flex-1'}></i></span>
      {/each}
    {:else}
      {#each [0, 1] as k (k)}
        <span class="grid gap-1"><span class="flex items-center gap-1"><b class="size-2"></b><i class="nick w-1/4"></i></span><i class={k ? 'w-1/2' : 'w-5/6'}></i></span>
      {/each}
    {/if}
  </span>
{/snippet}

{#snippet code(text: string)}<code class="font-mono text-[11px] whitespace-nowrap text-foreground/80">{text}</code>{/snippet}

<Dialog.Root open onOpenChange={(o) => !o && close()}>
  <Dialog.Content
    class="h-[min(640px,88vh)] w-full grid-cols-[208px_minmax(0,1fr)] grid-rows-1 gap-0 overflow-hidden p-0 sm:max-w-4xl"
    onOpenAutoFocus={(e) => {
      // Land on the current section so arrow/tab navigation starts there.
      const el = nav?.querySelector<HTMLElement>('[data-autofocus]');
      if (!el) return;
      e.preventDefault();
      el.focus({ preventScroll: true });
    }}
  >
    <nav bind:this={nav} aria-label="Settings sections" class="flex min-h-0 flex-col gap-0.5 border-r border-sidebar-border bg-sidebar p-3 text-sidebar-foreground">
      <Dialog.Title class="flex items-center gap-2 px-2 pt-1.5 pb-3 text-sm leading-none font-semibold">Settings</Dialog.Title>
      {#each tabs as t (t.id)}
        <Button
          variant="ghost"
          class="h-8 justify-start gap-2.5 px-2 text-[13.5px] font-normal text-sidebar-foreground/65 hover:bg-sidebar-accent hover:text-sidebar-foreground {tab === t.id ? 'bg-sidebar-accent font-medium text-sidebar-accent-foreground shadow-xs [&_svg]:text-brand' : ''}"
          aria-current={tab === t.id ? 'page' : undefined}
          data-autofocus={tab === t.id || undefined}
          onclick={() => go(t.id)}
        >
          <Icon name={t.icon} size={15} />{t.label}
        </Button>
      {/each}
      <p class="mt-auto px-2 text-xs text-muted-foreground">Hearth <span class="mono text-muted-foreground/70">{updater.current || '0.1.0'}</span></p>
    </nav>

    <section class="flex min-h-0 min-w-0 flex-col">
      <header class="grid gap-1 px-8 pt-7 pr-14 pb-2">
        <h2 class="m-0 truncate text-xl font-semibold tracking-tight">{title}</h2>
        {#if tab === 'aliases'}
          <p class="m-0 text-[13px] text-muted-foreground">
            Your own slash commands. {@render code('/j')} → {@render code('/join $1')}, {@render code('/ns')} → {@render code('/msg NickServ $*')}. They show up in command suggestions and Tab completion.
          </p>
        {:else if tab === 'servers' && editServer}
          <p class="m-0 text-[13px] text-muted-foreground">Connection, identity and authentication for this network.</p>
        {:else}
          <p class="m-0 text-[13px] text-muted-foreground">{descriptions[tab]}</p>
        {/if}
      </header>

      <div class="min-h-0 flex-1 overflow-y-auto px-8 pt-4 pb-8">
        <div class="max-w-xl">
          {#if tab === 'servers'}
            {#if editServer}
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
                <Separator class="my-6" />
                <BouncerNetworks conn={bouncer} />
              {/if}
              {#if editing !== 'new'}
                <Separator class="my-6" />
                {#key editing}
                  <NetworkHighlights serverId={editServer.id} nick={editServer.nick} />
                {/key}
              {/if}
            {:else}
              <div class="grid gap-2.5">
                {#each app.servers as s (s.id)}
                  {@const st = statusOf(s.id)}
                  <Card.Root size="sm">
                    <Card.Content class="flex items-center gap-3">
                      <div class="min-w-0 flex-1">
                        <div class="truncate text-sm font-medium">{s.name}</div>
                        <div class="mono truncate text-xs text-muted-foreground">{s.websocket ? (s.tls ? 'wss://' : 'ws://') : ''}{s.host}:{s.port}{s.websocket ? s.websocketPath : ''}</div>
                      </div>
                      <Badge variant="outline" class="gap-1.5 text-muted-foreground capitalize">
                        <span class="size-1.5 rounded-full {dots[st] ?? 'bg-muted-foreground/60'}"></span>{st}
                      </Badge>
                      {#if st === 'offline' || st === 'disconnected'}
                        <Button variant="ghost" size="sm" onclick={() => api.connect(s.id)}>Connect</Button>
                      {:else}
                        <Button variant="ghost" size="sm" onclick={() => api.disconnect(s.id)}>Disconnect</Button>
                      {/if}
                      <Button variant="outline" size="sm" onclick={() => go('servers', s.id)}>Edit</Button>
                    </Card.Content>
                  </Card.Root>
                {/each}
              </div>
              <Button class="mt-4" onclick={() => go('servers', 'new')}><Icon name="plus" size={14} />Add server</Button>
            {/if}
          {:else if tab === 'general'}
            <div class={rows}>
              <Switch
                bind:checked={() => app.settings.closeToTray, (v) => app.updateSettings({ closeToTray: v })}
                label="Keep running when the window is closed"
                hint="Hearth stays connected in the menu bar or system tray. Quit from the tray icon."
              />
              <Field.Field orientation="horizontal" class="has-[>[data-slot=field-content]]:items-center">
                <Field.Content>
                  <Field.Title class="text-[13.5px] font-normal">Mark me away</Field.Title>
                  <Field.Description class="text-xs">When Hearth has been in the background this long. You're marked back when you return.</Field.Description>
                </Field.Content>
                <Segmented label="Mark me away after"
                  bind:value={autoAway.get, autoAway.set}
                  options={[
                    { value: 0, label: 'Never' },
                    { value: 10, label: '10 min' },
                    { value: 30, label: '30 min' },
                    { value: 60, label: '1 hour' },
                  ]}
                />
              </Field.Field>
              <Switch
                bind:checked={() => app.settings.spellcheck, (v) => app.updateSettings({ spellcheck: v })}
                label="Check spelling as I type"
                hint="Underlines misspelled words in the message box. Autocorrect and auto-capitalization stay off."
              />
              <Switch
                bind:checked={() => app.settings.checkUpdates, (v) => app.updateSettings({ checkUpdates: v })}
                label="Check for updates automatically"
                hint="Looks for a new release when Hearth starts and every few hours. Nothing is installed without asking."
              />
              <Field.Field orientation="horizontal" class="has-[>[data-slot=field-content]]:items-center">
                <Field.Content>
                  <Field.Title class="text-[13.5px] font-normal">Hearth {updater.current}</Field.Title>
                  <Field.Description class="text-xs {updater.phase === 'error' ? 'text-destructive' : ''}">
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
                  </Field.Description>
                </Field.Content>
                {#if updater.phase === 'available' || updater.phase === 'installing'}
                  <Button class="shrink-0" disabled={updater.busy} onclick={() => updater.installAndRestart()}>Install and restart</Button>
                {:else}
                  <Button variant="outline" class="shrink-0" disabled={updater.busy} onclick={() => updater.check()}>Check now</Button>
                {/if}
              </Field.Field>
            </div>
          {:else if tab === 'privacy'}
            <div class={rows}>
              <Switch
                bind:checked={() => app.settings.storeLogs, (v) => app.updateSettings({ storeLogs: v })}
                label="Save messages on this computer"
                hint="Keeps scrollback across restarts and makes messages searchable. Stored only on this device."
              />
              <Field.Field orientation="horizontal" class="has-[>[data-slot=field-content]]:items-center">
                <Field.Content>
                  <Field.Title class="text-[13.5px] font-normal">Saved messages</Field.Title>
                  <Field.Description class="text-xs">Deleting them doesn't affect history kept by a server or bouncer.</Field.Description>
                </Field.Content>
                {#if confirmClear}
                  <div class="flex shrink-0 gap-2">
                    <Button variant="destructive" class="bg-destructive text-white hover:bg-destructive/90" onclick={clearLogs}>Delete all?</Button>
                    <Button variant="ghost" onclick={() => (confirmClear = false)}>Cancel</Button>
                  </div>
                {:else}
                  <Button variant="destructive" class="shrink-0" onclick={() => (confirmClear = true)}>Delete saved messages</Button>
                {/if}
              </Field.Field>
              <Switch
                bind:checked={() => app.settings.linkPreviews, (v) => app.updateSettings({ linkPreviews: v })}
                label="Show link previews"
                hint="Hearth fetches linked pages and images itself, so those sites see your IP address. Links to local or private addresses are never fetched."
              />
              <Switch
                bind:checked={() => app.settings.loadAvatars, (v) => app.updateSettings({ loadAvatars: v })}
                label="Show profile pictures"
                hint="People can publish an avatar on servers that support it. Hearth fetches the image itself, so its host sees your IP address."
              />
              {#if app.settings.linkPreviews}
                <Switch
                  bind:checked={() => app.settings.inlineImages, (v) => app.updateSettings({ inlineImages: v })}
                  label="Show inline images"
                  hint="Image links appear under the message. Images in messages that mention NSFW stay blurred until clicked. Video and audio are only described."
                />
                <Switch
                  bind:checked={() => app.settings.autoplayAnimations, (v) => app.updateSettings({ autoplayAnimations: v })}
                  label="Play animations automatically"
                  hint="Otherwise animated images play while the pointer is over them. Never when your system asks for reduced motion."
                />
              {/if}
              <Field.Field>
                <Field.Label for="{uid}-ignore">Ignored people</Field.Label>
                {#if app.settings.ignores.length}
                  <ul class="m-0 list-none divide-y divide-border overflow-hidden rounded-lg border border-border p-0">
                    {#each app.settings.ignores as mask (mask)}
                      <li class="flex items-center justify-between py-1 pr-1.5 pl-3 text-[12.5px]">
                        <span class="mono">{mask}</span>
                        <Button variant="ghost" size="icon-xs" title="Stop ignoring" aria-label="Stop ignoring {mask}" onclick={() => app.unignore(mask)}><Icon name="x" size={13} /></Button>
                      </li>
                    {/each}
                  </ul>
                {/if}
                <form class="flex gap-2" onsubmit={addIgnore}>
                  <Input id="{uid}-ignore" class="flex-1 font-mono" bind:value={newIgnore} placeholder="nick or nick!user@host" aria-label="Nick or mask to ignore" spellcheck="false" />
                  <Button variant="outline" type="submit" disabled={!newIgnore.trim()}>Ignore</Button>
                </form>
                <Field.Description class="text-xs">Hides their messages, notices, invites and reactions everywhere. {@render code('*')} and {@render code('?')} are wildcards.</Field.Description>
              </Field.Field>
            </div>
          {:else if tab === 'appearance'}
            {@render heading('Theme')}
            <ChoiceCards
              label="Theme"
              bind:value={theme.get, theme.set}
              preview={themePreview}
              options={[
                { value: 'system', label: 'System' },
                { value: 'light', label: 'Light' },
                { value: 'dark', label: 'Dark' },
              ]}
            />
            {@render heading('Message layout', 'Modern groups messages under avatars; columns lines nicks up in a gutter; compact stacks them above messages.')}
            <ChoiceCards
              label="Message layout"
              bind:value={layout.get, layout.set}
              preview={layoutPreview}
              options={[
                { value: 'cozy', label: 'Modern' },
                { value: 'columns', label: 'Columns' },
                { value: 'compact', label: 'Compact' },
              ]}
            />
            {@render heading('Display')}
            <div class={rows}>
              <Field.Field orientation="horizontal" class="has-[>[data-slot=field-content]]:items-center">
                <Field.Content><Field.Title class="text-[13.5px] font-normal">Text size</Field.Title></Field.Content>
                <Segmented label="Text size"
                  bind:value={fontSize.get, fontSize.set}
                  options={[
                    { value: 13, label: 'S' },
                    { value: 14, label: 'M' },
                    { value: 15, label: 'L' },
                    { value: 16, label: 'XL' },
                  ]}
                />
              </Field.Field>
              <Field.Field orientation="horizontal" class="has-[>[data-slot=field-content]]:items-center">
                <Field.Content><Field.Title class="text-[13.5px] font-normal">Clock</Field.Title></Field.Content>
                <Segmented label="Clock"
                  bind:value={clock.get, clock.set}
                  options={[
                    { value: '24h', label: '24-hour' },
                    { value: '12h', label: '12-hour' },
                  ]}
                />
              </Field.Field>
              <Field.Field orientation="horizontal" class="has-[>[data-slot=field-content]]:items-center">
                <Field.Content>
                  <Field.Title class="text-[13.5px] font-normal">Joins, parts and quits</Field.Title>
                  <Field.Description class="text-xs">Collapse folds consecutive events into one line.</Field.Description>
                </Field.Content>
                <Segmented label="Joins, parts and quits"
                  bind:value={membership.get, membership.set}
                  options={[
                    { value: 'show', label: 'Show' },
                    { value: 'collapse', label: 'Collapse' },
                    { value: 'hide', label: 'Hide' },
                  ]}
                />
              </Field.Field>
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
            <div class={rows}>
              <Switch
                bind:checked={() => app.settings.notifyHighlights, (v) => app.updateSettings({ notifyHighlights: v })}
                label="Mentions"
                hint="When someone says your nickname or a highlight word."
              />
              <Switch
                bind:checked={() => app.settings.notifyPrivate, (v) => app.updateSettings({ notifyPrivate: v })}
                label="Private messages"
              />
              <Field.Field>
                <Field.Label for="{uid}-words">Highlight words</Field.Label>
                <Input
                  id="{uid}-words"
                  bind:value={words}
                  onchange={() => app.updateSettings({ highlightWords: words.split(',').map((w) => w.trim()).filter(Boolean) })}
                  placeholder="hearth, deploy"
                />
                <Field.Description class="text-xs">Comma-separated. Your nickname always counts. Add per-server words and exceptions on each server's settings page.</Field.Description>
              </Field.Field>
            </div>
          {:else if tab === 'aliases'}
            <AliasEditor />
          {:else if tab === 'shortcuts'}
            <ShortcutList />
          {/if}
        </div>
      </div>
    </section>
  </Dialog.Content>
</Dialog.Root>

<style>
  .mini[data-t='dark'] {
    --w: #151311;
    --p: #211e1c;
    --l: #3b3633;
    --a: #e48a3e;
  }

  .mini[data-t='light'] {
    --w: #f2efeb;
    --p: #ffffff;
    --l: #e0dad3;
    --a: #d06a20;
  }

  .mini {
    background: var(--w);
  }

  .mini i {
    display: block;
    height: 4px;
    border-radius: 2px;
    background: var(--l);
  }

  .mini-side i.on {
    background: var(--a);
  }

  .mini-panel {
    background: var(--p);
    box-shadow: 0 0 0 1px color-mix(in srgb, var(--l) 70%, transparent);
  }

  .mini-panel i.box {
    height: 9px;
    border-radius: 3px;
    background: none;
    box-shadow: inset 0 0 0 1px var(--l);
  }

  .lp i,
  .lp b {
    display: block;
    flex: none;
    height: 4px;
    border-radius: 2px;
    background: var(--input);
  }

  .lp i.flex-1 {
    flex: 1;
  }

  .lp b {
    height: auto;
    aspect-ratio: 1;
    border-radius: 999px;
  }

  .lp i.nick {
    background: color-mix(in oklab, var(--brand) 70%, transparent);
  }
</style>
