<script lang="ts">
  import { app } from '../state.svelte';
  import { aliasError, aliasName, BODY_ERROR } from '../aliases';
  import { isBuiltin } from '../commands';
  import Icon from './Icon.svelte';

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

<div class="aliases">
  {#if entries.length}
    <div class="list">
      {#each entries as [n, b] (n)}
        <div class="alias">
          <button class="main" title="Edit" aria-label="Edit alias /{n}: {b}" onclick={() => edit(n)}>
            <span class="mono name">/{n}</span>
            <span class="mono body">{b}</span>
          </button>
          {#if isBuiltin(n)}<span class="warn" title="Replaces the built-in /{n}">replaces built-in</span>{/if}
          <button class="icon-btn" title="Delete alias" aria-label="Delete alias /{n}" onclick={() => remove(n)}><Icon name="x" size={13} /></button>
        </div>
      {/each}
    </div>
  {/if}

  <form onsubmit={save}>
    <input class="input mono name-in" bind:value={name} placeholder="/ns" spellcheck="false" autocapitalize="off" aria-label="Alias name" aria-invalid={nameBad} aria-describedby={nameBad ? 'alias-error' : undefined} />
    <input class="input mono" bind:value={body} placeholder="/msg NickServ $*" spellcheck="false" autocapitalize="off" aria-label="Commands" aria-invalid={bodyBad} aria-describedby={bodyBad ? 'alias-error' : undefined} />
    <button class="btn" type="submit" disabled={!name.trim() || !body.trim()}>{exists ? 'Update' : 'Add'}</button>
  </form>
  {#if tried && error}
    <small class="bad" id="alias-error">{error}</small>
  {:else if key && isBuiltin(key)}
    <small class="warn-text">/{key} is a built-in command. This alias will replace it; inside the alias, /{key} still runs the built-in.</small>
  {/if}
  <small>
    <code>$1</code>…<code>$9</code> are arguments, <code>$2-</code> is the second onward, <code>$*</code> is all of them.
    <code>$nick</code>, <code>$channel</code> and <code>$network</code> fill in the obvious. Separate commands with <code>;</code>
    (<code>\;</code> for a literal one). Lines without a leading <code>/</code> are sent as messages.
  </small>
</div>

<style>
  .aliases {
    display: grid;
    gap: 8px;
    max-width: 560px;
  }

  .aliases > small {
    font-size: 12px;
    color: var(--text-3);
    line-height: 1.5;
  }

  .list {
    display: grid;
    margin-bottom: 8px;
    border-radius: 8px;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
    overflow: hidden;
  }

  .alias {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 3px 4px 3px 0;
    font-size: 12.5px;
  }

  .alias + .alias {
    border-top: 1px solid var(--line);
  }

  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 12px;
    padding: 4px 12px;
    text-align: left;
  }

  .name {
    flex: none;
    min-width: 64px;
    color: var(--text);
  }

  .body {
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .warn {
    flex: none;
    font-size: 11px;
    color: var(--warn);
  }

  .warn-text {
    color: var(--warn) !important;
  }

  .bad {
    color: var(--danger) !important;
  }

  form {
    display: flex;
    gap: 8px;
  }

  form .input {
    flex: 1;
    min-width: 0;
  }

  form .name-in {
    flex: 0 0 110px;
  }

  code {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }
</style>
