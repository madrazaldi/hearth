// Headless smoke test: connect to a server and print every event.
// HEARTH_HOST=... HEARTH_PASS=... bun src/bun/probe.ts
import { defaultServer } from './core/config';
import { Emitter } from './core/events';
import { Manager } from './core/manager';
import { serialize } from './core/message';
import { Db } from './core/store';

const env = (k: string, d: string) => process.env[k] ?? d;
const nick = env('HEARTH_NICK', 'hearthprobe');
const server = {
  ...defaultServer(),
  id: 'probe',
  name: 'probe',
  host: env('HEARTH_HOST', 'irc.libera.chat'),
  port: Number(env('HEARTH_PORT', '6697')),
  tls: env('HEARTH_TLS', '1') === '1',
  websocket: env('HEARTH_WS', '0') === '1',
  websocketPath: env('HEARTH_WS_PATH', '/'),
  nick,
  ...(process.env.HEARTH_PASS ? { sasl: 'plain' as const, saslUsername: nick, saslPassword: process.env.HEARTH_PASS } : {}),
};
const verbose = process.env.VERBOSE !== undefined;
const mgr = new Manager(
  new Emitter((events) => {
    for (const e of events) {
      const s = JSON.stringify(e);
      if (verbose || e.type !== 'message') console.log(s.slice(0, 300));
    }
  }),
  Db.memory(),
);
mgr.connect(server);
await Bun.sleep(6000);
const target = process.env.HEARTH_HISTORY;
if (target) {
  const conn = env('HEARTH_HISTORY_CONN', 'probe');
  try {
    const msgs = await mgr.history(conn, 'messages', target, `CHATHISTORY LATEST ${target} * 20`);
    const last = msgs.at(-1);
    console.log(`history: ${msgs.length} messages, last: ${last ? serialize(last) : null}`);
  } catch (e) {
    console.log(`history error: ${(e as Error).message}`);
  }
  try {
    const t = await mgr.history(conn, 'targets', '*', 'CHATHISTORY TARGETS timestamp=2020-01-01T00:00:00Z timestamp=2100-01-01T00:00:00Z 50');
    console.log('targets:', t.map((m) => m.params[1]));
  } catch (e) {
    console.log(`targets error: ${(e as Error).message}`);
  }
}
mgr.disconnectAll();
await Bun.sleep(500);
process.exit(0);
