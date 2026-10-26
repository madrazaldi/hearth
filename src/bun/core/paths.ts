// Where Hearth keeps its files. These mirror what Tauri's path resolver gave
// the previous build (bundle identifier `app.hearth.irc`), so existing
// installs keep their config and log.

import { homedir } from 'node:os';
import { posix, win32 } from 'node:path';

export const APP_ID = 'app.hearth.irc';

export interface Dirs {
  config: string;
  data: string;
}

export function appDirs(env: Record<string, string | undefined> = process.env, platform: string = process.platform, home: string = homedir()): Dirs {
  // Escape hatches for running a build against throwaway data.
  const override = { config: env.HEARTH_CONFIG_DIR, data: env.HEARTH_DATA_DIR };
  // By the target platform's rules, not the host's, so every branch can be tested anywhere.
  const join = platform === 'win32' ? win32.join : posix.join;
  let base: Dirs;
  if (platform === 'darwin') {
    const dir = join(home, 'Library', 'Application Support', APP_ID);
    base = { config: dir, data: dir };
  } else if (platform === 'win32') {
    const dir = join(env.APPDATA || join(home, 'AppData', 'Roaming'), APP_ID);
    base = { config: dir, data: dir };
  } else {
    base = {
      config: join(env.XDG_CONFIG_HOME || join(home, '.config'), APP_ID),
      data: join(env.XDG_DATA_HOME || join(home, '.local', 'share'), APP_ID),
    };
  }
  return { config: override.config || base.config, data: override.data || base.data };
}
