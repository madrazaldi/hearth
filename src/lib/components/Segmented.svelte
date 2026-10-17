<script lang="ts" generics="T extends string | number">
  import * as ToggleGroup from '$lib/components/ui/toggle-group';

  let { value = $bindable(), options, label }: { value: T; options: { value: T; label: string }[]; label?: string } = $props();

  // bits-ui values are strings; map them back so numeric options round-trip.
  function select(v: string) {
    const o = options.find((o) => String(o.value) === v);
    if (o) value = o.value;
  }
</script>

<ToggleGroup.Root
  type="single"
  value={String(value)}
  onValueChange={(v) => v && select(v)}
  aria-label={label}
  spacing={1}
  class="w-max justify-self-start rounded-lg bg-muted p-[3px]"
>
  {#each options as o (o.value)}
    <ToggleGroup.Item
      value={String(o.value)}
      class="h-7 rounded-md px-3 text-[13px] font-normal text-muted-foreground hover:bg-transparent data-[state=on]:bg-background data-[state=on]:text-foreground data-[state=on]:shadow-sm dark:data-[state=on]:bg-input/50"
    >
      {o.label}
    </ToggleGroup.Item>
  {/each}
</ToggleGroup.Root>
