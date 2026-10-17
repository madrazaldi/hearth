<script lang="ts">
  import { app, type Conn, type ChannelInfo } from '../state.svelte';
  import { stripFormatting } from '../format';
  import Segmented from './Segmented.svelte';
  import * as Dialog from '$lib/components/ui/dialog';
  import * as InputGroup from '$lib/components/ui/input-group';
  import * as Empty from '$lib/components/ui/empty';
  import { Badge } from '$lib/components/ui/badge';
  import { Button } from '$lib/components/ui/button';
  import { Spinner } from '$lib/components/ui/spinner';
  import SearchIcon from '@lucide/svelte/icons/search';
  import RefreshCwIcon from '@lucide/svelte/icons/refresh-cw';
  import UsersIcon from '@lucide/svelte/icons/users';
  import ListXIcon from '@lucide/svelte/icons/list-x';

  let { conn }: { conn: Conn } = $props();

  const SHOWN = 300;
  let q = $state('');
  let sort = $state<'users' | 'name'>('users');
  let open = $state(true);
  let input = $state<HTMLInputElement | null>(null);

  const list = $derived(conn.channelList);

  const filtered = $derived.by(() => {
    const items = list?.items ?? [];
    const needle = q.trim().toLowerCase();
    const match = needle
      ? items.filter((c) => c.name.toLowerCase().includes(needle) || stripFormatting(c.topic).toLowerCase().includes(needle))
      : items;
    const sorted = [...match];
    if (sort === 'users') sorted.sort((a, b) => b.users - a.users || a.name.localeCompare(b.name));
    else sorted.sort((a, b) => conn.fold(a.name).localeCompare(conn.fold(b.name)));
    return sorted;
  });

  function close() {
    app.browserConn = null;
  }

  function join(c: ChannelInfo) {
    const existing = conn.buffer(c.name);
    if (existing?.joined) app.open(existing);
    else app.join(conn, c.name);
    close();
  }

  function refresh() {
    conn.channelList = null;
    app.browseChannels(conn);
  }
</script>

<Dialog.Root bind:open onOpenChange={(o) => !o && close()}>
  <!-- The refresh button comes first in tab order, but filtering is what people open this for. -->
  <Dialog.Content
    class="flex h-[min(640px,88vh)] flex-col gap-0 overflow-hidden p-0 sm:max-w-3xl"
    onOpenAutoFocus={(e) => {
      e.preventDefault();
      input?.focus();
    }}
  >
    <Dialog.Header class="gap-1 p-4 pr-20 pb-3">
      <Dialog.Title class="text-lg font-semibold tracking-tight">Channels on {conn.name}</Dialog.Title>
      <Dialog.Description class="flex items-center gap-1.5 text-[12.5px] text-muted-foreground" aria-live="polite">
        {#if list?.loading}
          <Spinner class="size-3" aria-label="Loading channels" />
          Loading… {list.items.length ? `${list.items.length.toLocaleString()} so far` : ''}
        {:else if list}
          {list.items.length.toLocaleString()} channels{q ? ` · ${filtered.length.toLocaleString()} match` : ''}
        {:else}
          Not connected
        {/if}
      </Dialog.Description>
    </Dialog.Header>
    <Button class="absolute top-2 right-10" variant="ghost" size="icon-sm" title="Refresh" aria-label="Refresh" onclick={refresh} disabled={list?.loading}>
      <RefreshCwIcon />
    </Button>
    <div class="flex gap-2.5 border-b px-4 pb-3">
      <InputGroup.Root class="h-9 flex-1">
        <InputGroup.Addon><SearchIcon /></InputGroup.Addon>
        <InputGroup.Input bind:ref={input} bind:value={q} placeholder="Filter by name or topic" aria-label="Filter channels" spellcheck="false" data-autofocus />
      </InputGroup.Root>
      <Segmented label="Sort by"
        bind:value={sort}
        options={[
          { value: 'users', label: 'Most users' },
          { value: 'name', label: 'Name' },
        ]}
      />
    </div>
    <div class="min-h-0 flex-1 overflow-y-auto p-2">
      {#each filtered.slice(0, SHOWN) as c (c.name)}
        {@const joined = conn.buffer(c.name)?.joined}
        <div class="flex items-center gap-3.5 rounded-lg px-2.5 py-2 hover:bg-accent">
          <div class="min-w-0 flex-1">
            <div class="text-[13.5px] font-semibold">{c.name}</div>
            {#if c.topic}<div class="truncate text-[12.5px] text-muted-foreground">{stripFormatting(c.topic)}</div>{/if}
          </div>
          <Badge variant="secondary" class="tabular-nums text-muted-foreground" title="Users">
            <UsersIcon />{c.users.toLocaleString()}<span class="sr-only"> users</span>
          </Badge>
          <Button
            variant={joined ? 'outline' : 'default'}
            size="sm"
            class="w-14"
            onclick={() => join(c)}
            aria-label="{joined ? 'Open' : 'Join'} {c.name}"
          >{joined ? 'Open' : 'Join'}</Button>
        </div>
      {:else}
        {#if !list?.loading}
          <Empty.Root class="border-0">
            <Empty.Header>
              <Empty.Media variant="icon"><ListXIcon /></Empty.Media>
              <Empty.Title>{q ? 'No channels match' : 'No channels listed'}</Empty.Title>
            </Empty.Header>
          </Empty.Root>
        {/if}
      {/each}
      {#if filtered.length > SHOWN}
        <p class="m-0 p-3 text-[13px] text-muted-foreground">Showing {SHOWN} of {filtered.length.toLocaleString()}. Narrow the filter to see more.</p>
      {/if}
    </div>
  </Dialog.Content>
</Dialog.Root>
