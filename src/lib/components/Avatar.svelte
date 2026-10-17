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

<!-- Plain elements styled like shadcn's Avatar: this renders per message and per
     member row, and bits-ui's Avatar would flash the fallback before each image. -->
<span
  class="avatar relative inline-grid shrink-0 select-none place-items-center overflow-hidden rounded-full bg-muted font-semibold leading-none after:absolute after:inset-0 after:rounded-full after:border after:border-border"
  style="width:{size}px; height:{size}px; font-size:{size * 0.5}px; --h:{nickHue(nick)}"
  aria-hidden="true"
>
  {#if src}
    <img {src} alt="" draggable="false" class="size-full object-cover" />
  {:else}
    <!-- Same hue as the nick, in either theme. -->
    <span
      class="grid size-full place-items-center"
      style="color: oklch(var(--nick-l) var(--nick-c) var(--h)); background: color-mix(in oklch, oklch(var(--nick-l) var(--nick-c) var(--h)) 18%, var(--muted))"
    >
      {[...nick.replace(/^[^\p{L}\p{N}]+/u, '')][0]?.toUpperCase() ?? '?'}
    </span>
  {/if}
</span>
