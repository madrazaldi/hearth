<script lang="ts" generics="T extends string | number">
  let { value = $bindable(), options, label }: { value: T; options: { value: T; label: string }[]; label?: string } = $props();

  const at = $derived(options.findIndex((o) => o.value === value));

  // Radio-group keys: arrows move the choice, and focus goes with it.
  function onKey(e: KeyboardEvent) {
    const d = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    const i = (Math.max(at, 0) + d + options.length) % options.length;
    value = options[i].value;
    ((e.currentTarget as HTMLElement).children[i] as HTMLElement | undefined)?.focus();
  }
</script>

<div
  role="radiogroup"
  aria-label={label}
  tabindex="-1"
  onkeydown={onKey}
  class="inline-flex w-max gap-px justify-self-start rounded-lg bg-muted p-[3px]"
>
  {#each options as o, i (o.value)}
    <button
      type="button"
      role="radio"
      aria-checked={value === o.value}
      tabindex={i === (at < 0 ? 0 : at) ? 0 : -1}
      data-state={value === o.value ? 'on' : 'off'}
      onclick={() => (value = o.value)}
      class="h-7 rounded-md px-3 text-[13px] text-muted-foreground outline-none transition-[color,background-color,box-shadow] hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50 data-[state=on]:bg-background data-[state=on]:text-foreground data-[state=on]:shadow-sm dark:data-[state=on]:bg-input/50"
    >
      {o.label}
    </button>
  {/each}
</div>
