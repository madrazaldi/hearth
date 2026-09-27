// Helpers for the virtualized message list: rows have variable heights that
// are only known once rendered, so unmeasured rows use a running average for
// their kind of row.

/** Running average height per category, seeded with a guess. */
export class Estimates {
  private stats = new Map<string, { sum: number; n: number }>();

  constructor(private guesses: Record<string, number>) {}

  get(cat: string): number {
    const s = this.stats.get(cat);
    return s ? s.sum / s.n : (this.guesses[cat] ?? 24);
  }

  add(cat: string, h: number, old?: number) {
    const s = this.stats.get(cat) ?? { sum: 0, n: 0 };
    if (old === undefined) s.n++;
    s.sum += h - (old ?? 0);
    this.stats.set(cat, s);
  }

  clear() {
    this.stats.clear();
  }
}

/** Prefix sums: `out[i]` is the top of item `i`, `out[n]` the total height. */
export function offsets(n: number, height: (i: number) => number): Float64Array {
  const out = new Float64Array(n + 1);
  for (let i = 0; i < n; i++) out[i + 1] = out[i] + height(i);
  return out;
}

/** The item at `y` (clamped to the list). */
export function indexAt(offs: Float64Array, y: number): number {
  const n = offs.length - 1;
  let lo = 0;
  let hi = n;
  // First i with offs[i + 1] > y.
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (offs[mid + 1] > y) hi = mid;
    else lo = mid + 1;
  }
  return Math.min(lo, Math.max(0, n - 1));
}

/** Items overlapping [top, bottom), as a half-open range. */
export function rangeFor(offs: Float64Array, top: number, bottom: number): [number, number] {
  const n = offs.length - 1;
  if (!n) return [0, 0];
  return [indexAt(offs, top), Math.min(n, indexAt(offs, bottom) + 1)];
}
