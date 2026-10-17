<script lang="ts">
  import type { CertInfo, ServerConfig } from '../types';
  import { api } from '../api';
  import Switch from './Switch.svelte';
  import Segmented from './Segmented.svelte';
  import { Button } from '$lib/components/ui/button';
  import { Input } from '$lib/components/ui/input';
  import { Textarea } from '$lib/components/ui/textarea';
  import * as Field from '$lib/components/ui/field';
  import { Separator } from '$lib/components/ui/separator';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';

  const uid = $props.id();

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

<form onsubmit={submit} class="grid gap-6">
  <Field.Group class="gap-4">
    <div class="grid grid-cols-[1fr_96px] gap-3">
      <Field.Field class="col-span-2">
        <Field.Label for="{uid}-name">Name</Field.Label>
        <Input id="{uid}-name" bind:value={s.name} placeholder="Libera.Chat" />
      </Field.Field>
      <Field.Field>
        <Field.Label for="{uid}-host">Host</Field.Label>
        <Input id="{uid}-host" class="font-mono" bind:value={s.host} placeholder="irc.libera.chat" spellcheck="false" autocapitalize="off" />
      </Field.Field>
      <Field.Field>
        <Field.Label for="{uid}-port">Port</Field.Label>
        <Input id="{uid}-port" class="font-mono" type="number" min="1" max="65535" bind:value={s.port} />
      </Field.Field>
    </div>

    <div class="flex items-end gap-6">
      <Field.Field class="w-auto">
        <Field.Title>Transport</Field.Title>
        <Segmented
          label="Transport"
          options={[
            { value: 'tcp', label: 'IRC' },
            { value: 'ws', label: 'WebSocket' },
          ]}
          bind:value={() => transport, setTransport}
        />
      </Field.Field>
      <div class="w-36 pb-0.5">
        <Switch bind:checked={() => s.tls, setTls} label="Use TLS" />
      </div>
    </div>

    {#if s.websocket}
      <Field.Field>
        <Field.Label for="{uid}-wspath">WebSocket path</Field.Label>
        <Input id="{uid}-wspath" class="font-mono" bind:value={s.websocketPath} placeholder="/socket" spellcheck="false" />
      </Field.Field>
    {/if}
  </Field.Group>

  <Separator />

  <Field.Group class="gap-4">
    <div class="grid grid-cols-3 gap-3">
      <Field.Field>
        <Field.Label for="{uid}-nick">Nickname</Field.Label>
        <Input id="{uid}-nick" bind:value={s.nick} spellcheck="false" autocapitalize="off" />
      </Field.Field>
      <Field.Field>
        <Field.Label for="{uid}-user">Username</Field.Label>
        <Input id="{uid}-user" bind:value={s.username} placeholder={s.nick || 'optional'} spellcheck="false" autocapitalize="off" />
      </Field.Field>
      <Field.Field>
        <Field.Label for="{uid}-real">Real name</Field.Label>
        <Input id="{uid}-real" bind:value={s.realname} placeholder={s.nick || 'optional'} />
      </Field.Field>
    </div>

    <Field.Field>
      <Field.Title>Authentication</Field.Title>
      <Segmented
        label="Authentication"
        bind:value={s.sasl}
        options={[
          { value: 'none', label: 'None' },
          { value: 'plain', label: 'Password' },
          { value: 'external', label: 'Certificate' },
        ]}
      />
    </Field.Field>
    {#if s.sasl === 'plain'}
      <div class="grid grid-cols-2 gap-3">
        <Field.Field>
          <Field.Label for="{uid}-account">Account</Field.Label>
          <Input id="{uid}-account" bind:value={s.saslUsername} placeholder={s.nick} spellcheck="false" autocapitalize="off" />
        </Field.Field>
        <Field.Field>
          <Field.Label for="{uid}-pass">Password</Field.Label>
          <Input id="{uid}-pass" type="password" bind:value={s.saslPassword} />
        </Field.Field>
      </div>
      <Field.Description class="-mt-2 text-xs">
        Uses SCRAM-SHA-256 when the server offers it, so the password never crosses the wire. Stored in your system keychain.
      </Field.Description>
    {:else if s.sasl === 'external'}
      <Field.Field>
        <Field.Label for="{uid}-cert">Client certificate</Field.Label>
        <div class="flex gap-2">
          <Input id="{uid}-cert" class="flex-1 font-mono" bind:value={s.clientCert} placeholder="/path/to/cert.pem (certificate and key)" spellcheck="false" />
          <Button variant="outline" onclick={generateCert}>Generate</Button>
        </div>
        {#if cert}
          <Field.Description class="text-xs">SHA-512 <code class="selectable font-mono text-[11.5px] break-all">{cert.sha512}</code></Field.Description>
          <Field.Description class="text-xs">
            Register it with the network's services while connected — on Libera.Chat and most Atheme networks:
            <code class="selectable font-mono text-[11.5px]">/msg NickServ CERT ADD</code>
          </Field.Description>
        {:else if certError}
          <Field.Error>{certError}</Field.Error>
        {/if}
      </Field.Field>
    {/if}
  </Field.Group>

  <Separator />

  <Field.Field>
    <Field.Label for="{uid}-autojoin">Channels to join</Field.Label>
    <Input id="{uid}-autojoin" class="font-mono" bind:value={autojoin} placeholder="#hearth, #rust" spellcheck="false" />
    <Field.Description class="text-xs">Ignored when connecting through a bouncer, which restores channels itself.</Field.Description>
  </Field.Field>

  <div class="grid gap-4">
    <Button variant="ghost" size="sm" class="justify-self-start text-muted-foreground" aria-expanded={advanced} onclick={() => (advanced = !advanced)}>
      <ChevronRight class="transition-transform {advanced ? 'rotate-90' : ''}" />Advanced
    </Button>
    {#if advanced}
      <Field.Group class="gap-4 border-l border-border pl-4">
        <Field.Field>
          <Field.Label for="{uid}-srvpass">Server password</Field.Label>
          <Input id="{uid}-srvpass" type="password" bind:value={s.password} placeholder="Usually empty" />
        </Field.Field>
        <Field.Field>
          <Field.Label for="{uid}-perform">Commands after connecting</Field.Label>
          <Textarea id="{uid}-perform" class="font-mono" rows={3} bind:value={perform} placeholder={'MODE $nick +x\nPRIVMSG NickServ :identify …'} spellcheck="false" />
          <Field.Description class="text-xs">One raw IRC line each. <code class="font-mono text-[11.5px]">$nick</code> expands to your nickname.</Field.Description>
        </Field.Field>
        <Switch bind:checked={s.autoconnect} label="Connect on launch" />
        <Switch
          bind:checked={s.acceptInvalidCerts}
          label="Accept invalid certificates"
          hint="Only for servers you trust with self-signed certificates."
        />
      </Field.Group>
    {/if}
  </div>

  {#if error}<Field.Error>{error}</Field.Error>{/if}

  <div class="flex items-center gap-2">
    {#if ondelete}
      {#if confirmDelete}
        <Button variant="destructive" class="bg-destructive text-white hover:bg-destructive/90" onclick={ondelete}>Really remove?</Button>
      {:else}
        <Button variant="destructive" onclick={() => (confirmDelete = true)}>Remove server</Button>
      {/if}
    {/if}
    <span class="flex-1"></span>
    {#if oncancel}<Button variant="ghost" onclick={oncancel}>Cancel</Button>{/if}
    <Button type="submit" disabled={saving}>{saving ? 'Saving…' : submitLabel}</Button>
  </div>
</form>
