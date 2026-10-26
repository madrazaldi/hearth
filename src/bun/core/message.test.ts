import { describe, expect, test } from 'bun:test';
import { casefold, message, parse, serialize, tag } from './message';

describe('message', () => {
  test('parses_full_message', () => {
    const m = parse('@time=2026-01-01T00:00:00.000Z;msgid=a\\sb :nick!user@host PRIVMSG #chan :hello there')!;
    expect(tag(m, 'time')).toBe('2026-01-01T00:00:00.000Z');
    expect(tag(m, 'msgid')).toBe('a b');
    expect(m.source).toEqual({ nick: 'nick', user: 'user', host: 'host' });
    expect(m.command).toBe('PRIVMSG');
    expect(m.params).toEqual(['#chan', 'hello there']);
  });

  test('parses_bare_and_trailing_edge_cases', () => {
    expect(parse('PING')!.params).toEqual([]);
    expect(parse(':srv 001 me :')!.params).toEqual(['me', '']);
    expect(parse('CAP * LS :a b=c')!.params).toEqual(['*', 'LS', 'a b=c']);
    expect(tag(parse('@+typing=active TAGMSG #x')!, '+typing')).toBe('active');
    expect(parse('')).toBeNull();
  });

  test('roundtrips', () => {
    const m = message('PRIVMSG', '#a', ':)');
    m.tags['+draft/reply'] = 'x;y z';
    const s = serialize(m);
    expect(s).toBe('@+draft/reply=x\\:y\\sz PRIVMSG #a ::)');
    expect(parse(s)).toEqual(m);
  });

  const tagOf = (line: string, key: string) => tag(parse(line)!, key);

  test('unescapes_tag_values', () => {
    expect(tagOf('@k=a\\:b\\sc\\\\d\\re\\nf CMD', 'k')).toBe('a;b c\\d\re\nf');
    // Unknown escapes drop the backslash; a trailing lone one vanishes.
    expect(tagOf('@k=\\b\\x CMD', 'k')).toBe('bx');
    expect(tagOf('@k=abc\\ CMD', 'k')).toBe('abc');
    expect(tagOf('@k=\\\\ CMD', 'k')).toBe('\\');
    // Only the first '=' separates key from value.
    expect(tagOf('@k=a=b CMD', 'k')).toBe('a=b');
    expect(tagOf('@k=café☃ CMD', 'k')).toBe('café☃');
  });

  test('tag_keys_and_empty_values', () => {
    const m = parse('@;a;b=;+example.com/c=1;;a=2 CMD')!;
    // A repeated key keeps its last value.
    expect(tag(m, 'a')).toBe('2');
    expect(tag(m, 'b')).toBe('');
    expect(tag(m, '+example.com/c')).toBe('1');
    expect(Object.keys(m.tags).length).toBe(3);
    expect(m.command).toBe('CMD');
  });

  test('escapes_roundtrip', () => {
    const nasty = '; \\\r\n\\s\\:=é';
    const m = message('TAGMSG', '#a');
    m.tags['+k'] = nasty;
    m.tags.flag = '';
    const line = serialize(m);
    expect(line.slice(0, line.indexOf(' '))).not.toMatch(/[ \r\n]/);
    // ':' and '=' need no escaping; an empty value is written bare.
    expect(line.startsWith('@+k=\\:\\s\\\\\\r\\n\\\\s\\\\:=é;flag TAGMSG')).toBe(true);
    expect(parse(line)).toEqual(m);
  });

  test('tags are not mistaken for object internals', () => {
    const m = parse('@__proto__=x;constructor=y;toString CMD')!;
    expect(tag(m, '__proto__')).toBe('x');
    expect(tag(m, 'constructor')).toBe('y');
    expect(tag(m, 'valueOf')).toBeUndefined();
    expect(tag(parse('CMD')!, 'toString')).toBeUndefined();
  });

  test('rejects_lines_without_a_command', () => {
    expect(parse('@a=1')).toBeNull();
    expect(parse(':nick')).toBeNull();
    expect(parse('@a=1 :nick ')).toBeNull();
    expect(parse('   ')).toBeNull();
  });

  test('tolerates_extra_spaces_and_lowercase_commands', () => {
    const m = parse('@a=1  :n!u@h   privmsg   #a    :hi  there\r\n')!;
    expect(m.command).toBe('PRIVMSG');
    expect(m.params).toEqual(['#a', 'hi  there']);
    expect(m.source?.host).toBe('h');
    const s = parse(':srv.example 005 me A B')!;
    expect(s.source?.nick).toBe('srv.example');
    expect(s.source?.user).toBeUndefined();
    expect(s.params).toEqual(['me', 'A', 'B']);
  });

  test('trailing_param_is_colon_prefixed_only_when_needed', () => {
    expect(serialize(message('NICK', 'me'))).toBe('NICK me');
    expect(serialize(message('PRIVMSG', '#a', ''))).toBe('PRIVMSG #a :');
    expect(serialize(message('PRIVMSG', '#a', 'a b'))).toBe('PRIVMSG #a :a b');
    expect(serialize(message('PRIVMSG', '#a', ':x'))).toBe('PRIVMSG #a ::x');
  });

  test('serializes the wire shape the UI reads', () => {
    const m = parse(':n!u@h PRIVMSG #a :hi')!;
    expect(JSON.parse(JSON.stringify(m))).toEqual({ tags: {}, source: { nick: 'n', user: 'u', host: 'h' }, command: 'PRIVMSG', params: ['#a', 'hi'] });
    expect(JSON.parse(JSON.stringify(parse(':srv 001 me :x')!)).source).toEqual({ nick: 'srv' });
    expect(JSON.parse(JSON.stringify(parse('PING x')!)).source).toBeNull();
  });

  test('casefolds', () => {
    expect(casefold('Foo[]\\~', 'rfc1459')).toBe('foo{}|^');
    expect(casefold('Foo[]', 'ascii')).toBe('foo[]');
    expect(casefold('Foo[]\\~', 'rfc1459-strict')).toBe('foo{}|~');
  });
});
