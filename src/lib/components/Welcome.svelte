<script lang="ts">
  import logo from '../../assets/logo.png';
  import { app, nextSeq } from '../state.svelte';
  import { api } from '../api';
  import { defaultServer, type ServerConfig } from '../types';
  import { explainError, nickError, parseChannels, presets, type NetworkPreset } from '../onboarding';
  import ServerForm from './ServerForm.svelte';
  import Switch from './Switch.svelte';
  import Icon from './Icon.svelte';

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
      (node.querySelector<HTMLElement>('[data-autofocus]') ?? node.querySelector<HTMLElement>('input, button.primary'))?.focus(),
    );
  }
</script>

<div class="welcome" data-tauri-drag-region>
  <main class="col">
    <img class="mark" src={logo} width="48" height="48" alt="" />
    <h1>Hearth</h1>
    <p class="lede">A quiet place for IRC. Pick a network to begin — you can add more, or a bouncer, any time.</p>

    {#if flow.length > 1}
      <ol class="steps" aria-label="Setup progress">
        {#each flow as f, i (f)}
          <li class:done={i < index} class:current={i === index} aria-current={i === index ? 'step' : undefined}>
            <span class="dot" aria-hidden="true">{#if i < index}<Icon name="check" size={11} />{:else}{i + 1}{/if}</span>
            {LABELS[f]}
          </li>
        {/each}
      </ol>
    {/if}

    {#key step}
      <section class="card" {@attach autofocus} aria-labelledby="step-title">
        {#if step === 'network'}
          <h2 id="step-title">Choose a network</h2>
          <ul class="nets" aria-label="Public networks">
            {#each presets.filter((p) => p.kind === 'network') as p (p.id)}
              <li>
                <button type="button" class="net" onclick={() => choose(p)} data-autofocus={p.id === 'libera' || undefined}>
                  <strong>{p.label}</strong>
                  <span class="blurb">{p.blurb}</span>
                  <span class="host mono">{p.host}</span>
                  <Icon name="chevron" size={14} />
                </button>
              </li>
            {/each}
          </ul>
          <p class="or">Or connect to your own</p>
          <ul class="own">
            {#each presets.filter((p) => p.kind !== 'network') as p (p.id)}
              <li>
                <button type="button" class="own-btn" onclick={() => choose(p)}>
                  <Icon name={p.kind === 'bouncer' ? 'layers' : 'settings'} size={16} />
                  <span>
                    <strong>{p.label}</strong>
                    <span class="blurb">{p.blurb}</span>
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        {:else if step === 'server'}
          <h2 id="step-title">Where is your bouncer?</h2>
          <form onsubmit={submitServer} novalidate>
            <div class="grid">
              <label class="field">
                <span>Host</span>
                <input
                  class="input mono"
                  bind:value={s.host}
                  placeholder="bouncer.example.org"
                  spellcheck="false"
                  autocapitalize="off"
                  aria-invalid={!!serverError}
                  aria-describedby={serverError ? 'server-error' : undefined}
                />
              </label>
              <label class="field">
                <span>Port</span>
                <input class="input mono" type="number" min="1" max="65535" bind:value={s.port} />
              </label>
            </div>
            <Switch bind:checked={s.tls} label="Use TLS" hint="Almost every bouncer supports it; the usual port is 6697." />
            {#if serverError}<p class="error" id="server-error" role="alert">{serverError}</p>{/if}
            {@render actions('Continue')}
          </form>
        {:else if step === 'nick'}
          <h2 id="step-title">Choose a nickname</h2>
          <form onsubmit={submitNick} novalidate>
            <div class="field">
              <label class="field">
                <span>Nickname</span>
                <input
                  class="input"
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
              </label>
              {#if nickTouched && nickProblem}
                <small id="nick-help" class="bad" role="alert">{nickProblem}</small>
              {:else}
                <small id="nick-help">
                  How others see you. Letters, digits and <code>- _ [ ] {'{ }'} \ | ^ `</code>, up to {nickLen} characters.
                </small>
              {/if}
            </div>
            {#if preset?.kind === 'network' && preset.sasl === false && preset.accountNote}
              <p class="note"><Icon name="info" size={14} />{preset.accountNote}</p>
            {/if}
            {@render actions('Continue')}
          </form>
        {:else if step === 'account'}
          <h2 id="step-title">{preset?.kind === 'bouncer' ? 'Log in to your bouncer' : 'Log in to an account'} <span class="opt">{preset?.kind === 'bouncer' ? '' : 'optional'}</span></h2>
          <form onsubmit={submitAccount} novalidate>
            {#if preset?.accountNote}<p class="explain">{preset.accountNote}</p>{/if}
            {#if preset?.kind !== 'bouncer'}
              <Switch bind:checked={useAccount} label="I have an account" hint="Skip this if you haven't registered your nickname." />
            {/if}
            {#if useAccount}
              <div class="grid two">
                <label class="field">
                  <span>{preset?.kind === 'bouncer' ? 'Username' : 'Account name'}</span>
                  <input class="input" bind:value={s.saslUsername} placeholder={s.nick} spellcheck="false" autocapitalize="off" autocomplete="username" />
                </label>
                <label class="field">
                  <span>Password</span>
                  <input
                    class="input"
                    type="password"
                    bind:value={s.saslPassword}
                    autocomplete="current-password"
                    aria-invalid={!!accountError}
                    aria-describedby="account-note"
                    data-autofocus={preset?.kind === 'bouncer' ? undefined : true}
                  />
                </label>
              </div>
              <small class="note" id="account-note"><Icon name="lock" size={14} />Kept in your system keychain, never in Hearth's config file. Sent with SCRAM-SHA-256 when the server supports it.</small>
            {:else if preset?.kind === 'network'}
              <small class="note">No account yet? Connect first, then register with <code>/msg NickServ REGISTER password email</code>.</small>
            {/if}
            {#if accountError}<p class="error" role="alert">{accountError}</p>{/if}
            {@render actions(preset?.kind === 'bouncer' ? 'Connect' : 'Continue')}
          </form>
        {:else if step === 'channels'}
          <h2 id="step-title">Join some channels <span class="opt">optional</span></h2>
          <form onsubmit={submitChannels} novalidate>
            {#if preset?.channels?.length}
              <fieldset>
                <legend>Suggested on {netName}</legend>
                {#each preset.channels as c (c)}
                  <Switch
                    bind:checked={() => picked.includes(c), (v) => (picked = v ? [...picked, c] : picked.filter((x) => x !== c))}
                    label={c}
                    hint="The network's official help and community channel."
                  />
                {/each}
              </fieldset>
            {/if}
            <div class="field">
              <label class="field">
                <span>Other channels</span>
                <input class="input mono" bind:value={extra} placeholder="#rust, #linux" spellcheck="false" autocapitalize="off" aria-describedby="chan-help" />
              </label>
              <small id="chan-help">Separate with spaces or commas. You can join more later with <code>/join</code> or the channel browser.</small>
            </div>
            {@render actions('Connect')}
          </form>
        {:else if step === 'custom'}
          <h2 id="step-title">Custom server</h2>
          {#key savedId}
            <ServerForm server={{ ...$state.snapshot(s), id: savedId }} submitLabel="Connect" onsave={async (cfg) => void connect(cfg)} oncancel={back} />
          {/key}
        {:else if step === 'connect'}
          <h2 id="step-title">
            {failure ? "Couldn't connect" : status === 'connected' ? "You're in" : `Connecting to ${netName}`}
          </h2>
          {#if failure && friendly}
            <div class="failure" role="alert">
              <p class="f-title">{friendly.title}</p>
              <p class="f-hint">{friendly.hint}</p>
              <details>
                <summary>Details</summary>
                <code class="selectable">{failure.raw}</code>
              </details>
            </div>
            <div class="actions">
              <button type="button" class="btn ghost" onclick={editAfterFailure}>Back</button>
              <span class="spacer"></span>
              <button type="button" class="btn primary" onclick={() => connect()}>Try again</button>
            </div>
          {:else}
            <ol class="progress" aria-live="polite">
              {#each stages as st, i (st.key)}
                <li class:done={i < at || status === 'connected'} class:now={i === at && status !== 'connected'}>
                  <span class="tick" aria-hidden="true">
                    {#if i < at || status === 'connected'}<Icon name="check" size={12} />{:else if i === at}<span class="spin"></span>{/if}
                  </span>
                  {st.label}
                  {#if i === at && status !== 'connected'}<span class="visually-hidden">(in progress)</span>{/if}
                </li>
              {/each}
            </ol>
            {#if status === 'connected'}
              {#if nickNote}<p class="note warn"><Icon name="info" size={14} />{nickNote}</p>{/if}
              {#if saslFailed}
                <p class="note warn"><Icon name="info" size={14} />Your account login failed, so you're connected without it. Check the password in Settings.</p>
              {/if}
              {#if s.autojoin.length && preset?.kind === 'network'}
                <p class="explain">Joining {s.autojoin.join(', ')}.</p>
              {/if}
              <div class="actions">
                <span class="spacer"></span>
                <button type="button" class="btn primary" onclick={ondone} {@attach (n) => n.focus()}>Start chatting</button>
              </div>
            {/if}
          {/if}
        {/if}
      </section>
    {/key}
  </main>
</div>

{#snippet actions(label: string)}
  <div class="actions">
    <button type="button" class="btn ghost" onclick={back}>Back</button>
    <span class="spacer"></span>
    <button type="submit" class="btn primary">{label}</button>
  </div>
{/snippet}

<style>
  .welcome {
    height: 100%;
    overflow-y: auto;
    display: flex;
    justify-content: center;
    padding: 64px 24px 48px;
    background:
      radial-gradient(1200px 400px at 50% -10%, color-mix(in oklab, var(--accent) 6%, transparent), transparent 70%),
      var(--bg);
  }

  .col {
    width: min(560px, 100%);
  }

  .mark {
    display: block;
    margin: 0 0 18px -4px;
  }

  h1 {
    margin: 0;
    font-size: 30px;
    font-weight: 600;
    letter-spacing: -0.025em;
  }

  .lede {
    margin: 8px 0 24px;
    color: var(--text-2);
    font-size: 15px;
    max-width: 44ch;
  }

  h2 {
    margin: 0 0 16px;
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .opt {
    margin-left: 6px;
    font-size: 12px;
    font-weight: 400;
    color: var(--text-3);
  }

  .steps {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
    margin: 0 0 12px;
    padding: 0;
    list-style: none;
    font-size: 12.5px;
    color: var(--text-3);
  }

  .steps li {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .steps .current {
    color: var(--text);
  }

  .steps .done {
    color: var(--text-2);
  }

  .dot {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    font-size: 10.5px;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
  }

  .current .dot {
    background: var(--text);
    color: var(--bg);
    box-shadow: none;
  }

  .done .dot {
    background: var(--accent-soft);
    color: var(--accent);
    box-shadow: none;
  }

  .card {
    padding: 22px;
    border-radius: 14px;
    background: var(--bg-side);
    box-shadow: 0 0 0 1px var(--line) inset;
  }

  .nets,
  .own {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .nets {
    display: grid;
    border-radius: 10px;
    background: var(--bg);
    box-shadow: 0 0 0 1px var(--line) inset;
    overflow: hidden;
  }

  .nets li + li {
    border-top: 1px solid var(--line);
  }

  .net {
    display: grid;
    grid-template-columns: 112px 1fr auto 14px;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 11px 14px;
    text-align: left;
    color: var(--text-3);
    transition: background 120ms;
  }

  .net:hover {
    background: var(--bg-hover);
  }

  .net:focus-visible {
    outline-offset: -2px;
  }

  .net strong {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
  }

  .blurb {
    font-size: 12.5px;
    color: var(--text-2);
    line-height: 1.35;
  }

  .host {
    font-size: 11px;
    color: var(--text-3);
  }

  .or {
    margin: 18px 0 8px;
    font-size: 12px;
    color: var(--text-3);
  }

  .own {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .own-btn {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    width: 100%;
    height: 100%;
    padding: 11px 14px;
    border-radius: 10px;
    text-align: left;
    color: var(--text-3);
    box-shadow: 0 0 0 1px var(--line) inset;
    transition: background 120ms, box-shadow 120ms;
  }

  .own-btn:hover {
    background: var(--bg-hover);
    box-shadow: 0 0 0 1px var(--line-strong) inset;
  }

  .own-btn :global(svg) {
    flex: none;
    margin-top: 1px;
  }

  .own-btn > span {
    display: grid;
    gap: 2px;
  }

  .own-btn strong {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--text);
  }

  @media (max-width: 520px) {
    .net {
      grid-template-columns: 1fr 14px;
    }

    .net .blurb,
    .net .host {
      grid-column: 1;
    }

    .own {
      grid-template-columns: 1fr;
    }
  }

  form {
    display: grid;
    gap: 16px;
  }

  .grid {
    display: grid;
    grid-template-columns: 1fr 96px;
    gap: 12px;
  }

  .grid.two {
    grid-template-columns: 1fr 1fr;
  }

  fieldset {
    margin: 0;
    padding: 0;
    border: 0;
    display: grid;
  }

  legend {
    padding: 0;
    margin-bottom: 4px;
    font-size: 12px;
    color: var(--text-2);
  }

  code {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }

  .explain {
    margin: 0;
    font-size: 13px;
    color: var(--text-2);
    line-height: 1.5;
  }

  .note {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    margin: 0;
    font-size: 12px;
    color: var(--text-3);
    line-height: 1.45;
  }

  .note :global(svg) {
    flex: none;
    margin-top: 1px;
  }

  .note.warn {
    color: var(--warn);
    margin-bottom: 12px;
  }

  .bad,
  .error {
    color: var(--danger);
  }

  .error {
    margin: 0;
    font-size: 13px;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 4px;
  }

  .spacer {
    flex: 1;
  }

  .progress {
    display: grid;
    gap: 10px;
    margin: 0 0 16px;
    padding: 0;
    list-style: none;
    font-size: 13.5px;
    color: var(--text-3);
  }

  .progress li {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .progress .now {
    color: var(--text);
  }

  .progress .done {
    color: var(--text-2);
  }

  .tick {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
  }

  .done .tick {
    background: var(--accent-soft);
    color: var(--accent);
    box-shadow: none;
  }

  .spin {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 1.5px solid var(--line-strong);
    border-top-color: var(--accent);
    animation: spin 800ms linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spin {
      animation-duration: 2.4s;
    }
  }

  .failure {
    padding: 12px 14px;
    margin-bottom: 16px;
    border-radius: 10px;
    background: color-mix(in oklab, var(--danger) 9%, transparent);
    box-shadow: 0 0 0 1px color-mix(in oklab, var(--danger) 30%, transparent) inset;
  }

  .f-title {
    margin: 0;
    font-weight: 600;
    color: var(--danger);
  }

  .f-hint {
    margin: 4px 0 0;
    font-size: 13px;
    color: var(--text-2);
  }

  details {
    margin-top: 8px;
    font-size: 12px;
    color: var(--text-3);
  }

  details code {
    display: block;
    margin-top: 6px;
    word-break: break-word;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
</style>
