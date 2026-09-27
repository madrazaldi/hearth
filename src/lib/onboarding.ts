// First-run helpers: network presets, input validation, and plain-language
// explanations for the ways a first connection can fail.

export interface NetworkPreset {
  id: string;
  label: string;
  blurb: string;
  kind: 'network' | 'bouncer' | 'custom';
  host: string;
  port: number;
  tls: boolean;
  /** NICKLEN, when the network is known to be stricter than the usual 30. */
  nickLen?: number;
  /** Whether services accept SASL; `false` skips the account step. */
  sasl?: boolean;
  /** Shown on the account step. */
  accountNote?: string;
  channels?: string[];
}

export const presets: NetworkPreset[] = [
  {
    id: 'libera',
    label: 'Libera.Chat',
    blurb: 'Free and open-source communities',
    kind: 'network',
    host: 'irc.libera.chat',
    port: 6697,
    tls: true,
    nickLen: 16,
    sasl: true,
    accountNote:
      'Libera.Chat requires SASL when you connect from some address ranges, such as many VPNs, cloud hosts and Tor. It also protects a registered nickname and lets you into channels that only allow registered users.',
    channels: ['#libera'],
  },
  {
    id: 'oftc',
    label: 'OFTC',
    blurb: 'Debian, Tor and other free software projects',
    kind: 'network',
    host: 'irc.oftc.net',
    port: 6697,
    tls: true,
    sasl: false,
    accountNote:
      "OFTC doesn't use SASL. To use a registered nickname, identify with NickServ after connecting, or add a client certificate under Custom server.",
    channels: ['#oftc'],
  },
  {
    id: 'efnet',
    label: 'EFnet',
    blurb: 'One of the oldest networks, no services',
    kind: 'network',
    host: 'irc.efnet.org',
    port: 6697,
    tls: true,
    nickLen: 9,
    sasl: false,
    accountNote: "EFnet has no nickname services, so there's no account to log in to.",
    channels: ['#efnet'],
  },
  {
    id: 'rizon',
    label: 'Rizon',
    blurb: 'Large general-purpose network',
    kind: 'network',
    host: 'irc.rizon.net',
    port: 6697,
    tls: true,
    sasl: true,
    accountNote: 'Log in if you have registered your nickname with NickServ.',
    channels: ['#rizon'],
  },
  {
    id: 'hackint',
    label: 'hackint',
    blurb: 'Hacker and maker communities',
    kind: 'network',
    host: 'irc.hackint.org',
    port: 6697,
    tls: true,
    sasl: true,
    accountNote: 'Log in if you have registered your nickname with NickServ.',
    channels: ['#hackint'],
  },
  {
    id: 'soju',
    label: 'soju bouncer',
    blurb: 'Stays connected for you and keeps history',
    kind: 'bouncer',
    host: '',
    port: 6697,
    tls: true,
    sasl: true,
    accountNote: 'soju needs the username and password of your bouncer account.',
  },
  {
    id: 'custom',
    label: 'Custom server',
    blurb: 'Any other server, with every option',
    kind: 'custom',
    host: '',
    port: 6697,
    tls: true,
  },
];

const SPECIAL = '[]\\`_^{|}';

/** Checks a nickname against the RFC 2812 grammar most servers still enforce. */
export function nickError(nick: string, maxLen = 30, network = 'this network'): string | null {
  if (!nick) return 'Choose a nickname.';
  if (/\s/.test(nick)) return "Nicknames can't contain spaces.";
  if (/^[0-9-]/.test(nick)) return "Nicknames can't start with a digit or a dash.";
  const bad = [...nick].find((c) => !/[A-Za-z0-9-]/.test(c) && !SPECIAL.includes(c));
  if (bad) return `“${bad}” isn't allowed. Use letters, digits and - _ [ ] { } \\ | ^ \``;
  if (nick.length > maxLen) return `Keep it to ${maxLen} characters or fewer on ${network}.`;
  return null;
}

/** Splits a channel list typed by hand; bare names get a `#`. */
export function parseChannels(text: string): string[] {
  const out: string[] = [];
  for (const raw of text.split(/[\s,]+/)) {
    const c = raw.trim();
    if (!c) continue;
    const name = /^[#&+!]/.test(c) ? c : '#' + c;
    if (!out.some((x) => x.toLowerCase() === name.toLowerCase())) out.push(name);
  }
  return out;
}

export interface Friendly {
  title: string;
  hint: string;
}

/**
 * Turns a backend connection error (an anyhow chain, or the server's ERROR
 * reason) into something a newcomer can act on. The raw text is still shown
 * alongside for anyone who wants it.
 */
export function explainError(raw: string, ctx: { host: string; port: number; tls: boolean; sasl: boolean }): Friendly {
  const e = raw.toLowerCase();
  if (/sasl|authenticat/.test(e)) {
    return ctx.sasl
      ? { title: 'Your account name or password was not accepted', hint: 'Check them on the account step and try again.' }
      : {
          title: 'This network wants you to log in first',
          hint: 'Connections from your address need an account (SASL). Add your account on the account step, or connect from another network.',
        };
  }
  if (/lookup|resolve|nodename|name or service|no such host|dns/.test(e))
    return { title: `Couldn't find ${ctx.host}`, hint: 'Check the server address for typos.' };
  if (/refused/.test(e))
    return {
      title: 'The server refused the connection',
      hint: `Nothing is accepting connections on port ${ctx.port}. Check the port, and whether the server expects TLS.`,
    };
  if (/certificate|tls|handshake|unknownissuer|notvalidforname|expired/.test(e))
    return {
      title: "Couldn't set up a secure connection",
      hint: ctx.tls
        ? `The server's certificate wasn't trusted, or port ${ctx.port} doesn't speak TLS. TLS ports are usually 6697.`
        : 'The server may require TLS. Turn TLS on and use port 6697.',
    };
  if (/timed out connecting|unreachable|network is down|no route/.test(e))
    return { title: "The server didn't answer", hint: 'Check your internet connection and the server address.' };
  if (/registration timed out/.test(e))
    return { title: 'The server never finished logging you in', hint: 'It may be busy. Try again in a moment.' };
  if (/\b[kgzd]-?line|banned|\bban\b/.test(e))
    return { title: 'This network is refusing connections from your address', hint: 'This is set by the network, not Hearth.' };
  if (/too many|throttl|reconnecting too fast/.test(e))
    return { title: 'Too many connections from your address', hint: 'Wait a minute, then try again.' };
  return { title: "Couldn't connect", hint: 'The details below may help.' };
}
