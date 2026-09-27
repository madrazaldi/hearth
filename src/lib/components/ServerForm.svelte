<script lang="ts">
  import type { CertInfo, ServerConfig } from '../types';
  import { api } from '../api';
  import Switch from './Switch.svelte';
  import Segmented from './Segmented.svelte';

  let {
    server,
    submitLabel = 'Save & connect',
    onsave,
    oncancel,
    ondelete,
  }: {
    server: ServerConfig;
    submitLabel?: string;
    onsave: (s: ServerConfig) => Promise<void>;
    oncancel?: () => void;
    ondelete?: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let s = $state<ServerConfig>(structuredClone($state.snapshot(server)) as ServerConfig);
  let autojoin = $state(s.autojoin.join(', '));
  let perform = $state(s.perform.join('\n'));
  let saving = $state(false);
  let error = $state('');
  let advanced = $state(false);
  let confirmDelete = $state(false);

  const transport = $derived(s.websocket ? 'ws' : 'tcp');

  let cert = $state<CertInfo | null>(null);
  let certError = $state('');

  $effect(() => {
    const path = s.clientCert.trim();
    cert = null;
    certError = '';
    if (!path) return;
    api.certInfo(path).then(
      (info) => (cert = info),
      (e) => (certError = String(e)),
    );
  });

  async function generateCert() {
    try {
      const info = await api.generateCert(s.id || `new-${crypto.randomUUID().slice(0, 8)}`);
      s.clientCert = info.path;
    } catch (e) {
      certError = String(e);
    }
  }

  function setTransport(v: string) {
    s.websocket = v === 'ws';
    if (s.websocket && (s.port === 6697 || s.port === 6667)) s.port = s.tls ? 443 : 80;
    if (!s.websocket && (s.port === 443 || s.port === 80)) s.port = s.tls ? 6697 : 6667;
  }

  function setTls(v: boolean) {
    s.tls = v;
    if (!s.websocket && s.port === (v ? 6667 : 6697)) s.port = v ? 6697 : 6667;
    if (s.websocket && s.port === (v ? 80 : 443)) s.port = v ? 443 : 80;
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    error = '';
    if (!s.host.trim()) return (error = 'A host is required.');
    if (!s.nick.trim()) return (error = 'Choose a nickname.');
    s.autojoin = autojoin
      .split(/[\s,]+/)
      .map((c) => c.trim())
      .filter(Boolean)
      .map((c) => (/^[#&+!]/.test(c) ? c : '#' + c));
    s.perform = perform.split('\n').map((l) => l.trim()).filter(Boolean);
    if (s.sasl === 'plain' && !s.saslUsername) s.saslUsername = s.nick;
    if (s.sasl === 'external' && !s.clientCert.trim()) return (error = 'Choose or generate a client certificate.');
    if (s.sasl === 'external' && !s.tls) return (error = 'Certificate authentication needs TLS.');
    saving = true;
    try {
      await onsave($state.snapshot(s) as ServerConfig);
    } catch (err) {
      error = String(err);
    } finally {
      saving = false;
    }
  }
</script>

<form onsubmit={submit}>
  <div class="grid">
    <label class="field span2">
      <span>Name</span>
      <input class="input" bind:value={s.name} placeholder="Libera.Chat" />
    </label>
    <label class="field grow">
      <span>Host</span>
      <input class="input mono" bind:value={s.host} placeholder="irc.libera.chat" spellcheck="false" autocapitalize="off" />
    </label>
    <label class="field port">
      <span>Port</span>
      <input class="input mono" type="number" min="1" max="65535" bind:value={s.port} />
    </label>
  </div>

  <div class="row">
    <div class="field">
      <span>Transport</span>
      <Segmented
        label="Transport"
        options={[
          { value: 'tcp', label: 'IRC' },
          { value: 'ws', label: 'WebSocket' },
        ]}
        bind:value={() => transport, setTransport}
      />
    </div>
    <div class="tls">
      <Switch bind:checked={() => s.tls, setTls} label="Use TLS" />
    </div>
  </div>

  {#if s.websocket}
    <label class="field">
      <span>WebSocket path</span>
      <input class="input mono" bind:value={s.websocketPath} placeholder="/socket" spellcheck="false" />
    </label>
  {/if}

  <div class="grid three">
    <label class="field">
      <span>Nickname</span>
      <input class="input" bind:value={s.nick} spellcheck="false" autocapitalize="off" />
    </label>
    <label class="field">
      <span>Username</span>
      <input class="input" bind:value={s.username} placeholder={s.nick || 'optional'} spellcheck="false" autocapitalize="off" />
    </label>
    <label class="field">
      <span>Real name</span>
      <input class="input" bind:value={s.realname} placeholder={s.nick || 'optional'} />
    </label>
  </div>

  <div class="field">
    <span>Authentication</span>
    <Segmented
      label="Authentication"
      bind:value={s.sasl}
      options={[
        { value: 'none', label: 'None' },
        { value: 'plain', label: 'Password' },
        { value: 'external', label: 'Certificate' },
      ]}
    />
  </div>
  {#if s.sasl === 'plain'}
    <div class="grid two">
      <label class="field">
        <span>Account</span>
        <input class="input" bind:value={s.saslUsername} placeholder={s.nick} spellcheck="false" autocapitalize="off" />
      </label>
      <label class="field">
        <span>Password</span>
        <input class="input" type="password" bind:value={s.saslPassword} />
      </label>
    </div>
    <small class="note">Uses SCRAM-SHA-256 when the server offers it, so the password never crosses the wire. Stored in your system keychain.</small>
  {:else if s.sasl === 'external'}
    <div class="field">
      <span>Client certificate</span>
      <div class="cert-row">
        <input class="input mono" bind:value={s.clientCert} placeholder="/path/to/cert.pem (certificate and key)" spellcheck="false" />
        <button type="button" class="btn" onclick={generateCert}>Generate</button>
      </div>
      {#if cert}
        <small class="fp">SHA-512 <code class="selectable">{cert.sha512}</code></small>
        <small>
          Register it with the network's services while connected — on Libera.Chat and most Atheme networks:
          <code class="selectable">/msg NickServ CERT ADD</code>
        </small>
      {:else if certError}
        <small class="bad">{certError}</small>
      {/if}
    </div>
  {/if}

  <label class="field">
    <span>Channels to join</span>
    <input class="input mono" bind:value={autojoin} placeholder="#hearth, #rust" spellcheck="false" />
    <small>Ignored when connecting through a bouncer, which restores channels itself.</small>
  </label>

  <button type="button" class="disclosure" aria-expanded={advanced} onclick={() => (advanced = !advanced)}>
    <span aria-hidden="true">{advanced ? '▾' : '▸'}</span> Advanced
  </button>
  {#if advanced}
    <div class="advanced">
      <label class="field">
        <span>Server password</span>
        <input class="input" type="password" bind:value={s.password} placeholder="Usually empty" />
      </label>
      <label class="field">
        <span>Commands after connecting</span>
        <textarea class="input mono" rows="3" bind:value={perform} placeholder={'MODE $nick +x\nPRIVMSG NickServ :identify …'}></textarea>
        <small>One raw IRC line each. <code>$nick</code> expands to your nickname.</small>
      </label>
      <Switch bind:checked={s.autoconnect} label="Connect on launch" />
      <Switch
        bind:checked={s.acceptInvalidCerts}
        label="Accept invalid certificates"
        hint="Only for servers you trust with self-signed certificates."
      />
    </div>
  {/if}

  {#if error}<p class="error">{error}</p>{/if}

  <div class="actions">
    {#if ondelete}
      {#if confirmDelete}
        <button type="button" class="btn danger" onclick={ondelete}>Really remove?</button>
      {:else}
        <button type="button" class="btn ghost danger" onclick={() => (confirmDelete = true)}>Remove server</button>
      {/if}
    {/if}
    <span class="spacer"></span>
    {#if oncancel}<button type="button" class="btn ghost" onclick={oncancel}>Cancel</button>{/if}
    <button type="submit" class="btn primary" disabled={saving}>{saving ? 'Saving…' : submitLabel}</button>
  </div>
</form>

<style>
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

  .grid.three {
    grid-template-columns: 1fr 1fr 1fr;
  }

  .span2 {
    grid-column: span 2;
  }

  .row {
    display: flex;
    align-items: flex-end;
    gap: 24px;
  }

  .tls {
    padding-bottom: 2px;
    min-width: 150px;
  }

  .disclosure {
    justify-self: start;
    font-size: 12.5px;
    color: var(--text-3);
  }

  .disclosure:hover {
    color: var(--text);
  }

  .advanced {
    display: grid;
    gap: 14px;
    padding-left: 12px;
    border-left: 1px solid var(--line);
  }

  code {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }

  .note {
    margin-top: -8px;
    font-size: 12px;
    color: var(--text-3);
  }

  .cert-row {
    display: flex;
    gap: 8px;
  }

  .cert-row .input {
    flex: 1;
  }

  .fp code {
    word-break: break-all;
  }

  .bad {
    color: var(--danger);
  }

  .error {
    margin: 0;
    color: var(--danger);
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
</style>
