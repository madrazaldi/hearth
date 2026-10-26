// A raw TCP "HTTP server" for tests: each connection gets the request text and
// writes whatever bytes the test wants, so framing edge cases are reachable.

import { createServer, type Server, type Socket } from 'node:net';
import { isPublic } from './net';

export interface Fixture {
  port: number;
  /** Connections accepted so far. */
  connections: number;
  requests: string[];
  url(path?: string, host?: string): URL;
  close(): void;
}

/** Lets loopback through; everything else is judged as usual. */
export const loopbackNet = {
  resolve: async () => ['127.0.0.1'],
  isPublic: (ip: string) => ip === '127.0.0.1' || isPublic(ip),
};

export async function serve(handler: (sock: Socket, request: string, n: number) => void): Promise<Fixture> {
  const fx: Fixture = {
    port: 0,
    connections: 0,
    requests: [],
    url: (path = '/', host = 'fixture.example') => new URL(`http://${host}:${fx.port}${path}`),
    close: () => server.close(),
  };
  const server: Server = createServer((sock) => {
    const n = fx.connections++;
    sock.on('error', () => {});
    let buf = '';
    sock.on('data', (d) => {
      buf += d.toString('latin1');
      if (buf.includes('\r\n\r\n')) {
        fx.requests.push(buf);
        handler(sock, buf, n);
        buf = '';
      }
    });
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  fx.port = (server.address() as { port: number }).port;
  return fx;
}

/** Write `head` and `body` as one HTTP/1.1 response, then close. */
export function respond(sock: Socket, head: string, body: Uint8Array | string = '') {
  sock.write(head.replace(/\n/g, '\r\n') + '\r\n\r\n');
  if (body.length) sock.write(body);
  sock.end();
}
