<script lang="ts">
  import { onMount, tick } from 'svelte';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { app, type Buffer } from '../state.svelte';
  import { COMMANDS, runInput } from '../commands';
  import { api } from '../api';
  import Icon from './Icon.svelte';

  let { buf }: { buf: Buffer } = $props();

  // svelte-ignore state_referenced_locally
  let text = $state(buf.draft);
  let ta = $state<HTMLTextAreaElement>();
  let fileInput = $state<HTMLInputElement>();
  let dragging = $state(false);
  let uploading = $state(0);
  let histIndex = -1;
  let completion: { start: number; end: number; options: string[]; index: number; suffix: string } | null = null;
  let cmdIndex = $state(0);

  const conn = $derived(buf.conn);
  const filehost = $derived(conn.isupport['soju.im/FILEHOST'] || conn.isupport['FILEHOST'] || '');
  const placeholder = $derived(
    buf.kind === 'server'
      ? `Type /help for commands`
      : buf.kind === 'channel' && !buf.joined
        ? `You're not in ${buf.name}`
        : `Message ${buf.name}`,
  );

  const cmdMatches = $derived.by(() => {
    const m = /^\/(\w*)$/.exec(text);
    if (!m) return [];
    const q = m[1].toLowerCase();
    return Object.entries(COMMANDS)
      .filter(([name]) => name.startsWith(q))
      .slice(0, 7);
  });

  $effect(() => {
    void cmdMatches;
    cmdIndex = 0;
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
    if (await runInput(app, buf, value)) {
      app.inputHistory = [value, ...app.inputHistory.filter((h) => h !== value)].slice(0, 100);
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

  function complete(backwards: boolean) {
    if (!ta) return;
    if (completion && ta.selectionStart === completion.end) {
      const c = completion;
      c.index = (c.index + (backwards ? -1 : 1) + c.options.length) % c.options.length;
      apply(c);
      return;
    }
    const caret = ta.selectionStart;
    const before = text.slice(0, caret);
    const start = before.search(/\S*$/);
    const word = before.slice(start);
    if (!word) return;
    const lw = word.toLowerCase();
    let options: string[] = [];
    let suffix = ' ';
    if (start === 0 && word.startsWith('/')) {
      options = Object.keys(COMMANDS)
        .filter((c) => c.startsWith(lw.slice(1)))
        .map((c) => '/' + c);
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
    if (!options.length) return;
    completion = { start, end: caret, options, index: backwards ? options.length - 1 : 0, suffix };
    apply(completion);
  }

  function apply(c: NonNullable<typeof completion>) {
    const insert = c.options[c.index] + c.suffix;
    const after = text.slice(c.end).replace(/^(: | )/, '');
    text = text.slice(0, c.start) + insert + after;
    c.end = c.start + insert.length;
    tick().then(() => ta?.setSelectionRange(c.end, c.end));
  }

  function onKeydown(e: KeyboardEvent) {
    if (cmdMatches.length && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      cmdIndex = (cmdIndex + (e.key === 'ArrowUp' ? -1 : 1) + cmdMatches.length) % cmdMatches.length;
      return;
    }
    if (cmdMatches.length && (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey && cmdMatches[cmdIndex][0] !== text.slice(1)))) {
      e.preventDefault();
      text = `/${cmdMatches[cmdIndex][0]} `;
      return;
    }
    if (e.key === 'Tab') {
      e.preventDefault();
      complete(e.shiftKey);
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
    if (e.key === 'Escape') ta?.blur();
  }

  function onInput() {
    histIndex = -1;
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
      const t = e.target as HTMLElement;
      if (t.closest('input, textarea, [contenteditable]') || e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key.length === 1) ta?.focus({ preventScroll: true });
    };
    const onInsert = (e: Event) => {
      const s = (e as CustomEvent<string>).detail;
      text = text ? `${text.replace(/\s*$/, ' ')}${s}` : s;
      ta?.focus({ preventScroll: true });
    };
    window.addEventListener('keydown', refocus);
    window.addEventListener('hearth:insert', onInsert);
    return () => {
      unlisten?.();
      window.removeEventListener('keydown', refocus);
      window.removeEventListener('hearth:insert', onInsert);
    };
  });
</script>

<div class="composer" class:dragging>
  {#if cmdMatches.length}
    <div class="suggest" role="listbox">
      {#each cmdMatches as [name, c], i (name)}
        <button
          class:sel={i === cmdIndex}
          role="option"
          aria-selected={i === cmdIndex}
          onmousedown={(e) => {
            e.preventDefault();
            text = `/${name} `;
            ta?.focus({ preventScroll: true });
          }}
        >
          <span class="mono">{c.usage}</span>
          <span class="help">{c.help}</span>
        </button>
      {/each}
    </div>
  {/if}
  <div class="box">
    <button class="nickname mono" title="Change nickname" onclick={() => (text = `/nick ${conn.nick}`)}>{conn.nick || '…'}</button>
    <textarea
      bind:this={ta}
      bind:value={text}
      rows="1"
      {placeholder}
      spellcheck="true"
      autocomplete="off"
      onkeydown={onKeydown}
      oninput={onInput}
      onpaste={onPaste}
    ></textarea>
    {#if uploading}
      <span class="uploading" title="Uploading"></span>
    {/if}
    {#if filehost && buf.kind !== 'server'}
      <button class="icon-btn" title="Attach file" onclick={() => fileInput?.click()}><Icon name="clip" /></button>
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
  </div>
  {#if dragging}
    <div class="drop">Drop to upload to {conn.name}</div>
  {/if}
</div>

<style>
  .composer {
    position: relative;
    padding: 0 16px 14px;
  }

  .box {
    display: flex;
    align-items: flex-end;
    gap: 10px;
    padding: 6px 6px 6px 12px;
    background: var(--bg-raised);
    border-radius: 10px;
    box-shadow: 0 0 0 1px var(--line-strong) inset;
    transition: box-shadow 120ms;
  }

  .box:focus-within {
    box-shadow:
      0 0 0 1px color-mix(in oklab, var(--accent) 45%, var(--line-strong)) inset,
      0 0 0 3px var(--accent-soft);
  }

  .nickname {
    flex: none;
    height: 28px;
    font-size: 12px;
    color: var(--text-3);
    padding: 0 8px 0 0;
    border-right: 1px solid var(--line);
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .nickname:hover {
    color: var(--text);
  }

  textarea {
    flex: 1;
    min-height: 28px;
    max-height: 40vh;
    padding: 4px 0;
    border: 0;
    outline: none;
    resize: none;
    background: none;
    line-height: 1.45;
    font-size: var(--fs);
    user-select: text;
    -webkit-user-select: text;
  }

  textarea::placeholder {
    color: var(--text-4);
  }

  .uploading {
    align-self: center;
    width: 14px;
    height: 14px;
    margin: 0 6px;
    border-radius: 50%;
    border: 1.5px solid var(--line-strong);
    border-top-color: var(--accent);
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .suggest {
    position: absolute;
    left: 16px;
    right: 16px;
    bottom: calc(100% - 8px);
    padding: 4px;
    background: var(--bg-raised);
    border-radius: 10px;
    box-shadow: var(--shadow);
    display: grid;
    z-index: 10;
  }

  .suggest button {
    display: flex;
    align-items: baseline;
    gap: 14px;
    padding: 6px 10px;
    border-radius: 6px;
    text-align: left;
  }

  .suggest button.sel {
    background: var(--bg-active);
  }

  .suggest .mono {
    font-size: 12.5px;
    color: var(--text);
    white-space: nowrap;
  }

  .suggest .help {
    font-size: 12.5px;
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .drop {
    position: absolute;
    inset: 0 16px 14px;
    display: grid;
    place-items: center;
    border-radius: 10px;
    border: 1.5px dashed var(--accent);
    background: color-mix(in oklab, var(--bg) 85%, var(--accent));
    color: var(--accent);
    font-size: 13px;
    font-weight: 500;
  }
</style>
