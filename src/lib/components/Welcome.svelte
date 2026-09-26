<script lang="ts">
  import { app } from '../state.svelte';
  import { defaultServer, type ServerConfig } from '../types';
  import ServerForm from './ServerForm.svelte';

  const presets: { id: string; label: string; server: Partial<ServerConfig> }[] = [
    { id: 'libera', label: 'Libera.Chat', server: { name: 'Libera.Chat', host: 'irc.libera.chat', port: 6697 } },
    { id: 'oftc', label: 'OFTC', server: { name: 'OFTC', host: 'irc.oftc.net', port: 6697 } },
    { id: 'soju', label: 'soju bouncer', server: { name: 'soju', host: '', port: 6697, sasl: 'plain' } },
    { id: 'custom', label: 'Custom', server: {} },
  ];

  let preset = $state('libera');
  const server = $derived({ ...defaultServer(), ...presets.find((p) => p.id === preset)!.server });
</script>

<div class="welcome" data-tauri-drag-region>
  <div class="col">
    <div class="mark" aria-hidden="true">
      <svg width="30" height="30" viewBox="0 0 30 30">
        <path d="M8 24V13a7 7 0 0 1 14 0v11Z" fill="var(--bg-sunken)" stroke="var(--line-strong)" />
        <path d="M15 14.5c1.9 2.1 2.7 3.6 2.7 5a2.7 2.7 0 0 1-5.4 0c0-1.4.8-2.9 2.7-5Z" fill="var(--accent)" />
        <rect x="5" y="24" width="20" height="2" rx="1" fill="var(--text-3)" />
      </svg>
    </div>
    <h1>Hearth</h1>
    <p class="lede">A quiet place for IRC. Add a network to begin — you can add more, or a bouncer, any time.</p>

    <div class="presets" role="tablist">
      {#each presets as p (p.id)}
        <button role="tab" aria-selected={preset === p.id} class:on={preset === p.id} onclick={() => (preset = p.id)}>{p.label}</button>
      {/each}
    </div>

    <div class="card">
      {#key preset}
        <ServerForm
          {server}
          submitLabel="Connect"
          onsave={async (s) => {
            await app.saveServer(s, true);
          }}
        />
      {/key}
    </div>
  </div>
</div>

<style>
  .welcome {
    height: 100%;
    overflow-y: auto;
    display: flex;
    justify-content: center;
    padding: 72px 24px 48px;
    background:
      radial-gradient(1200px 400px at 50% -10%, color-mix(in oklab, var(--accent) 6%, transparent), transparent 70%),
      var(--bg);
  }

  .col {
    width: min(560px, 100%);
  }

  .mark {
    margin-bottom: 18px;
  }

  h1 {
    margin: 0;
    font-size: 30px;
    font-weight: 600;
    letter-spacing: -0.025em;
  }

  .lede {
    margin: 8px 0 28px;
    color: var(--text-2);
    font-size: 15px;
    max-width: 44ch;
  }

  .presets {
    display: flex;
    gap: 6px;
    margin-bottom: 12px;
  }

  .presets button {
    height: 30px;
    padding: 0 12px;
    border-radius: 15px;
    font-size: 13px;
    color: var(--text-2);
    box-shadow: 0 0 0 1px var(--line-strong) inset;
  }

  .presets button:hover {
    color: var(--text);
  }

  .presets button.on {
    background: var(--text);
    color: var(--bg);
    box-shadow: none;
  }

  .card {
    padding: 22px;
    border-radius: 14px;
    background: var(--bg-side);
    box-shadow: 0 0 0 1px var(--line) inset;
  }
</style>
