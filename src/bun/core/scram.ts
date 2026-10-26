// SASL SCRAM-SHA-256 (RFC 5802 / RFC 7677), client side, without channel binding.

import { createHash, createHmac, pbkdf2Sync, randomBytes, timingSafeEqual } from 'node:crypto';

const hmac = (key: Uint8Array, data: string | Uint8Array) => createHmac('sha256', key).update(data).digest();
const b64 = (b: Uint8Array) => Buffer.from(b).toString('base64');

export class Scram {
  private clientFirstBare: string;
  /** Expected server signature, known after the client-final message. */
  private serverSignature: Buffer | null = null;

  constructor(
    user: string,
    private password: string,
    private cnonce: string = b64(randomBytes(18)),
  ) {
    const name = user.replaceAll('=', '=3D').replaceAll(',', '=2C');
    this.clientFirstBare = `n=${name},r=${cnonce}`;
  }

  clientFirst(): string {
    return `n,,${this.clientFirstBare}`;
  }

  /** Answer the server-first message with the client-final message. Throws on a malformed or hostile challenge. */
  clientFinal(serverFirst: string): string {
    const attr = (k: string): string => {
      for (const kv of serverFirst.split(',')) if (kv.startsWith(k + '=')) return kv.slice(k.length + 1);
      throw new Error(`server-first message is missing '${k}'`);
    };
    const nonce = attr('r');
    if (!nonce.startsWith(this.cnonce) || nonce.length <= this.cnonce.length) throw new Error('server nonce does not extend the client nonce');
    const saltText = attr('s');
    if (!/^[A-Za-z0-9+/]*={0,2}$/.test(saltText)) throw new Error('invalid salt');
    const salt = Buffer.from(saltText, 'base64');
    const itText = attr('i');
    const iterations = /^\d+$/.test(itText) ? Number(itText) : NaN;
    if (!Number.isFinite(iterations)) throw new Error('invalid iteration count');
    if (iterations === 0 || iterations > 1_000_000) throw new Error('unreasonable iteration count');

    const salted = pbkdf2Sync(this.password, salt, iterations, 32, 'sha256');
    const clientKey = hmac(salted, 'Client Key');
    const storedKey = createHash('sha256').update(clientKey).digest();
    const withoutProof = `c=biws,r=${nonce}`;
    const authMessage = `${this.clientFirstBare},${serverFirst},${withoutProof}`;
    const signature = hmac(storedKey, authMessage);
    const proof = clientKey.map((b, i) => b ^ signature[i]);
    this.serverSignature = hmac(hmac(salted, 'Server Key'), authMessage);
    return `${withoutProof},p=${b64(proof)}`;
  }

  /** Check the server-final message proves the server knows the password. Throws otherwise. */
  verify(serverFinal: string) {
    if (serverFinal.startsWith('e=')) throw new Error(`server rejected authentication: ${serverFinal.slice(2)}`);
    if (!serverFinal.startsWith('v=')) throw new Error('malformed server-final message');
    const text = serverFinal.slice(2).split(',')[0];
    if (!/^[A-Za-z0-9+/]*={0,2}$/.test(text)) throw new Error('malformed server signature');
    const got = Buffer.from(text, 'base64');
    if (!this.serverSignature || got.length !== this.serverSignature.length || !timingSafeEqual(got, this.serverSignature)) {
      throw new Error('server signature mismatch');
    }
  }
}
