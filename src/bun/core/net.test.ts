import { afterEach, describe, expect, test } from 'bun:test';
import { createServer as tlsServer } from 'node:tls';
import { selfSigned } from './certs';
import { loopbackNet, respond, serve, type Fixture } from './http-fixture';
import { checkPublic, errorForStatus, hopAllowed, isLocalName, isPublic, safeGet } from './net';

const fixtures: Fixture[] = [];
const fx = async (...args: Parameters<typeof serve>) => {
  const f = await serve(...args);
  fixtures.push(f);
  return f;
};
afterEach(() => {
  while (fixtures.length) fixtures.pop()!.close();
});

async function readAll(chunks: AsyncIterable<Uint8Array>): Promise<string> {
  const parts: Buffer[] = [];
  for await (const c of chunks) parts.push(Buffer.from(c));
  return Buffer.concat(parts).toString();
}

describe('address classification', () => {
  test('classifies_addresses', () => {
    for (const ip of ['10.0.0.1', '127.0.0.1', '192.168.1.1', '169.254.1.1', '100.64.0.1', '::1', 'fd00::1', 'fe80::1', '::ffff:10.0.0.1']) {
      expect(isPublic(ip)).toBe(false);
    }
    for (const ip of ['1.1.1.1', '93.184.216.34', '2606:4700::1111']) expect(isPublic(ip)).toBe(true);
    expect(isLocalName('localhost')).toBe(true);
    expect(isLocalName('printer.local')).toBe(true);
    expect(isLocalName('router')).toBe(true);
    expect(isLocalName('example.com')).toBe(false);
  });

  test('covers the rest of the reserved ranges', () => {
    const private4 = ['0.0.0.0', '0.1.2.3', '172.16.0.1', '172.31.255.255', '192.0.0.1', '192.0.2.1', '198.51.100.7', '203.0.113.9', '224.0.0.1', '240.0.0.1', '255.255.255.255'];
    for (const ip of private4) expect(isPublic(ip), ip).toBe(false);
    for (const ip of ['172.15.0.1', '172.32.0.1', '100.63.255.255', '100.128.0.1', '8.8.8.8']) expect(isPublic(ip), ip).toBe(true);
  });

  test('sees through IPv4-mapped and embedded IPv6 forms', () => {
    const private6 = [
      '::',
      '::ffff:0a00:0001', // ::ffff:10.0.0.1 in hex
      '::ffff:7f00:1',
      '0:0:0:0:0:ffff:192.168.0.1',
      '::10.0.0.1', // IPv4-compatible
      '64:ff9b::10.0.0.1', // NAT64
      '64:ff9b::7f00:1',
      '2002:0a00:0001::', // 6to4 embedding 10.0.0.1
      '2002:7f00:1::1',
      '2001:db8::1',
      '2001:0:4136:e378:8000:63bf:3fff:fdd2', // Teredo
      'fc00::1',
      'fe80::1%eth0',
      'fec0::1',
      'ff02::1',
    ];
    for (const ip of private6) expect(isPublic(ip), ip).toBe(false);
    for (const ip of ['::ffff:8.8.8.8', '64:ff9b::808:808', '2002:0808:0808::1', '2606:4700::1111']) expect(isPublic(ip), ip).toBe(true);
  });

  test('refuses what it cannot parse', () => {
    for (const s of ['', 'garbage', '1.2.3', '1.2.3.256', '1::2::3', ':::', '12345::1', 'g::1']) expect(isPublic(s), s).toBe(false);
  });

  test('local names', () => {
    expect(isLocalName('Printer.LOCAL.')).toBe(true);
    expect(isLocalName('a.localhost')).toBe(true);
    expect(isLocalName('db.internal')).toBe(true);
    expect(isLocalName('[::1]')).toBe(false);
    expect(isLocalName('nas.example')).toBe(false);
  });

  test('refuses_private_hops: literal addresses and local names stop a redirect chain', () => {
    for (const url of ['http://127.0.0.1/a.png', 'http://10.1.2.3/', 'http://[::1]/', 'http://router/', 'https://nas.local/x', 'file:///etc/passwd', 'http://[::ffff:a00:1]/', 'http://2130706433/']) {
      expect(hopAllowed(new URL(url)), url).toBe(false);
    }
    expect(hopAllowed(new URL('https://example.com/cat.gif'))).toBe(true);
    expect(hopAllowed(new URL('http://93.184.216.34/'))).toBe(true);
  });
});

describe('checkPublic', () => {
  test('rejects before resolving anything it can judge from the URL', async () => {
    let resolved = 0;
    const net = { resolve: async () => (resolved++, ['93.184.216.34']) };
    await expect(checkPublic(new URL('ftp://example.com/'), net)).rejects.toThrow('only http and https');
    await expect(checkPublic(new URL('http://localhost/'), net)).rejects.toThrow('local address');
    await expect(checkPublic(new URL('http://10.0.0.1/'), net)).rejects.toThrow('private');
    expect(resolved).toBe(0);
  });

  test('a name with any private answer is refused', async () => {
    const mixed = { resolve: async () => ['93.184.216.34', '10.0.0.1'] };
    await expect(checkPublic(new URL('http://mixed.example/'), mixed)).rejects.toThrow('private');
    const none = { resolve: async () => [] };
    await expect(checkPublic(new URL('http://none.example/'), none)).rejects.toThrow('private');
    const fine = { resolve: async () => ['93.184.216.34', '2606:4700::1111'] };
    expect(await checkPublic(new URL('http://fine.example/'), fine)).toEqual(['93.184.216.34', '2606:4700::1111']);
  });

  test('resolution failures are reported', async () => {
    const down = { resolve: async () => Promise.reject(new Error('ENOTFOUND')) };
    await expect(checkPublic(new URL('http://nowhere.example/'), down)).rejects.toThrow('could not resolve host: ENOTFOUND');
  });
});

describe('safeGet', () => {
  test('reads a Content-Length body and sends the expected request', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Type: Text/Plain\nContent-Length: 5', 'hello'));
    const resp = await safeGet(f.url('/a?b=1'), loopbackNet);
    expect(resp.status).toBe(200);
    expect(resp.ok).toBe(true);
    expect(resp.contentType).toBe('text/plain');
    expect(resp.contentLength).toBe(5);
    expect(await readAll(resp.chunks())).toBe('hello');
    const req = f.requests[0];
    expect(req.startsWith('GET /a?b=1 HTTP/1.1\r\n')).toBe(true);
    expect(req).toContain(`Host: fixture.example:${f.port}\r\n`);
    expect(req).toContain('Accept-Encoding: identity\r\n');
    expect(req).toContain('User-Agent: Mozilla/5.0 (compatible; Hearth link preview)\r\n');
  });

  test('decodes chunked bodies, including extensions and trailers', async () => {
    const f = await fx((s) => {
      s.write('HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n');
      // Split mid-chunk and mid-size-line to exercise the incremental parser.
      s.write('5;ext=1\r\nhel');
      setTimeout(() => {
        s.write('lo\r\n6\r\n wor');
        s.write('ld\r\n0\r\nX-Trailer: 1\r\n\r\n');
        s.end();
      }, 20);
    });
    const resp = await safeGet(f.url(), loopbackNet);
    expect(resp.contentLength).toBeNull();
    expect(await readAll(resp.chunks())).toBe('hello world');
  });

  test('close-delimited bodies run to the end of the stream', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Type: text/html', 'until the end'));
    const resp = await safeGet(f.url(), loopbackNet);
    expect(await readAll(resp.chunks())).toBe('until the end');
  });

  test('a body cut short is an error, not a short read', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Length: 10', 'abc'));
    const resp = await safeGet(f.url(), loopbackNet);
    await expect(readAll(resp.chunks())).rejects.toThrow('before the response was complete');
  });

  test('a chunked body cut short is an error too', async () => {
    const g = await fx((s) => {
      s.write('HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n');
      s.end();
    });
    const resp2 = await safeGet(g.url(), loopbackNet);
    await expect(readAll(resp2.chunks())).rejects.toThrow('before the response was complete');
  });

  test('no-body statuses end immediately', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 204 No Content'));
    const resp = await safeGet(f.url(), loopbackNet);
    expect(resp.status).toBe(204);
    expect(await readAll(resp.chunks())).toBe('');
  });

  test('breaking out of the body closes the connection', async () => {
    let closed = false;
    const f = await fx((s) => {
      s.on('close', () => (closed = true));
      s.write('HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\n\r\n' + 'x'.repeat(1000));
    });
    const resp = await safeGet(f.url(), loopbackNet);
    for await (const _ of resp.chunks()) break;
    await Bun.sleep(50);
    expect(closed).toBe(true);
  });

  test('interim responses are skipped', async () => {
    const f = await fx((s) => {
      s.write('HTTP/1.1 103 Early Hints\r\nLink: </a.css>; rel=preload\r\n\r\n');
      setTimeout(() => respond(s, 'HTTP/1.1 200 OK\nContent-Length: 2', 'ok'), 20);
    });
    const resp = await safeGet(f.url(), loopbackNet);
    expect(resp.status).toBe(200);
    expect(await readAll(resp.chunks())).toBe('ok');
  });

  test('oversized headers and trailers are refused', async () => {
    const big = await fx((s) => respond(s, `HTTP/1.1 200 OK\nX-Big: ${'a'.repeat(70 * 1024)}\nContent-Length: 0`));
    await expect(safeGet(big.url(), loopbackNet)).rejects.toThrow('headers too large');
    const trailer = await fx((s) => {
      s.write('HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nok\r\n0\r\nX-T: ');
      s.write('a'.repeat(70 * 1024));
    });
    const resp = await safeGet(trailer.url(), loopbackNet);
    await expect(readAll(resp.chunks())).rejects.toThrow('trailers too large');
  });

  test('compressed bodies are refused', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Encoding: gzip\nContent-Length: 3', 'abc'));
    const resp = await safeGet(f.url(), loopbackNet);
    await expect(readAll(resp.chunks())).rejects.toThrow('unsupported content encoding');
  });

  test('garbage and silence fail', async () => {
    const junk = await fx((s) => respond(s, 'NOT HTTP'));
    await expect(safeGet(junk.url(), loopbackNet)).rejects.toThrow('invalid HTTP response');
    const mute = await fx(() => {});
    await expect(safeGet(mute.url(), { ...loopbackNet, timeoutMs: 150 })).rejects.toThrow('timed out');
    const early = await fx((s) => s.end());
    await expect(safeGet(early.url(), loopbackNet)).rejects.toThrow('before a response arrived');
  });

  test('errorForStatus turns non-2xx into an error', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 404 Not Found\nContent-Length: 0'));
    const resp = await safeGet(f.url(), loopbackNet);
    expect(resp.ok).toBe(false);
    expect(() => errorForStatus(resp)).toThrow('HTTP status 404');
  });

  test('follows redirects and reports the final URL', async () => {
    const f = await fx((s, req) => {
      if (req.startsWith('GET /start')) respond(s, 'HTTP/1.1 302 Found\nLocation: /middle\nContent-Length: 0');
      else if (req.startsWith('GET /middle')) respond(s, `HTTP/1.1 301 Moved\nLocation: http://other.example:${f.port}/end\nContent-Length: 0`);
      else respond(s, 'HTTP/1.1 200 OK\nContent-Length: 4', 'done');
    });
    const resp = await safeGet(f.url('/start'), loopbackNet);
    expect(resp.url.pathname).toBe('/end');
    expect(await readAll(resp.chunks())).toBe('done');
    expect(f.requests[2]).toContain(`Host: other.example:${f.port}`);
  });

  test('gives up after too many redirects', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 302 Found\nLocation: /again\nContent-Length: 0'));
    await expect(safeGet(f.url(), loopbackNet)).rejects.toThrow('too many redirects');
    expect(f.connections).toBe(5);
  });

  test('a redirect without a location is returned as is', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 302 Found\nContent-Length: 0'));
    const resp = await safeGet(f.url(), loopbackNet);
    expect(resp.status).toBe(302);
  });

  test('a redirect to a private literal is refused without connecting', async () => {
    const target = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Length: 2', 'no'));
    const f = await fx((s) => respond(s, `HTTP/1.1 302 Found\nLocation: http://10.0.0.1:${target.port}/\nContent-Length: 0`));
    await expect(safeGet(f.url(), loopbackNet)).rejects.toThrow('private');
    expect(target.connections).toBe(0);
  });

  test('a redirect to a name that resolves privately is refused', async () => {
    const target = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Length: 2', 'no'));
    const f = await fx((s) => respond(s, `HTTP/1.1 302 Found\nLocation: http://127.0.0.1.nip.example:${target.port}/\nContent-Length: 0`));
    // The first host resolves to the (allowed) fixture; the second to a refused address.
    const net = {
      resolve: async (host: string) => (host.startsWith('127.0.0.1.nip') ? ['10.0.0.1'] : ['127.0.0.1']),
      isPublic: loopbackNet.isPublic,
    };
    await expect(safeGet(f.url(), net)).rejects.toThrow('private');
    expect(target.connections).toBe(0);
  });

  test('a redirect to a local name or another scheme is refused', async () => {
    const a = await fx((s) => respond(s, 'HTTP/1.1 302 Found\nLocation: http://router/admin\nContent-Length: 0'));
    await expect(safeGet(a.url(), loopbackNet)).rejects.toThrow('local address');
    const b = await fx((s) => respond(s, 'HTTP/1.1 302 Found\nLocation: file:///etc/passwd\nContent-Length: 0'));
    await expect(safeGet(b.url(), loopbackNet)).rejects.toThrow('only http and https');
  });

  test('a refused address is never connected to', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Length: 2', 'no'));
    // The fixture is listening on loopback, but the default check says that's private.
    await expect(safeGet(new URL(`http://127.0.0.1:${f.port}/`))).rejects.toThrow('private');
    await expect(safeGet(f.url(), { resolve: async () => ['127.0.0.1'] })).rejects.toThrow('private');
    await expect(safeGet(f.url(), { resolve: async () => ['93.184.216.34', '127.0.0.1'], isPublic: () => false })).rejects.toThrow('private');
    expect(f.connections).toBe(0);
  });

  test('connects to the vetted address, not a fresh lookup', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Length: 2', 'ok'));
    let lookups = 0;
    const net = { resolve: async () => (lookups++, ['127.0.0.1']), isPublic: loopbackNet.isPublic };
    const resp = await safeGet(f.url(), net);
    expect(await readAll(resp.chunks())).toBe('ok');
    expect(lookups).toBe(1);
  });

  test('falls back to the next vetted address', async () => {
    const f = await fx((s) => respond(s, 'HTTP/1.1 200 OK\nContent-Length: 2', 'ok'));
    // Nothing listens on port 1 of ::1, so the second answer must carry the request.
    const net = { resolve: async () => ['::1', '127.0.0.1'], isPublic: () => true };
    const resp = await safeGet(f.url(), net);
    expect(await readAll(resp.chunks())).toBe('ok');
  });

  test('unreachable hosts name the address', async () => {
    const f = await fx(() => {});
    const port = f.port;
    f.close();
    await expect(safeGet(new URL(`http://gone.example:${port}/`), loopbackNet)).rejects.toThrow(`could not reach gone.example:${port}`);
  });
});

describe('safeGet over TLS', () => {
  async function tls(dnsName: string) {
    const { cert, key } = selfSigned(dnsName);
    const server = tlsServer({ cert, key }, (s) => {
      s.on('error', () => {});
      s.once('data', () => s.end('HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecure'));
    });
    await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
    const port = (server.address() as { port: number }).port;
    return { cert, port, close: () => server.close() };
  }

  test('verifies the certificate against the host name', async () => {
    const s = await tls('irc.hearth.test');
    try {
      const url = new URL(`https://irc.hearth.test:${s.port}/`);
      const resp = await safeGet(url, { ...loopbackNet, ca: s.cert });
      expect(await readAll(resp.chunks())).toBe('secure');
      // Not trusted without the CA, however well-formed.
      await expect(safeGet(url, loopbackNet)).rejects.toThrow();
      // Trusted, but issued for another name.
      await expect(safeGet(new URL(`https://other.hearth.test:${s.port}/`), { ...loopbackNet, ca: s.cert })).rejects.toThrow();
    } finally {
      s.close();
    }
  });
});
