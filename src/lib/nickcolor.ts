const cache = new Map<string, number>();

/** A stable hue for a nick; lightness and chroma come from the theme. */
export function nickHue(nick: string): number {
  const key = nick.toLowerCase().replace(/[_`|^]+$/, '');
  let h = cache.get(key);
  if (h !== undefined) return h;
  let hash = 2166136261;
  for (let i = 0; i < key.length; i++) {
    hash ^= key.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  h = (hash >>> 0) % 360;
  cache.set(key, h);
  return h;
}
