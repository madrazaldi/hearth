import { expect, test } from 'bun:test';
import { Scram } from './scram';

test('rfc7677_vector', () => {
  const s = new Scram('user', 'pencil', 'rOprNGfwEbeRWgbNEkqO');
  expect(s.clientFirst()).toBe('n,,n=user,r=rOprNGfwEbeRWgbNEkqO');
  const serverFirst = 'r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096';
  expect(s.clientFinal(serverFirst)).toBe(
    'c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ=',
  );
  expect(() => s.verify('v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=')).not.toThrow();
  expect(() => s.verify('v=AAAA')).toThrow();
  expect(() => s.verify('e=invalid-proof')).toThrow('invalid-proof');
});

test('rejects_foreign_nonce_and_escapes_names', () => {
  const s = new Scram('a,b=c', 'pw', 'abc');
  expect(s.clientFirst()).toBe('n,,n=a=2Cb=3Dc,r=abc');
  expect(() => s.clientFinal('r=zzz,s=AAAA,i=4096')).toThrow('nonce');
  expect(() => s.clientFinal('r=abcdef,s=AAAA,i=0')).toThrow('iteration');
  expect(() => s.clientFinal('r=abcdef,s=AAAA,i=99999999')).toThrow('iteration');
  expect(() => s.clientFinal('r=abcdef,s=AAAA')).toThrow("missing 'i'");
  expect(() => s.clientFinal('r=abcdef,s=@@@,i=4096')).toThrow('salt');
});

test('verify before the client-final message fails', () => {
  expect(() => new Scram('u', 'p').verify('v=AAAA')).toThrow();
});

test('random nonces differ', () => {
  expect(new Scram('u', 'p').clientFirst()).not.toBe(new Scram('u', 'p').clientFirst());
});
