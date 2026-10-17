<script lang="ts">
  import { app } from '../state.svelte';
  import { formatClock, formatDuration, formatFull } from '../time';
  import type { Line } from '../types';
  import RichText from './RichText.svelte';
  import Nick from './Nick.svelte';
  import { Card, CardContent } from '$lib/components/ui/card';

  let { l, onnick }: { l: Line; onnick: (nick: string, e: MouseEvent) => void } = $props();
  const w = $derived(l.data!);
</script>

<div class="row block">
  <time title={formatFull(l.time)}>{formatClock(l.time, app.settings)}</time>
  <span class="who"></span>
  <Card size="sm" class="selectable max-w-xl py-2.5 text-[13px]">
    <CardContent>
      <div class="card-head mb-2 flex items-baseline gap-2.5">
        <Nick nick={w.nick} {onnick} />
        <span class="faint mono">{w.user}@{w.host}</span>
      </div>
      <dl class="m-0 grid grid-cols-[auto_1fr] gap-x-4 gap-y-[3px]">
        {#snippet row(label: string)}<dt class="text-muted-foreground">{label}</dt>{/snippet}
        {#if w.realname}{@render row('Name')}<dd><RichText text={w.realname} /></dd>{/if}
        {#if w.account}{@render row('Account')}<dd>{w.account}</dd>{/if}
        {#if w.server}{@render row('Server')}<dd>{w.server}</dd>{/if}
        {#if w.channels}{@render row('Channels')}<dd class="mono text-xs">{w.channels}</dd>{/if}
        {#if w.idle}{@render row('Idle')}<dd>{formatDuration(parseInt(w.idle))}{#if w.signon}<span class="faint"> · online since {formatFull(parseInt(w.signon) * 1000)}</span>{/if}</dd>{/if}
        {#if w.away}{@render row('Away')}<dd>{w.away}</dd>{/if}
        {#if w.oper}{@render row('Operator')}<dd>{w.oper}</dd>{/if}
        {#if w.secure}{@render row('Security')}<dd>{w.secure}</dd>{/if}
        {#if w.actual}{@render row('Actual')}<dd class="mono text-xs">{w.actual}</dd>{/if}
      </dl>
    </CardContent>
  </Card>
</div>

<style>
  dd {
    margin: 0;
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
</style>
