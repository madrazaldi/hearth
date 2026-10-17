<script lang="ts">
  import { app } from '../state.svelte';
  import ShortcutList from './ShortcutList.svelte';
  import * as Dialog from '$lib/components/ui/dialog';

  const close = () => (app.shortcutsOpen = false);
</script>

<Dialog.Root open onOpenChange={(o) => !o && close()}>
  <Dialog.Content class="flex max-h-[86vh] w-[min(520px,92vw)] flex-col gap-0 p-0 sm:max-w-lg">
    <Dialog.Header class="gap-1 border-b border-border px-6 py-4">
      <Dialog.Title class="text-lg font-semibold tracking-tight">Keyboard shortcuts</Dialog.Title>
      <Dialog.Description>Available anywhere in Hearth.</Dialog.Description>
    </Dialog.Header>
    <div class="min-h-0 flex-1 overflow-y-auto px-6 py-5">
      <ShortcutList />
    </div>
  </Dialog.Content>
</Dialog.Root>
