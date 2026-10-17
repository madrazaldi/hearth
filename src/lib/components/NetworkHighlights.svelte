<script lang="ts">
  import { app } from '../state.svelte';
  import { matchingRule, ruleError } from '../format';
  import { isIgnored } from '../ignore';
  import { casefold } from '../irc';
  import { Input } from '$lib/components/ui/input';
  import { Textarea } from '$lib/components/ui/textarea';
  import { Label } from '$lib/components/ui/label';
  import { cn } from '$lib/utils';

  const uid = $props.id();

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
    // Keep what's saved until the edit is valid, rather than dropping the bad rule.
    if (errors.length) return;
    const next = { ...app.settings.networkHighlights };
    if (!valid.length && !neverList.length) delete next[serverId];
    else next[serverId] = { words: valid, never: neverList };
    app.updateSettings({ networkHighlights: next });
  }
</script>

<div class="mt-8 grid gap-3.5 border-t pt-6">
  <div class="grid gap-1">
    <h3 class="text-[15px] font-semibold">Highlights on this server</h3>
    <p class="m-0 text-[12.5px] text-muted-foreground">Added to your global highlight words. For a bouncer, these apply to all of its networks.</p>
  </div>
  <div class="grid gap-1.5">
    <Label for="{uid}-words">Extra highlight words</Label>
    <Textarea
      id="{uid}-words"
      class="mono min-h-0"
      rows={3}
      bind:value={words}
      onchange={save}
      placeholder={'deploy\n/\\bci (failed|broke)\\b/i'}
      spellcheck="false"
      aria-invalid={errors.length > 0}
    />
    {#if errors.length}
      {#each errors as e (e)}<small class="text-xs text-destructive">Invalid pattern: {e}</small>{/each}
    {:else}
      <small class="text-xs text-muted-foreground">
        One per line. Words match whole-word, ignoring case. Wrap a regular expression in slashes: {@render code('/pattern/i')}.
      </small>
    {/if}
  </div>
  <div class="grid gap-1.5">
    <Label for="{uid}-never">Never highlight from</Label>
    <Input
      id="{uid}-never"
      class="mono"
      bind:value={never}
      onchange={save}
      placeholder="ChanServ, *bot*, *!*@services.*"
      spellcheck="false"
      autocapitalize="off"
    />
    <small class="text-xs text-muted-foreground">Nicks or {@render code('nick!user@host')} masks, such as bots that repeat your name.</small>
  </div>
  <div class="grid gap-1.5">
    <Label for="{uid}-sample">Test a message</Label>
    <div class="flex gap-2">
      <Input class="mono w-30 min-w-0 flex-none" bind:value={sampleFrom} placeholder="from" spellcheck="false" autocapitalize="off" aria-label="Sender" />
      <Input id="{uid}-sample" class="min-w-0 flex-1" bind:value={sample} placeholder="Type a sample line" />
    </div>
    {#if result}
      <small class={cn('text-xs text-muted-foreground', result.hit && 'text-brand')}>
        {result.hit ? 'Highlights' : 'Doesn’t highlight'} · {result.why}
      </small>
    {/if}
  </div>
</div>

{#snippet code(text: string)}<code class="rounded bg-muted px-1 py-0.5 font-mono text-[11.5px] text-foreground">{text}</code>{/snippet}
