<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { app, type Buffer } from '../state.svelte';
  import { commandInfos, runInput } from '../commands';
  import { emojiQuery, searchEmoji, typedShortcode, type Emoji } from '../emoji';
  import { api } from '../api';
  import { stripFormatting } from '../format';
  import { nickHue } from '../nickcolor';
  import Icon from './Icon.svelte';
  import { cn } from '$lib/utils';
  import * as InputGroup from '$lib/components/ui/input-group';
  import { Button } from '$lib/components/ui/button';
  import { Spinner } from '$lib/components/ui/spinner';
  import * as Tooltip from '$lib/components/ui/tooltip';

  let { buf }: { buf: Buffer } = $props();

  // svelte-ignore state_referenced_locally
  let text = $state(buf.draft);
  let ta = $state<HTMLTextAreaElement | null>(null);
  let fileInput = $state<HTMLInputElement>();
  let dragging = $state(false);
  let uploading = $state(0);
  let histIndex = -1;
  let completion: { start: number; end: number; options: string[]; index: number; suffix: string } | null = null;
  let cmdIndex = $state(0);
  let caret = $state(0);
  let emojiIndex = $state(0);
  // The colon position whose suggestions Escape closed.
  let emojiDismissed = $state(-1);

  // Suggestion popups float above the box, like a shadcn popover.
  const suggestClass =
    'absolute inset-x-4 bottom-[calc(100%-8px)] z-10 grid rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10';
  const suggestItem = 'flex items-center gap-3.5 rounded-md px-2.5 py-1.5 text-left';

  const conn = $derived(buf.conn);
  const filehost = $derived(conn.isupport['soju.im/FILEHOST'] || conn.isupport['FILEHOST'] || '');
  const placeholder = $derived(
    buf.kind === 'server'
      ? `Type /help for commands`
      : buf.kind === 'channel' && !buf.joined
        ? `You're not in ${buf.name}`
        : `Message ${buf.name}`,
  );

  const commands = $derived(commandInfos(app.settings.aliases));
  const cmdMatches = $derived.by(() => {
    const m = /^\/([\w-]*)$/.exec(text);
    if (!m) return [];
    const q = m[1].toLowerCase();
    return commands.filter((c) => c.name.startsWith(q)).slice(0, 7);
  });

  const emoji = $derived.by(() => {
    if (cmdMatches.length) return null;
    const q = emojiQuery(text.slice(0, caret));
    if (!q || q.start === emojiDismissed) return null;
    const matches = searchEmoji(q.query);
    return matches.length ? { ...q, matches } : null;
  });
  const emojiKey = $derived(emoji ? `${emoji.start}:${emoji.query}` : '');

  $effect(() => {
    void cmdMatches;
    cmdIndex = 0;
  });

  $effect(() => {
    void emojiKey;
    emojiIndex = 0;
  });

  $effect(() => {
    buf.draft = text;
    void text;
    autosize();
  });

  function autosize() {
    if (!ta) return;
    ta.style.height = '0';
    ta.style.height = Math.min(ta.scrollHeight, window.innerHeight * 0.4) + 'px';
  }

  async function submit() {
    const value = text;
    if (!value.trim()) return;
    const replyTo = value.startsWith('/') && !value.startsWith('//') ? undefined : buf.replyTo?.msgid;
    if (await runInput(app, buf, value, replyTo)) {
      app.inputHistory = [value, ...app.inputHistory.filter((h) => h !== value)].slice(0, 100);
      if (replyTo) buf.replyTo = null;
      text = '';
      histIndex = -1;
      app.sendTyping(buf, 'done');
      await tick();
      autosize();
    }
  }

  function recentSpeakers(): string[] {
    const seen: string[] = [];
    const msgs = buf.messages;
    for (let i = msgs.length - 1; i >= 0 && seen.length < 50 && i > msgs.length - 400; i--) {
      const n = msgs[i].nick;
      if (n && !msgs[i].self && !seen.includes(n)) seen.push(n);
    }
    return seen;
  }

  /** Tab completion; false when there's nothing to complete, so Tab can move focus instead. */
  function complete(backwards: boolean): boolean {
    if (!ta) return false;
    if (completion && ta.selectionStart === completion.end) {
      const c = completion;
      c.index = (c.index + (backwards ? -1 : 1) + c.options.length) % c.options.length;
      apply(c);
      return true;
    }
    const caret = ta.selectionStart;
    const before = text.slice(0, caret);
    const start = before.search(/\S*$/);
    const word = before.slice(start);
    if (!word) return false;
    const lw = word.toLowerCase();
    let options: string[] = [];
    let suffix = ' ';
    if (start === 0 && word.startsWith('/')) {
      options = commands.filter((c) => c.name.startsWith(lw.slice(1))).map((c) => '/' + c.name);
    } else if (conn.isChannel(word)) {
      options = [...conn.buffers.values()]
        .filter((b) => b.kind === 'channel' && b.name.toLowerCase().startsWith(lw))
        .map((b) => b.name);
    } else {
      const recent = recentSpeakers();
      const all = [...buf.members.values()].map((m) => m.nick);
      if (buf.kind === 'query') all.push(buf.name);
      const pool = [...recent, ...all.sort((a, b) => a.localeCompare(b))];
      options = [...new Set(pool)].filter((n) => n.toLowerCase().startsWith(lw) && !conn.isMe(n));
      if (start === 0) suffix = ': ';
    }
    if (!options.length) return false;
    completion = { start, end: caret, options, index: backwards ? options.length - 1 : 0, suffix };
    apply(completion);
    return true;
  }

  function apply(c: NonNullable<typeof completion>) {
    const insert = c.options[c.index] + c.suffix;
    const after = text.slice(c.end).replace(/^(: | )/, '');
    text = text.slice(0, c.start) + insert + after;
    c.end = c.start + insert.length;
    tick().then(() => ta?.setSelectionRange(c.end, c.end));
  }

  function setCaret(pos: number) {
    caret = pos;
    tick().then(() => ta?.setSelectionRange(pos, pos));
  }

  function pickEmoji(em: Emoji) {
    if (!emoji) return;
    const insert = em.char + ' ';
    text = text.slice(0, emoji.start) + insert + text.slice(caret).replace(/^ /, '');
    setCaret(emoji.start + insert.length);
  }

  function onKeydown(e: KeyboardEvent) {
    if (emoji && !e.isComposing) {
      const n = emoji.matches.length;
      if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
        e.preventDefault();
        emojiIndex = (emojiIndex + (e.key === 'ArrowUp' ? -1 : 1) + n) % n;
        return;
      }
      // Enter still sends a lone `:D` or `:P`; Tab always picks.
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey && emoji.query.length > 1)) {
        e.preventDefault();
        pickEmoji(emoji.matches[emojiIndex]);
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        emojiDismissed = emoji.start;
        return;
      }
    }
    if (cmdMatches.length && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      cmdIndex = (cmdIndex + (e.key === 'ArrowUp' ? -1 : 1) + cmdMatches.length) % cmdMatches.length;
      return;
    }
    if (cmdMatches.length && (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey && cmdMatches[cmdIndex].name !== text.slice(1)))) {
      e.preventDefault();
      text = `/${cmdMatches[cmdIndex].name} `;
      return;
    }
    if (e.key === 'Tab') {
      if (complete(e.shiftKey)) e.preventDefault();
      return;
    }
    completion = null;
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      submit();
      return;
    }
    if (e.key === 'ArrowUp' && ta && !text.slice(0, ta.selectionStart).includes('\n')) {
      if (histIndex + 1 < app.inputHistory.length && (text === '' || histIndex >= 0)) {
        e.preventDefault();
        histIndex++;
        text = app.inputHistory[histIndex];
      }
      return;
    }
    if (e.key === 'ArrowDown' && histIndex >= 0 && ta && !text.slice(ta.selectionEnd).includes('\n')) {
      e.preventDefault();
      histIndex--;
      text = histIndex >= 0 ? app.inputHistory[histIndex] : '';
      return;
    }
    if (e.key === 'Escape') {
      if (buf.replyTo) buf.replyTo = null;
      else ta?.blur();
    }
  }

  function onInput() {
    histIndex = -1;
    if (ta) {
      const pos = ta.selectionStart;
      const before = ta.value.slice(0, pos);
      const sc = typedShortcode(before);
      if (sc) {
        text = ta.value.slice(0, sc.start) + sc.char + ta.value.slice(pos);
        setCaret(sc.start + sc.char.length);
      } else {
        caret = pos;
        if (!emojiQuery(before)) emojiDismissed = -1;
      }
    }
    if (text.startsWith('/') || !text) app.sendTyping(buf, 'done');
    else app.sendTyping(buf, 'active');
  }

  function insertText(s: string) {
    const sep = text && !text.endsWith(' ') ? ' ' : '';
    text = text + sep + s;
    ta?.focus({ preventScroll: true });
  }

  async function uploadBytes(file: File) {
    uploading++;
    try {
      const data = new Uint8Array(await file.arrayBuffer());
      const url = await api.uploadBytes(conn.serverId, filehost, file.name || 'pasted.png', file.type, data);
      insertText(url);
    } catch (e) {
      app.toast(`Upload failed: ${e}`, 'error');
    } finally {
      uploading--;
    }
  }

  async function uploadPath(path: string) {
    uploading++;
    try {
      insertText(await api.uploadFile(conn.serverId, filehost, path));
    } catch (e) {
      app.toast(`Upload failed: ${e}`, 'error');
    } finally {
      uploading--;
    }
  }

  function onPaste(e: ClipboardEvent) {
    const files = [...(e.clipboardData?.files ?? [])];
    if (!files.length) return;
    if (!filehost) {
      app.toast('This server does not accept file uploads', 'error');
      return;
    }
    e.preventDefault();
    files.forEach(uploadBytes);
  }

  onMount(() => {
    // Focus follows the buffer unless the user is selecting text elsewhere.
    if (!window.getSelection()?.toString()) ta?.focus({ preventScroll: true });
    autosize();
    let unlisten: (() => void) | undefined;
    getCurrentWebview()
      .onDragDropEvent((ev) => {
        const p = ev.payload;
        if (p.type === 'enter' || p.type === 'over') dragging = !!filehost;
        else if (p.type === 'leave') dragging = false;
        else if (p.type === 'drop') {
          dragging = false;
          if (!filehost) return app.toast('This server does not accept file uploads', 'error');
          p.paths.forEach(uploadPath);
        }
      })
      .then((u) => (unlisten = u))
      .catch(() => {});
    const refocus = (e: KeyboardEvent) => {
      // Typing anywhere lands in the composer.
      const t = e.target;
      if (e.defaultPrevented || !(t instanceof Element) || t.closest('input, textarea, [contenteditable]') || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key.length === 1) ta?.focus({ preventScroll: true });
    };
    const onInsert = (e: Event) => {
      const s = (e as CustomEvent<string>).detail;
      text = text ? `${text.replace(/\s*$/, ' ')}${s}` : s;
      ta?.focus({ preventScroll: true });
    };
    const onFocusReq = () => ta?.focus({ preventScroll: true });
    window.addEventListener('keydown', refocus);
    window.addEventListener('hearth:insert', onInsert);
    window.addEventListener('hearth:focus-composer', onFocusReq);
    return () => {
      unlisten?.();
      window.removeEventListener('keydown', refocus);
      window.removeEventListener('hearth:insert', onInsert);
      window.removeEventListener('hearth:focus-composer', onFocusReq);
    };
  });
</script>

<div class="relative px-4 pb-3.5">
  {#if cmdMatches.length}
    <div class={suggestClass} role="listbox" aria-label="Commands">
      {#each cmdMatches as c, i (c.name)}
        <button
          class={cn(suggestItem, i === cmdIndex && 'bg-accent')}
          role="option"
          aria-selected={i === cmdIndex}
          onmousedown={(e) => {
            e.preventDefault();
            text = `/${c.name} `;
            ta?.focus({ preventScroll: true });
          }}
        >
          <span class="mono text-[12.5px] whitespace-nowrap">{c.usage}</span>
          <span class="truncate text-[12.5px] text-muted-foreground">{c.help}</span>
        </button>
      {/each}
    </div>
  {/if}
  {#if emoji}
    <div class={suggestClass} role="listbox" aria-label="Emoji">
      {#each emoji.matches as em, i (em.name)}
        <button
          class={cn(suggestItem, i === emojiIndex && 'bg-accent')}
          role="option"
          aria-selected={i === emojiIndex}
          onmousedown={(e) => {
            e.preventDefault();
            pickEmoji(em);
            ta?.focus({ preventScroll: true });
          }}
        >
          <span class="w-5 text-center text-base leading-none" aria-hidden="true">{em.char}</span>
          <span class="mono text-[12.5px]">:{em.name}:</span>
        </button>
      {/each}
    </div>
  {/if}
  {#if buf.replyTo}
    {@const r = buf.replyTo}
    <div class="flex items-center gap-2 rounded-t-lg border border-b-0 bg-muted/50 py-1 pr-1.5 pl-3 text-[12.5px] text-muted-foreground">
      <Icon name="reply" size={13} />
      <span>Replying to <b class="nick font-semibold" style="--h:{nickHue(r.nick)}">{r.nick}</b></span>
      <span class="min-w-0 flex-1 truncate">{stripFormatting(r.text)}</span>
      <Button variant="ghost" size="icon-xs" title="Cancel reply (Esc)" aria-label="Cancel reply" onclick={() => (buf.replyTo = null)}><Icon name="x" size={13} /></Button>
    </div>
  {/if}
  <InputGroup.Root class={cn('items-end', buf.replyTo && 'rounded-t-none')}>
    <InputGroup.Addon class="self-end py-1">
      <InputGroup.Button
        class="mono max-w-36 justify-start text-xs font-normal text-muted-foreground"
        title="Change nickname"
        aria-label="Your nickname, {conn.nick}. Change nickname"
        onclick={() => (text = `/nick ${conn.nick}`)}
      >
        <span class="truncate">{conn.nick || '…'}</span>
      </InputGroup.Button>
    </InputGroup.Addon>
    <InputGroup.Textarea
      bind:ref={ta}
      bind:value={text}
      rows={1}
      {placeholder}
      aria-label={placeholder}
      spellcheck={app.settings.spellcheck}
      autocomplete="off"
      {...{ autocorrect: 'off' }}
      autocapitalize="off"
      class="field-sizing-fixed max-h-[40vh] min-h-8 py-[7px] leading-[1.45] select-text md:text-(length:--fs)"
      style="font-size:var(--fs)"
      onkeydown={onKeydown}
      onkeyup={() => ta && (caret = ta.selectionStart)}
      onclick={() => ta && (caret = ta.selectionStart)}
      oninput={onInput}
      onpaste={onPaste}
    />
    {#if uploading || (filehost && buf.kind !== 'server')}
      <InputGroup.Addon align="inline-end" class="self-end py-1">
        {#if uploading}
          <Spinner class="size-3.5 text-muted-foreground" aria-label="Uploading" />
        {/if}
        {#if filehost && buf.kind !== 'server'}
          <Tooltip.Root>
            <Tooltip.Trigger>
              {#snippet child({ props })}
                <InputGroup.Button {...props} size="icon-xs" aria-label="Attach file" onclick={() => fileInput?.click()}>
                  <Icon name="clip" />
                </InputGroup.Button>
              {/snippet}
            </Tooltip.Trigger>
            <Tooltip.Content side="top">Attach file</Tooltip.Content>
          </Tooltip.Root>
          <input
            type="file"
            hidden
            multiple
            bind:this={fileInput}
            onchange={(e) => {
              const input = e.currentTarget;
              [...(input.files ?? [])].forEach(uploadBytes);
              input.value = '';
            }}
          />
        {/if}
      </InputGroup.Addon>
    {/if}
  </InputGroup.Root>
  {#if dragging}
    <div class="absolute inset-x-4 top-0 bottom-3.5 grid place-items-center rounded-lg border-2 border-dashed border-brand bg-background/85 text-[13px] font-medium text-brand">
      Drop to upload to {conn.name}
    </div>
  {/if}
</div>
