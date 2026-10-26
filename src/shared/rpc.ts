// What crosses between the webview and the Bun process besides commands:
// events Bun pushes to the UI. Commands are named requests (see
// src/bun/commands.ts); the UI-side bridge is src/lib/ipc.ts.

export interface UpdaterProgress {
  event: 'Started' | 'Progress' | 'Finished';
  /** Size of the download, when known. */
  contentLength?: number;
  /** Bytes received since the last event. */
  chunkLength?: number;
}

/** Everything Bun pushes to the webview, by event name. */
export interface PushEvents {
  irc: unknown[];
  'updater-progress': UpdaterProgress;
}
