// IRC message parsing and serialization, including IRCv3 message tags.

export interface Source {
  nick: string;
  user?: string;
  host?: string;
}

/** Serialized to the UI as-is, so the field names are the wire format. */
export interface Message {
  tags: Record<string, string>;
  source: Source | null;
  command: string;
  params: string[];
}

export function message(command: string, ...params: string[]): Message {
  return { tags: {}, source: null, command, params };
}

export function param(m: Message, i: number): string {
  return m.params[i] ?? '';
}

export function tag(m: Message, key: string): string | undefined {
  return Object.hasOwn(m.tags, key) ? m.tags[key] : undefined;
}

const trimStart = (s: string) => s.replace(/^ +/, '');

export function parse(line: string): Message | null {
  let rest = trimStart(line.replace(/[\r\n]+$/, ''));
  const tags: Record<string, string> = Object.create(null);

  if (rest.startsWith('@')) {
    const sp = rest.indexOf(' ');
    if (sp < 0) return null;
    for (const t of rest.slice(1, sp).split(';')) {
      if (!t) continue;
      const eq = t.indexOf('=');
      const k = eq < 0 ? t : t.slice(0, eq);
      tags[k] = eq < 0 ? '' : unescapeTag(t.slice(eq + 1));
    }
    rest = trimStart(rest.slice(sp + 1));
  }

  let source: Source | null = null;
  if (rest.startsWith(':')) {
    const sp = rest.indexOf(' ');
    if (sp < 0) return null;
    source = parseSource(rest.slice(1, sp));
    rest = trimStart(rest.slice(sp + 1));
  }

  const sp = rest.indexOf(' ');
  const command = sp < 0 ? rest : rest.slice(0, sp);
  rest = sp < 0 ? '' : rest.slice(sp + 1);
  if (!command) return null;

  const params: string[] = [];
  for (;;) {
    rest = trimStart(rest);
    if (!rest) break;
    if (rest.startsWith(':')) {
      params.push(rest.slice(1));
      break;
    }
    const i = rest.indexOf(' ');
    if (i < 0) {
      params.push(rest);
      break;
    }
    params.push(rest.slice(0, i));
    rest = rest.slice(i + 1);
  }

  return { tags: { ...tags }, source, command: command.toUpperCase(), params };
}

// Tags are written in key order so serialization is stable.
const byKey = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

export function serialize(m: Message): string {
  let out = '';
  const keys = Object.keys(m.tags).sort(byKey);
  if (keys.length) {
    out += '@' + keys.map((k) => (m.tags[k] ? `${k}=${escapeTag(m.tags[k])}` : k)).join(';') + ' ';
  }
  if (m.source) {
    out += ':' + m.source.nick;
    if (m.source.user != null) out += '!' + m.source.user;
    if (m.source.host != null) out += '@' + m.source.host;
    out += ' ';
  }
  out += m.command;
  m.params.forEach((p, i) => {
    out += ' ';
    if (i + 1 === m.params.length && (p === '' || p.includes(' ') || p.startsWith(':'))) out += ':';
    out += p;
  });
  return out;
}

function parseSource(raw: string): Source {
  const at = raw.indexOf('@');
  const host = at < 0 ? undefined : raw.slice(at + 1);
  const rest = at < 0 ? raw : raw.slice(0, at);
  const bang = rest.indexOf('!');
  const src: Source = { nick: bang < 0 ? rest : rest.slice(0, bang) };
  if (bang >= 0) src.user = rest.slice(bang + 1);
  if (host !== undefined) src.host = host;
  return src;
}

function unescapeTag(v: string): string {
  let out = '';
  for (let i = 0; i < v.length; i++) {
    const c = v[i];
    if (c !== '\\') {
      out += c;
      continue;
    }
    const n = v[++i];
    if (n === undefined) break;
    out += n === ':' ? ';' : n === 's' ? ' ' : n === 'r' ? '\r' : n === 'n' ? '\n' : n;
  }
  return out;
}

const ESCAPES: Record<string, string> = { ';': '\\:', ' ': '\\s', '\\': '\\\\', '\r': '\\r', '\n': '\\n' };

function escapeTag(v: string): string {
  return v.replace(/[; \\\r\n]/g, (c) => ESCAPES[c]);
}

/** Case-fold a name according to the server's CASEMAPPING. */
export function casefold(s: string, mapping: string): string {
  const rfc = mapping === 'rfc1459' || mapping === 'rfc1459-strict';
  let out = '';
  for (const c of s) {
    if (c >= 'A' && c <= 'Z') out += c.toLowerCase();
    else if (rfc && c === '[') out += '{';
    else if (rfc && c === ']') out += '}';
    else if (rfc && c === '\\') out += '|';
    else if (mapping === 'rfc1459' && c === '~') out += '^';
    else out += c;
  }
  return out;
}
