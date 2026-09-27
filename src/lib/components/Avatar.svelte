<script lang="ts">
  import { app, type Conn } from '../state.svelte';
  import { nickHue } from '../nickcolor';

  let { conn, nick, size = 18 }: { conn: Conn; nick: string; size?: number } = $props();

  const url = $derived(conn.profile(nick).avatar);
  let src = $state<string | null>(null);

  // The backend fetches the image (CSP forbids remote images) and only while
  // the privacy setting allows it.
  $effect(() => {
    const u = url;
    src = null;
    if (!u || !app.settings.loadAvatars) return;
    let live = true;
    app.avatar(u).then((s) => {
      if (live) src = s;
    });
    return () => (live = false);
  });
</script>

<span class="avatar" style="--size:{size}px; --h:{nickHue(nick)}" aria-hidden="true">
  {#if src}
    <img {src} alt="" draggable="false" />
  {:else}
    {[...nick.replace(/^[^\p{L}\p{N}]+/u, '')][0]?.toUpperCase() ?? '?'}
  {/if}
</span>

<style>
  .avatar {
    width: var(--size);
    height: var(--size);
    flex: none;
    border-radius: 50%;
    overflow: hidden;
    display: inline-grid;
    place-items: center;
    font-size: calc(var(--size) * 0.5);
    font-weight: 600;
    line-height: 1;
    /* Same hue as the nick, in either theme. */
    color: oklch(var(--nick-l) var(--nick-c) var(--h));
    background: color-mix(in oklch, oklch(var(--nick-l) var(--nick-c) var(--h)) 20%, var(--bg-raised));
    user-select: none;
  }

  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
</style>
