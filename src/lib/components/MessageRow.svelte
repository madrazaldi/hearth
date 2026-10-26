<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { firstLink, stripFormatting } from '../format';
  import { isNsfw } from '../media';
  import { nickHue } from '../nickcolor';
  import { formatClock, formatFull } from '../time';
  import type { Line } from '../types';
  import RichText from './RichText.svelte';
  import LinkPreview from './LinkPreview.svelte';
  import MessageTools from './MessageTools.svelte';
  import Nick from './Nick.svelte';
  import Avatar from './Avatar.svelte';

  let {
    l,
    cont,
    buf,
    flash,
    onnick,
  }: { l: Line; cont: boolean; buf: Buffer; flash: boolean; onnick: (nick: string, e: MouseEvent) => void } = $props();

  const s = $derived(app.settings);
  const clock = $derived(formatClock(l.time, s));
  const displayName = $derived(buf.conn.profile(l.nick).displayName);
  const url = $derived(s.linkPreviews && !l.redacted && buf.kind !== 'server' ? firstLink(l.text) : null);
</script>

{#snippet body()}
  {#if l.redacted}
    <span class="faint"><i>message deleted</i></span>
  {:else}
    <span class="body"><RichText text={l.text} /></span>
  {/if}
{/snippet}

{#snippet quote()}
  {#if l.replyTo && !l.redacted}
    {@const parent = buf.byMsgid.get(l.replyTo)}
    <button
      class="reply-quote"
      disabled={!parent}
      onclick={() => parent && app.jumpTo(buf, parent.time, parent.msgid)}
      title={parent ? 'Show the original message' : undefined}
    >
      <span class="reply-arrow">↪</span>
      {#if parent}
        <span class="reply-nick" style="--h:{nickHue(parent.nick)}">{parent.nick}</span>
        <span class="reply-snip">{parent.redacted ? 'message deleted' : stripFormatting(parent.text).slice(0, 140)}</span>
      {:else}
        <span class="faint">a message that isn't loaded</span>
      {/if}
    </button>
  {/if}
{/snippet}

{#snippet extras()}
  {#if l.reactions && !l.redacted}
    <div class="reactions">
      {#each Object.entries(l.reactions) as [emoji, nicks] (emoji)}
        {@const mine = nicks.some((n) => buf.conn.isMe(n))}
        <button
          class="chip"
          class:mine
          title={nicks.join(', ')}
          disabled={mine || !app.canReact(buf, l)}
          onclick={() => app.react(buf, l, emoji)}>{emoji}<span>{nicks.length}</span></button
        >
      {/each}
    </div>
  {/if}
  {#if url}<LinkPreview {url} nsfw={isNsfw(l.text)} />{/if}
{/snippet}

<div class="row msg {l.type}" class:cont class:hl={l.highlight} class:self={l.self} class:flash data-line={l.id}>
  {#if s.layout === 'cozy'}
    <div class="gutter">
      {#if cont || l.type === 'action'}
        <time title={formatFull(l.time)}>{clock}</time>
      {:else}
        <!-- The nick beside it is the keyboard route to the same menu. -->
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <span class="av" onclick={(e) => onnick(l.nick, e)}><Avatar conn={buf.conn} nick={l.nick} size={36} /></span>
      {/if}
    </div>
    <div class="text selectable">
      {#if !cont && l.type !== 'action'}
        <div class="head">
          <span>{#if l.prefix}<span class="mode-prefix">{l.prefix}</span>{/if}<Nick nick={l.nick} self={l.self} {onnick} /></span>
          {#if displayName && displayName !== l.nick}<span class="display-name">{displayName}</span>{/if}
          {#if l.type === 'notice'}<span class="tag">notice</span>{/if}
          <time class="head-time" title={formatFull(l.time)}>{clock}</time>
        </div>
      {/if}
      {@render quote()}
      {#if l.type === 'action'}<span class="glyph">*</span>{' '}<Nick nick={l.nick} self={l.self} {onnick} />{' '}{/if}{@render body()}
      {@render extras()}
    </div>
  {:else}
  <time title={formatFull(l.time)}>{clock}</time>
  {#if s.layout === 'columns'}
    <span class="who">
      {#if l.type === 'action'}<span class="glyph">*</span>
      {:else if !cont}
        {#if l.type === 'notice'}<span class="notice-mark">–</span>{/if}{#if l.prefix}<span class="mode-prefix">{l.prefix}</span>{/if}<Nick nick={l.nick} self={l.self} {onnick} />
      {/if}
    </span>
    <div class="text selectable">
      {@render quote()}
      {#if l.type === 'action'}<Nick nick={l.nick} self={l.self} {onnick} />{' '}{/if}{@render body()}
      {@render extras()}
    </div>
  {:else}
    <div class="text selectable">
      {#if !cont && l.type !== 'action'}
        <div class="head" title={displayName && displayName !== l.nick ? `${displayName} (${l.nick})` : undefined}>
          <!-- Always the same size, image or initial, so row heights don't shift. -->
          {#if s.loadAvatars}<Avatar conn={buf.conn} nick={l.nick} size={18} />{/if}
          <span>{#if l.prefix}<span class="mode-prefix">{l.prefix}</span>{/if}<Nick nick={l.nick} self={l.self} {onnick} /></span>
          {#if l.type === 'notice'}<span class="tag">notice</span>{/if}
          <span class="head-time" title={formatFull(l.time)}>{clock}</span>
        </div>
      {/if}
      {@render quote()}
      {#if l.type === 'action'}<span class="glyph">*</span>{' '}<Nick nick={l.nick} self={l.self} {onnick} />{' '}{/if}{@render body()}
      {@render extras()}
    </div>
  {/if}
  {/if}
  <MessageTools {l} {buf} />
</div>

<style>
  .body {
    white-space: pre-wrap;
  }

  .mode-prefix {
    color: var(--muted-foreground);
    font-weight: 500;
  }

  .notice-mark {
    color: var(--muted-foreground);
    margin-right: 4px;
  }

  .head :global(.avatar) {
    align-self: center;
  }

  .head-time {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--muted-foreground);
  }

  .display-name {
    font-size: 0.9em;
    color: var(--muted-foreground);
  }

  .av {
    display: flex;
    cursor: pointer;
    border-radius: 999px;
    transition: opacity 120ms var(--ease);
  }

  .av:hover {
    opacity: 0.85;
  }

  /* shadcn's outline badge, small. */
  .tag {
    font-size: 10.5px;
    line-height: 16px;
    font-weight: 500;
    color: var(--muted-foreground);
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  /* Replies and reactions */
  .reply-quote {
    display: flex;
    align-items: baseline;
    gap: 6px;
    max-width: 100%;
    margin: 2px 0 3px;
    padding: 1px 8px;
    border-left: 2px solid var(--input);
    border-radius: 2px;
    font-size: 0.86em;
    color: var(--muted-foreground);
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
  }

  .reply-quote:not(:disabled):hover .reply-snip {
    color: var(--foreground);
  }

  .reply-arrow {
    color: var(--muted-foreground);
  }

  .reply-nick {
    font-weight: 600;
    color: oklch(var(--nick-l) var(--nick-c) var(--h));
  }

  :global(.no-nick-colors) .reply-nick {
    color: var(--text-2);
  }

  .reply-snip {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .reactions {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 3px 0 2px;
  }

  /* shadcn's outline badge; "mine" takes the brand tint. */
  .chip {
    height: 24px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: color-mix(in oklab, var(--muted) 45%, transparent);
    font-size: 13px;
    transition: background-color 120ms var(--ease);
  }

  .chip span {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--muted-foreground);
    font-variant-numeric: tabular-nums;
  }

  .chip.mine {
    background: var(--brand-soft);
    border-color: color-mix(in oklab, var(--brand) 45%, transparent);
  }

  .chip.mine span {
    color: var(--brand);
  }

  .chip:not(:disabled):hover {
    background: var(--muted);
    border-color: var(--input);
  }
</style>
