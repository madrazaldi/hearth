<script lang="ts">
  import { untrack } from 'svelte';
  import { app } from '../state.svelte';
  import { formatClock, formatFull } from '../time';
  import type { Line } from '../types';
  import RichText from './RichText.svelte';

  /** `open` is only the initial state; the user can toggle it. */
  let { l, open }: { l: Line; open: boolean } = $props();
  const initialOpen = untrack(() => open);
</script>

<div class="row block">
  <time title={formatFull(l.time)}>{formatClock(l.time, app.settings)}</time>
  <span class="who"></span>
  <details class="motd" open={initialOpen}>
    <summary>{l.data!.title ?? 'Message of the day'}</summary>
    <pre class="selectable"><RichText text={l.data!.motd} /></pre>
  </details>
</div>

<style>
  .motd {
    max-width: 640px;
    border-radius: 8px;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
    padding: 10px 14px;
    font-size: 13px;
  }

  .motd summary {
    color: var(--text-3);
    font-size: 12px;
    font-weight: 500;
    cursor: default;
    list-style: none;
  }

  .motd summary::before {
    content: '▸ ';
    color: var(--text-4);
  }

  .motd[open] summary::before {
    content: '▾ ';
  }

  .motd pre {
    margin: 8px 0 2px;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-2);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
