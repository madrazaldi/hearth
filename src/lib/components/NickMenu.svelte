<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { nickHue } from '../nickcolor';

  let { buf, nick, x, y, onclose }: { buf: Buffer; nick: string; x: number; y: number; onclose: () => void } = $props();

  const conn = $derived(buf.conn);
  const member = $derived(buf.member(nick));
  const me = $derived(buf.member(conn.nick));
  const myRank = $derived(me?.prefix ? conn.prefix.symbols.indexOf(me.prefix[0]) : 99);
  const opIndex = $derived(conn.prefix.symbols.indexOf('@'));
  const canModerate = $derived(buf.kind === 'channel' && myRank <= (opIndex < 0 ? 0 : opIndex) && myRank !== 99);
  const isOp = $derived(!!member?.prefix.includes('@'));
  const isVoice = $derived(!!member?.prefix.includes('+'));
  const self = $derived(conn.isMe(nick));
  const source = $derived({ nick, user: member?.user, host: member?.host });
  const ignored = $derived(app.isIgnored(conn, source));
  const watched = $derived(app.isWatched(conn, nick));
  const online = $derived(conn.online.get(conn.fold(nick)));

  let el = $state<HTMLDivElement>();
  let pos = $state({ left: 0, top: 0 });
  $effect(() => {
    if (!el) return;
    const r = el.getBoundingClientRect();
    pos = {
      left: Math.min(x, window.innerWidth - r.width - 12),
      top: Math.min(y + 8, window.innerHeight - r.height - 12),
    };
  });

  function act(fn: () => void) {
    fn();
    onclose();
  }

  function mention() {
    window.dispatchEvent(new CustomEvent('hearth:insert', { detail: `${nick}: ` }));
  }
</script>

<div
  class="pop"
  bind:this={el}
  style="left:{pos.left}px; top:{pos.top}px"
  role="dialog"
  tabindex="-1"
  onclick={(e) => e.stopPropagation()}
  onkeydown={(e) => e.key === 'Escape' && onclose()}
>
  <div class="head">
    <div class="name nick" style="--h:{nickHue(nick)}">{nick}</div>
    {#if member?.user}<div class="mask mono selectable">{member.user}@{member.host}</div>{/if}
    <div class="meta">
      {#if member?.prefix}<span>{member.prefix}</span>{/if}
      {#if member?.account}<span>signed in as {member.account}</span>{/if}
      {#if member?.away}<span class="away">away</span>{/if}
      {#if online !== undefined}<span class:on={online}>{online ? 'online' : 'offline'}</span>{/if}
      {#if ignored}<span class="away">ignored</span>{/if}
    </div>
    {#if member?.realname}<div class="real">{member.realname}</div>{/if}
  </div>
  <div class="actions">
    {#if !self}
      <button onclick={() => act(() => app.query(conn, nick))}>Message</button>
      <button onclick={() => act(mention)}>Mention</button>
    {/if}
    <button onclick={() => act(() => conn.sendFrom(buf, `WHOIS ${nick} ${nick}`))}>Whois</button>
    {#if !self && conn.canMonitor}
      <button onclick={() => act(() => (watched ? app.unwatch(conn, nick) : app.watch(conn, nick)))}>
        {watched ? 'Stop watching' : 'Tell me when online'}
      </button>
    {/if}
    {#if !self}
      <button onclick={() => act(() => (ignored ? app.unignore(nick) : app.ignore(nick)))}>{ignored ? 'Stop ignoring' : 'Ignore'}</button>
    {/if}
    {#if canModerate && !self}
      <hr />
      <button onclick={() => act(() => conn.send(`MODE ${buf.name} ${isOp ? '-' : '+'}o ${nick}`))}>{isOp ? 'Remove operator' : 'Make operator'}</button>
      <button onclick={() => act(() => conn.send(`MODE ${buf.name} ${isVoice ? '-' : '+'}v ${nick}`))}>{isVoice ? 'Remove voice' : 'Give voice'}</button>
      <button class="danger" onclick={() => act(() => conn.send(`KICK ${buf.name} ${nick}`))}>Kick</button>
    {/if}
  </div>
</div>

<style>
  .pop {
    position: fixed;
    z-index: 70;
    width: 260px;
    background: var(--bg-raised);
    border-radius: 10px;
    box-shadow: var(--shadow);
    overflow: hidden;
    animation: pop 140ms var(--ease);
  }

  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-3px) scale(0.99);
    }
  }

  .head {
    padding: 12px 14px 10px;
    border-bottom: 1px solid var(--line);
  }

  .name {
    font-size: 15px;
    font-weight: 600;
  }

  .mask {
    margin-top: 2px;
    font-size: 11.5px;
    color: var(--text-3);
    overflow-wrap: anywhere;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
    margin-top: 6px;
    font-size: 12px;
    color: var(--text-3);
  }

  .meta:empty {
    display: none;
  }

  .away {
    color: var(--warn);
  }

  .on {
    color: var(--ok);
  }

  .real {
    margin-top: 6px;
    font-size: 12.5px;
    color: var(--text-2);
  }

  .actions {
    padding: 4px;
    display: grid;
  }

  .actions button {
    text-align: left;
    padding: 6px 10px;
    border-radius: 6px;
    font-size: 13px;
  }

  .actions button:hover {
    background: var(--accent);
    color: var(--accent-text);
  }

  .actions .danger {
    color: var(--danger);
  }

  hr {
    border: 0;
    border-top: 1px solid var(--line);
    margin: 4px 0;
  }
</style>
