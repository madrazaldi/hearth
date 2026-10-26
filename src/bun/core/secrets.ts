// Server passwords live in the OS keychain, not in config.json. When no
// keychain is available (e.g. Linux without a Secret Service), they fall back
// to the config file, which is written with 0600 permissions.
//
// The service and account names match the Rust engine's keyring entries. On
// macOS Bun reads those entries as they are; on Windows and Linux the Rust
// `keyring` crate stored them differently, so they are looked up the old way
// once and copied into Bun's format.

import type { Pointer } from 'bun:ffi';
import type { ServerConfig } from './config';

export const SERVICE = 'app.hearth.irc';

/** Seam for tests, which must never touch the real keychain. */
export interface Keychain {
  get(service: string, name: string): Promise<string | null>;
  set(service: string, name: string, value: string): Promise<void>;
  delete(service: string, name: string): Promise<boolean>;
  /** An entry written by the Rust engine, where its format differs from Bun's. */
  legacy?(service: string, name: string): Promise<string | null>;
}

const bunKeychain: Keychain = {
  get: (service, name) => Bun.secrets.get({ service, name }),
  set: (service, name, value) => Bun.secrets.set({ service, name, value }),
  delete: (service, name) => Bun.secrets.delete({ service, name }),
  legacy: async (service, name) => {
    if (process.platform === 'win32') return windowsLegacy(service, name);
    if (process.platform === 'linux') return linuxLegacy(service, name);
    return null;
  },
};

/** keyring-rs: a generic credential named `<account>.<service>` holding UTF-16. */
async function windowsLegacy(service: string, name: string): Promise<string | null> {
  const { dlopen, ptr, read, toArrayBuffer } = await import('bun:ffi');
  const lib = dlopen('advapi32.dll', {
    CredReadW: { args: ['ptr', 'u32', 'u32', 'ptr'], returns: 'i32' },
    CredFree: { args: ['ptr'], returns: 'void' },
  });
  try {
    const target = Buffer.from(`${name}.${service}\0`, 'utf16le');
    const out = new BigUint64Array(1);
    const CRED_TYPE_GENERIC = 1;
    if (!lib.symbols.CredReadW(ptr(target), CRED_TYPE_GENERIC, 0, ptr(out))) return null;
    const cred = Number(out[0]) as Pointer;
    try {
      // CREDENTIALW on 64-bit: CredentialBlobSize at 32, CredentialBlob at 40.
      const size = read.u32(cred, 32);
      const blob = Number(read.ptr(cred, 40)) as Pointer;
      if (!size || !blob) return '';
      return Buffer.from(toArrayBuffer(blob, 0, size).slice(0)).toString('utf16le');
    } finally {
      lib.symbols.CredFree(cred);
    }
  } finally {
    lib.close();
  }
}

/** keyring-rs: a Secret Service item keyed by `service` and `username`. Needs secret-tool. */
async function linuxLegacy(service: string, name: string): Promise<string | null> {
  if (!Bun.which('secret-tool')) return null;
  const p = Bun.spawn(['secret-tool', 'lookup', 'service', service, 'username', name], { stdout: 'pipe', stderr: 'ignore' });
  const out = await new Response(p.stdout).text();
  return (await p.exited) === 0 && out !== '' ? out : null;
}

let keychain: Keychain = bunKeychain;

export function useKeychain(k: Keychain | null) {
  keychain = k ?? bunKeychain;
}

const account = (server: string, kind: string) => `${server}:${kind}`;

async function get(server: string, kind: string): Promise<string | null> {
  const name = account(server, kind);
  let value: string | null;
  try {
    value = (await keychain.get(SERVICE, name)) ?? null;
  } catch (e) {
    console.warn(`keychain read failed: ${e}`);
    return null;
  }
  if (value !== null || !keychain.legacy) return value;
  try {
    value = await keychain.legacy(SERVICE, name);
  } catch (e) {
    console.warn(`legacy keychain read failed: ${e}`);
    return null;
  }
  if (value) {
    try {
      await keychain.set(SERVICE, name, value);
    } catch (e) {
      console.warn(`could not migrate a keychain entry: ${e}`);
    }
  }
  return value;
}

/** Store (or with an empty value, remove) one secret. Returns false when the keychain is unavailable. */
async function put(server: string, kind: string, value: string): Promise<boolean> {
  try {
    if (value === '') await keychain.delete(SERVICE, account(server, kind));
    else await keychain.set(SERVICE, account(server, kind), value);
    return true;
  } catch (e) {
    console.warn(`keychain write failed, keeping secrets in the config file: ${e}`);
    return false;
  }
}

/** Move a server's secrets into the keychain. Sets `inKeychain` on success. */
export async function store(server: ServerConfig) {
  server.inKeychain = (await put(server.id, 'password', server.password)) && (await put(server.id, 'sasl', server.saslPassword));
}

/**
 * Fill in secrets for a server loaded from disk. Returns true when secrets
 * were found in the file and migrated, so the caller should re-save.
 */
export async function load(server: ServerConfig): Promise<boolean> {
  if (server.password || server.saslPassword) {
    await store(server);
    return !!server.inKeychain;
  }
  const password = await get(server.id, 'password');
  const sasl = await get(server.id, 'sasl');
  if (password !== null || sasl !== null) {
    server.password = password ?? '';
    server.saslPassword = sasl ?? '';
    server.inKeychain = true;
  }
  return false;
}

export async function forget(server: string) {
  await put(server, 'password', '');
  await put(server, 'sasl', '');
}
