import { afterEach, describe, expect, test } from 'bun:test';
import { loopbackNet, respond, serve, type Fixture } from './http-fixture';
import { safeGet } from './net';
import { Cache, clip, decodeEntities, gifFrames, imageDataUrl, inspectImage, isApng, isGenericBinary, MAX_INLINE_IMAGE, parseMeta, Previewer, readImage } from './preview';

const fixtures: Fixture[] = [];
const fx = async (...args: Parameters<typeof serve>) => {
  const f = await serve(...args);
  fixtures.push(f);
  return f;
};
afterEach(() => {
  while (fixtures.length) fixtures.pop()!.close();
});

const bytes = (...parts: (string | number[] | Uint8Array)[]) =>
  Uint8Array.from(parts.flatMap((p) => (typeof p === 'string' ? [...p].map((c) => c.charCodeAt(0)) : [...p])));
const be32 = (n: number) => [(n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255];
const le16 = (n: number) => [n & 255, n >> 8];

function png(w: number, h: number, extra: Uint8Array = new Uint8Array()): Uint8Array {
  return bytes('\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR', be32(w), be32(h), [8, 6, 0, 0, 0, 0, 0, 0, 0], extra, '\0\0\0\0IDAT\0\0\0\0');
}

function gif(frames: number): Uint8Array {
  // 2x1, global colour table of 2 entries, NETSCAPE loop extension.
  const parts: (string | number[])[] = ['GIF89a\x02\0\x01\0\x80\0\0\0\0\0\xff\xff\xff', '\x21\xff\x0bNETSCAPE2.0\x03\x01\0\0\0'];
  for (let i = 0; i < frames; i++) parts.push('\x21\xf9\x04\0\x0a\0\0\0', '\x2c\0\0\0\0\x02\0\x01\0\0\x02\x02\x44\x01\0');
  parts.push('\x3b');
  return bytes(...parts);
}

function jpeg(w: number, h: number): Uint8Array {
  const app0 = [0xff, 0xe0, 0, 16, ...bytes('JFIF\0'), 1, 1, 0, 0, 1, 0, 1, 0, 0];
  const dht = [0xff, 0xc4, 0, 4, 0, 0]; // must not be mistaken for a frame header
  const sof = [0xff, 0xc0, 0, 17, 8, h >> 8, h & 255, w >> 8, w & 255, 3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1];
  return bytes([0xff, 0xd8], app0, dht, sof, [0xff, 0xda, 0, 2]);
}

function webp(kind: 'VP8 ' | 'VP8L' | 'VP8X', w: number, h: number, animated = false): Uint8Array {
  let chunk: number[];
  if (kind === 'VP8X') chunk = [animated ? 2 : 0, 0, 0, 0, (w - 1) & 255, ((w - 1) >> 8) & 255, (w - 1) >> 16, (h - 1) & 255, ((h - 1) >> 8) & 255, (h - 1) >> 16];
  else if (kind === 'VP8L') {
    const bits = ((w - 1) | ((h - 1) << 14)) >>> 0;
    chunk = [0x2f, bits & 255, (bits >> 8) & 255, (bits >> 16) & 255, bits >>> 24, 0, 0, 0];
  } else chunk = [0, 0, 0, 0x9d, 0x01, 0x2a, ...le16(w), ...le16(h), 0, 0];
  return bytes('RIFF', [0, 0, 0, 0], 'WEBP', kind, [chunk.length, 0, 0, 0], chunk);
}

describe('page metadata', () => {
  test('extracts_open_graph', () => {
    const html = `<html><head><title>Plain &amp; simple</title>
            <meta property="og:title" content="Hearth &#8212; IRC">
            <meta name=description content='A quiet client'>
            <META PROPERTY="og:image" CONTENT="/img.png"/>
            </head><body><meta property="og:title" content="ignored"></body></html>`;
    const m = parseMeta(html);
    expect(m.get('og:title')).toBe('Hearth — IRC');
    expect(m.get('description')).toBe('A quiet client');
    expect(m.get('og:image')).toBe('/img.png');
    expect(m.get('<title>')).toBe('Plain & simple');
  });

  test('decodes entities, leaving unknown ones alone', () => {
    expect(decodeEntities('a &lt;b&gt; &quot;c&quot; &apos;d&apos;&nbsp;e &#x41;&#66; &bogus; & &#xD800; &#1114112;')).toBe('a <b> "c" \'d\' e AB &bogus; & &#xD800; &#1114112;');
    expect(decodeEntities('no entities')).toBe('no entities');
  });

  test('clips_and_rejects_svg', () => {
    expect(clip('a   b\n c', 10)).toBe('a b c');
    expect(clip('abcdef', 4)).toBe('abc…');
    expect(() => imageDataUrl('image/svg+xml', Uint8Array.of(1))).toThrow();
    expect(imageDataUrl('image/png; charset=binary', Uint8Array.of(1))).toStartWith('data:image/png;base64,');
  });
});

describe('cache', () => {
  test('cache_forgets_the_oldest_entries', () => {
    const c = new Cache<number>(2);
    c.insert('a', 1);
    c.insert('b', null);
    c.insert('a', 2);
    expect(c.get('a')).toBe(2);
    expect(c.get('b')).toBeNull(); // failures are remembered
    c.insert('c', 3);
    expect(c.get('a')).toBeUndefined();
    expect(c.get('c')).toBe(3);
  });
});

describe('image inspection', () => {
  test('sniffs_images_by_content', () => {
    const img = inspectImage(png(640, 480));
    expect([img.mime, img.width, img.height, img.animated]).toEqual(['image/png', 640, 480, false]);
    expect(img.dataUrl).toStartWith('data:image/png;base64,');
    // An APNG announces its animation before the image data.
    expect(inspectImage(png(8, 8, bytes('\0\0\0\x08acTL\0\0\0\x02\0\0\0\0\0\0\0\0'))).animated).toBe(true);

    const still = inspectImage(gif(1));
    expect([still.mime, still.width, still.height, still.animated]).toEqual(['image/gif', 2, 1, false]);
    expect(inspectImage(gif(3)).animated).toBe(true);

    // Whatever the server claims, SVG and HTML are never passed on.
    expect(() => inspectImage(bytes("<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>"))).toThrow();
    expect(() => inspectImage(bytes('<!doctype html><html><body>hi</body></html>'))).toThrow();
    expect(isGenericBinary('application/octet-stream') && !isGenericBinary('text/html')).toBe(true);
  });

  test('reads JPEG and WebP dimensions', () => {
    const j = inspectImage(jpeg(1024, 768));
    expect([j.mime, j.width, j.height, j.animated]).toEqual(['image/jpeg', 1024, 768, false]);
    for (const kind of ['VP8 ', 'VP8L', 'VP8X'] as const) {
      const w = inspectImage(webp(kind, 321, 123));
      expect([w.mime, w.width, w.height, w.animated], kind).toEqual(['image/webp', 321, 123, false]);
    }
    expect(inspectImage(webp('VP8X', 10, 10, true)).animated).toBe(true);
  });

  test('truncated headers are unreadable, not a crash', () => {
    expect(() => inspectImage(png(1, 1).subarray(0, 20))).toThrow('unreadable image');
    expect(() => inspectImage(jpeg(10, 10).subarray(0, 8))).toThrow('unreadable image');
    expect(() => inspectImage(webp('VP8X', 5, 5).subarray(0, 25))).toThrow('unreadable image');
    expect(gifFrames(gif(2).subarray(0, 12))).toBe(0);
    expect(isApng(png(1, 1).subarray(0, 12))).toBe(false);
  });

  test('rejects_absurd_dimensions', () => {
    expect(() => inspectImage(png(0, 10))).toThrow();
    expect(() => inspectImage(png(20_000, 10))).toThrow();
    expect(() => inspectImage(png(10_000, 10_000))).toThrow();
    expect(() => inspectImage(png(4000, 3000))).not.toThrow();
  });
});

// `expect(...).rejects` stalls socket reads in bun:test while a large body is in
// flight, so this waits for the rejection itself.
const failure = (p: Promise<unknown>) =>
  p.then(
    () => {
      throw new Error('expected a rejection');
    },
    (e: unknown) => String(e),
  );

describe('size caps', () => {
  test('caps_image_size', async () => {
    const big = new Uint8Array(MAX_INLINE_IMAGE + 1);
    // Declared too large: refused before reading.
    const declared = await fx((s) => respond(s, `HTTP/1.1 200 OK\nContent-Type: image/png\nContent-Length: ${big.length}`));
    expect(await failure(readImage(await safeGet(declared.url(), loopbackNet), MAX_INLINE_IMAGE))).toContain('image too large');
    // Undeclared: refused once the stream passes the cap.
    const body = new Uint8Array(MAX_INLINE_IMAGE + 1);
    body.set(png(10, 10));
    const undeclared = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Type: image/png\nConnection: close', body));
    expect(await failure(readImage(await safeGet(undeclared.url(), loopbackNet), MAX_INLINE_IMAGE))).toContain('image too large');
    // A mislabelled non-image is dropped after its first bytes.
    const junk = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Type: application/octet-stream\nConnection: close', 'P'.repeat(4096)));
    await expect(readImage(await safeGet(junk.url(), loopbackNet), MAX_INLINE_IMAGE)).rejects.toThrow('unsupported');
  });

  test('a mislabelled upload is dropped after its first bytes, not downloaded', async () => {
    let closed = false;
    const f = await fx((s) => {
      s.on('close', () => (closed = true));
      s.write('HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\r\n' + 'P'.repeat(64));
    });
    await expect(readImage(await safeGet(f.url(), loopbackNet), MAX_INLINE_IMAGE)).rejects.toThrow('unsupported');
    await Bun.sleep(50);
    expect(closed).toBe(true);
  });
});

describe('Previewer', () => {
  test('avatars_refuse_private_addresses', async () => {
    const p = new Previewer();
    for (const url of ['http://127.0.0.1/a.png', 'http://localhost/a.png', 'file:///etc/passwd', 'http://10.1.2.3/x.png']) {
      await expect(p.avatar(url), url).rejects.toThrow();
    }
  });

  test('refuses_private_hops: the first hop is checked before anything is sent', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 302 Found\nLocation: http://10.0.0.1/\nContent-Length: 0'));
    await expect(new Previewer().preview(`http://127.0.0.1:${f.port}/`)).rejects.toThrow('private');
    expect(f.connections).toBe(0);
    // And with loopback allowed, the redirect target is what stops it.
    await expect(new Previewer(loopbackNet).preview(f.url().href)).rejects.toThrow('private');
  });

  test('previews a page with Open Graph metadata and an image', async () => {
    const img = png(640, 480);
    const f = await fx((s, req) => {
      if (req.startsWith('GET /img.png')) respond(s, `HTTP/1.1 200 OK\nContent-Type: image/png\nContent-Length: ${img.length}`, img);
      else {
        const html = `<html><head><title>T</title><meta property="og:title" content="  Hearth &#8212; IRC "><meta property="og:description" content="A quiet client"><meta property="og:image" content="/img.png"></head><body></body></html>`;
        respond(s, `HTTP/1.1 200 OK\nContent-Type: text/html; charset=utf-8\nContent-Length: ${Buffer.byteLength(html)}`, html);
      }
    });
    const p = new Previewer(loopbackNet);
    const url = f.url('/page', 'www.example.com').href;
    const pv = await p.preview(url);
    expect(pv).toMatchObject({
      url,
      kind: 'page',
      title: 'Hearth — IRC',
      description: 'A quiet client',
      site: 'example.com',
      width: 640,
      height: 480,
      animated: false,
      mime: null,
      size: null,
    });
    expect(pv.image).toStartWith('data:image/png;base64,');
    // The second call comes from the cache.
    const before = f.connections;
    expect(await p.preview(url)).toEqual(pv);
    expect(f.connections).toBe(before);
  });

  test('previews a direct image link, labelled or not', async () => {
    const img = gif(2);
    const f = await fx((s, req) => {
      const ct = req.startsWith('GET /raw') ? 'application/octet-stream' : 'image/gif';
      respond(s, `HTTP/1.1 200 OK\nContent-Type: ${ct}\nContent-Length: ${img.length}`, img);
    });
    const p = new Previewer(loopbackNet);
    for (const path of ['/cat.gif', '/raw']) {
      const pv = await p.preview(f.url(path).href);
      expect(pv).toMatchObject({ kind: 'image', width: 2, height: 1, animated: true, mime: 'image/gif', size: img.length, title: null });
    }
  });

  test('describes media without downloading it', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Type: video/mp4\nContent-Length: 18400000'));
    const pv = await new Previewer(loopbackNet).preview(f.url('/v.mp4').href);
    expect(pv).toMatchObject({ kind: 'media', mime: 'video/mp4', size: 18_400_000, image: null });
  });

  test('failures are cached too', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Type: application/zip\nContent-Length: 2', 'PK'));
    const p = new Previewer(loopbackNet);
    const url = f.url().href;
    await expect(p.preview(url)).rejects.toThrow('nothing to preview');
    const seen = f.connections;
    await expect(p.preview(url)).rejects.toThrow('no preview');
    expect(f.connections).toBe(seen);
  });

  test('a page with nothing to show, and a missing page, are errors', async () => {
    const f = await fx((s, req) =>
      req.startsWith('GET /empty')
        ? respond(s, 'HTTP/1.1 200 OK\nContent-Type: text/html\nContent-Length: 13', '<html></html>')
        : respond(s, 'HTTP/1.1 404 Not Found\nContent-Length: 0'),
    );
    const p = new Previewer(loopbackNet);
    await expect(p.preview(f.url('/empty').href)).rejects.toThrow('nothing to preview');
    await expect(p.preview(f.url('/missing').href)).rejects.toThrow('HTTP status 404');
    await expect(p.preview('not a url')).rejects.toThrow('invalid URL');
  });

  test('a blank first metadata key hides the later ones, as before', async () => {
    const html = '<head><meta property="og:title" content=" "><meta name="twitter:title" content="Other"><title>Fallback</title></head>';
    const f = await fx((s) => respond(s, `HTTP/1.1 200 OK\nContent-Type: text/html\nContent-Length: ${html.length}`, html));
    // Nothing else to show, so the blank og:title makes the whole preview fail.
    await expect(new Previewer(loopbackNet).preview(f.url().href)).rejects.toThrow('nothing to preview');
  });

  test('fetches avatars as data URLs and rejects non-images', async () => {
    const f = await fx((s, req) =>
      req.startsWith('GET /ok')
        ? respond(s, 'HTTP/1.1 200 OK\nContent-Type: image/png', png(40, 40))
        : respond(s, 'HTTP/1.1 200 OK\nContent-Type: image/png', '<svg/>'),
    );
    const p = new Previewer(loopbackNet);
    expect(await p.avatar(f.url('/ok').href)).toStartWith('data:image/png;base64,');
    await expect(p.avatar(f.url('/bad').href)).rejects.toThrow();
    await expect(p.avatar(f.url('/bad').href)).rejects.toThrow('no avatar');
  });
});
