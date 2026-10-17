<script lang="ts">
  import { app } from '../state.svelte';
  import { aliasError, aliasName, BODY_ERROR } from '../aliases';
  import { isBuiltin } from '../commands';
  import Icon from './Icon.svelte';
  import { Button } from '$lib/components/ui/button';
  import { Input } from '$lib/components/ui/input';
  import { Badge } from '$lib/components/ui/badge';
  import * as Field from '$lib/components/ui/field';

  let name = $state('');
  let body = $state('');
  let tried = $state(false);

  const key = $derived(aliasName(name));
  const error = $derived(aliasError(key, body));
  // Which input the error is about: aliasError reports name problems before body ones.
  const nameBad = $derived(tried && !!error && error !== BODY_ERROR);
  const bodyBad = $derived(tried && error === BODY_ERROR);
  const exists = $derived(key in app.settings.aliases);
  const entries = $derived(Object.entries(app.settings.aliases).sort(([a], [b]) => a.localeCompare(b)));

  function save(e: SubmitEvent) {
    e.preventDefault();
    tried = true;
    if (error) return;
    app.updateSettings({ aliases: { ...app.settings.aliases, [key]: body.trim() } });
    name = body = '';
    tried = false;
  }

  function remove(n: string) {
    const next = { ...app.settings.aliases };
    delete next[n];
    app.updateSettings({ aliases: next });
  }

  function edit(n: string) {
    name = n;
    body = app.settings.aliases[n];
  }
</script>

{#snippet code(text: string)}<code class="font-mono text-[11px] whitespace-nowrap text-foreground/80">{text}</code>{/snippet}

<div class="grid gap-3">
  {#if entries.length}
    <ul class="m-0 mb-1 list-none divide-y divide-border overflow-hidden rounded-lg border border-border p-0">
      {#each entries as [n, b] (n)}
        <li class="flex items-center gap-2 py-1 pr-1.5 text-[12.5px] transition-colors hover:bg-accent/50">
          <button class="focus-inset flex min-w-0 flex-1 gap-3 px-3 py-1 text-left" title="Edit" aria-label="Edit alias /{n}: {b}" onclick={() => edit(n)}>
            <span class="mono min-w-16 flex-none text-foreground">/{n}</span>
            <span class="mono truncate text-muted-foreground">{b}</span>
          </button>
          {#if isBuiltin(n)}<Badge variant="outline" class="text-warn" title="Replaces the built-in /{n}">replaces built-in</Badge>{/if}
          <Button variant="ghost" size="icon-xs" title="Delete alias" aria-label="Delete alias /{n}" onclick={() => remove(n)}><Icon name="x" size={13} /></Button>
        </li>
      {/each}
    </ul>
  {/if}

  <form onsubmit={save} class="flex gap-2">
    <Input class="w-28 flex-none font-mono" bind:value={name} placeholder="/ns" spellcheck="false" autocapitalize="off" aria-label="Alias name" aria-invalid={nameBad} aria-describedby={nameBad ? 'alias-error' : undefined} />
    <Input class="min-w-0 flex-1 font-mono" bind:value={body} placeholder="/msg NickServ $*" spellcheck="false" autocapitalize="off" aria-label="Commands" aria-invalid={bodyBad} aria-describedby={bodyBad ? 'alias-error' : undefined} />
    <Button variant="outline" type="submit" disabled={!name.trim() || !body.trim()}>{exists ? 'Update' : 'Add'}</Button>
  </form>
  {#if tried && error}
    <Field.Error id="alias-error" class="text-xs">{error}</Field.Error>
  {:else if key && isBuiltin(key)}
    <p class="m-0 text-xs text-warn">/{key} is a built-in command. This alias will replace it; inside the alias, /{key} still runs the built-in.</p>
  {/if}
  <Field.Description class="text-xs leading-relaxed">
    {@render code('$1')}…{@render code('$9')} are arguments, {@render code('$2-')} is the second onward, {@render code('$*')} is all of them.
    {@render code('$nick')}, {@render code('$channel')} and {@render code('$network')} fill in the obvious. Separate commands with {@render code(';')}
    ({@render code('\\;')} for a literal one). Lines without a leading {@render code('/')} are sent as messages.
  </Field.Description>
</div>
