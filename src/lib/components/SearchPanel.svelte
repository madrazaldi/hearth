<script module lang="ts">
  import type { Buffer as Buf } from '../state.svelte';
  import type { SearchHit as Hit } from '../types';

  /** The last search, kept so reopening after a jump picks up where it left off. */
  let last: { q: string; origin: Buf | null; scope: 'all' | 'network' | 'buffer'; hits: Hit[]; next: number | null; sel: number } | null =
    null;
</script>

<script lang="ts">
  import { tick } from 'svelte';
  import { app, type Buffer, type Conn } from '../state.svelte';
  import { parseCtcp } from '../irc';
  import { stripFormatting } from '../format';
  import { nickHue } from '../nickcolor';
  import { dedupeHits, hitTime, parseQuery } from '../search';
  import type { SearchHit } from '../types';
  import Segmented from './Segmented.svelte';
  import * as Dialog from '$lib/components/ui/dialog';
  import * as InputGroup from '$lib/components/ui/input-group';
  import * as Item from '$lib/components/ui/item';
  import * as Empty from '$lib/components/ui/empty';
  import { Badge } from '$lib/components/ui/badge';
  import { Button } from '$lib/components/ui/button';
  import { Kbd } from '$lib/components/ui/kbd';
  import { Spinner } from '$lib/components/ui/spinner';
  import SearchIcon from '@lucide/svelte/icons/search';
  import SearchXIcon from '@lucide/svelte/icons/search-x';

  const resumed = last && app.searchQuery && last.q === app.searchQuery ? last : null;
  const origin = resumed ? resumed.origin : app.active;
  let q = $state(app.searchQuery);
  let scope = $state<'all' | 'network' | 'buffer'>(resumed?.scope ?? 'all');
  let hits = $state<SearchHit[]>(resumed?.hits ?? []);
  let next = $state<number | null>(resumed?.next ?? null);
  let busy = $state(false);
  let more = $state(false);
  let sel = $state(resumed?.sel ?? 0);
  let sugSel = $state(0);
  let list = $state<HTMLDivElement>();
  let input = $state<HTMLInputElement | null>(null);
  let open = $state(true);
  let run = 0;
  let skip = !!resumed;

  const remoteAvailable = $derived([...app.conns.values()].some((c) => c.has('soju.im/search')));
  const canSearch = $derived(app.settings.storeLogs || remoteAvailable);

  function scoped(sc: typeof scope): { conn?: Conn; buf?: Buffer } {
    return sc === 'buffer' && origin ? { conn: origin.conn, buf: origin } : sc === 'network' && origin ? { conn: origin.conn } : {};
  }

  $effect(() => {
    const query = q;
    const sc = scope;
    const id = ++run;
    if (skip) {
      skip = false;
      return;
    }
    more = false;
    if (!query.trim()) {
      hits = [];
      next = null;
      busy = false;
      return;
    }
    busy = true;
    const t = setTimeout(async () => {
      const page = await app.search(query, scoped(sc));
      if (id !== run) return;
      hits = page.hits;
      next = page.next;
      sel = 0;
      busy = false;
      list?.scrollTo({ top: 0 });
    }, 180);
    return () => clearTimeout(t);
  });

  /** Fetch the next page of older hits. */
  async function loadMore() {
    if (more || busy || next === null) return;
    const id = run;
    more = true;
    const page = await app.search(q, scoped(scope), next);
    if (id !== run) return;
    hits = dedupeHits([...hits, ...page.hits]);
    next = page.next;
    more = false;
  }

  function onListScroll() {
    if (list && list.scrollTop + list.clientHeight > list.scrollHeight - 240) loadMore();
  }

  $effect(() => {
    void sel;
    const el = list?.querySelector('[aria-selected="true"]');
    el?.scrollIntoView({ block: 'nearest' });
    if (sel >= hits.length - 5) loadMore();
  });

  $effect(() => {
    void sugSel;
    list?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
  });

  // Resuming: show the query selected, so typing starts a new one.
  $effect(() => {
    if (resumed) input?.select();
  });

  function close() {
    app.searchOpen = false;
    app.searchQuery = q;
  }

  // ── from: and in: suggestions ─────────────────────────────────
  const token = $derived(/(?:^|\s)(from|in):(\S*)$/i.exec(q));

  const suggestions = $derived.by((): string[] => {
    if (!token) return [];
    const kind = token[1].toLowerCase();
    const part = token[2].toLowerCase();
    const conns = scope !== 'all' && origin ? [origin.conn] : [...app.conns.values()];
    const names = new Map<string, string>();
    const add = (n: string) => n && !names.has(n.toLowerCase()) && names.set(n.toLowerCase(), n);
    if (kind === 'in') {
      if (origin && origin.kind !== 'server') add(origin.name);
      for (const c of conns) for (const b of c.buffers.values()) add(b.name);
    } else {
      // People who spoke here recently first, then everyone else in view.
      if (origin) for (let i = origin.messages.length - 1, n = 0; i >= 0 && n < 400; i--, n++) add(origin.messages[i].nick);
      if (origin) for (const m of origin.sortedMembers) add(m.nick);
      for (const c of conns) for (const b of c.buffers.values()) if (b.kind === 'query') add(b.name);
    }
    const all = [...names.values()].filter((n) => n.toLowerCase().startsWith(part));
    return all.length === 1 && all[0].toLowerCase() === part ? [] : all.slice(0, 8);
  });

  $effect(() => {
    void suggestions;
    sugSel = 0;
  });

  function complete(name: string) {
    if (!token) return;
    q = q.slice(0, q.length - token[2].length) + name + ' ';
    input?.focus();
  }

  function addFilter(prefix: string) {
    q = (q.trim() ? q.trim() + ' ' : '') + prefix;
    tick().then(() => {
      input?.focus();
      input?.setSelectionRange(q.length, q.length);
    });
  }

  // ── Results ───────────────────────────────────────────────────
  function where(h: SearchHit): { conn: Conn | undefined; name: string } {
    const conn = app.conns.get(h.network);
    const to = h.msg.params[0] ?? h.target;
    const nick = h.msg.source?.nick ?? '';
    if (!conn) return { conn, name: to };
    const name = conn.isChannel(to) || !nick || conn.isMe(nick) ? to : nick;
    return { conn, name: conn.fold(name) === h.target ? name : (conn.buffer(h.target)?.name ?? h.target) };
  }

  function text(h: SearchHit): string {
    const raw = h.msg.params[1] ?? '';
    const ctcp = parseCtcp(raw);
    return stripFormatting(ctcp ? ctcp.args : raw);
  }

  function choose(h: SearchHit | undefined) {
    if (!h) return;
    const { conn, name } = where(h);
    if (!conn) return app.toast('That network is not connected', 'error');
    const buf = conn.buffer(name) ?? conn.ensure(name);
    last = { q, origin, scope, hits, next, sel: Math.max(0, hits.indexOf(h)) };
    close();
    app.jumpTo(buf, hitTime(h), h.msg.tags.msgid);
  }

  /** Split text into plain and matched parts for highlighting. */
  function marked(s: string): { t: string; m: boolean }[] {
    const words = parseQuery(q)
      .text.split(/\s+/)
      .map((w) => w.replace(/[^\p{L}\p{N}#@]/gu, ''))
      .filter(Boolean)
      .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
    if (!words.length) return [{ t: s, m: false }];
    const re = new RegExp(`(${words.join('|')})`, 'giu');
    return s.split(re).map((t, i) => ({ t, m: i % 2 === 1 }));
  }

  function onKey(e: KeyboardEvent) {
    if (suggestions.length) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        sugSel = (sugSel + (e.key === 'ArrowDown' ? 1 : suggestions.length - 1)) % suggestions.length;
        return;
      }
      if (e.key === 'Enter' || e.key === 'Tab') {
        e.preventDefault();
        complete(suggestions[sugSel]);
        return;
      }
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      sel = Math.min(sel + 1, hits.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      sel = Math.max(sel - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      choose(hits[sel]);
    }
  }

  const stamp = (t: number) => new Date(t).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });

  const FILTERS = [
    { prefix: 'from:', hint: 'nick', label: 'Messages from someone' },
    { prefix: 'in:', hint: 'channel', label: 'In one channel or conversation' },
    { prefix: 'on:', hint: '2026-10-12', label: 'On a day; also today or yesterday' },
    { prefix: 'before:', hint: 'date', label: 'Before a day' },
    { prefix: 'after:', hint: 'date', label: 'After a day' },
  ];
</script>

<Dialog.Root bind:open onOpenChange={(o) => !o && close()}>
  <Dialog.Content showCloseButton={false} class="top-[12vh] translate-y-0 gap-0 overflow-hidden p-0 sm:max-w-2xl">
    <Dialog.Header class="sr-only">
      <Dialog.Title>Search messages</Dialog.Title>
      <Dialog.Description>Search saved messages, with from:, in: and date filters</Dialog.Description>
    </Dialog.Header>
    <div class="grid gap-2 border-b p-3">
      <InputGroup.Root class="h-9">
        <InputGroup.Addon><SearchIcon /></InputGroup.Addon>
        <InputGroup.Input
          bind:ref={input}
          bind:value={q}
          onkeydown={onKey}
          placeholder="Search messages"
          spellcheck="false"
          role="combobox"
          aria-label="Search messages"
          aria-expanded={hits.length > 0 || suggestions.length > 0}
          aria-controls="search-results"
          aria-autocomplete="list"
          aria-activedescendant={suggestions.length ? `search-sug-${sugSel}` : hits.length ? `search-hit-${sel}` : undefined}
          data-autofocus
        />
        {#if busy}
          <InputGroup.Addon align="inline-end"><Spinner aria-label="Searching" /></InputGroup.Addon>
        {/if}
      </InputGroup.Root>
      {#if origin}
        <Segmented label="Search in"
          bind:value={scope}
          options={[
            { value: 'all', label: 'Everywhere' },
            { value: 'network', label: origin.conn.name },
            ...(origin.kind !== 'server' ? [{ value: 'buffer' as const, label: origin.name }] : []),
          ]}
        />
      {/if}
    </div>
    <div
      class="max-h-[56vh] min-h-48 overflow-y-auto p-1.5"
      bind:this={list}
      onscroll={onListScroll}
      id="search-results"
      role={hits.length || suggestions.length ? 'listbox' : undefined}
      aria-label={suggestions.length ? 'Suggestions' : hits.length ? 'Results' : undefined}
    >
      {#if !canSearch}
        <Empty.Root class="border-0">
          <Empty.Header>
            <Empty.Media variant="icon"><SearchXIcon /></Empty.Media>
            <Empty.Title>Search is off</Empty.Title>
            <Empty.Description>Search uses messages saved on this computer, which is turned off.</Empty.Description>
          </Empty.Header>
          <Empty.Content>
            <Button variant="outline" size="sm" onclick={() => (app.settingsOpen = { tab: 'privacy' })}>Change in Settings</Button>
          </Empty.Content>
        </Empty.Root>
      {:else if suggestions.length}
        {#each suggestions as name, i (name)}
          {@const from = token?.[1].toLowerCase() === 'from'}
          <Item.Root size="xs" class="text-left aria-selected:bg-accent" onmouseenter={() => (sugSel = i)}>
            {#snippet child({ props })}
              <button
                {...props}
                id="search-sug-{i}"
                role="option"
                aria-selected={i === sugSel}
                tabindex="-1"
                onclick={() => complete(name)}
              >
                <span class="text-muted-foreground">{token?.[1].toLowerCase()}:</span><b class={['font-semibold', from && 'nick']} style={from ? `--h:${nickHue(name)}` : ''}>{name}</b>
              </button>
            {/snippet}
          </Item.Root>
        {/each}
      {:else if !q.trim()}
        <p class="eyebrow px-2.5 pt-1.5 pb-1">Narrow it down</p>
        {#each FILTERS as f (f.prefix)}
          <Item.Root size="xs" class="text-left hover:bg-accent focus-visible:bg-accent">
            {#snippet child({ props })}
              <button {...props} tabindex="-1" onclick={() => addFilter(f.prefix)}>
                <span class="mono min-w-36 text-[12.5px]">{f.prefix}<span class="text-muted-foreground">{f.hint}</span></span>
                <span class="text-muted-foreground">{f.label}</span>
              </button>
            {/snippet}
          </Item.Root>
        {/each}
      {:else}
        {#each hits as h, i (h.network + '|' + h.target + '|' + (h.msg.tags.msgid ?? h.msg.tags.time) + '|' + i)}
          {@const w = where(h)}
          {@const nick = h.msg.source?.nick ?? ''}
          <Item.Root size="xs" class="flex-col items-stretch gap-0.5 py-1.5 text-left aria-selected:bg-accent disabled:opacity-50" onmouseenter={() => (sel = i)}>
            {#snippet child({ props })}
              <button
                {...props}
                id="search-hit-{i}"
                role="option"
                aria-selected={i === sel}
                tabindex="-1"
                disabled={!w.conn}
                onclick={() => choose(h)}
              >
                <div class="flex items-center gap-2 text-xs text-muted-foreground">
                  <span class="font-semibold text-foreground/80">{w.name}</span>
                  {#if w.conn}<Badge variant="secondary" class="h-4 px-1.5 text-[11px] font-normal text-muted-foreground">{w.conn.name}</Badge>{/if}
                  <span class="ml-auto tabular-nums">{stamp(hitTime(h))}</span>
                </div>
                <div class="flex min-w-0 items-baseline gap-2 text-[13.5px]">
                  <b class="nick flex-none font-semibold" style="--h:{nickHue(nick)}">{nick}</b>
                  <span class="line-clamp-3 text-foreground/80 [overflow-wrap:anywhere] whitespace-pre-wrap">{#each marked(text(h)) as part, j (j)}{#if part.m}<mark class="rounded-[2px] bg-brand-soft text-foreground">{part.t}</mark>{:else}{part.t}{/if}{/each}</span>
                </div>
              </button>
            {/snippet}
          </Item.Root>
        {:else}
          {#if !busy}
            <Empty.Root class="border-0">
              <Empty.Header>
                <Empty.Media variant="icon"><SearchXIcon /></Empty.Media>
                <Empty.Title>No messages found</Empty.Title>
                <Empty.Description>Try different words or fewer filters.</Empty.Description>
              </Empty.Header>
            </Empty.Root>
          {/if}
        {/each}
        {#if more}
          <p class="flex items-center justify-center gap-2 p-3 text-[13px] text-muted-foreground"><Spinner class="size-3.5" />Loading older messages…</p>
        {/if}
      {/if}
    </div>
    <footer class="flex items-center gap-4 border-t px-3 py-2 text-xs text-muted-foreground">
      <span class="flex items-center gap-1.5"><Kbd>↑↓</Kbd> navigate</span>
      <span class="flex items-center gap-1.5"><Kbd>↵</Kbd> {suggestions.length ? 'complete' : 'jump to message'}</span>
      <span class="flex items-center gap-1.5"><Kbd>esc</Kbd> close</span>
      <span class="ml-auto" aria-live="polite">
        {busy ? 'Searching…' : hits.length ? `${hits.length}${next !== null ? '+' : ''} result${hits.length === 1 ? '' : 's'}` : ''}
      </span>
    </footer>
  </Dialog.Content>
</Dialog.Root>
