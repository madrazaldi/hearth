// Turns IRC messages from the server into state changes and buffer lines.

import { msgTime, parseCtcp, parseAttrs, parseMask } from './irc';
import { mentions } from './format';
import { mkLine, type App, type Buffer, type Conn, type Member } from './state.svelte';
import type { IrcMessage, Line, LineType } from './types';

type Base = Pick<Line, 'time' | 'nick' | 'self' | 'msgid' | 'account' | 'replyTo'>;

function baseOf(conn: Conn, m: IrcMessage): Base {
  const nick = m.source?.nick ?? '';
  return {
    time: msgTime(m.tags),
    nick,
    self: conn.isMe(nick),
    msgid: m.tags.msgid,
    account: m.tags.account,
    replyTo: m.tags['+draft/reply'],
  };
}

function messageBody(m: IrcMessage): { type: LineType | null; text: string } {
  const raw = m.params[1] ?? '';
  const ctcp = parseCtcp(raw);
  if (ctcp) {
    if (ctcp.command === 'ACTION') return { type: 'action', text: ctcp.args };
    return { type: null, text: '' };
  }
  return { type: m.command === 'NOTICE' ? 'notice' : 'privmsg', text: raw };
}

/** Build a line for a message replayed from history (server or local log). */
export function historyLine(app: App, conn: Conn, m: IrcMessage): Line | null {
  const base = baseOf(conn, m);
  switch (m.command) {
    case 'PRIVMSG':
    case 'NOTICE': {
      if (app.isIgnored(conn, m.source)) return null;
      const { type, text } = messageBody(m);
      if (type === null) return null;
      return mkLine({ ...base, type, text, highlight: !base.self && mentions(text, app.highlightWords(conn)) });
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
      return null;
  }
}

/**
 * Walk a channel mode string, calling `each` for every mode letter with its
 * argument (if it takes one).
 */
function walkModes(conn: Conn, modestr: string, args: string[], each: (adding: boolean, mode: string, arg?: string) => void) {
  const [a = '', b = '', c = ''] = (conn.isupport.CHANMODES ?? 'beI,k,l,imnpst').split(',');
  const pmodes = conn.prefix.modes;
  let adding = true;
  let ai = 0;
  for (const ch of modestr) {
    if (ch === '+' || ch === '-') {
      adding = ch === '+';
      continue;
    }
    const takesArg = pmodes.includes(ch) || a.includes(ch) || b.includes(ch) || (c.includes(ch) && adding);
    each(adding, ch, takesArg ? args[ai++] : undefined);
  }
}

export function handleMessage(app: App, conn: Conn, m: IrcMessage) {
  const src = m.source;
  const p = m.params;
  const base = baseOf(conn, m);
  const { nick, self, time } = base;

  switch (m.command) {
    case 'PRIVMSG':
    case 'NOTICE':
      return onMessage(app, conn, m, base);

    case 'TAGMSG': {
      if (app.isIgnored(conn, src)) return;
      const target = p[0] ?? '';
      const buf = conn.isChannel(target) ? conn.buffer(target) : conn.buffer(self ? target : nick);
      if (!buf) return;
      const emoji = m.tags['+draft/react'];
      const parent = m.tags['+draft/reply'];
      if (emoji && parent) return buf.react(parent, emoji, nick);
      const typing = m.tags['+typing'];
      if (!typing || self || !app.settings.showTyping) return;
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
        const key = conn.pendingKeys.get(buf.key);
        if (key !== undefined) {
          buf.joinKey = key;
          conn.pendingKeys.delete(buf.key);
        }
        buf.members.clear();
        if (app.pendingActive.get(conn.id) === conn.fold(chan)) {
          app.pendingActive.delete(conn.id);
          app.open(buf);
        } else app.maybeRestoreActive(conn, buf.key);
        app.saveBuffersSoon(conn);
        app.fetchHistory(buf, buf.messages.length ? 'after' : 'latest');
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
        buf.wantJoined = false;
        buf.members.clear();
        app.saveBuffersSoon(conn);
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
      // Snapshot: renaming a query re-inserts it into the map being walked.
      for (const buf of [...conn.buffers.values()]) {
        const mem = buf.member(nick);
        const isQuery = buf.kind === 'query' && conn.fold(buf.name) === conn.fold(nick);
        if (mem) {
          buf.removeMember(nick);
          buf.setMember({ ...mem, nick: next });
        }
        if (mem || isQuery || (self && buf.kind === 'query')) buf.push(mkLine({ ...base, type: 'nick', target: next }));
        // A case-only change folds to the same key, so the buffer is still "free".
        if (isQuery && (conn.buffer(next) ?? buf) === buf) {
          conn.buffers.delete(buf.key);
          buf.name = next;
          conn.buffers.set(conn.fold(next), buf);
          app.saveBuffersSoon(conn);
        }
      }
      if (self) conn.server.push(mkLine({ ...base, type: 'nick', target: next }));
      return;
    }

    case 'MODE':
      return onMode(conn, m, base);

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
      if (app.isIgnored(conn, src)) return;
      const text = `invited ${conn.isMe(p[0]) ? 'you' : p[0]} to ${p[1]}`;
      app.here(conn).push(mkLine({ ...base, type: 'invite', target: p[1], text, highlight: conn.isMe(p[0]) }));
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
      return updateMember(conn, nick, { account: p[0] === '*' ? undefined : p[0] });
    case 'CHGHOST':
      return updateMember(conn, nick, { user: p[0], host: p[1] });
    case 'SETNAME':
      return updateMember(conn, nick, { realname: p[0] });

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
      } else if (p[0] === 'ADDNETWORK') app.toast('Network added');
      return;
    }

    case 'REDACT': {
      const buf = conn.isChannel(p[0]) ? conn.buffer(p[0]) : conn.buffer(self ? p[0] : nick);
      if (!buf) return;
      buf.messages = buf.messages.map((l) => (l.msgid === p[1] ? { ...l, redacted: true, reactions: undefined } : l));
      return;
    }

    case 'ERROR':
      conn.server.push(mkLine({ ...base, nick: '', type: 'error', text: p[0] ?? 'Connection error' }));
      return;

    case 'FAIL':
    case 'WARN':
    case 'NOTE': {
      const text = p[p.length - 1] ?? '';
      if (p[0] === 'CHATHISTORY' || p[0] === 'MARKREAD' || p[0] === 'SEARCH') return;
      if (p[0] === 'BOUNCER' && m.command === 'FAIL') return app.toast(`Bouncer: ${text}`, 'error');
      const target = m.command === 'NOTE' ? conn.server : app.here(conn, m.tags.label);
      target.push(mkLine({ ...base, nick: '', type: m.command === 'NOTE' ? 'info' : 'error', text: `${p[0]}: ${text}` }));
      return;
    }

    case 'PING':
    case 'PONG':
    case 'CAP':
    case 'AUTHENTICATE':
    case 'BATCH':
    case 'ACK':
      return;
  }

  if (/^\d{3}$/.test(m.command)) onNumeric(app, conn, m, time);
}

function onMessage(app: App, conn: Conn, m: IrcMessage, base: Base) {
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
  if (app.isIgnored(conn, src)) return;

  if (ctcp && ctcp.command !== 'ACTION') {
    if (!isNotice && !base.self) {
      // Answer at most a few CTCPs per 10s: replies share our send queue.
      const now = Date.now();
      conn.ctcpTimes = conn.ctcpTimes.filter((t) => now - t < 10_000);
      if (conn.ctcpTimes.length >= 3) {
        if (now - conn.ctcpWarned > 60_000) {
          conn.ctcpWarned = now;
          conn.server.push(mkLine({ ...base, type: 'info', text: 'Ignoring a flood of CTCP requests' }));
        }
        return;
      }
      conn.ctcpTimes.push(now);
      const reply = ctcpReply(ctcp.command, ctcp.args);
      if (reply !== null) conn.send(`NOTICE ${nick} :\x01${ctcp.command}${reply ? ' ' + reply : ''}\x01`);
      app.here(conn).push(mkLine({ ...base, type: 'info', text: `CTCP ${ctcp.command} request from ${nick}` }));
    } else if (isNotice) {
      app.here(conn).push(mkLine({ ...base, type: 'info', text: `CTCP ${ctcp.command} reply from ${nick}: ${ctcp.args}` }));
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
      const here = app.here(conn, m.tags.label);
      here.push(mkLine({ ...base, type, text, target: base.self ? target : undefined, highlight: !base.self }));
      if (!base.self) app.maybeNotify(conn, here, nick, text, true);
      return;
    }
    buf = existing ?? conn.ensure(peer, 'query');
  }

  const highlight = !base.self && mentions(text, app.highlightWords(conn));
  buf.typing.delete(nick);
  if (!isChan) {
    const mem = buf.member(nick);
    if (!mem && !base.self) buf.setMember({ nick, prefix: '', away: false, user: src?.user, host: src?.host });
  }
  buf.push(mkLine({ ...base, type, text, highlight }));
  if (!base.self) app.maybeNotify(conn, buf, nick, text, highlight || !isChan);
}

function ctcpReply(cmd: string, args: string): string | null {
  switch (cmd) {
    case 'VERSION':
      return 'Hearth — a modern IRC client';
    case 'PING':
      return args.slice(0, 64);
    case 'TIME':
      return new Date().toString();
    case 'CLIENTINFO':
      return 'ACTION CLIENTINFO PING TIME VERSION';
    default:
      return null;
  }
}

function updateMember(conn: Conn, nick: string, patch: Partial<Member>) {
  for (const buf of conn.buffers.values()) {
    const mem = buf.member(nick);
    if (mem) buf.setMember({ ...mem, ...patch });
  }
}

function onMode(conn: Conn, m: IrcMessage, base: Base) {
  const [target, modestr = '', ...args] = m.params;
  if (!conn.isChannel(target)) {
    if (conn.isMe(target)) conn.server.push(mkLine({ ...base, type: 'mode', text: [modestr, ...args].join(' ') }));
    return;
  }
  const buf = conn.buffer(target);
  if (!buf) return;
  const { modes: pmodes, symbols } = conn.prefix;
  walkModes(conn, modestr, args, (adding, ch, arg) => {
    const pi = pmodes.indexOf(ch);
    if (pi >= 0) {
      const mem = arg ? buf.member(arg) : undefined;
      if (mem) {
        const sym = symbols[pi];
        let prefix = mem.prefix.replace(sym, '');
        if (adding) prefix = [...(prefix + sym)].sort((x, y) => symbols.indexOf(x) - symbols.indexOf(y)).join('');
        buf.setMember({ ...mem, prefix });
      }
    } else if (ch === 'k') {
      // Servers hide the key from non-ops with "*".
      if (!adding) buf.joinKey = '';
      else if (arg && arg !== '*') buf.joinKey = arg;
      conn.app.saveBuffersSoon(conn);
    }
  });
  buf.push(mkLine({ ...base, type: 'mode', text: [modestr, ...args].join(' ') }));
}

// Error numerics whose first parameter is the channel or nick they concern.
const TARGET_ERRORS = new Set(['401', '403', '404', '405', '406', '442', '467', '471', '473', '474', '475', '477', '478', '482', '489']);

function onNumeric(app: App, conn: Conn, m: IrcMessage, time: number) {
  const p = m.params;
  const rest = p.slice(1);
  const trailing = p[p.length - 1] ?? '';
  const label = m.tags.label;
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
      if (conn.motd.length) {
        const buf = label ? app.here(conn, label) : conn.server;
        buf.push(mkLine({ time, type: 'info', text: '', data: { motd: conn.motd.join('\n') } }));
      }
      conn.motd = [];
      return;
    }
    case '422':
      return;
    case '221':
      return info(`Your user modes: ${p[1]}`, app.here(conn, label));
    case '324': {
      const buf = conn.buffer(p[1]);
      if (buf) {
        buf.modes = p.slice(2).join(' ');
        walkModes(conn, p[2] ?? '', p.slice(3), (adding, ch, arg) => {
          if (ch === 'k' && adding && arg && arg !== '*' && buf.joinKey !== arg) {
            buf.joinKey = arg;
            app.saveBuffersSoon(conn);
          }
        });
      }
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
        const { nick, user, host } = parseMask(entry.slice(i));
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
      conn.autoAway = false;
      return info('You are no longer marked as away', app.here(conn, label));
    case '306':
      conn.away = true;
      return info(conn.autoAway ? 'You have been marked as away (window in the background)' : 'You have been marked as away', app.here(conn, label));
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
      if (!w) return info(rest.join(' '), app.here(conn, label));
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
      if (w) app.here(conn, label).push(mkLine({ time, type: 'whois', nick: w.nick, text: '', data: w }));
      return;
    }
    case '341':
      return info(`Invited ${p[1]} to ${p[2]}`, conn.buffer(p[2]) ?? app.here(conn, label));

    // Channel list
    case '321':
      conn.listAcc = [];
      conn.channelList = { loading: true, items: conn.channelList?.items ?? [], at: Date.now() };
      return;
    case '322': {
      conn.listAcc ??= [];
      conn.listAcc.push({ name: p[1], users: parseInt(p[2]) || 0, topic: trailing });
      // Publish progress in steps; a big network lists tens of thousands.
      if (conn.listAcc.length % 1000 === 0) conn.channelList = { loading: true, items: conn.listAcc.slice(), at: Date.now() };
      return;
    }
    case '323':
      conn.channelList = { loading: false, items: conn.listAcc ?? [], at: Date.now() };
      conn.listAcc = null;
      return;

    // MONITOR
    case '730':
    case '731': {
      const online = m.command === '730';
      for (const entry of trailing.split(',')) {
        const nick = parseMask(entry.trim()).nick;
        if (!nick) continue;
        const key = conn.fold(nick);
        const was = conn.online.get(key);
        conn.online.set(key, online);
        if (was === undefined || was === online || !app.isWatched(conn, nick)) continue;
        const text = `${nick} is ${online ? 'online' : 'offline'}`;
        info(text);
        const q = conn.buffer(nick);
        if (q) info(text, q);
      }
      return;
    }
    case '732':
    case '733':
      return;
    case '734':
      return info(`Can't watch more nicknames: the server allows ${p[1]}`, conn.server, 'error');

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
      return info(`Nickname ${p[1]} is already in use`, app.here(conn, label), 'error');
  }

  const code = parseInt(m.command);
  if (code >= 400 && code < 600) {
    // Unlabeled errors about a channel or nick belong with that conversation,
    // not whatever the user happens to be looking at.
    const about = !label && TARGET_ERRORS.has(m.command) ? conn.buffer(p[1] ?? '') : undefined;
    return info(rest.filter((x, i) => i < rest.length - 1 || x).join(' '), about ?? app.here(conn, label), 'error');
  }
  info(rest.join(' '), label ? app.here(conn, label) : conn.server);
}
