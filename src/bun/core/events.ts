// Events sent to the frontend. They are coalesced into arrays so a burst of
// traffic (NAMES, history, netsplits) costs one IPC round instead of hundreds.

import type { Message } from './message';

export type Status = 'connecting' | 'registering' | 'connected' | 'disconnected';

export type Event =
  | { type: 'conn-added'; conn: string; server: string; parent: string | null; netid: string | null }
  | { type: 'conn-removed'; conn: string }
  | { type: 'status'; conn: string; status: Status; error: string | null; retryIn: number | null }
  /** `capValues` holds the values of enabled caps that have one, e.g. `draft/multiline` limits. */
  | { type: 'registered'; conn: string; nick: string; caps: string[]; capValues: Record<string, string> }
  | { type: 'caps'; conn: string; caps: string[]; capValues: Record<string, string> }
  | { type: 'isupport'; conn: string; isupport: Record<string, string> }
  | { type: 'message'; conn: string; msg: Message };

const BATCH_DELAY_MS = 8;
const MAX_BATCH = 2000;

export class Emitter {
  private queue: Event[] = [];
  private timer: ReturnType<typeof setTimeout> | null = null;

  /** `sink` receives each coalesced batch of events. */
  constructor(private sink: (events: Event[]) => void) {}

  emit(ev: Event) {
    this.queue.push(ev);
    // Let a burst accumulate briefly so it crosses IPC as one payload.
    this.timer ??= setTimeout(() => this.drain(), BATCH_DELAY_MS);
  }

  private drain() {
    this.timer = null;
    const batch = this.queue.splice(0, MAX_BATCH);
    if (this.queue.length) this.timer = setTimeout(() => this.drain(), 0);
    if (batch.length) this.sink(batch);
  }

  /** Deliver everything queued now; for shutdown. */
  flush() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    while (this.queue.length) this.sink(this.queue.splice(0, MAX_BATCH));
  }
}
