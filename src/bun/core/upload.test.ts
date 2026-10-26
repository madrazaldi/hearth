import { afterEach, describe, expect, test } from 'bun:test';
import { defaultServer, type ServerConfig } from './config';
import { guessType, upload } from './upload';

const servers: { stop(force?: boolean): void }[] = [];
afterEach(() => {
  while (servers.length) servers.pop()!.stop(true);
});

interface Seen {
  method: string;
  path: string;
  headers: Headers;
  body: Uint8Array;
}

function host(handler: (req: Request, seen: Seen) => Response | Promise<Response>): { url: string; seen: Seen[] } {
  const seen: Seen[] = [];
  const server = Bun.serve({
    port: 0,
    async fetch(req) {
      const s: Seen = { method: req.method, path: new URL(req.url).pathname, headers: req.headers, body: new Uint8Array(await req.arrayBuffer()) };
      seen.push(s);
      return handler(req, s);
    },
  });
  servers.push(server);
  return { url: `http://127.0.0.1:${server.port}/files/up`, seen };
}

const cfg = (over: Partial<ServerConfig> = {}): ServerConfig => ({ ...defaultServer(), id: 's', nick: 'ember', ...over });

describe('upload', () => {
  test('posts the file and resolves the Location against the endpoint', async () => {
    const h = host(() => new Response(null, { status: 201, headers: { Location: '/files/abc.png' } }));
    const url = await upload(cfg(), h.url, 'a.png', 'image/png', Uint8Array.of(1, 2, 3));
    expect(url).toBe(new URL('/files/abc.png', h.url).toString());
    const s = h.seen[0];
    expect(s.method).toBe('POST');
    expect(s.headers.get('content-type')).toBe('image/png');
    expect(s.headers.get('content-disposition')).toBe('attachment; filename="a.png"');
    expect([...s.body]).toEqual([1, 2, 3]);
    expect(s.headers.get('authorization')).toBeNull();
  });

  test('absolute Location headers are kept', async () => {
    const h = host(() => new Response(null, { status: 201, headers: { Location: 'https://files.example/x/y.png' } }));
    expect(await upload(cfg(), h.url, 'y.png', 'image/png', new Uint8Array())).toBe('https://files.example/x/y.png');
  });

  test('authenticates with the SASL login when there is one, else the server password', async () => {
    const h = host(() => new Response(null, { status: 201, headers: { Location: '/f' } }));
    const basic = (u: string, p: string) => 'Basic ' + Buffer.from(`${u}:${p}`).toString('base64');
    await upload(cfg({ sasl: 'plain', saslUsername: 'acct', saslPassword: 'pw1' }), h.url, 'f', '', new Uint8Array());
    expect(h.seen[0].headers.get('authorization')).toBe(basic('acct', 'pw1'));
    // The SASL username defaults to the nick.
    await upload(cfg({ saslPassword: 'pw2' }), h.url, 'f', '', new Uint8Array());
    expect(h.seen[1].headers.get('authorization')).toBe(basic('ember', 'pw2'));
    await upload(cfg({ username: 'u', password: 'serverpw' }), h.url, 'f', '', new Uint8Array());
    expect(h.seen[2].headers.get('authorization')).toBe(basic('u', 'serverpw'));
    // An empty content type becomes a generic one.
    expect(h.seen[2].headers.get('content-type')).toBe('application/octet-stream');
  });

  test('file names are cleaned of quotes and control characters, and keep their UTF-8 bytes', async () => {
    const h = host(() => new Response(null, { status: 201, headers: { Location: '/f' } }));
    await upload(cfg(), h.url, 'we"ird\r\nna\x00me café☃.txt', 'text/plain', new Uint8Array());
    const raw = h.seen[0].headers.get('content-disposition')!;
    // The server sees the header as Latin-1 text of the UTF-8 bytes.
    expect(Buffer.from(raw, 'latin1').toString('utf8')).toBe('attachment; filename="weirdname café☃.txt"');
  });

  test('a rejection carries the status and the server text', async () => {
    const h = host(() => new Response(' quota exceeded \n', { status: 413, statusText: 'Payload Too Large' }));
    await expect(upload(cfg(), h.url, 'a', '', new Uint8Array())).rejects.toThrow('upload rejected (413 Payload Too Large): quota exceeded');
  });

  test('a missing Location is an error', async () => {
    const h = host(() => new Response(null, { status: 201 }));
    await expect(upload(cfg(), h.url, 'a', '', new Uint8Array())).rejects.toThrow('server did not return a file URL');
  });

  test('redirects are not followed, so credentials stay put', async () => {
    const other = host(() => new Response(null, { status: 201, headers: { Location: '/stolen' } }));
    const h = host(() => new Response(null, { status: 307, headers: { Location: other.url } }));
    await expect(upload(cfg({ saslPassword: 'pw' }), h.url, 'a', '', new Uint8Array())).rejects.toThrow('upload rejected (307');
    expect(other.seen).toHaveLength(0);
  });

  test('connection failures are labelled', async () => {
    const h = host(() => new Response());
    const url = h.url;
    servers.pop()!.stop(true);
    await expect(upload(cfg(), url, 'a', '', new Uint8Array())).rejects.toThrow('upload failed:');
  });
});

describe('guessType', () => {
  test('maps known extensions, case-insensitively, and defaults to binary', () => {
    expect(guessType('a.PNG')).toBe('image/png');
    expect(guessType('a.jpeg')).toBe('image/jpeg');
    expect(guessType('notes.log')).toBe('text/plain; charset=utf-8');
    expect(guessType('clip.webm')).toBe('video/webm');
    expect(guessType('x.tar.gz')).toBe('application/octet-stream');
    expect(guessType('noext')).toBe('application/octet-stream');
  });
});
