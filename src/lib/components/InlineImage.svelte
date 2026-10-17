<script lang="ts">
  import { app } from '../state.svelte';
  import { lightbox } from '../lightbox.svelte';
  import { fitBox } from '../media';

  let {
    src,
    url,
    width,
    height,
    animated = false,
    nsfw = false,
  }: { src: string; url: string; width: number | null; height: number | null; animated?: boolean; nsfw?: boolean } = $props();

  // The backend measured the image, so the box is sized before it decodes and
  // the row doesn't grow a second time.
  const box = $derived(fitBox(width ?? 0, height ?? 0));

  const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
  let reduce = $state(reducedMotion.matches);
  $effect(() => {
    const on = () => (reduce = reducedMotion.matches);
    reducedMotion.addEventListener('change', on);
    return () => reducedMotion.removeEventListener('change', on);
  });

  // svelte-ignore state_referenced_locally
  let hidden = $state(nsfw);
  let hover = $state(false);
  const autoplay = $derived(app.settings.autoplayAnimations && !reduce);
  const still = $derived(animated && !hidden && !(autoplay || hover));

  // A still frame for animations that shouldn't play: the webview can't
  // pause a GIF, so draw its first frame onto a canvas.
  let canvas = $state<HTMLCanvasElement>();
  $effect(() => {
    if (!animated || !canvas) return;
    const c = canvas;
    const img = new Image();
    img.onload = () => {
      const k = window.devicePixelRatio || 1;
      c.width = Math.round(box.width * k);
      c.height = Math.round(box.height * k);
      c.getContext('2d')?.drawImage(img, 0, 0, c.width, c.height);
    };
    img.src = src;
    return () => (img.onload = null);
  });

  function onclick(e: MouseEvent) {
    e.preventDefault();
    if (hidden) hidden = false;
    else lightbox.current = { src, url, width, height };
  }
</script>

<button
  class="inline-image"
  class:veiled={hidden}
  style="width:{box.width}px;aspect-ratio:{box.width}/{box.height}"
  title={hidden ? 'Marked NSFW: click to show' : url}
  aria-label={hidden ? 'Show NSFW image' : 'Open image'}
  onpointerenter={() => (hover = true)}
  onpointerleave={() => (hover = false)}
  {onclick}
>
  {#if animated}
    <canvas bind:this={canvas} class:show={still || hidden}></canvas>
  {/if}
  {#if !animated || !(still || hidden)}
    <img {src} alt="" width={box.width} height={box.height} draggable="false" />
  {/if}
  {#if hidden}
    <span class="badge nsfw">NSFW · click to show</span>
  {:else if still}
    <span class="badge">GIF</span>
  {/if}
</button>

<style>
  .inline-image {
    position: relative;
    display: block;
    max-width: 100%;
    margin: 6px 0 4px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    overflow: hidden;
    background: var(--muted);
    cursor: zoom-in;
  }

  img,
  canvas {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  canvas:not(.show) {
    display: none;
  }

  .veiled {
    cursor: pointer;
  }

  .veiled img,
  .veiled canvas {
    filter: blur(28px) saturate(0.6);
    transform: scale(1.15);
  }

  .badge {
    position: absolute;
    left: 8px;
    bottom: 8px;
    padding: 1px 6px;
    border-radius: 6px;
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.04em;
    color: #fff;
    background: rgb(0 0 0 / 0.6);
    backdrop-filter: blur(4px);
    pointer-events: none;
  }

  .badge.nsfw {
    left: 50%;
    top: 50%;
    bottom: auto;
    transform: translate(-50%, -50%);
    padding: 4px 10px;
    font-size: 12px;
    letter-spacing: 0;
    white-space: nowrap;
  }

  .inline-image:focus-visible {
    outline: 2px solid var(--ring);
    outline-offset: 2px;
  }
</style>
