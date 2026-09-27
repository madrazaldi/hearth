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

<div class="seg" role="radiogroup" aria-label={label} tabindex="-1" onkeydown={onKey}>
  {#each options as o, i (o.value)}
    <button
      role="radio"
      aria-checked={value === o.value}
      tabindex={i === (at < 0 ? 0 : at) ? 0 : -1}
      class:on={value === o.value}
      onclick={() => (value = o.value)}
    >
      {o.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    justify-self: start;
    width: max-content;
    padding: 2px;
    gap: 2px;
    border-radius: 8px;
    background: var(--bg-sunken);
    box-shadow: 0 0 0 1px var(--line) inset;
  }

  button {
    height: 26px;
    padding: 0 12px;
    border-radius: 6px;
    font-size: 12.5px;
    color: var(--text-3);
    transition: color 120ms, background 120ms;
  }

  button:hover {
    color: var(--text);
  }

  button.on {
    background: var(--bg-raised);
    color: var(--text);
    box-shadow: 0 0 0 1px var(--line-strong) inset, 0 1px 2px rgb(0 0 0 / 0.15);
  }
</style>
