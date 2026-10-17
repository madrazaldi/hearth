<script lang="ts">
  import { SHORTCUT_GROUPS } from '../shortcuts';
  import { Kbd } from '$lib/components/ui/kbd';
</script>

<div class="grid gap-6">
  {#each SHORTCUT_GROUPS as g (g.title)}
    <section class="grid gap-1">
      <h3 class="eyebrow mb-1 font-medium">{g.title}</h3>
      <dl class="m-0 divide-y divide-border text-[13.5px]">
        {#each g.items as s (s.label)}
          <div class="flex items-center justify-between gap-6 py-2">
            <dt class="text-foreground/90">{s.label}</dt>
            <dd class="m-0 flex shrink-0 items-center gap-1.5">
              {#each s.keys as combo, i (i)}
                {#if i}<span class="text-xs text-muted-foreground/70">or</span>{/if}
                <span class="flex items-center gap-1">
                  {#each combo as key, j (j)}<Kbd>{key}</Kbd>{/each}
                </span>
              {/each}
            </dd>
          </div>
        {/each}
      </dl>
    </section>
  {/each}
</div>
