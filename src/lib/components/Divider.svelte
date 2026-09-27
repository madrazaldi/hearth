<script lang="ts">
  let { label }: { label?: string } = $props();
</script>

{#if label === undefined}
  <div class="unread-divider"><span>New</span></div>
{:else}
  <div class="day"><span>{label}</span></div>
{/if}

<style>
  .day {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 18px 20px 8px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-3);
  }

  .day::before,
  .day::after {
    content: '';
    flex: 1;
    height: 1px;
    background: var(--line);
  }

  .unread-divider {
    position: relative;
    height: 1px;
    margin: 10px 20px 8px 14px;
    background: var(--accent);
    opacity: 0.8;
  }

  .unread-divider span {
    position: absolute;
    right: 0;
    top: -8px;
    padding: 0 6px;
    background: var(--bg);
    color: var(--accent);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    line-height: 16px;
  }
</style>
