<script lang="ts">
  let { checked = $bindable(false), label, hint }: { checked?: boolean; label: string; hint?: string } = $props();
</script>

<label class="switch">
  <span class="text">
    <span class="label">{label}</span>
    {#if hint}<span class="hint">{hint}</span>{/if}
  </span>
  <input type="checkbox" bind:checked />
  <span class="track" aria-hidden="true"><span class="thumb"></span></span>
</label>

<style>
  .switch {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 6px 0;
  }

  .text {
    flex: 1;
    display: grid;
    gap: 1px;
  }

  .label {
    font-size: 13.5px;
  }

  .hint {
    font-size: 12px;
    color: var(--text-3);
  }

  input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .track {
    width: 30px;
    height: 18px;
    flex: none;
    border-radius: 9px;
    background: var(--bg-active);
    box-shadow: 0 0 0 1px var(--line-strong) inset;
    position: relative;
    transition: background 150ms;
  }

  .thumb {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--text-2);
    transition: transform 180ms var(--ease), background 150ms;
  }

  input:checked + .track {
    background: var(--accent);
    box-shadow: none;
  }

  input:checked + .track .thumb {
    transform: translateX(12px);
    background: var(--accent-text);
  }

  input:focus-visible + .track {
    outline: 2px solid color-mix(in oklab, var(--accent) 70%, transparent);
    outline-offset: 2px;
  }
</style>
