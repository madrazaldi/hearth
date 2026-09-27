<script lang="ts" module>
  // One menu open at a time across all rows; rows unmount while scrolled away.
  const menu = $state({ react: null as number | null, del: null as number | null });

  export function closeMenus() {
    menu.react = null;
    menu.del = null;
  }
</script>

<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { stripFormatting } from '../format';
  import type { Line } from '../types';
  import Icon from './Icon.svelte';

  let { l, buf }: { l: Line; buf: Buffer } = $props();

  const EMOJI = ['👍', '❤️', '😂', '🎉', '😮', '😢', '👀', '✅'];
  const canReply = $derived(!!l.msgid && buf.kind !== 'server' && buf.conn.canTag('draft/reply') && !l.redacted);
  const react = $derived(app.canReact(buf, l));
  const redact = $derived(app.canRedact(buf, l));

  async function copy() {
    try {
      await navigator.clipboard.writeText(stripFormatting(l.text));
      app.toast('Copied');
    } catch (e) {
      app.toast(String(e), 'error');
    }
  }

  function startReply() {
    buf.replyTo = l;
    window.dispatchEvent(new CustomEvent('hearth:focus-composer'));
  }
</script>

<div class="tools" class:open={menu.react === l.id || menu.del === l.id}>
  {#if canReply}
    <button title="Reply" onclick={startReply}><Icon name="reply" size={14} /></button>
  {/if}
  {#if react}
    <button title="React" class:on={menu.react === l.id} onclick={(e) => { e.stopPropagation(); menu.react = menu.react === l.id ? null : l.id; }}>
      <Icon name="smile" size={14} />
    </button>
  {/if}
  {#if !l.redacted}
    <button title="Copy text" onclick={copy}><Icon name="copy" size={14} /></button>
  {/if}
  {#if redact && !l.redacted}
    {#if menu.del === l.id}
      <button class="danger wide" onclick={() => { app.redact(buf, l); menu.del = null; }}>Delete?</button>
    {:else}
      <button title="Delete" class="danger" onclick={(e) => { e.stopPropagation(); menu.del = l.id; }}><Icon name="trash" size={14} /></button>
    {/if}
  {/if}
  {#if menu.react === l.id}
    <div class="emoji" role="menu" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
      {#each EMOJI as e (e)}
        <button onclick={() => { app.react(buf, l, e); menu.react = null; }}>{e}</button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .tools {
    position: absolute;
    right: 16px;
    top: -14px;
    display: none;
    align-items: center;
    gap: 1px;
    padding: 2px;
    border-radius: 7px;
    background: var(--bg-raised);
    box-shadow: var(--shadow);
    z-index: 3;
  }

  :global(.row.msg:hover) > .tools,
  .tools.open {
    display: flex;
  }

  .tools button {
    height: 24px;
    min-width: 26px;
    display: grid;
    place-items: center;
    border-radius: 5px;
    color: var(--text-3);
  }

  .tools button:hover,
  .tools button.on {
    background: var(--bg-active);
    color: var(--text);
  }

  .tools .danger:hover {
    color: var(--danger);
  }

  .tools .wide {
    padding: 0 8px;
    font-size: 12px;
    color: var(--danger);
  }

  .emoji {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    display: flex;
    gap: 2px;
    padding: 4px;
    border-radius: 8px;
    background: var(--bg-raised);
    box-shadow: var(--shadow);
  }

  .emoji button {
    font-size: 16px;
    width: 30px;
    height: 30px;
  }
</style>
