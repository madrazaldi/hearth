// Client certificates for SASL EXTERNAL / CertFP.

import { createHash, generateKeyPairSync, randomBytes, sign, X509Certificate } from 'node:crypto';
import { chmodSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

export interface CertInfo {
  path: string;
  sha256: string;
  sha512: string;
}

// ---- Minimal DER encoding: just enough for a self-signed certificate ----

function der(tag: number, ...parts: Uint8Array[]): Buffer {
  const body = Buffer.concat(parts);
  let len: Buffer;
  if (body.length < 0x80) len = Buffer.from([body.length]);
  else {
    const bytes: number[] = [];
    for (let n = body.length; n > 0; n = Math.floor(n / 256)) bytes.unshift(n & 0xff);
    len = Buffer.from([0x80 | bytes.length, ...bytes]);
  }
  return Buffer.concat([Buffer.from([tag]), len, body]);
}

const seq = (...p: Uint8Array[]) => der(0x30, ...p);
const set = (...p: Uint8Array[]) => der(0x31, ...p);
const oid = (...bytes: number[]) => der(0x06, Buffer.from(bytes));
const utf8 = (s: string) => der(0x0c, Buffer.from(s, 'utf8'));

/** A positive INTEGER from big-endian bytes. */
function integer(bytes: Uint8Array): Buffer {
  let b = Buffer.from(bytes);
  while (b.length > 1 && b[0] === 0 && b[1] < 0x80) b = b.subarray(1);
  if (b[0] >= 0x80) b = Buffer.concat([Buffer.from([0]), b]);
  return der(0x02, b);
}

/** UTCTime up to 2049, GeneralizedTime after, as RFC 5280 requires. */
function time(d: Date): Buffer {
  const p = (n: number, w = 2) => String(n).padStart(w, '0');
  const y = d.getUTCFullYear();
  const rest = `${p(d.getUTCMonth() + 1)}${p(d.getUTCDate())}${p(d.getUTCHours())}${p(d.getUTCMinutes())}${p(d.getUTCSeconds())}Z`;
  return y < 2050 ? der(0x17, Buffer.from(`${p(y % 100)}${rest}`)) : der(0x18, Buffer.from(`${p(y, 4)}${rest}`));
}

const ECDSA_SHA256 = seq(oid(0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02));
const subject = (cn: string) => seq(set(seq(oid(0x55, 0x04, 0x03), utf8(cn))));

const pem = (label: string, data: Uint8Array) =>
  `-----BEGIN ${label}-----\n${Buffer.from(data)
    .toString('base64')
    .replace(/(.{64})/g, '$1\n')
    .replace(/\n$/, '')}\n-----END ${label}-----\n`;

/** A self-signed ECDSA P-256 certificate and its PKCS#8 key, both as PEM. `dnsName` is for tests that need a name to verify. */
export function selfSigned(dnsName = 'hearth'): { cert: string; key: string } {
  const { publicKey, privateKey } = generateKeyPairSync('ec', { namedCurve: 'P-256' });
  const serial = randomBytes(16);
  serial[0] &= 0x7f;
  if (serial[0] === 0) serial[0] = 1;
  // Backdated a day against clock skew; valid for about a century.
  const from = new Date(Date.now() - 24 * 3600 * 1000);
  const to = new Date(from.getTime() + 100 * 365 * 24 * 3600 * 1000);
  const san = seq(oid(0x55, 0x1d, 0x11), der(0x04, seq(der(0x82, Buffer.from(dnsName)))));
  const tbs = seq(
    der(0xa0, der(0x02, Buffer.from([2]))),
    integer(serial),
    ECDSA_SHA256,
    subject(dnsName),
    seq(time(from), time(to)),
    subject(dnsName),
    publicKey.export({ type: 'spki', format: 'der' }),
    der(0xa3, seq(san)),
  );
  const signature = sign('sha256', tbs, privateKey);
  const cert = seq(tbs, ECDSA_SHA256, der(0x03, Buffer.from([0]), signature));
  return { cert: pem('CERTIFICATE', cert), key: privateKey.export({ type: 'pkcs8', format: 'pem' }).toString() };
}

/** Create a self-signed certificate and key in `dir/<name>.pem` (mode 0600). */
export function generate(dir: string, name: string): CertInfo {
  const safe = name.replace(/[^A-Za-z0-9_-]/g, '');
  if (!safe) throw new Error('invalid certificate name');
  mkdirSync(dir, { recursive: true });
  const { cert, key } = selfSigned();
  // Check our own encoding before it's saved and presented to a server.
  new X509Certificate(cert);
  const path = join(dir, `${safe}.pem`);
  writeFileSync(path, cert + key, { mode: 0o600 });
  try {
    chmodSync(path, 0o600);
  } catch {}
  return info(path);
}

/** Fingerprints of the first certificate in a PEM file, as NickServ shows them. */
export function info(path: string): CertInfo {
  const { certs } = loadIdentity(path);
  const der = Buffer.from(certs[0].replace(/-----[^-]+-----|\s/g, ''), 'base64');
  const hex = (alg: string) => createHash(alg).update(der).digest('hex');
  return { path, sha256: hex('sha256'), sha512: hex('sha512') };
}

export interface Identity {
  /** PEM blocks, leaf first. */
  certs: string[];
  /** The private key as PEM. */
  key: string;
}

const KEY_LABELS = ['PRIVATE KEY', 'RSA PRIVATE KEY', 'EC PRIVATE KEY'];

/** Read a certificate chain and private key from one PEM file. */
export function loadIdentity(path: string): Identity {
  let text: string;
  try {
    text = readFileSync(path, 'utf8');
  } catch (e) {
    throw new Error(`could not read client certificate ${path}: ${e instanceof Error ? e.message : e}`);
  }
  const certs: string[] = [];
  let key: string | null = null;
  for (const m of text.matchAll(/-----BEGIN ([A-Z0-9 ]+)-----([\s\S]*?)-----END \1-----/g)) {
    const body = m[2].replace(/\s/g, '');
    if (!/^[A-Za-z0-9+/]*={0,2}$/.test(body)) {
      if (m[1] === 'CERTIFICATE') throw new Error(`invalid client certificate: bad base64 in ${path}`);
      continue;
    }
    if (m[1] === 'CERTIFICATE') certs.push(m[0]);
    else if (key === null && KEY_LABELS.includes(m[1])) key = m[0];
  }
  if (certs.length === 0) throw new Error(`${path} contains no certificate`);
  if (key === null) throw new Error('client certificate file has no usable private key');
  return { certs, key };
}
