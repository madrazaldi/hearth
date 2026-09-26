<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { app } from '../state.svelte';
  import type { Preview } from '../types';

  let { url }: { url: string } = $props();

  let el = $state<HTMLElement>();
  let preview = $state<Preview | null>(null);

  onMount(() => {
    if (!el) return;
    // Only fetch once the message scrolls into view.
    const io = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        io.disconnect();
        app.preview(url).then(async (p) => {
          preview = p;
          await tick();
          grew();
        });
      },
      { rootMargin: '200px' },
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

<div class="slot" bind:this={el}>
  {#if preview?.kind === 'image' && preview.image}
    <a href={url} class="image" onclick={open} title={url}><img src={preview.image} alt="" onload={grew} /></a>
  {:else if preview?.kind === 'page'}
    <a href={url} class="card" class:with-image={!!preview.image} onclick={open} title={url}>
      <div class="meta">
        {#if preview.site}<div class="site">{preview.site}</div>{/if}
        {#if preview.title}<div class="title">{preview.title}</div>{/if}
        {#if preview.description}<div class="desc">{preview.description}</div>{/if}
      </div>
      {#if preview.image}<img src={preview.image} alt="" onload={grew} />{/if}
    </a>
  {/if}
</div>

<style>
  .image,
  .card {
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

  .image img {
    display: block;
    max-width: 100%;
    max-height: 280px;
    object-fit: contain;
    background: var(--bg-sunken);
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
</style>
