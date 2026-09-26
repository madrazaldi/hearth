import { SvelteMap } from 'svelte/reactivity';
import { api, onIrc } from './api';
import { casefold, escapeTag, isoTime, parsePrefix, splitMessage, escapeAttr, type PrefixMap } from './irc';
import { isIgnored, normalizeMask } from './ignore';
import { stripFormatting } from './format';
import { notify } from './notify';
import { handleMessage, historyLine } from './incoming';
import {
  defaultSettings,
  type ConnStatus,
  type IrcEvent,
  type IrcMessage,
  type Line,
  type LineType,
  type NotifyLevel,
  type Preview,
  type SearchHit,
  type ServerConfig,
  type Settings,
  type Source,
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

export interface ChannelInfo {
  name: string;
  users: number;
  topic: string;
}

export function mkLine(p: Partial<Line> & Pick<Line, 'type'>): Line {
  return { id: ++seq, time: Date.now(), nick: '', text: '', self: false, highlight: false, ...p };
}

export function nextSeq(): number {
  return ++seq;
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
  /** The message the composer is replying to. */
  replyTo = $state<Line | null>(null);
  /** A line to scroll to and flash, set by search and reply quotes. */
  focus = $state<{ time: number; msgid?: string; seq: number } | null>(null);

  // Non-reactive UI memory.
  wantJoined = false;
  /** Channel key, remembered for rejoining. */
  joinKey = '';
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

  /** Stable across restarts; used for saved preferences. */
  get prefKey(): string {
    return `${this.conn.prefKey}|${this.key}`;
  }

  level = $derived.by((): NotifyLevel | 'default' => this.conn.app.settings.bufferNotify[this.prefKey] ?? 'default');

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
    if (this.level === 'none') return 0;
    const everything = this.kind === 'query' || this.level === 'all';
    let n = 0;
    for (let i = this.messages.length - 1; i >= 0; i--) {
      const l = this.messages[i];
      if (l.time <= this.readMarker) break;
      if (!l.self && (l.highlight || (everything && COUNTED.includes(l.type)))) n++;
    }
    return n;
  });

  sortedMembers = $derived.by(() => {
    const rank = this.conn.prefix.symbols;
    const r = (m: Member) => (m.prefix ? rank.indexOf(m.prefix[0]) : rank.length);
    return [...this.members.values()].sort((a, b) => r(a) - r(b) || this.conn.fold(a.nick).localeCompare(this.conn.fold(b.nick)));
  });

  byMsgid = $derived.by(() => {
    const m = new Map<string, Line>();
    for (const l of this.messages) if (l.msgid) m.set(l.msgid, l);
    return m;
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

  /** Add lines from history, skipping ones already present. */
  merge(lines: Line[]) {
    if (!lines.length) return;
    const seen = new Set(this.messages.map(lineKey));
    // Lines without a msgid can differ by a few ms between the live copy and
    // the logged one (our own messages without echo-message), so also match
    // same-content lines that are close in time.
    const loose = new Map<string, number[]>();
    for (const l of this.messages) {
      if (l.msgid) continue;
      const k = `${l.type}|${l.nick}|${l.text}`;
      const times = loose.get(k);
      if (times) times.push(l.time);
      else loose.set(k, [l.time]);
    }
    const fresh = lines.filter((l) => {
      if (seen.has(lineKey(l))) return false;
      if (l.msgid) return true;
      const times = loose.get(`${l.type}|${l.nick}|${l.text}`);
      return !times?.some((t) => Math.abs(t - l.time) < 3000);
    });
    if (!fresh.length) return;
    this.messages = [...this.messages, ...fresh].sort((a, b) => a.time - b.time || a.id - b.id);
  }

  /** Record a +draft/react on the message with `msgid`. */
  react(msgid: string, emoji: string, nick: string) {
    const i = this.messages.findIndex((l) => l.msgid === msgid);
    if (i < 0) return;
    const l = this.messages[i];
    const who = l.reactions?.[emoji] ?? [];
    if (who.some((n) => this.conn.fold(n) === this.conn.fold(nick))) return;
    const next = [...this.messages];
    next[i] = { ...l, reactions: { ...(l.reactions ?? {}), [emoji]: [...who, nick] } };
    this.messages = next;
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
  /** MONITOR state: case-folded nick → online. */
  online = new SvelteMap<string, boolean>();
  channelList = $state.raw<{ loading: boolean; items: ChannelInfo[]; at: number } | null>(null);

  whois: Record<string, Record<string, string>> = {};
  motd: string[] = [];
  listAcc: ChannelInfo[] | null = null;
  /** We sent AWAY because the window sat in the background. */
  autoAway = false;
  /** Saved buffers have been restored, so saving won't clobber them. */
  buffersLoaded = false;
  /** Channel keys from /join, applied when the JOIN comes back. */
  pendingKeys = new Map<string, string>();
  /** labeled-response: label → buffer that issued the command. */
  labels = new Map<string, Buffer>();
  ctcpTimes: number[] = [];
  ctcpWarned = 0;

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

  get prefKey(): string {
    return `${this.serverId}|${this.netid ?? ''}`;
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

  /** Nicks explicitly watched with MONITOR. */
  get watched(): string[] {
    return this.app.settings.watch[this.prefKey] ?? [];
  }

  get canMonitor(): boolean {
    return 'MONITOR' in this.isupport && !this.isControl;
  }

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
    if (b.kind === 'query') {
      this.app.saveBuffersSoon(this);
      if (this.canMonitor && this.status === 'connected') this.send(`MONITOR + ${name}`);
    }
    return b;
  }

  allBuffers(): Buffer[] {
    return [this.server, ...this.buffers.values()];
  }

  send(...lines: string[]) {
    return api.send(this.id, lines).catch((e) => this.app.here(this).push(mkLine({ type: 'error', text: String(e) })));
  }

  /** Send a command whose replies should land in `buf` (via labeled-response). */
  sendFrom(buf: Buffer, line: string) {
    if (!this.caps.has('labeled-response') || line.startsWith('@')) return this.send(line);
    const label = `f${nextSeq()}`;
    this.labels.set(label, buf);
    if (this.labels.size > 200) this.labels.delete(this.labels.keys().next().value!);
    return this.send(`@label=${label} ${line}`);
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
  searchOpen = $state(false);
  searchQuery = $state('');
  browserConn = $state<Conn | null>(null);
  settingsOpen = $state<null | { tab: string; serverId?: string }>(null);
  toasts = $state<Toast[]>([]);
  inputHistory: string[] = [];

  /** conn id → folded channel to open when our JOIN arrives. */
  pendingActive = new Map<string, string>();
  private restoreKey: string | null = null;
  private blurredAt = Date.now();
  private saveBufferTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private previews = new Map<string, Promise<Preview | null>>();

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
    setInterval(() => this.checkAutoAway(), 30_000);
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
    if (buf.kind === 'query' && buf.conn.canMonitor && !this.isWatched(buf.conn, buf.name)) {
      buf.conn.send(`MONITOR - ${buf.name}`);
    }
    this.saveBuffersSoon(buf.conn);
    if (this.active === buf) this.open(list[i - 1] ?? list[i + 1] ?? null);
  }

  /** Open `buf` and scroll to a message, loading history around it if needed. */
  async jumpTo(buf: Buffer, time: number, msgid?: string) {
    this.open(buf);
    const present = buf.messages.some((l) => (msgid ? l.msgid === msgid : l.time === time));
    if (!present) {
      try {
        let msgs: IrcMessage[] = [];
        if (buf.conn.has('draft/chathistory') && buf.conn.status === 'connected') {
          const at = msgid ? `msgid=${msgid}` : `timestamp=${isoTime(time)}`;
          msgs = await api.history(buf.conn.id, 'messages', buf.name, `CHATHISTORY AROUND ${buf.name} ${at} 100`);
        } else if (this.settings.storeLogs) {
          const [before, after] = await Promise.all([
            api.localHistory(buf.conn.id, buf.key, { before: time + 1, limit: 60 }),
            api.localHistory(buf.conn.id, buf.key, { after: time, limit: 60 }),
          ]);
          msgs = [...before, ...after];
        }
        this.applyHistory(buf, msgs);
      } catch (e) {
        console.warn('jump', e);
      }
    }
    buf.focus = { time, msgid, seq: ++seq };
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

  setNotify(buf: Buffer, level: NotifyLevel | 'default') {
    const next = { ...this.settings.bufferNotify };
    if (level === 'default') delete next[buf.prefKey];
    else next[buf.prefKey] = level;
    this.updateSettings({ bufferNotify: next });
  }

  // ─── Ignore & watch ─────────────────────────────────────────

  isIgnored(conn: Conn, src: Source | null | undefined): boolean {
    if (!src || conn.isMe(src.nick)) return false;
    return isIgnored(this.settings.ignores, src, (s) => conn.fold(s));
  }

  ignore(mask: string) {
    const m = normalizeMask(mask);
    if (!m || this.settings.ignores.includes(m)) return;
    this.updateSettings({ ignores: [...this.settings.ignores, m] });
  }

  unignore(mask: string): boolean {
    const m = normalizeMask(mask);
    const next = this.settings.ignores.filter((x) => x !== m && x !== mask);
    if (next.length === this.settings.ignores.length) return false;
    this.updateSettings({ ignores: next });
    return true;
  }

  isWatched(conn: Conn, nick: string): boolean {
    return conn.watched.some((n) => conn.fold(n) === conn.fold(nick));
  }

  watch(conn: Conn, nick: string) {
    if (this.isWatched(conn, nick)) return;
    this.updateSettings({ watch: { ...this.settings.watch, [conn.prefKey]: [...conn.watched, nick] } });
    if (conn.canMonitor) conn.send(`MONITOR + ${nick}`);
  }

  unwatch(conn: Conn, nick: string) {
    const rest = conn.watched.filter((n) => conn.fold(n) !== conn.fold(nick));
    this.updateSettings({ watch: { ...this.settings.watch, [conn.prefKey]: rest } });
    if (conn.canMonitor && !conn.buffer(nick)) {
      conn.send(`MONITOR - ${nick}`);
      conn.online.delete(conn.fold(nick));
    }
  }

  /** Subscribe to presence for watched nicks and open conversations. */
  private syncMonitor(conn: Conn) {
    if (!conn.canMonitor) return;
    const limit = parseInt(conn.isupport.MONITOR) || Infinity;
    const nicks = new Map<string, string>();
    for (const n of conn.watched) nicks.set(conn.fold(n), n);
    for (const b of conn.buffers.values()) if (b.kind === 'query') nicks.set(conn.fold(b.name), b.name);
    conn.online.clear();
    const list = [...nicks.values()].slice(0, limit);
    let chunk: string[] = [];
    const flush = () => {
      if (chunk.length) conn.send(`MONITOR + ${chunk.join(',')}`);
      chunk = [];
    };
    for (const n of list) {
      if (chunk.join(',').length + n.length > 400) flush();
      chunk.push(n);
    }
    flush();
  }

  // ─── Presence ───────────────────────────────────────────────

  setFocused(focused: boolean) {
    this.focused = focused;
    if (!focused) {
      this.blurredAt = Date.now();
      return;
    }
    for (const c of this.conns.values()) {
      if (c.autoAway) {
        c.autoAway = false;
        if (c.away && c.status === 'connected') c.send('AWAY');
      }
    }
  }

  private checkAutoAway() {
    const mins = this.settings.autoAwayMinutes;
    if (!mins || this.focused || Date.now() - this.blurredAt < mins * 60_000) return;
    for (const c of this.conns.values()) {
      if (c.status !== 'connected' || c.isControl || c.away || c.autoAway) continue;
      c.autoAway = true;
      c.send('AWAY :Away from keyboard');
    }
  }

  // ─── Saved buffers ──────────────────────────────────────────

  saveBuffersSoon(conn: Conn) {
    if (!conn.buffersLoaded || conn.isControl) return;
    clearTimeout(this.saveBufferTimers.get(conn.id));
    this.saveBufferTimers.set(
      conn.id,
      setTimeout(() => {
        const list = [...conn.buffers.values()]
          .filter((b) => b.kind === 'query' || (b.kind === 'channel' && b.wantJoined))
          .map((b) => ({ name: b.name, kind: b.kind as 'channel' | 'query', key: b.joinKey }));
        api.saveBuffers(conn.id, list).catch((e) => console.warn('save buffers', e));
      }, 500),
    );
  }

  private async restoreBuffers(conn: Conn) {
    try {
      const saved = await api.loadBuffers(conn.id);
      for (const s of saved) {
        // Bouncers keep their own channel list; only restore conversations there.
        if (s.kind === 'channel' && conn.netid) continue;
        const b = conn.ensure(s.name, s.kind);
        if (s.kind === 'channel') {
          b.wantJoined = true;
          b.joinKey = s.key;
        } else if (!b.messages.length) this.fetchHistory(b, 'latest');
        this.maybeRestoreActive(conn, b.key);
      }
    } catch (e) {
      console.warn('restore buffers', e);
    }
    conn.buffersLoaded = true;
  }

  // ─── Outgoing ────────────────────────────────────────────────

  sendMessage(buf: Buffer, text: string, kind: 'PRIVMSG' | 'NOTICE' = 'PRIVMSG', action = false, replyTo?: string) {
    const conn = buf.conn;
    const target = buf.name;
    const reply = replyTo && conn.canTag('draft/reply') ? replyTo : undefined;
    // Leave room for ":nick!user@host CMD target :" plus CTCP wrapping.
    const budget = 510 - (conn.nick.length + 100 + kind.length + target.length + (action ? 9 : 0));
    splitMessage(text, Math.max(budget, 60)).forEach((part, i) => {
      const body = action ? `\x01ACTION ${part}\x01` : part;
      const tags = reply && i === 0 ? `@+draft/reply=${escapeTag(reply)} ` : '';
      conn.send(`${tags}${kind} ${target} :${body}`);
      if (!conn.has('echo-message')) {
        buf.push(
          mkLine({
            type: action ? 'action' : kind === 'NOTICE' ? 'notice' : 'privmsg',
            nick: conn.nick,
            text: part,
            self: true,
            replyTo: i === 0 ? reply : undefined,
          }),
        );
      }
    });
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

  canReact(buf: Buffer, line: Line): boolean {
    return !!line.msgid && buf.kind !== 'server' && buf.conn.canTag('draft/react') && buf.conn.canTag('draft/reply');
  }

  react(buf: Buffer, line: Line, emoji: string) {
    if (!line.msgid) return;
    const conn = buf.conn;
    if (line.reactions?.[emoji]?.some((n) => conn.isMe(n))) return;
    conn.send(`@+draft/react=${escapeTag(emoji)};+draft/reply=${escapeTag(line.msgid)} TAGMSG ${buf.name}`);
    if (!conn.has('echo-message')) buf.react(line.msgid, emoji, conn.nick);
  }

  canRedact(buf: Buffer, line: Line): boolean {
    if (!line.msgid || !buf.conn.has('draft/message-redaction')) return false;
    if (line.self) return true;
    const me = buf.member(buf.conn.nick);
    const op = buf.conn.prefix.symbols.indexOf('@');
    return !!me?.prefix && op >= 0 && buf.conn.prefix.symbols.indexOf(me.prefix[0]) <= op;
  }

  redact(buf: Buffer, line: Line) {
    if (line.msgid) buf.conn.send(`REDACT ${buf.name} ${line.msgid}`);
  }

  join(conn: Conn, channel: string, key?: string) {
    this.pendingActive.set(conn.id, conn.fold(channel));
    if (key) conn.pendingKeys.set(conn.fold(channel), key);
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

  browseChannels(conn: Conn) {
    this.browserConn = conn;
    const stale = !conn.channelList || (!conn.channelList.loading && Date.now() - conn.channelList.at > 10 * 60_000);
    if (stale && conn.status === 'connected') {
      conn.channelList = { loading: true, items: conn.channelList?.items ?? [], at: Date.now() };
      conn.send('LIST');
    }
  }

  // ─── Link previews ──────────────────────────────────────────

  preview(url: string): Promise<Preview | null> {
    let p = this.previews.get(url);
    if (!p) {
      p = api.linkPreview(url).catch(() => null);
      this.previews.set(url, p);
      if (this.previews.size > 500) this.previews.delete(this.previews.keys().next().value!);
    }
    return p;
  }

  // ─── History & search ──────────────────────────────────────

  async fetchHistory(buf: Buffer, mode: 'latest' | 'before' | 'after') {
    const conn = buf.conn;
    if (buf.kind === 'server') return;
    const remote = conn.has('draft/chathistory') && conn.status === 'connected';
    // The local log only fills gaps before what we have: anything newer
    // arrived live in this session.
    if (!remote && (!this.settings.storeLogs || mode === 'after')) return;
    if (buf.history === 'loading' || (mode === 'before' && buf.history === 'done')) return;
    const max = parseInt(conn.isupport.CHATHISTORY || '100') || 100;
    const first = buf.messages[0];
    const last = buf.messages[buf.messages.length - 1];
    let limit = Math.min(100, max);
    buf.history = 'loading';
    try {
      let msgs: IrcMessage[];
      if (remote) {
        let line: string;
        if (mode === 'after' && last) {
          limit = Math.min(1000, max);
          line = `CHATHISTORY AFTER ${buf.name} timestamp=${isoTime(last.time)} ${limit}`;
        } else if (mode === 'before' && first) {
          line = `CHATHISTORY BEFORE ${buf.name} timestamp=${isoTime(first.time)} ${limit}`;
        } else {
          line = `CHATHISTORY LATEST ${buf.name} * ${limit}`;
        }
        msgs = await api.history(conn.id, 'messages', buf.name, line);
      } else {
        limit = 100;
        msgs = await api.localHistory(conn.id, buf.key, { before: mode === 'before' ? first?.time : undefined, limit });
      }
      this.applyHistory(buf, msgs);
      buf.history = mode !== 'after' && msgs.length < limit ? 'done' : 'idle';
    } catch (e) {
      buf.history = 'idle';
      console.warn('history', buf.name, e);
    }
  }

  private applyHistory(buf: Buffer, msgs: IrcMessage[]) {
    const lines = msgs.map((m) => historyLine(this, buf.conn, m)).filter((l): l is Line => !!l);
    buf.merge(lines);
    for (const m of msgs) {
      const emoji = m.tags['+draft/react'];
      const parent = m.tags['+draft/reply'];
      if (m.command === 'TAGMSG' && emoji && parent && m.source && !this.isIgnored(buf.conn, m.source)) {
        buf.react(parent, emoji, m.source.nick);
      }
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

  /** Search the local log and, where supported, the bouncer. Newest first. */
  async search(query: string, scope: { conn?: Conn; buf?: Buffer }): Promise<SearchHit[]> {
    const q = query.trim();
    if (!q) return [];
    const target = scope.buf && scope.buf.kind !== 'server' ? scope.buf : undefined;
    const tasks: Promise<SearchHit[]>[] = [];
    if (this.settings.storeLogs) {
      tasks.push(api.search(q, scope.conn?.id ?? null, target?.key ?? null, 200).catch(() => []));
    }
    const remote = scope.conn ? [scope.conn] : [...this.conns.values()];
    for (const c of remote) {
      if (!c.has('soju.im/search') || c.status !== 'connected' || c.isControl) continue;
      let attrs = `text=${escapeAttr(q)};limit=100`;
      if (target) attrs += `;in=${escapeAttr(target.name)}`;
      tasks.push(
        api
          .history(c.id, 'search', '*', `SEARCH ${attrs}`)
          .then((msgs) =>
            msgs.map((msg) => {
              const to = msg.params[0] ?? '';
              const peer = c.isChannel(to) || !msg.source || c.isMe(msg.source.nick) ? to : msg.source.nick;
              return { network: c.id, target: c.fold(peer), msg };
            }),
          )
          .catch(() => []),
      );
    }
    const all = (await Promise.all(tasks)).flat();
    const seen = new Set<string>();
    const out: SearchHit[] = [];
    for (const h of all) {
      const k = h.msg.tags.msgid ? `${h.network}|${h.msg.tags.msgid}` : `${h.network}|${h.target}|${h.msg.tags.time}|${h.msg.params[1]}`;
      if (seen.has(k)) continue;
      seen.add(k);
      out.push(h);
    }
    const t = (h: SearchHit) => Date.parse(h.msg.tags.time ?? '') || 0;
    return out.sort((a, b) => t(b) - t(a));
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
          conn.online.clear();
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
        handleMessage(this, conn, ev.msg);
        break;
    }
  }

  private async onConnected(conn: Conn) {
    conn.error = null;
    conn.autoAway = false;
    this.maybeRestoreActive(conn, '');
    if (!this.active) this.open(conn.server);
    if (conn.isControl) return;
    if (!conn.buffersLoaded) await this.restoreBuffers(conn);
    // Rejoin channels on direct connections; bouncers restore them on their own.
    if (!conn.netid) {
      const auto = new Set((conn.config?.autojoin ?? []).map((c) => conn.fold(c.trim())));
      const rejoin = [...conn.buffers.values()].filter((b) => b.kind === 'channel' && b.wantJoined && !auto.has(b.key));
      if (rejoin.length) setTimeout(() => this.rejoin(conn, rejoin), 1500);
    }
    this.syncMonitor(conn);
    setTimeout(() => this.fetchTargets(conn), 800);
  }

  private rejoin(conn: Conn, chans: Buffer[]) {
    for (let i = 0; i < chans.length; i += 10) {
      const group = chans.slice(i, i + 10);
      // Keyed channels must come first so keys line up by position.
      const keyed = group.filter((b) => b.joinKey);
      const names = [...keyed, ...group.filter((b) => !b.joinKey)].map((b) => b.name).join(',');
      conn.send(keyed.length ? `JOIN ${names} ${keyed.map((b) => b.joinKey).join(',')}` : `JOIN ${names}`);
    }
  }

  maybeRestoreActive(conn: Conn, key: string) {
    if (!this.restoreKey) return;
    if (this.restoreKey === `${conn.serverId}|${conn.netid ?? ''}|${key}`) {
      this.restoreKey = null;
      this.open(key ? (conn.buffer(key) ?? conn.server) : conn.server);
    }
  }

  /**
   * Where replies and errors go: the buffer that sent the command (by label),
   * else the active buffer if it's on this connection, else the server buffer.
   */
  here(conn: Conn, label?: string): Buffer {
    const byLabel = label ? conn.labels.get(label) : undefined;
    if (byLabel && (byLabel.kind === 'server' || conn.buffers.get(byLabel.key) === byLabel)) return byLabel;
    return this.active?.conn === conn ? this.active : conn.server;
  }

  highlightWords(conn: Conn): string[] {
    return [conn.nick, ...this.settings.highlightWords];
  }

  maybeNotify(conn: Conn, buf: Buffer, nick: string, text: string, important: boolean) {
    const level = buf.level;
    if (level === 'none' || (!important && level !== 'all')) return;
    const isQuery = buf.kind === 'query';
    if (level === 'default' && (isQuery ? !this.settings.notifyPrivate : !this.settings.notifyHighlights)) return;
    if (this.focused && this.active === buf) return;
    const where = isQuery ? nick : `${nick} in ${buf.name}`;
    notify(`${where} · ${conn.name}`, stripFormatting(text));
  }

  private pruneTyping() {
    const now = Date.now();
    for (const c of this.conns.values())
      for (const b of c.buffers.values()) for (const [n, t] of b.typing) if (t < now) b.typing.delete(n);
  }
}

export const app = new App();
