// Browser-only preview: fakes the Tauri backend with a scripted IRC session so
// the UI can be developed and screenshotted outside the app (`bun run dev`, open
// http://localhost:1420/?theme=light&view=settings). `?stress` adds #stress,
// a channel with ~10k lines of scrollback and ~5k members, for perf work;
// `?stress=live` also posts a line every 3s (which trims the buffer to
// Buffer's live-line cap).

import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import type { IrcEvent, IrcMessage, ServerConfig } from './types';
import { defaultServer } from './types';

const params = new URLSearchParams(location.search);
const empty = params.has('empty');
const stress = params.has('stress');

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
    const caps = [
      'message-tags',
      'server-time',
      'echo-message',
      'away-notify',
      'multi-prefix',
      'draft/message-redaction',
      'labeled-response',
      'batch',
      'draft/multiline',
      'draft/metadata-2',
      'draft/channel-rename',
      'draft/account-registration',
      'standard-replies',
    ];
    const capValues = { 'draft/multiline': 'max-bytes=4096,max-lines=24', 'draft/account-registration': 'custom-account-name' };
    ev.push({ type: 'registered', conn: c, nick: 'ember', caps, capValues });
    ev.push({ type: 'isupport', conn: c, isupport: { CHANTYPES: '#', PREFIX: '(ov)@+', CASEMAPPING: 'rfc1459', NETWORK: name, MONITOR: '100' } });
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
  // draft/metadata-2: a few people have avatars and display names.
  for (const [nick, name] of [['mira', 'Mira Okafor'], ['tobias', 'Tobias'], ['wren', ''], ['juniper', 'Juniper L.'], ['sable', '']]) {
    m(L, msg('irc.libera.chat', 'METADATA', [nick, 'avatar', '*', `https://avatars.example/${nick}.png`]));
    if (name) m(L, msg('irc.libera.chat', 'METADATA', [nick, 'display-name', '*', name]));
  }
  m(L, { ...say('mira', chan, 'morning all — the websocket transport landed on main last night', 2), tags: { time: at(0), msgid: 'ws1' } });
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
  // A draft/multiline message, as the backend delivers it: one message with line breaks.
  m(L, { ...say('oskar', chan, 'repro steps for the reconnect bug:\n1. /raw QUIT\n2. watch it come back anyway\n\nfixed on the ircv3 branch', 1), tags: { time: at(0), msgid: 'ml1' } });
  m(L, msg('irc.libera.chat', 'FAIL', ['TOPIC', 'NEED_MORE_PRIVS', chan, 'You need channel operator status to change the topic'], { time: at(0) }));
  m(L, msg('tobias!tobias@user/tobias', 'MODE', [chan, '+v', 'juniper'], { time: at(1) }));
  m(L, { ...say('halvard', chan, 'drafted the release notes: https://hearth.example/notes/0.1 — shout if anything is wrong', 6), tags: { time: at(0), msgid: 'rn1' } });
  m(L, msg('tobias!tobias@user/tobias', 'TAGMSG', [chan], { '+draft/react': '🎉', '+draft/reply': 'rn1' }));
  m(L, msg('wren!wren@user/wren', 'TAGMSG', [chan], { '+draft/react': '🎉', '+draft/reply': 'rn1' }));
  m(L, msg('mira!mira@user/mira', 'TAGMSG', [chan], { '+draft/react': '👀', '+draft/reply': 'rn1' }));
  m(L, { ...say('sable', chan, 'the typing indicator is subtle in a good way', 1), tags: { time: at(0), msgid: 'ty1', '+draft/reply': 'ws1' } });
  m(L, say('oskar', chan, 'new sidebar spacing, before/after: https://files.hearth.example/uploads/k3x9/sidebar.png', 2));
  m(L, say('wren', chan, 'kettle status https://media.example/kettle.gif', 1));
  m(L, say('lin_', chan, 'screen recording of the reconnect flow https://files.hearth.example/uploads/p2q1/reconnect-demo.mp4', 1));
  m(L, say('quill', chan, 'nsfw (not really, just a very tall screenshot) https://img.example/tall.jpg', 1));
  m(L, msg('irc.libera.chat', '730', ['ember', 'mira!mira@user/mira']));

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
  // draft/channel-rename: #hearth-design becomes #hearth-ui, keeping its history.
  m(L, msg('ember!ember@user/ember', 'JOIN', ['#hearth-design'], { time: at(0) }));
  m(L, msg('srv', '353', ['ember', '=', '#hearth-design', 'ember @mira wren']));
  m(L, msg('srv', '366', ['ember', '#hearth-design', 'End']));
  m(L, say('wren', '#hearth-design', 'mockups for the member list avatars are up', 1));
  m(L, msg('mira!mira@user/mira', 'RENAME', ['#hearth-design', '#hearth-ui', 'design and UI talk'], { time: at(1) }));

  if (stress) {
    const S = '#stress';
    m(L, msg('ember!ember@user/ember', 'JOIN', [S], { time: at(0) }));
    const names = STRESS_NICKS.map((n, i) => (i < 12 ? '@' : i < 60 ? '+' : '') + n);
    for (let i = 0; i < names.length; i += 400) m(L, msg('srv', '353', ['ember', '=', S, names.slice(i, i + 400).join(' ')]));
    m(L, msg('srv', '366', ['ember', S, 'End']));
  }
  return ev;
}

// ── ?stress ─────────────────────────────────────────────────
// Deterministic so a line's content doesn't change between page loads.

const STRESS_NICKS = Array.from({ length: 5000 }, (_, i) => {
  const a = ['ash', 'birch', 'cedar', 'dune', 'elm', 'fern', 'gale', 'heath', 'iris', 'juno', 'kite', 'lark', 'moss', 'nova', 'onyx', 'pike'];
  return i === 0 ? 'ember' : `${a[i % a.length]}${i % 7 === 0 ? '_' : ''}${Math.floor(i / a.length) || ''}`;
});
const STRESS_TOTAL = 12_000;
const STRESS_FIRST = 10_000;
const STRESS_END = Date.now() - 1000 * 60 * 100;
const WORDS =
  'the a scroll anchor websocket parser buffer render frame layout paint row height measure virtual list prepend history bouncer cap sasl batch label nick channel topic kettle tea patch review merge ship release build tokio actor webkit'.split(
    ' ',
  );

function rng(seed: number) {
  let t = seed + 0x6d2b79f5;
  return () => {
    t = (t + 0x6d2b79f5) | 0;
    let r = Math.imul(t ^ (t >>> 15), 1 | t);
    r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
    return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
  };
}

/** Line `i` counts back from the newest (0). */
const stressTime = (i: number) => STRESS_END - i * 31_000 - (i % 5) * 1000;

function stressLines(i: number): IrcMessage[] {
  const r = rng(i);
  const nick = STRESS_NICKS[1 + Math.floor(r() * r() * 400)];
  const src = `${nick}!${nick}@user/${nick}`;
  const tags: Record<string, string> = { time: new Date(stressTime(i)).toISOString(), msgid: `s${i}` };
  const k = r();
  if (k < 0.05) return [msg(src, ['JOIN', 'PART', 'QUIT'][Math.floor(r() * 3)], k < 0.017 ? ['#stress'] : ['#stress', 'bye'], tags)];
  let text = Array.from({ length: 3 + Math.floor(r() * (r() < 0.1 ? 90 : 18)) }, () => WORDS[Math.floor(r() * WORDS.length)]).join(' ');
  if (r() < 0.03) text = 'ember: ' + text;
  if (r() < 0.02) text += ' https://hearth.example/notes/' + i;
  // Not drawn from r(), so the rest of the line stays as it was.
  if (i % 41 === 7) text += ` https://img.example/s/${i}.png`;
  if (r() < 0.03) tags['+draft/reply'] = `s${i + 1 + Math.floor(r() * 4)}`;
  const out = [msg(src, k < 0.08 ? 'NOTICE' : 'PRIVMSG', ['#stress', k < 0.12 && k >= 0.08 ? `\x01ACTION ${text}\x01` : `#${i} ${text}`], tags)];
  if (r() < 0.02) out.push(msg(`${STRESS_NICKS[7]}!x@y`, 'TAGMSG', ['#stress'], { '+draft/react': '👍', '+draft/reply': `s${i}` }));
  return out;
}

function stressHistory(before: number | null, limit: number): IrcMessage[] {
  let from = 0;
  let count = STRESS_FIRST;
  if (before !== null) {
    from = Math.max(0, Math.ceil((STRESS_END - before) / 31_000) - 1);
    while (from < STRESS_TOTAL && stressTime(from) >= before) from++;
    count = limit;
  }
  const out: IrcMessage[] = [];
  for (let i = Math.min(STRESS_TOTAL, from + count) - 1; i >= from; i--) out.push(...stressLines(i));
  return out;
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

// ── Link previews ───────────────────────────────────────────
// Images are drawn on a canvas; what the backend would send as a data URL.

const GIF = 'data:image/gif;base64,R0lGODlhyAB4APcfMd9vOv7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3/7y3yH/C05FVFNDQVBFMi4wAwEAAAAh+QQEFAAfACwAAAAAyAB4AAAI/wABCBxIsKDBgwgTKlzIsKHDhxAjSpxIsaLFixgzatzIsaPHjyBDihxJsqTJkyhTqlzJsqXLlzBjypxJs6bNmzhz6tzJs6fPn0CDCh1KtKjR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';
const drawn = new Map<string, string>();

function draw(w: number, h: number, hue: number): string {
  const key = `${w}x${h}@${hue}`;
  let url = drawn.get(key);
  if (!url) {
    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    const g = c.getContext('2d')!;
    const grad = g.createLinearGradient(0, 0, w, h);
    grad.addColorStop(0, `hsl(${hue} 55% 38%)`);
    grad.addColorStop(1, `hsl(${(hue + 50) % 360} 60% 62%)`);
    g.fillStyle = grad;
    g.fillRect(0, 0, w, h);
    g.fillStyle = 'rgb(255 255 255 / 0.85)';
    for (let i = 0; i < 5; i++) g.fillRect(w * 0.08, h * (0.12 + i * 0.16), w * (0.3 + ((i * 37) % 50) / 100), h * 0.07);
    g.font = `600 ${Math.round(Math.min(w, h) / 10)}px sans-serif`;
    g.fillText(`${w}×${h}`, w * 0.08, h * 0.95);
    url = c.toDataURL('image/png');
    drawn.set(key, url);
  }
  return url;
}

function mockPreview(url: string) {
  const base = { url, title: null, description: null, site: null, image: null, width: null, height: null, animated: false, mime: null, size: null };
  const image = (w: number, h: number, hue: number, mime = 'image/png') => ({ ...base, kind: 'image', image: draw(w, h, hue), width: w, height: h, mime, size: w * h });
  if (url.includes('hearth.example/notes'))
    return {
      ...base,
      kind: 'page',
      title: 'Hearth 0.1 release notes',
      description: 'Local search, saved scrollback, replies and reactions, link previews, and a channel browser.',
      site: 'hearth.example',
      image: draw(240, 240, 20),
      width: 240,
      height: 240,
    };
  if (url.endsWith('sidebar.png')) return image(1600, 900, 24);
  if (url.endsWith('tall.jpg')) return image(700, 1400, 200, 'image/jpeg');
  if (url.endsWith('kettle.gif')) return { ...base, kind: 'image', image: GIF, width: 200, height: 120, animated: true, mime: 'image/gif', size: 2809 };
  if (url.endsWith('.mp4')) return { ...base, kind: 'media', mime: 'video/mp4', size: 18_400_000 };
  const s = url.match(/img\.example\/s\/(\d+)/);
  if (s) {
    const n = Number(s[1]);
    return image([320, 800, 1200, 500][n % 4], [480, 600, 300, 500][n % 4], (n * 47) % 360);
  }
  throw new Error('no preview');
}

// First-run flow (`?empty`): a scripted connection for the server just saved.
// Host containing "nowhere" fails DNS, SASL password "wrong" is rejected, nick "taken" collides.
let lastSaved: ServerConfig | null = null;
function firstRun(id: string) {
  const s = lastSaved!;
  const send = (ev: IrcEvent[], ms: number) => setTimeout(() => emit('irc', ev), ms);
  send([{ type: 'conn-added', conn: id, server: id, parent: null, netid: null }, { type: 'status', conn: id, status: 'connecting', error: null, retryIn: null }], 20);
  if (s.host.includes('nowhere')) {
    const error = `could not reach ${s.host}:${s.port}: failed to lookup address information: nodename nor servname provided`;
    return send([{ type: 'status', conn: id, status: 'disconnected', error, retryIn: 2 }], 900);
  }
  send([{ type: 'status', conn: id, status: 'registering', error: null, retryIn: null }], 700);
  if (s.sasl === 'plain' && s.saslPassword === 'wrong') {
    send([{ type: 'message', conn: id, msg: msg(s.host, '904', [s.nick, 'SASL authentication failed'], { time: new Date().toISOString() }) }], 1300);
    return send([{ type: 'status', conn: id, status: 'disconnected', error: 'Closing Link: (SASL authentication failed)', retryIn: 2 }], 1500);
  }
  const nick = s.nick === 'taken' ? 'taken_' : s.nick;
  send(
    [
      { type: 'registered', conn: id, nick, caps: ['server-time', 'message-tags'] },
      { type: 'message', conn: id, msg: msg(s.host, '001', [nick, `Welcome to ${s.name || s.host}, ${nick}`]) },
      { type: 'status', conn: id, status: 'connected', error: null, retryIn: null },
    ],
    1800,
  );
}

mockWindows('main');
mockIPC(
  async (cmd, args) => {
    const a = args as Record<string, unknown>;
    switch (cmd) {
      case 'get_config':
        return {
          servers: empty ? [] : [server],
          settings: {
            theme: params.get('theme') ?? 'dark',
            layout: params.get('layout') ?? 'columns',
            linkPreviews: true,
            loadAvatars: !params.has('noavatars'),
            aliases: { ns: '/msg NickServ $*', j: '/join $1', hi: 'hello $1!; /me waves' },
            networkHighlights: { soju: { words: ['release', '/\\bci (failed|broke)\\b/i'], never: ['*bot*'] } },
          },
        };
      case 'connect':
        if (lastSaved && a.server === lastSaved.id) {
          firstRun(lastSaved.id);
          return null;
        }
        setTimeout(() => emit('irc', scene()), 30);
        return null;
      case 'history':
        return a.kind === 'targets' || a.kind === 'search' ? [] : history(a.target as string);
      case 'local_history':
        if (stress && a.target === '#stress' && a.after === null) return stressHistory((a.before as number | null) ?? null, a.limit as number);
        return [];
      case 'load_buffers':
        return [];
      case 'search': {
        const q = String(a.query).toLowerCase();
        const all = [...history('#hearth')];
        clock = T0 + 60_000 * 30;
        all.push(say('mira', '#hearth', 'morning all — the websocket transport landed on main last night', 0));
        if (stress) all.push(...stressHistory(null, 0).filter((x) => x.command === 'PRIVMSG'));
        return all
          .filter((x) => x.params[1].toLowerCase().includes(q))
          .slice(-200)
          .map((x) => ({ network: 'soju/1', target: x.params[0], msg: x }));
      }
      case 'link_preview':
        await new Promise((r) => setTimeout(r, 120));
        return mockPreview(String(a.url));
      case 'avatar': {
        // A stand-in for the backend's fetched image: initials on the nick's hue.
        const nick = String(a.url).replace(/^.*\/|\.png$/g, '');
        const hue = [...nick].reduce((h, ch) => (h * 31 + ch.charCodeAt(0)) % 360, 7);
        const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 40"><rect width="40" height="40" fill="hsl(${hue} 55% 55%)"/><circle cx="20" cy="16" r="7" fill="hsl(${hue} 60% 85%)"/><ellipse cx="20" cy="36" rx="13" ry="10" fill="hsl(${hue} 60% 85%)"/></svg>`;
        return `data:image/svg+xml;base64,${btoa(svg)}`;
      }
      case 'cert_info':
        throw new Error('file not found');
      case 'save_server':
        return (lastSaved = { ...(a.server as ServerConfig), id: 'new' });
      // Updater and process plugins: `?update=none` pretends to be current.
      case 'plugin:app|version':
        return '0.1.0';
      case 'plugin:updater|check':
        await new Promise((r) => setTimeout(r, 600));
        if (params.get('update') === 'none') return null;
        return { rid: 1, currentVersion: '0.1.0', version: '0.2.0', date: new Date().toISOString(), body: 'Mock release', rawJson: {} };
      case 'plugin:updater|download_and_install':
        await new Promise((r) => setTimeout(r, 1500));
        return null;
      case 'plugin:process|restart':
        location.reload();
        return null;
      default:
        return null;
    }
  },
  { shouldMockEvents: true },
);

// Scripted interactions for screenshots.
setTimeout(async () => {
  const { app } = await import('./state.svelte');
  const target = params.get('buf') ?? (stress ? '#stress' : '#hearth');
  const conn = app.conns.get('soju/1');
  const buf = conn?.buffer(target);
  if (buf) app.open(buf);
  const view = params.get('view');
  if (view === 'switcher') window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', metaKey: true, ctrlKey: true }));
  if (view === 'settings') window.dispatchEvent(new KeyboardEvent('keydown', { key: ',', metaKey: true, ctrlKey: true }));
  if (view === 'shortcuts') window.dispatchEvent(new KeyboardEvent('keydown', { key: '/', metaKey: true, ctrlKey: true }));
  if (view === 'search') {
    app.searchQuery = params.get('q') ?? 'transport';
    app.searchOpen = true;
  }
  if (view === 'channels' && conn) {
    conn.channelList = {
      loading: false,
      at: Date.now(),
      items: [
        { name: '#libera', users: 2210, topic: 'Welcome to Libera.Chat | https://libera.chat' },
        { name: '#linux', users: 1874, topic: 'Linux help and discussion' },
        { name: '#rust', users: 1203, topic: 'The Rust programming language | https://rust-lang.org' },
        { name: '#hearth', users: 16, topic: 'Hearth — a quiet IRC client' },
      ],
    };
    app.browserConn = conn;
  }
  if (params.get('stress') === 'live' && conn) {
    // Live traffic, to check that the view stays pinned to the bottom.
    let n = 0;
    setInterval(() => {
      const nick = STRESS_NICKS[1 + (n % 50)];
      const text = `live ${++n}: ` + WORDS.slice(0, 3 + (n % 20)).join(' ');
      const live = msg(`${nick}!${nick}@user/${nick}`, 'PRIVMSG', ['#stress', text], { time: new Date().toISOString() });
      emit('irc', [{ type: 'message', conn: 'soju/1', msg: live }]);
    }, 3000);
  }
  if (params.get('tab')) app.settingsOpen = { tab: params.get('tab')!, serverId: params.get('server') ?? undefined };
}, 400);
