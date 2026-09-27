<script lang="ts">
  import { app } from '../state.svelte';
  import Icon from './Icon.svelte';
  import ShortcutList from './ShortcutList.svelte';
  import { modal } from '../a11y';

  const close = () => (app.shortcutsOpen = false);
</script>

<div class="scrim" onclick={close} role="presentation">
  <div class="card" onclick={(e) => e.stopPropagation()} role="dialog" aria-modal="true" aria-labelledby="shortcuts-title" tabindex="-1" onkeydown={() => {}} use:modal={{ onclose: close }}>
    <header>
      <h2 id="shortcuts-title">Keyboard shortcuts</h2>
      <button class="icon-btn" onclick={close} aria-label="Close"><Icon name="x" /></button>
    </header>
    <ShortcutList />
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 95;
    background: rgb(0 0 0 / 0.35);
    display: grid;
    place-items: center;
    animation: fade 120ms;
  }

  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  .card {
    width: min(520px, 92vw);
    max-height: 86vh;
    overflow-y: auto;
    padding: 22px 26px 26px;
    background: var(--bg);
    border-radius: 14px;
    box-shadow: var(--shadow);
    user-select: none;
    animation: rise 180ms var(--ease);
  }

  @keyframes rise {
    from {
      transform: translateY(8px) scale(0.995);
      opacity: 0;
    }
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 18px;
  }

  h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.015em;
  }
</style>
