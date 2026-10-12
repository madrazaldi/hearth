import { SvelteMap } from 'svelte/reactivity';
import { api, onIrc } from './api';
import { casefold, escapeTag, parsePrefix, splitMessage, type PrefixMap } from './irc';
import { isIgnored, normalizeMask } from './ignore';
import { mentions, stripFormatting } from './format';
import { notify } from './notify';
import { handleMessage } from './incoming';
import { fetchHistory, fetchTargets, fillGap, loadAround, mergeLines, type Gap } from './history';
import { search, type SearchPage } from './search';
import { isWatched, syncMonitor, unwatch, watch } from './monitor';
import { rejoinCommands, restoreBuffers, saveBuffersSoon } from './saved';
import { countHighlights, countUnread, markRead } from './unread';
import { joinParts, multilineBatches, parseMultilineCap } from './multiline';
import {
  defaultSettings,
  type ConnStatus,
  type IrcEvent,
  type IrcMessage,
  type Line,
  type NetworkHighlights,
  type NotifyLevel,
  type Preview,
  type SearchHit,
  type ServerConfig,
  type Settings,
  type Source,
} from './types';

let seq = 0;
// Rows are virtualized, so the cap is about memory and the O(n) derivations
// per new line (unread counts, msgid index, timeline), not DOM size.
const MAX_LINES = 20_000;
/** Trimmed in chunks so a busy channel doesn't slice the array on every line. */
const TRIM_TO = 18_000;

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

/** What someone has published about themselves with draft/metadata-2. */
export interface Profile {
  /** An image URL; load it with `App.avatar`, never directly. */
  avatar?: string;
  displayName?: string;
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
  /** Unloaded stretches between loaded lines, oldest first. */
  gaps = $state.raw<Gap[]>([]);
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
  /** The history request in flight, if any. */
  pending: Promise<void> | null = null;
  fillingGap = false;
  gapRetryAt = 0;

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

  unread = $derived.by(() => countUnread(this.messages, this.readMarker));

  highlights = $derived.by(() => {
    if (this.level === 'none') return 0;
    return countHighlights(this.messages, this.readMarker, this.kind === 'query' || this.level === 'all');
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
    this.stampPrefix(line);
    let next = this.messages;
    // Live lines are almost always newest; keep order if a server-time tag says otherwise.
    const last = next[next.length - 1];
    if (!last || last.time <= line.time) next = [...next, line];
    else next = [...next, line].sort((a, b) => a.time - b.time);
    if (next.length > MAX_LINES) {
      next = next.slice(next.length - TRIM_TO);
      this.history = 'idle';
      // Paging back from the new oldest line covers a gap that was cut off.
      if (this.gaps.length) this.gaps = this.gaps.filter((g) => g.after >= next[0].time);
    }
    this.messages = next;
  }

  /** Add lines from history, skipping ones already present. */
  merge(lines: Line[]) {
    for (const l of lines) this.stampPrefix(l);
    const next = mergeLines(this.messages, lines);
    if (next) this.messages = next;
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

  /**
   * Stamped on the line rather than looked up when rendering, so rows don't
   * re-render on every join and mode change. History gets the current prefix.
   */
  private stampPrefix(l: Line) {
    if (this.kind !== 'channel' || l.prefix !== undefined || !l.nick) return;
    if (l.type !== 'privmsg' && l.type !== 'action' && l.type !== 'notice') return;
    const p = this.member(l.nick)?.prefix[0];
    if (p) l.prefix = p;
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
  /** Values of enabled caps that have one, e.g. draft/multiline limits. */
  capValues = $state.raw<Record<string, string>>({});
  isupport = $state.raw<Record<string, string>>({});
  buffers = new SvelteMap<string, Buffer>();
  server: Buffer;
  networks = new SvelteMap<string, Record<string, string>>();
  away = $state(false);
  collapsed = $state(false);
  /** MONITOR state: case-folded nick → online. */
  online = new SvelteMap<string, boolean>();
  channelList = $state.raw<{ loading: boolean; items: ChannelInfo[]; at: number } | null>(null);
  /** draft/metadata-2 key/values: case-folded nick (or channel) → key → value. */
  metadata = new SvelteMap<string, Record<string, string>>();

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
  /** The bouncer's SEARCH needs `in`, so search buffer by buffer. */
  searchPerTarget = false;
  ctcpWarned = 0;
  /**
   * draft/account-registration: credentials from /register, kept in memory
   * only until the server answers, so they can be offered for SASL.
   */
  registration: { account: string; password: string } | null = null;

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
      // Servers only push markers for joined channels; queries must ask.
      if (this.has('draft/read-marker') && this.status === 'connected') this.send(`MARKREAD ${name}`);
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
    const label = line.startsWith('@') ? undefined : this.labelFor(buf);
    return this.send(label ? `@label=${label} ${line}` : line);
  }

  /** A fresh label whose replies go to `buf`, if the server supports labels. */
  labelFor(buf: Buffer): string | undefined {
    if (!this.caps.has('labeled-response')) return undefined;
    const label = `f${nextSeq()}`;
    this.labels.set(label, buf);
    if (this.labels.size > 200) this.labels.delete(this.labels.keys().next().value!);
    return label;
  }

  profile(nick: string): Profile {
    const m = this.metadata.get(this.fold(nick));
    return { avatar: m?.avatar || undefined, displayName: m?.['display-name'] || undefined };
  }

  /** Record a metadata key; `value` undefined means it was removed. */
  setMetadata(target: string, key: string, value: string | undefined) {
    const k = this.fold(target);
    const next = { ...(this.metadata.get(k) ?? {}) };
    if (value === undefined) delete next[key];
    else next[key] = value;
    if (Object.keys(next).length) this.metadata.set(k, next);
    else this.metadata.delete(k);
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

export type Toast = { id: number; text: string; kind: 'info' | 'error'; action?: { label: string; run: () => void } };

/** Composer input that carries a password and must not be kept for recall. */
export const secretInput = (line: string) => /^\/(register|verify)\b/i.test(line);

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
  shortcutsOpen = $state(false);
  /** Latched by the first-run flow so it can show connection progress after the first save. */
  onboarding = $state(false);
  toasts = $state<Toast[]>([]);
  private recall: string[] = [];
  get inputHistory(): string[] {
    return this.recall;
  }
  set inputHistory(v: string[]) {
    this.recall = v.filter((h) => !secretInput(h));
  }

  /** conn id → folded channel to open when our JOIN arrives. */
  pendingActive = new Map<string, string>();
  private restoreKey: string | null = null;
  private blurredAt = Date.now();
  private saveBufferTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private previews = new Map<string, Promise<Preview | null>>();
  private avatars = new Map<string, Promise<string | null>>();

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

  toast(text: string, kind: Toast['kind'] = 'info', action?: Toast['action']) {
    const t = { id: ++seq, text, kind, action };
    this.toasts = [...this.toasts, t];
    // Leave time to act on an offer.
    setTimeout(() => this.dismissToast(t.id), action ? 20_000 : 5000);
  }

  dismissToast(id: number) {
    this.toasts = this.toasts.filter((x) => x.id !== id);
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
    this.rememberActive();
    if (buf.kind !== 'server' && buf.history === 'idle' && buf.messages.length < 30) this.fetchHistory(buf, 'before');
  }

  /** Persist which buffer is open, to restore it on the next launch. */
  rememberActive() {
    const buf = this.active;
    if (!buf) return;
    try {
      localStorage.setItem('hearth.active', `${buf.conn.serverId}|${buf.conn.netid ?? ''}|${buf.key}`);
    } catch {}
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
    markRead(buf);
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
    const present = () => buf.messages.some((l) => (msgid && l.msgid ? l.msgid === msgid : l.time === time));
    if (!present()) {
      // A buffer that's still loading its newest lines: let them land first,
      // so the jump knows where the gap to the present is.
      await buf.pending;
      if (!present()) await loadAround(this, buf, time, msgid);
    }
    buf.focus = { time, msgid, seq: nextSeq() };
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
    return isWatched(conn, nick);
  }

  watch(conn: Conn, nick: string) {
    watch(this, conn, nick);
  }

  unwatch(conn: Conn, nick: string) {
    unwatch(this, conn, nick);
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
    saveBuffersSoon(conn, this.saveBufferTimers);
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

  /**
   * Send text with line breaks as draft/multiline batches, so it arrives as
   * one message. Returns false when the server can't take it that way.
   */
  sendMultiline(buf: Buffer, text: string, replyTo?: string): boolean {
    const conn = buf.conn;
    const limits = parseMultilineCap(conn.capValues['draft/multiline']);
    if (!limits || !conn.has('batch') || buf.kind === 'server') return false;
    // A single line (maybe with stray line breaks around it) needs no batch.
    if (text.split('\n').filter((l) => l.trim()).length < 2) return false;
    const target = buf.name;
    const budget = 510 - (conn.nick.length + 100 + 'PRIVMSG'.length + target.length);
    const batches = multilineBatches(text, limits, Math.max(budget, 60));
    if (!batches.length) return false;
    const reply = replyTo && conn.canTag('draft/reply') ? replyTo : undefined;
    batches.forEach((parts, i) => {
      const ref = `ml${nextSeq()}`;
      // Labeled, so a FAIL about the batch (too long, say) lands here.
      const label = conn.labelFor(buf);
      const tags = [label && `label=${label}`, reply && i === 0 && `+draft/reply=${escapeTag(reply)}`].filter(Boolean);
      conn.send(
        `${tags.length ? `@${tags.join(';')} ` : ''}BATCH +${ref} draft/multiline ${target}`,
        ...parts.map((p) => `@batch=${ref}${p.concat ? ';draft/multiline-concat' : ''} PRIVMSG ${target} :${p.text}`),
        `BATCH -${ref}`,
      );
      if (!conn.has('echo-message')) {
        buf.push(mkLine({ type: 'privmsg', nick: conn.nick, text: joinParts(parts), self: true, replyTo: i === 0 ? reply : undefined }));
      }
    });
    buf.lastTypingSent = 0;
    return true;
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
      // Only in-flight dedupe for big images; the preview component keeps
      // its own byte-bounded cache of settled ones.
      p.then((r) => (r?.image?.length ?? 0) > 256 * 1024 && this.previews.get(url) === p && this.previews.delete(url));
      if (this.previews.size > 500) this.previews.delete(this.previews.keys().next().value!);
    }
    return p;
  }

  /** A metadata avatar as a data: URL, or null if it can't or mustn't be loaded. */
  avatar(url: string): Promise<string | null> {
    if (!this.settings.loadAvatars) return Promise.resolve(null);
    let p = this.avatars.get(url);
    if (!p) {
      p = api.avatar(url).catch(() => null);
      this.avatars.set(url, p);
      if (this.avatars.size > 1000) this.avatars.delete(this.avatars.keys().next().value!);
    }
    return p;
  }

  /** Use the account from /register for SASL on this server from now on. */
  async saveLogin(conn: Conn, account: string, password: string) {
    const cfg = conn.config;
    if (!cfg) return;
    try {
      await this.saveServer({ ...$state.snapshot(cfg), sasl: 'plain', saslUsername: account, saslPassword: password }, false);
      this.toast(`Hearth will sign in as ${account} on ${conn.name}`);
    } catch (e) {
      this.toast(`Couldn't save the login: ${e}`, 'error');
    }
  }

  // ─── History & search ──────────────────────────────────────

  fetchHistory(buf: Buffer, mode: 'latest' | 'before' | 'after'): Promise<void> {
    return fetchHistory(this, buf, mode);
  }

  fillGap(buf: Buffer, gap: Gap, from: 'after' | 'before'): Promise<void> {
    return fillGap(this, buf, gap, from);
  }

  /** Search the local log and, where supported, the bouncer. Newest first. */
  search(query: string, scope: { conn?: Conn; buf?: Buffer }, before?: number): Promise<SearchPage> {
    return search(this, query, scope, before);
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
          conn.metadata.clear();
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
        conn.capValues = ev.capValues ?? {};
        break;
      case 'caps':
        conn.caps = new Set(ev.caps);
        conn.capValues = ev.capValues ?? {};
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
    if (!conn.buffersLoaded) await restoreBuffers(this, conn);
    else if (conn.has('draft/read-marker')) {
      // Reconnecting: queries already exist, so ensure() won't ask for their markers.
      for (const b of conn.buffers.values()) if (b.kind === 'query') conn.send(`MARKREAD ${b.name}`);
    }
    // Rejoin channels on direct connections; bouncers restore them on their own.
    if (!conn.netid) {
      const auto = new Set((conn.config?.autojoin ?? []).map((c) => conn.fold(c.trim())));
      const rejoin = [...conn.buffers.values()].filter((b) => b.kind === 'channel' && b.wantJoined && !auto.has(b.key));
      if (rejoin.length) setTimeout(() => rejoinCommands(rejoin).forEach((l) => conn.send(l)), 1500);
    }
    syncMonitor(conn);
    setTimeout(() => fetchTargets(this, conn), 800);
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

  get welcoming(): boolean {
    return this.ready && (this.servers.length === 0 || this.onboarding);
  }

  networkHighlights(serverId: string): NetworkHighlights {
    return this.settings.networkHighlights[serverId] ?? { words: [], never: [] };
  }

  highlightWords(conn: Conn): string[] {
    return [conn.nick, ...this.settings.highlightWords, ...this.networkHighlights(conn.serverId).words];
  }

  /** Whether a message from `src` highlights us. */
  isHighlight(conn: Conn, src: Source | null, text: string): boolean {
    const never = this.networkHighlights(conn.serverId).never;
    if (src && isIgnored(never, src, (s) => conn.fold(s))) return false;
    return mentions(text, this.highlightWords(conn));
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
