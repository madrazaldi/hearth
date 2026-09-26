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
  sasl: 'none' | 'plain';
  saslUsername: string;
  saslPassword: string;
  autoconnect: boolean;
  autojoin: string[];
  perform: string[];
}

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
});
