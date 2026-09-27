export interface Source {
  nick: string;
  user?: string;
  host?: string;
}

export interface IrcMessage {
  tags: Record<string, string>;
  source: Source | null;
  command: string;
  params: string[];
}

export type ConnStatus = 'connecting' | 'registering' | 'connected' | 'disconnected';

export type IrcEvent =
  | { type: 'conn-added'; conn: string; server: string; parent: string | null; netid: string | null }
  | { type: 'conn-removed'; conn: string }
  | { type: 'status'; conn: string; status: ConnStatus; error: string | null; retryIn: number | null }
  | { type: 'registered'; conn: string; nick: string; caps: string[] }
  | { type: 'caps'; conn: string; caps: string[] }
  | { type: 'isupport'; conn: string; isupport: Record<string, string> }
  | { type: 'message'; conn: string; msg: IrcMessage };

export interface ServerConfig {
  id: string;
  name: string;
  host: string;
  port: number;
  tls: boolean;
  acceptInvalidCerts: boolean;
  websocket: boolean;
  websocketPath: string;
  nick: string;
  username: string;
  realname: string;
  password: string;
  sasl: 'none' | 'plain' | 'external';
  saslUsername: string;
  saslPassword: string;
  autoconnect: boolean;
  autojoin: string[];
  perform: string[];
  /** PEM file with a client certificate and key (SASL EXTERNAL / CertFP). */
  clientCert: string;
}

export type NotifyLevel = 'all' | 'mentions' | 'none';

export interface Config {
  servers: ServerConfig[];
  settings: Partial<Settings> | null;
}

export interface Settings {
  theme: 'system' | 'dark' | 'light';
  layout: 'columns' | 'compact';
  fontSize: number;
  clock: '24h' | '12h';
  showSeconds: boolean;
  membership: 'show' | 'collapse' | 'hide';
  nickColors: boolean;
  showMembers: boolean;
  notifyHighlights: boolean;
  notifyPrivate: boolean;
  highlightWords: string[];
  sendTyping: boolean;
  showTyping: boolean;
  sidebarWidth: number;
  /** Keep a local copy of messages (enables offline scrollback and search). */
  storeLogs: boolean;
  /** Fetch link and image previews (through the backend, never the webview). */
  linkPreviews: boolean;
  /** With link previews on, show direct image links as images under the message. */
  inlineImages: boolean;
  /** Play animated images without hovering (still off under prefers-reduced-motion). */
  autoplayAnimations: boolean;
  closeToTray: boolean;
  /** Check the release feed on launch and every few hours. */
  checkUpdates: boolean;
  /** Mark away after this many minutes with the window in the background; 0 disables. */
  autoAwayMinutes: number;
  /** nick or nick!user@host masks, `*` and `?` wildcards. */
  ignores: string[];
  /** Per-buffer notification level, keyed by `Buffer.prefKey`. Absent means the default. */
  bufferNotify: Record<string, NotifyLevel>;
  /** Nicks to watch with MONITOR, keyed by `Conn.prefKey`. */
  watch: Record<string, string[]>;
}

export type LineType =
  | 'privmsg'
  | 'action'
  | 'notice'
  | 'join'
  | 'part'
  | 'quit'
  | 'kick'
  | 'nick'
  | 'mode'
  | 'topic'
  | 'invite'
  | 'info'
  | 'error'
  | 'whois';

export interface Line {
  id: number;
  time: number;
  type: LineType;
  nick: string;
  text: string;
  self: boolean;
  highlight: boolean;
  msgid?: string;
  /** Extra subject: kicked nick, new nick, whois payload, … */
  target?: string;
  account?: string;
  redacted?: boolean;
  data?: Record<string, string>;
  /** msgid of the message this one replies to (+draft/reply). */
  replyTo?: string;
  /** emoji → nicks who reacted (+draft/react). */
  reactions?: Record<string, string[]>;
}

export interface SavedBuffer {
  name: string;
  kind: 'channel' | 'query';
  key: string;
}

export interface SearchHit {
  network: string;
  target: string;
  msg: IrcMessage;
}

export interface Preview {
  url: string;
  /** `media` is a video or audio link, described but never loaded. */
  kind: 'image' | 'page' | 'media';
  title: string | null;
  description: string | null;
  site: string | null;
  /** A data: URL. */
  image: string | null;
  /** Pixel size of `image`. */
  width: number | null;
  height: number | null;
  animated: boolean;
  mime: string | null;
  size: number | null;
}

export interface CertInfo {
  path: string;
  sha256: string;
  sha512: string;
}

export const defaultServer = (): ServerConfig => ({
  id: '',
  name: '',
  host: '',
  port: 6697,
  tls: true,
  acceptInvalidCerts: false,
  websocket: false,
  websocketPath: '/',
  nick: '',
  username: '',
  realname: '',
  password: '',
  sasl: 'none',
  saslUsername: '',
  saslPassword: '',
  autoconnect: true,
  autojoin: [],
  perform: [],
  clientCert: '',
});

export const defaultSettings = (): Settings => ({
  theme: 'system',
  layout: 'columns',
  fontSize: 14,
  clock: '24h',
  showSeconds: false,
  membership: 'collapse',
  nickColors: true,
  showMembers: true,
  notifyHighlights: true,
  notifyPrivate: true,
  highlightWords: [],
  sendTyping: true,
  showTyping: true,
  sidebarWidth: 248,
  storeLogs: true,
  linkPreviews: false,
  inlineImages: true,
  autoplayAnimations: false,
  closeToTray: false,
  checkUpdates: true,
  autoAwayMinutes: 0,
  ignores: [],
  bufferNotify: {},
  watch: {},
});
