<script lang="ts" module>
  import { app, type Buffer } from '../state.svelte';
  import { stripFormatting } from '../format';
  import type { Line } from '../types';

  // One menu open at a time across all rows; rows unmount while scrolled away.
  const menu = $state({ react: null as number | null, del: null as number | null });

  export function closeMenus() {
    menu.react = null;
    menu.del = null;
  }

  // The row actions, shared with MessageList's keyboard shortcuts.

  export function canReply(buf: Buffer, l: Line): boolean {
    return !!l.msgid && buf.kind !== 'server' && buf.conn.canTag('draft/reply') && !l.redacted;
  }

  export function startReply(buf: Buffer, l: Line) {
    buf.replyTo = l;
    window.dispatchEvent(new CustomEvent('hearth:focus-composer'));
  }

  export async function copyLine(l: Line) {
    try {
      await navigator.clipboard.writeText(stripFormatting(l.text));
      app.toast('Copied');
    } catch (e) {
      app.toast(String(e), 'error');
    }
  }

  export function openReact(l: Line) {
    menu.del = null;
    menu.react = l.id;
  }

  export function askDelete(l: Line) {
    menu.react = null;
    menu.del = l.id;
  }
</script>

<script lang="ts">
  import { emojiName, menu as menuNav } from '../a11y';
  import Icon from './Icon.svelte';

  let { l, buf }: { l: Line; buf: Buffer } = $props();

  const EMOJI = ['👍', '❤️', '😂', '🎉', '😮', '😢', '👀', '✅'];
  const reply = $derived(canReply(buf, l));
  const react = $derived(app.canReact(buf, l));
  const redact = $derived(app.canRedact(buf, l));

  /** Back to the message row, before the focused control goes away. */
  function refocusRow(e: Event) {
    (e.currentTarget as HTMLElement).closest<HTMLElement>('[role="article"]')?.focus({ preventScroll: true });
  }
</script>

<div class="tools" class:open={menu.react === l.id || menu.del === l.id}>
  {#if reply}
    <button title="Reply" aria-label="Reply" onclick={() => startReply(buf, l)}><Icon name="reply" size={14} /></button>
  {/if}
  {#if react}
    <button
      title="React"
      aria-label="React"
      aria-haspopup="menu"
      aria-expanded={menu.react === l.id}
      class:on={menu.react === l.id}
      onclick={(e) => {
        e.stopPropagation();
        if (menu.react === l.id) menu.react = null;
        else openReact(l);
      }}
    >
      <Icon name="smile" size={14} />
    </button>
  {/if}
  {#if !l.redacted}
    <button title="Copy text" aria-label="Copy text" onclick={() => copyLine(l)}><Icon name="copy" size={14} /></button>
  {/if}
  {#if redact && !l.redacted}
    {#if menu.del === l.id}
      <button
        class="danger wide"
        onclick={(e) => {
          refocusRow(e);
          app.redact(buf, l);
          menu.del = null;
        }}
        onkeydown={(e) => {
          if (e.key !== 'Escape') return;
          e.preventDefault();
          e.stopPropagation();
          refocusRow(e);
          menu.del = null;
        }}>Delete?</button
      >
    {:else}
      <button title="Delete" aria-label="Delete message" class="danger" onclick={(e) => { e.stopPropagation(); askDelete(l); }}>
        <Icon name="trash" size={14} />
      </button>
    {/if}
  {/if}
  {#if menu.react === l.id}
    <div
      class="emoji"
      role="menu"
      aria-label="Reactions"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={() => {}}
      use:menuNav={{ onclose: () => (menu.react = null), horizontal: true }}
    >
      {#each EMOJI as e (e)}
        <button role="menuitem" aria-label={emojiName(e)} onclick={() => { app.react(buf, l, e); menu.react = null; }}>{e}</button>
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
  :global(.row.msg:focus-within) > .tools,
  :global(:focus-visible > .row.msg) > .tools,
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
