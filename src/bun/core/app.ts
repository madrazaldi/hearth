// The command API the frontend uses: one method per command, plus `invoke`,
// which the shell calls with the command name and JSON arguments the UI sent.

import { randomUUID } from 'node:crypto';
import { mkdirSync, readFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import type { HistoryKind } from './client';
import * as certs from './certs';
import { normalizeServer, publicServer, Store, type Config, type ServerConfig } from './config';
import { Emitter, type Event } from './events';
import { Manager } from './manager';
import type { Message } from './message';
import { Previewer, type Preview } from './preview';
import * as secrets from './secrets';
import { Db, type SavedBuffer, type SearchHit, type SearchQuery } from './store';
import { guessType, upload } from './upload';

type Args = Record<string, unknown>;

function str(a: Args, k: string): string {
  const v = a[k];
  if (typeof v !== 'string') throw new Error(`invalid arguments: ${k} must be a string`);
  return v;
}

function optNum(a: Args, k: string): number | null {
  const v = a[k];
  if (v === undefined || v === null) return null;
  if (typeof v !== 'number' || !Number.isFinite(v)) throw new Error(`invalid arguments: ${k} must be a number`);
  return Math.trunc(v);
}

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

const clamp = (n: number, lo: number, hi: number) => Math.min(Math.max(n, lo), hi);

export class Core {
  private config: Config;
  private readonly irc: Manager;
  private readonly previews = new Previewer();
  private closeToTrayFlag = false;
  private loadAvatars = false;

  private constructor(
    private readonly store: Store,
    config: Config,
    private readonly configDir: string,
    db: Db,
    sink: (events: Event[]) => void,
  ) {
    this.config = config;
    this.irc = new Manager(new Emitter(sink), db);
    this.applySettings(config.settings);
  }

  /**
   * Load the config from `configDir` and open the log in `dataDir`. `sink`
   * receives each coalesced batch of connection events.
   */
  static async open(configDir: string, dataDir: string, sink: (events: Event[]) => void): Promise<Core> {
    const store = new Store(configDir);
    const config = await store.load();
    mkdirSync(dataDir, { recursive: true });
    const db = Db.open(join(dataDir, 'hearth.db'));
    return new Core(store, config, configDir, db, sink);
  }

  private server(id: string): ServerConfig {
    const s = this.config.servers.find((s) => s.id === id);
    if (!s) throw new Error('unknown server');
    return s;
  }

  private persist() {
    try {
      this.store.save(this.config);
    } catch (e) {
      throw new Error(errText(e));
    }
  }

  private get db(): Db {
    return this.irc.db;
  }

  /** Settings the backend acts on itself. */
  private applySettings(settings: Record<string, unknown>) {
    const flag = (k: string, d: boolean) => (typeof settings[k] === 'boolean' ? (settings[k] as boolean) : d);
    this.db.setLogging(flag('storeLogs', true));
    this.closeToTrayFlag = flag('closeToTray', false);
    this.loadAvatars = flag('loadAvatars', false);
  }

  closeToTray(): boolean {
    return this.closeToTrayFlag;
  }

  /** Quit every connection and flush the log; call before the process exits. */
  shutdown() {
    this.irc.disconnectAll();
    this.db.flush();
  }

  /** Like `shutdown`, then waits (briefly) for the QUIT lines to reach the servers. */
  async shutdownGracefully(ms = 1500) {
    this.shutdown();
    await this.irc.drain(ms);
    this.db.flush();
  }

  // ---- Commands ----

  getConfig(): Config {
    return { servers: this.config.servers.map((s) => publicServer(s) as ServerConfig), settings: this.config.settings };
  }

  async saveServer(input: ServerConfig): Promise<ServerConfig> {
    const server = normalizeServer(input);
    if (server.host.trim() === '') throw new Error('host is required');
    if (server.id === '') server.id = randomUUID().replaceAll('-', '').slice(0, 12);
    if (server.name.trim() === '') server.name = server.host;
    if (server.sasl !== 'plain') server.saslPassword = '';
    await secrets.store(server);
    const i = this.config.servers.findIndex((s) => s.id === server.id);
    if (i >= 0) this.config.servers[i] = server;
    else this.config.servers.push(server);
    this.persist();
    return publicServer(server) as ServerConfig;
  }

  async deleteServer(id: string) {
    this.irc.disconnect(id, null);
    this.config.servers = this.config.servers.filter((s) => s.id !== id);
    await secrets.forget(id);
    try {
      this.db.forgetServer(id);
    } catch (e) {
      console.warn(`could not delete logs for ${id}: ${errText(e)}`);
    }
    this.persist();
  }

  saveSettings(settings: Record<string, unknown>) {
    if (typeof settings !== 'object' || settings === null || Array.isArray(settings)) throw new Error('invalid arguments: settings must be an object');
    this.applySettings(settings);
    this.config.settings = settings;
    this.persist();
  }

  connect(server: string) {
    this.irc.connect({ ...this.server(server) });
  }

  disconnect(server: string, reason: string | null) {
    this.irc.disconnect(server, reason);
  }

  reconnect(server: string) {
    this.irc.reconnect(server);
  }

  /** Called when the UI (re)loads: drop every live connection so the frontend rebuilds its state from a clean registration burst. */
  reset() {
    this.irc.disconnectAll();
  }

  send(conn: string, lines: string[]) {
    if (!Array.isArray(lines) || lines.some((l) => typeof l !== 'string')) throw new Error('invalid arguments: lines must be a list of strings');
    this.irc.send(conn, lines);
  }

  history(conn: string, kind: string, target: string, line: string): Promise<Message[]> {
    const k: HistoryKind = kind === 'targets' ? 'targets' : kind === 'search' ? 'search' : 'messages';
    return this.irc.history(conn, k, target, line);
  }

  /** Messages from the local log. `target` is the case-folded buffer name. */
  localHistory(conn: string, target: string, before: number | null, after: number | null, limit: number): Message[] {
    const n = clamp(limit, 1, 1000);
    return after !== null ? this.db.historyAfter(conn, target, after, n) : this.db.history(conn, target, before, n);
  }

  search(query: SearchQuery): SearchHit[] {
    return this.db.search({ ...query, limit: clamp(query.limit, 1, 500) });
  }

  loadBuffers(conn: string): SavedBuffer[] {
    return this.db.buffers(conn);
  }

  saveBuffers(conn: string, buffers: SavedBuffer[]) {
    this.db.saveBuffers(conn, buffers, (s) => s.toLowerCase());
  }

  clearLogs() {
    this.db.clearLogs();
  }

  linkPreview(url: string): Promise<Preview> {
    return this.previews.preview(url);
  }

  /**
   * A draft/metadata-2 avatar as a data: URL. Frontends never fetch remote
   * images themselves, so the backend does (public addresses only).
   */
  avatar(url: string): Promise<string> {
    if (!this.loadAvatars) throw new Error('avatars are turned off');
    return this.previews.avatar(url);
  }

  generateCert(server: string) {
    return certs.generate(join(this.configDir, 'certs'), server);
  }

  certInfo(path: string) {
    return certs.info(path);
  }

  async uploadFile(server: string, url: string, path: string): Promise<string> {
    const cfg = this.server(server);
    const body = readFileSync(path);
    const name = basename(path) || 'file';
    return upload(cfg, url, name, guessType(name), body);
  }

  async uploadBytes(server: string, url: string, name: string, contentType: string, data: Uint8Array): Promise<string> {
    const cfg = this.server(server);
    return upload(cfg, url, name, contentType === '' ? guessType(name) : contentType, data);
  }

  // ---- Dispatch ----

  /** Run a command by name with the arguments the UI passed. Rejects with a plain message. */
  async invoke(cmd: string, a: Args = {}): Promise<unknown> {
    try {
      switch (cmd) {
        case 'get_config':
          return this.getConfig();
        case 'save_server':
          return await this.saveServer(a.server as ServerConfig);
        case 'delete_server':
          return await this.deleteServer(str(a, 'id'));
        case 'save_settings':
          return this.saveSettings(a.settings as Record<string, unknown>);
        case 'connect':
          return this.connect(str(a, 'server'));
        case 'disconnect':
          return this.disconnect(str(a, 'server'), typeof a.reason === 'string' ? a.reason : null);
        case 'reconnect':
          return this.reconnect(str(a, 'server'));
        case 'reset':
          return this.reset();
        case 'send':
          return this.send(str(a, 'conn'), a.lines as string[]);
        case 'history':
          return await this.history(str(a, 'conn'), str(a, 'kind'), str(a, 'target'), str(a, 'line'));
        case 'local_history':
          return this.localHistory(str(a, 'conn'), str(a, 'target'), optNum(a, 'before'), optNum(a, 'after'), optNum(a, 'limit') ?? 100);
        case 'search':
          return this.search(a.query as SearchQuery);
        case 'load_buffers':
          return this.loadBuffers(str(a, 'conn'));
        case 'save_buffers':
          return this.saveBuffers(str(a, 'conn'), a.buffers as SavedBuffer[]);
        case 'clear_logs':
          return this.clearLogs();
        case 'link_preview':
          return await this.linkPreview(str(a, 'url'));
        case 'avatar':
          return await this.avatar(str(a, 'url'));
        case 'generate_cert':
          return this.generateCert(str(a, 'server'));
        case 'cert_info':
          return this.certInfo(str(a, 'path'));
        case 'upload_file':
          return await this.uploadFile(str(a, 'server'), str(a, 'url'), str(a, 'path'));
        case 'upload_bytes':
          return await this.uploadBytes(str(a, 'server'), str(a, 'url'), str(a, 'name'), typeof a.contentType === 'string' ? a.contentType : '', bytesArg(a.data));
        default:
          throw new Error(`unknown command: ${cmd}`);
      }
    } catch (e) {
      // The UI shows these strings as they are.
      throw new Error(errText(e));
    }
  }
}

/** Upload bodies arrive base64-encoded (compact over the bridge); a plain list of byte values is accepted too. */
function bytesArg(v: unknown): Uint8Array {
  if (typeof v === 'string') return Buffer.from(v, 'base64');
  if (Array.isArray(v)) return Uint8Array.from(v as number[]);
  throw new Error('invalid arguments: data must be bytes');
}
