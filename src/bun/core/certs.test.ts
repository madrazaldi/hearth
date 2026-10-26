import { afterEach, describe, expect, test } from 'bun:test';
import { X509Certificate, createHash, createPrivateKey, createPublicKey } from 'node:crypto';
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createSecureContext } from 'node:tls';
import { generate, info, loadIdentity, selfSigned } from './certs';

const dirs: string[] = [];
const tmp = () => {
  const d = mkdtempSync(join(tmpdir(), 'hearth-cert-'));
  dirs.push(d);
  return d;
};
afterEach(() => {
  while (dirs.length) rmSync(dirs.pop()!, { recursive: true, force: true });
});

describe('generate', () => {
  test('generates_a_loadable_identity', () => {
    const dir = tmp();
    const made = generate(dir, 'srv1');
    expect(made.path).toBe(join(dir, 'srv1.pem'));
    expect(made.sha256).toHaveLength(64);
    expect(made.sha512).toHaveLength(128);
    expect(() => loadIdentity(made.path)).not.toThrow();
    expect(info(made.path).sha512).toBe(made.sha512);
  });

  test('writes the certificate, then the key, readable only by the owner', () => {
    const made = generate(tmp(), 'srv');
    const text = readFileSync(made.path, 'utf8');
    expect(text.indexOf('BEGIN CERTIFICATE')).toBeLessThan(text.indexOf('BEGIN PRIVATE KEY'));
    if (process.platform !== 'win32') expect(statSync(made.path).mode & 0o777).toBe(0o600);
  });

  test('the certificate is well formed and matches its key', () => {
    const made = generate(tmp(), 'srv');
    const { certs, key } = loadIdentity(made.path);
    const cert = new X509Certificate(certs[0]);
    expect(cert.subject).toBe('CN=hearth');
    expect(cert.issuer).toBe('CN=hearth');
    expect(cert.subjectAltName).toBe('DNS:hearth');
    expect(cert.publicKey.asymmetricKeyType).toBe('ec');
    expect(cert.publicKey.asymmetricKeyDetails?.namedCurve).toBe('prime256v1');
    expect(cert.checkPrivateKey(createPrivateKey(key))).toBe(true);
    expect(cert.verify(cert.publicKey)).toBe(true); // self-signed
    expect(new Date(cert.validFrom).getTime()).toBeLessThan(Date.now());
    expect(new Date(cert.validTo).getFullYear()).toBeGreaterThan(2100);
    // TLS accepts the pair, which is what a connection to the server needs.
    expect(() => createSecureContext({ cert: certs.join('\n'), key })).not.toThrow();
    expect(createPublicKey(key).export({ type: 'spki', format: 'der' }).equals(cert.publicKey.export({ type: 'spki', format: 'der' }))).toBe(true);
  });

  test('fingerprints are the digests of the DER certificate', () => {
    const made = generate(tmp(), 'srv');
    const cert = new X509Certificate(loadIdentity(made.path).certs[0]);
    expect(made.sha256).toBe(createHash('sha256').update(cert.raw).digest('hex'));
    expect(made.sha256).toBe(cert.fingerprint256.replace(/:/g, '').toLowerCase());
    expect(made.sha512).toBe(createHash('sha512').update(cert.raw).digest('hex'));
  });

  test('every certificate is different', () => {
    const dir = tmp();
    expect(generate(dir, 'a').sha256).not.toBe(generate(dir, 'b').sha256);
    expect(selfSigned().cert).not.toBe(selfSigned().cert);
  });

  test('names are sanitised, and an empty result is refused', () => {
    const dir = tmp();
    expect(generate(dir, '../../etc/pa ss').path).toBe(join(dir, 'etcpass.pem'));
    expect(() => generate(dir, '../..')).toThrow('invalid certificate name');
    expect(() => generate(dir, '')).toThrow('invalid certificate name');
  });
});

describe('loadIdentity', () => {
  test('client_certificates_must_contain_a_certificate', () => {
    expect(() => loadIdentity('/nonexistent/hearth-cert.pem')).toThrow('could not read client certificate');
    const path = join(tmp(), 'empty.pem');
    writeFileSync(path, 'not a pem file\n');
    expect(() => loadIdentity(path)).toThrow('contains no certificate');
  });

  test('a certificate without a key is refused', () => {
    const { cert } = selfSigned();
    const path = join(tmp(), 'nokey.pem');
    writeFileSync(path, cert);
    expect(() => loadIdentity(path)).toThrow('no usable private key');
  });

  test('accepts a chain, and key blocks in any order or flavour', () => {
    const a = selfSigned();
    const b = selfSigned();
    const path = join(tmp(), 'chain.pem');
    writeFileSync(path, `${a.key}${a.cert}${b.cert}`);
    const id = loadIdentity(path);
    expect(id.certs).toEqual([a.cert.trim(), b.cert.trim()]);
    expect(id.key).toBe(a.key.trim());
    const rsa = join(tmp(), 'rsa.pem');
    writeFileSync(rsa, `${a.cert}-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----\n`);
    expect(loadIdentity(rsa).key).toContain('RSA PRIVATE KEY');
  });

  test('garbled certificate data is reported', () => {
    const path = join(tmp(), 'bad.pem');
    writeFileSync(path, '-----BEGIN CERTIFICATE-----\n!!!!\n-----END CERTIFICATE-----\n');
    expect(() => loadIdentity(path)).toThrow('invalid client certificate');
  });
});
