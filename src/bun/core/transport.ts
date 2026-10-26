// Byte transports for IRC: plain TCP, TLS, and WebSocket (over either).
//
// Every transport is reduced to lines in and lines out, so the protocol layer
// never cares how bytes travel.

import { isIP } from 'node:net';
import { loadIdentity, type Identity } from './certs';

export const MAX_LINE = 64 * 1024;
const CONNECT_TIMEOUT_MS = 20_000;

export interface Endpoint {
  host: string;
  port: number;
  tls: boolean;
  acceptInvalidCerts: boolean;
  /** When set, speak IRC over WebSocket at this path. */
  websocketPath: string | null;
  /** PEM file with a client certificate and key to present (CertFP). */
  clientCert: string | null;
}

export interface LineHandlers {
  /** One complete line, without its terminator. */
  line(line: string): void;
  /** The link ended, once: `null` when the peer closed it cleanly. */
  close(error: Error | null): void;
}

export interface LineConn {
  /** Resolves once the line is handed to the socket (later, if the socket is backed up). */
  sendLine(line: string): Promise<void>;
  /** Finish writing and half-close; reading goes on until the peer closes, or until `destroy`. */
  end(): void;
  /** Drop the link now. */
  destroy(): void;
}

const messageOf = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** IRC has no mandated encoding; prefer UTF-8 and fall back to Latin-1. */
export function decode(bytes: Uint8Array): string {
  try {
    return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    let s = '';
    for (const b of bytes) s += String.fromCharCode(b);
    return s;
  }
}

/** Splits a byte stream into lines, as IRC servers send them. */
export class LineSplitter {
  private buf: Uint8Array = new Uint8Array(0);

  /** Feed bytes; calls `line` for each complete non-empty line. Throws if a line is too long. */
  push(data: Uint8Array, line: (l: string) => void) {
    let buf: Uint8Array;
    if (this.buf.length) {
      buf = new Uint8Array(this.buf.length + data.length);
      buf.set(this.buf);
      buf.set(data, this.buf.length);
    } else buf = data;
    let start = 0;
    for (;;) {
      const nl = buf.indexOf(0x0a, start);
      if (nl < 0) break;
      if (nl + 1 - start > MAX_LINE) throw new Error(`line exceeds ${MAX_LINE} bytes`);
      let end = nl;
      while (end > start && (buf[end - 1] === 0x0d || buf[end - 1] === 0x0a)) end--;
      if (end > start) line(decode(buf.subarray(start, end)));
      start = nl + 1;
    }
    if (buf.length - start > MAX_LINE) throw new Error(`line exceeds ${MAX_LINE} bytes`);
    this.buf = Uint8Array.from(buf.subarray(start));
  }

  /** The peer closed: a final line without a terminator still counts. */
  finish(line: (l: string) => void) {
    let end = this.buf.length;
    while (end > 0 && (this.buf[end - 1] === 0x0d || this.buf[end - 1] === 0x0a)) end--;
    if (end > 0) line(decode(this.buf.subarray(0, end)));
    this.buf = new Uint8Array(0);
  }
}

export async function connect(ep: Endpoint, handlers: LineHandlers): Promise<LineConn> {
  const identity = ep.tls && ep.clientCert ? loadIdentity(ep.clientCert) : null;
  return ep.websocketPath !== null ? connectWebSocket(ep, ep.websocketPath, identity, handlers) : connectSocket(ep, identity, handlers);
}

async function connectSocket(ep: Endpoint, identity: Identity | null, h: LineHandlers): Promise<LineConn> {
  const where = `${ep.host}:${ep.port}`;
  const splitter = new LineSplitter();
  let usable = false;
  let closed = false;
  // Bytes the socket couldn't take yet, written again on 'drain'.
  let pending: Uint8Array | null = null;
  let drained: (() => void) | null = null;
  let sock: Awaited<ReturnType<typeof Bun.connect>> | null = null;

  let ready!: { resolve: () => void; reject: (e: Error) => void };
  const handshaken = new Promise<void>((resolve, reject) => (ready = { resolve, reject }));

  const finish = (err: Error | null) => {
    if (closed) return;
    closed = true;
    drained?.();
    if (usable) h.close(err);
  };

  const connecting = Bun.connect({
    hostname: ep.host,
    port: ep.port,
    tls: ep.tls
      ? {
          serverName: isIP(ep.host) ? undefined : ep.host,
          rejectUnauthorized: !ep.acceptInvalidCerts,
          ...(identity ? { cert: identity.certs.join('\n'), key: identity.key } : {}),
        }
      : undefined,
    socket: {
      open: () => {
        if (!ep.tls) ready.resolve();
      },
      handshake: (s, ok, authError) => {
        if (ok || ep.acceptInvalidCerts) ready.resolve();
        else {
          ready.reject(new Error(`TLS handshake failed: ${authError?.message ?? 'certificate rejected'}`));
          s.terminate();
        }
      },
      data: (_s, d) => {
        try {
          splitter.push(d, h.line);
        } catch (e) {
          finish(e instanceof Error ? e : new Error(String(e)));
          sock?.terminate();
        }
      },
      drain: (s) => {
        if (pending) {
          const n = s.write(pending);
          pending = n < pending.length ? pending.subarray(n) : null;
        }
        if (!pending) {
          const d = drained;
          drained = null;
          d?.();
        }
      },
      end: () => {
        if (closed) return;
        try {
          splitter.finish(h.line);
        } catch {}
        finish(null);
      },
      close: () => {
        ready.reject(new Error('connection closed during the handshake'));
        finish(null);
      },
      error: (_s, e) => {
        ready.reject(e);
        finish(e);
      },
      connectError: (_s, e) => ready.reject(e),
    },
  });
  connecting.catch((e) => ready.reject(e instanceof Error ? e : new Error(String(e))));

  let timer: ReturnType<typeof setTimeout>;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`timed out connecting to ${where}`)), CONNECT_TIMEOUT_MS);
  });
  try {
    [sock] = await Promise.race([Promise.all([connecting, handshaken]), timeout]);
  } catch (e) {
    connecting.then((s) => s.terminate(), () => {});
    const msg = messageOf(e);
    throw new Error(msg.startsWith('timed out') || msg.startsWith('TLS handshake failed') ? msg : `could not reach ${where}: ${msg}`);
  } finally {
    clearTimeout(timer!);
  }
  const socket = sock;
  usable = true;
  if (closed) throw new Error(`could not reach ${where}: connection closed`);

  return {
    sendLine(line) {
      if (closed) return Promise.reject(new Error('connection closed'));
      const bytes = new TextEncoder().encode(line + '\r\n');
      const n = pending ? 0 : socket.write(bytes);
      if (n >= bytes.length) return Promise.resolve();
      // Backed up: queue the rest behind what is waiting and resolve on drain.
      if (pending) {
        const joined = new Uint8Array(pending.length + bytes.length);
        joined.set(pending);
        joined.set(bytes, pending.length);
        pending = joined;
      } else pending = bytes.subarray(n);
      return new Promise<void>((resolve, reject) => {
        const prev = drained;
        drained = () => {
          prev?.();
          if (closed && pending) reject(new Error('connection closed'));
          else resolve();
        };
      });
    },
    end: () => void socket.shutdown(),
    destroy: () => void socket.terminate(),
  };
}

async function connectWebSocket(ep: Endpoint, path: string, identity: Identity | null, h: LineHandlers): Promise<LineConn> {
  const url = `${ep.tls ? 'wss' : 'ws'}://${ep.host.includes(':') ? `[${ep.host}]` : ep.host}:${ep.port}${path.startsWith('/') ? path : '/' + path}`;
  const ws = new WebSocket(url, {
    protocols: ['text.ircv3.net', 'binary.ircv3.net'],
    tls: ep.tls ? { rejectUnauthorized: !ep.acceptInvalidCerts, ...(identity ? { cert: identity.certs.join('\n'), key: identity.key } : {}) } : undefined,
  } as unknown as string[]);
  ws.binaryType = 'arraybuffer';

  let usable = false;
  let closed = false;
  const finish = (err: Error | null) => {
    if (closed) return;
    closed = true;
    if (usable) h.close(err);
  };

  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error(`timed out connecting to ${ep.host}:${ep.port}`));
    }, CONNECT_TIMEOUT_MS);
    ws.addEventListener('open', () => {
      clearTimeout(timer);
      resolve();
    });
    ws.addEventListener('error', (ev) => {
      clearTimeout(timer);
      const detail = (ev as ErrorEvent).message;
      reject(new Error(`WebSocket handshake failed${detail ? `: ${detail}` : ''}`));
    });
    ws.addEventListener('close', () => {
      clearTimeout(timer);
      reject(new Error('WebSocket handshake failed: connection closed'));
    });
  });
  usable = true;

  ws.addEventListener('message', (ev) => {
    const data = ev.data as string | ArrayBuffer;
    const text = typeof data === 'string' ? data : decode(new Uint8Array(data));
    if (text.length > MAX_LINE) {
      finish(new Error(`line exceeds ${MAX_LINE} bytes`));
      ws.close();
      return;
    }
    h.line(text.trimEnd());
  });
  ws.addEventListener('error', () => finish(new Error('WebSocket error')));
  ws.addEventListener('close', () => finish(null));

  return {
    sendLine(line) {
      if (closed || ws.readyState !== WebSocket.OPEN) return Promise.reject(new Error('connection closed'));
      ws.send(line);
      return Promise.resolve();
    },
    end: () => ws.close(),
    destroy: () => ws.close(),
  };
}
