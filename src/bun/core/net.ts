// Network guards and an HTTP/1.1 client for fetching URLs that strangers post
// in channels. Only public addresses are fetched: a link must not be able to
// make Hearth probe the user's LAN or localhost.
//
// Bun's fetch can't pin the address it connects to, so a check followed by a
// fetch would leave a DNS-rebinding window. This client resolves each hop
// itself, checks every answer, and then connects to the vetted IP.

import { lookup } from 'node:dns/promises';
import { isIP } from 'node:net';

const USER_AGENT = 'Mozilla/5.0 (compatible; Hearth link preview)';
const MAX_HEADER_BYTES = 64 * 1024;
/**
 * Bytes a reader may leave unread before the response is dropped. Callers cap
 * what they accept far below this; it only bounds a reader that has stalled.
 * (Pausing the socket would be gentler, but a stalled reader is rare.)
 */
const MAX_QUEUED = 64 * 1024 * 1024;

// ---- Address classification ----

/** Parse a dotted quad into 4 octets, or null. */
function parseV4(s: string): number[] | null {
  const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(s);
  if (!m) return null;
  const o = m.slice(1).map(Number);
  return o.every((n) => n <= 255) ? o : null;
}

/** Parse an IPv6 literal into 8 16-bit groups, or null. Handles `::` and an embedded dotted quad. */
function parseV6(input: string): number[] | null {
  let s = input.replace(/^\[|\]$/g, '');
  const zone = s.indexOf('%');
  if (zone >= 0) s = s.slice(0, zone);
  const tail: number[] = [];
  const dot = s.lastIndexOf('.');
  if (dot >= 0) {
    const colon = s.lastIndexOf(':');
    const v4 = parseV4(s.slice(colon + 1));
    if (!v4) return null;
    tail.push((v4[0] << 8) | v4[1], (v4[2] << 8) | v4[3]);
    s = s.slice(0, colon + 1) + '0:0';
  }
  const halves = s.split('::');
  if (halves.length > 2) return null;
  const groups = (part: string) => (part === '' ? [] : part.split(':'));
  const head = groups(halves[0]);
  const rest = halves.length === 2 ? groups(halves[1]) : [];
  const total = head.length + rest.length;
  if (halves.length === 1 ? total !== 8 : total > 7) return null;
  const all = [...head, ...Array<string>(8 - total).fill('0'), ...rest];
  if (halves.length === 1 && all.length !== 8) return null;
  const out: number[] = [];
  for (const g of all) {
    if (!/^[0-9a-fA-F]{1,4}$/.test(g)) return null;
    out.push(parseInt(g, 16));
  }
  if (tail.length) {
    out[6] = tail[0];
    out[7] = tail[1];
  }
  return out;
}

function publicV4(o: number[]): boolean {
  const [a, b, c] = o;
  return !(
    a === 0 ||
    a === 10 ||
    a === 127 ||
    (a === 172 && b >= 16 && b < 32) ||
    (a === 192 && b === 168) ||
    (a === 169 && b === 254) ||
    (a === 192 && b === 0 && c === 2) ||
    (a === 198 && b === 51 && c === 100) ||
    (a === 203 && b === 0 && c === 113) ||
    (a === 100 && b >= 64 && b < 128) ||
    (a === 192 && b === 0 && c === 0) ||
    a >= 224 // multicast and reserved
  );
}

const quad = (hi: number, lo: number) => [hi >> 8, hi & 255, lo >> 8, lo & 255];

function publicV6(g: number[]): boolean {
  const zeros = (n: number) => g.slice(0, n).every((x) => x === 0);
  // ::, ::1, and the deprecated IPv4-compatible ::a.b.c.d
  if (zeros(5) && g[5] === 0) {
    if (g[6] === 0 && g[7] <= 1) return false;
    return publicV4(quad(g[6], g[7]));
  }
  // ::ffff:a.b.c.d, however it was written
  if (zeros(5) && g[5] === 0xffff) return publicV4(quad(g[6], g[7]));
  // 64:ff9b::/96 (NAT64) reaches the embedded IPv4 address
  if (g[0] === 0x64 && g[1] === 0xff9b && g.slice(2, 6).every((x) => x === 0)) return publicV4(quad(g[6], g[7]));
  // 2002::/16 (6to4) embeds an IPv4 address too
  if (g[0] === 0x2002) return publicV4(quad(g[1], g[2]));
  return !(
    (g[0] & 0xfe00) === 0xfc00 || // unique local
    (g[0] & 0xffc0) === 0xfe80 || // link-local
    (g[0] & 0xffc0) === 0xfec0 || // deprecated site-local
    (g[0] & 0xff00) === 0xff00 || // multicast
    (g[0] === 0x2001 && g[1] === 0x0db8) || // documentation
    (g[0] === 0x2001 && g[1] === 0) // Teredo
  );
}

/** Whether `ip` (a literal, v4 or v6) is a globally routable address. Anything unparseable is refused. */
export function isPublic(ip: string): boolean {
  const v4 = parseV4(ip);
  if (v4) return publicV4(v4);
  const v6 = parseV6(ip);
  return v6 ? publicV6(v6) : false;
}

export function isLocalName(host: string): boolean {
  const h = host.replace(/\.$/, '').toLowerCase();
  return h === 'localhost' || h.endsWith('.localhost') || h.endsWith('.local') || h.endsWith('.internal') || (!h.includes('.') && isIP(h.replace(/^\[|\]$/g, '')) === 0);
}

/**
 * Whether a redirect target may be followed without resolving it; the check
 * in `safeGet` catches names that resolve to private addresses.
 */
export function hopAllowed(url: URL): boolean {
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return false;
  const host = url.hostname.replace(/^\[|\]$/g, '');
  if (!host) return false;
  return isIP(host) ? isPublic(host) : !isLocalName(host);
}

// ---- Checked resolution ----

export interface NetOptions {
  /** Addresses a name resolves to. Tests inject this to serve from loopback. */
  resolve?: (host: string) => Promise<string[]>;
  /** Override of the public-address test, for the same reason. */
  isPublic?: (ip: string) => boolean;
}

async function systemResolve(host: string): Promise<string[]> {
  return (await lookup(host, { all: true })).map((a) => a.address);
}

/** Resolve `url`'s host and refuse unless every answer is public. Returns the vetted addresses. */
export async function checkPublic(url: URL, net: NetOptions = {}): Promise<string[]> {
  if (url.protocol !== 'http:' && url.protocol !== 'https:') throw new Error('only http and https links are previewed');
  const host = url.hostname.replace(/^\[|\]$/g, '');
  if (!host) throw new Error('URL has no host');
  const ok = net.isPublic ?? isPublic;
  if (isIP(host)) {
    if (!ok(host)) throw new Error('refusing to preview a private address');
    return [host];
  }
  if (isLocalName(host)) throw new Error('refusing to preview a local address');
  let addrs: string[];
  try {
    addrs = await (net.resolve ?? systemResolve)(host);
  } catch (e) {
    throw new Error(`could not resolve host: ${e instanceof Error ? e.message : e}`);
  }
  if (addrs.length === 0 || addrs.some((a) => !ok(a))) throw new Error('refusing to preview a private address');
  return addrs;
}

// ---- HTTP/1.1 client ----

export interface GetOptions extends NetOptions {
  /** Whole-request deadline, redirects and body included. */
  timeoutMs?: number;
  connectTimeoutMs?: number;
  maxRedirects?: number;
  /** Extra trusted CA certificates (PEM), for tests. */
  ca?: string;
}

export interface HttpResponse {
  /** The URL that produced this response, after redirects. */
  url: URL;
  status: number;
  ok: boolean;
  /** Lowercased names; repeated headers are joined with ", ". */
  headers: Record<string, string>;
  /** Lowercased `Content-Type`, or ''. */
  contentType: string;
  contentLength: number | null;
  /** The body, de-framed. Breaking out of the loop closes the connection. */
  chunks(): AsyncGenerator<Uint8Array, void, void>;
  /** Drop the connection without reading the rest. */
  close(): void;
}

/** Throws for a non-2xx status, like reqwest's `error_for_status`. */
export function errorForStatus(resp: HttpResponse): HttpResponse {
  if (!resp.ok) {
    resp.close();
    throw new Error(`HTTP status ${resp.status} for url (${resp.url})`);
  }
  return resp;
}

/** A queue the socket pushes into and the reader pulls from, with backpressure. */
class Body {
  private queue: Uint8Array[] = [];
  private queued = 0;
  private wake: (() => void) | null = null;
  private finished = false;
  private failure: Error | null = null;
  constructor(private readonly onOverflow: () => void) {}

  push(chunk: Uint8Array) {
    if (this.finished || chunk.length === 0) return;
    this.queue.push(chunk);
    this.queued += chunk.length;
    if (this.queued > MAX_QUEUED) {
      this.fail(new Error('response too large'));
      this.onOverflow();
      return;
    }
    this.signal();
  }

  end() {
    this.finished = true;
    this.signal();
  }

  fail(e: Error) {
    if (this.finished) return;
    this.failure = e;
    this.finished = true;
    this.signal();
  }

  private signal() {
    const w = this.wake;
    this.wake = null;
    w?.();
  }

  async next(): Promise<Uint8Array | null> {
    for (;;) {
      const chunk = this.queue.shift();
      if (chunk) {
        this.queued -= chunk.length;
        return chunk;
      }
      if (this.failure) throw this.failure;
      if (this.finished) return null;
      await new Promise<void>((r) => (this.wake = r));
    }
  }
}

type Framing = { kind: 'none' } | { kind: 'length'; left: number } | { kind: 'chunked'; state: 'size' | 'data' | 'crlf' | 'trailers'; left: number } | { kind: 'close' };

/** Incremental parser for one response. */
class Parser {
  private buf = Buffer.alloc(0);
  private framing: Framing | null = null;
  done = false;
  constructor(
    private readonly onHead: (status: number, headers: Record<string, string>) => Framing,
    private readonly body: Body,
  ) {}

  feed(data: Uint8Array) {
    this.buf = this.buf.length ? Buffer.concat([this.buf, data]) : Buffer.from(data);
    this.run();
  }

  private run() {
    if (!this.framing) {
      const end = this.buf.indexOf('\r\n\r\n');
      if (end < 0) {
        if (this.buf.length > MAX_HEADER_BYTES) throw new Error('response headers too large');
        return;
      }
      const head = this.buf.subarray(0, end).toString('latin1');
      this.buf = this.buf.subarray(end + 4);
      const [statusLine, ...lines] = head.split('\r\n');
      const m = /^HTTP\/1\.[01] (\d{3})/.exec(statusLine);
      if (!m) throw new Error('invalid HTTP response');
      const headers: Record<string, string> = {};
      for (const line of lines) {
        const i = line.indexOf(':');
        if (i <= 0) continue;
        const k = line.slice(0, i).trim().toLowerCase();
        const v = line.slice(i + 1).trim();
        headers[k] = k in headers ? `${headers[k]}, ${v}` : v;
      }
      this.framing = this.onHead(Number(m[1]), headers);
    }
    const f = this.framing;
    switch (f.kind) {
      case 'none':
        this.finish();
        return;
      case 'close':
        if (this.buf.length) this.body.push(this.take(this.buf.length));
        return;
      case 'length': {
        const n = Math.min(f.left, this.buf.length);
        if (n) this.body.push(this.take(n));
        f.left -= n;
        if (f.left === 0) this.finish();
        return;
      }
      case 'chunked':
        this.runChunked(f);
    }
  }

  private take(n: number): Uint8Array {
    const out = Uint8Array.from(this.buf.subarray(0, n));
    this.buf = this.buf.subarray(n);
    return out;
  }

  private runChunked(f: Extract<Framing, { kind: 'chunked' }>) {
    for (;;) {
      if (f.state === 'size') {
        const eol = this.buf.indexOf('\r\n');
        if (eol < 0) {
          if (this.buf.length > 1024) throw new Error('invalid chunk size');
          return;
        }
        const size = this.buf.subarray(0, eol).toString('latin1').split(';')[0].trim();
        if (!/^[0-9a-fA-F]+$/.test(size)) throw new Error('invalid chunk size');
        this.buf = this.buf.subarray(eol + 2);
        f.left = parseInt(size, 16);
        f.state = f.left === 0 ? 'trailers' : 'data';
      } else if (f.state === 'data') {
        const n = Math.min(f.left, this.buf.length);
        if (n) this.body.push(this.take(n));
        f.left -= n;
        if (f.left > 0) return;
        f.state = 'crlf';
      } else if (f.state === 'crlf') {
        if (this.buf.length < 2) return;
        this.buf = this.buf.subarray(2);
        f.state = 'size';
      } else {
        // Trailers end at an empty line; they are ignored.
        const eol = this.buf.indexOf('\r\n');
        if (eol < 0) return;
        const empty = eol === 0;
        this.buf = this.buf.subarray(eol + 2);
        if (empty) return this.finish();
      }
    }
  }

  private finish() {
    this.done = true;
    this.body.end();
  }

  /** The peer closed. Fails the body unless that is how it was meant to end. */
  eof() {
    if (this.done) return;
    if (this.framing?.kind === 'close') this.finish();
    else if (!this.framing) throw new Error('connection closed before a response arrived');
    else throw new Error('connection closed before the response was complete');
  }
}

const REDIRECTS = new Set([301, 302, 303, 307, 308]);

interface Wire {
  write(data: string): void;
  destroy(): void;
}

interface WireHandlers {
  data(chunk: Uint8Array): void;
  /** The peer finished sending (or the socket closed). Called once. */
  end(): void;
  error(e: Error): void;
}

/** Connect to a vetted address. Bun's native sockets report the peer closing reliably, unlike its node:net shim. */
async function openWire(url: URL, ip: string, o: GetOptions, h: WireHandlers): Promise<Wire> {
  const host = url.hostname.replace(/^\[|\]$/g, '');
  const tls = url.protocol === 'https:';
  const port = Number(url.port) || (tls ? 443 : 80);
  let ended = false;
  // Failures before the connection is usable belong to the caller of openWire, not to the response.
  let usable = false;
  const end = () => {
    if (ended || !usable) return;
    ended = true;
    h.end();
  };
  // For TLS the socket is only usable once the handshake has passed verification.
  let ready!: { resolve: () => void; reject: (e: Error) => void };
  const handshaken = new Promise<void>((resolve, reject) => (ready = { resolve, reject }));
  const connecting = Bun.connect({
    hostname: ip,
    port,
    tls: tls ? { serverName: isIP(host) ? undefined : host, rejectUnauthorized: true, ca: o.ca } : undefined,
    socket: {
      open: () => {
        if (!tls) ready.resolve();
      },
      handshake: (s, ok, authError) => {
        if (ok) ready.resolve();
        else {
          ready.reject(new Error(authError?.message ?? 'TLS handshake failed'));
          s.terminate();
        }
      },
      // Copied because Bun may reuse its read buffer.
      data: (_s, d) => h.data(Uint8Array.from(d)),
      end: () => end(),
      close: () => {
        ready.reject(new Error('connection closed during the handshake'));
        end();
      },
      error: (_s, e) => {
        ready.reject(e);
        if (usable) h.error(e);
      },
      connectError: (_s, e) => ready.reject(e),
    },
  });
  connecting.catch((e) => ready.reject(e instanceof Error ? e : new Error(String(e))));
  let timer: ReturnType<typeof setTimeout>;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`timed out connecting to ${host}:${port}`)), o.connectTimeoutMs ?? 5000);
  });
  try {
    const [sock] = await Promise.race([Promise.all([connecting, handshaken]), timeout]);
    usable = true;
    return {
      write: (data) => void sock.write(data),
      destroy: () => void sock.terminate(),
    };
  } catch (e) {
    // A connection that completes after the deadline must not linger.
    connecting.then((s) => s.terminate(), () => {});
    throw e;
  } finally {
    clearTimeout(timer!);
  }
}

/** One request, no redirects: the address list is already vetted. */
async function request(url: URL, addrs: string[], o: GetOptions, deadline: number): Promise<HttpResponse> {
  let settled = false;
  let parser!: Parser;
  let wire: Wire | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const body = new Body(() => wire?.destroy());
  let rejectHead: (e: Error) => void = () => {};
  const abort = (e: Error) => {
    clearTimeout(timer);
    // Record the failure before destroying: closing the socket calls back into `end`.
    if (!settled) {
      settled = true;
      rejectHead(e);
    } else body.fail(e);
    wire?.destroy();
  };
  const guard = (fn: () => void) => {
    try {
      fn();
    } catch (e) {
      abort(e instanceof Error ? e : new Error(String(e)));
    }
  };

  const head = new Promise<HttpResponse>((resolve, reject) => {
    rejectHead = reject;
    parser = new Parser((status, headers) => {
      settled = true;
      const clen = headers['content-length'];
      const length = clen !== undefined && /^\d+$/.test(clen) ? Number(clen) : null;
      const enc = headers['content-encoding']?.toLowerCase();
      let framing: Framing;
      if (enc && enc !== 'identity') {
        // We asked for identity; a compressed body can't be passed on as-is.
        framing = { kind: 'none' };
        body.fail(new Error(`unsupported content encoding: ${enc}`));
      } else if (status < 200 || status === 204 || status === 304) framing = { kind: 'none' };
      else if (headers['transfer-encoding']?.toLowerCase().includes('chunked')) framing = { kind: 'chunked', state: 'size', left: 0 };
      else if (length !== null) framing = { kind: 'length', left: length };
      else framing = { kind: 'close' };
      resolve({
        url,
        status,
        ok: status >= 200 && status < 300,
        headers,
        contentType: (headers['content-type'] ?? '').toLowerCase(),
        contentLength: length,
        close: () => {
          clearTimeout(timer);
          wire?.destroy();
          body.end();
        },
        async *chunks() {
          try {
            for (;;) {
              const c = await body.next();
              if (c === null) return;
              yield c;
            }
          } finally {
            clearTimeout(timer);
            wire?.destroy();
          }
        },
      });
      return framing;
    }, body);
  });

  timer = setTimeout(() => abort(new Error('request timed out')), Math.max(1, deadline - Date.now()));
  const handlers: WireHandlers = {
    data: (chunk) => guard(() => parser.feed(chunk)),
    end: () =>
      guard(() => {
        clearTimeout(timer);
        parser.eof();
        body.end();
        wire?.destroy();
      }),
    error: abort,
  };

  let lastError = new Error(`could not reach ${url.host}`);
  for (const ip of addrs) {
    try {
      wire = await openWire(url, ip, o, handlers);
      break;
    } catch (e) {
      lastError = new Error(`could not reach ${url.host}: ${e instanceof Error ? e.message : e}`);
    }
  }
  if (!wire) {
    clearTimeout(timer);
    throw lastError;
  }
  wire.write(
    `GET ${url.pathname}${url.search} HTTP/1.1\r\nHost: ${url.host}\r\nUser-Agent: ${USER_AGENT}\r\nAccept: */*\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n`,
  );
  return head;
}

/**
 * GET a URL. Every hop, including redirects, resolves its name, requires all
 * answers to be public, and connects to those exact addresses.
 */
export async function safeGet(url: URL, opts: GetOptions = {}): Promise<HttpResponse> {
  const deadline = Date.now() + (opts.timeoutMs ?? 8000);
  const max = opts.maxRedirects ?? 4;
  let current = url;
  for (let hop = 0; ; hop++) {
    const addrs = await checkPublic(current, opts);
    const resp = await request(current, addrs, opts, deadline);
    const location = resp.headers['location'];
    if (!REDIRECTS.has(resp.status) || !location) return resp;
    resp.close();
    if (hop >= max) throw new Error('too many redirects');
    try {
      current = new URL(location, current);
    } catch {
      throw new Error('invalid redirect');
    }
  }
}
