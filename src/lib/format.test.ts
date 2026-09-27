import { describe, expect, it } from 'vitest';
import { firstLink, mentions, parseFormatting, stripFormatting, type Segment } from './format';

const B = '\x02';
const C = '\x03';
const HEX = '\x04';
const I = '\x1d';
const U = '\x1f';
const S = '\x1e';
const M = '\x11';
const R = '\x16';
const O = '\x0f';

/** Segments with undefined/false keys dropped, for compact comparisons. */
const seg = (s: string) =>
  parseFormatting(s).map((x) => Object.fromEntries(Object.entries(x).filter(([, v]) => v !== undefined && v !== false)) as Segment);
const links = (s: string) => parseFormatting(s).filter((x) => x.href).map((x) => x.href);
const chans = (s: string) => parseFormatting(s).filter((x) => x.channel).map((x) => x.channel);

describe('mIRC formatting', () => {
  it('toggles bold, italic, underline, strike and monospace', () => {
    expect(seg(`a${B}b${I}c${B}d${I}${U}e${U}${S}f${S}${M}g`)).toEqual([
      { text: 'a' },
      { text: 'b', bold: true },
      { text: 'c', bold: true, italic: true },
      { text: 'd', italic: true },
      { text: 'e', underline: true },
      { text: 'f', strike: true },
      { text: 'g', mono: true },
    ]);
  });

  it('resets everything with ^O', () => {
    expect(seg(`${B}${C}4bold red${O}plain`)).toEqual([{ text: 'bold red', bold: true, fg: 'var(--irc-4)' }, { text: 'plain' }]);
  });

  it('parses basic colours with and without backgrounds', () => {
    expect(seg(`${C}4red${C}04,2red on blue${C}3green keeps bg${C}plain`)).toEqual([
      { text: 'red', fg: 'var(--irc-4)' },
      { text: 'red on blue', fg: 'var(--irc-4)', bg: 'var(--irc-2)' },
      { text: 'green keeps bg', fg: 'var(--irc-3)', bg: 'var(--irc-2)' },
      { text: 'plain' },
    ]);
  });

  it('reads at most two digits, so trailing digits are text', () => {
    expect(seg(`${C}1234`)).toEqual([{ text: '34', fg: 'var(--irc-12)' }]);
    expect(seg(`${C}4,123`)).toEqual([{ text: '3', fg: 'var(--irc-4)', bg: 'var(--irc-12)' }]);
  });

  it('keeps a comma that is not followed by a colour', () => {
    expect(seg(`${C}4,hello`)).toEqual([{ text: ',hello', fg: 'var(--irc-4)' }]);
    expect(seg(`${C},4x`)).toEqual([{ text: ',4x' }]);
  });

  it('maps the extended 16–98 palette and treats 99 as default', () => {
    expect(seg(`${C}16a${C}52b${C}98c${C}99d`)).toEqual([
      { text: 'a', fg: '#470000' },
      { text: 'b', fg: '#ff0000' },
      { text: 'c', fg: '#ffffff' },
      { text: 'd' },
    ]);
    expect(seg(`${C}88,99x`)).toEqual([{ text: 'x', fg: '#000000' }]);
  });

  it('parses hex colours', () => {
    expect(seg(`${HEX}FF8800orange${HEX}00ff00,000000green on black${HEX}reset`)).toEqual([
      { text: 'orange', fg: '#FF8800' },
      { text: 'green on black', fg: '#00ff00', bg: '#000000' },
      { text: 'reset' },
    ]);
    // Not six hex digits: a plain reset, the rest is text.
    expect(seg(`${HEX}12345x`)).toEqual([{ text: '12345x' }]);
  });

  it('reverses colours', () => {
    expect(seg(`${R}rev`)).toEqual([{ text: 'rev', fg: 'var(--bg)', bg: 'var(--text)' }]);
    expect(seg(`${C}4,2${R}rev`)).toEqual([{ text: 'rev', fg: 'var(--irc-2)', bg: 'var(--irc-4)' }]);
  });

  it('drops empty segments', () => {
    expect(seg(`${B}${B}${C}4${C}`)).toEqual([]);
  });

  it('stripFormatting removes every control code', () => {
    expect(stripFormatting(`${B}bold${B} ${C}04,12col${C} ${HEX}ff0000,00ff00hex ${I}${U}${S}${M}${R}x${O}`)).toBe('bold col hex x');
    expect(stripFormatting(`${C}99,99plain`)).toBe('plain');
  });
});

describe('link detection', () => {
  it('finds http(s), www and mailto links', () => {
    expect(links('see https://example.com/a?b=1&c=2#frag and http://x.org')).toEqual([
      'https://example.com/a?b=1&c=2#frag',
      'http://x.org',
    ]);
    expect(links('www.example.com/path')).toEqual(['https://www.example.com/path']);
    expect(links('mail mailto:ann@example.com')).toEqual(['mailto:ann@example.com']);
  });

  it('trims trailing punctuation', () => {
    expect(links('go to https://example.com.')).toEqual(['https://example.com']);
    expect(links('https://example.com/a, then')).toEqual(['https://example.com/a']);
    expect(links('really? https://example.com/q?!')).toEqual(['https://example.com/q']);
    expect(links('"https://example.com/quoted"')).toEqual(['https://example.com/quoted']);
    expect(links('<https://example.com/angle>')).toEqual(['https://example.com/angle']);
    expect(links('[https://example.com/sq]')).toEqual(['https://example.com/sq']);
  });

  it('balances parentheses', () => {
    expect(links('https://en.wikipedia.org/wiki/Foo_(bar)')).toEqual(['https://en.wikipedia.org/wiki/Foo_(bar)']);
    expect(links('(see https://en.wikipedia.org/wiki/Foo_(bar))')).toEqual(['https://en.wikipedia.org/wiki/Foo_(bar)']);
    expect(links('(https://example.com)')).toEqual(['https://example.com']);
    expect(links('(https://example.com).')).toEqual(['https://example.com']);
  });

  it('keeps surrounding text and formatting around a link', () => {
    expect(seg(`hi ${B}https://a.io/x.${B} ok`)).toEqual([
      { text: 'hi ' },
      { text: 'https://a.io/x', bold: true, href: 'https://a.io/x' },
      { text: '.', bold: true },
      { text: ' ok' },
    ]);
  });

  it('needs a word boundary before the scheme', () => {
    expect(links('xhttps://nope.example')).toEqual([]);
    expect(links('notwww.example.com')).toEqual([]);
  });

  it('firstLink returns the first previewable link', () => {
    expect(firstLink('mailto:a@b.c then (www.example.com).')).toBe('https://www.example.com');
    expect(firstLink(`${C}4https://red.example${C}!`)).toBe('https://red.example');
    expect(firstLink('no links here')).toBeNull();
  });
});

describe('channel links', () => {
  it('linkifies #channels at word starts', () => {
    expect(chans('join #hearth, #rust-lang and (#svelte)')).toEqual(['#hearth', '#rust-lang', '#svelte']);
    expect(chans('#start of line')).toEqual(['#start']);
    expect(chans('see #hearth.')).toEqual(['#hearth']);
  });

  it('skips mid-word hashes and URL fragments', () => {
    expect(chans('issue#12 c#')).toEqual([]);
    expect(chans('https://example.com/ #real https://x.io/#frag')).toEqual(['#real']);
    expect(links('https://example.com/ #real https://x.io/#frag')).toEqual(['https://example.com/', 'https://x.io/#frag']);
  });

  it('splits the text around the channel', () => {
    expect(seg('go to #a now')).toEqual([{ text: 'go to ' }, { text: '#a', channel: '#a' }, { text: ' now' }]);
  });
});

describe('mentions', () => {
  it('matches whole words case-insensitively', () => {
    expect(mentions('Ann: hi', ['ann'])).toBe(true);
    expect(mentions('hi @ann!', ['ann'])).toBe(true);
    expect(mentions('annabelle', ['ann'])).toBe(false);
    expect(mentions('ann-marie', ['ann'])).toBe(false);
    expect(mentions('joann', ['ann'])).toBe(false);
  });

  it('handles regex metacharacters and nick punctuation', () => {
    expect(mentions('hey [ann]', ['[ann]'])).toBe(true);
    expect(mentions('ping ann|away', ['ann|away'])).toBe(true);
    expect(mentions('ping ann', ['ann|away'])).toBe(false);
    expect(mentions('I use c++', ['c++'])).toBe(true);
  });

  it('ignores formatting and empty words', () => {
    expect(mentions(`${B}a${B}nn`, ['ann'])).toBe(true);
    expect(mentions('anything', [''])).toBe(false);
  });
});
