// Keeps the app version in sync across electrobun.config.ts and package.json.
// `bun scripts/version.mjs` checks that they agree (CI runs this);
// `bun run bump 0.2.0` sets both.

import { readFileSync, writeFileSync } from 'node:fs';

const root = new URL('../', import.meta.url);
const files = {
  electrobun: new URL('electrobun.config.ts', root),
  npm: new URL('package.json', root),
};
// The only `version: '…'` in electrobun.config.ts is app.version.
const configVersion = /(\bversion:\s*)'([^']*)'/;

function read() {
  return {
    'electrobun.config.ts': readFileSync(files.electrobun, 'utf8').match(configVersion)?.[2],
    'package.json': JSON.parse(readFileSync(files.npm, 'utf8')).version,
  };
}

const next = process.argv[2]?.replace(/^v/, '');
if (!next) {
  const versions = read();
  const distinct = new Set(Object.values(versions));
  if (distinct.size !== 1 || distinct.has(undefined)) {
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
writeFileSync(files.electrobun, readFileSync(files.electrobun, 'utf8').replace(configVersion, `$1'${next}'`));
writeFileSync(files.npm, readFileSync(files.npm, 'utf8').replace(/("version":\s*)"[^"]*"/, `$1"${next}"`));

console.log(`Version set to ${next}. Commit, then: git tag v${next} && git push origin v${next}`);
