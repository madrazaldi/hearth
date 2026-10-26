import { afterEach, describe, expect, test } from 'bun:test';
import { createServer, type Server, type Socket } from 'node:net';
import { createServer as tlsServer } from 'node:tls';
import { selfSigned } from './certs';
import { connect, decode, LineSplitter, MAX_LINE, type Endpoint } from './transport';

const cleanup: (() => void)[] = [];
afterEach(() => {
  while (cleanup.length) cleanup.pop()!();
});

const endpoint = (port: number, extra: Partial<Endpoint> = {}): Endpoint => ({
  host: '127.0.0.1',
  port,
  tls: false,
  acceptInvalidCerts: false,
  websocketPath: null,
  clientCert: null,
  ...extra,
});

/** A client whose lines and close are collected; `next` waits for the n-th line. */
function collector() {
  const lines: string[] = [];
  const waiting: (() => void)[] = [];
  const closed: { error: Error | null; done: boolean } = { error: null, done: false };
  let closeWaiter: (() => void) | null = null;
  return {
    lines,
    handlers: {
      line: (l: string) => {
        lines.push(l);
        waiting.splice(0).forEach((w) => w());
      },
      close: (error: Error | null) => {
        closed.error = error;
        closed.done = true;
        closeWaiter?.();
      },
    },
    async until(n: number) {
      while (lines.length < n) await new Promise<void>((r) => waiting.push(r));
    },
    async closed() {
      if (!closed.done) await new Promise<void>((r) => (closeWaiter = r));
      return closed.error;
    },
  };
}

async function listen(onSocket: (s: Socket) => void): Promise<{ server: Server; port: number }> {
  const server = createServer((s) => {
    s.on('error', () => {});
    onSocket(s);
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  cleanup.push(() => server.close());
  return { server, port: (server.address() as { port: number }).port };
}

describe('raw sockets', () => {
  test('raw_lines_are_split_trimmed_and_decoded', async () => {
    const { port } = await listen((s) => {
      s.write(Buffer.from('PING a\r\n\r\n\nPING b\n:x PRIVMSG #a :caf\xe9\r\n:x PRIVMSG #a :caf\xc3\xa9\r\nPARTIAL', 'latin1'));
      s.end();
    });
    const c = collector();
    await connect(endpoint(port), c.handlers);
    expect(await c.closed()).toBeNull();
    // Invalid UTF-8 falls back to Latin-1; a final line without a terminator still counts.
    expect(c.lines).toEqual(['PING a', 'PING b', ':x PRIVMSG #a :café', ':x PRIVMSG #a :café', 'PARTIAL']);
  });

  test('lines are written with CRLF and the socket ends gracefully', async () => {
    let got = '';
    const done = Promise.withResolvers<void>();
    const { port } = await listen((s) => {
      s.on('data', (d) => (got += d));
      s.on('end', () => {
        s.end();
        done.resolve();
      });
    });
    const c = collector();
    const conn = await connect(endpoint(port), c.handlers);
    await conn.sendLine('NICK me');
    await conn.sendLine('USER é 0 * :ü');
    conn.end();
    await done.promise;
    expect(got).toBe('NICK me\r\nUSER é 0 * :ü\r\n');
    expect(await c.closed()).toBeNull();
  });

  test('ending with unread input half-closes instead of resetting', async () => {
    // A reset makes Windows discard what the peer hasn't read yet, such as a QUIT.
    let got = '';
    let reset = false;
    const done = Promise.withResolvers<void>();
    const { port } = await listen((s) => {
      s.write(':srv 001 me :Welcome\r\n:srv 422 me :No MOTD\r\n');
      s.on('data', (d) => (got += d));
      s.on('error', () => (reset = true));
      s.on('end', () => {
        s.end();
        done.resolve();
      });
    });
    const c = collector();
    const conn = await connect(endpoint(port), c.handlers);
    await conn.sendLine('QUIT :bye');
    conn.end();
    await done.promise;
    expect(got).toBe('QUIT :bye\r\n');
    expect(reset).toBe(false);
    expect(await c.closed()).toBeNull();
  });

  test('overlong_lines_are_rejected', async () => {
    const { port } = await listen((s) => {
      s.write(Buffer.alloc(MAX_LINE + 10, 'a'));
      s.write('\r\n');
    });
    const c = collector();
    await connect(endpoint(port), c.handlers);
    const err = await c.closed();
    expect(String(err)).toContain(`line exceeds ${MAX_LINE} bytes`);
  });

  test('a long line split over many reads is still one line', async () => {
    const { port } = await listen((s) => {
      s.write(':x PRIVMSG #a :' + 'a'.repeat(400));
      setTimeout(() => s.write('b'.repeat(400) + '\r\n'), 20);
    });
    const c = collector();
    await connect(endpoint(port), c.handlers);
    await c.until(1);
    expect(c.lines[0]).toBe(':x PRIVMSG #a :' + 'a'.repeat(400) + 'b'.repeat(400));
  });

  test('a large write that backs up the socket still arrives whole', async () => {
    const received: Buffer[] = [];
    const done = Promise.withResolvers<void>();
    const { port } = await listen((s) => {
      s.on('data', (d) => received.push(Buffer.from(d)));
      s.on('end', () => done.resolve());
    });
    const c = collector();
    const conn = await connect(endpoint(port), c.handlers);
    const lines = Array.from({ length: 200 }, (_, i) => `PRIVMSG #a :${i} ${'x'.repeat(30_000)}`);
    for (const l of lines) await conn.sendLine(l);
    conn.end();
    await done.promise;
    expect(Buffer.concat(received).toString()).toBe(lines.map((l) => l + '\r\n').join(''));
  });

  test('the peer dropping the link is reported once', async () => {
    const { port } = await listen((s) => s.destroy());
    const c = collector();
    await connect(endpoint(port), c.handlers);
    await c.closed();
  });

  test('unreachable_hosts_name_the_address', async () => {
    const { server, port } = await listen(() => {});
    server.close();
    await Bun.sleep(10);
    const err = await connect(endpoint(port), collector().handlers).then(
      () => null,
      (e: Error) => e,
    );
    expect(err?.message).toContain(`could not reach 127.0.0.1:${port}`);
  });
});

describe('tls', () => {
  test('accept_invalid_certs controls certificate verification', async () => {
    const { cert, key } = selfSigned('localhost');
    const server = tlsServer({ cert, key }, (s) => {
      s.on('error', () => {});
      s.write('PING tls\r\n');
    });
    await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
    cleanup.push(() => server.close());
    const port = (server.address() as { port: number }).port;

    const strict = await connect(endpoint(port, { tls: true, host: 'localhost' }), collector().handlers).then(
      () => null,
      (e: Error) => e,
    );
    expect(strict?.message).toContain('TLS handshake failed');

    const c = collector();
    const conn = await connect(endpoint(port, { tls: true, host: 'localhost', acceptInvalidCerts: true }), c.handlers);
    await c.until(1);
    expect(c.lines).toEqual(['PING tls']);
    conn.destroy();
  });

  test('a client certificate is presented', async () => {
    const client = selfSigned('hearth');
    const { cert, key } = selfSigned('localhost');
    let presented = null as string | null;
    const server = tlsServer({ cert, key, requestCert: true, rejectUnauthorized: false }, (s) => {
      s.on('error', () => {});
      presented = s.getPeerCertificate()?.fingerprint256 ?? null;
      s.write('PING hi\r\n');
    });
    await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
    cleanup.push(() => server.close());
    const port = (server.address() as { port: number }).port;

    const { mkdtempSync, writeFileSync, rmSync } = await import('node:fs');
    const { tmpdir } = await import('node:os');
    const { join } = await import('node:path');
    const dir = mkdtempSync(join(tmpdir(), 'hearth-tls-'));
    cleanup.push(() => rmSync(dir, { recursive: true, force: true }));
    writeFileSync(join(dir, 'c.pem'), client.cert + client.key);
    const c = collector();
    const conn = await connect(endpoint(port, { tls: true, host: 'localhost', acceptInvalidCerts: true, clientCert: join(dir, 'c.pem') }), c.handlers);
    await c.until(1);
    expect(presented).toBe(new (await import('node:crypto')).X509Certificate(client.cert).fingerprint256);
    conn.destroy();
  });

  test('a missing client certificate file fails before connecting', async () => {
    const err = await connect(endpoint(1, { tls: true, clientCert: '/nonexistent/hearth-cert.pem' }), collector().handlers).then(
      () => null,
      (e: Error) => e,
    );
    expect(err?.message).toContain('could not read client certificate');
  });
});

describe('websocket', () => {
  function wsServer(onOpen: (ws: import('bun').ServerWebSocket<unknown>) => void, onMessage?: (m: string | Buffer) => void) {
    const seen: { path: string; protocols: string | null } = { path: '', protocols: null };
    const server = Bun.serve({
      port: 0,
      hostname: '127.0.0.1',
      fetch(req, srv) {
        const u = new URL(req.url);
        seen.path = u.pathname;
        seen.protocols = req.headers.get('sec-websocket-protocol');
        return srv.upgrade(req, { headers: { 'Sec-WebSocket-Protocol': 'text.ircv3.net' }, data: undefined }) ? undefined : new Response('no', { status: 400 });
      },
      websocket: { open: onOpen, message: (_ws, m) => onMessage?.(m) },
    });
    cleanup.push(() => void server.stop(true));
    return { port: server.port as number, seen };
  }

  test('websocket_frames_are_lines', async () => {
    const got: (string | Buffer)[] = [];
    const { port, seen } = wsServer(
      (ws) => {
        ws.send('PING a\r\n');
        ws.ping();
        ws.send(Buffer.from('PING b'));
      },
      (m) => got.push(m),
    );
    const c = collector();
    const conn = await connect(endpoint(port, { websocketPath: 'irc' }), c.handlers);
    await c.until(2);
    expect(c.lines).toEqual(['PING a', 'PING b']);
    expect(seen.path).toBe('/irc');
    expect(seen.protocols).toContain('text.ircv3.net');
    await conn.sendLine('PONG a');
    while (got.length === 0) await Bun.sleep(5);
    expect(String(got[0])).toBe('PONG a');
    conn.destroy();
  });

  test('a closing server ends the link cleanly', async () => {
    const { port } = wsServer((ws) => ws.close());
    const c = collector();
    await connect(endpoint(port, { websocketPath: '/' }), c.handlers);
    expect(await c.closed()).toBeNull();
  });

  test('an unreachable websocket names the failure', async () => {
    const { port } = wsServer(() => {});
    cleanup.pop()!();
    await Bun.sleep(20);
    const err = await connect(endpoint(port, { websocketPath: '/' }), collector().handlers).then(
      () => null,
      (e: Error) => e,
    );
    expect(err?.message).toContain('WebSocket handshake failed');
  });
});

describe('decoding', () => {
  test('decode_prefers_utf8', () => {
    expect(decode(new TextEncoder().encode('naïve ☃'))).toBe('naïve ☃');
    expect(decode(Uint8Array.from([0xa3, 0xff]))).toBe('£ÿ');
  });

  test('the splitter handles CR-only garbage and splits across feeds', () => {
    const s = new LineSplitter();
    const out: string[] = [];
    s.push(new TextEncoder().encode('A\r'), (l) => out.push(l));
    s.push(new TextEncoder().encode('\nB\r\r\nC'), (l) => out.push(l));
    s.finish((l) => out.push(l));
    expect(out).toEqual(['A', 'B', 'C']);
  });
});
