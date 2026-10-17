<script lang="ts">
  import { app, type Conn } from '../state.svelte';
  import { escapeAttr } from '../irc';
  import Switch from './Switch.svelte';
  import { Button } from '$lib/components/ui/button';
  import { Input } from '$lib/components/ui/input';
  import { Label } from '$lib/components/ui/label';
  import { cn } from '$lib/utils';

  let { conn }: { conn: Conn } = $props();

  type Draft = { netid: string | null; name: string; host: string; port: string; tls: boolean; nickname: string; username: string; pass: string };
  const blank = (): Draft => ({ netid: null, name: '', host: '', port: '6697', tls: true, nickname: '', username: '', pass: '' });

  const uid = $props.id();

  let editing = $state<Draft | null>(null);
  let confirmDelete = $state<string | null>(null);

  const networks = $derived(
    [...conn.networks.entries()].sort((a, b) => (a[1].name ?? a[0]).localeCompare(b[1].name ?? b[0])),
  );

  function edit(netid: string, attrs: Record<string, string>) {
    editing = {
      netid,
      name: attrs.name ?? '',
      host: attrs.host ?? '',
      port: attrs.port ?? '',
      tls: attrs.tls !== '0',
      nickname: attrs.nickname ?? '',
      username: attrs.username ?? '',
      pass: '',
    };
  }

  function attrs(d: Draft, original?: Record<string, string>): string {
    const out: string[] = [];
    const put = (k: string, v: string) => {
      if (original ? original[k] !== v : v) out.push(`${k}=${escapeAttr(v)}`);
    };
    put('name', d.name.trim());
    put('host', d.host.trim());
    if (d.port.trim()) put('port', d.port.trim());
    put('tls', d.tls ? '1' : '0');
    put('nickname', d.nickname.trim());
    put('username', d.username.trim());
    if (d.pass) out.push(`pass=${escapeAttr(d.pass)}`);
    return out.join(';');
  }

  function save(e: SubmitEvent) {
    e.preventDefault();
    const d = editing;
    if (!d) return;
    if (!d.host.trim()) return app.toast('A host is required', 'error');
    if (d.netid) {
      const changes = attrs(d, conn.networks.get(d.netid));
      if (changes) conn.send(`BOUNCER CHANGENETWORK ${d.netid} ${changes}`);
    } else conn.send(`BOUNCER ADDNETWORK ${attrs(d)}`);
    editing = null;
  }

  function remove(netid: string) {
    conn.send(`BOUNCER DELNETWORK ${netid}`);
    confirmDelete = null;
  }
</script>

<section class="mt-7 border-t pt-5">
  <div class="mb-3 flex items-center justify-between">
    <h3 class="text-[15px] font-semibold">Networks on this bouncer</h3>
    {#if !editing}<Button variant="outline" size="sm" onclick={() => (editing = blank())}>Add network</Button>{/if}
  </div>

  {#if editing}
    <form class="mb-3.5 grid gap-3 rounded-lg border bg-card p-3.5" onsubmit={save}>
      <div class="grid grid-cols-[1fr_1.4fr_90px] gap-2.5">
        <div class="grid gap-1.5">
          <Label for="{uid}-name">Name</Label>
          <Input id="{uid}-name" bind:value={editing.name} placeholder="Libera.Chat" />
        </div>
        <div class="grid gap-1.5">
          <Label for="{uid}-host">Host</Label>
          <Input id="{uid}-host" class="mono" bind:value={editing.host} placeholder="irc.libera.chat" spellcheck="false" />
        </div>
        <div class="grid gap-1.5">
          <Label for="{uid}-port">Port</Label>
          <Input id="{uid}-port" class="mono" bind:value={editing.port} inputmode="numeric" />
        </div>
      </div>
      <div class="grid grid-cols-3 gap-2.5">
        <div class="grid gap-1.5">
          <Label for="{uid}-nick">Nickname</Label>
          <Input id="{uid}-nick" bind:value={editing.nickname} placeholder="Bouncer default" spellcheck="false" />
        </div>
        <div class="grid gap-1.5">
          <Label for="{uid}-user">Username</Label>
          <Input id="{uid}-user" bind:value={editing.username} placeholder="Bouncer default" spellcheck="false" />
        </div>
        <div class="grid gap-1.5">
          <Label for="{uid}-pass">Server password</Label>
          <Input id="{uid}-pass" type="password" bind:value={editing.pass} placeholder={editing.netid ? 'Unchanged' : 'Usually empty'} />
        </div>
      </div>
      <Switch bind:checked={editing.tls} label="Use TLS" />
      <div class="flex justify-end gap-2">
        <Button type="button" variant="ghost" onclick={() => (editing = null)}>Cancel</Button>
        <Button type="submit">{editing.netid ? 'Save network' : 'Add network'}</Button>
      </div>
    </form>
  {/if}

  <div class="grid divide-y overflow-hidden rounded-lg border">
    {#each networks as [netid, a] (netid)}
      <div class="flex items-center gap-2.5 px-3.5 py-2.5">
        <span
          class={cn(
            'size-1.5 flex-none rounded-full bg-muted-foreground/50',
            a.state === 'connected' && 'bg-ok',
            a.state === 'connecting' && 'bg-warn',
            a.state === 'disconnected' && 'bg-destructive',
          )}
        ></span>
        <div class="min-w-0 flex-1">
          <div class="text-sm font-medium">{a.name || a.host || netid}</div>
          <div class="mono truncate text-xs text-muted-foreground">
            {a.host}{a.port ? ':' + a.port : ''}{a.tls === '0' ? '' : ' · TLS'}{a.error ? ` · ${a.error}` : ''}
          </div>
        </div>
        {#if confirmDelete === netid}
          <Button variant="destructive" size="sm" onclick={() => remove(netid)}>Really remove?</Button>
          <Button variant="ghost" size="sm" onclick={() => (confirmDelete = null)}>Keep</Button>
        {:else}
          <Button variant="ghost" size="sm" class="text-destructive hover:text-destructive" onclick={() => (confirmDelete = netid)}>Remove</Button>
          <Button variant="outline" size="sm" onclick={() => edit(netid, a)}>Edit</Button>
        {/if}
      </div>
    {:else}
      <p class="m-0 p-3.5 text-[13px] text-muted-foreground">No networks yet.</p>
    {/each}
  </div>
</section>
