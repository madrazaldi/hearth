// Ignore masks: "nick", "nick!user@host", with * and ? wildcards.

/** Expand shorthand into a full nick!user@host mask. */
export function normalizeMask(input: string): string {
  const m = input.trim();
  if (!m) return '';
  if (m.includes('!')) return m.includes('@') ? m : `${m}@*`;
  if (m.includes('@')) return `*!${m}`;
  return `${m}!*@*`;
}

function toRegex(mask: string): RegExp {
  const body = mask.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.');
  return new RegExp(`^${body}$`);
}

const cache = new Map<string, RegExp>();

/** Whether `source` (nick, user, host) matches any mask. `fold` case-folds per the server's casemapping. */
export function isIgnored(
  masks: string[],
  source: { nick: string; user?: string; host?: string },
  fold: (s: string) => string,
): boolean {
  if (!masks.length || !source.nick) return false;
  const full = fold(`${source.nick}!${source.user ?? ''}@${source.host ?? ''}`);
  return masks.some((raw) => {
    const mask = fold(normalizeMask(raw));
    let re = cache.get(mask);
    if (!re) cache.set(mask, (re = toRegex(mask)));
    return re.test(full);
  });
}
