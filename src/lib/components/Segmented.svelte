<script lang="ts" generics="T extends string | number">
  let { value = $bindable(), options }: { value: T; options: { value: T; label: string }[] } = $props();
</script>

<div class="seg" role="radiogroup">
  {#each options as o (o.value)}
    <button role="radio" aria-checked={value === o.value} class:on={value === o.value} onclick={() => (value = o.value)}>
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
