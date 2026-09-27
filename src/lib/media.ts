// Pure helpers for inline images and media cards.

import { stripFormatting } from './format';
import type { Preview } from './types';

export const THUMB_W = 360;
export const THUMB_H = 280;

/** Scale `w`×`h` down (never up) to fit inside `maxW`×`maxH`, keeping the aspect ratio. */
export function fitBox(w: number, h: number, maxW = THUMB_W, maxH = THUMB_H): { width: number; height: number } {
  if (!(w > 0 && h > 0)) return { width: maxW, height: maxH };
  const k = Math.min(1, maxW / w, maxH / h);
  return { width: Math.max(1, Math.round(w * k)), height: Math.max(1, Math.round(h * k)) };
}

/** Messages that mention NSFW/NSFL get their images blurred until clicked. */
export function isNsfw(text: string): boolean {
  return /(^|[^a-z0-9])nsf[wl]([^a-z0-9]|$)/i.test(stripFormatting(text));
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) (v /= 1024), i++;
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/** The last path segment of a URL, decoded, for naming a file card. */
export function fileName(url: string): string {
  try {
    const u = new URL(url);
    const last = u.pathname.split('/').filter(Boolean).pop();
    return last ? decodeURIComponent(last) : u.host;
  } catch {
    return url;
  }
}

/** "Video · MP4 · 12 MB" */
export function mediaLabel(p: Pick<Preview, 'kind' | 'mime' | 'size' | 'width' | 'height'>): string {
  const [type, sub = ''] = (p.mime ?? '').split('/');
  const kind = type === 'video' ? 'Video' : type === 'audio' ? 'Audio' : type === 'image' ? 'Image' : 'File';
  const parts = [kind];
  const fmt = sub.replace(/^x-/, '').replace(/^(vnd\.|x-)/, '').split(/[+;.]/)[0];
  if (fmt) parts.push(fmt === 'mpeg' && type === 'audio' ? 'MP3' : fmt.toUpperCase());
  if (p.width && p.height) parts.push(`${p.width}×${p.height}`);
  if (p.size) parts.push(formatBytes(p.size));
  return parts.join(' · ');
}

/**
 * A map that keeps its most recently set entries within an entry and a byte
 * budget, so cached data URLs can't pile up.
 */
export class ByteLru<V> {
  private map = new Map<string, { value: V; bytes: number }>();
  private total = 0;

  constructor(
    private maxEntries: number,
    private maxBytes: number,
  ) {}

  get size() {
    return this.map.size;
  }

  get bytes() {
    return this.total;
  }

  has(key: string) {
    return this.map.has(key);
  }

  get(key: string): V | undefined {
    return this.map.get(key)?.value;
  }

  set(key: string, value: V, bytes: number) {
    this.delete(key);
    this.map.set(key, { value, bytes });
    this.total += bytes;
    for (const [k] of this.map) {
      if (this.map.size <= 1 || (this.map.size <= this.maxEntries && this.total <= this.maxBytes)) break;
      this.delete(k);
    }
  }

  delete(key: string) {
    const e = this.map.get(key);
    if (!e) return;
    this.total -= e.bytes;
    this.map.delete(key);
  }
}
