# AGENTS.md

Guidance for coding agents working in this repo. See README.md for features and user-facing docs.

## Stack

Hearth is an IRC client. The backend is TypeScript on Bun (`src/bun/core`), running in the main process of an [Electrobun](https://electrobun.dev) app (`src/bun`). The UI is Svelte 5 runes + TypeScript (`src/`), built with Vite and rendered by Electrobun's bundled Chromium (CEF) on every platform (`renderer: 'cef'` in `electrobun.config.ts`), so don't add WebKit/WebView2-specific workarounds.

## Commands

```sh
bun install                    # also runs `electrobun prepare`, which projects the SDK into .hutch/devkit (git-ignored)
bun run dev                    # UI only, in a browser, against the scripted mock (src/lib/mock.ts)
bun run start                  # build the UI and run the real app (electrobun dev)
bun run check                  # svelte-check + tsc for UI, build scripts and src/bun — must report 0 errors
bun run test                   # Vitest (UI) then bun test (src/bun)
bun run build                  # stable Electrobun build into build/ and artifacts/
bun run probe                  # headless backend smoke test: HEARTH_HOST=… HEARTH_NICK=… bun run probe
bun run screenshots            # regenerate docs/screenshots (needs `bun run dev` running)
```

Run a single test: `bun x vitest run -t "name"` / `bun test src/bun -t "name"`.

CI (`.github/workflows/ci.yml`) runs check and both test suites on Linux/macOS/Windows. Releases come from `.github/workflows/release.yml` on a `v*` tag (see README "Releasing"): it reruns CI, then builds macOS, Linux and Windows apps into a draft release with git-cliff notes (`cliff.toml`). Bump versions with `bun run bump <x.y.z>`, which keeps `electrobun.config.ts` and `package.json` in sync (CI checks this). Use conventional commit prefixes; the release notes are grouped by them. Never commit signing material. The frontend updater lives in `src/lib/updater.svelte.ts` over the `updater-*` commands in `src/bun/index.ts` (Electrobun's hash-based `Updater`); `mock.ts` fakes those commands.

## Architecture

**Core (`src/bun/core`)**
- `app.ts`: `Core`, which owns everything below; `Core.invoke(command, args)` implements every command the UI can call.
- `client.ts`: one actor per connection (a serialized async loop). It handles registration, CAP/SASL (PLAIN, EXTERNAL, SCRAM), STS, reconnect/backoff, labeled-response and chathistory routing (`Route`, pending requests keyed by id), and logging to the store. It stamps a `time` tag on live messages that lack one. Timers go through `clock.ts` so tests can drive them with `ManualClock` (in `harness.ts`).
- `manager.ts`: owns the connections, spawns a child connection per soju network, and caps the send backlog (`MAX_BACKLOG`).
- `events.ts`: an Emitter that batches events into one RPC payload per burst.
- `transport.ts`: TCP/TLS/WebSocket as a line reader and writer (`Bun.connect`, native WebSocket). It also does client certificates.
- `store.ts`: SQLite (`bun:sqlite`, WAL, FTS5). Writes are queued and flushed in batches. Use `Db.memory()` in tests.
- `secrets.ts`: the keychain via `Bun.secrets` (service `app.hearth.irc`). Config secrets are stripped from `config.json` when `inKeychain` is set.
- `preview.ts` + `net.ts`: link previews with SSRF guards. Every hop must pass `checkPublic`.
- `paths.ts`: config/data directories; `HEARTH_CONFIG_DIR`/`HEARTH_DATA_DIR` override them.

**Main process (`src/bun/index.ts`, `src/bun/commands.ts`)**: window (`BrowserWindow`, CEF), tray, close-to-tray, notifications, opener, updater, and the RPC server. `src/shared/rpc.ts` is the RPC schema; `IRC_COMMANDS` in `commands.ts` lists the commands forwarded to `Core.invoke`. Events go back with the `irc` push message.

**Frontend (`src/lib`)**
- `ipc.ts`: `invoke`/`listen` shim. It uses Electrobun RPC inside the app and the mock backend in a browser (`installMock`); `platform.ts` has the platform/shell flags.
**Frontend (`src/lib`)**
- `state.svelte.ts`: the `App`, `Conn` and `Buffer` classes using `$state` runes, a single `app` instance.
- `incoming.ts`: turns each server message into state changes (`handleMessage`, `historyLine`).
- `history.ts`: CHATHISTORY paging with the local-log fallback, `mergeLines` (history dedupe), and gaps: `loadAround` for jumps, `addWindow`/`fillGap` for the unloaded stretch between a jump and the present.
- `search.ts`: search across the local FTS log and `soju.im/search`, deduped and newest first. `parseQuery` handles `from:`/`in:`/date filters; `mergePages` pages across sources. soju requires `in=`, so it falls back to one SEARCH per buffer.
- `saved.ts`: saving and restoring the buffer list, and rejoin commands.
- `monitor.ts`: MONITOR subscriptions and the watch list.
- `unread.ts`: unread and highlight counts, and read markers.
- `commands.ts`: slash commands (`runInput`).
- `api.ts`: typed `invoke` wrappers.
- `types.ts`: types shared with the backend (`camelCase` JSON).
- `components/`: the UI. Keep it small and reuse `Segmented`, `Switch` and `Icon`.

## Conventions and gotchas

- **Adding a command** takes these steps:
  1. Implement it in `src/bun/core/app.ts` and add a case to `Core.invoke`.
  2. Add its name to `IRC_COMMANDS` in `src/bun/commands.ts` (shell-level commands such as the tray or updater are handled in `src/bun/index.ts` instead).
  3. Add a wrapper in `api.ts`, and a case in `mock.ts` so the browser preview keeps working.
- **Settings** are a JSON blob owned by the frontend (`Settings` in `types.ts`, with defaults). The backend reads only the keys it needs when settings are applied.
- **CSP is strict** (set in `vite.config.ts`). The UI must not fetch remote URLs or load remote images. Route network access through the backend, as `preview.ts` does, and return data URLs.
- **Sending from a buffer** uses `conn.sendFrom(buf, line)` when the reply should come back to that buffer. That call adds a labeled-response label.
- **History dedupe:** `Buffer.merge` dedupes by msgid, then fuzzily (same nick and text within 3 s). Keep `time` tags on everything.
- **Secrets never go in logs, config or test fixtures.**
- **Comments are sparse** and explain *why*. Match the surrounding style. There is no prettier config, so follow the existing formatting (2-space TS/Svelte).

## Testing

- **Frontend:** `src/lib/test-utils.ts` provides `mockApi`, `connect(app, {caps, isupport})`, `feed(app, ...rawLines)` and `sent` (outgoing lines). Mock `./api` and `./notify` with `vi.mock` as in `state.test.ts`, and test behaviour by feeding raw IRC lines.
- **Backend:** `src/bun/core/*.test.ts` (bun test) run the actor against a scripted local TCP server (`harness.ts`, `Peer`/`Listener`). Add a test there for protocol changes such as CAP, SASL, batches or labels. Use `ManualClock` for backoff, keepalive and pacing.
- **UI changes:** check them visually with `bun run dev`. The mock query params `?view=search|channels|settings|switcher|shortcuts`, `?tab=<settings tab>&server=<id>`, `?theme=light`, `?layout=columns|compact`, `?update=none` and `?empty` open the matching views; `?noavatars` turns off mock avatars. `?stress` opens `#stress`, a channel with about 10k lines and 5k members, for checking performance. `?stress=live` also posts a line every 3 s.
