<script lang="ts">
  import { app } from '../state.svelte';
  import { matchingRule, ruleError } from '../format';
  import { isIgnored } from '../ignore';
  import { casefold } from '../irc';

  let { serverId, nick }: { serverId: string; nick: string } = $props();

  // svelte-ignore state_referenced_locally
  const initial = app.networkHighlights(serverId);
  let words = $state(initial.words.join('\n'));
  let never = $state(initial.never.join(', '));
  let sample = $state('');
  let sampleFrom = $state('');

  const lines = $derived(
    words
      .split('\n')
      .map((w) => w.trim())
      .filter(Boolean),
  );
  const errors = $derived(lines.flatMap((w) => (ruleError(w) ? [`${w} — ${ruleError(w)}`] : [])));
  const valid = $derived(lines.filter((w) => !ruleError(w)));
  const neverList = $derived(
    never
      .split(/[\s,]+/)
      .map((n) => n.trim())
      .filter(Boolean),
  );
  const conn = $derived([...app.conns.values()].find((c) => c.serverId === serverId && !c.isControl));
  const myNick = $derived(conn?.nick || nick);

  const result = $derived.by(() => {
    if (!sample.trim()) return null;
    const from = sampleFrom.trim();
    const fold = (s: string) => (conn ? conn.fold(s) : casefold(s));
    if (from && isIgnored(neverList, { nick: from }, fold)) return { hit: false, why: `${from} never highlights` };
    const rule = matchingRule(sample, [myNick, ...app.settings.highlightWords, ...valid].filter(Boolean));
    if (!rule) return { hit: false, why: 'No rule matches' };
    return { hit: true, why: rule === myNick ? 'Matches your nickname' : `Matches ${rule}` };
  });

  function save() {
    const next = { ...app.settings.networkHighlights };
    if (!valid.length && !neverList.length) delete next[serverId];
    else next[serverId] = { words: valid, never: neverList };
    app.updateSettings({ networkHighlights: next });
  }
</script>

<div class="highlights">
  <h3>Highlights on this server</h3>
  <p class="lede">Added to your global highlight words. For a bouncer, these apply to all of its networks.</p>
  <label class="field">
    <span>Extra highlight words</span>
    <textarea class="input mono" rows="3" bind:value={words} onchange={save} placeholder={'deploy\n/\\bci (failed|broke)\\b/i'} spellcheck="false"></textarea>
    {#if errors.length}
      {#each errors as e (e)}<small class="bad">Invalid pattern: {e}</small>{/each}
    {:else}
      <small>One per line. Words match whole-word, ignoring case. Wrap a regular expression in slashes: <code>/pattern/i</code>.</small>
    {/if}
  </label>
  <label class="field">
    <span>Never highlight from</span>
    <input class="input mono" bind:value={never} onchange={save} placeholder="ChanServ, *bot*, *!*@services.*" spellcheck="false" autocapitalize="off" />
    <small>Nicks or <code>nick!user@host</code> masks, such as bots that repeat your name.</small>
  </label>
  <div class="field">
    <span>Test a message</span>
    <div class="test">
      <input class="input mono from" bind:value={sampleFrom} placeholder="from" spellcheck="false" autocapitalize="off" aria-label="Sender" />
      <input class="input" bind:value={sample} placeholder="Type a sample line" aria-label="Sample message" />
    </div>
    {#if result}
      <small class:hit={result.hit}>{result.hit ? 'Highlights' : 'Doesn’t highlight'} · {result.why}</small>
    {/if}
  </div>
</div>

<style>
  .highlights {
    display: grid;
    gap: 14px;
    margin-top: 32px;
    padding-top: 24px;
    border-top: 1px solid var(--line);
  }

  h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }

  .lede {
    margin: -10px 0 0;
    color: var(--text-3);
    font-size: 12.5px;
  }

  .test {
    display: flex;
    gap: 8px;
  }

  .test .input {
    flex: 1;
    min-width: 0;
  }

  .test .from {
    flex: 0 0 120px;
  }

  .bad {
    color: var(--danger) !important;
  }

  .hit {
    color: var(--accent) !important;
  }

  code {
    font-family: var(--font-mono);
    font-size: 11.5px;
  }
</style>
