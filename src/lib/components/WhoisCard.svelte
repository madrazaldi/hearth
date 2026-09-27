<script lang="ts">
  import { app } from '../state.svelte';
  import { formatClock, formatDuration, formatFull } from '../time';
  import type { Line } from '../types';
  import RichText from './RichText.svelte';
  import Nick from './Nick.svelte';

  let { l, onnick }: { l: Line; onnick: (nick: string, e: MouseEvent) => void } = $props();
  const w = $derived(l.data!);
</script>

<div class="row block">
  <time title={formatFull(l.time)}>{formatClock(l.time, app.settings)}</time>
  <span class="who"></span>
  <div class="card selectable">
    <div class="card-head">
      <Nick nick={w.nick} {onnick} />
      <span class="faint mono">{w.user}@{w.host}</span>
    </div>
    <dl>
      {#if w.realname}<dt>Name</dt><dd><RichText text={w.realname} /></dd>{/if}
      {#if w.account}<dt>Account</dt><dd>{w.account}</dd>{/if}
      {#if w.server}<dt>Server</dt><dd>{w.server}</dd>{/if}
      {#if w.channels}<dt>Channels</dt><dd class="mono small">{w.channels}</dd>{/if}
      {#if w.idle}<dt>Idle</dt><dd>{formatDuration(parseInt(w.idle))}{#if w.signon}<span class="faint"> · online since {formatFull(parseInt(w.signon) * 1000)}</span>{/if}</dd>{/if}
      {#if w.away}<dt>Away</dt><dd>{w.away}</dd>{/if}
      {#if w.oper}<dt>Operator</dt><dd>{w.oper}</dd>{/if}
      {#if w.secure}<dt>Security</dt><dd>{w.secure}</dd>{/if}
      {#if w.actual}<dt>Actual</dt><dd class="mono small">{w.actual}</dd>{/if}
    </dl>
  </div>
</div>

<style>
  .card {
    max-width: 640px;
    border-radius: 8px;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
    padding: 10px 14px;
    font-size: 13px;
  }

  .card-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-bottom: 8px;
  }

  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 3px 16px;
    margin: 0;
  }

  dt {
    color: var(--text-3);
  }

  dd {
    margin: 0;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }

  .small {
    font-size: 12px;
  }
</style>
