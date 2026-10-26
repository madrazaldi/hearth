import { platform } from './platform';
import { app } from './state.svelte';

type Phase = 'idle' | 'checking' | 'current' | 'available' | 'installing' | 'error';

// IRC clients tend to stay open for weeks, so a launch-only check isn't enough.
const RECHECK_MS = 6 * 60 * 60 * 1000;

class Updater {
  current = $state('');
  phase = $state<Phase>('idle');
  /** Version offered by the update feed, once one is found. */
  version = $state<string | null>(null);
  /** Download progress 0..1, or null while the size is unknown. */
  progress = $state<number | null>(null);
  error = $state<string | null>(null);
  private announced: string | null = null;
  private pending = false;

  /** Reads the running version and, if enabled, checks quietly now and periodically. */
  async start() {
    try {
      this.current = await platform.version();
    } catch {}
    const tick = () => app.settings.checkUpdates && this.check(true);
    tick();
    setInterval(tick, RECHECK_MS);
  }

  get busy() {
    return this.phase === 'checking' || this.phase === 'installing';
  }

  /** `quiet` checks swallow errors and announce a new version once with a toast. */
  async check(quiet = false) {
    if (this.busy) return;
    this.phase = 'checking';
    this.error = null;
    try {
      const found = await platform.checkForUpdate();
      if (!found) {
        this.version = null;
        this.phase = 'current';
        return;
      }
      this.version = found.version;
      this.phase = 'available';
      if (quiet && this.announced !== found.version) {
        this.pending = true;
        this.announce();
      }
    } catch (e) {
      // A failed background recheck shouldn't hide an update that was already found.
      this.phase = quiet ? (this.version ? 'available' : 'idle') : 'error';
      this.error = String(e);
    }
  }

  /** Toasts a quietly found update, unless the first-run welcome card is up (the toast would cover it). */
  announce() {
    if (!this.pending || !this.version || app.welcoming) return;
    this.pending = false;
    this.announced = this.version;
    app.toast(`Hearth ${this.version} is available. Install it from Settings › General.`);
  }

  async installAndRestart() {
    if (!this.version || this.busy) return;
    this.phase = 'installing';
    this.progress = null;
    let total = 0;
    let done = 0;
    try {
      await platform.installUpdate((e) => {
        if (e.event === 'Started') total = e.contentLength ?? 0;
        else if (e.event === 'Progress' && total) this.progress = Math.min(1, (done += e.chunkLength ?? 0) / total);
      });
      // Applying the update restarts the app by itself; this is for platforms where it doesn't.
      await platform.relaunch();
    } catch (e) {
      this.phase = 'error';
      this.error = String(e);
    }
  }
}

export const updater = new Updater();
