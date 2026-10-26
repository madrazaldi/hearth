<script lang="ts" generics="T extends string | number">
  import type { Snippet } from 'svelte';
  import { cn } from '$lib/utils';

  let {
    value = $bindable(),
    options,
    label,
    preview,
  }: { value: T; options: { value: T; label: string }[]; label?: string; preview: Snippet<[T]> } = $props();

  const at = $derived(options.findIndex((o) => o.value === value));

  // Radio-group keys, as in Segmented: arrows move the choice and focus.
  function onKey(e: KeyboardEvent) {
    const d = e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 0;
    if (!d) return;
    e.preventDefault();
    const i = (Math.max(at, 0) + d + options.length) % options.length;
    value = options[i].value;
    ((e.currentTarget as HTMLElement).children[i] as HTMLElement | undefined)?.focus();
  }
</script>

<div role="radiogroup" aria-label={label} tabindex="-1" onkeydown={onKey} class="grid grid-cols-3 gap-3">
  {#each options as o, i (o.value)}
    {@const on = value === o.value}
    <button
      type="button"
      role="radio"
      aria-checked={on}
      tabindex={i === (at < 0 ? 0 : at) ? 0 : -1}
      onclick={() => (value = o.value)}
      class="group grid gap-2 text-left outline-none"
    >
      <span
        class={cn(
          'block overflow-hidden rounded-lg border p-1 ring-offset-2 ring-offset-background transition-[border-color,box-shadow] group-focus-visible:ring-2 group-focus-visible:ring-ring',
          on ? 'border-brand shadow-[0_0_0_1px_var(--brand)]' : 'border-border group-hover:border-input',
        )}
      >
        {@render preview(o.value)}
      </span>
      <span class={cn('flex items-center gap-1.5 px-0.5 text-[13px]', on ? 'font-medium text-foreground' : 'text-muted-foreground')}>
        <span
          class={cn('grid size-3.5 place-items-center rounded-full border', on ? 'border-brand bg-brand' : 'border-input')}
          aria-hidden="true"
        >
          {#if on}<span class="size-1.5 rounded-full bg-brand-text"></span>{/if}
        </span>
        {o.label}
      </span>
    </button>
  {/each}
</div>
