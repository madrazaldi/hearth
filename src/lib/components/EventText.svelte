<script lang="ts" module>
  export const GLYPH: Record<string, string> = {
    join: '→',
    part: '←',
    quit: '←',
    nick: '↻',
    kick: '⨯',
    mode: '±',
    topic: '¶',
    invite: '✉',
    info: '·',
    error: '!',
  };
</script>

<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { stripFormatting } from '../format';
  import type { Line } from '../types';
  import RichText from './RichText.svelte';
  import Nick from './Nick.svelte';

  let { l, buf, onnick }: { l: Line; buf: Buffer; onnick: (nick: string, e: MouseEvent) => void } = $props();
</script>

{#if l.type === 'join'}
  <Nick nick={l.nick} {onnick} /> joined {#if l.data?.host && l.data.host !== '@'}<span class="faint">{l.data.host}</span>{/if}
{:else if l.type === 'part'}
  <Nick nick={l.nick} {onnick} /> left{#if l.text}<span class="faint"> ({stripFormatting(l.text)})</span>{/if}
{:else if l.type === 'quit'}
  <Nick nick={l.nick} {onnick} /> quit{#if l.text}<span class="faint"> ({stripFormatting(l.text)})</span>{/if}
{:else if l.type === 'nick'}
  <Nick nick={l.nick} {onnick} /> is now <Nick nick={l.target ?? ''} {onnick} />
{:else if l.type === 'kick'}
  <Nick nick={l.nick} {onnick} /> removed <Nick nick={l.target ?? ''} {onnick} />{#if l.text}<span class="faint"> ({stripFormatting(l.text)})</span>{/if}
{:else if l.type === 'mode'}
  {#if l.nick}<Nick nick={l.nick} {onnick} /> set mode{:else}Mode{/if} <span class="mono">{l.text}</span>
{:else if l.type === 'topic'}
  <Nick nick={l.nick} {onnick} /> changed the topic to <span class="quote"><RichText text={l.text} /></span>
{:else if l.type === 'invite'}
  <Nick nick={l.nick} {onnick} /> {l.text}
  {#if l.target}<button class="inline-action" onclick={() => app.join(buf.conn, l.target!)}>Join</button>{/if}
{:else}
  <RichText text={l.text} />
{/if}

<style>
  .quote {
    color: var(--text-2);
  }

  .inline-action {
    margin-left: 8px;
    font-size: 12px;
    color: var(--brand);
  }

  .inline-action:hover {
    text-decoration: underline;
  }
</style>
