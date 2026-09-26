// Protocol helpers that don't depend on app state.

export function casefold(s: string, mapping = 'rfc1459'): string {
  let out = s.toLowerCase();
  if (mapping === 'rfc1459' || mapping === 'rfc1459-strict') {
    out = out.replace(/\[/g, '{').replace(/\]/g, '}').replace(/\\/g, '|');
    if (mapping === 'rfc1459') out = out.replace(/~/g, '^');
  }
  return out;
}

export interface PrefixMap {
  modes: string; // e.g. "qaohv"
  symbols: string; // e.g. "~&@%+"
}

export function parsePrefix(isupport: Record<string, string>): PrefixMap {
  const m = /^\(([^)]*)\)(.*)$/.exec(isupport.PREFIX ?? '(ov)@+');
  if (!m || m[1].length !== m[2].length) return { modes: 'ov', symbols: '@+' };
  return { modes: m[1], symbols: m[2] };
}

/** Parse a CTCP payload wrapped in \x01. */
export function parseCtcp(text: string): { command: string; args: string } | null {
  if (!text.startsWith('\x01')) return null;
  const body = text.slice(1).replace(/\x01$/, '');
  const i = body.indexOf(' ');
  return i < 0 ? { command: body.toUpperCase(), args: '' } : { command: body.slice(0, i).toUpperCase(), args: body.slice(i + 1) };
}

export function isoTime(ms: number): string {
  return new Date(ms).toISOString();
}

export function msgTime(tags: Record<string, string>): number {
  const t = tags.time ? Date.parse(tags.time) : NaN;
  return Number.isNaN(t) ? Date.now() : t;
}

/** Split text into chunks that fit an IRC line, preferring word boundaries. */
export function splitMessage(text: string, maxBytes: number): string[] {
  const enc = new TextEncoder();
  if (enc.encode(text).length <= maxBytes) return [text];
  const out: string[] = [];
  let rest = text;
  while (enc.encode(rest).length > maxBytes) {
    // Find the longest prefix by characters that fits in maxBytes.
    let lo = 1;
    let hi = rest.length;
    while (lo < hi) {
      const mid = Math.ceil((lo + hi) / 2);
      if (enc.encode(rest.slice(0, mid)).length <= maxBytes) lo = mid;
      else hi = mid - 1;
    }
    let cut = lo;
    const space = rest.lastIndexOf(' ', cut);
    if (space > cut * 0.6) cut = space;
    out.push(rest.slice(0, cut));
    rest = rest.slice(cut).replace(/^ /, '');
  }
  if (rest) out.push(rest);
  return out;
}

/** Parse "a=b;c=d" attribute lists used by soju BOUNCER messages. */
export function parseAttrs(s: string): Record<string, string> {
  const out: Record<string, string> = {};
  if (!s || s === '*') return out;
  for (const part of s.split(';')) {
    const i = part.indexOf('=');
    const k = i < 0 ? part : part.slice(0, i);
    const v = i < 0 ? '' : part.slice(i + 1);
    out[k] = v.replace(/\\(.)/g, (_, c) => ({ ':': ';', s: ' ', '\\': '\\', r: '\r', n: '\n' })[c as string] ?? c);
  }
  return out;
}

export function escapeAttr(v: string): string {
  return v.replace(/\\/g, '\\\\').replace(/;/g, '\\:').replace(/ /g, '\\s');
}
