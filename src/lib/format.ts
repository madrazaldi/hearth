// mIRC formatting codes and link detection → renderable segments.

export interface Segment {
  text: string;
  bold?: boolean;
  italic?: boolean;
  underline?: boolean;
  strike?: boolean;
  mono?: boolean;
  fg?: string;
  bg?: string;
  href?: string;
  channel?: string;
}

// Colors 16–98 from the extended mIRC table.
const EXTENDED = [
  '470000', '472100', '474700', '324700', '004700', '00472c', '004747', '002747', '000047', '2e0047', '470047', '47002a',
  '740000', '743a00', '747400', '517400', '007400', '007449', '007474', '004074', '000074', '4b0074', '740074', '740045',
  'b50000', 'b56300', 'b5b500', '7db500', '00b500', '00b571', '00b5b5', '0063b5', '0000b5', '7500b5', 'b500b5', 'b5006b',
  'ff0000', 'ff8c00', 'ffff00', 'b2ff00', '00ff00', '00ffa0', '00ffff', '008cff', '0000ff', 'a500ff', 'ff00ff', 'ff0098',
  'ff5959', 'ffb459', 'ffff71', 'cfff60', '6fff6f', '65ffc9', '6dffff', '59b4ff', '5959ff', 'c459ff', 'ff66ff', 'ff59bc',
  'ff9c9c', 'ffd39c', 'ffff9c', 'e2ff9c', '9cff9c', '9cffdb', '9cffff', '9cd3ff', '9c9cff', 'dc9cff', 'ff9cff', 'ff94d3',
  '000000', '131313', '282828', '363636', '4d4d4d', '656565', '818181', '9f9f9f', 'bcbcbc', 'e2e2e2', 'ffffff',
];

function colorCss(n: number): string | undefined {
  if (n >= 0 && n <= 15) return `var(--irc-${n})`;
  if (n >= 16 && n <= 98) return '#' + EXTENDED[n - 16];
  return undefined;
}

const URL_RE = /\b(?:https?:\/\/|www\.)[^\s<>"'\x00-\x1f]+|\bmailto:[^\s<>"']+/gi;
const CHAN_RE = /(^|[\s(])(#[^\s,\x07()]{1,64})/g;

export function stripFormatting(s: string): string {
  return s.replace(/\x03(\d{1,2}(,\d{1,2})?)?|\x04([0-9a-fA-F]{6}(,[0-9a-fA-F]{6})?)?|[\x02\x0f\x11\x16\x1d\x1e\x1f]/g, '');
}

export function parseFormatting(input: string): Segment[] {
  const out: Segment[] = [];
  let cur: Segment = { text: '' };
  const push = () => {
    if (cur.text) out.push(cur);
    cur = { ...cur, text: '' };
  };
  let i = 0;
  while (i < input.length) {
    const c = input[i];
    switch (c) {
      case '\x02': push(); cur.bold = !cur.bold; i++; continue;
      case '\x1d': push(); cur.italic = !cur.italic; i++; continue;
      case '\x1f': push(); cur.underline = !cur.underline; i++; continue;
      case '\x1e': push(); cur.strike = !cur.strike; i++; continue;
      case '\x11': push(); cur.mono = !cur.mono; i++; continue;
      case '\x16': push(); [cur.fg, cur.bg] = [cur.bg ?? 'var(--bg)', cur.fg ?? 'var(--text)']; i++; continue;
      case '\x0f': push(); cur = { text: '' }; i++; continue;
      case '\x03': {
        push();
        const m = /^(\d{1,2})(?:,(\d{1,2}))?/.exec(input.slice(i + 1));
        if (!m) {
          cur.fg = cur.bg = undefined;
          i++;
        } else {
          cur.fg = colorCss(+m[1]);
          if (m[2] !== undefined) cur.bg = colorCss(+m[2]);
          i += 1 + m[0].length;
        }
        continue;
      }
      case '\x04': {
        push();
        const m = /^([0-9a-fA-F]{6})(?:,([0-9a-fA-F]{6}))?/.exec(input.slice(i + 1));
        if (!m) {
          cur.fg = cur.bg = undefined;
          i++;
        } else {
          cur.fg = '#' + m[1];
          if (m[2]) cur.bg = '#' + m[2];
          i += 1 + m[0].length;
        }
        continue;
      }
      default:
        cur.text += c;
        i++;
    }
  }
  push();
  return out.flatMap(linkify);
}

function trimUrl(url: string): string {
  // Drop trailing punctuation, but keep a closing paren that balances one in the URL.
  while (/[.,;:!?'"\]>]$/.test(url) || (url.endsWith(')') && count(url, '(') < count(url, ')'))) {
    url = url.slice(0, -1);
  }
  return url;
}

function count(s: string, ch: string): number {
  return s.split(ch).length - 1;
}

function linkify(seg: Segment): Segment[] {
  const text = seg.text;
  const found: { start: number; end: number; href?: string; channel?: string }[] = [];
  for (const m of text.matchAll(URL_RE)) {
    const url = trimUrl(m[0]);
    const href = /^www\./i.test(url) ? 'https://' + url : url;
    found.push({ start: m.index!, end: m.index! + url.length, href });
  }
  for (const m of text.matchAll(CHAN_RE)) {
    const name = m[2].replace(/[.,;:!?'"]+$/, '');
    const start = m.index! + m[1].length;
    if (!found.some((f) => start < f.end && start + name.length > f.start)) {
      found.push({ start, end: start + name.length, channel: name });
    }
  }
  if (!found.length) return [seg];
  found.sort((a, b) => a.start - b.start);
  const out: Segment[] = [];
  let pos = 0;
  for (const f of found) {
    if (f.start > pos) out.push({ ...seg, text: text.slice(pos, f.start) });
    out.push({ ...seg, text: text.slice(f.start, f.end), href: f.href, channel: f.channel });
    pos = f.end;
  }
  if (pos < text.length) out.push({ ...seg, text: text.slice(pos) });
  return out;
}

const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const REGEX_RULE = /^\/(.+)\/([a-z]*)$/s;
const ruleCache = new Map<string, RegExp | null>();

function compileRule(rule: string): RegExp | null {
  let re = ruleCache.get(rule);
  if (re !== undefined) return re;
  const m = REGEX_RULE.exec(rule);
  try {
    // g and y make .test() stateful, so they're dropped.
    re = m ? new RegExp(m[1], m[2].replace(/[gy]/g, '')) : new RegExp(`(^|[^\\w-])${escapeRe(rule)}($|[^\\w-])`, 'i');
  } catch {
    re = null;
  }
  if (ruleCache.size > 500) ruleCache.clear();
  ruleCache.set(rule, re);
  return re;
}

/** Why a highlight rule is invalid, or null. `/pattern/flags` rules are regexes; anything else is a word. */
export function ruleError(rule: string): string | null {
  const m = REGEX_RULE.exec(rule.trim());
  if (!m) return null;
  try {
    new RegExp(m[1], m[2]);
    return null;
  } catch (e) {
    return e instanceof Error ? e.message.replace(/^Invalid regular expression: /, '') : String(e);
  }
}

/** The first rule that matches: words match whole-word and case-insensitively, `/regex/flags` as written. */
export function matchingRule(text: string, rules: string[]): string | null {
  const plain = stripFormatting(text);
  for (const r of rules) if (r && compileRule(r)?.test(plain)) return r;
  return null;
}

export function mentions(text: string, rules: string[]): boolean {
  return matchingRule(text, rules) !== null;
}

/** The first http(s) link in a message, for previews. */
export function firstLink(text: string): string | null {
  const plain = stripFormatting(text);
  for (const m of plain.matchAll(URL_RE)) {
    const url = trimUrl(m[0]);
    if (/^mailto:/i.test(url)) continue;
    return /^www\./i.test(url) ? 'https://' + url : url;
  }
  return null;
}
