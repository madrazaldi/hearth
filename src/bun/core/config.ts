// Persistent configuration: server definitions plus opaque UI settings.
// config.json keeps the camelCase layout the Rust engine wrote, so existing
// installs load unchanged.

import { chmodSync, copyFileSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import * as secrets from './secrets';

export type SaslMechanism = 'none' | 'plain' | 'external';

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
  /** Server password (PASS). */
  password: string;
  /** `plain` is an account and password: SCRAM-SHA-256 when offered, else PLAIN. `external` is a TLS client certificate (CertFP). */
  sasl: SaslMechanism;
  saslUsername: string;
  saslPassword: string;
  autoconnect: boolean;
  autojoin: string[];
  /** Raw lines sent after registration. */
  perform: string[];
  /** PEM file holding a client certificate and its private key. */
  clientCert: string;
  /** Secrets are held in the OS keychain rather than this file. Never serialized. */
  inKeychain?: boolean;
}

export interface Config {
  servers: ServerConfig[];
  /** Owned by the frontend; stored verbatim. */
  settings: Record<string, unknown>;
}

export function defaultServer(): ServerConfig {
  return {
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
  };
}

const SASL: SaslMechanism[] = ['none', 'plain', 'external'];

/** Fill in defaults for missing fields, as serde's `#[serde(default)]` did. Throws on a wrongly typed field. */
export function normalizeServer(raw: unknown): ServerConfig {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) throw new Error('server must be an object');
  const r = raw as Record<string, unknown>;
  const s = defaultServer();
  const str = (k: keyof ServerConfig) => {
    const v = r[k];
    if (v === undefined || v === null) return;
    if (typeof v !== 'string') throw new Error(`invalid type for ${k}`);
    (s as unknown as Record<string, unknown>)[k] = v;
  };
  const bool = (k: keyof ServerConfig) => {
    const v = r[k];
    if (v === undefined || v === null) return;
    if (typeof v !== 'boolean') throw new Error(`invalid type for ${k}`);
    (s as unknown as Record<string, unknown>)[k] = v;
  };
  const list = (k: keyof ServerConfig) => {
    const v = r[k];
    if (v === undefined || v === null) return;
    if (!Array.isArray(v) || v.some((x) => typeof x !== 'string')) throw new Error(`invalid type for ${k}`);
    (s as unknown as Record<string, unknown>)[k] = [...v];
  };
  for (const k of ['id', 'name', 'host', 'websocketPath', 'nick', 'username', 'realname', 'password', 'saslUsername', 'saslPassword', 'clientCert'] as const) str(k);
  for (const k of ['tls', 'acceptInvalidCerts', 'websocket', 'autoconnect'] as const) bool(k);
  for (const k of ['autojoin', 'perform'] as const) list(k);
  if (r.port !== undefined && r.port !== null) {
    if (typeof r.port !== 'number' || !Number.isInteger(r.port) || r.port < 0 || r.port > 65535) throw new Error('invalid port');
    s.port = r.port;
  }
  if (r.sasl !== undefined && r.sasl !== null) {
    if (!SASL.includes(r.sasl as SaslMechanism)) throw new Error('invalid sasl mechanism');
    s.sasl = r.sasl as SaslMechanism;
  }
  return s;
}

/** What goes over the wire and into config.json: no internal fields. */
export function publicServer(s: ServerConfig): Omit<ServerConfig, 'inKeychain'> {
  const { inKeychain: _, ...rest } = s;
  return rest;
}

export function cloneConfig(c: Config): Config {
  return { servers: c.servers.map((s) => ({ ...s, autojoin: [...s.autojoin], perform: [...s.perform] })), settings: structuredClone(c.settings) };
}

export class Store {
  readonly path: string;

  constructor(dir: string) {
    this.path = join(dir, 'config.json');
  }

  /** Load the config and fill in secrets from the keychain, migrating any that are still stored in the file. */
  async load(): Promise<Config> {
    const config = this.read();
    let migrated = false;
    for (const s of config.servers) migrated = (await secrets.load(s)) || migrated;
    if (migrated) {
      try {
        this.save(config);
      } catch (e) {
        console.error(`could not rewrite config after moving secrets to the keychain: ${e}`);
      }
    }
    return config;
  }

  private read(): Config {
    let text: string;
    try {
      text = readFileSync(this.path, 'utf8');
    } catch {
      return { servers: [], settings: {} };
    }
    try {
      const raw = JSON.parse(text);
      if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) throw new Error('not an object');
      const servers = raw.servers ?? [];
      if (!Array.isArray(servers)) throw new Error('servers must be a list');
      const settings = raw.settings ?? {};
      return { servers: servers.map(normalizeServer), settings: typeof settings === 'object' && settings !== null ? settings : {} };
    } catch (e) {
      console.error(`config at ${this.path} is invalid (${e}); starting fresh`);
      try {
        copyFileSync(this.path, this.path.replace(/\.json$/, '.json.bak'));
      } catch {}
      return { servers: [], settings: {} };
    }
  }

  save(config: Config) {
    mkdirSync(dirname(this.path), { recursive: true });
    const onDisk = cloneConfig(config);
    for (const s of onDisk.servers) {
      if (s.inKeychain) {
        s.password = '';
        s.saslPassword = '';
      }
    }
    const tmp = this.path + '.tmp';
    writeFileSync(tmp, JSON.stringify({ servers: onDisk.servers.map(publicServer), settings: onDisk.settings }, null, 2), { mode: 0o600 });
    try {
      chmodSync(tmp, 0o600);
    } catch {}
    renameSync(tmp, this.path);
  }
}
