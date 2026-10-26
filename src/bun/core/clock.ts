// Time as the connection actor sees it. Production uses the real clock; the
// lifecycle tests substitute one they step by hand, so backoff and keepalive
// can be checked without waiting.

export interface Clock {
  /** Monotonic milliseconds. */
  now(): number;
  /** Run `fn` after `ms`. Returns a function that cancels it. */
  after(ms: number, fn: () => void): () => void;
}

export const realClock: Clock = {
  now: () => performance.now(),
  after(ms, fn) {
    const t = setTimeout(fn, ms);
    return () => clearTimeout(t);
  },
};

export function sleep(clock: Clock, ms: number): Promise<void> {
  return new Promise((resolve) => clock.after(ms, resolve));
}
