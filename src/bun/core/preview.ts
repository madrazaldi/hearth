// Link previews, fetched by the backend so the webview never talks to
// third-party hosts. Images come back as data URLs.
//
// Avatars (draft/metadata-2 `avatar` URLs) are fetched the same way, with a
// smaller size limit.
//
// Only public addresses are fetched (see net.ts): a link in a channel must not
// be able to make Hearth probe the user's LAN or localhost.

import { errorForStatus, safeGet, type HttpResponse, type NetOptions } from './net';

const MAX_HTML = 512 * 1024;
const MAX_IMAGE = 4 * 1024 * 1024;
const CACHE = 300;
const MAX_AVATAR = 1024 * 1024;
const AVATAR_CACHE = 1000;
/** Direct image links can be bigger than page thumbnails. */
const MAX_INLINE_IMAGE = 8 * 1024 * 1024;
/** What the webview is asked to decode: no absurd sides or pixel counts. */
const MAX_SIDE = 16_384;
const MAX_PIXELS = 50_000_000;
/** Total data-URL bytes the cache may hold, on top of its entry cap. */
const CACHE_BYTES = 64 * 1024 * 1024;

export { MAX_INLINE_IMAGE };

export interface Preview {
  url: string;
  /** "image" for a direct image link, "page" for an HTML page, "media" for a video or audio link (never downloaded, only described). */
  kind: string;
  title: string | null;
  description: string | null;
  site: string | null;
  /** A data: URL. */
  image: string | null;
  /** Pixel size of `image`, so the view can reserve its space up front. */
  width: number | null;
  height: number | null;
  /** `image` has more than one frame. */
  animated: boolean;
  /** Content type of a direct image, video or audio link. */
  mime: string | null;
  /** Byte size of a direct link, when known. */
  size: number | null;
}

/** A fetched image whose bytes were sniffed and measured. */
export interface Inline {
  dataUrl: string;
  mime: string;
  width: number;
  height: number;
  animated: boolean;
}

function emptyPreview(url: string): Preview {
  return { url, kind: '', title: null, description: null, site: null, image: null, width: null, height: null, animated: false, mime: null, size: null };
}

/**
 * A bounded map that forgets its oldest entries. Failures (null) are cached
 * too, so a dead URL isn't fetched again every time it's shown.
 */
export class Cache<T> {
  readonly map = new Map<string, T | null>();
  readonly order: string[] = [];
  constructor(private readonly cap: number) {}

  /** `undefined` is a miss; `null` is a remembered failure. */
  get(key: string): T | null | undefined {
    return this.map.get(key);
  }

  insert(key: string, value: T | null) {
    if (!this.map.has(key)) this.order.push(key);
    this.map.set(key, value);
    while (this.order.length > this.cap) this.evictOldest();
  }

  evictOldest() {
    const old = this.order.shift();
    if (old !== undefined) this.map.delete(old);
  }
}

function cachedBytes(c: Cache<Preview>): number {
  let n = 0;
  for (const p of c.map.values()) n += p?.image?.length ?? 0;
  return n;
}

export class Previewer {
  private readonly cache = new Cache<Preview>(CACHE);
  private readonly avatars = new Cache<string>(AVATAR_CACHE);

  /** `net` is for tests, which serve from loopback. */
  constructor(private readonly net: NetOptions = {}) {}

  async preview(url: string): Promise<Preview> {
    const hit = this.cache.get(url);
    if (hit !== undefined) {
      if (hit === null) throw new Error('no preview');
      return { ...hit };
    }
    let result: Preview | null = null;
    try {
      result = await this.fetch(url);
      return result;
    } finally {
      this.cache.insert(url, result);
      while (this.cache.order.length > 1 && cachedBytes(this.cache) > CACHE_BYTES) this.cache.evictOldest();
    }
  }

  /** A user's avatar image as a data: URL. */
  async avatar(url: string): Promise<string> {
    const hit = this.avatars.get(url);
    if (hit !== undefined) {
      if (hit === null) throw new Error('no avatar');
      return hit;
    }
    let result: string | null = null;
    try {
      result = await this.fetchAvatar(url);
      return result;
    } finally {
      this.avatars.insert(url, result);
    }
  }

  private get(url: URL): Promise<HttpResponse> {
    return safeGet(url, this.net);
  }

  private async fetchAvatar(url: string): Promise<string> {
    const resp = errorForStatus(await this.get(parseUrl(url)));
    return inspectImage(await readImage(resp, MAX_AVATAR)).dataUrl;
  }

  private async fetch(url: string): Promise<Preview> {
    const resp = errorForStatus(await this.get(parseUrl(url)));
    const ctype = resp.contentType;
    const mime = ctype.split(';')[0].trim();
    if (mime.startsWith('video/') || mime.startsWith('audio/')) {
      // The CSP keeps remote media out of the webview; describe it instead.
      resp.close();
      return { ...emptyPreview(url), kind: 'media', mime, size: resp.contentLength };
    }
    if (mime.startsWith('image/') || isGenericBinary(mime)) {
      const bytes = await readImage(resp, MAX_INLINE_IMAGE);
      const img = inspectImage(bytes);
      return { ...emptyPreview(url), kind: 'image', image: img.dataUrl, width: img.width, height: img.height, animated: img.animated, mime: img.mime, size: bytes.length };
    }
    if (!ctype.startsWith('text/html') && !ctype.startsWith('application/xhtml')) {
      resp.close();
      throw new Error('nothing to preview');
    }
    const body = await readLimited(resp, MAX_HTML);
    const meta = parseMeta(new TextDecoder().decode(body));
    // The first key present wins, even if its value is blank.
    const pick = (keys: string[]) => {
      const v = keys.map((k) => meta.get(k)).find((v) => v !== undefined);
      return v !== undefined && v.trim() !== '' ? v : null;
    };
    const title = pick(['og:title', 'twitter:title', '<title>']);
    const description = pick(['og:description', 'twitter:description', 'description']);
    const site = pick(['og:site_name']) ?? (resp.url.hostname ? resp.url.hostname.replace(/^www\./, '') : null);
    let image: Inline | null = null;
    const src = pick(['og:image:secure_url', 'og:image', 'twitter:image', 'twitter:image:src']);
    if (src) {
      try {
        image = await this.fetchImageInfo(new URL(src, resp.url), MAX_IMAGE);
      } catch {}
    }
    if (title === null && description === null && image === null) throw new Error('nothing to preview');
    return {
      ...emptyPreview(url),
      kind: 'page',
      title: title === null ? null : clip(title, 200),
      description: description === null ? null : clip(description, 400),
      site,
      image: image?.dataUrl ?? null,
      width: image?.width ?? null,
      height: image?.height ?? null,
      animated: image?.animated ?? false,
    };
  }

  /** Fetch an image as a data URL: raster formats only, sniffed from the bytes. */
  async fetchImage(url: URL): Promise<string> {
    return (await this.fetchImageInfo(url, MAX_IMAGE)).dataUrl;
  }

  async fetchImageInfo(url: URL, limit: number): Promise<Inline> {
    const resp = errorForStatus(await this.get(url));
    const mime = resp.contentType.split(';')[0].trim();
    if (!mime.startsWith('image/') && !isGenericBinary(mime)) {
      resp.close();
      throw new Error('not an image');
    }
    return inspectImage(await readImage(resp, limit));
  }
}

function parseUrl(url: string): URL {
  try {
    return new URL(url);
  } catch {
    throw new Error('invalid URL');
  }
}

/**
 * Servers (soju's FILEHOST among them) often label uploads this way; the
 * bytes decide whether they're an image.
 */
export function isGenericBinary(mime: string): boolean {
  return ['', 'application/octet-stream', 'binary/octet-stream', 'application/binary'].includes(mime);
}

function concat(parts: Uint8Array[], total: number): Uint8Array {
  const out = new Uint8Array(total);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

/**
 * Reads a body, but gives up after the first bytes if they aren't an image
 * we'd show, so a mislabelled archive isn't downloaded for nothing.
 */
export async function readImage(resp: HttpResponse, limit: number): Promise<Uint8Array> {
  if (resp.contentLength !== null && resp.contentLength > limit) {
    resp.close();
    throw new Error('image too large');
  }
  const parts: Uint8Array[] = [];
  let total = 0;
  let sniffed = false;
  for await (const chunk of resp.chunks()) {
    parts.push(chunk);
    total += chunk.length;
    if (total > limit) throw new Error('image too large');
    if (!sniffed && total >= 32) {
      sniffed = true;
      sniff(concat(parts, total));
    }
  }
  return concat(parts, total);
}

export async function readLimited(resp: HttpResponse, limit: number): Promise<Uint8Array> {
  if (resp.contentLength !== null && resp.contentLength > limit) {
    resp.close();
    throw new Error('response too large');
  }
  const parts: Uint8Array[] = [];
  let total = 0;
  for await (const chunk of resp.chunks()) {
    parts.push(chunk);
    total += chunk.length;
    if (total > limit) {
      // HTML metadata lives in the head; a truncated page is still useful.
      if (limit === MAX_HTML) return concat(parts, total).subarray(0, limit);
      throw new Error('response too large');
    }
  }
  return concat(parts, total);
}

// ---- Image sniffing ----

const startsWith = (b: Uint8Array, s: string, at = 0) => b.length >= at + s.length && [...s].every((c, i) => b[at + i] === c.charCodeAt(0));

export function sniff(b: Uint8Array): string {
  if (b.length >= 8 && b[0] === 0x89 && startsWith(b, 'PNG\r\n\x1a\n', 1)) return 'image/png';
  if (b.length >= 3 && b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return 'image/jpeg';
  if (startsWith(b, 'GIF87a') || startsWith(b, 'GIF89a')) return 'image/gif';
  if (startsWith(b, 'RIFF') && startsWith(b, 'WEBP', 8)) return 'image/webp';
  throw new Error('unsupported image type');
}

const u16be = (b: Uint8Array, i: number) => (b[i] << 8) | b[i + 1];
const u16le = (b: Uint8Array, i: number) => b[i] | (b[i + 1] << 8);
const u24le = (b: Uint8Array, i: number) => b[i] | (b[i + 1] << 8) | (b[i + 2] << 16);
const u32be = (b: Uint8Array, i: number) => ((b[i] << 24) | (b[i + 1] << 16) | (b[i + 2] << 8) | b[i + 3]) >>> 0;

/** Pixel size from the format's header; replaces a crate for four formats. */
function dimensions(b: Uint8Array, mime: string): [number, number] | null {
  switch (mime) {
    case 'image/png':
      return b.length >= 24 && startsWith(b, 'IHDR', 12) ? [u32be(b, 16), u32be(b, 20)] : null;
    case 'image/gif':
      return b.length >= 10 ? [u16le(b, 6), u16le(b, 8)] : null;
    case 'image/webp': {
      if (startsWith(b, 'VP8X', 12)) return b.length >= 30 ? [u24le(b, 24) + 1, u24le(b, 27) + 1] : null;
      if (startsWith(b, 'VP8L', 12)) {
        if (b.length < 25 || b[20] !== 0x2f) return null;
        const bits = (b[21] | (b[22] << 8) | (b[23] << 16) | (b[24] << 24)) >>> 0;
        return [(bits & 0x3fff) + 1, ((bits >>> 14) & 0x3fff) + 1];
      }
      if (startsWith(b, 'VP8 ', 12)) {
        return b.length >= 30 && b[23] === 0x9d && b[24] === 0x01 && b[25] === 0x2a ? [u16le(b, 26) & 0x3fff, u16le(b, 28) & 0x3fff] : null;
      }
      return null;
    }
    case 'image/jpeg': {
      let i = 2;
      while (i + 4 <= b.length) {
        if (b[i] !== 0xff) return null;
        const marker = b[i + 1];
        if (marker === 0xff) {
          i++;
          continue;
        }
        if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) {
          i += 2;
          continue;
        }
        if (marker === 0xd9 || marker === 0xda) return null;
        const isSof = marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
        if (isSof) return i + 9 <= b.length ? [u16be(b, i + 7), u16be(b, i + 5)] : null;
        i += 2 + u16be(b, i + 2);
      }
      return null;
    }
  }
  return null;
}

/**
 * Check that the bytes really are a PNG, JPEG, GIF or WebP of sane size. The
 * declared content type is ignored: it's the bytes the webview decodes.
 */
export function inspectImage(bytes: Uint8Array): Inline {
  const mime = sniff(bytes);
  const size = dimensions(bytes, mime);
  if (!size) throw new Error('unreadable image');
  const [w, h] = size;
  if (w === 0 || h === 0 || w > MAX_SIDE || h > MAX_SIDE || w * h > MAX_PIXELS) throw new Error('image dimensions out of range');
  const animated =
    mime === 'image/gif'
      ? gifFrames(bytes) > 1
      : mime === 'image/webp'
        ? bytes.length > 20 && startsWith(bytes, 'VP8X', 12) && (bytes[20] & 0x02) !== 0
        : mime === 'image/png'
          ? isApng(bytes)
          : false;
  return { dataUrl: imageDataUrl(mime, bytes), mime, width: w, height: h, animated };
}

/** Image frames in a GIF, counting no further than two. */
export function gifFrames(b: Uint8Array): number {
  const skipBlocks = (start: number): number | null => {
    let i = start;
    for (;;) {
      if (i >= b.length) return null;
      const n = b[i];
      i += 1 + n;
      if (n === 0) return i;
    }
  };
  // Header and logical screen descriptor, then the global colour table.
  if (b.length <= 10) return 0;
  const flags = b[10];
  let i = 13;
  if (flags & 0x80) i += 3 << ((flags & 7) + 1);
  let frames = 0;
  while (i < b.length) {
    const t = b[i];
    let next: number | null;
    if (t === 0x21) next = skipBlocks(i + 2);
    else if (t === 0x2c) {
      frames++;
      if (i + 9 >= b.length) break;
      const f = b[i + 9];
      const table = f & 0x80 ? 3 << ((f & 7) + 1) : 0;
      // Descriptor, local colour table, LZW code size, data.
      next = skipBlocks(i + 10 + table + 1);
    } else next = null;
    if (next !== null && frames < 2) i = next;
    else break;
  }
  return frames;
}

/** An animated PNG has an `acTL` chunk before its first `IDAT`. */
export function isApng(b: Uint8Array): boolean {
  let i = 8;
  while (i + 8 <= b.length) {
    const len = u32be(b, i);
    if (startsWith(b, 'acTL', i + 4)) return true;
    if (startsWith(b, 'IDAT', i + 4) || startsWith(b, 'IEND', i + 4)) return false;
    i += 12 + len;
  }
  return false;
}

export function imageDataUrl(ctype: string, bytes: Uint8Array): string {
  const mime = ctype.split(';')[0].trim();
  // SVG can carry script and external references; raster formats only.
  if (!['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/avif'].includes(mime)) throw new Error('unsupported image type');
  return `data:${mime};base64,${Buffer.from(bytes.buffer, bytes.byteOffset, bytes.length).toString('base64')}`;
}

// ---- Text helpers ----

export function clip(s: string, max: number): string {
  const t = s.split(/\s+/).filter(Boolean).join(' ');
  const chars = [...t];
  if (chars.length <= max) return t;
  return chars.slice(0, max - 1).join('') + '…';
}

/**
 * Pull `<meta property|name=… content=…>` pairs and the `<title>` out of HTML.
 * Keys are lowercased; the title is stored under `<title>`.
 */
export function parseMeta(html: string): Map<string, string> {
  const out = new Map<string, string>();
  // Lowercasing only ASCII keeps offsets aligned with `html`.
  const lower = html.replace(/[A-Z]/g, (c) => c.toLowerCase());
  // Metadata is in <head>; stop at <body> to avoid scanning huge pages.
  const bodyAt = lower.indexOf('<body');
  const end = bodyAt < 0 ? lower.length : bodyAt;
  let pos = 0;
  for (;;) {
    const i = lower.indexOf('<meta', pos);
    if (i < 0 || i >= end) break;
    const close = lower.indexOf('>', i);
    if (close < 0 || close >= end) break;
    const attrs = parseAttrs(html.slice(i + 5, close));
    const key = attrs.get('property') ?? attrs.get('name');
    const content = attrs.get('content');
    if (key !== undefined && content !== undefined) {
      const k = key.replace(/[A-Z]/g, (c) => c.toLowerCase());
      if (!out.has(k)) out.set(k, decodeEntities(content));
    }
    pos = close;
  }
  const t = lower.indexOf('<title');
  if (t >= 0 && t < end) {
    const gt = lower.indexOf('>', t);
    if (gt >= 0 && gt < end) {
      const from = gt + 1;
      const stop = lower.indexOf('</title', from);
      if (stop >= 0 && stop < end) out.set('<title>', decodeEntities(html.slice(from, stop).trim()));
    }
  }
  return out;
}

const isSpace = (c: string) => c === ' ' || c === '\t' || c === '\n' || c === '\r' || c === '\f';

export function parseAttrs(tag: string): Map<string, string> {
  const out = new Map<string, string>();
  let i = 0;
  const n = tag.length;
  while (i < n) {
    while (i < n && (isSpace(tag[i]) || tag[i] === '/')) i++;
    const ks = i;
    while (i < n && !isSpace(tag[i]) && tag[i] !== '=' && tag[i] !== '/') i++;
    const key = tag.slice(ks, i).replace(/[A-Z]/g, (c) => c.toLowerCase());
    while (i < n && isSpace(tag[i])) i++;
    if (i < n && tag[i] === '=') {
      i++;
      while (i < n && isSpace(tag[i])) i++;
      let value: string;
      if (i < n && (tag[i] === '"' || tag[i] === "'")) {
        const q = tag[i++];
        const vs = i;
        while (i < n && tag[i] !== q) i++;
        value = tag.slice(vs, i);
        i++;
      } else {
        const vs = i;
        while (i < n && !isSpace(tag[i])) i++;
        value = tag.slice(vs, i);
      }
      if (key) out.set(key, value);
    } else if (!key) i++;
  }
  return out;
}

const NAMED: Record<string, string> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ' };

export function decodeEntities(s: string): string {
  let out = '';
  let rest = s;
  for (;;) {
    const i = rest.indexOf('&');
    if (i < 0) break;
    out += rest.slice(0, i);
    rest = rest.slice(i);
    const semi = rest.slice(0, 12).indexOf(';');
    if (semi < 0) {
      out += '&';
      rest = rest.slice(1);
      continue;
    }
    const name = rest.slice(1, semi);
    let decoded: string | undefined = NAMED[name];
    if (decoded === undefined) {
      const hex = /^#[xX]([0-9a-fA-F]+)$/.exec(name);
      const dec = /^#(\d+)$/.exec(name);
      const cp = hex ? parseInt(hex[1], 16) : dec ? parseInt(dec[1], 10) : NaN;
      // Surrogates and out-of-range values aren't characters.
      if (Number.isInteger(cp) && cp <= 0x10ffff && !(cp >= 0xd800 && cp <= 0xdfff)) decoded = String.fromCodePoint(cp);
    }
    if (decoded === undefined) {
      out += '&';
      rest = rest.slice(1);
    } else {
      out += decoded;
      rest = rest.slice(semi + 1);
    }
  }
  return out + rest;
}
