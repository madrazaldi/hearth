<script lang="ts" module>
  import type { Preview } from '../types';
  import { ByteLru } from '../media';

  // Settled previews, so a row the message list re-mounts while scrolling
  // renders at its full height straight away instead of growing a frame later.
  // Bounded by bytes too: an inline image is kept as a data URL of up to ~11 MB.
  const settled = new ByteLru<Preview | null>(500, 48 * 1024 * 1024);
</script>

<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { app } from '../state.svelte';
  import { fileName, mediaLabel } from '../media';
  import { buttonVariants } from '$lib/components/ui/button';
  import Icon from './Icon.svelte';
  import InlineImage from './InlineImage.svelte';

  // One string for every preview: these are plain elements, not Button instances.
  const openBtn = buttonVariants({ variant: 'outline', size: 'sm' });

  let { url, nsfw = false }: { url: string; nsfw?: boolean } = $props();

  let el = $state<HTMLElement>();
  // The url never changes for a mounted preview (rows are keyed by line).
  // svelte-ignore state_referenced_locally
  let preview = $state<Preview | null>(settled.get(url) ?? null);

  onMount(() => {
    if (!el || settled.has(url)) return;
    // Only fetch once the message nears the view; early enough that it has
    // usually grown (and been anchored for) before it scrolls in.
    const io = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        io.disconnect();
        app.preview(url).then(async (p) => {
          settled.set(url, p, p?.image?.length ?? 0);
          preview = p;
          await tick();
          grew();
        });
      },
      { root: el.closest('.scroller'), rootMargin: '800px 0px' },
    );
    io.observe(el);
    return () => io.disconnect();
  });

  /** Tell the message list this row got taller. */
  function grew() {
    el?.dispatchEvent(new Event('contentgrow', { bubbles: true }));
  }

  function open(e: MouseEvent) {
    e.preventDefault();
    openUrl(url).catch((err) => app.toast(String(err), 'error'));
  }
</script>

{#snippet file(p: Preview)}
  <div class="file">
    <span class="file-icon"><Icon name={p.kind === 'image' ? 'image' : p.mime?.startsWith('audio/') ? 'music' : 'play'} size={16} /></span>
    <span class="file-meta">
      <span class="file-name" title={url}>{fileName(url)}</span>
      <span class="file-kind">{mediaLabel(p)}</span>
    </span>
    <a href={url} class={openBtn} onclick={open} title="Open in browser"><Icon name="external" size={14} />Open</a>
  </div>
{/snippet}

<div class="slot" bind:this={el}>
  {#if preview?.kind === 'image' && preview.image}
    {#if app.settings.inlineImages}
      <InlineImage src={preview.image} {url} width={preview.width} height={preview.height} animated={preview.animated} {nsfw} />
    {:else}
      {@render file(preview)}
    {/if}
  {:else if preview?.kind === 'media'}
    {@render file(preview)}
  {:else if preview?.kind === 'page'}
    <a href={url} class="card" class:with-image={!!preview.image} onclick={open} title={url}>
      <div class="meta">
        {#if preview.site}<div class="site">{preview.site}</div>{/if}
        {#if preview.title}<div class="title">{preview.title}</div>{/if}
        {#if preview.description}<div class="desc">{preview.description}</div>{/if}
      </div>
      {#if preview.image}<img src={preview.image} alt="" class:veiled={nsfw} onload={grew} />{/if}
    </a>
  {/if}
</div>

<style>
  /* shadcn's Card: rounded-lg, 1px border, card surface. */
  .card,
  .file {
    display: inline-flex;
    margin: 6px 0 4px;
    max-width: min(520px, 100%);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--card);
    color: var(--card-foreground);
    overflow: hidden;
    text-decoration: none;
    cursor: pointer;
  }

  .file {
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 10px;
    min-width: 260px;
    cursor: default;
  }

  .file-icon {
    width: 32px;
    height: 32px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: calc(var(--radius) - 2px);
    color: var(--muted-foreground);
    background: var(--muted);
  }

  .file-meta {
    flex: 1;
    min-width: 0;
    display: grid;
  }

  .file-name {
    font-size: 13px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-kind {
    font-size: 11.5px;
    color: var(--muted-foreground);
    font-variant-numeric: tabular-nums;
  }

  .card {
    gap: 12px;
    padding: 12px 14px;
    align-items: flex-start;
    transition: background-color 120ms var(--ease);
  }

  .card:hover {
    background: color-mix(in oklab, var(--card) 80%, var(--muted));
  }

  .card:hover .title {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .meta {
    min-width: 0;
    flex: 1;
    display: grid;
    gap: 2px;
  }

  .site {
    font-size: 11.5px;
    color: var(--muted-foreground);
  }

  .title {
    font-weight: 600;
    font-size: 13.5px;
    color: var(--foreground);
  }

  .desc {
    font-size: 12.5px;
    color: var(--muted-foreground);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .card img {
    width: 88px;
    height: 88px;
    object-fit: cover;
    border-radius: calc(var(--radius) - 2px);
    flex: none;
  }

  .card img.veiled {
    filter: blur(12px);
  }
</style>
