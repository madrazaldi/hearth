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
  import { cn } from '$lib/utils';
  import { buttonVariants } from '$lib/components/ui/button';
  import Icon from './Icon.svelte';

  let { l, buf }: { l: Line; buf: Buffer } = $props();

  // Computed once, not per row: the toolbar is in every row's DOM (shown by CSS
  // on hover), so the buttons are plain elements wearing shadcn's classes
  // instead of component instances.
  const tool = cn(buttonVariants({ variant: 'ghost', size: 'icon-xs' }), 'text-muted-foreground [&_svg]:size-3.5!');
  const danger = cn(tool, 'hover:text-destructive');
  const confirm = cn(buttonVariants({ variant: 'destructive', size: 'xs' }), 'wide');
  const emoji = cn(buttonVariants({ variant: 'ghost', size: 'icon-sm' }), 'text-base');

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
    <button class={tool} title="Reply" aria-label="Reply" onclick={() => startReply(buf, l)}><Icon name="reply" size={14} /></button>
  {/if}
  {#if react}
    <button
      title="React"
      aria-label="React"
      aria-haspopup="menu"
      aria-expanded={menu.react === l.id}
      class={tool}
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
    <button class={tool} title="Copy text" aria-label="Copy text" onclick={() => copyLine(l)}><Icon name="copy" size={14} /></button>
  {/if}
  {#if redact && !l.redacted}
    {#if menu.del === l.id}
      <button
        class={confirm}
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
      <button title="Delete" aria-label="Delete message" class={danger} onclick={(e) => { e.stopPropagation(); askDelete(l); }}>
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
        <button class={emoji} role="menuitem" aria-label={emojiName(e)} onclick={() => { app.react(buf, l, e); menu.react = null; }}>{e}</button>
      {/each}
    </div>
  {/if}
</div>

<style>
  /* Popover-like group: bordered, with a small shadow. */
  .tools {
    position: absolute;
    right: 16px;
    top: -14px;
    display: none;
    align-items: center;
    gap: 1px;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: calc(var(--radius) - 2px);
    background: var(--popover);
    box-shadow: 0 1px 2px 0 rgb(0 0 0 / 0.12), 0 1px 3px 0 rgb(0 0 0 / 0.1);
    z-index: 3;
  }

  :global(.row.msg:hover) > .tools,
  :global(.row.msg:focus-within) > .tools,
  :global(:focus-visible > .row.msg) > .tools,
  .tools.open {
    display: flex;
  }

  .emoji {
    position: absolute;
    right: -1px;
    top: calc(100% + 4px);
    display: flex;
    gap: 1px;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--popover);
    box-shadow: 0 4px 6px -1px rgb(0 0 0 / 0.2), 0 2px 4px -2px rgb(0 0 0 / 0.15);
  }
</style>
