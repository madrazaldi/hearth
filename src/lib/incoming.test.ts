// Behavioural tests for incoming.ts: raw IRC lines in, App state out.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { connect, feed, mockApi, sent } from './test-utils';

vi.mock('./api', () => ({ api: mockApi, onIrc: vi.fn(async () => () => {}) }));
vi.mock('./notify', () => ({ notify: vi.fn() }));

const { App } = await import('./state.svelte');
const { notify } = await import('./notify');

let app: InstanceType<typeof App>;

beforeEach(() => {
  vi.clearAllMocks();
  sent.length = 0;
  app = new App();
});

const conn = () => app.conns.get('c')!;
const buf = (name: string) => conn().buffer(name)!;
const lines = (name: string) => buf(name).messages;
const texts = (name: string) => lines(name).map((l) => l.text);
const types = (name: string) => lines(name).map((l) => l.type);
const nicks = (name: string) => buf(name).sortedMembers.map((m) => m.prefix + m.nick);
const serverTexts = () => conn().server.messages.map((l) => l.text);

/** Join `chan` and load a NAMES list. */
function joinWith(chan: string, names: string) {
  feed(app, `:me!u@h JOIN ${chan}`, `:srv 353 me = ${chan} :${names}`, `:srv 366 me ${chan} :End of /NAMES list.`);
}

describe('NICK', () => {
  it('renames another user in every shared channel and their query', () => {
    connect(app);
    joinWith('#a', '@me ann bob');
    joinWith('#b', 'me +ann');
    joinWith('#c', 'me bob');
    feed(app, ':ann!a@h PRIVMSG me :hi');
    feed(app, ':ann!a@h NICK anna');

    expect(nicks('#a')).toEqual(['@me', 'anna', 'bob']);
    expect(nicks('#b')).toEqual(['+anna', 'me']);
    expect(types('#a')).toEqual(['nick']);
    expect(lines('#a')[0]).toMatchObject({ nick: 'ann', target: 'anna' });
    expect(types('#b')).toEqual(['nick']);
    expect(lines('#c')).toEqual([]);

    // The private query follows the rename and is filed under the new name.
    expect(conn().buffer('ann')).toBeUndefined();
    const q = buf('anna');
    expect(q.name).toBe('anna');
    expect(q.kind).toBe('query');
    expect(q.messages.map((l) => l.type)).toEqual(['privmsg', 'nick']);
    feed(app, ':anna!a@h PRIVMSG me :still me');
    expect(texts('anna')).toEqual(['hi', '', 'still me']);
  });

  it("doesn't merge a query into an existing one for the new nick", () => {
    connect(app);
    feed(app, ':ann!a@h PRIVMSG me :one', ':anna!a@h PRIVMSG me :two');
    feed(app, ':ann!a@h NICK anna');
    expect(buf('ann').name).toBe('ann');
    expect(texts('anna')).toEqual(['two']);
  });

  it('updates the query name on a case-only nick change', () => {
    connect(app);
    feed(app, ':ann!a@h PRIVMSG me :hi', ':ann!a@h NICK Ann');
    expect(buf('ann').name).toBe('Ann');
    expect(buf('Ann').member('ann')!.nick).toBe('Ann');
  });

  it('tracks our own nick change everywhere', () => {
    connect(app);
    joinWith('#a', '@me ann');
    feed(app, ':ann!a@h PRIVMSG me :hi');
    feed(app, ':me!u@h NICK me2');
    expect(conn().nick).toBe('me2');
    expect(nicks('#a')).toEqual(['@me2', 'ann']);
    expect(lines('#a').at(-1)).toMatchObject({ type: 'nick', self: true, target: 'me2' });
    // Queries and the server buffer hear about it too.
    expect(types('ann')).toEqual(['privmsg', 'nick']);
    expect(conn().server.messages.at(-1)).toMatchObject({ type: 'nick', target: 'me2' });
    // Mentions now use the new nick.
    feed(app, ':ann!a@h PRIVMSG #a :me2: yo', ':ann!a@h PRIVMSG #a :me: old');
    expect(lines('#a').slice(-2).map((l) => l.highlight)).toEqual([true, false]);
  });
});

describe('KICK, PART, QUIT', () => {
  it('kicking someone else removes only them', () => {
    connect(app);
    joinWith('#a', '@me ann bob');
    feed(app, ':me!u@h KICK #a bob :bye');
    expect(nicks('#a')).toEqual(['@me', 'ann']);
    expect(buf('#a').joined).toBe(true);
    expect(lines('#a').at(-1)).toMatchObject({ type: 'kick', nick: 'me', target: 'bob', text: 'bye', highlight: false });
  });

  it('being kicked leaves the channel and highlights', () => {
    connect(app);
    joinWith('#a', 'me @ann');
    feed(app, ':ann!a@h KICK #a me :out');
    expect(buf('#a').joined).toBe(false);
    expect(buf('#a').members.size).toBe(0);
    expect(lines('#a').at(-1)).toMatchObject({ type: 'kick', target: 'me', text: 'out', highlight: true });
  });

  it('PART removes the member; our own PART clears the list', () => {
    connect(app);
    joinWith('#a', 'me ann bob');
    feed(app, ':ann!a@h PART #a :later');
    expect(nicks('#a')).toEqual(['bob', 'me']);
    expect(lines('#a').at(-1)).toMatchObject({ type: 'part', nick: 'ann', text: 'later' });
    feed(app, ':me!u@h PART #a');
    expect(buf('#a').joined).toBe(false);
    expect(buf('#a').members.size).toBe(0);
  });

  it('QUIT only shows in buffers shared with the user', () => {
    connect(app);
    joinWith('#a', 'me ann');
    joinWith('#b', 'me ann');
    joinWith('#c', 'me bob');
    feed(app, ':ann!a@h PRIVMSG me :hi');
    feed(app, '@+typing=active :ann!a@h TAGMSG #a');
    feed(app, ':ann!a@h QUIT :Quit: gone');
    expect(lines('#a').at(-1)).toMatchObject({ type: 'quit', nick: 'ann', text: 'Quit: gone' });
    expect(types('#b')).toEqual(['quit']);
    expect(lines('#c')).toEqual([]);
    expect(types('ann')).toEqual(['privmsg', 'quit']);
    expect(buf('#a').member('ann')).toBeUndefined();
    expect(buf('#a').typing.has('ann')).toBe(false);
  });

  it('QUIT reaches a query even when the member list is empty', () => {
    connect(app);
    conn().ensure('ann', 'query');
    feed(app, ':ann!a@h QUIT :bye');
    expect(types('ann')).toEqual(['quit']);
  });
});

describe('MODE', () => {
  it('adds and removes prefixes with multi-prefix, keeping rank order', () => {
    connect(app, { caps: ['multi-prefix'], isupport: { PREFIX: '(qaohv)~&@%+' } });
    joinWith('#a', 'me ann +bob');
    feed(app, ':srv MODE #a +v ann', ':srv MODE #a +o ann');
    expect(buf('#a').member('ann')!.prefix).toBe('@+');
    feed(app, ':srv MODE #a +q ann');
    expect(buf('#a').member('ann')!.prefix).toBe('~@+');
    feed(app, ':srv MODE #a -o ann');
    expect(buf('#a').member('ann')!.prefix).toBe('~+');
    feed(app, ':srv MODE #a +vv ann bob');
    expect(buf('#a').member('bob')!.prefix).toBe('+');
    expect(buf('#a').member('ann')!.prefix).toBe('~+');
  });

  it('applies several modes with mixed signs and arguments in one line', () => {
    connect(app, { isupport: { CHANMODES: 'beI,k,l,imnpst' } });
    joinWith('#a', 'me @ann bob +cat');
    feed(app, ':ann!a@h MODE #a +ov-v+lk-o bob bob cat 50 sekrit ann');
    // bob gets both +o and +v; cat loses voice; ann deops herself.
    expect(buf('#a').member('bob')!.prefix).toBe('@+');
    expect(buf('#a').member('cat')!.prefix).toBe('');
    expect(buf('#a').member('ann')!.prefix).toBe('');
    expect(buf('#a').joinKey).toBe('sekrit');
    expect(lines('#a').at(-1)).toMatchObject({ type: 'mode', nick: 'ann', text: '+ov-v+lk-o bob bob cat 50 sekrit ann' });
    expect(nicks('#a')).toEqual(['@+bob', 'ann', 'cat', 'me']);
  });

  it("doesn't consume an argument for -l, but does for bans", () => {
    connect(app, { isupport: { CHANMODES: 'beI,k,l,imnpst' } });
    joinWith('#a', 'me ann');
    feed(app, ':srv MODE #a -l+b+v *!*@spam ann');
    expect(buf('#a').member('ann')!.prefix).toBe('+');
  });

  it('forgets the key on -k and ignores a hidden key', () => {
    connect(app);
    joinWith('#a', '@me');
    feed(app, ':srv MODE #a +k real');
    expect(buf('#a').joinKey).toBe('real');
    feed(app, ':srv MODE #a +k *');
    expect(buf('#a').joinKey).toBe('real');
    feed(app, ':srv MODE #a -k *');
    expect(buf('#a').joinKey).toBe('');
  });

  it('records channel modes from 324 and user modes on us', () => {
    connect(app);
    joinWith('#a', '@me');
    feed(app, ':srv 324 me #a +ntkl hunter2 10');
    expect(buf('#a').modes).toBe('+ntkl hunter2 10');
    expect(buf('#a').joinKey).toBe('hunter2');
    feed(app, ':me MODE me :+iw');
    expect(conn().server.messages.at(-1)).toMatchObject({ type: 'mode', text: '+iw' });
  });

  it('parses multi-prefix NAMES and ranks members', () => {
    connect(app, { caps: ['multi-prefix'], isupport: { PREFIX: '(qaohv)~&@%+' } });
    joinWith('#a', '+zed @+ann ~@bob %cat me');
    expect(buf('#a').member('ann')!.prefix).toBe('@+');
    expect(nicks('#a')).toEqual(['~@bob', '@+ann', '%cat', '+zed', 'me']);
  });
});

describe('IRCv3 membership updates', () => {
  it('extended-join records account and realname', () => {
    connect(app, { caps: ['extended-join'] });
    joinWith('#a', 'me');
    feed(app, ':ann!a@host.example JOIN #a annacct :Ann Example', ':bob!b@h JOIN #a * :Bob');
    expect(buf('#a').member('ann')).toMatchObject({ account: 'annacct', realname: 'Ann Example', user: 'a', host: 'host.example' });
    expect(buf('#a').member('bob')!.account).toBeUndefined();
    expect(lines('#a').map((l) => [l.type, l.nick, l.data?.host])).toEqual([
      ['join', 'ann', 'a@host.example'],
      ['join', 'bob', 'b@h'],
    ]);
  });

  it('userhost-in-names fills user and host', () => {
    connect(app, { caps: ['userhost-in-names'] });
    joinWith('#a', '@me!u@h ann!a@ann.host');
    expect(buf('#a').member('ann')).toMatchObject({ nick: 'ann', user: 'a', host: 'ann.host', prefix: '' });
    expect(buf('#a').member('me')!.prefix).toBe('@');
  });

  it('keeps away and account across a NAMES refresh', () => {
    connect(app, { caps: ['away-notify', 'account-notify'] });
    joinWith('#a', 'me ann');
    feed(app, ':ann!a@h AWAY :lunch', ':ann!a@h ACCOUNT annacct');
    feed(app, ':srv 353 me = #a :me @ann', ':srv 366 me #a :End');
    expect(buf('#a').member('ann')).toMatchObject({ prefix: '@', away: true, account: 'annacct' });
  });

  it('chghost, setname, account-notify and away-notify patch every channel', () => {
    connect(app, { caps: ['chghost', 'setname', 'account-notify', 'away-notify'] });
    joinWith('#a', 'me ann');
    joinWith('#b', 'me ann');
    feed(app, ':ann!a@old CHGHOST newuser new.host', ':ann!newuser@new.host SETNAME :Ann Two');
    feed(app, ':ann!a@h ACCOUNT annacct', ':ann!a@h AWAY :brb');
    for (const c of ['#a', '#b']) {
      expect(buf(c).member('ann')).toMatchObject({ user: 'newuser', host: 'new.host', realname: 'Ann Two', account: 'annacct', away: true });
    }
    feed(app, ':ann!a@h ACCOUNT *', ':ann!a@h AWAY');
    expect(buf('#a').member('ann')).toMatchObject({ account: undefined, away: false });
    // None of these produce chat lines.
    expect(lines('#a')).toEqual([]);
  });

  it('tracks our own away state', () => {
    connect(app);
    feed(app, ':srv 306 me :You have been marked as being away');
    expect(conn().away).toBe(true);
    feed(app, ':srv 305 me :You are no longer marked as being away');
    expect(conn().away).toBe(false);
  });
});

describe('TOPIC and NAMES', () => {
  it('stores the topic from 332/333 without a line', () => {
    connect(app);
    joinWith('#a', 'me');
    feed(app, ':srv 332 me #a :Welcome to #a', ':srv 333 me #a ann!a@h 1700000000');
    expect(buf('#a')).toMatchObject({ topic: 'Welcome to #a', topicBy: 'ann', topicTime: 1700000000_000 });
    expect(lines('#a')).toEqual([]);
    feed(app, ':srv 331 me #a :No topic is set');
    expect(buf('#a').topic).toBe('');
  });

  it('a live TOPIC updates state and adds a line', () => {
    connect(app);
    joinWith('#a', 'me');
    feed(app, '@time=2026-01-02T03:04:05.000Z :bob!b@h TOPIC #a :new topic');
    expect(buf('#a')).toMatchObject({ topic: 'new topic', topicBy: 'bob', topicTime: Date.parse('2026-01-02T03:04:05.000Z') });
    expect(lines('#a').at(-1)).toMatchObject({ type: 'topic', nick: 'bob', text: 'new topic' });
  });

  it('accumulates NAMES across 353s and swaps on 366', () => {
    connect(app);
    feed(app, ':me!u@h JOIN #a', ':srv 353 me = #a :me ann', ':srv 353 me = #a :@bob');
    // Nothing visible until the list is complete.
    expect(buf('#a').members.size).toBe(0);
    feed(app, ':srv 366 me #a :End');
    expect(nicks('#a')).toEqual(['@bob', 'ann', 'me']);
    // A refresh replaces the list rather than adding to it.
    feed(app, ':srv 353 me = #a :me', ':srv 366 me #a :End');
    expect(nicks('#a')).toEqual(['me']);
  });

  it('ignores NAMES for channels we have no buffer for', () => {
    connect(app);
    feed(app, ':srv 353 me = #nope :ann bob', ':srv 366 me #nope :End');
    expect(conn().buffer('#nope')).toBeUndefined();
  });
});

describe('echo-message, labels, redaction', () => {
  it('shows our message once when the server echoes it', () => {
    connect(app, { caps: ['echo-message', 'message-tags'] });
    joinWith('#a', 'me');
    app.sendMessage(buf('#a'), 'hello');
    expect(sent).toContain('PRIVMSG #a :hello');
    expect(lines('#a')).toEqual([]);
    feed(app, '@msgid=e1 :me!u@h PRIVMSG #a :hello');
    expect(lines('#a').map((l) => [l.text, l.self, l.msgid])).toEqual([['hello', true, 'e1']]);
  });

  it('files echoed private messages under the recipient', () => {
    connect(app, { caps: ['echo-message'] });
    feed(app, ':me!u@h PRIVMSG ann :psst');
    expect(buf('ann').kind).toBe('query');
    expect(lines('ann')[0]).toMatchObject({ self: true, text: 'psst', highlight: false });
    expect(notify).not.toHaveBeenCalled();
  });

  it('reactions wait for the echo', () => {
    connect(app, { caps: ['echo-message', 'message-tags'] });
    joinWith('#a', 'me');
    feed(app, '@msgid=m1 :ann!a@h PRIVMSG #a :ship it');
    app.react(buf('#a'), lines('#a')[0], '🚀');
    expect(lines('#a')[0].reactions).toBeUndefined();
    feed(app, '@+draft/react=🚀;+draft/reply=m1 :me!u@h TAGMSG #a');
    feed(app, '@+draft/react=🚀;+draft/reply=m1 :Me!u@h TAGMSG #a');
    expect(lines('#a')[0].reactions).toEqual({ '🚀': ['me'] });
  });

  it('routes labeled replies (and labeled errors) to the asking buffer', () => {
    connect(app, { caps: ['labeled-response'] });
    joinWith('#a', 'me');
    joinWith('#b', 'me');
    app.open(buf('#b'));
    conn().sendFrom(buf('#a'), 'TOPIC #zzz');
    const label = /^@label=(\S+) /.exec(sent.at(-1)!)![1];
    feed(app, `@label=${label} :srv 403 me #zzz :No such channel`);
    expect(lines('#a').at(-1)).toMatchObject({ type: 'error', text: '#zzz No such channel' });
    expect(lines('#b')).toEqual([]);
  });

  it('marks redacted messages in channels and queries', () => {
    connect(app, { caps: ['draft/message-redaction', 'message-tags'] });
    joinWith('#a', 'me');
    feed(app, '@msgid=m1 :ann!a@h PRIVMSG #a :oops', '@msgid=m2 :ann!a@h PRIVMSG #a :fine');
    feed(app, '@+draft/react=👍;+draft/reply=m1 :bob!b@h TAGMSG #a');
    feed(app, ':ann!a@h REDACT #a m1 :typo');
    expect(lines('#a').map((l) => [l.msgid, !!l.redacted])).toEqual([
      ['m1', true],
      ['m2', false],
    ]);
    expect(lines('#a')[0].reactions).toBeUndefined();

    feed(app, '@msgid=p1 :ann!a@h PRIVMSG me :secret');
    feed(app, ':ann!a@h REDACT me p1');
    expect(lines('ann')[0].redacted).toBe(true);
  });

  it('lets ops and authors redact', () => {
    connect(app, { caps: ['draft/message-redaction'] });
    joinWith('#a', 'me ann');
    feed(app, '@msgid=m1 :ann!a@h PRIVMSG #a :x', '@msgid=m2 :me!u@h PRIVMSG #a :mine');
    const [theirs, mine] = lines('#a');
    expect(app.canRedact(buf('#a'), theirs)).toBe(false);
    expect(app.canRedact(buf('#a'), mine)).toBe(true);
    feed(app, ':srv MODE #a +o me');
    expect(app.canRedact(buf('#a'), theirs)).toBe(true);
    app.redact(buf('#a'), theirs);
    expect(sent.at(-1)).toBe('REDACT #a m1');
  });
});

describe('typing and read markers', () => {
  it('tracks typing notifications per buffer', () => {
    connect(app, { caps: ['message-tags'] });
    joinWith('#a', 'me ann bob');
    feed(app, '@+typing=active :ann!a@h TAGMSG #a', '@+typing=paused :bob!b@h TAGMSG #a');
    const typing = buf('#a').typing;
    expect([...typing.keys()].sort()).toEqual(['ann', 'bob']);
    expect(typing.get('bob')! - typing.get('ann')!).toBeGreaterThan(20_000);
    feed(app, '@+typing=done :bob!b@h TAGMSG #a');
    expect(typing.has('bob')).toBe(false);
    // Sending a message clears the indicator.
    feed(app, ':ann!a@h PRIVMSG #a :done typing');
    expect(typing.size).toBe(0);
  });

  it('files private typing under the sender and skips our own', () => {
    connect(app, { caps: ['message-tags'] });
    feed(app, ':ann!a@h PRIVMSG me :hi', '@+typing=active :ann!a@h TAGMSG me');
    expect(buf('ann').typing.has('ann')).toBe(true);
    joinWith('#a', 'me');
    feed(app, '@+typing=active :me!u@h TAGMSG #a');
    expect(buf('#a').typing.size).toBe(0);
  });

  it('respects the showTyping setting', () => {
    connect(app, { caps: ['message-tags'] });
    app.settings.showTyping = false;
    joinWith('#a', 'me ann');
    feed(app, '@+typing=active :ann!a@h TAGMSG #a');
    expect(buf('#a').typing.size).toBe(0);
  });

  it('sends typing only when allowed and throttled', () => {
    connect(app, { caps: ['message-tags'], isupport: { CLIENTTAGDENY: '*,-typing' } });
    joinWith('#a', 'me');
    app.sendTyping(buf('#a'), 'active');
    app.sendTyping(buf('#a'), 'active');
    app.sendTyping(buf('#a'), 'done');
    expect(sent).toEqual(['@+typing=active TAGMSG #a', '@+typing=done TAGMSG #a']);
    sent.length = 0;
    conn().isupport = { ...conn().isupport, CLIENTTAGDENY: 'typing' };
    app.sendTyping(buf('#a'), 'active');
    expect(sent).toEqual([]);
  });

  it('moves the read marker forward only', () => {
    connect(app, { caps: ['draft/read-marker'] });
    joinWith('#a', 'me');
    expect(buf('#a').readMarker).toBe(0);
    feed(app, ':srv MARKREAD #a timestamp=2026-01-01T00:00:10.000Z');
    expect(buf('#a').readMarker).toBe(Date.parse('2026-01-01T00:00:10.000Z'));
    feed(app, ':srv MARKREAD #a timestamp=2026-01-01T00:00:05.000Z', ':srv MARKREAD #a *');
    expect(buf('#a').readMarker).toBe(Date.parse('2026-01-01T00:00:10.000Z'));
  });

  it('counts unread lines after the marker and syncs markRead', async () => {
    vi.useFakeTimers();
    connect(app, { caps: ['draft/read-marker'] });
    joinWith('#a', 'me');
    feed(app, '@time=2026-01-01T00:00:01.000Z :ann!a@h PRIVMSG #a :one', '@time=2026-01-01T00:00:02.000Z :ann!a@h PRIVMSG #a :two');
    feed(app, ':srv MARKREAD #a timestamp=2026-01-01T00:00:01.000Z');
    expect(buf('#a').unread).toBe(1);
    app.markRead(buf('#a'));
    expect(buf('#a').unread).toBe(0);
    await vi.advanceTimersByTimeAsync(500);
    expect(sent.at(-1)).toBe('MARKREAD #a timestamp=2026-01-01T00:00:02.000Z');
    vi.useRealTimers();
  });
});

describe('PRIVMSG, NOTICE and CTCP', () => {
  it('shows ACTIONs as action lines', () => {
    connect(app);
    joinWith('#a', 'me ann');
    feed(app, ':ann!a@h PRIVMSG #a :\x01ACTION waves at me\x01', ':ann!a@h PRIVMSG #a :\x01ACTION\x01');
    expect(lines('#a').map((l) => [l.type, l.text, l.highlight])).toEqual([
      ['action', 'waves at me', true],
      ['action', '', false],
    ]);
  });

  it('answers CTCP requests and reports replies without chat lines', () => {
    connect(app);
    joinWith('#a', 'me ann');
    app.open(buf('#a'));
    feed(app, ':ann!a@h PRIVMSG me :\x01PING 12345\x01', ':ann!a@h PRIVMSG #a :\x01VERSION\x01', ':ann!a@h PRIVMSG me :\x01FOO\x01');
    expect(sent).toEqual(['NOTICE ann :\x01PING 12345\x01', expect.stringMatching(/^NOTICE ann :\x01VERSION .+\x01$/)]);
    feed(app, ':ann!a@h NOTICE me :\x01VERSION irssi\x01');
    expect(texts('#a')).toEqual([
      'CTCP PING request from ann',
      'CTCP VERSION request from ann',
      'CTCP FOO request from ann',
      'CTCP VERSION reply from ann: irssi',
    ]);
    expect(conn().buffer('ann')).toBeUndefined();
  });

  it('routes server notices to the server buffer', () => {
    connect(app);
    joinWith('#a', 'me');
    app.open(buf('#a'));
    feed(app, ':irc.example.net NOTICE * :*** Looking up your hostname', ':irc.example.net NOTICE me :server says hi', 'NOTICE me :sourceless');
    expect(serverTexts()).toEqual(['*** Looking up your hostname', 'server says hi', 'sourceless']);
    expect(lines('#a')).toEqual([]);
  });

  it('routes user notices to an open query, else the active buffer', () => {
    connect(app);
    joinWith('#a', 'me');
    app.open(buf('#a'));
    feed(app, ':NickServ!s@services. NOTICE me :Please identify');
    expect(conn().buffer('NickServ')).toBeUndefined();
    expect(lines('#a').at(-1)).toMatchObject({ type: 'notice', nick: 'NickServ', text: 'Please identify' });
    feed(app, ':ann!a@h PRIVMSG me :hi', ':ann!a@h NOTICE me :fyi');
    expect(types('ann')).toEqual(['privmsg', 'notice']);
    feed(app, ':ann!a@h NOTICE #a :channel notice');
    expect(lines('#a').at(-1)).toMatchObject({ type: 'notice', text: 'channel notice' });
  });

  it('strips STATUSMSG prefixes', () => {
    connect(app, { isupport: { STATUSMSG: '@+' } });
    joinWith('#a', 'me');
    feed(app, ':ann!a@h NOTICE @#a :ops only');
    expect(texts('#a')).toEqual(['ops only']);
    expect(conn().buffer('@#a')).toBeUndefined();
  });

  it('highlights on nick and configured words as whole words', () => {
    connect(app);
    app.settings.highlightWords = ['hearth', 'c++'];
    joinWith('#a', 'me ann');
    feed(
      app,
      ':ann!a@h PRIVMSG #a :ME, look',
      ':ann!a@h PRIVMSG #a :meme time',
      ':ann!a@h PRIVMSG #a :I love Hearth!',
      ':ann!a@h PRIVMSG #a :hearthstone',
      ':ann!a@h PRIVMSG #a :learning c++ today',
      ':ann!a@h PRIVMSG #a :\x02me\x02 in bold',
    );
    expect(lines('#a').map((l) => l.highlight)).toEqual([true, false, true, false, true, true]);
    // Our own messages never highlight.
    feed(app, ':me!u@h PRIVMSG #a :hearth me');
    expect(lines('#a').at(-1)!.highlight).toBe(false);
  });

  it('notifies for highlights and private messages when not looking', () => {
    connect(app);
    joinWith('#a', 'me ann');
    app.focused = false;
    feed(app, ':ann!a@h PRIVMSG #a :chatter', ':ann!a@h PRIVMSG #a :me: ping', ':ann!a@h PRIVMSG me :dm');
    expect(vi.mocked(notify).mock.calls.map((c) => c[0].split(' · ')[0])).toEqual(['ann in #a', 'ann']);
  });

  it('drops everything from ignored masks, case-insensitively', () => {
    connect(app, { isupport: { CASEMAPPING: 'rfc1459' } });
    app.settings.ignores = ['troll[1]!*@*', '*!*@*.spam.example'];
    joinWith('#a', 'me');
    app.open(buf('#a'));
    feed(
      app,
      ':TROLL{1}!x@y PRIVMSG #a :hi',
      ':bot!b@node.spam.example PRIVMSG me :buy now',
      ':bot!b@node.spam.example INVITE me #spam',
      ':bot!b@node.spam.example NOTICE me :notice spam',
      ':ok!o@good.example PRIVMSG #a :real',
    );
    expect(texts('#a')).toEqual(['real']);
    expect(conn().buffer('bot')).toBeUndefined();
  });

  it('announces invites', () => {
    connect(app);
    feed(app, ':ann!a@h INVITE me #party', ':ann!a@h INVITE bob #party');
    expect(conn().server.messages.map((l) => [l.text, l.highlight])).toEqual([
      ['invited you to #party', true],
      ['invited bob to #party', false],
    ]);
  });
});

describe('ISUPPORT', () => {
  it('folds nicks and channels with rfc1459 casemapping', () => {
    connect(app, { isupport: { CASEMAPPING: 'rfc1459' } });
    feed(app, ':me!u@h JOIN #Chan[x]', ':Ann[away]!a@h JOIN #chan{X}', ':ann{AWAY}!a@h PRIVMSG #CHAN{x} :hi');
    expect(conn().buffers.size).toBe(1);
    const b = buf('#chan{x}');
    expect(b.name).toBe('#Chan[x]');
    expect(b.member('ANN{away}')!.nick).toBe('Ann[away]');
    expect(texts('#Chan[x]')).toEqual(['', 'hi']);
    feed(app, ':ann~!a@h PRIVMSG me :tilde', ':ANN^!a@h PRIVMSG me :caret');
    expect(texts('ann^')).toEqual(['tilde', 'caret']);
    expect(conn().isMe('ME')).toBe(true);
  });

  it('ascii casemapping keeps brackets distinct', () => {
    connect(app, { isupport: { CASEMAPPING: 'ascii' } });
    feed(app, ':a[1]!a@h PRIVMSG me :x', ':A{1}!a@h PRIVMSG me :y');
    expect(texts('a[1]')).toEqual(['x']);
    expect(texts('a{1}')).toEqual(['y']);
  });

  it('uses CHANTYPES to tell channels from nicks', () => {
    connect(app, { isupport: { CHANTYPES: '#&' } });
    feed(app, ':me!u@h JOIN &local', ':ann!a@h PRIVMSG &local :hey');
    expect(buf('&local').kind).toBe('channel');
    expect(texts('&local')).toEqual(['hey']);
    expect(conn().isChannel('!odd')).toBe(false);
    expect(conn().isChannel('')).toBe(false);
  });

  it('defaults PREFIX when it is missing or malformed', () => {
    connect(app, { isupport: { PREFIX: '(ohv)@+' } });
    expect(conn().prefix).toEqual({ modes: 'ov', symbols: '@+' });
    conn().isupport = { ...conn().isupport, PREFIX: '(yqaohv)!~&@%+' };
    expect(conn().prefix).toEqual({ modes: 'yqaohv', symbols: '!~&@%+' });
  });
});

describe('numerics', () => {
  it('routes errors about a buffer to that buffer', () => {
    connect(app);
    joinWith('#a', 'me');
    joinWith('#b', 'me');
    feed(app, ':ann!a@h PRIVMSG me :hi');
    app.open(buf('#b'));
    feed(app, ':srv 404 me #a :Cannot send to channel');
    feed(app, ':srv 482 me #a :You\'re not channel operator');
    feed(app, ':srv 401 me ann :No such nick/channel');
    expect(lines('#a').slice(-2).map((l) => [l.type, l.text])).toEqual([
      ['error', '#a Cannot send to channel'],
      ['error', "#a You're not channel operator"],
    ]);
    expect(lines('ann').at(-1)).toMatchObject({ type: 'error', text: 'ann No such nick/channel' });
    expect(lines('#b')).toEqual([]);
  });

  it('falls back to the active buffer, then the server buffer', () => {
    connect(app);
    feed(app, ':srv 403 me #nowhere :No such channel');
    expect(conn().server.messages.at(-1)).toMatchObject({ type: 'error', text: '#nowhere No such channel' });
    joinWith('#a', 'me');
    app.open(buf('#a'));
    feed(app, ':srv 401 me ghost :No such nick/channel', ':srv 421 me FOO :Unknown command');
    expect(lines('#a').map((l) => l.text)).toEqual(['ghost No such nick/channel', 'FOO Unknown command']);
  });

  it('reports nick collisions and SASL failures', () => {
    connect(app);
    feed(app, ':srv 433 me taken :Nickname is already in use', ':srv 904 me :SASL authentication failed');
    expect(conn().server.messages.map((l) => [l.type, l.text])).toEqual([
      ['error', 'Nickname taken is already in use'],
      ['error', 'SASL: SASL authentication failed'],
    ]);
  });

  it('collects the MOTD into one block, dropping the 375 header', () => {
    connect(app);
    feed(app, ':srv 375 me :- srv Message of the day', ':srv 372 me :- line one', ':srv 372 me :-line two', ':srv 376 me :End');
    expect(conn().server.messages).toHaveLength(1);
    expect(conn().server.messages[0].data).toEqual({ motd: 'line one\nline two' });
  });

  it('assembles WHOIS replies into one card', () => {
    connect(app);
    feed(
      app,
      ':srv 311 me ann a host.example * :Ann Example',
      ':srv 312 me ann irc.example :The Server',
      ':srv 319 me ann :@#a',
      ':srv 319 me ann :+#b',
      ':srv 330 me ann annacct :is logged in as',
      ':srv 301 me ann :lunch',
      ':srv 318 me ann :End of /WHOIS list.',
    );
    const card = conn().server.messages.at(-1)!;
    expect(card.type).toBe('whois');
    expect(card.data).toMatchObject({
      nick: 'ann',
      user: 'a',
      host: 'host.example',
      realname: 'Ann Example',
      server: 'irc.example — The Server',
      channels: '@#a +#b',
      account: 'annacct',
      away: 'lunch',
    });
  });

  it('shows FAIL for the asking buffer and hides CHATHISTORY failures', () => {
    connect(app);
    feed(app, 'FAIL CHATHISTORY INVALID_TARGET #a :nope', 'FAIL NICK ERRONEOUS_NICKNAME * :Bad nick', 'NOTE SERVER RESTART :soon');
    expect(conn().server.messages.map((l) => [l.type, l.text])).toEqual([
      ['error', 'NICK: Bad nick'],
      ['info', 'SERVER: soon'],
    ]);
  });
});
