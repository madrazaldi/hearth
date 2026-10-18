<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { formatClock, formatFull } from '../time';
  import type { Line } from '../types';
  import Icon from './Icon.svelte';
  import EventText, { GLYPH } from './EventText.svelte';

  let {
    lines,
    buf,
    open,
    ontoggle,
    onnick,
  }: { lines: Line[]; buf: Buffer; open: boolean; ontoggle: () => void; onnick: (nick: string, e: MouseEvent) => void } = $props();

  const clock = (t: number) => formatClock(t, app.settings);

  function summarize(lines: Line[]): string {
    const joined: string[] = [];
    const left: string[] = [];
    const renamed: string[] = [];
    for (const l of lines) {
      if (l.type === 'join' && !joined.includes(l.nick)) joined.push(l.nick);
      else if ((l.type === 'part' || l.type === 'quit') && !left.includes(l.nick)) left.push(l.nick);
      else if (l.type === 'nick') renamed.push(`${l.nick} → ${l.target}`);
    }
    const names = (a: string[]) => (a.length <= 3 ? a.join(', ') : `${a.slice(0, 2).join(', ')} and ${a.length - 2} others`);
    const parts: string[] = [];
    if (joined.length) parts.push(`${names(joined)} joined`);
    if (left.length) parts.push(`${names(left)} left`);
    if (renamed.length) parts.push(renamed.length <= 2 ? renamed.join(', ') : `${renamed.length} nick changes`);
    return parts.join(' · ');
  }
</script>

<div class="row event run" class:open>
  <time title={formatFull(lines[0].time)}>{clock(lines[0].time)}</time>
  <span class="who"><span class="glyph" aria-hidden="true">⇅</span></span>
  <div class="text">
    <button class="run-toggle" onclick={ontoggle} title={open ? 'Collapse' : 'Show all'} aria-expanded={open}>
      {summarize(lines)}
      <span class="chev" class:open><Icon name="chevron" size={11} /></span>
    </button>
    {#if open}
      <div class="run-lines">
        {#each lines as l (l.id)}
          <div class="run-line">
            <time>{clock(l.time)}</time>
            <span class="glyph" aria-hidden="true">{GLYPH[l.type]}</span>
            <span><EventText {l} {buf} {onnick} /></span>
          </div>
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .run-toggle {
    text-align: left;
    color: var(--muted-foreground);
    display: inline;
  }

  .run-toggle:hover {
    color: var(--foreground);
  }

  .chev {
    display: inline-flex;
    vertical-align: middle;
    margin-left: 4px;
    color: var(--muted-foreground);
    transition: transform 150ms var(--ease);
  }

  .chev.open {
    transform: rotate(90deg);
  }

  .run-lines {
    margin: 4px 0 2px;
    display: grid;
    gap: 1px;
  }

  .run-line {
    display: flex;
    gap: 8px;
    align-items: baseline;
  }
</style>
