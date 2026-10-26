// One IRC connection as an actor: it owns the link, negotiates capabilities
// and SASL, keeps it alive, reconnects with backoff, and forwards everything it
// reads to the frontend.
//
// Everything the actor does happens in callbacks on one thread, in order, so
// there is no locking: a line is fully handled before the next is looked at.

import { realClock, sleep, type Clock } from './clock';
import type { SaslMechanism, ServerConfig } from './config';
import type { Emitter } from './events';
import { casefold, message, param, parse, serialize, tag, type Message, type Source } from './message';
import { Scram } from './scram';
import type { Db } from './store';
import { formatMs, nowMs } from './time';
import { connect, type Endpoint, type LineConn } from './transport';

const WANTED_CAPS = new Set([
  'account-notify',
  'account-tag',
  'away-notify',
  'batch',
  'cap-notify',
  'chghost',
  'draft/account-registration',
  'draft/channel-rename',
  'draft/chathistory',
  'draft/event-playback',
  'draft/extended-monitor',
  'draft/message-redaction',
  'draft/metadata-2',
  'draft/multiline',
  'draft/read-marker',
  'echo-message',
  'extended-join',
  'extended-monitor',
  'invite-notify',
  'labeled-response',
  'message-tags',
  'multi-prefix',
  'server-time',
  'setname',
  'soju.im/bouncer-networks',
  'soju.im/search',
  'standard-replies',
  'userhost-in-names',
]);

/** Metadata keys the UI shows (draft/metadata-2). */
const METADATA_KEYS = 'avatar display-name';

/** Lines accepted in one incoming multiline batch before the rest is dropped. */
const MAX_MULTILINE_LINES = 1000;

export type HistoryKind = 'messages' | 'targets' | 'search';

const BATCH_TYPE: Record<HistoryKind, string> = { messages: 'chathistory', targets: 'draft/chathistory-targets', search: 'soju.im/search' };
const HISTORY_KINDS: HistoryKind[] = ['messages', 'targets', 'search'];
const historyCap = (k: HistoryKind) => (k === 'search' ? 'soju.im/search' : 'draft/chathistory');

export type Reply = (result: { ok: Message[] } | { err: string }) => void;

export type Cmd =
  | { type: 'send'; lines: string[] }
  | { type: 'history'; kind: HistoryKind; target: string; line: string; reply: Reply }
  | { type: 'quit'; reason?: string | null }
  | { type: 'reconnect' };

/** What a connection needs from whoever owns it. */
export interface Parent {
  ensureChild(server: ServerConfig, netid: string): void;
  removeChild(server: string, netid: string): void;
  actorFinished(id: string, generation: number): void;
}

export interface Spec {
  id: string;
  server: ServerConfig;
  /** soju network id to bind to (child connections of a bouncer). */
  bind: string | null;
  /** Distinguishes this actor from a later one reusing the same id. */
  generation: number;
  db: Db;
  /** Lines queued for the socket but not yet written. */
  backlog: { n: number };
}

type End =
  | { kind: 'quit' }
  | { kind: 'reconnect' }
  | { kind: 'error'; error: Error; wasRegistered: boolean }
  /** The server's STS policy asks us to reconnect over TLS on this port. */
  | { kind: 'upgrade'; port: number };

interface Pending {
  id: number;
  kind: HistoryKind;
  /** Case-folded target, for matching unlabeled replies. */
  target: string;
  label: string | null;
  /** A reply batch has been matched to this request. */
  assigned: boolean;
  messages: Message[];
  reply: Reply;
}

/** Where the contents of a batch go. */
type Route =
  /** Collected as the reply to a pending request. */
  | { kind: 'collect'; id: number }
  /** Nested inside a collected batch (e.g. multiline): same reply, but its end doesn't complete it. */
  | { kind: 'nested'; id: number }
  /** A labeled-response wrapper around the reply to a pending request. */
  | { kind: 'wrap'; id: number }
  /** A labeled response to a line the frontend sent; its messages carry this label so the UI can route them. */
  | { kind: 'label'; label: string }
  | { kind: 'other' };

/** A draft/multiline batch being collected, sent or received. */
interface Multiline {
  open: Message;
  parts: Message[];
}

/**
 * The batch as one PRIVMSG/NOTICE with newline-separated text. It keeps the
 * opening line's tags (msgid, time, label, and the parent batch), so it's
 * routed exactly as a single message would be.
 */
function combine(ml: Multiline): Message | null {
  const first = ml.parts[0];
  if (!first) return null;
  let text = '';
  ml.parts.forEach((p, i) => {
    if (i > 0 && tag(p, 'draft/multiline-concat') === undefined) text += '\n';
    text += param(p, 1);
  });
  const tags = { ...ml.open.tags };
  for (const [k, v] of Object.entries(first.tags)) {
    if (k !== 'batch' && k !== 'draft/multiline-concat' && !(k in tags)) tags[k] = v;
  }
  return {
    tags,
    source: ml.open.source ?? first.source,
    command: first.command,
    params: [param(ml.open, 2) || param(first, 0), text],
  };
}

type Sasl = { kind: 'plain' } | { kind: 'external' } | { kind: 'scram'; scram: Scram };

/** Whether a raw line is a QUIT, labeled or not, in any letter case. */
function isQuit(line: string): boolean {
  return parse(line)?.command === 'QUIT';
}

/** Whether a message is worth keeping in the local log. */
function loggable(msg: Message): boolean {
  switch (msg.command) {
    case 'PRIVMSG':
    case 'NOTICE': {
      const body = param(msg, 1);
      return !body.startsWith('\x01') || body.startsWith('\x01ACTION');
    }
    case 'TAGMSG':
      return tag(msg, '+draft/react') !== undefined;
    case 'JOIN':
    case 'PART':
    case 'KICK':
    case 'TOPIC':
    case 'MODE':
      return true;
    default:
      return false;
  }
}

/**
 * The nick to try when ours is taken: ours with '_' appended, or, at the
 * 30-character cap where appending changes nothing, with its last
 * character cycled so the same nick isn't retried forever.
 */
export function fallbackNick(nick: string): string {
  const next = [...`${nick}_`].slice(0, 30).join('');
  if (next !== nick) return next;
  const CYCLE = '_0123456789';
  const chars = [...nick];
  const last = chars.pop();
  const at = last === undefined ? -1 : CYCLE.indexOf(last);
  chars.push(CYCLE[at < 0 ? 0 : (at + 1) % CYCLE.length]);
  return chars.join('');
}

export function unescapeIsupport(v: string): string {
  let out = '';
  let rest = v;
  for (;;) {
    const i = rest.indexOf('\\x');
    if (i < 0) break;
    out += rest.slice(0, i);
    const hex = rest.slice(i + 2, i + 4);
    if (/^[0-9a-fA-F]{2}$/.test(hex)) {
      out += String.fromCharCode(parseInt(hex, 16));
      rest = rest.slice(i + 4);
    } else {
      out += '\\x';
      rest = rest.slice(i + 2);
    }
  }
  return out + rest;
}

/** Backoff after `attempt` consecutive failures: 2s, 4s, ... capped at two minutes. */
export function backoffSecs(attempt: number): number {
  return Math.min(2 ** Math.min(attempt, 7), 120);
}

const TICK_MS = 15_000;
/** Token bucket for direct connections: bursts of 8, then one line every 400ms. */
const BURST = 8;
const REFILL_S = 0.4;

class Session {
  private nick: string;
  private registered = false;
  private welcomed = false;
  private available = new Map<string, string>();
  private enabled = new Set<string>();
  private lsBuffer: string[] = [];
  private reqsOutstanding = 0;
  private capEnded = false;
  private isupport = new Map<string, string>();
  private batches = new Map<string, Route>();
  /** Incoming and outgoing draft/multiline batches, by reference. */
  private multiline = new Map<string, Multiline>();
  private sentMultiline = new Map<string, Multiline>();
  private pending: Pending[] = [];
  private nextId = 1;
  private sasl: Sasl | null = null;
  private saslStep = 0;
  private saslIn = '';
  private saslFellBack = false;
  private stsUpgrade: number | null = null;
  /** The user sent QUIT themselves: the link closing is expected. */
  private userQuit = false;

  private link: LineConn | null = null;
  private out: string[] = [];
  private pumping: Promise<void> | null = null;
  private writerStopped = false;
  private tokens = BURST;
  private lastRefill: number;
  private lastRx: number;
  private pinged = false;
  private readonly started: number;
  private cancelTick: (() => void) | null = null;
  private result: End | null = null;
  private done!: (e: End) => void;
  readonly ended = new Promise<End>((resolve) => (this.done = resolve));

  constructor(
    private readonly spec: Spec,
    private readonly emitter: Emitter,
    private readonly parent: Parent | null,
    private readonly clock: Clock,
    /** Whether this link is encrypted, and the port it uses (for STS). */
    private readonly secure: boolean,
    private readonly port: number,
    private readonly throttle: boolean,
  ) {
    const s = spec.server;
    this.nick = s.nick.trim() === '' ? 'hearth' : s.nick.trim();
    this.started = this.lastRx = this.lastRefill = clock.now();
  }

  // ---- Plumbing ----

  attach(link: LineConn) {
    this.link = link;
  }

  /** Begin registration and start watching the link. */
  begin() {
    const s = this.spec.server;
    this.spec.backlog.n = 0;
    this.send('CAP LS 302');
    if (s.password) this.send(`PASS ${s.password}`);
    const nick = this.nick;
    const username = s.username.trim() === '' ? nick : s.username.trim();
    const realname = s.realname.trim() === '' ? nick : s.realname.trim();
    this.send(`NICK ${nick}`);
    this.send(serialize(message('USER', username, '0', '*', realname)));
    const tick = () => {
      if (this.result) return;
      this.onTick();
      if (!this.result) this.cancelTick = this.clock.after(TICK_MS, tick);
    };
    this.cancelTick = this.clock.after(TICK_MS, tick);
  }

  private get id() {
    return this.spec.id;
  }

  send(line: string) {
    if (this.writerStopped) return;
    this.spec.backlog.n++;
    this.out.push(line);
    this.pump();
  }

  private pump() {
    if (!this.pumping) this.pumping = this.writeLoop().finally(() => (this.pumping = null));
  }

  private async writeLoop() {
    while (this.out.length && !this.writerStopped && this.link) {
      const line = this.out.shift()!;
      if (this.throttle) {
        const now = this.clock.now();
        this.tokens = Math.min(BURST, this.tokens + (now - this.lastRefill) / 1000 / REFILL_S);
        this.lastRefill = now;
        if (this.tokens < 1) {
          await sleep(this.clock, (1 - this.tokens) * REFILL_S * 1000);
          // close() gave up on us while we waited; the backlog belongs to the next session now.
          if (this.writerStopped) return;
          this.tokens = 1;
          this.lastRefill = this.clock.now();
        }
        this.tokens -= 1;
      }
      const quitting = isQuit(line);
      try {
        await this.link.sendLine(line);
      } catch (e) {
        if (this.writerStopped) return;
        this.spec.backlog.n--;
        this.writerStopped = true;
        this.finish(this.endedBy(e instanceof Error ? e : new Error(String(e))));
        return;
      }
      if (this.writerStopped) return;
      this.spec.backlog.n--;
      if (quitting) {
        // Nothing may follow a QUIT.
        this.writerStopped = true;
        this.link.end();
        this.finish(this.endedBy(new Error('writer stopped')));
        return;
      }
    }
  }

  /** How the session ends when the link fails: a QUIT the user sent means they wanted out, so it must not be retried like a dropped connection. */
  private endedBy(error: Error): End {
    return this.userQuit ? { kind: 'quit' } : { kind: 'error', error, wasRegistered: this.registered };
  }

  /** Ends the session once; the first reason wins. */
  finish(end: End) {
    if (this.result) return;
    this.result = end;
    this.cancelTick?.();
    for (const p of this.pending.splice(0)) p.reply({ err: 'disconnected' });
    this.done(end);
  }

  get finished() {
    return this.result !== null;
  }

  /** Let the writer flush (e.g. QUIT) before the socket drops. */
  async close() {
    const link = this.link;
    if (!link) return;
    const flushed = this.pumping ?? Promise.resolve();
    let cancel = () => {};
    await Promise.race([flushed, new Promise<void>((resolve) => (cancel = this.clock.after(2000, resolve)))]);
    cancel();
    // A writer still pacing or blocked must not send or count anything once the next session starts.
    this.writerStopped = true;
    this.out.length = 0;
    link.end();
    // A peer that never answers the half-close must not leak the socket.
    const t = setTimeout(() => link.destroy(), 3000);
    t.unref?.();
  }

  private onTick() {
    const idle = this.clock.now() - this.lastRx;
    if (!this.registered && this.clock.now() - this.started > 60_000) {
      this.finish({ kind: 'error', error: new Error('registration timed out'), wasRegistered: false });
    } else if (this.pinged && idle > 150_000) {
      this.finish({ kind: 'error', error: new Error('ping timeout'), wasRegistered: this.registered });
    } else if (!this.pinged && idle > 90_000) {
      this.pinged = true;
      this.send('PING :hearth');
    }
  }

  onLine(line: string) {
    if (this.result) return;
    this.lastRx = this.clock.now();
    this.pinged = false;
    const msg = parse(line);
    if (!msg) return;
    try {
      this.handle(msg);
    } catch (e) {
      if (this.stsUpgrade !== null) this.finish({ kind: 'upgrade', port: this.stsUpgrade });
      else this.finish(this.endedBy(e instanceof Error ? e : new Error(String(e))));
    }
  }

  onClose(error: Error | null) {
    this.finish(this.endedBy(error ?? new Error('connection closed by server')));
  }

  // ---- Commands ----

  command(cmd: Cmd) {
    switch (cmd.type) {
      case 'send':
        for (const l of cmd.lines) {
          this.logOutgoing(l);
          if (isQuit(l)) this.userQuit = true;
          this.send(l);
        }
        break;
      case 'history': {
        if (!this.enabled.has(historyCap(cmd.kind))) {
          cmd.reply({ err: 'server does not support this request' });
          break;
        }
        const id = this.nextId++;
        const label = this.enabled.has('labeled-response') ? `h${id}` : null;
        const line = label ? `@label=${label} ${cmd.line}` : cmd.line;
        this.pending.push({ id, kind: cmd.kind, target: this.fold(cmd.target), label, assigned: false, messages: [], reply: cmd.reply });
        this.send(line);
        break;
      }
      case 'quit':
        this.send(`QUIT :${cmd.reason ?? 'Hearth'}`);
        this.finish({ kind: 'quit' });
        break;
      case 'reconnect':
        this.send('QUIT :Reconnecting');
        this.finish({ kind: 'reconnect' });
        break;
    }
  }

  // ---- Naming ----

  private get casemapping() {
    return this.isupport.get('CASEMAPPING') ?? 'rfc1459';
  }

  private fold(s: string) {
    return casefold(s, this.casemapping);
  }

  private isChannel(name: string) {
    const types = this.isupport.get('CHANTYPES') ?? '#&';
    return name.length > 0 && types.includes(name[0]);
  }

  private isMe(nick: string) {
    return this.fold(nick) === this.fold(this.nick);
  }

  private get isControl() {
    return this.spec.bind === null && this.enabled.has('soju.im/bouncer-networks');
  }

  // ---- Capabilities and SASL ----

  private get saslWanted() {
    const s = this.spec.server;
    switch (s.sasl as SaslMechanism) {
      case 'plain':
        return s.saslPassword !== '';
      case 'external':
        return s.clientCert.trim() !== '';
      default:
        return false;
    }
  }

  private saslOffered(): string[] {
    const v = this.available.get('sasl');
    return v ? v.split(',').filter((m) => m).map((m) => m.toUpperCase()) : [];
  }

  private requestCaps(offered: string[]) {
    const want = [
      ...new Set(
        offered.filter(
          (c) =>
            (WANTED_CAPS.has(c) ||
              (c === 'sasl' && this.saslWanted && !this.registered) ||
              (c === 'soju.im/bouncer-networks-notify' && this.spec.bind === null)) &&
            !this.enabled.has(c),
        ),
      ),
    ].sort();
    // Keep each REQ comfortably under the 512-byte line limit.
    const chunks: string[] = [];
    let chunk = '';
    for (const cap of want) {
      if (chunk.length + cap.length > 400) {
        chunks.push(chunk);
        chunk = '';
      }
      if (chunk) chunk += ' ';
      chunk += cap;
    }
    if (chunk) chunks.push(chunk);
    this.reqsOutstanding += chunks.length;
    for (const c of chunks) this.send(`CAP REQ :${c}`);
  }

  private maybeEndCap() {
    if (this.capEnded || this.registered || this.reqsOutstanding > 0 || this.sasl) return;
    if (this.enabled.has('sasl') && this.saslWanted) {
      this.startSasl();
      return;
    }
    this.finishCap();
  }

  private get saslUser() {
    const s = this.spec.server;
    return s.saslUsername === '' ? s.nick : s.saslUsername;
  }

  private startSasl() {
    const s = this.spec.server;
    const offered = this.saslOffered();
    let mech: Sasl;
    if (s.sasl === 'external') mech = { kind: 'external' };
    else if (offered.includes('SCRAM-SHA-256')) mech = { kind: 'scram', scram: new Scram(this.saslUser, s.saslPassword) };
    else mech = { kind: 'plain' };
    this.beginMechanism(mech);
  }

  private beginMechanism(mech: Sasl) {
    const name = mech.kind === 'plain' ? 'PLAIN' : mech.kind === 'external' ? 'EXTERNAL' : 'SCRAM-SHA-256';
    this.sasl = mech;
    this.saslStep = 0;
    this.saslIn = '';
    this.send(`AUTHENTICATE ${name}`);
  }

  /** Send a SASL payload, base64-encoded and split into 400-byte chunks. */
  private sendSasl(payload: Uint8Array) {
    if (payload.length === 0) {
      this.send('AUTHENTICATE +');
      return;
    }
    const encoded = Buffer.from(payload).toString('base64');
    for (let i = 0; i < encoded.length; i += 400) this.send(`AUTHENTICATE ${encoded.slice(i, i + 400)}`);
    if (encoded.length % 400 === 0) this.send('AUTHENTICATE +');
  }

  private handleAuthenticate(data: string) {
    if (data !== '+') {
      this.saslIn += data;
      if (data.length === 400) return; // More chunks follow.
    }
    const raw = this.saslIn;
    this.saslIn = '';
    const challenge = Buffer.from(raw, 'base64').toString('utf8');
    const user = this.saslUser;
    const password = this.spec.server.saslPassword;
    const step = this.saslStep++;
    const sasl = this.sasl;
    if (!sasl) return;
    try {
      let answer: Uint8Array;
      if (sasl.kind === 'plain') answer = Buffer.from(`${user}\0${user}\0${password}`);
      else if (sasl.kind === 'external') answer = new Uint8Array();
      else if (step === 0) answer = Buffer.from(sasl.scram.clientFirst());
      else if (step === 1) answer = Buffer.from(sasl.scram.clientFinal(challenge));
      else {
        sasl.scram.verify(challenge);
        answer = new Uint8Array();
      }
      this.sendSasl(answer);
    } catch (e) {
      const text = e instanceof Error ? e.message : String(e);
      console.warn(`${this.id}: SASL aborted: ${text}`);
      this.forward(message('FAIL', 'AUTHENTICATE', 'SCRAM', `SCRAM failed: ${text}`));
      this.send('AUTHENTICATE *');
    }
  }

  private finishCap() {
    if (this.capEnded) return;
    this.capEnded = true;
    if (this.spec.bind !== null) this.send(`BOUNCER BIND ${this.spec.bind}`);
    this.send('CAP END');
  }

  private capsEvent() {
    this.emitter.emit({ type: 'caps', conn: this.id, caps: [...this.enabled].sort(), capValues: this.capValues() });
  }

  /** Values of enabled caps that advertise one (e.g. draft/multiline limits). */
  private capValues(): Record<string, string> {
    const out: Record<string, string> = {};
    for (const c of [...this.enabled].sort()) {
      const v = this.available.get(c);
      if (v) out[c] = v;
    }
    return out;
  }

  /** React to an advertised STS policy. Returns true when the connection must be dropped and re-established over TLS. */
  private checkSts(): boolean {
    const value = this.available.get('sts');
    if (value === undefined) return false;
    const attrs = new Map(value.split(',').map((kv) => {
      const i = kv.indexOf('=');
      return (i < 0 ? [kv, ''] : [kv.slice(0, i), kv.slice(i + 1)]) as [string, string];
    }));
    const s = this.spec.server;
    if (!this.secure) {
      if (s.websocket) return false;
      const port = parsePort(attrs.get('port'));
      if (port !== null) {
        this.stsUpgrade = port;
        return true;
      }
    } else if (!s.acceptInvalidCerts) {
      // Only persist a policy learned over a verified TLS connection.
      const duration = attrs.has('duration') && /^-?\d+$/.test(attrs.get('duration')!) ? Number(attrs.get('duration')) : null;
      if (duration !== null) this.spec.db.setStsPolicy(s.host.trim(), this.port, duration);
    }
    return false;
  }

  // ---- Incoming lines ----

  /** Throws to tear the session down. */
  private handle(msg: Message): void {
    // Stamp arrival time once so the UI and the local log agree on it.
    if (!('time' in msg.tags)) msg.tags.time = formatMs(nowMs());
    // Fold multiline batches into one message before any other routing.
    if (msg.command === 'BATCH') {
      const reference = param(msg, 0);
      if (reference.startsWith('+') && param(msg, 1) === 'draft/multiline') {
        this.multiline.set(reference.slice(1), { open: msg, parts: [] });
        return;
      }
      if (reference.startsWith('-')) {
        const ml = this.multiline.get(reference.slice(1));
        if (ml) {
          this.multiline.delete(reference.slice(1));
          const combined = combine(ml);
          if (combined) this.handle(combined);
          return;
        }
      }
    }
    const batchTag = tag(msg, 'batch');
    const ml = batchTag !== undefined ? this.multiline.get(batchTag) : undefined;
    if (ml) {
      if (ml.parts.length < MAX_MULTILINE_LINES) ml.parts.push(msg);
      return;
    }

    // Route messages that belong to a batch.
    if (batchTag !== undefined) {
      const route = this.batches.get(batchTag);
      if (route && (route.kind === 'collect' || route.kind === 'nested' || route.kind === 'wrap') && msg.command !== 'BATCH') {
        if (msg.command === 'FAIL') this.complete(route.id, { err: msg.params.at(-1) ?? '' });
        else this.pending.find((p) => p.id === route.id)?.messages.push(msg);
        return;
      }
      if (route?.kind === 'label' && !('label' in msg.tags)) msg.tags.label = route.label;
    }

    const c = msg.command;
    if (c === 'PING') {
      this.send(serialize(message('PONG', ...msg.params)));
    } else if (c === 'CAP') {
      if (this.handleCap(msg)) throw new Error("upgrading to TLS as required by the server's STS policy");
    } else if (c === 'AUTHENTICATE') {
      this.handleAuthenticate(param(msg, 0));
    } else if (c === '903' || c === '907') {
      this.sasl = null;
      this.finishCap();
    } else if (c === '904' && this.sasl?.kind === 'scram' && !this.saslFellBack) {
      // Accounts without SCRAM credentials (common on Atheme) still accept
      // PLAIN; try it once before giving up.
      const offered = this.saslOffered();
      if (offered.length === 0 || offered.includes('PLAIN')) {
        this.saslFellBack = true;
        this.beginMechanism({ kind: 'plain' });
        return;
      }
      this.sasl = null;
      this.finishCap();
    } else if (c === '902' || c === '904' || c === '905' || c === '906') {
      this.sasl = null;
      this.finishCap();
    } else if (c === '001') {
      this.registered = true;
      this.nick = param(msg, 0);
      this.emitter.emit({ type: 'registered', conn: this.id, nick: this.nick, caps: [...this.enabled].sort(), capValues: this.capValues() });
      if (this.enabled.has('draft/metadata-2') && !this.isControl) this.send(`METADATA * SUB ${METADATA_KEYS}`);
      this.emitter.emit({ type: 'status', conn: this.id, status: 'connected', error: null, retryIn: null });
    } else if (c === '005') {
      for (const token of msg.params.slice(1, -1)) {
        if (token.startsWith('-')) this.isupport.delete(token.slice(1));
        else {
          const i = token.indexOf('=');
          this.isupport.set(i < 0 ? token : token.slice(0, i), i < 0 ? '' : unescapeIsupport(token.slice(i + 1)));
        }
      }
      this.emitter.emit({ type: 'isupport', conn: this.id, isupport: Object.fromEntries([...this.isupport].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) });
    } else if ((c === '376' || c === '422') && !this.welcomed) {
      this.welcomed = true;
      this.onWelcome();
    } else if ((c === '433' || c === '432') && !this.registered) {
      this.nick = fallbackNick(this.nick);
      this.send(`NICK ${this.nick}`);
    } else if (c === 'NICK') {
      if (msg.source && this.isMe(msg.source.nick)) this.nick = param(msg, 0);
    } else if (c === 'BATCH') {
      this.handleBatch(msg);
    } else if (c === 'ACK') {
      const label = tag(msg, 'label');
      const id = label !== undefined ? this.pendingByLabel(label) : null;
      if (id !== null) {
        this.complete(id, { ok: [] });
        return;
      }
    } else if (c === 'FAIL' && (param(msg, 0) === 'CHATHISTORY' || param(msg, 0) === 'SEARCH')) {
      const id = this.pendingForFail(msg);
      if (id !== null) this.complete(id, { err: msg.params.at(-1) ?? '' });
    } else if (c === 'BOUNCER' && param(msg, 0) === 'NETWORK' && this.isControl) {
      const netid = param(msg, 1);
      if (param(msg, 2) === '*') this.parent?.removeChild(this.id, netid);
      else this.parent?.ensureChild(this.spec.server, netid);
    } else if (c === 'ERROR' && this.userQuit) {
      this.forward(msg);
      throw new Error('quit');
    } else if (c === 'ERROR') {
      const reason = param(msg, 0);
      this.forward(msg);
      throw new Error(reason === '' ? 'server closed the connection' : reason);
    }
    this.logLive(msg);
    this.forward(msg);
  }

  private forward(msg: Message) {
    this.emitter.emit({ type: 'message', conn: this.id, msg });
  }

  /** Returns true when an STS upgrade is required. */
  private handleCap(msg: Message): boolean {
    const sub = param(msg, 1).toUpperCase();
    const last = msg.params.at(-1) ?? '';
    const tokens = () =>
      last.split(/\s+/).filter(Boolean).map((tok) => {
        const i = tok.indexOf('=');
        return (i < 0 ? [tok, ''] : [tok.slice(0, i), tok.slice(i + 1)]) as [string, string];
      });
    switch (sub) {
      case 'LS': {
        this.lsBuffer.push(last);
        // A "*" before the list means more lines follow.
        if (msg.params.length >= 4 && param(msg, 2) === '*') return false;
        const all = this.lsBuffer.splice(0).join(' ');
        for (const tok of all.split(/\s+/).filter(Boolean)) {
          const i = tok.indexOf('=');
          this.available.set(i < 0 ? tok : tok.slice(0, i), i < 0 ? '' : tok.slice(i + 1));
        }
        if (this.checkSts()) return true;
        this.requestCaps([...this.available.keys()].sort());
        this.maybeEndCap();
        break;
      }
      case 'ACK':
        for (const c of last.split(/\s+/).filter(Boolean)) {
          if (c.startsWith('-')) this.enabled.delete(c.slice(1));
          else this.enabled.add(c);
        }
        this.reqsOutstanding = Math.max(0, this.reqsOutstanding - 1);
        this.maybeEndCap();
        this.capsEvent();
        break;
      case 'NAK':
        this.reqsOutstanding = Math.max(0, this.reqsOutstanding - 1);
        this.maybeEndCap();
        break;
      case 'NEW': {
        const offered: string[] = [];
        for (const [k, v] of tokens()) {
          this.available.set(k, v);
          offered.push(k);
        }
        if (offered.includes('sts') && this.checkSts()) return true;
        this.requestCaps(offered);
        break;
      }
      case 'DEL':
        for (const c of last.split(/\s+/).filter(Boolean)) {
          this.available.delete(c);
          this.enabled.delete(c);
        }
        this.capsEvent();
        break;
    }
    return false;
  }

  private pendingByLabel(label: string): number | null {
    return this.pending.find((p) => p.label === label)?.id ?? null;
  }

  /** The oldest unlabeled request of `kind` still waiting for its batch, preferring one for `target`. */
  private unassigned(kind: HistoryKind, target: string | null): number | null {
    const open = this.pending.filter((p) => !p.assigned && p.label === null && p.kind === kind);
    return ((target !== null ? open.find((p) => p.target === target) : undefined) ?? open[0])?.id ?? null;
  }

  private pendingForFail(msg: Message): number | null {
    const label = tag(msg, 'label');
    const byLabel = label !== undefined ? this.pendingByLabel(label) : null;
    if (byLabel !== null) return byLabel;
    const kind: HistoryKind = param(msg, 0) === 'SEARCH' ? 'search' : param(msg, 2) === 'TARGETS' ? 'targets' : 'messages';
    // FAIL CHATHISTORY <code> <subcommand> [<target>] :<description>
    const target = msg.params.length > 4 ? this.fold(param(msg, 3)) : null;
    return this.unassigned(kind, target);
  }

  private complete(id: number, result: { ok: Message[] } | { err: string }) {
    const i = this.pending.findIndex((p) => p.id === id);
    if (i < 0) return;
    const [p] = this.pending.splice(i, 1);
    if ('ok' in result && p.kind === 'messages') {
      // Backfill the local log; only messages with an id can be deduplicated.
      for (const m of result.ok) {
        if (tag(m, 'msgid') !== undefined && loggable(m)) this.spec.db.log({ network: this.id, target: p.target, msg: m });
      }
    }
    p.reply(result);
  }

  private handleBatch(msg: Message) {
    const reference = param(msg, 0);
    if (reference.startsWith('+')) {
      const kind = param(msg, 1);
      const parentTag = tag(msg, 'batch');
      const parent = parentTag !== undefined ? this.batches.get(parentTag) : undefined;
      const label = tag(msg, 'label');
      const own = label !== undefined ? this.pendingByLabel(label) : null;
      let route: Route;
      if (kind === 'labeled-response') {
        if (own !== null) route = { kind: 'wrap', id: own };
        else if (label !== undefined) route = { kind: 'label', label };
        else route = { kind: 'other' };
      } else {
        const hk = HISTORY_KINDS.find((k) => BATCH_TYPE[k] === kind);
        let id: number | null = null;
        if (hk) {
          id = own ?? (parent?.kind === 'wrap' ? parent.id : null) ?? this.unassigned(hk, hk === 'messages' ? this.fold(param(msg, 2)) : null);
        }
        if (id !== null) {
          const p = this.pending.find((p) => p.id === id);
          if (p) p.assigned = true;
          route = { kind: 'collect', id };
        } else if (parent?.kind === 'label') route = { kind: 'label', label: parent.label };
        else if (parent?.kind === 'collect' || parent?.kind === 'nested') route = { kind: 'nested', id: parent.id };
        else route = { kind: 'other' };
      }
      this.batches.set(reference.slice(1), route);
    } else if (reference.startsWith('-')) {
      const key = reference.slice(1);
      const route = this.batches.get(key);
      this.batches.delete(key);
      if (route?.kind === 'collect' || route?.kind === 'wrap') {
        const p = this.pending.find((p) => p.id === route.id);
        if (p) this.complete(route.id, { ok: p.messages.splice(0) });
      }
    }
  }

  // ---- Logging ----

  /** The buffer a message belongs in, case-folded, if it should be logged. */
  private logTarget(msg: Message): string | null {
    const src = msg.source;
    const target = param(msg, 0);
    switch (msg.command) {
      case 'PRIVMSG':
      case 'NOTICE':
      case 'TAGMSG': {
        if (!src) return null;
        // Server notices have no user part and usually a dotted name.
        if (src.user === undefined && (src.nick.includes('.') || !this.registered)) return null;
        const statusmsg = this.isupport.get('STATUSMSG') ?? '';
        let t = target;
        while (t.length > 1 && statusmsg.includes(t[0]) && this.isChannel(t.slice(1))) t = t.slice(1);
        if (t === '' || t === '*') return null;
        const peer = this.isChannel(t) || this.isMe(src.nick) ? t : src.nick;
        return this.fold(peer);
      }
      case 'JOIN':
      case 'PART':
      case 'KICK':
      case 'TOPIC':
        return this.fold(target);
      case 'MODE':
        return this.isChannel(target) ? this.fold(target) : null;
      default:
        return null;
    }
  }

  private logLive(msg: Message) {
    if (this.isControl) return;
    const db = this.spec.db;
    if (msg.command === 'REDACT') {
      db.redact(this.id, param(msg, 1));
      return;
    }
    // draft/channel-rename: the channel's saved log follows it.
    if (msg.command === 'RENAME' && this.isChannel(param(msg, 0)) && this.isChannel(param(msg, 1))) {
      db.renameTarget(this.id, this.fold(param(msg, 0)), this.fold(param(msg, 1)));
      return;
    }
    if (!db.logging() || !loggable(msg)) return;
    const target = this.logTarget(msg);
    if (target !== null) db.log({ network: this.id, target, msg });
  }

  /** Without echo-message the server never reflects our own messages, so log them as they are sent. */
  private logOutgoing(line: string) {
    if (this.enabled.has('echo-message') || this.isControl || !this.spec.db.logging()) return;
    let msg = parse(line);
    if (!msg) return;
    // A multiline batch is logged as the one message it reads as.
    if (msg.command === 'BATCH') {
      const reference = param(msg, 0);
      if (reference.startsWith('+') && param(msg, 1) === 'draft/multiline') {
        delete msg.tags.label;
        this.sentMultiline.set(reference.slice(1), { open: msg, parts: [] });
        return;
      }
      const ml = reference.startsWith('-') ? this.sentMultiline.get(reference.slice(1)) : undefined;
      if (!ml) return;
      this.sentMultiline.delete(reference.slice(1));
      const combined = combine(ml);
      if (!combined) return;
      msg = combined;
    } else {
      const sent = this.sentMultiline.get(tag(msg, 'batch') ?? '');
      if (sent) {
        sent.parts.push(msg);
        return;
      }
    }
    if ((msg.command !== 'PRIVMSG' && msg.command !== 'NOTICE') || !loggable(msg)) return;
    msg.source = { nick: this.nick, user: '' } satisfies Source;
    msg.tags.time = formatMs(nowMs());
    this.spec.db.log({ network: this.id, target: this.fold(param(msg, 0)), msg });
  }

  private onWelcome() {
    if (this.isControl) return;
    // Bouncers restore channels themselves; only join when talking to a network directly.
    const s = this.spec.server;
    if (this.spec.bind !== null) return;
    const chans = s.autojoin.filter((c) => c.trim() !== '').map((c) => c.trim());
    for (let i = 0; i < chans.length; i += 10) this.send(`JOIN ${chans.slice(i, i + 10).join(',')}`);
    for (const line of s.perform.filter((l) => l.trim() !== '')) {
      this.send(line.trim().replace(/^\/+/, '').replaceAll('$nick', this.nick));
    }
  }
}

function parsePort(v: string | undefined): number | null {
  if (v === undefined || !/^\d+$/.test(v)) return null;
  const n = Number(v);
  return n <= 65535 ? n : null;
}

export class Connection {
  private session: Session | null = null;
  /** Commands that arrive while no session is listening yet, handled once it is. */
  private early: Cmd[] = [];
  /** Set while waiting out a backoff. */
  private waking: ((quit: boolean) => void) | null = null;
  private quitRequested = false;
  private quitWake: (() => void) | null = null;
  private running: Promise<void> | null = null;

  constructor(
    readonly spec: Spec,
    private readonly emitter: Emitter,
    private readonly parent: Parent | null,
    private readonly clock: Clock = realClock,
  ) {}

  /** Run until quit. Safe to call once. */
  start(): Promise<void> {
    return (this.running ??= this.run());
  }

  command(cmd: Cmd) {
    if (this.session && !this.session.finished) {
      this.session.command(cmd);
    } else if (this.waking) {
      // Offline, waiting to retry.
      if (cmd.type === 'quit') this.waking(true);
      else if (cmd.type === 'reconnect') this.waking(false);
      else if (cmd.type === 'history') cmd.reply({ err: 'not connected' });
    } else {
      if (cmd.type === 'quit') {
        this.quitRequested = true;
        this.quitWake?.();
      }
      this.early.push(cmd);
    }
  }

  private async run() {
    const { spec, emitter } = this;
    let attempt = 0;
    let upgrade: number | null = null;
    for (;;) {
      emitter.emit({ type: 'status', conn: spec.id, status: 'connecting', error: null, retryIn: null });
      const end = await this.runSession(upgrade);
      let error: string | null = null;
      if (end.kind === 'quit') break;
      if (end.kind === 'reconnect') attempt = 0;
      else if (end.kind === 'upgrade') {
        console.info(`${spec.id}: STS policy requires TLS, reconnecting on port ${end.port}`);
        upgrade = end.port;
        attempt = 0;
      } else {
        if (end.wasRegistered) attempt = 0;
        error = formatError(end.error);
      }

      attempt++;
      const delay = error === null ? 0 : backoffSecs(attempt);
      emitter.emit({ type: 'status', conn: spec.id, status: 'disconnected', error, retryIn: error === null ? null : delay });
      if (await this.idle(delay * 1000)) break;
    }
    this.parent?.actorFinished(spec.id, spec.generation);
  }

  /** Waits out a backoff. Resolves true if the connection should stop instead. */
  private idle(ms: number): Promise<boolean> {
    if (this.quitRequested) return Promise.resolve(true);
    // Reconnect commands that arrived while the last session was ending cut the wait short.
    const early = this.early.splice(0);
    if (early.some((c) => c.type === 'quit')) return Promise.resolve(true);
    for (const c of early) if (c.type === 'history') c.reply({ err: 'not connected' });
    if (ms <= 0) return Promise.resolve(false);
    return new Promise<boolean>((resolve) => {
      const cancel = this.clock.after(ms, () => settle(false));
      const settle = (quit: boolean) => {
        if (!this.waking) return;
        this.waking = null;
        cancel();
        resolve(quit);
      };
      this.waking = settle;
      if (early.some((c) => c.type === 'reconnect')) settle(false);
    });
  }

  private async runSession(upgrade: number | null): Promise<End> {
    const { spec, emitter, clock } = this;
    const s = spec.server;
    const host = s.host.trim();
    // A remembered STS policy upgrades plaintext connections before they start.
    const policy = !s.tls && !s.websocket ? spec.db.stsPolicy(host) : null;
    const [tls, port] = upgrade !== null ? [true, upgrade] : policy ? [true, policy.port] : [s.tls, s.port];
    const cert = s.clientCert.trim();
    const ep: Endpoint = {
      host,
      port,
      tls,
      acceptInvalidCerts: s.acceptInvalidCerts,
      websocketPath: s.websocket ? s.websocketPath : null,
      clientCert: tls && cert ? cert : null,
    };

    // The session exists before the link so lines the server sends first are not lost.
    const session = new Session(spec, emitter, this.parent, clock, tls, port, spec.bind === null && !s.websocket);
    let link: LineConn;
    const connecting = connect(ep, { line: (l) => session.onLine(l), close: (e) => session.onClose(e) });
    const aborted = new Promise<'quit'>((resolve) => {
      this.quitWake = () => resolve('quit');
      if (this.quitRequested) resolve('quit');
    });
    try {
      const winner = await Promise.race([connecting, aborted]);
      if (winner === 'quit') {
        connecting.then((l) => l.destroy(), () => {});
        this.early = [];
        return { kind: 'quit' };
      }
      link = winner;
    } catch (e) {
      return { kind: 'error', error: e instanceof Error ? e : new Error(String(e)), wasRegistered: false };
    } finally {
      this.quitWake = null;
    }

    emitter.emit({ type: 'status', conn: spec.id, status: 'registering', error: null, retryIn: null });
    session.attach(link);
    this.session = session;
    session.begin();
    for (const cmd of this.early.splice(0)) session.command(cmd);

    const end = await session.ended;
    await session.close();
    this.session = null;
    return end;
  }
}

/** The text shown for a failed session (anyhow's `{:#}` form: the cause chain on one line). */
function formatError(e: Error): string {
  return e.message;
}
