<script lang="ts">
  let { label }: { label?: string } = $props();
</script>

{#if label === undefined}
  <div class="unread-divider"><span>New messages</span></div>
{:else}
  <div class="day"><span>{label}</span></div>
{/if}

<style>
  .day {
    display: flex;
    align-items: center;
    gap: 12px;
    margin: 20px 20px 6px 16px;
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

  /* shadcn's outline badge, sitting on the rule. */
  .day span {
    padding: 1px 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--background);
    line-height: 18px;
  }

  .unread-divider {
    position: relative;
    height: 1px;
    margin: 14px 20px 10px 16px;
    background: color-mix(in oklab, var(--brand) 65%, transparent);
  }

  /* A small filled badge on the line, in the ember colour. */
  .unread-divider span {
    position: absolute;
    right: 0;
    top: -9px;
    padding: 0 8px;
    border-radius: 999px;
    background: var(--brand);
    color: var(--brand-text);
    font-size: 10.5px;
    font-weight: 600;
    line-height: 18px;
  }
</style>
