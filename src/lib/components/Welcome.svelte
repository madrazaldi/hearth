<script lang="ts">
  import logo from '../../assets/logo.png';
  import { app, nextSeq } from '../state.svelte';
  import { api } from '../api';
  import { defaultServer, type ServerConfig } from '../types';
  import { explainError, nickError, parseChannels, presets, type NetworkPreset } from '../onboarding';
  import ServerForm from './ServerForm.svelte';
  import Switch from './Switch.svelte';
  import Icon from './Icon.svelte';
  import { Alert, AlertDescription, AlertTitle } from '$lib/components/ui/alert';
  import { Badge } from '$lib/components/ui/badge';
  import { Button } from '$lib/components/ui/button';
  import * as Card from '$lib/components/ui/card';
  import * as Field from '$lib/components/ui/field';
  import { Input } from '$lib/components/ui/input';
  import * as Item from '$lib/components/ui/item';
  import { Spinner } from '$lib/components/ui/spinner';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';

  let { ondone }: { ondone: () => void } = $props();

  type Step = 'network' | 'server' | 'nick' | 'account' | 'channels' | 'custom' | 'connect';
  const LABELS: Record<Step, string> = {
    network: 'Network',
    server: 'Server',
    nick: 'Nickname',
    account: 'Account',
    channels: 'Channels',
    custom: 'Server',
    connect: 'Connect',
  };

  let step = $state<Step>('network');
  let preset = $state<NetworkPreset | null>(null);
  let s = $state<ServerConfig>(defaultServer());
  /** Set once saved, so retries update the same server instead of adding another. */
  let savedId = $state('');

  let nickTouched = $state(false);
  let useAccount = $state(false);
  let accountError = $state('');
  let serverError = $state('');
  let picked = $state<string[]>([]);
  let extra = $state('');

  const flow = $derived.by((): Step[] => {
    if (!preset) return ['network'];
    if (preset.kind === 'custom') return ['network', 'custom', 'connect'];
    if (preset.kind === 'bouncer') return ['network', 'server', 'nick', 'account', 'connect'];
    return ['network', 'nick', ...(preset.sasl ? (['account'] as const) : []), 'channels', 'connect'];
  });
  const index = $derived(flow.indexOf(step));
  const nickLen = $derived(preset?.nickLen ?? 30);
  const nickProblem = $derived(nickError(s.nick.trim(), nickLen, preset?.label));
  const netName = $derived(preset?.kind === 'network' ? preset.label : s.name || s.host);

  function choose(p: NetworkPreset) {
    const keepNick = s.nick;
    preset = p;
    s = { ...defaultServer(), name: p.kind === 'network' ? p.label : '', host: p.host, port: p.port, tls: p.tls, nick: keepNick };
    // A failed attempt was already saved; don't leave it behind when switching networks.
    if (savedId) app.deleteServer(savedId).catch(() => {});
    savedId = '';
    useAccount = p.kind === 'bouncer';
    picked = [...(p.channels ?? [])];
    extra = '';
    nickTouched = false;
    step = flow[1];
  }

  const next = () => (step = flow[index + 1]);
  const back = () => (step = flow[index - 1]);

  function submitServer(e: SubmitEvent) {
    e.preventDefault();
    s.host = s.host.trim();
    serverError = s.host ? '' : 'Enter the address of your bouncer.';
    if (!serverError) next();
  }

  function submitNick(e: SubmitEvent) {
    e.preventDefault();
    nickTouched = true;
    s.nick = s.nick.trim();
    if (!nickProblem) next();
  }

  function submitAccount(e: SubmitEvent) {
    e.preventDefault();
    accountError = '';
    if (useAccount && !s.saslPassword) return (accountError = 'Enter your password, or turn off “I have an account”.');
    if (preset?.kind === 'network') next();
    else connect();
  }

  function submitChannels(e: SubmitEvent) {
    e.preventDefault();
    connect();
  }

  // ─── Connecting ──────────────────────────────────────────────

  let saving = $state(false);
  /** Line ids are sequential, so anything above this belongs to the current attempt. */
  let baseline = $state(0);
  /** The backend has picked up this attempt, so a disconnect belongs to it. */
  let live = $state(false);
  let failure = $state<{ raw: string; sasl: boolean } | null>(null);

  const conn = $derived(savedId ? app.conns.get(savedId) : undefined);
  const status = $derived(saving ? 'saving' : (conn?.status ?? 'connecting'));
  const saslFailed = $derived(
    conn?.server.messages.find((l) => l.type === 'error' && l.text.startsWith('SASL:') && l.id > baseline)?.text ?? null,
  );

  $effect(() => {
    if (step !== 'connect' || saving || !conn || failure) return;
    if (conn.status === 'connecting' || conn.status === 'registering') live = true;
    else if (live && conn.status === 'disconnected' && conn.error) {
      failure = { raw: [saslFailed, conn.error].filter(Boolean).join(' — '), sasl: s.sasl === 'plain' };
      // Stop the backoff loop; the user decides whether to retry or edit.
      api.disconnect(savedId).catch(() => {});
    }
  });

  async function connect(from?: ServerConfig) {
    if (from) s = from;
    else if (preset?.kind !== 'custom') {
      s.sasl = useAccount ? 'plain' : 'none';
      if (!useAccount) s.saslPassword = '';
      if (useAccount && !s.saslUsername.trim()) s.saslUsername = s.nick;
      s.autojoin = preset?.kind === 'network' ? parseChannels([...picked, extra].join(' ')) : s.autojoin;
    }
    if (savedId) s.id = savedId;
    step = 'connect';
    failure = null;
    live = false;
    saving = true;
    baseline = nextSeq();
    try {
      const saved = await app.saveServer($state.snapshot(s) as ServerConfig, true);
      savedId = saved.id;
      s.id = saved.id;
    } catch (e) {
      failure = { raw: String(e), sasl: false };
    } finally {
      saving = false;
    }
  }

  // Send the user back to the step that can fix the problem.
  function editAfterFailure() {
    const auth = /sasl|authenticat/i.test(failure?.raw ?? '');
    failure = null;
    if (auth && flow.includes('account')) step = 'account';
    else if (flow.includes('server')) step = 'server';
    else step = flow[flow.length - 2];
  }

  const friendly = $derived(failure ? explainError(failure.raw, { host: s.host, port: s.port, tls: s.tls, sasl: failure.sasl }) : null);
  const nickNote = $derived(
    conn?.status === 'connected' && conn.nick && conn.fold(conn.nick) !== conn.fold(s.nick)
      ? `“${s.nick}” was taken, so you're “${conn.nick}” for now.`
      : null,
  );

  type Stage = { key: string; label: string };
  const stages = $derived<Stage[]>([
    { key: 'saving', label: s.sasl === 'plain' ? 'Saving settings, password to your keychain' : 'Saving settings' },
    { key: 'connecting', label: `Reaching ${s.host}${s.tls ? ' securely' : ''}` },
    { key: 'registering', label: s.sasl === 'plain' ? 'Logging in to your account' : 'Signing on' },
    { key: 'connected', label: preset?.kind === 'bouncer' ? 'Connected to your bouncer' : `Connected to ${netName}` },
  ]);
  const order = ['saving', 'connecting', 'registering', 'connected'];
  const at = $derived(order.indexOf(status === 'disconnected' ? 'connecting' : status));

  // Move focus to the first field of each step so the flow works from the keyboard.
  function autofocus(node: HTMLElement) {
    requestAnimationFrame(() =>
      (node.querySelector<HTMLElement>('[data-autofocus]') ?? node.querySelector<HTMLElement>('input, [role=switch], button[type=submit]'))?.focus(),
    );
  }

  const focusNow = (n: HTMLElement) => n.focus();
  const codeClass = 'font-mono text-[11.5px]';
</script>

<div
  class="flex h-full justify-center overflow-y-auto bg-background bg-[radial-gradient(1200px_400px_at_50%_-10%,color-mix(in_oklab,var(--brand)_7%,transparent),transparent_70%)] px-6 pt-14 pb-12"
  data-tauri-drag-region
>
  <main class="w-full max-w-xl">
    <header class="mb-6 flex flex-col items-center text-center">
      <img class="mb-4 rounded-xl" src={logo} width="48" height="48" alt="" />
      <h1 class="m-0 text-3xl font-semibold tracking-tight">Hearth</h1>
      <p class="mt-1.5 text-[15px] text-muted-foreground">A quiet place for IRC.</p>
    </header>

    {#if flow.length > 1}
      <ol class="m-0 mb-4 flex list-none flex-wrap justify-center gap-x-4 gap-y-1.5 p-0 text-[12.5px] text-muted-foreground" aria-label="Setup progress">
        {#each flow as f, i (f)}
          <li
            class={['flex items-center gap-1.5', i === index && 'text-foreground', i < index && 'text-foreground/70']}
            aria-current={i === index ? 'step' : undefined}
          >
            <span
              class={[
                'grid size-[18px] place-items-center rounded-full text-[10.5px]',
                i === index ? 'bg-primary text-primary-foreground' : i < index ? 'bg-brand-soft text-brand' : 'ring-1 ring-border ring-inset',
              ]}
              aria-hidden="true"
            >
              {#if i < index}<Icon name="check" size={11} />{:else}{i + 1}{/if}
            </span>
            {LABELS[f]}
          </li>
        {/each}
      </ol>
    {/if}

    {#key step}
      <Card.Root class="shadow-sm [--card-spacing:--spacing(6)]" role="group" {@attach autofocus} aria-labelledby="step-title">
        {#if step === 'network'}
          <Card.Header>
            <Card.Title id="step-title" role="heading" aria-level={2}>Choose a network</Card.Title>
            <Card.Description>Pick a public network to begin. You can add more, or a bouncer, any time.</Card.Description>
          </Card.Header>
          <Card.Content class="grid gap-5">
            <ul class="m-0 flex list-none flex-col gap-2 p-0" aria-label="Public networks">
              {#each presets.filter((p) => p.kind === 'network') as p (p.id)}
                {@const selected = preset?.id === p.id}
                <li>
                  <Item.Root variant="outline" size="sm" class={['hover:bg-muted', selected && 'border-primary bg-muted/60']}>
                    {#snippet child({ props })}
                      <button {...props} type="button" onclick={() => choose(p)} data-autofocus={p.id === 'libera' || undefined} aria-current={selected || undefined}>
                        <Item.Content class="gap-0.5">
                          <Item.Title>{p.label}</Item.Title>
                          <Item.Description>{p.blurb}</Item.Description>
                        </Item.Content>
                        <span class="font-mono text-[11px] text-muted-foreground max-sm:hidden">{p.host}</span>
                        <span class="text-muted-foreground"><Icon name={selected ? 'check' : 'chevron'} size={14} /></span>
                      </button>
                    {/snippet}
                  </Item.Root>
                </li>
              {/each}
            </ul>
            <Field.Separator class="[&_[data-slot=field-separator-content]]:bg-card">Or connect to your own</Field.Separator>
            <ul class="m-0 grid list-none grid-cols-2 gap-2 p-0 max-sm:grid-cols-1">
              {#each presets.filter((p) => p.kind !== 'network') as p (p.id)}
                {@const selected = preset?.id === p.id}
                <li class="flex">
                  <Item.Root variant="outline" size="sm" class={['items-start hover:bg-muted', selected && 'border-primary bg-muted/60']}>
                    {#snippet child({ props })}
                      <button {...props} type="button" onclick={() => choose(p)} aria-current={selected || undefined}>
                        <Item.Media variant="icon" class="text-muted-foreground">
                          <Icon name={p.kind === 'bouncer' ? 'layers' : 'settings'} size={16} />
                        </Item.Media>
                        <Item.Content class="gap-0.5">
                          <Item.Title>{p.label}</Item.Title>
                          <Item.Description>{p.blurb}</Item.Description>
                        </Item.Content>
                      </button>
                    {/snippet}
                  </Item.Root>
                </li>
              {/each}
            </ul>
          </Card.Content>
        {:else if step === 'server'}
          <Card.Header>
            <Card.Title id="step-title" role="heading" aria-level={2}>Where is your bouncer?</Card.Title>
            <Card.Description>The address Hearth should connect to.</Card.Description>
          </Card.Header>
          <form class="contents" onsubmit={submitServer} novalidate>
            <Card.Content>
              <Field.Group class="gap-4">
                <div class="grid grid-cols-[1fr_6rem] gap-3">
                  <Field.Field>
                    <Field.Label for="w-host">Host</Field.Label>
                    <Input
                      id="w-host"
                      class="font-mono"
                      bind:value={s.host}
                      placeholder="bouncer.example.org"
                      spellcheck="false"
                      autocapitalize="off"
                      aria-invalid={!!serverError}
                      aria-describedby={serverError ? 'server-error' : undefined}
                    />
                  </Field.Field>
                  <Field.Field>
                    <Field.Label for="w-port">Port</Field.Label>
                    <Input id="w-port" class="font-mono" type="number" min="1" max="65535" bind:value={s.port} />
                  </Field.Field>
                </div>
                <Switch bind:checked={s.tls} label="Use TLS" hint="Almost every bouncer supports it; the usual port is 6697." />
                {#if serverError}<Field.Error id="server-error">{serverError}</Field.Error>{/if}
              </Field.Group>
            </Card.Content>
            {@render actions('Continue')}
          </form>
        {:else if step === 'nick'}
          <Card.Header>
            <Card.Title id="step-title" role="heading" aria-level={2}>Choose a nickname</Card.Title>
            <Card.Description>How others will see you on {preset?.kind === 'network' ? preset.label : 'the server'}.</Card.Description>
          </Card.Header>
          <form class="contents" onsubmit={submitNick} novalidate>
            <Card.Content>
              <Field.Group class="gap-4">
                <Field.Field>
                  <Field.Label for="w-nick">Nickname</Field.Label>
                  <Input
                    id="w-nick"
                    bind:value={s.nick}
                    onblur={() => (nickTouched = !!s.nick)}
                    maxlength={64}
                    spellcheck="false"
                    autocapitalize="off"
                    autocomplete="nickname"
                    placeholder="ember"
                    aria-invalid={nickTouched && !!nickProblem}
                    aria-describedby="nick-help"
                  />
                  {#if nickTouched && nickProblem}
                    <Field.Error id="nick-help">{nickProblem}</Field.Error>
                  {:else}
                    <Field.Description id="nick-help" class="text-xs">
                      Letters, digits and <code class={codeClass}>- _ [ ] {'{ }'} \ | ^ `</code>, up to {nickLen} characters.
                    </Field.Description>
                  {/if}
                </Field.Field>
                {#if preset?.kind === 'network' && preset.sasl === false && preset.accountNote}
                  <Alert>
                    <Icon name="info" size={16} />
                    <AlertDescription>{preset.accountNote}</AlertDescription>
                  </Alert>
                {/if}
              </Field.Group>
            </Card.Content>
            {@render actions('Continue')}
          </form>
        {:else if step === 'account'}
          <Card.Header>
            <Card.Title id="step-title" role="heading" aria-level={2} class="flex items-center gap-2">
              {preset?.kind === 'bouncer' ? 'Log in to your bouncer' : 'Log in to an account'}
              {#if preset?.kind !== 'bouncer'}<Badge variant="outline">optional</Badge>{/if}
            </Card.Title>
            {#if preset?.accountNote}<Card.Description>{preset.accountNote}</Card.Description>{/if}
          </Card.Header>
          <form class="contents" onsubmit={submitAccount} novalidate>
            <Card.Content>
              <Field.Group class="gap-4">
                {#if preset?.kind !== 'bouncer'}
                  <Switch bind:checked={useAccount} label="I have an account" hint="Skip this if you haven't registered your nickname." />
                {/if}
                {#if useAccount}
                  <div class="grid grid-cols-2 gap-3 max-sm:grid-cols-1">
                    <Field.Field>
                      <Field.Label for="w-user">{preset?.kind === 'bouncer' ? 'Username' : 'Account name'}</Field.Label>
                      <Input id="w-user" bind:value={s.saslUsername} placeholder={s.nick} spellcheck="false" autocapitalize="off" autocomplete="username" />
                    </Field.Field>
                    <Field.Field>
                      <Field.Label for="w-pass">Password</Field.Label>
                      <Input
                        id="w-pass"
                        type="password"
                        bind:value={s.saslPassword}
                        autocomplete="current-password"
                        aria-invalid={!!accountError}
                        aria-describedby="account-note"
                        data-autofocus={preset?.kind === 'bouncer' ? undefined : true}
                      />
                    </Field.Field>
                  </div>
                  <Field.Description id="account-note" class="flex items-start gap-2 text-xs">
                    <span class="mt-px shrink-0"><Icon name="lock" size={14} /></span>
                    Kept in your system keychain, never in Hearth's config file. Sent with SCRAM-SHA-256 when the server supports it.
                  </Field.Description>
                {:else if preset?.kind === 'network'}
                  <Field.Description class="text-xs">
                    No account yet? Connect first, then register with <code class={codeClass}>/msg NickServ REGISTER password email</code>.
                  </Field.Description>
                {/if}
                {#if accountError}<Field.Error>{accountError}</Field.Error>{/if}
              </Field.Group>
            </Card.Content>
            {@render actions(preset?.kind === 'bouncer' ? 'Connect' : 'Continue')}
          </form>
        {:else if step === 'channels'}
          <Card.Header>
            <Card.Title id="step-title" role="heading" aria-level={2} class="flex items-center gap-2">
              Join some channels <Badge variant="outline">optional</Badge>
            </Card.Title>
            <Card.Description>Pick a few to open as soon as you connect.</Card.Description>
          </Card.Header>
          <form class="contents" onsubmit={submitChannels} novalidate>
            <Card.Content>
              <Field.Group class="gap-4">
                {#if preset?.channels?.length}
                  <Field.Set class="gap-1">
                    <Field.Legend variant="label" class="mb-0 text-muted-foreground">Suggested on {netName}</Field.Legend>
                    {#each preset.channels as c (c)}
                      <Switch
                        bind:checked={() => picked.includes(c), (v) => (picked = v ? [...picked, c] : picked.filter((x) => x !== c))}
                        label={c}
                        hint="The network's official help and community channel."
                      />
                    {/each}
                  </Field.Set>
                {/if}
                <Field.Field>
                  <Field.Label for="w-extra">Other channels</Field.Label>
                  <Input id="w-extra" class="font-mono" bind:value={extra} placeholder="#rust, #linux" spellcheck="false" autocapitalize="off" aria-describedby="chan-help" />
                  <Field.Description id="chan-help" class="text-xs">
                    Separate with spaces or commas. You can join more later with <code class={codeClass}>/join</code> or the channel browser.
                  </Field.Description>
                </Field.Field>
              </Field.Group>
            </Card.Content>
            {@render actions('Connect')}
          </form>
        {:else if step === 'custom'}
          <Card.Header>
            <Card.Title id="step-title" role="heading" aria-level={2}>Custom server</Card.Title>
            <Card.Description>Any other server, with every option.</Card.Description>
          </Card.Header>
          <Card.Content>
            {#key savedId}
              <ServerForm server={{ ...$state.snapshot(s), id: savedId }} submitLabel="Connect" onsave={async (cfg) => void connect(cfg)} oncancel={back} />
            {/key}
          </Card.Content>
        {:else if step === 'connect'}
          <Card.Header>
            <Card.Title id="step-title" role="heading" aria-level={2}>
              {failure ? "Couldn't connect" : status === 'connected' ? "You're in" : `Connecting to ${netName}`}
            </Card.Title>
          </Card.Header>
          {#if failure && friendly}
            <Card.Content>
              <Alert variant="destructive">
                <CircleAlert />
                <AlertTitle>{friendly.title}</AlertTitle>
                <AlertDescription>
                  <p>{friendly.hint}</p>
                  <details class="mt-2 text-xs text-muted-foreground">
                    <summary class="cursor-pointer">Details</summary>
                    <code class="selectable mt-1.5 block font-mono text-[11.5px] break-words">{failure.raw}</code>
                  </details>
                </AlertDescription>
              </Alert>
            </Card.Content>
            <Card.Footer class="justify-between">
              <Button variant="ghost" onclick={editAfterFailure}>Back</Button>
              <Button onclick={() => connect()} {@attach focusNow}>Try again</Button>
            </Card.Footer>
          {:else}
            <Card.Content class="grid gap-4">
              <ol class="m-0 grid list-none gap-3 p-0 text-[13.5px] text-muted-foreground" aria-live="polite">
                {#each stages as st, i (st.key)}
                  {@const done = i < at || status === 'connected'}
                  {@const now = i === at && status !== 'connected'}
                  <li class={['flex items-center gap-2.5', now && 'text-foreground', done && 'text-foreground/70']}>
                    <span
                      class={[
                        'grid size-5 shrink-0 place-items-center rounded-full',
                        done ? 'bg-brand-soft text-brand' : 'ring-1 ring-border ring-inset',
                      ]}
                      aria-hidden="true"
                    >
                      {#if done}<Icon name="check" size={12} />{:else if now}<Spinner class="size-3.5 text-brand" aria-label="" role="presentation" />{/if}
                    </span>
                    {st.label}
                    {#if now}<span class="sr-only">(in progress)</span>{/if}
                  </li>
                {/each}
              </ol>
              {#if status === 'connected'}
                {#if nickNote}
                  <Alert class="text-warn">
                    <TriangleAlert />
                    <AlertDescription class="text-warn/90">{nickNote}</AlertDescription>
                  </Alert>
                {/if}
                {#if saslFailed}
                  <Alert class="text-warn">
                    <TriangleAlert />
                    <AlertDescription class="text-warn/90">
                      Your account login failed, so you're connected without it. Check the password in Settings.
                    </AlertDescription>
                  </Alert>
                {/if}
                {#if s.autojoin.length && preset?.kind === 'network'}
                  <p class="m-0 text-sm text-muted-foreground">Joining {s.autojoin.join(', ')}.</p>
                {/if}
              {/if}
            </Card.Content>
            {#if status === 'connected'}
              <Card.Footer class="justify-end">
                <Button onclick={ondone} {@attach focusNow}>Start chatting</Button>
              </Card.Footer>
            {/if}
          {/if}
        {/if}
      </Card.Root>
    {/key}
  </main>
</div>

{#snippet actions(label: string)}
  <Card.Footer class="justify-between">
    <Button variant="ghost" onclick={back}>Back</Button>
    <Button type="submit">{label}</Button>
  </Card.Footer>
{/snippet}
