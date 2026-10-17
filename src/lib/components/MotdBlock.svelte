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
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--card);
    color: var(--card-foreground);
    padding: 10px 14px;
    font-size: 13px;
  }

  .motd summary {
    color: var(--muted-foreground);
    font-size: 12px;
    font-weight: 500;
    cursor: default;
    list-style: none;
  }

  .motd summary::before {
    content: '▸ ';
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
