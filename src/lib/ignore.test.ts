import { describe, expect, it } from 'vitest';
import { isIgnored, normalizeMask } from './ignore';
import { casefold, splitMessage } from './irc';

describe('ignore masks', () => {
  it('normalizes shorthand', () => {
    expect(normalizeMask('nick')).toBe('nick!*@*');
    expect(normalizeMask('*@host')).toBe('*!*@host');
    expect(normalizeMask('nick!user')).toBe('nick!user@*');
    expect(normalizeMask(' a!b@c ')).toBe('a!b@c');
  });

  it('matches with wildcards and case-folding', () => {
    const fold = (s: string) => casefold(s, 'rfc1459');
    expect(isIgnored(['Troll[1]'], { nick: 'troll{1}', user: 'x', host: 'y' }, fold)).toBe(true);
    expect(isIgnored(['*!*@*.bad.net'], { nick: 'a', user: 'b', host: 'x.bad.net' }, fold)).toBe(true);
    expect(isIgnored(['a?c'], { nick: 'abc' }, fold)).toBe(true);
    expect(isIgnored(['a?c'], { nick: 'abbc' }, fold)).toBe(false);
    expect(isIgnored(['x.y'], { nick: 'xzy' }, fold)).toBe(false);
  });
});

describe('splitMessage', () => {
  it('splits on word boundaries within the byte budget', () => {
    const parts = splitMessage('lorem ipsum dolor sit amet '.repeat(20).trim(), 100);
    expect(parts.every((p) => new TextEncoder().encode(p).length <= 100)).toBe(true);
    expect(parts.join(' ')).toBe('lorem ipsum dolor sit amet '.repeat(20).trim());
  });

  it('never splits inside a multi-byte character', () => {
    const parts = splitMessage('é'.repeat(100), 51);
    expect(parts.join('')).toBe('é'.repeat(100));
    expect(parts.every((p) => new TextEncoder().encode(p).length <= 51)).toBe(true);
  });
});
