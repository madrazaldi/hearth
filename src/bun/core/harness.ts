// Shared harness for driving connection actors against a scripted IRC server
// on localhost (test support only).
//
// Waits are bounded by wall-clock time. The lifecycle tests drive the actor's
// clock by hand, so a timeout on that clock would never fire and a hung test
// would hang forever.

import { expect } from 'bun:test';
import { Connection, type Cmd } from './client';
import type { Clock } from './clock';
import { defaultServer, type ServerConfig } from './config';
import { Emitter, type Event, type Status } from './events';
import { Manager } from './manager';
import type { Message } from './message';
import { Db } from './store';

/** How long any single wait may take before the test fails. */
export const LIMIT = 10_000;

export const realSleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/** `f`'s result, or undefined if it takes longer than `ms` of wall-clock time. */
export async function within<T>(ms: number, f: Promise<T>): Promise<T | undefined> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<undefined>((r) => (timer = setTimeout(() => r(undefined), ms)));
  try {
    return await Promise.race([f, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

export async function must<T>(what: string, f: Promise<T>): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, rej) => (timer = setTimeout(() => rej(new Error(`timed out waiting for ${what}`)), LIMIT)));
  try {
    return await Promise.race([f, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

/** A clock that moves only when a test advances it. */
export class ManualClock implements Clock {
  private t = 0;
  private timers: { at: number; seq: number; fn: () => void }[] = [];
  private seq = 0;

  now() {
    return this.t;
  }

  after(ms: number, fn: () => void) {
    const timer = { at: this.t + ms, seq: this.seq++, fn };
    this.timers.push(timer);
    return () => {
      this.timers = this.timers.filter((x) => x !== timer);
    };
  }

  /** Move time forward `total` ms in `step`-sized hops, firing timers that fall due and letting real I/O settle after each. */
  async advance(total: number, step = total) {
    let left = total;
    while (left > 0) {
      const d = Math.min(step, left);
      const target = this.t + d;
      for (;;) {
        const due = this.timers.filter((x) => x.at <= target).sort((a, b) => a.at - b.at || a.seq - b.seq)[0];
        if (!due) break;
        this.timers = this.timers.filter((x) => x !== due);
        this.t = Math.max(this.t, due.at);
        due.fn();
        await realSleep(0);
      }
      this.t = target;
      left -= d;
      // Sockets run on real time: give them a moment to deliver.
      await realSleep(4);
    }
  }
}

export const secs = (s: number) => s * 1000;

export class Peer {
  private buf = '';
  private lines: string[] = [];
  private waiter: (() => void) | null = null;
  isClosed = false;

  constructor(private sock: Bun.Socket<Peer>) {}

  /** @internal */
  onData(chunk: Buffer) {
    this.buf += chunk.toString('utf8');
    let i: number;
    while ((i = this.buf.indexOf('\n')) >= 0) {
      this.lines.push(this.buf.slice(0, i).replace(/\r$/, '').trimEnd());
      this.buf = this.buf.slice(i + 1);
    }
    this.wake();
  }

  /** @internal */
  onClose() {
    this.isClosed = true;
    this.wake();
  }

  private wake() {
    const w = this.waiter;
    this.waiter = null;
    w?.();
  }

  /** The next line, or null once the client hangs up. */
  private async nextNow(): Promise<string | null> {
    for (;;) {
      const l = this.lines.shift();
      if (l !== undefined) return l;
      if (this.isClosed) return null;
      await new Promise<void>((r) => (this.waiter = r));
    }
  }

  async next(): Promise<string | null> {
    return must('a client line', this.nextNow());
  }

  async line(): Promise<string> {
    const l = await this.next();
    if (l === null) throw new Error('client hung up');
    return l;
  }

  /** A line arriving within `ms`, if any. */
  async tryLine(ms: number): Promise<string | null> {
    // A cancelled wait must not swallow a line, so only peek here.
    const got = await within(ms, this.peek());
    return got === undefined ? null : this.lines.shift() ?? null;
  }

  private async peek() {
    while (this.lines.length === 0) {
      if (this.isClosed) return;
      await new Promise<void>((r) => (this.waiter = r));
    }
  }

  /** Read lines until one starts with `prefix`, returning it. */
  async expect(prefix: string): Promise<string> {
    for (;;) {
      const l = await this.line();
      if (l.startsWith(prefix)) return l;
    }
  }

  /** Read until the client closes the connection, returning what it sent. */
  async closed(): Promise<string[]> {
    const out: string[] = [];
    for (;;) {
      const l = await this.next();
      if (l === null) return out;
      out.push(l);
    }
  }

  async send(line: string) {
    this.sock.write(`${line}\r\n`);
    this.sock.flush();
  }

  /** Hang up. */
  drop() {
    this.sock.terminate();
  }

  /**
   * Assert the client sent nothing in reply to what came before: lines are
   * handled in order, so the answer to a PING must be the next thing read.
   */
  async assertQuiet() {
    await this.send('PING :sync');
    expect(await this.line()).toBe('PONG sync');
  }

  async register(caps: string) {
    await this.expect('CAP LS');
    await this.send(`:srv CAP * LS :${caps}`);
    if (caps !== '') {
      const req = await this.expect('CAP REQ');
      await this.send(`:srv CAP * ACK :${req.slice(req.indexOf(':') + 1)}`);
    }
    await this.expect('CAP END');
    await this.welcome();
  }

  async welcome() {
    await this.send(':srv 001 me :Welcome');
    await this.send(':srv 005 me CHANTYPES=# CASEMAPPING=ascii :are supported');
    await this.send(':srv 422 me :No MOTD');
  }
}

export class Listener {
  private accepted: Peer[] = [];
  private waiter: (() => void) | null = null;
  readonly port: number;
  private server: Bun.TCPSocketListener<Peer>;

  constructor() {
    this.server = Bun.listen<Peer>({
      hostname: '127.0.0.1',
      port: 0,
      socket: {
        open: (s) => {
          s.data = new Peer(s);
          this.accepted.push(s.data);
          const w = this.waiter;
          this.waiter = null;
          w?.();
        },
        data: (s, d) => s.data.onData(d),
        end: (s) => s.data.onClose(),
        close: (s) => s.data.onClose(),
        error: (s) => s.data.onClose(),
      },
    });
    this.port = this.server.port;
  }

  private async acceptNow(): Promise<Peer> {
    for (;;) {
      const p = this.accepted.shift();
      if (p) return p;
      await new Promise<void>((r) => (this.waiter = r));
    }
  }

  accept(): Promise<Peer> {
    return must('the client to connect', this.acceptNow());
  }

  /** A connection arriving within `ms`, if any. */
  async tryAccept(ms: number): Promise<Peer | null> {
    const got = await within(ms, this.peekAccept());
    return got === undefined ? null : this.accepted.shift() ?? null;
  }

  private async peekAccept() {
    while (this.accepted.length === 0) await new Promise<void>((r) => (this.waiter = r));
  }

  stop() {
    this.server.stop(true);
  }
}

export type Events = Event[];

export function recorder(): { emitter: Emitter; events: Events } {
  const events: Events = [];
  return { emitter: new Emitter((evs) => events.push(...evs)), events };
}

export function serverConfig(listener: Listener, configure?: (s: ServerConfig) => void): ServerConfig {
  const s: ServerConfig = { ...defaultServer(), id: 't', host: '127.0.0.1', port: listener.port, tls: false, nick: 'me' };
  configure?.(s);
  return s;
}

/** Wait for an emitted event matching `pred`. */
export async function waitEvent(events: Events, what: string, pred: (e: Event) => boolean): Promise<Event> {
  return must(
    what,
    (async () => {
      for (;;) {
        const e = events.find(pred);
        if (e) return e;
        await realSleep(5);
      }
    })(),
  );
}

export async function waitMessage(events: Events, pred: (m: Message) => boolean): Promise<Message> {
  const e = await waitEvent(events, 'a message event', (e) => e.type === 'message' && pred(e.msg));
  return (e as Extract<Event, { type: 'message' }>).msg;
}

/** Wait until `n` events match `pred`, then return all of them. */
export async function waitCount(events: Events, n: number, pred: (e: Event) => boolean): Promise<Event[]> {
  return must(
    `${n} matching events`,
    (async () => {
      for (;;) {
        const found = events.filter(pred);
        if (found.length >= n) return found;
        await realSleep(5);
      }
    })(),
  );
}

export const count = (events: Events, pred: (e: Event) => boolean) => events.filter(pred).length;

export const isStatus = (e: Event, conn: string, want: Status) => e.type === 'status' && e.conn === conn && e.status === want;

/** Connections spawned through a real `Manager`, as the app does. */
export class Harness {
  readonly db = Db.memory();
  readonly events: Events;
  readonly mgr: Manager;
  readonly listener = new Listener();

  constructor() {
    const { emitter, events } = recorder();
    this.events = events;
    this.mgr = new Manager(emitter, this.db);
  }

  server(configure?: (s: ServerConfig) => void) {
    return serverConfig(this.listener, configure);
  }

  event(pred: (m: Message) => boolean) {
    return waitMessage(this.events, pred);
  }

  /** Stop everything this harness started. */
  async finish() {
    this.mgr.disconnectAll();
    await realSleep(20);
    this.listener.stop();
    this.db.close();
  }
}

/** One actor run directly, so tests can step its clock. */
export class Direct {
  readonly clock = new ManualClock();
  readonly db = Db.memory();
  readonly events: Events;
  readonly listener = new Listener();
  readonly backlog = { n: 0 };
  readonly conn: Connection;
  readonly task: Promise<void>;
  finished = false;

  constructor(configure?: (s: ServerConfig) => void, bind: string | null = null) {
    const { emitter, events } = recorder();
    this.events = events;
    const server = serverConfig(this.listener, configure);
    this.conn = new Connection({ id: server.id, server, bind, generation: 1, db: this.db, backlog: this.backlog }, emitter, null, this.clock);
    this.task = this.conn.start().then(() => void (this.finished = true));
  }

  command(cmd: Cmd) {
    this.conn.command(cmd);
  }

  send(...lines: string[]) {
    this.command({ type: 'send', lines });
  }

  advance(total: number, step?: number) {
    return this.clock.advance(total, step);
  }

  /** Stop the actor. Its QUIT may be waiting on the send pacing, so this steps the clock. */
  async quit() {
    this.command({ type: 'quit' });
    await must(
      'the actor to stop',
      (async () => {
        while (!this.finished) {
          await this.advance(500);
          await realSleep(2);
        }
      })(),
    );
    this.listener.stop();
    this.db.close();
  }

  /** Wait until the actor has handled a line with this command. */
  handled(command: string) {
    return waitMessage(this.events, (m) => m.command === command);
  }

  /** The actor has stopped on its own. */
  stopped() {
    return must('the actor to stop', this.task);
  }

  /** Release the listener after a test that ended without `quit`. */
  dispose() {
    this.listener.stop();
    this.db.close();
  }
}

/** The retry delays announced in Disconnected events, in order. */
export function retryDelays(events: Events): (number | null)[] {
  return events.flatMap((e) => (e.type === 'status' && e.status === 'disconnected' ? [e.retryIn] : []));
}

export const b64decode = (s: string) => Buffer.from(s, 'base64');
export const b64encode = (b: Uint8Array | string) => Buffer.from(b).toString('base64');
