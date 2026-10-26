import type { ElectrobunConfig } from 'electrobun';

// Signing is opt-in: the release workflow exports these when its secrets are set,
// so forks and local builds still produce (unsigned) apps.
const env = process.env;
const sign = Boolean(env.ELECTROBUN_DEVELOPER_ID);
const notarize =
  sign &&
  Boolean((env.ELECTROBUN_APPLEID && env.ELECTROBUN_APPLEIDPASS && env.ELECTROBUN_TEAMID) || (env.ELECTROBUN_APPLEAPIKEY && env.ELECTROBUN_APPLEAPIISSUER && env.ELECTROBUN_APPLEAPIKEYPATH));

// `app.version` is kept in step with package.json by `bun run bump`.
export default {
  app: {
    name: 'Hearth',
    identifier: 'app.hearth.irc',
    version: '0.1.0',
  },
  build: {
    mainProcess: 'bun',
    bun: { entrypoint: 'src/bun/index.ts' },
    // Vite builds the UI to dist/; the app serves it from views://mainview.
    copy: {
      'dist/index.html': 'views/mainview/index.html',
      'dist/assets': 'views/mainview/assets',
      'assets/logo/tray-template@2x.png': 'views/tray/tray-template.png',
      'assets/icons/icon.png': 'views/tray/icon.png',
    },
    watchIgnore: ['dist/**'],
    // Every platform ships the same bundled Chromium (CEF) instead of the
    // system webview, so the UI renders identically everywhere.
    mac: { bundleCEF: true, defaultRenderer: 'cef', icons: 'assets/icon.iconset', codesign: sign, notarize },
    linux: { bundleCEF: true, defaultRenderer: 'cef', icon: 'assets/icons/icon.png' },
    win: { bundleCEF: true, defaultRenderer: 'cef', icon: 'assets/icons/icon.ico' },
  },
  runtime: {
    // Closing the window quits, unless "close to tray" hides it first.
    exitOnLastWindowClosed: true,
  },
  release: {
    // Where the updater looks for `<channel>-<os>-<arch>-update.json` and the
    // artifacts next to it: the latest GitHub release.
    baseUrl: 'https://github.com/madrazaldi/hearth/releases/latest/download',
    generatePatch: false,
  },
} satisfies ElectrobunConfig;
