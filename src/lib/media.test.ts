import { describe, expect, it } from 'vitest';
import { ByteLru, fileName, fitBox, formatBytes, isNsfw, mediaLabel } from './media';

describe('media helpers', () => {
  it('fits images inside the thumbnail box without upscaling', () => {
    expect(fitBox(1920, 1080)).toEqual({ width: 360, height: 203 });
    expect(fitBox(600, 1200)).toEqual({ width: 140, height: 280 });
    expect(fitBox(100, 50)).toEqual({ width: 100, height: 50 });
    expect(fitBox(10000, 1)).toEqual({ width: 360, height: 1 });
    expect(fitBox(0, 0)).toEqual({ width: 360, height: 280 });
  });

  it('spots NSFW markers as words', () => {
    expect(isNsfw('NSFW: https://x.example/a.png')).toBe(true);
    expect(isNsfw('(nsfl) look')).toBe(true);
    expect(isNsfw('\x02nsfw\x02 pic')).toBe(true);
    expect(isNsfw('https://x.example/unsfwx.png')).toBe(false);
    expect(isNsfw('safe for work')).toBe(false);
  });

  it('describes files', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(1536)).toBe('1.5 KB');
    expect(formatBytes(12 * 1024 * 1024)).toBe('12 MB');
    expect(mediaLabel({ kind: 'media', mime: 'video/mp4', size: 3 * 1024 * 1024, width: null, height: null })).toBe('Video · MP4 · 3.0 MB');
    expect(mediaLabel({ kind: 'media', mime: 'audio/mpeg', size: null, width: null, height: null })).toBe('Audio · MP3');
    expect(mediaLabel({ kind: 'image', mime: 'image/png', size: 2048, width: 800, height: 600 })).toBe('Image · PNG · 800×600 · 2.0 KB');
    expect(fileName('https://up.example/uploads/abc/My%20clip.webm?x=1')).toBe('My clip.webm');
    expect(fileName('https://up.example/')).toBe('up.example');
  });

  it('bounds a cache by entries and bytes', () => {
    const c = new ByteLru<string>(3, 100);
    c.set('a', 'a', 40);
    c.set('b', 'b', 40);
    c.set('c', 'c', 40);
    expect(c.has('a')).toBe(false);
    expect(c.bytes).toBe(80);
    c.set('d', 'd', 1);
    c.set('e', 'e', 1);
    expect([c.has('b'), c.has('c'), c.has('d'), c.has('e')]).toEqual([false, true, true, true]);
    // An entry bigger than the budget still stays, alone.
    c.set('huge', 'h', 500);
    expect(c.size).toBe(1);
    expect(c.get('huge')).toBe('h');
    c.set('huge', 'h2', 10);
    expect(c.bytes).toBe(10);
  });
});
