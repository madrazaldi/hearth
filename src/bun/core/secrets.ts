// Server passwords live in the OS keychain, not in config.json. When no
// keychain is available (e.g. Linux without a Secret Service), they fall back
// to the config file, which is written with 0600 permissions.
//
// The service and account names match the Rust engine's keyring entries, so
// secrets saved by earlier versions still load.

import type { ServerConfig } from './config';

export const SERVICE = 'app.hearth.irc';

/** Seam for tests, which must never touch the real keychain. */
export interface Keychain {
  get(service: string, name: string): Promise<string | null>;
  set(service: string, name: string, value: string): Promise<void>;
  delete(service: string, name: string): Promise<boolean>;
}

const bunKeychain: Keychain = {
  get: (service, name) => Bun.secrets.get({ service, name }),
  set: (service, name, value) => Bun.secrets.set({ service, name, value }),
  delete: (service, name) => Bun.secrets.delete({ service, name }),
};

let keychain: Keychain = bunKeychain;

export function useKeychain(k: Keychain | null) {
  keychain = k ?? bunKeychain;
}

const account = (server: string, kind: string) => `${server}:${kind}`;

async function get(server: string, kind: string): Promise<string | null> {
  try {
    return (await keychain.get(SERVICE, account(server, kind))) ?? null;
  } catch (e) {
    console.warn(`keychain read failed: ${e}`);
    return null;
  }
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
