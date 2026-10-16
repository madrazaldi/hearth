// Keeps the app version in sync across tauri.conf.json, the root Cargo.toml and
// package.json. `bun scripts/version.mjs` checks that they agree (CI runs this);
// `bun run bump 0.2.0` sets all three and refreshes Cargo.lock.

import { readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

const root = new URL('../', import.meta.url);
const files = {
  tauri: new URL('src-tauri/tauri.conf.json', root),
  cargo: new URL('Cargo.toml', root),
  npm: new URL('package.json', root),
};
// The first `version = "…"` in the root Cargo.toml is [workspace.package].
const cargoVersion = /^version = "([^"]+)"/m;

function read() {
  return {
    'src-tauri/tauri.conf.json': JSON.parse(readFileSync(files.tauri, 'utf8')).version,
    'Cargo.toml': readFileSync(files.cargo, 'utf8').match(cargoVersion)?.[1],
    'package.json': JSON.parse(readFileSync(files.npm, 'utf8')).version,
  };
}

const next = process.argv[2]?.replace(/^v/, '');
if (!next) {
  const versions = read();
  const distinct = new Set(Object.values(versions));
  if (distinct.size !== 1) {
    for (const [file, v] of Object.entries(versions)) console.error(`${file}: ${v}`);
    console.error('Versions disagree. Run `bun run bump <version>` to sync them.');
    process.exit(1);
  }
  console.log([...distinct][0]);
  process.exit(0);
}

if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(next)) {
  console.error(`Not a semver version: ${next}`);
  process.exit(1);
}

// Text replacement rather than JSON.stringify, so the files keep their formatting.
const setJson = (url) => writeFileSync(url, readFileSync(url, 'utf8').replace(/("version":\s*)"[^"]*"/, `$1"${next}"`));
setJson(files.tauri);
setJson(files.npm);
writeFileSync(files.cargo, readFileSync(files.cargo, 'utf8').replace(cargoVersion, `version = "${next}"`));
execFileSync('cargo', ['update', '--workspace'], { cwd: root, stdio: 'inherit' });

console.log(`Version set to ${next}. Commit, then: git tag v${next} && git push origin v${next}`);
