// draft/multiline: sending a composer message with line breaks as one batch.

export interface MultilineLimits {
  maxBytes: number;
  maxLines: number;
}

/** One PRIVMSG in a batch; `concat` joins it to the previous one without a line break. */
export interface Part {
  text: string;
  concat: boolean;
}

const enc = new TextEncoder();
const bytes = (s: string) => enc.encode(s).length;

/** Parse a cap value like "max-bytes=4096,max-lines=24". max-bytes is mandatory. */
export function parseMultilineCap(value: string | undefined): MultilineLimits | null {
  const attrs = new Map((value ?? '').split(',').map((kv): [string, string] => [kv.split('=')[0], kv.split('=')[1] ?? '']));
  const maxBytes = parseInt(attrs.get('max-bytes') ?? '');
  if (!(maxBytes > 0)) return null;
  const maxLines = parseInt(attrs.get('max-lines') ?? '');
  return { maxBytes, maxLines: maxLines > 0 ? maxLines : Infinity };
}

/**
 * Split text at byte limits without losing anything, since the pieces are
 * concatenated as-is: cuts go after a space when there's one near the end.
 */
function splitExact(text: string, maxBytes: number): string[] {
  const out: string[] = [];
  let rest = text;
  while (bytes(rest) > maxBytes) {
    let lo = 1;
    let hi = rest.length;
    while (lo < hi) {
      const mid = Math.ceil((lo + hi) / 2);
      if (bytes(rest.slice(0, mid)) <= maxBytes) lo = mid;
      else hi = mid - 1;
    }
    let cut = lo;
    // Don't split a surrogate pair.
    if (cut > 1 && /[\uDC00-\uDFFF]/.test(rest[cut] ?? '')) cut--;
    const space = rest.lastIndexOf(' ', cut - 1);
    if (space + 1 > cut * 0.6) cut = space + 1;
    out.push(rest.slice(0, cut));
    rest = rest.slice(cut);
  }
  out.push(rest);
  return out;
}

/**
 * The batches to send `text` in: each is a list of parts that fits the
 * server's limits, with over-long lines split into concatenated pieces.
 */
export function multilineBatches(text: string, limits: MultilineLimits, lineBytes: number): Part[][] {
  const lines = text.replace(/\r\n?/g, '\n').replace(/^\n+|\n+$/g, '').split('\n');
  const parts: Part[] = lines.flatMap((line) =>
    line ? splitExact(line, Math.min(lineBytes, limits.maxBytes - 1)).map((t, i) => ({ text: t, concat: i > 0 })) : [{ text: '', concat: false }],
  );
  const batches: Part[][] = [];
  let cur: Part[] = [];
  let size = 0;
  for (const part of parts) {
    // Count a line break for every part: conservative about how servers measure.
    const n = bytes(part.text) + 1;
    if (cur.length && (cur.length >= limits.maxLines || size + n > limits.maxBytes)) {
      batches.push(cur);
      cur = [];
      size = 0;
    }
    // A batch can't open with a continuation; it starts a fresh line instead.
    cur.push(cur.length ? part : { ...part, concat: false });
    size += n;
  }
  if (cur.length) batches.push(cur);
  // A batch of nothing but blank lines is invalid.
  return batches.filter((b) => b.some((p) => p.text.trim()));
}

/** Join parts back into the text readers see. */
export function joinParts(parts: Part[]): string {
  return parts.map((p, i) => (i && !p.concat ? '\n' : '') + p.text).join('');
}
