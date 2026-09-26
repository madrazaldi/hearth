import { SvelteMap } from 'svelte/reactivity';
import { api, onIrc } from './api';
import { casefold, isoTime, msgTime, parseAttrs, parseCtcp, parsePrefix, splitMessage, type PrefixMap } from './irc';
import { mentions, stripFormatting } from './format';
import { notify } from './notify';
import {
  defaultSettings,
  type ConnStatus,
  type IrcEvent,
  type IrcMessage,
  type Line,
  type LineType,
  type ServerConfig,
  type Settings,
} from './types';

let seq = 0;
const MAX_LINES = 4000;
const COUNTED: LineType[] = ['privmsg', 'action', 'notice'];

export type BufferKind = 'server' | 'channel' | 'query';

export interface Member {
  nick: string;
  prefix: string;
  away: boolean;
  account?: string;
  user?: string;
  host?: string;
  realname?: string;
}

export function mkLine(p: Partial<Line> & Pick<Line, 'type'>): Line {
  return { id: ++seq, time: Date.now(), nick: '', text: '', self: false, highlight: false, ...p };
}

function lineKey(l: Line): string {
  return l.msgid ?? `${l.time}|${l.type}|${l.nick}|${l.text}`;
}

export class Buffer {
  readonly conn: Conn;
  readonly kind: BufferKind;
  name = $state('');
  messages = $state.raw<Line[]>([]);
  members = new SvelteMap<string, Member>();
  topic = $state('');
  topicBy = $state('');
  topicTime = $state(0);
  modes = $state('');
  joined = $state(false);
  readMarker = $state(0);
  divider = $state(0);
  history = $state<'idle' | 'loading' | 'done'>('idle');
  typing = new SvelteMap<string, number>();

  // Non-reactive UI memory.
  wantJoined = false;
  draft = '';
  scrollTop: number | null = null;
  namesAcc: Map<string, Member> | null = null;
  lastTypingSent = 0;
  markTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(conn: Conn, kind: BufferKind, name: string) {
    this.conn = conn;
    this.kind = kind;
    this.name = name;
    this.readMarker = conn.caps.has('draft/read-marker') ? 0 : Date.now();
  }

  get key(): string {
    return this.kind === 'server' ? '' : this.conn.fold(this.name);
  }

  get id(): string {
    return `${this.conn.id}\u0000${this.key}`;
  }

  unread = $derived.by(() => {
    let n = 0;
    for (let i = this.messages.length - 1; i >= 0; i--) {
      const l = this.messages[i];
      if (l.time <= this.readMarker) break;
      if (!l.self && COUNTED.includes(l.type)) n++;
    }
    return n;
  });

  highlights = $derived.by(() => {
    let n = 0;
    for (let i = this.messages.length - 1; i >= 0; i--) {
      const l = this.messages[i];
      if (l.time <= this.readMarker) break;
      if (!l.self && (l.highlight || (this.kind === 'query' && COUNTED.includes(l.type)))) n++;
    }
    return n;
  });

  sortedMembers = $derived.by(() => {
    const rank = this.conn.prefix.symbols;
    const r = (m: Member) => (m.prefix ? rank.indexOf(m.prefix[0]) : rank.length);
    return [...this.members.values()].sort((a, b) => r(a) - r(b) || this.conn.fold(a.nick).localeCompare(this.conn.fold(b.nick)));
  });

  push(line: Line) {
    let next = this.messages;
    // Live lines are almost always newest; keep order if a server-time tag says otherwise.
    const last = next[next.length - 1];
    if (!last || last.time <= line.time) next = [...next, line];
    else next = [...next, line].sort((a, b) => a.time - b.time);
    if (next.length > MAX_LINES) {
      next = next.slice(next.length - MAX_LINES + 500);
      this.history = 'idle';
    }
    this.messages = next;
  }

  merge(lines: Line[]) {
    if (!lines.length) return;
    const seen = new Set(this.messages.map(lineKey));
    const fresh = lines.filter((l) => !seen.has(lineKey(l)));
    if (!fresh.length) return;
    this.messages = [...this.messages, ...fresh].sort((a, b) => a.time - b.time || a.id - b.id);
  }

  member(nick: string): Member | undefined {
    return this.members.get(this.conn.fold(nick));
  }

  setMember(m: Member) {
    this.members.set(this.conn.fold(m.nick), m);
  }

  removeMember(nick: string): boolean {
    return this.members.delete(this.conn.fold(nick));
  }
}

export class Conn {
  readonly id: string;
  readonly serverId: string;
  readonly parentId: string | null;
  readonly netid: string | null;
  readonly app: App;
  status = $state<ConnStatus>('connecting');
  error = $state<string | null>(null);
  retryAt = $state(0);
  nick = $state('');
  caps = $state.raw(new Set<string>());
  isupport = $state.raw<Record<string, string>>({});
  buffers = new SvelteMap<string, Buffer>();
  server: Buffer;
  networks = new SvelteMap<string, Record<string, string>>();
  away = $state(false);
  collapsed = $state(false);

  whois: Record<string, Record<string, string>> = {};
  motd: string[] = [];
  lastChannelList = 0;

  constructor(app: App, id: string, serverId: string, parentId: string | null, netid: string | null) {
    this.app = app;
    this.id = id;
    this.serverId = serverId;
    this.parentId = parentId;
    this.netid = netid;
    this.server = new Buffer(this, 'server', '');
  }

  get config(): ServerConfig | undefined {
    return this.app.servers.find((s) => s.id === this.serverId);
  }

  get casemapping() {
    return this.isupport.CASEMAPPING ?? 'rfc1459';
  }

  fold(s: string) {
    return casefold(s, this.casemapping);
  }

  prefix: PrefixMap = $derived(parsePrefix(this.isupport));

  isControl = $derived.by(() => this.netid === null && this.caps.has('soju.im/bouncer-networks'));

  name = $derived.by(() => {
    if (this.netid && this.parentId) {
      const attrs = this.app.conns.get(this.parentId)?.networks.get(this.netid);
      if (attrs?.name) return attrs.name;
    }
    if (this.netid && this.isupport.NETWORK) return this.isupport.NETWORK;
    return this.config?.name || this.isupport.NETWORK || this.config?.host || this.id;
  });

  channelBuffers = $derived.by(() => {
    const all = [...this.buffers.values()];
    const sortKey = (b: Buffer) => this.fold(b.name).replace(/^[#&+!]+/, '');
    const chans = all.filter((b) => b.kind === 'channel').sort((a, b) => sortKey(a).localeCompare(sortKey(b)));
    const queries = all.filter((b) => b.kind === 'query').sort((a, b) => sortKey(a).localeCompare(sortKey(b)));
    return [...chans, ...queries];
  });

  isChannel(name: string): boolean {
    const types = this.isupport.CHANTYPES ?? '#&';
    return name.length > 0 && types.includes(name[0]);
  }

  isMe(nick: string | undefined): boolean {
    return !!nick && this.fold(nick) === this.fold(this.nick);
  }

  buffer(name: string): Buffer | undefined {
    return this.buffers.get(this.fold(name));
  }

  ensure(name: string, kind?: BufferKind): Buffer {
    const existing = this.buffer(name);
    if (existing) return existing;
    const b = new Buffer(this, kind ?? (this.isChannel(name) ? 'channel' : 'query'), name);
    this.buffers.set(this.fold(name), b);
    return b;
  }

  allBuffers(): Buffer[] {
    return [this.server, ...this.buffers.values()];
  }

  send(...lines: string[]) {
    return api.send(this.id, lines).catch((e) => this.server.push(mkLine({ type: 'error', text: String(e) })));
  }

  has(cap: string) {
    return this.caps.has(cap);
  }

  canTag(tag: string): boolean {
    if (!this.caps.has('message-tags')) return false;
    const deny = (this.isupport.CLIENTTAGDENY ?? '').split(',').filter(Boolean);
    if (deny.includes('*')) return deny.includes('-' + tag);
    return !deny.includes(tag);
  }
}

export type Toast = { id: number; text: string; kind: 'info' | 'error' };

export class App {
  servers = $state<ServerConfig[]>([]);
  settings = $state<Settings>(defaultSettings());
  conns = new SvelteMap<string, Conn>();
  active = $state<Buffer | null>(null);
  focused = $state(true);
  ready = $state(false);
  atBottom = $state(true);
  switcherOpen = $state(false);
  settingsOpen = $state<null | { tab: string; serverId?: string }>(null);
  toasts = $state<Toast[]>([]);
  inputHistory: string[] = [];

  private pendingActive = new Map<string, string>();
  private restoreKey: string | null = null;

  async init() {
    const cfg = await api.getConfig();
    this.servers = cfg.servers;
    this.settings = { ...defaultSettings(), ...(cfg.settings ?? {}) };
    try {
      this.restoreKey = localStorage.getItem('hearth.active');
    } catch {}
    await onIrc((events) => this.handleEvents(events));
    await api.reset();
    for (const s of this.servers) if (s.autoconnect) await api.connect(s.id);
    this.ready = true;
    setInterval(() => this.pruneTyping(), 2000);
  }

  toast(text: string, kind: Toast['kind'] = 'info') {
    const t = { id: ++seq, text, kind };
    this.toasts = [...this.toasts, t];
    setTimeout(() => (this.toasts = this.toasts.filter((x) => x.id !== t.id)), 5000);
  }

  /** Root connections in config order, each followed by its bouncer children. */
  get orderedConns(): Conn[] {
    const out: Conn[] = [];
    for (const s of this.servers) {
      const root = this.conns.get(s.id);
      if (!root) continue;
      out.push(root);
      const kids = [...this.conns.values()]
        .filter((c) => c.parentId === s.id)
        .sort((a, b) => a.name.localeCompare(b.name));
      out.push(...kids);
    }
    return out;
  }

  get allBuffers(): Buffer[] {
    return this.orderedConns.flatMap((c) => [c.server, ...c.channelBuffers]);
  }

  // ─── Navigation ──────────────────────────────────────────────

  open(buf: Buffer | null) {
    if (this.active === buf) return;
    if (this.active) this.active.divider = 0;
    this.active = buf;
    if (!buf) return;
    buf.divider = buf.unread > 0 ? buf.readMarker : 0;
    try {
      localStorage.setItem('hearth.active', `${buf.conn.serverId}|${buf.conn.netid ?? ''}|${buf.key}`);
    } catch {}
    if (buf.kind !== 'server' && buf.history === 'idle' && buf.messages.length < 30) this.fetchHistory(buf, 'before');
  }

  step(delta: number, unreadOnly = false) {
    const list = this.allBuffers.filter((b) => !b.conn.collapsed || b.kind === 'server');
    if (!list.length) return;
    let i = this.active ? list.indexOf(this.active) : -1;
    for (let n = 0; n < list.length; n++) {
      i = (i + delta + list.length) % list.length;
      if (!unreadOnly || list[i].unread > 0) return this.open(list[i]);
    }
  }

  markRead(buf: Buffer) {
    const last = buf.messages[buf.messages.length - 1];
    if (!last || last.time <= buf.readMarker) return;
    buf.readMarker = last.time;
    if (buf.kind === 'server' || !buf.conn.has('draft/read-marker')) return;
    if (buf.markTimer) clearTimeout(buf.markTimer);
    buf.markTimer = setTimeout(() => {
      buf.markTimer = null;
      buf.conn.send(`MARKREAD ${buf.name} timestamp=${isoTime(buf.readMarker)}`);
    }, 400);
  }

  closeBuffer(buf: Buffer) {
    if (buf.kind === 'server') return;
    if (buf.kind === 'channel' && buf.joined) buf.conn.send(`PART ${buf.name}`);
    buf.wantJoined = false;
    const list = this.allBuffers;
    const i = list.indexOf(buf);
    buf.conn.buffers.delete(buf.key);
    if (this.active === buf) this.open(list[i - 1] ?? list[i + 1] ?? null);
  }

  // ─── Settings & servers ─────────────────────────────────────

  private saveTimer: ReturnType<typeof setTimeout> | null = null;
  updateSettings(patch: Partial<Settings>) {
    this.settings = { ...this.settings, ...patch };
    if (this.saveTimer) clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => api.saveSettings($state.snapshot(this.settings)), 300);
  }

  async saveServer(server: ServerConfig, connectNow = true): Promise<ServerConfig> {
    const saved = await api.saveServer(server);
    const i = this.servers.findIndex((s) => s.id === saved.id);
    if (i >= 0) this.servers[i] = saved;
    else this.servers.push(saved);
    if (connectNow) {
      await api.disconnect(saved.id);
      await api.connect(saved.id);
    }
    return saved;
  }

  async deleteServer(id: string) {
    await api.deleteServer(id);
    this.servers = this.servers.filter((s) => s.id !== id);
    for (const c of [...this.conns.values()]) if (c.serverId === id) this.removeConn(c.id);
  }

  private removeConn(id: string) {
    const c = this.conns.get(id);
    if (!c) return;
    this.conns.delete(id);
    if (this.active?.conn === c) this.open(this.allBuffers[0] ?? null);
  }

  // ─── Outgoing ────────────────────────────────────────────────

  sendMessage(buf: Buffer, text: string, kind: 'PRIVMSG' | 'NOTICE' = 'PRIVMSG', action = false) {
    const conn = buf.conn;
    const target = buf.name;
    // Leave room for ":nick!user@host CMD target :" plus CTCP wrapping.
    const budget = 510 - (conn.nick.length + 100 + kind.length + target.length + (action ? 9 : 0));
    for (const part of splitMessage(text, Math.max(budget, 60))) {
      const body = action ? `\x01ACTION ${part}\x01` : part;
      conn.send(`${kind} ${target} :${body}`);
      if (!conn.has('echo-message')) {
        buf.push(
          mkLine({
            type: action ? 'action' : kind === 'NOTICE' ? 'notice' : 'privmsg',
            nick: conn.nick,
            text: part,
            self: true,
          }),
        );
      }
    }
    buf.lastTypingSent = 0;
  }

  sendTyping(buf: Buffer, state: 'active' | 'done') {
    if (!this.settings.sendTyping || buf.kind === 'server' || !buf.conn.canTag('typing')) return;
    const now = Date.now();
    if (state === 'active' && now - buf.lastTypingSent < 3000) return;
    if (state === 'done' && buf.lastTypingSent === 0) return;
    buf.lastTypingSent = state === 'active' ? now : 0;
    buf.conn.send(`@+typing=${state} TAGMSG ${buf.name}`);
  }

  join(conn: Conn, channel: string, key?: string) {
    this.pendingActive.set(conn.id, conn.fold(channel));
    const existing = conn.buffer(channel);
    if (existing?.joined) {
      this.open(existing);
      return;
    }
    conn.send(key ? `JOIN ${channel} ${key}` : `JOIN ${channel}`);
  }

  query(conn: Conn, nick: string): Buffer {
    const b = conn.ensure(nick, 'query');
    this.open(b);
    return b;
  }

  // ─── History ────────────────────────────────────────────────

  async fetchHistory(buf: Buffer, mode: 'latest' | 'before' | 'after') {
    const conn = buf.conn;
    if (buf.kind === 'server' || !conn.has('draft/chathistory') || conn.status !== 'connected') return;
    if (buf.history === 'loading' || (mode === 'before' && buf.history === 'done')) return;
    const max = parseInt(conn.isupport.CHATHISTORY || '100') || 100;
    const first = buf.messages[0];
    const last = buf.messages[buf.messages.length - 1];
    let line: string;
    let limit = Math.min(100, max);
    if (mode === 'after' && last) {
      limit = Math.min(1000, max);
      line = `CHATHISTORY AFTER ${buf.name} timestamp=${isoTime(last.time)} ${limit}`;
    } else if (mode === 'before' && first) {
      line = `CHATHISTORY BEFORE ${buf.name} timestamp=${isoTime(first.time)} ${limit}`;
    } else {
      line = `CHATHISTORY LATEST ${buf.name} * ${limit}`;
    }
    buf.history = 'loading';
    try {
      const msgs = await api.history(conn.id, 'messages', buf.name, line);
      const lines = msgs.map((m) => this.historyLine(conn, buf, m)).filter((l): l is Line => !!l);
      buf.merge(lines);
      buf.history = mode !== 'after' && msgs.length < limit ? 'done' : 'idle';
    } catch (e) {
      buf.history = 'idle';
      console.warn('history', buf.name, e);
    }
  }

  private async fetchTargets(conn: Conn) {
    if (!conn.has('draft/chathistory') || conn.isControl) return;
    const now = Date.now();
    const line = `CHATHISTORY TARGETS timestamp=${isoTime(now + 60000)} timestamp=${isoTime(now - 14 * 86400000)} 50`;
    try {
      const msgs = await api.history(conn.id, 'targets', '*', line);
      for (const m of msgs) {
        const target = m.params[1];
        if (!target || conn.isChannel(target) || conn.isMe(target)) continue;
        const b = conn.ensure(target, 'query');
        if (!b.messages.length) this.fetchHistory(b, 'latest');
      }
    } catch (e) {
      console.warn('targets', e);
    }
  }

  // ─── Incoming ────────────────────────────────────────────────

  handleEvents(events: IrcEvent[]) {
    for (const ev of events) {
      try {
        this.handleEvent(ev);
      } catch (e) {
        console.error('event failed', ev, e);
      }
    }
  }

  private handleEvent(ev: IrcEvent) {
    if (ev.type === 'conn-added') {
      if (!this.conns.has(ev.conn)) this.conns.set(ev.conn, new Conn(this, ev.conn, ev.server, ev.parent, ev.netid));
      return;
    }
    if (ev.type === 'conn-removed') return this.removeConn(ev.conn);
    const conn = this.conns.get(ev.conn);
    if (!conn) return;
    switch (ev.type) {
      case 'status': {
        const was = conn.status;
        conn.status = ev.status;
        conn.error = ev.error;
        conn.retryAt = ev.retryIn ? Date.now() + ev.retryIn * 1000 : 0;
        if (ev.status === 'disconnected') {
          for (const b of conn.buffers.values()) {
            b.joined = false;
            b.members.clear();
            b.typing.clear();
          }
          if (was === 'connected' || ev.error) {
            conn.server.push(
              mkLine({
                type: 'error',
                text: `Disconnected${ev.error ? ': ' + ev.error : ''}${ev.retryIn ? ` — retrying in ${ev.retryIn}s` : ''}`,
              }),
            );
          }
        }
        if (ev.status === 'connected') this.onConnected(conn);
        break;
      }
      case 'registered':
        conn.nick = ev.nick;
        conn.caps = new Set(ev.caps);
        break;
      case 'caps':
        conn.caps = new Set(ev.caps);
        break;
      case 'isupport':
        conn.isupport = ev.isupport;
        break;
      case 'message':
        this.live(conn, ev.msg);
        break;
    }
  }

  private onConnected(conn: Conn) {
    conn.error = null;
    this.maybeRestoreActive(conn, '');
    if (!this.active) this.open(conn.server);
    // Rejoin channels on direct connections; bouncers restore them on their own.
    if (!conn.netid) {
      const rejoin = [...conn.buffers.values()].filter((b) => b.kind === 'channel' && b.wantJoined).map((b) => b.name);
      const auto = new Set((conn.config?.autojoin ?? []).map((c) => conn.fold(c.trim())));
      const missing = rejoin.filter((c) => !auto.has(conn.fold(c)));
      if (missing.length) setTimeout(() => conn.send(`JOIN ${missing.join(',')}`), 1500);
    }
    setTimeout(() => this.fetchTargets(conn), 800);
  }

  private maybeRestoreActive(conn: Conn, key: string) {
    if (!this.restoreKey) return;
    if (this.restoreKey === `${conn.serverId}|${conn.netid ?? ''}|${key}`) {
      this.restoreKey = null;
      this.open(key ? conn.buffer(key) ?? conn.server : conn.server);
    }
  }

  /** Where replies and errors go: the active buffer if it's on this connection. */
  private here(conn: Conn): Buffer {
    return this.active?.conn === conn ? this.active : conn.server;
  }

  private highlightWords(conn: Conn): string[] {
    return [conn.nick, ...this.settings.highlightWords];
  }

  /** Build a line for a message replayed from history into `buf`. */
  private historyLine(conn: Conn, buf: Buffer, m: IrcMessage): Line | null {
    const nick = m.source?.nick ?? '';
    const time = msgTime(m.tags);
    const self = conn.isMe(nick);
    const base = { time, nick, self, msgid: m.tags.msgid, account: m.tags.account };
    switch (m.command) {
      case 'PRIVMSG':
      case 'NOTICE': {
        const { type, text } = this.messageBody(m);
        if (type === null) return null;
        return mkLine({ ...base, type, text, highlight: !self && mentions(text, this.highlightWords(conn)) });
      }
      case 'JOIN':
        return mkLine({ ...base, type: 'join', data: { host: `${m.source?.user ?? ''}@${m.source?.host ?? ''}` } });
      case 'PART':
        return mkLine({ ...base, type: 'part', text: m.params[1] ?? '' });
      case 'QUIT':
        return mkLine({ ...base, type: 'quit', text: m.params[0] ?? '' });
      case 'KICK':
        return mkLine({ ...base, type: 'kick', target: m.params[1], text: m.params[2] ?? '' });
      case 'NICK':
        return mkLine({ ...base, type: 'nick', target: m.params[0] });
      case 'TOPIC':
        return mkLine({ ...base, type: 'topic', text: m.params[1] ?? '' });
      case 'MODE':
        return mkLine({ ...base, type: 'mode', text: m.params.slice(1).join(' ') });
      default:
        void buf;
        return null;
    }
  }

  private messageBody(m: IrcMessage): { type: LineType | null; text: string } {
    const raw = m.params[1] ?? '';
    const ctcp = parseCtcp(raw);
    if (ctcp) {
      if (ctcp.command === 'ACTION') return { type: 'action', text: ctcp.args };
      return { type: null, text: '' };
    }
    return { type: m.command === 'NOTICE' ? 'notice' : 'privmsg', text: raw };
  }

  private live(conn: Conn, m: IrcMessage) {
    const src = m.source;
    const nick = src?.nick ?? '';
    const time = msgTime(m.tags);
    const self = conn.isMe(nick);
    const p = m.params;
    const base = { time, nick, self, msgid: m.tags.msgid, account: m.tags.account };

    switch (m.command) {
      case 'PRIVMSG':
      case 'NOTICE':
        return this.onMessage(conn, m, base);

      case 'TAGMSG': {
        const typing = m.tags['+typing'];
        if (!typing || self || !this.settings.showTyping) return;
        const target = p[0];
        const buf = conn.isChannel(target) ? conn.buffer(target) : conn.buffer(nick);
        if (!buf) return;
        if (typing === 'done') buf.typing.delete(nick);
        else buf.typing.set(nick, Date.now() + (typing === 'active' ? 6000 : 30000));
        return;
      }

      case 'JOIN': {
        const chan = p[0];
        const buf = conn.ensure(chan, 'channel');
        if (self) {
          buf.joined = true;
          buf.wantJoined = true;
          buf.members.clear();
          if (this.pendingActive.get(conn.id) === conn.fold(chan)) {
            this.pendingActive.delete(conn.id);
            this.open(buf);
          } else this.maybeRestoreActive(conn, buf.key);
          this.fetchHistory(buf, buf.messages.length ? 'after' : 'latest');
          return;
        }
        buf.setMember({
          nick,
          prefix: '',
          away: false,
          user: src?.user,
          host: src?.host,
          account: conn.has('extended-join') && p[1] !== '*' ? p[1] : undefined,
          realname: p[2],
        });
        buf.push(mkLine({ ...base, type: 'join', data: { host: `${src?.user ?? ''}@${src?.host ?? ''}` } }));
        return;
      }

      case 'PART': {
        const buf = conn.buffer(p[0]);
        if (!buf) return;
        if (self) {
          buf.joined = false;
          buf.members.clear();
        } else buf.removeMember(nick);
        buf.push(mkLine({ ...base, type: 'part', text: p[1] ?? '' }));
        return;
      }

      case 'KICK': {
        const buf = conn.buffer(p[0]);
        if (!buf) return;
        const victim = p[1];
        if (conn.isMe(victim)) {
          buf.joined = false;
          buf.members.clear();
        } else buf.removeMember(victim);
        buf.push(mkLine({ ...base, type: 'kick', target: victim, text: p[2] ?? '', highlight: conn.isMe(victim) }));
        return;
      }

      case 'QUIT': {
        for (const buf of conn.buffers.values()) {
          if (buf.removeMember(nick) || (buf.kind === 'query' && conn.fold(buf.name) === conn.fold(nick))) {
            buf.push(mkLine({ ...base, type: 'quit', text: p[0] ?? '' }));
            buf.typing.delete(nick);
          }
        }
        return;
      }

      case 'NICK': {
        const next = p[0];
        if (self) conn.nick = next;
        for (const buf of conn.buffers.values()) {
          const mem = buf.member(nick);
          const isQuery = buf.kind === 'query' && conn.fold(buf.name) === conn.fold(nick);
          if (mem) {
            buf.removeMember(nick);
            buf.setMember({ ...mem, nick: next });
          }
          if (mem || isQuery || (self && buf.kind === 'query')) buf.push(mkLine({ ...base, type: 'nick', target: next }));
          if (isQuery && !conn.buffer(next)) {
            conn.buffers.delete(buf.key);
            buf.name = next;
            conn.buffers.set(conn.fold(next), buf);
          }
        }
        if (self) conn.server.push(mkLine({ ...base, type: 'nick', target: next }));
        return;
      }

      case 'MODE':
        return this.onMode(conn, m, base);

      case 'TOPIC': {
        const buf = conn.buffer(p[0]);
        if (!buf) return;
        buf.topic = p[1] ?? '';
        buf.topicBy = nick;
        buf.topicTime = time;
        buf.push(mkLine({ ...base, type: 'topic', text: p[1] ?? '' }));
        return;
      }

      case 'INVITE': {
        const text = `invited ${conn.isMe(p[0]) ? 'you' : p[0]} to ${p[1]}`;
        this.here(conn).push(mkLine({ ...base, type: 'invite', target: p[1], text, highlight: conn.isMe(p[0]) }));
        return;
      }

      case 'AWAY': {
        const away = !!p[0];
        for (const buf of conn.buffers.values()) {
          const mem = buf.member(nick);
          if (mem && mem.away !== away) buf.setMember({ ...mem, away });
        }
        if (self) conn.away = away;
        return;
      }

      case 'ACCOUNT':
        return this.updateMember(conn, nick, { account: p[0] === '*' ? undefined : p[0] });
      case 'CHGHOST':
        return this.updateMember(conn, nick, { user: p[0], host: p[1] });
      case 'SETNAME':
        return this.updateMember(conn, nick, { realname: p[0] });

      case 'MARKREAD': {
        const buf = conn.buffer(p[0]);
        const ts = p[1]?.startsWith('timestamp=') ? Date.parse(p[1].slice(10)) : NaN;
        if (buf && !Number.isNaN(ts) && ts > buf.readMarker) buf.readMarker = ts;
        return;
      }

      case 'BOUNCER': {
        if (p[0] === 'NETWORK' && conn.isControl) {
          if (p[2] === '*') conn.networks.delete(p[1]);
          else conn.networks.set(p[1], { ...(conn.networks.get(p[1]) ?? {}), ...parseAttrs(p[2]) });
        }
        return;
      }

      case 'REDACT': {
        const buf = conn.isChannel(p[0]) ? conn.buffer(p[0]) : conn.buffer(self ? p[0] : nick);
        if (!buf) return;
        buf.messages = buf.messages.map((l) => (l.msgid === p[1] ? { ...l, redacted: true } : l));
        return;
      }

      case 'ERROR':
        conn.server.push(mkLine({ ...base, nick: '', type: 'error', text: p[0] ?? 'Connection error' }));
        return;

      case 'FAIL':
      case 'WARN':
      case 'NOTE': {
        const text = p[p.length - 1] ?? '';
        const target = m.command === 'NOTE' ? conn.server : this.here(conn);
        if (p[0] === 'CHATHISTORY' || p[0] === 'MARKREAD') return;
        target.push(mkLine({ ...base, nick: '', type: m.command === 'NOTE' ? 'info' : 'error', text: `${p[0]}: ${text}` }));
        return;
      }

      case 'PING':
      case 'PONG':
      case 'CAP':
      case 'AUTHENTICATE':
      case 'BATCH':
        return;
    }

    if (/^\d{3}$/.test(m.command)) this.onNumeric(conn, m, time);
  }

  private onMessage(conn: Conn, m: IrcMessage, base: Pick<Line, 'time' | 'nick' | 'self' | 'msgid' | 'account'>) {
    const p = m.params;
    const nick = base.nick;
    const src = m.source;
    const statusmsg = conn.isupport.STATUSMSG ?? '';
    let target = p[0] ?? '';
    while (target && statusmsg.includes(target[0]) && conn.isChannel(target.slice(1))) target = target.slice(1);
    const raw = p[1] ?? '';
    const ctcp = parseCtcp(raw);
    const isNotice = m.command === 'NOTICE';

    // Server-originated notices: no source, a server name as source, or pre-registration.
    const fromServer = !src || target === '*' || (!src.user && (src.nick.includes('.') || !conn.nick));
    if (fromServer && !conn.isChannel(target)) {
      conn.server.push(mkLine({ ...base, type: 'notice', text: raw }));
      return;
    }

    if (ctcp && ctcp.command !== 'ACTION') {
      if (!isNotice && !base.self) {
        const reply = this.ctcpReply(ctcp.command, ctcp.args);
        if (reply !== null) conn.send(`NOTICE ${nick} :\x01${ctcp.command}${reply ? ' ' + reply : ''}\x01`);
        this.here(conn).push(
          mkLine({ ...base, type: 'info', text: `CTCP ${ctcp.command} request from ${nick}` }),
        );
      } else if (isNotice) {
        this.here(conn).push(mkLine({ ...base, type: 'info', text: `CTCP ${ctcp.command} reply from ${nick}: ${ctcp.args}` }));
      }
      return;
    }

    const type: LineType = ctcp ? 'action' : isNotice ? 'notice' : 'privmsg';
    const text = ctcp ? ctcp.args : raw;
    const isChan = conn.isChannel(target);
    let buf: Buffer;
    if (isChan) buf = conn.ensure(target, 'channel');
    else {
      const peer = base.self ? target : nick;
      const existing = conn.buffer(peer);
      if (isNotice && !existing) {
        // Private notices without a conversation land where the user is looking.
        const here = this.here(conn);
        here.push(mkLine({ ...base, type, text, target: base.self ? target : undefined, highlight: !base.self }));
        if (!base.self) this.maybeNotify(conn, here, nick, text, true);
        return;
      }
      buf = existing ?? conn.ensure(peer, 'query');
    }

    const highlight = !base.self && mentions(text, this.highlightWords(conn));
    buf.typing.delete(nick);
    if (!isChan) {
      const mem = buf.member(nick);
      if (!mem && !base.self) buf.setMember({ nick, prefix: '', away: false, user: src?.user, host: src?.host });
    }
    buf.push(mkLine({ ...base, type, text, highlight }));
    if (!base.self) this.maybeNotify(conn, buf, nick, text, highlight || !isChan);
  }

  private ctcpReply(cmd: string, args: string): string | null {
    switch (cmd) {
      case 'VERSION':
        return 'Hearth — a modern IRC client';
      case 'PING':
        return args;
      case 'TIME':
        return new Date().toString();
      case 'CLIENTINFO':
        return 'ACTION CLIENTINFO PING TIME VERSION';
      default:
        return null;
    }
  }

  private maybeNotify(conn: Conn, buf: Buffer, nick: string, text: string, important: boolean) {
    if (!important) return;
    const isQuery = buf.kind === 'query';
    if (isQuery ? !this.settings.notifyPrivate : !this.settings.notifyHighlights) return;
    if (this.focused && this.active === buf) return;
    const where = isQuery ? nick : `${nick} in ${buf.name}`;
    notify(`${where} · ${conn.name}`, stripFormatting(text));
  }

  private updateMember(conn: Conn, nick: string, patch: Partial<Member>) {
    for (const buf of conn.buffers.values()) {
      const mem = buf.member(nick);
      if (mem) buf.setMember({ ...mem, ...patch });
    }
  }

  private onMode(conn: Conn, m: IrcMessage, base: Pick<Line, 'time' | 'nick' | 'self' | 'msgid' | 'account'>) {
    const [target, modestr = '', ...args] = m.params;
    if (!conn.isChannel(target)) {
      if (conn.isMe(target)) conn.server.push(mkLine({ ...base, type: 'mode', text: [modestr, ...args].join(' ') }));
      return;
    }
    const buf = conn.buffer(target);
    if (!buf) return;
    const [a = '', b = '', c = ''] = (conn.isupport.CHANMODES ?? 'beI,k,l,imnpst').split(',');
    const { modes: pmodes, symbols } = conn.prefix;
    let adding = true;
    let ai = 0;
    for (const ch of modestr) {
      if (ch === '+' || ch === '-') {
        adding = ch === '+';
        continue;
      }
      const pi = pmodes.indexOf(ch);
      if (pi >= 0) {
        const who = args[ai++];
        const mem = who ? buf.member(who) : undefined;
        if (mem) {
          const sym = symbols[pi];
          let prefix = mem.prefix.replace(sym, '');
          if (adding) prefix = [...(prefix + sym)].sort((x, y) => symbols.indexOf(x) - symbols.indexOf(y)).join('');
          buf.setMember({ ...mem, prefix });
        }
      } else if (a.includes(ch) || b.includes(ch) || (c.includes(ch) && adding)) ai++;
    }
    buf.push(mkLine({ ...base, type: 'mode', text: [modestr, ...args].join(' ') }));
  }

  private onNumeric(conn: Conn, m: IrcMessage, time: number) {
    const p = m.params;
    const rest = p.slice(1);
    const trailing = p[p.length - 1] ?? '';
    const info = (text: string, buf: Buffer = conn.server, type: LineType = 'info') =>
      buf.push(mkLine({ time, type, text }));

    switch (m.command) {
      case '001':
        return info(trailing);
      case '002':
      case '003':
      case '004':
      case '005':
      case '250':
      case '251':
      case '252':
      case '253':
      case '254':
      case '255':
      case '265':
      case '266':
      case '396':
        return;
      case '375':
        conn.motd = [];
        return;
      case '372':
        conn.motd.push(trailing.replace(/^- ?/, ''));
        return;
      case '376': {
        if (conn.motd.length) conn.server.push(mkLine({ time, type: 'info', text: '', data: { motd: conn.motd.join('\n') } }));
        conn.motd = [];
        return;
      }
      case '422':
        return;
      case '221':
        return info(`Your user modes: ${p[1]}`);
      case '324': {
        const buf = conn.buffer(p[1]);
        if (buf) buf.modes = p.slice(2).join(' ');
        return;
      }
      case '329':
        return;
      case '331': {
        const buf = conn.buffer(p[1]);
        if (buf) buf.topic = '';
        return;
      }
      case '332': {
        const buf = conn.buffer(p[1]);
        if (buf) buf.topic = trailing;
        return;
      }
      case '333': {
        const buf = conn.buffer(p[1]);
        if (buf) {
          buf.topicBy = (p[2] ?? '').split('!')[0];
          buf.topicTime = parseInt(p[3] ?? '0') * 1000;
        }
        return;
      }
      case '353': {
        const buf = conn.buffer(p[2]);
        if (!buf) return;
        buf.namesAcc ??= new Map();
        const syms = conn.prefix.symbols;
        for (const entry of trailing.split(' ')) {
          if (!entry) continue;
          let i = 0;
          while (i < entry.length && syms.includes(entry[i])) i++;
          const prefix = entry.slice(0, i);
          const [nickUser, host] = entry.slice(i).split('@');
          const [nick, user] = nickUser.split('!');
          const prev = buf.member(nick);
          buf.namesAcc.set(conn.fold(nick), { nick, prefix, away: prev?.away ?? false, user, host, account: prev?.account });
        }
        return;
      }
      case '366': {
        const buf = conn.buffer(p[1]);
        if (!buf?.namesAcc) return;
        buf.members.clear();
        for (const [k, v] of buf.namesAcc) buf.members.set(k, v);
        buf.namesAcc = null;
        return;
      }
      case '301': {
        const nick = p[1];
        const w = conn.whois[conn.fold(nick)];
        if (w) w.away = trailing;
        else {
          const buf = conn.buffer(nick);
          if (buf) info(`${nick} is away: ${trailing}`, buf);
        }
        return;
      }
      case '305':
        conn.away = false;
        return info('You are no longer marked as away', this.here(conn));
      case '306':
        conn.away = true;
        return info('You have been marked as away', this.here(conn));
      case '311':
      case '314': {
        const nick = p[1];
        conn.whois[conn.fold(nick)] = { nick, user: p[2], host: p[3], realname: trailing };
        return;
      }
      case '312':
      case '313':
      case '317':
      case '319':
      case '320':
      case '330':
      case '338':
      case '378':
      case '379':
      case '671':
      case '276': {
        const w = conn.whois[conn.fold(p[1])];
        if (!w) return info(rest.join(' '), this.here(conn));
        if (m.command === '312') w.server = `${p[2]} — ${trailing}`;
        else if (m.command === '313') w.oper = trailing;
        else if (m.command === '317') {
          w.idle = p[2];
          if (p[3] && /^\d+$/.test(p[3])) w.signon = p[3];
        } else if (m.command === '319') w.channels = ((w.channels ?? '') + ' ' + trailing).trim();
        else if (m.command === '330') w.account = p[2];
        else if (m.command === '671' || m.command === '276') w.secure = trailing;
        else if (m.command === '338') w.actual = p.slice(2, -1).join(' ');
        else w[`extra${m.command}`] = trailing;
        return;
      }
      case '318':
      case '369': {
        const key = conn.fold(p[1]);
        const w = conn.whois[key];
        delete conn.whois[key];
        if (w) this.here(conn).push(mkLine({ time, type: 'whois', nick: w.nick, text: '', data: w }));
        return;
      }
      case '341':
        return info(`Invited ${p[1]} to ${p[2]}`, conn.buffer(p[2]) ?? this.here(conn));
      case '900':
        return info(trailing);
      case '903':
        return;
      case '904':
      case '905':
      case '906':
      case '902':
        return info(`SASL: ${trailing}`, conn.server, 'error');
      case '433':
        return info(`Nickname ${p[1]} is already in use`, this.here(conn), 'error');
    }

    const code = parseInt(m.command);
    if (code >= 400 && code < 600) return info(rest.filter((x, i) => i < rest.length - 1 || x).join(' '), this.here(conn), 'error');
    info(rest.join(' '));
  }

  private pruneTyping() {
    const now = Date.now();
    for (const c of this.conns.values())
      for (const b of c.buffers.values()) for (const [n, t] of b.typing) if (t < now) b.typing.delete(n);
  }
}

export const app = new App();
