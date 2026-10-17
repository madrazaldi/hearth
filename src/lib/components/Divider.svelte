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
    font-size: 11.5px;
    font-weight: 500;
    color: var(--muted-foreground);
  }

  .day::before,
  .day::after {
    content: '';
    flex: 1;
    height: 1px;
    background: var(--border);
  }

  .unread-divider {
    position: relative;
    height: 1px;
    margin: 12px 20px 10px 14px;
    background: color-mix(in oklab, var(--brand) 70%, transparent);
  }

  /* A small filled badge on the line, in the ember colour. */
  .unread-divider span {
    position: absolute;
    right: 0;
    top: -8px;
    padding: 0 7px;
    border-radius: 8px;
    background: var(--brand);
    color: var(--brand-text);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    line-height: 16px;
  }
</style>
