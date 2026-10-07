# AGENTS.md

Guidance for coding agents working in this repo. See README.md for features and user-facing docs.

## Stack

Hearth is an IRC client with a Rust engine (`crates/hearth-core`) and one frontend:
Tauri 2 (`src-tauri/`) + Svelte 5 runes + TypeScript (`src/`), built with Vite and pnpm.

The root `Cargo.toml` is a workspace of the Rust crates.

## Commands

```sh
pnpm install
pnpm tauri dev                 # full app
pnpm dev                       # UI only, in a browser, against the scripted mock (src/lib/mock.ts)
pnpm check                     # svelte-check + tsc — must report 0 errors
pnpm test                      # Vitest
cargo test --workspace         # unit tests + crates/hearth-core/tests
cargo clippy --workspace --all-targets -- -D warnings   # CI runs with -D warnings
```

Run a single test: `pnpm vitest run -t "name"` / `cargo test name`. Headless backend smoke test: `HEARTH_HOST=… HEARTH_NICK=… cargo run -p hearth-core --example probe`.

CI (`.github/workflows/ci.yml`) runs all of the above on Linux/macOS/Windows. Releases come from `.github/workflows/release.yml` on a `v*` tag (see README "Releasing"). Never commit the updater private key (`~/.tauri/hearth-updater.key`). Releases ship Tauri bundles on Windows/Linux. The frontend updater lives in `src/lib/updater.svelte.ts`, and `mock.ts` fakes its `plugin:updater|*` calls. MSRV is `rust-version` in the root `Cargo.toml` (1.90); clippy lints depend on it.

## Architecture

**Core (`crates/hearth-core/src`)**
- `app.rs`: `Core`, which owns everything below and implements every command.
- `runtime.rs`: the global tokio runtime. Use `runtime::spawn`, not `tauri::async_runtime`; the core must not depend on Tauri.
- `irc/client.rs`: one tokio actor per connection. It handles registration, CAP/SASL (PLAIN, EXTERNAL, SCRAM), STS, reconnect/backoff, labeled-response and chathistory routing (`Route`, pending requests keyed by id), and logging to the store. It stamps a `time` tag on live messages that lack one.
- `irc/manager.rs`: owns the connections, spawns a child connection per soju network, and caps the send backlog (`MAX_BACKLOG`).
- `irc/events.rs`: an Emitter that batches events into one IPC payload per burst.
- `irc/transport.rs`: TCP/TLS/WebSocket as a line reader and writer. It also does client certificates.
- `store.rs`: SQLite (rusqlite bundled, WAL, FTS5). All writes go through a writer thread (`Write` enum). Use `Db::memory()` in tests.
- `secrets.rs`: the keychain. Config secrets are stripped from `config.json` when `in_keychain` is set.
- `preview.rs`: link previews with SSRF guards. Every hop must pass `check_public`.

**Tauri (`src-tauri/src/lib.rs`)**: thin `#[tauri::command]` wrappers over `State<Core>`, tray, close-to-tray and the updater.

**Frontend (`src/lib`)**
- `state.svelte.ts`: the `App`, `Conn` and `Buffer` classes using `$state` runes, a single `app` instance.
- `incoming.ts`: turns each server message into state changes (`handleMessage`, `historyLine`).
- `history.ts`: CHATHISTORY paging with the local-log fallback, and `mergeLines` (history dedupe).
- `search.ts`: search across the local FTS log and `soju.im/search`, deduped and newest first.
- `saved.ts`: saving and restoring the buffer list, and rejoin commands.
- `monitor.ts`: MONITOR subscriptions and the watch list.
- `unread.ts`: unread and highlight counts, and read markers.
- `commands.ts`: slash commands (`runInput`).
- `api.ts`: typed `invoke` wrappers.
- `types.ts`: types shared with Rust (serde `camelCase`).
- `components/`: the UI. Keep it small and reuse `Segmented`, `Switch` and `Icon`.

## Conventions and gotchas

- **Adding a command** takes these steps:
  1. Implement it as a `Core` method in `crates/hearth-core/src/app.rs`.
  2. Add a `#[tauri::command]` wrapper in `src-tauri/src/lib.rs` and register it in `generate_handler!`.
  3. Add a wrapper in `api.ts`, and a case in `mock.ts` so the browser preview keeps working.
- **Settings** are a JSON blob owned by the frontend (`Settings` in `types.ts`, with defaults). Rust reads only the keys it needs in `apply_settings`.
- **CSP is strict** (`tauri.conf.json`). The webview must not fetch remote URLs or load remote images. Route network access through Rust, as `preview.rs` does, and return data URLs.
- **Sending from a buffer** uses `conn.sendFrom(buf, line)` when the reply should come back to that buffer. That call adds a labeled-response label.
- **History dedupe:** `Buffer.merge` dedupes by msgid, then fuzzily (same nick and text within 3 s). Keep `time` tags on everything.
- **Secrets never go in logs, config or test fixtures.**
- **Comments are sparse** and explain *why*. Match the surrounding style. There is no prettier config, so follow the existing formatting (2-space TS/Svelte, rustfmt).

## Testing

- **Frontend:** `src/lib/test-utils.ts` provides `mockApi`, `connect(app, {caps, isupport})`, `feed(app, ...rawLines)` and `sent` (outgoing lines). Mock `./api` and `./notify` with `vi.mock` as in `state.test.ts`, and test behaviour by feeding raw IRC lines.
- **Backend:** `crates/hearth-core/tests/` runs the actor against a scripted local TCP server. Add a test there for protocol changes such as CAP, SASL, batches or labels.
- **UI changes:** check them visually with `pnpm dev`. The mock query params `?view=search|channels|settings|switcher|shortcuts`, `?tab=<settings tab>&server=<id>`, `?theme=light`, `?layout=compact`, `?update=none` and `?empty` open the matching views; `?noavatars` turns off mock avatars. `?stress` opens `#stress`, a channel with about 10k lines and 5k members, for checking performance. `?stress=live` also posts a line every 3 s.
