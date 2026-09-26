import { api } from './api';
import { mkLine, type App, type Buffer } from './state.svelte';

interface Command {
  usage: string;
  help: string;
  run: (ctx: Ctx) => void | Promise<void>;
}

interface Ctx {
  app: App;
  buf: Buffer;
  args: string[];
  rest: string;
}

class UsageError extends Error {}

function need(cond: unknown, usage: string): asserts cond {
  if (!cond) throw new UsageError(usage);
}

const chanOf = (ctx: Ctx, arg?: string) => {
  if (arg && ctx.buf.conn.isChannel(arg)) return arg;
  need(ctx.buf.kind === 'channel', 'This command needs a channel');
  return ctx.buf.name;
};

function modeFor(sign: '+' | '-', mode: string) {
  return (ctx: Ctx) => {
    need(ctx.args.length, `/${sign === '+' ? '' : 'de'}${mode} <nick…>`);
    const chan = chanOf(ctx);
    ctx.buf.conn.send(`MODE ${chan} ${sign}${mode.repeat(ctx.args.length)} ${ctx.args.join(' ')}`);
  };
}

export const COMMANDS: Record<string, Command> = {
  join: {
    usage: '/join <#channel> [key]',
    help: 'Join a channel',
    run: ({ app, buf, args }) => {
      need(args[0], '/join <#channel> [key]');
      let chan = args[0];
      if (!buf.conn.isChannel(chan)) chan = '#' + chan;
      app.join(buf.conn, chan, args[1]);
    },
  },
  part: {
    usage: '/part [#channel] [reason]',
    help: 'Leave a channel',
    run: (ctx) => {
      const explicit = ctx.args[0] && ctx.buf.conn.isChannel(ctx.args[0]);
      const chan = chanOf(ctx, explicit ? ctx.args[0] : undefined);
      const reason = explicit ? ctx.args.slice(1).join(' ') : ctx.rest;
      ctx.buf.conn.send(reason ? `PART ${chan} :${reason}` : `PART ${chan}`);
    },
  },
  close: {
    usage: '/close',
    help: 'Close this buffer (parts channels)',
    run: ({ app, buf }) => app.closeBuffer(buf),
  },
  msg: {
    usage: '/msg <target> <text>',
    help: 'Send a private message',
    run: ({ app, buf, args }) => {
      need(args.length >= 2, '/msg <target> <text>');
      const target = buf.conn.ensure(args[0]);
      app.sendMessage(target, args.slice(1).join(' '));
    },
  },
  query: {
    usage: '/query <nick> [text]',
    help: 'Open a conversation',
    run: ({ app, buf, args }) => {
      need(args[0], '/query <nick> [text]');
      const q = app.query(buf.conn, args[0]);
      if (args.length > 1) app.sendMessage(q, args.slice(1).join(' '));
    },
  },
  me: {
    usage: '/me <action>',
    help: 'Send an action',
    run: ({ app, buf, rest }) => {
      need(rest && buf.kind !== 'server', '/me <action>');
      app.sendMessage(buf, rest, 'PRIVMSG', true);
    },
  },
  notice: {
    usage: '/notice <target> <text>',
    help: 'Send a notice',
    run: ({ app, buf, args }) => {
      need(args.length >= 2, '/notice <target> <text>');
      app.sendMessage(buf.conn.ensure(args[0]), args.slice(1).join(' '), 'NOTICE');
    },
  },
  nick: {
    usage: '/nick <newnick>',
    help: 'Change your nickname',
    run: ({ buf, args }) => {
      need(args[0], '/nick <newnick>');
      buf.conn.send(`NICK ${args[0]}`);
    },
  },
  topic: {
    usage: '/topic [text]',
    help: 'Show or set the channel topic',
    run: (ctx) => {
      const chan = chanOf(ctx);
      ctx.buf.conn.send(ctx.rest ? `TOPIC ${chan} :${ctx.rest}` : `TOPIC ${chan}`);
    },
  },
  whois: {
    usage: '/whois <nick>',
    help: 'Look up a user',
    run: ({ buf, args }) => {
      const nick = args[0] ?? (buf.kind === 'query' ? buf.name : '');
      need(nick, '/whois <nick>');
      buf.conn.send(`WHOIS ${nick} ${nick}`);
    },
  },
  away: {
    usage: '/away [message]',
    help: 'Mark yourself away (no message clears it)',
    run: ({ buf, rest }) => void buf.conn.send(rest ? `AWAY :${rest}` : 'AWAY'),
  },
  back: {
    usage: '/back',
    help: 'Clear away status',
    run: ({ buf }) => void buf.conn.send('AWAY'),
  },
  invite: {
    usage: '/invite <nick> [#channel]',
    help: 'Invite someone to a channel',
    run: (ctx) => {
      need(ctx.args[0], '/invite <nick> [#channel]');
      ctx.buf.conn.send(`INVITE ${ctx.args[0]} ${chanOf(ctx, ctx.args[1])}`);
    },
  },
  kick: {
    usage: '/kick <nick> [reason]',
    help: 'Remove someone from the channel',
    run: (ctx) => {
      need(ctx.args[0], '/kick <nick> [reason]');
      const reason = ctx.args.slice(1).join(' ');
      ctx.buf.conn.send(`KICK ${chanOf(ctx)} ${ctx.args[0]}${reason ? ' :' + reason : ''}`);
    },
  },
  ban: {
    usage: '/ban <nick|mask>',
    help: 'Ban a nick or mask',
    run: (ctx) => {
      need(ctx.args[0], '/ban <nick|mask>');
      const mask = ctx.args[0].includes('!') || ctx.args[0].includes('@') ? ctx.args[0] : `${ctx.args[0]}!*@*`;
      ctx.buf.conn.send(`MODE ${chanOf(ctx)} +b ${mask}`);
    },
  },
  unban: {
    usage: '/unban <mask>',
    help: 'Remove a ban',
    run: (ctx) => {
      need(ctx.args[0], '/unban <mask>');
      ctx.buf.conn.send(`MODE ${chanOf(ctx)} -b ${ctx.args[0]}`);
    },
  },
  op: { usage: '/op <nick…>', help: 'Give operator status', run: modeFor('+', 'o') },
  deop: { usage: '/deop <nick…>', help: 'Take operator status', run: modeFor('-', 'o') },
  voice: { usage: '/voice <nick…>', help: 'Give voice', run: modeFor('+', 'v') },
  devoice: { usage: '/devoice <nick…>', help: 'Take voice', run: modeFor('-', 'v') },
  mode: {
    usage: '/mode [target] <modes> [args]',
    help: 'Show or change modes',
    run: ({ buf, args }) => {
      const first = args[0];
      if (first && (buf.conn.isChannel(first) || buf.conn.isMe(first))) buf.conn.send(`MODE ${args.join(' ')}`);
      else {
        need(buf.kind === 'channel', '/mode <target> <modes>');
        buf.conn.send(`MODE ${buf.name} ${args.join(' ')}`.trim());
      }
    },
  },
  names: {
    usage: '/names',
    help: 'Refresh the member list',
    run: (ctx) => void ctx.buf.conn.send(`NAMES ${chanOf(ctx)}`),
  },
  ctcp: {
    usage: '/ctcp <nick> <command> [args]',
    help: 'Send a CTCP request',
    run: ({ buf, args }) => {
      need(args.length >= 2, '/ctcp <nick> <command> [args]');
      const payload = [args[1].toUpperCase(), ...args.slice(2)].join(' ');
      buf.conn.send(`PRIVMSG ${args[0]} :\x01${payload}\x01`);
    },
  },
  raw: {
    usage: '/raw <line>',
    help: 'Send a raw IRC line',
    run: ({ buf, rest }) => {
      need(rest, '/raw <line>');
      buf.conn.send(rest);
    },
  },
  quote: { usage: '/quote <line>', help: 'Alias for /raw', run: (ctx) => COMMANDS.raw.run(ctx) },
  clear: {
    usage: '/clear',
    help: 'Clear this buffer',
    run: ({ buf }) => {
      buf.messages = [];
      buf.history = 'done';
    },
  },
  reconnect: {
    usage: '/reconnect',
    help: 'Reconnect this server',
    run: ({ buf }) => api.reconnect(buf.conn.serverId),
  },
  quit: {
    usage: '/quit [reason]',
    help: 'Disconnect from this server',
    run: ({ buf, rest }) => api.disconnect(buf.conn.serverId, rest || undefined),
  },
  motd: { usage: '/motd', help: 'Show the message of the day', run: ({ buf }) => void buf.conn.send('MOTD') },
  list: {
    usage: '/list [filter]',
    help: 'List channels (output in server buffer)',
    run: ({ buf, rest }) => void buf.conn.send(rest ? `LIST ${rest}` : 'LIST'),
  },
  help: {
    usage: '/help',
    help: 'Show commands',
    run: ({ buf }) => {
      const text = Object.entries(COMMANDS)
        .map(([, c]) => `${c.usage.padEnd(32)} ${c.help}`)
        .join('\n');
      buf.push(mkLine({ type: 'info', data: { motd: text, title: 'Commands' } }));
    },
  },
};

export const ALIASES: Record<string, string> = { j: 'join', p: 'part', q: 'query', w: 'whois', leave: 'part', m: 'msg' };

/** Run composer input. Returns false if nothing was sent. */
export async function runInput(app: App, buf: Buffer, input: string): Promise<boolean> {
  if (!input.trim()) return false;
  if (input.startsWith('/') && !input.startsWith('//')) {
    const [rawName, ...args] = input.slice(1).split(/ +/);
    const name = ALIASES[rawName.toLowerCase()] ?? rawName.toLowerCase();
    const rest = input.slice(1 + rawName.length).replace(/^ /, '');
    const cmd = COMMANDS[name];
    if (!cmd) {
      // Unknown commands go to the server verbatim, which is what most IRC users expect.
      buf.conn.send(`${rawName.toUpperCase()}${rest ? ' ' + rest : ''}`);
      return true;
    }
    try {
      await cmd.run({ app, buf, args: args.filter(Boolean), rest });
    } catch (e) {
      app.toast(e instanceof UsageError ? `Usage: ${e.message}` : String(e), 'error');
      return false;
    }
    return true;
  }
  const text = input.startsWith('//') ? input.slice(1) : input;
  if (buf.kind === 'server') {
    app.toast('This is a server buffer — use /raw to send commands', 'error');
    return false;
  }
  for (const line of text.split('\n')) if (line.trim()) app.sendMessage(buf, line);
  return true;
}
