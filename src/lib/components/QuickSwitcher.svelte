<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { bufferLabel } from '../a11y';
  import * as Command from '$lib/components/ui/command';
  import { Badge } from '$lib/components/ui/badge';
  import { Kbd } from '$lib/components/ui/kbd';
  import HashIcon from '@lucide/svelte/icons/hash';
  import AtSignIcon from '@lucide/svelte/icons/at-sign';
  import ServerIcon from '@lucide/svelte/icons/server';
  import PlusIcon from '@lucide/svelte/icons/plus';

  let open = $state(true);
  let q = $state('');

  function score(name: string, query: string): number {
    if (!query) return 1;
    const n = name.toLowerCase();
    const s = query.toLowerCase();
    if (n === s) return 1000;
    if (n.replace(/^#+/, '').startsWith(s.replace(/^#+/, ''))) return 500 - n.length;
    const i = n.indexOf(s);
    if (i >= 0) return 300 - i;
    // Subsequence match, rewarding tight runs.
    let pos = 0;
    let gaps = 0;
    for (const ch of s) {
      const at = n.indexOf(ch, pos);
      if (at < 0) return 0;
      gaps += at - pos;
      pos = at + 1;
    }
    return 100 - gaps;
  }

  type Result = { buf: Buffer; score: number } | { join: string; score: number };

  const results = $derived.by(() => {
    const out: Result[] = [];
    for (const b of app.allBuffers) {
      const label = b.kind === 'server' ? b.conn.name : b.name;
      const sc = score(label, q.trim());
      if (sc > 0) out.push({ buf: b, score: sc + (b.highlights ? 50 : 0) + (b.unread ? 10 : 0) + (b === app.active ? -40 : 0) });
    }
    out.sort((a, b) => b.score - a.score);
    const t = q.trim();
    const conn = app.active?.conn;
    if (conn && t && conn.isChannel(t) && !conn.buffer(t)) out.push({ join: t, score: 0 });
    return out.slice(0, 40);
  });

  // The join action is always last, so it gets its own group without reordering anything.
  const buffers = $derived(results.filter((r): r is Extract<Result, { buf: Buffer }> => 'buf' in r));
  const joins = $derived(results.filter((r): r is Extract<Result, { join: string }> => 'join' in r));

  function label(b: Buffer): string {
    const name = b.kind === 'server' ? b.conn.name : b.name;
    const where = b.kind === 'server' ? (b.conn.isControl ? 'bouncer' : 'network') : b.conn.name;
    return `${bufferLabel({ name, kind: b.kind, unread: b.unread, highlights: b.highlights, muted: b.level === 'none' })}, ${where}`;
  }

  function close() {
    app.switcherOpen = false;
  }

  function choose(r: Result) {
    if ('buf' in r) app.open(r.buf);
    else if (app.active) app.join(app.active.conn, r.join);
    close();
  }
</script>

<!-- Ranking is ours, so bits-ui's own filtering and re-sorting stay off. -->
<Command.Dialog
  bind:open
  onOpenChange={(o) => !o && close()}
  shouldFilter={false}
  title="Jump to"
  description="Jump to a channel, conversation or network"
  class="top-[14vh] sm:max-w-xl"
>
  <Command.Input bind:value={q} placeholder="Jump to a channel, conversation or network" spellcheck="false" aria-label="Jump to" />
  <Command.List class="max-h-[50vh] p-1" aria-label="Matches">
    <Command.Empty class="py-8 text-muted-foreground">No matches</Command.Empty>
    {#if buffers.length}
      <Command.Group>
        {#each buffers as r (r.buf.id)}
          {@const b = r.buf}
          <Command.Item
            value={b.id}
            aria-label={label(b)}
            onSelect={() => choose(r)}
            class="h-9 data-selected:bg-accent"
          >
            {#if b.kind === 'channel'}<HashIcon class="text-muted-foreground" />
            {:else if b.kind === 'query'}<AtSignIcon class="text-muted-foreground" />
            {:else}<ServerIcon class="text-muted-foreground" />{/if}
            <span class={['min-w-0 truncate', b.unread > 0 ? 'font-medium text-foreground' : 'text-foreground/80']}>
              {b.kind === 'server' ? b.conn.name : b.kind === 'channel' ? b.name.replace(/^#/, '') : b.name}
            </span>
            {#if b.highlights}
              <Badge class="h-4.5 min-w-4.5 bg-brand px-1.5 text-[11px] text-(--brand-text)">{b.highlights}</Badge>
            {/if}
            <Command.Shortcut class="tracking-normal">
              {b.kind === 'server' ? (b.conn.isControl ? 'bouncer' : 'network') : b.conn.name}
            </Command.Shortcut>
          </Command.Item>
        {/each}
      </Command.Group>
    {/if}
    {#each joins as r (r.join)}
      <Command.Group heading="Join">
        <Command.Item value={'join ' + r.join} onSelect={() => choose(r)} class="h-9 data-selected:bg-accent">
          <PlusIcon class="text-muted-foreground" />
          <span class="min-w-0 truncate">Join {r.join}</span>
          <Command.Shortcut class="tracking-normal">{app.active?.conn.name}</Command.Shortcut>
        </Command.Item>
      </Command.Group>
    {/each}
  </Command.List>
  <footer class="flex items-center gap-4 border-t px-3 py-2 text-xs text-muted-foreground">
    <span class="flex items-center gap-1.5"><Kbd>↑↓</Kbd> navigate</span>
    <span class="flex items-center gap-1.5"><Kbd>↵</Kbd> open</span>
    <span class="flex items-center gap-1.5"><Kbd>esc</Kbd> close</span>
    <span class="ml-auto">Tip: type a #channel to join it</span>
  </footer>
</Command.Dialog>
