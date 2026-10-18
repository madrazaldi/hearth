<script lang="ts">
  import { Switch } from '$lib/components/ui/switch';
  import { Label } from '$lib/components/ui/label';

  let { checked = $bindable(false), label, hint }: { checked?: boolean; label: string; hint?: string } = $props();
  const id = $props.id();
</script>

<div class="flex items-center gap-4 py-1.5">
  <div class="grid flex-1 gap-0.5">
    <Label for={id} class="text-[13.5px] font-normal">{label}</Label>
    {#if hint}<span id="{id}-hint" class="text-xs text-muted-foreground">{hint}</span>{/if}
  </div>
  <Switch {id} bind:checked aria-describedby={hint ? `${id}-hint` : undefined} />
</div>
