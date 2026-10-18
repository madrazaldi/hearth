<script lang="ts">
  import { Dialog as DialogPrimitive } from 'bits-ui';
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { app } from '../state.svelte';
  import { lightbox } from '../lightbox.svelte';
  import { fileName } from '../media';
  import { focusHome } from '../a11y';
  import { Button } from '$lib/components/ui/button';
  import * as Dialog from '$lib/components/ui/dialog';
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

<Dialog.Root open={!!img} onOpenChange={(o) => !o && close()}>
  {#if img}
    <Dialog.Portal>
      <Dialog.Overlay class="bg-black/80 supports-backdrop-filter:backdrop-blur-sm" />
      <!-- The stock Dialog.Content is sized for forms; this one wraps the image. -->
      <DialogPrimitive.Content
        aria-describedby={undefined}
        onCloseAutoFocus={focusHome}
        class="bg-popover text-popover-foreground ring-foreground/10 data-open:animate-in data-closed:animate-out data-closed:fade-out-0 data-open:fade-in-0 data-closed:zoom-out-95 data-open:zoom-in-95 fixed top-1/2 left-1/2 z-50 flex max-h-[calc(100vh-4rem)] max-w-[calc(100vw-4rem)] min-w-[min(360px,calc(100vw-4rem))] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl shadow-lg ring-1 duration-100 outline-none"
      >
        <header class="flex h-12 flex-none items-center gap-2 border-b pr-2 pl-4">
          <Dialog.Title class="min-w-0 flex-1 truncate text-sm font-medium" title={img.url}>
            {fileName(img.url)}
            {#if img.width && img.height}<span class="text-muted-foreground ml-2 font-mono text-xs font-normal">{img.width}×{img.height}</span>{/if}
          </Dialog.Title>
          <Button variant="outline" size="sm" onclick={open}><Icon name="external" size={14} />Open in browser</Button>
          <Dialog.Close>
            {#snippet child({ props })}
              <Button {...props} variant="ghost" size="icon-sm" title="Close (Esc)" data-autofocus>
                <Icon name="x" size={16} />
                <span class="sr-only">Close</span>
              </Button>
            {/snippet}
          </Dialog.Close>
        </header>
        <div class="bg-muted/30 flex min-h-0 items-center justify-center p-3">
          <img
            class="bg-muted block h-auto max-h-[calc(100vh-10rem)] min-h-0 w-auto max-w-full rounded-md object-contain"
            src={img.src}
            alt={fileName(img.url)}
            width={img.width ?? undefined}
            height={img.height ?? undefined}
          />
        </div>
      </DialogPrimitive.Content>
    </Dialog.Portal>
  {/if}
</Dialog.Root>
