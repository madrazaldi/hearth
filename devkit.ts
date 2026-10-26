// Electrobun's SDK is projected into .hutch/devkit by `electrobun prepare`
// (it is never installed from npm). Vite and Vitest resolve `electrobun/*`
// imports against that projection, so make sure it exists first.

import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('.', import.meta.url));
const devkit = resolve(root, '.hutch/devkit');

export async function electrobunAliases() {
  if (!existsSync(resolve(devkit, 'package.json'))) {
    execFileSync('bunx', ['electrobun', 'prepare'], { cwd: root, stdio: 'inherit' });
  }
  const { electrobunViteAliases } = await import('./.hutch/devkit/api/config/electrobun-vite.ts');
  return electrobunViteAliases(devkit);
}
