// Browser-only preview: fakes the Tauri backend with a scripted IRC session so
// the UI can be developed and screenshotted outside the app (`pnpm dev`, open
// http://localhost:1420/?theme=light&view=settings).

import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import type { IrcEvent, IrcMessage, ServerConfig } from './types';
import { defaultServer } from './types';

const params = new URLSearchParams(location.search);
const empty = params.has('empty');

const server: ServerConfig = { ...defaultServer(), id: 'soju', name: 'soju', host: 'bouncer.example', port: 443, websocket: true, nick: 'ember' };

const T0 = Date.now() - 1000 * 60 * 95;
let clock = T0;
const at = (mins: number) => new Date((clock += mins * 60_000)).toISOString();

function msg(src: string, command: string, params: string[], tags: Record<string, string> = {}): IrcMessage {
  const [nick, rest] = src.split('!');
  const [user, host] = (rest ?? '').split('@');
  return { tags, source: src ? { nick, user: user || undefined, host: host || undefined } : null, command, params };
}

const say = (nick: string, target: string, text: string, mins = 1) =>
  msg(`${nick}!${nick.toLowerCase()}@user/${nick.toLowerCase()}`, 'PRIVMSG', [target, text], { time: at(mins) });

function scene(): IrcEvent[] {
  const ev: IrcEvent[] = [];
  const m = (conn: string, x: IrcMessage) => ev.push({ type: 'message', conn, msg: x });
  const nets = [
    ['1', 'Libera.Chat'],
    ['2', 'OFTC'],
    ['3', 'Rizon'],
  ];
  ev.push({ type: 'conn-added', conn: 'soju', server: 'soju', parent: null, netid: null });
  ev.push({ type: 'caps', conn: 'soju', caps: ['soju.im/bouncer-networks', 'draft/chathistory'] });
  ev.push({ type: 'registered', conn: 'soju', nick: 'ember', caps: ['soju.im/bouncer-networks'] });
  ev.push({ type: 'status', conn: 'soju', status: 'connected', error: null, retryIn: null });
  for (const [id, name] of nets) {
    m('soju', msg('bouncer.example', 'BOUNCER', ['NETWORK', id, `name=${name};state=connected`]));
    const c = `soju/${id}`;
    ev.push({ type: 'conn-added', conn: c, server: 'soju', parent: 'soju', netid: id });
    const caps = ['message-tags', 'server-time', 'echo-message', 'away-notify', 'multi-prefix'];
    ev.push({ type: 'registered', conn: c, nick: 'ember', caps });
    ev.push({ type: 'isupport', conn: c, isupport: { CHANTYPES: '#', PREFIX: '(ov)@+', CASEMAPPING: 'rfc1459', NETWORK: name } });
    ev.push({ type: 'status', conn: c, status: id === '3' ? 'disconnected' : 'connected', error: id === '3' ? 'connection reset by peer' : null, retryIn: id === '3' ? 16 : null });
  }

  const L = 'soju/1';
  const chan = '#hearth';
  m(L, msg('ember!ember@user/ember', 'JOIN', [chan], { time: at(0) }));
  m(L, msg('irc.libera.chat', '332', ['ember', chan, 'Hearth — a quiet IRC client · https://hearth.example · Be kind, ship small']));
  m(L, msg('irc.libera.chat', '333', ['ember', chan, 'mira', String(Math.floor(T0 / 1000) - 86400 * 3)]));
  const people = ['@mira', '@tobias', '+wren', '+kestrel', 'juniper', 'oskar', 'lin_', 'halvard', 'sable', 'quill', 'rook', 'ada', 'pim', 'felix', 'noor', 'ember'];
  m(L, msg('irc.libera.chat', '353', ['ember', '=', chan, people.join(' ')]));
  m(L, msg('irc.libera.chat', '366', ['ember', chan, 'End of /NAMES list.']));
  m(L, say('mira', chan, 'morning all — the websocket transport landed on main last night', 2));
  m(L, say('tobias', chan, 'nice. does it negotiate text.ircv3.net or just assume it?', 1));
  m(L, say('mira', chan, 'negotiates, falls back to binary frames', 1));
  m(L, say('mira', chan, 'and lines over 4k get rejected before they hit the parser', 0));
  m(L, msg('pim!pim@gateway/web', 'JOIN', [chan], { time: at(1) }));
  m(L, msg('rook!rook@host', 'QUIT', ['Ping timeout: 240 seconds'], { time: at(0) }));
  m(L, msg('felix!felix@host', 'PART', [chan, 'later'], { time: at(0) }));
  m(L, say('wren', chan, 'ember: did you get a chance to look at the scroll anchoring bug? WebKit still ignores overflow-anchor', 3));
  m(L, say('ember', chan, 'yeah — ended up measuring scrollHeight before and after the prepend. no more jumping', 2));
  m(L, msg('kestrel!kestrel@user/kestrel', 'PRIVMSG', [chan, '\x01ACTION puts the kettle on\x01'], { time: at(1) }));
  m(L, say('juniper', chan, 'bench numbers: \x02NAMES\x02 for a 2k-member channel now renders in \x0303~9ms\x03, down from 40', 4));
  m(L, say('oskar', chan, 'wait, are we coalescing the IPC events? that would explain it', 1));
  m(L, say('juniper', chan, 'yep, 8ms window, capped at 2000 events per payload', 1));
  m(L, msg('tobias!tobias@user/tobias', 'MODE', [chan, '+v', 'juniper'], { time: at(1) }));
  m(L, say('halvard', chan, 'drafted the release notes: https://hearth.example/notes/0.1 — shout if anything is wrong', 6));
  m(L, say('sable', chan, 'the typing indicator is subtle in a good way', 1));

  m(L, say('mira', 'ember', 'hey, can you review #42 when you have a minute?', 3));
  m(L, say('ember', 'mira', 'on it after lunch', 1));
  ev.push({ type: 'message', conn: L, msg: { tags: { '+typing': 'active' }, source: { nick: 'mira', user: 'mira', host: 'x' }, command: 'TAGMSG', params: ['ember'] } });
  ev.push({ type: 'message', conn: L, msg: { tags: { '+typing': 'active' }, source: { nick: 'oskar', user: 'oskar', host: 'x' }, command: 'TAGMSG', params: [chan] } });

  for (const [c, name] of [['#rust', 'soju/1'], ['#svelte', 'soju/1'], ['#tauri', 'soju/1'], ['#debian', 'soju/2'], ['#oftc', 'soju/2']] as const) {
    m(name, msg('ember!ember@user/ember', 'JOIN', [c], { time: at(0) }));
    m(name, msg('srv', '353', ['ember', '=', c, 'ember @op user1 user2']));
    m(name, msg('srv', '366', ['ember', c, 'End']));
  }
  m('soju/1', say('ferris', '#rust', 'anyone around who knows pin projection well?', 1));
  m('soju/1', say('ferris', '#rust', 'ember: you did the tokio actor thing, right?', 1));
  m('soju/2', say('deb', '#debian', 'trixie point release is out', 1));
  return ev;
}

function history(target: string): IrcMessage[] {
  if (target !== '#hearth') return [];
  clock = T0 - 1000 * 60 * 60 * 26;
  return [
    say('mira', target, 'proposal: we keep the palette warm but let the ember accent do all the work', 0),
    say('tobias', target, 'agreed, no gradients on anything that is not a first-run screen', 2),
    say('lin_', target, 'monospace only for timestamps and raw protocol stuff?', 3),
    say('mira', target, 'exactly. Plex Sans for prose, Plex Mono for machine text', 1),
  ];
}

mockWindows('main');
mockIPC(
  async (cmd, args) => {
    const a = args as Record<string, unknown>;
    switch (cmd) {
      case 'get_config':
        return {
          servers: empty ? [] : [server],
          settings: { theme: params.get('theme') ?? 'dark', layout: params.get('layout') ?? 'columns' },
        };
      case 'connect':
        setTimeout(() => emit('irc', scene()), 30);
        return null;
      case 'history':
        return a.kind === 'targets' ? [] : history(a.target as string);
      case 'save_server':
        return { ...(a.server as ServerConfig), id: 'new' };
      default:
        return null;
    }
  },
  { shouldMockEvents: true },
);

// Scripted interactions for screenshots.
setTimeout(async () => {
  const { app } = await import('./state.svelte');
  const target = params.get('buf') ?? '#hearth';
  const conn = app.conns.get('soju/1');
  const buf = conn?.buffer(target);
  if (buf) app.open(buf);
  const view = params.get('view');
  if (view === 'switcher') window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', metaKey: true, ctrlKey: true }));
  if (view === 'settings') window.dispatchEvent(new KeyboardEvent('keydown', { key: ',', metaKey: true, ctrlKey: true }));
}, 400);
