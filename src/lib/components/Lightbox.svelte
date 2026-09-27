<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { app } from '../state.svelte';
  import { lightbox } from '../lightbox.svelte';
  import { fileName } from '../media';
  import { modal } from '../a11y';
  import Icon from './Icon.svelte';

  const img = $derived(lightbox.current);

  function close() {
    lightbox.current = null;
  }

  function onkey(e: KeyboardEvent) {
    if (!img || e.key !== 'Escape') return;
    // Ours alone: Escape would otherwise also close the search or composer state.
    e.preventDefault();
    e.stopPropagation();
    close();
  }

  function open() {
    if (img) openUrl(img.url).catch((err) => app.toast(String(err), 'error'));
  }
</script>

<svelte:window onkeydowncapture={onkey} />

{#if img}
  <div class="lightbox" role="dialog" aria-modal="true" aria-label="Image" tabindex="-1" onclick={close} onkeydown={() => {}} use:modal={{ onclose: close }}>
    <div class="bar" role="toolbar" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={() => {}}>
      <span class="name" title={img.url}>{fileName(img.url)}{#if img.width && img.height}<span class="dims">{img.width}×{img.height}</span>{/if}</span>
      <button class="action" onclick={open}><Icon name="external" size={14} />Open in browser</button>
      <button class="close" data-autofocus onclick={close} title="Close (Esc)"><Icon name="x" size={16} /></button>
    </div>
    <img src={img.src} alt={fileName(img.url)} width={img.width ?? undefined} height={img.height ?? undefined} />
  </div>
{/if}

<style>
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    padding: 56px 32px 32px;
    background: color-mix(in oklab, black 78%, transparent);
    animation: fade 140ms var(--ease);
  }

  img {
    max-width: 100%;
    max-height: 100%;
    width: auto;
    height: auto;
    object-fit: contain;
    min-height: 0;
    border-radius: 6px;
    box-shadow: 0 12px 48px rgb(0 0 0 / 0.5);
    background: var(--bg-sunken);
  }

  .bar {
    position: absolute;
    top: 12px;
    left: 16px;
    right: 16px;
    display: flex;
    align-items: center;
    gap: 8px;
    color: #eee;
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }

  .dims {
    margin-left: 10px;
    font-family: var(--font-mono);
    font-size: 11.5px;
    opacity: 0.7;
  }

  .action,
  .close {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px;
    border-radius: 6px;
    color: #eee;
    background: rgb(255 255 255 / 0.1);
    font-size: 12.5px;
  }

  .close {
    width: 30px;
    padding: 0;
    justify-content: center;
  }

  .action:hover,
  .close:hover {
    background: rgb(255 255 255 / 0.18);
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .lightbox {
      animation: none;
    }
  }
</style>
