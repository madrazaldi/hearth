import { EMOJI_DATA } from './emoji-data';

export interface Emoji {
  name: string;
  char: string;
}

let table: Emoji[] | undefined;
let byName: Map<string, string> | undefined;

function all(): Emoji[] {
  table ??= EMOJI_DATA.split('\n').map((l) => {
    const i = l.indexOf(' ');
    return { name: l.slice(0, i), char: l.slice(i + 1) };
  });
  return table;
}

export function emojiByName(name: string): string | undefined {
  byName ??= new Map(all().map((e) => [e.name, e.char]));
  return byName.get(name.toLowerCase());
}

// The colon has to start a word, so `12:30`, URLs and `nick:` don't trigger it.
const QUERY = /(?:^|\s)(:([\w+-]+))$/;
const SHORTCODE = /(?:^|\s)(:([\w+-]+):)$/;

/** The `:name` being typed at the end of `before`, with where its colon is. */
export function emojiQuery(before: string): { start: number; query: string } | null {
  const m = QUERY.exec(before);
  return m ? { start: before.length - m[1].length, query: m[2].toLowerCase() } : null;
}

/** Names starting with the query first, then names containing it, each in gemoji order. */
export function searchEmoji(query: string, limit = 7): Emoji[] {
  const q = query.toLowerCase();
  const out = all().filter((e) => e.name.startsWith(q));
  if (out.length < limit) out.push(...all().filter((e) => !e.name.startsWith(q) && e.name.includes(q)));
  return out.slice(0, limit);
}

/** A complete `:name:` just typed at the end of `before`, as its emoji and where it starts. */
export function typedShortcode(before: string): { start: number; char: string } | null {
  const m = SHORTCODE.exec(before);
  const char = m && emojiByName(m[2]);
  return char ? { start: before.length - m[1].length, char } : null;
}
