// Owns every live connection. A server entry maps to one connection, or, for a
// soju bouncer, a control connection plus one child per network.

import { realClock, type Clock } from './clock';
import { Connection, type Cmd, type HistoryKind, type Parent } from './client';
import type { ServerConfig } from './config';
import type { Emitter } from './events';
import type { Message } from './message';
import type { Db } from './store';

/**
 * Lines a connection may have waiting for the socket before further sends are
 * refused. At the direct-connection pace of 2.5 lines/s this is minutes of
 * backlog, which only a flood (or a runaway paste) produces.
 */
export const MAX_BACKLOG = 300;

const HISTORY_TIMEOUT_MS = 30_000;

/** The part of a connection the manager talks to. */
export interface Target {
  command(cmd: Cmd): void;
}

interface Entry {
  target: Target;
  generation: number;
  server: string;
  backlog: { n: number };
}

export function childId(server: string, netid: string): string {
  return `${server}/${netid}`;
}

export class Manager implements Parent {
  private conns = new Map<string, Entry>();
  private generation = 1;
  /** Connections that are still winding down, so shutdown can wait for their QUITs to go out. */
  private running = new Set<Promise<void>>();

  constructor(
    private readonly emitter: Emitter,
    readonly db: Db,
    private readonly clock: Clock = realClock,
  ) {}

  private spawn(id: string, server: ServerConfig, bind: string | null) {
    if (this.conns.has(id)) return;
    const generation = this.generation++;
    const backlog = { n: 0 };
    const conn = new Connection({ id, server, bind, generation, db: this.db, backlog }, this.emitter, this, this.clock);
    this.conns.set(id, { target: conn, generation, server: server.id, backlog });
    this.emitter.emit({ type: 'conn-added', conn: id, server: server.id, parent: bind === null ? null : server.id, netid: bind });
    const done = conn.start().finally(() => this.running.delete(done));
    this.running.add(done);
  }

  connect(server: ServerConfig) {
    this.spawn(server.id, server, null);
  }

  ensureChild(server: ServerConfig, netid: string) {
    this.spawn(childId(server.id, netid), server, netid);
  }

  removeChild(server: string, netid: string) {
    this.quit(childId(server, netid), null);
  }

  private quit(id: string, reason: string | null) {
    const entry = this.conns.get(id);
    if (!entry) return;
    this.conns.delete(id);
    entry.target.command({ type: 'quit', reason });
    this.emitter.emit({ type: 'conn-removed', conn: id });
  }

  /** Disconnect a server and all of its bouncer children. */
  disconnect(server: string, reason: string | null = null) {
    for (const [id, e] of [...this.conns]) if (e.server === server) this.quit(id, reason);
  }

  disconnectAll() {
    for (const id of [...this.conns.keys()]) this.quit(id, null);
  }

  reconnect(server: string) {
    for (const e of this.conns.values()) if (e.server === server) e.target.command({ type: 'reconnect' });
  }

  actorFinished(id: string, generation: number) {
    if (this.conns.get(id)?.generation === generation) {
      this.conns.delete(id);
      this.emitter.emit({ type: 'conn-removed', conn: id });
    }
  }

  /** Resolves once every connection has finished closing, or after `ms`. */
  async drain(ms: number): Promise<void> {
    let cancel = () => {};
    const timeout = new Promise<void>((resolve) => (cancel = this.clock.after(ms, resolve)));
    await Promise.race([Promise.allSettled([...this.running]), timeout]);
    cancel();
  }

  /** Whether a connection with this id is live. */
  has(id: string): boolean {
    return this.conns.has(id);
  }

  private entry(conn: string): Entry {
    const e = this.conns.get(conn);
    if (!e) throw new Error('no such connection');
    return e;
  }

  send(conn: string, lines: string[]) {
    const backlog = this.conns.get(conn)?.backlog.n ?? 0;
    if (backlog >= MAX_BACKLOG) throw new Error('too many lines waiting to be sent; try again in a moment');
    const clean = lines.map((l) => l.replace(/[\r\n]/g, ' ')).filter((l) => l.trim() !== '');
    this.entry(conn).target.command({ type: 'send', lines: clean });
  }

  history(conn: string, kind: HistoryKind, target: string, line: string): Promise<Message[]> {
    const entry = this.entry(conn);
    return new Promise((resolve, reject) => {
      const cancel = this.clock.after(HISTORY_TIMEOUT_MS, () => reject(new Error('history request timed out')));
      entry.target.command({
        type: 'history',
        kind,
        target,
        line,
        reply: (r) => {
          cancel();
          if ('ok' in r) resolve(r.ok);
          else reject(new Error(r.err));
        },
      });
    });
  }

  /** Test hook: register a stand-in connection. */
  adopt(id: string, target: Target, server: string, generation: number): { n: number } {
    const backlog = { n: 0 };
    this.conns.set(id, { target, generation, server, backlog });
    return backlog;
  }
}
