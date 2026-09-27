import { describe, expect, it } from 'vitest';
import { explainError, nickError, parseChannels, presets } from './onboarding';

describe('nickError', () => {
  it('accepts ordinary and special-character nicks', () => {
    for (const n of ['ember', 'Ember_', '[away]', 'a-b', 'x^y', '`q', '{z}|w\\']) expect(nickError(n)).toBeNull();
  });

  it('rejects empty, spaced and badly-started nicks', () => {
    expect(nickError('')).toMatch(/Choose/);
    expect(nickError('my nick')).toMatch(/spaces/);
    expect(nickError('9lives')).toMatch(/digit/);
    expect(nickError('-dash')).toMatch(/digit or a dash/);
  });

  it('names the offending character', () => {
    expect(nickError('ember!')).toContain('“!”');
    expect(nickError('émile')).toContain('“é”');
  });

  it('enforces the network NICKLEN', () => {
    expect(nickError('abcdefghij', 9, 'EFnet')).toMatch(/9 characters or fewer on EFnet/);
    expect(nickError('abcdefghi', 9)).toBeNull();
  });
});

describe('parseChannels', () => {
  it('adds # to bare names and drops duplicates', () => {
    expect(parseChannels('#libera, rust  #Rust\n&local')).toEqual(['#libera', '#rust', '&local']);
    expect(parseChannels('  ')).toEqual([]);
  });
});

describe('explainError', () => {
  const ctx = { host: 'irc.example', port: 6697, tls: true, sasl: false };

  it('explains common transport failures', () => {
    expect(explainError('could not reach irc.example:6697: failed to lookup address information', ctx).title).toMatch(/find irc.example/);
    expect(explainError('could not reach irc.example:6697: Connection refused (os error 61)', ctx).title).toMatch(/refused/);
    expect(explainError('TLS handshake failed: invalid peer certificate: UnknownIssuer', ctx).title).toMatch(/secure/);
    expect(explainError('timed out connecting to irc.example:6697', ctx).title).toMatch(/didn't answer/);
  });

  it('tells SASL-required networks apart from bad credentials', () => {
    expect(explainError('Closing Link: (SASL access only)', ctx).title).toMatch(/log in first/);
    expect(explainError('SASL authentication failed', { ...ctx, sasl: true }).title).toMatch(/not accepted/);
  });

  it('falls back to a generic message', () => {
    expect(explainError('something odd', ctx).title).toBe("Couldn't connect");
  });
});

describe('presets', () => {
  it('have TLS defaults on the standard port', () => {
    for (const p of presets.filter((p) => p.kind === 'network')) {
      expect(p.host).toMatch(/\./);
      expect(p.tls).toBe(true);
      expect(p.port).toBe(6697);
    }
  });
});
