<script lang="ts">
  import { openUrl } from '@tauri-apps/plugin-opener';
  import { parseFormatting, stripFormatting } from '../format';
  import { app } from '../state.svelte';

  let { text, plain = false }: { text: string; plain?: boolean } = $props();

  const segments = $derived(plain ? [] : parseFormatting(text));

  function open(e: MouseEvent, href: string) {
    e.preventDefault();
    e.stopPropagation();
    openUrl(href).catch((err) => app.toast(String(err), 'error'));
  }

  function joinChan(e: MouseEvent, name: string) {
    e.preventDefault();
    e.stopPropagation();
    const conn = app.active?.conn;
    if (!conn) return;
    const existing = conn.buffer(name);
    if (existing) app.open(existing);
    else app.join(conn, name);
  }

  function style(s: (typeof segments)[number]): string | undefined {
    let out = '';
    if (s.fg) out += `color:${s.fg};`;
    if (s.bg) out += `background:${s.bg};`;
    return out || undefined;
  }
</script>

{#if plain}{stripFormatting(text)}{:else}{#each segments as s, i (i)}{#if s.href}<a
        href={s.href}
        class="link"
        class:b={s.bold}
        class:i={s.italic}
        title={s.href}
        onclick={(e) => open(e, s.href!)}>{s.text}</a
      >{:else if s.channel}<a href="#{s.channel}" class="chan" onclick={(e) => joinChan(e, s.channel!)}>{s.text}</a
      >{:else if s.bold || s.italic || s.underline || s.strike || s.mono || s.fg || s.bg}<span
        class:b={s.bold}
        class:i={s.italic}
        class:u={s.underline}
        class:s={s.strike}
        class:m={s.mono}
        class:bgc={!!s.bg}
        style={style(s)}>{s.text}</span
      >{:else}{s.text}{/if}{/each}{/if}

<style>
  .b {
    font-weight: 600;
  }
  .i {
    font-style: italic;
  }
  .u {
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .s {
    text-decoration: line-through;
  }
  .u.s {
    text-decoration: underline line-through;
  }
  .m {
    font-family: var(--font-mono);
    font-size: 0.92em;
  }
  .bgc {
    padding: 0 2px;
    border-radius: 2px;
  }
  .link {
    color: var(--brand);
    text-decoration: none;
    word-break: break-all;
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  .chan {
    color: inherit;
    text-decoration: underline;
    text-decoration-color: var(--line-strong);
    text-underline-offset: 2px;
    cursor: pointer;
  }
  .chan:hover {
    text-decoration-color: currentColor;
  }
</style>
