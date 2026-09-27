<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { formatClock, formatFull } from '../time';
  import type { Line } from '../types';
  import EventText, { GLYPH } from './EventText.svelte';

  let { l, buf, flash, onnick }: { l: Line; buf: Buffer; flash: boolean; onnick: (nick: string, e: MouseEvent) => void } = $props();
</script>

<div class="row event {l.type}" class:hl={l.highlight} class:flash data-line={l.id}>
  <time title={formatFull(l.time)}>{formatClock(l.time, app.settings)}</time>
  <span class="who"><span class="glyph">{GLYPH[l.type] ?? '·'}</span></span>
  <div class="text selectable"><EventText {l} {buf} {onnick} /></div>
</div>
