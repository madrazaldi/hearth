// RFC 3339 timestamps as used by the IRCv3 `server-time` tag.

export function nowMs(): number {
  return Date.now();
}

/** Parse `YYYY-MM-DDTHH:MM:SS[.fff]Z` into milliseconds since the epoch. */
export function parseMs(s: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?Z$/.exec(s);
  if (!m) return null;
  const [y, mo, d, h, mi, sec] = m.slice(1, 7).map(Number);
  const ms = m[7] ? Number(m[7].slice(0, 3).padEnd(3, '0')) : 0;
  return Date.UTC(y, mo - 1, d, h, mi, sec) + ms;
}

/** Format milliseconds since the epoch as `YYYY-MM-DDTHH:MM:SS.fffZ`. */
export function formatMs(ms: number): string {
  return new Date(ms).toISOString();
}
