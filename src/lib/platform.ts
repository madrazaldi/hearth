// Desktop integration that lives in the Bun process: opening links, system
// notifications, the dock badge, the version and updates. Each one is a
// command on the same bridge as the IRC API.

import { invoke, listen } from './ipc';
import type { UpdaterProgress } from '../shared/rpc';

export interface UpdateOffer {
  version: string;
  notes?: string;
}

export const platform = {
  /** Open a URL in the user's browser. */
  openUrl: (url: string) => invoke<void>('open_url', { url }),
  notify: (title: string, body: string) => invoke<void>('notify', { title, body }),
  /** Show a count on the dock icon where the platform has one; 0 clears it. */
  setBadge: (count: number) => invoke<void>('set_badge', { count }),
  version: () => invoke<string>('app_version'),
  /** The release the update feed offers, or null when this one is current. */
  checkForUpdate: () => invoke<UpdateOffer | null>('updater_check'),
  /** Download and stage the offered update; `onProgress` sees the download advance. */
  async installUpdate(onProgress: (e: UpdaterProgress) => void): Promise<void> {
    const off = await listen('updater-progress', onProgress);
    try {
      await invoke<void>('updater_install');
    } finally {
      off();
    }
  },
  /** Restart into the installed update. A no-op when the installer already did. */
  relaunch: () => invoke<void>('relaunch'),
};

export const openUrl = platform.openUrl;
