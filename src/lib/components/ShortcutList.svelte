<script lang="ts">
  import { SHORTCUT_GROUPS } from '../shortcuts';
</script>

<div class="groups">
  {#each SHORTCUT_GROUPS as g (g.title)}
    <div class="group">
      <p class="eyebrow">{g.title}</p>
      <dl>
        {#each g.items as s (s.label)}
          <dt>
            {#each s.keys as combo, i (i)}
              {#if i}<span class="or">/</span>{/if}
              <span class="kbd">{combo.join(' ')}</span>
            {/each}
          </dt>
          <dd>{s.label}</dd>
        {/each}
      </dl>
    </div>
  {/each}
</div>

<style>
  .groups {
    display: grid;
    gap: 20px;
  }

  .group .eyebrow {
    margin: 0 0 8px;
  }

  dl {
    display: grid;
    grid-template-columns: 180px 1fr;
    gap: 9px 16px;
    margin: 0;
    font-size: 13.5px;
  }

  dt {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .or {
    color: var(--text-4);
    font-size: 12px;
  }

  dd {
    margin: 0;
    color: var(--text-2);
  }
</style>
