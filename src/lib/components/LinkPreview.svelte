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
  import Icon from './Icon.svelte';
  import InlineImage from './InlineImage.svelte';

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
    <a href={url} class="btn ghost" onclick={open} title="Open in browser"><Icon name="external" size={14} />Open</a>
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
  .card,
  .file {
    display: inline-flex;
    margin: 6px 0 4px;
    max-width: min(520px, 100%);
    border-radius: 8px;
    overflow: hidden;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
    text-decoration: none;
    color: inherit;
    cursor: pointer;
  }

  .file {
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 10px;
    min-width: 260px;
    background: var(--bg-side);
    cursor: default;
  }

  .file-icon {
    width: 32px;
    height: 32px;
    flex: none;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--accent);
    background: var(--accent-soft);
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
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }

  .file .btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    flex: none;
    text-decoration: none;
  }

  .card {
    gap: 12px;
    padding: 10px 12px;
    align-items: flex-start;
    border-left: 3px solid var(--accent);
    background: var(--bg-side);
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
    color: var(--text-3);
  }

  .title {
    font-weight: 600;
    font-size: 13.5px;
    color: var(--accent);
  }

  .desc {
    font-size: 12.5px;
    color: var(--text-2);
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
    border-radius: 6px;
    flex: none;
  }

  .card img.veiled {
    filter: blur(12px);
  }
</style>
