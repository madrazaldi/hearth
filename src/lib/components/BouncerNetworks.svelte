<script lang="ts">
  import { app, type Conn } from '../state.svelte';
  import { escapeAttr } from '../irc';
  import Switch from './Switch.svelte';

  let { conn }: { conn: Conn } = $props();

  type Draft = { netid: string | null; name: string; host: string; port: string; tls: boolean; nickname: string; username: string; pass: string };
  const blank = (): Draft => ({ netid: null, name: '', host: '', port: '6697', tls: true, nickname: '', username: '', pass: '' });

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

<section>
  <div class="head">
    <h3>Networks on this bouncer</h3>
    {#if !editing}<button class="btn" onclick={() => (editing = blank())}>Add network</button>{/if}
  </div>

  {#if editing}
    <form onsubmit={save}>
      <div class="grid">
        <label class="field"><span>Name</span><input class="input" bind:value={editing.name} placeholder="Libera.Chat" /></label>
        <label class="field grow"><span>Host</span><input class="input mono" bind:value={editing.host} placeholder="irc.libera.chat" spellcheck="false" /></label>
        <label class="field port"><span>Port</span><input class="input mono" bind:value={editing.port} inputmode="numeric" /></label>
      </div>
      <div class="grid three">
        <label class="field"><span>Nickname</span><input class="input" bind:value={editing.nickname} placeholder="Bouncer default" spellcheck="false" /></label>
        <label class="field"><span>Username</span><input class="input" bind:value={editing.username} placeholder="Bouncer default" spellcheck="false" /></label>
        <label class="field">
          <span>Server password</span>
          <input class="input" type="password" bind:value={editing.pass} placeholder={editing.netid ? 'Unchanged' : 'Usually empty'} />
        </label>
      </div>
      <Switch bind:checked={editing.tls} label="Use TLS" />
      <div class="actions">
        <span class="spacer"></span>
        <button type="button" class="btn ghost" onclick={() => (editing = null)}>Cancel</button>
        <button type="submit" class="btn primary">{editing.netid ? 'Save network' : 'Add network'}</button>
      </div>
    </form>
  {/if}

  <div class="list">
    {#each networks as [netid, a] (netid)}
      <div class="net">
        <span class="dot {a.state ?? ''}"></span>
        <div class="main">
          <div class="name">{a.name || a.host || netid}</div>
          <div class="host mono">{a.host}{a.port ? ':' + a.port : ''}{a.tls === '0' ? '' : ' · TLS'}{a.error ? ` · ${a.error}` : ''}</div>
        </div>
        {#if confirmDelete === netid}
          <button class="btn danger" onclick={() => remove(netid)}>Really remove?</button>
          <button class="btn ghost" onclick={() => (confirmDelete = null)}>Keep</button>
        {:else}
          <button class="btn ghost danger" onclick={() => (confirmDelete = netid)}>Remove</button>
          <button class="btn" onclick={() => edit(netid, a)}>Edit</button>
        {/if}
      </div>
    {:else}
      <p class="empty">No networks yet.</p>
    {/each}
  </div>
</section>

<style>
  section {
    margin-top: 28px;
    padding-top: 20px;
    border-top: 1px solid var(--line);
  }

  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
  }

  h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  form {
    display: grid;
    gap: 12px;
    padding: 14px;
    margin-bottom: 14px;
    border-radius: 10px;
    background: var(--bg-side);
    box-shadow: 0 0 0 1px var(--line) inset;
  }

  .grid {
    display: grid;
    grid-template-columns: 1fr 1.4fr 90px;
    gap: 10px;
  }

  .grid.three {
    grid-template-columns: 1fr 1fr 1fr;
  }

  .actions {
    display: flex;
    gap: 8px;
  }

  .spacer {
    flex: 1;
  }

  .list {
    display: grid;
    border-radius: 10px;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
    overflow: hidden;
  }

  .net {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
  }

  .net + .net {
    border-top: 1px solid var(--line);
  }

  .main {
    flex: 1;
    min-width: 0;
  }

  .name {
    font-weight: 500;
  }

  .host {
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

  .dot.connecting {
    background: var(--warn);
  }

  .dot.disconnected {
    background: var(--danger);
  }

  .empty {
    margin: 0;
    padding: 14px;
    color: var(--text-3);
    font-size: 13px;
  }
</style>
