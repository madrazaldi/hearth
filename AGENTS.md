# AGENTS.md

Guidance for coding agents working in this repo. See README.md for features and user-facing docs.

## Stack

Hearth is an IRC client written in Rust. The engine (`crates/hearth-core`) runs connections, storage and the keychain on its own tokio runtime. The client state (`crates/hearth-model`) is a synchronous state machine with no UI toolkit and no async runtime. The app (`crates/hearth-app`) renders it with [GPUI Kit](https://github.com/longbridge/gpui-kit) (`gpui-kit` 0.9, pinned exactly: GPUI snapshots change API between releases, so bump it deliberately). `xtask/` holds repo chores.

## Commands

```sh
cargo run                                   # the app with your real config (set HEARTH_CONFIG_DIR/HEARTH_DATA_DIR for throwaway data)
HEARTH_MOCK=1 cargo run                     # the app against the scripted mock (crates/hearth-model/src/mock.rs)
cargo test --workspace                      # all tests; CI runs them with --locked
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all                             # rustfmt.toml: max_width 160
cargo xtask screenshots                     # regenerate docs/screenshots offscreen
cargo xtask bump <x.y.z>                    # the version lives only in the workspace Cargo.toml
HEARTH_HOST=… HEARTH_NICK=… cargo run -p hearth-core --example probe   # headless engine smoke test
```

Run a single test: `cargo test -p hearth-model <name>` (or `-p hearth-core`, `-p hearth-app`).

CI (`.github/workflows/ci.yml`) runs fmt, clippy and the tests on Linux/macOS/Windows, plus `cargo xtask check-version`. Releases come from `.github/workflows/release.yml` on a `v*` tag (see README "Releasing"): it reruns CI, then builds installers with cargo-packager (config in `[package.metadata.packager]` of `crates/hearth-app/Cargo.toml`) into a draft release with git-cliff notes (`cliff.toml`) and a minisign-signed `latest.json` for the updater. Use conventional commit prefixes; the release notes are grouped by them. Never commit signing material.

## Architecture

**Engine (`crates/hearth-core`)**
- `app.rs`: `Core`, a cheap `Clone` handle that owns everything below. Sync commands need no tokio context; async ones return `Pending<T>`, a boxed future already running on the engine's runtime, so any executor can await it. Commands for the same server run in call order.
- `irc/client.rs`: one actor per connection (a serialized async loop). It handles registration, CAP/SASL (PLAIN, EXTERNAL, SCRAM), STS, reconnect/backoff, labeled-response and chathistory routing, and logging to the store. It stamps a `time` tag on live messages that lack one.
- `irc/manager.rs`: owns the connections, spawns a child connection per soju network, and caps the send backlog (`MAX_BACKLOG`).
- `irc/events.rs`: an emitter that batches events into one delivery per burst.
- `irc/transport.rs`: TCP/TLS/WebSocket as a line reader and writer, including client certificates.
- `store.rs`: SQLite (rusqlite, WAL, FTS5). Writes are queued and flushed in batches; reads flush first. Use `Db::memory()` in tests.
- `secrets.rs`: the keychain (service `app.hearth.irc`), behind a `Keychain` trait. Entries match what the old Bun build wrote (`Bun.secrets`), so saved passwords carry over. Tests use `MemoryKeychain`; never touch the real keychain from a test.
- `preview.rs`: link previews and avatars with SSRF guards. Every hop must pass the public-address check.
- `paths.rs`: config/data directories; `HEARTH_CONFIG_DIR`/`HEARTH_DATA_DIR` override them.

**Client state (`crates/hearth-model`)**
- `state.rs`: `App`, with `Conn` (`conn.rs`) and `Buffer` (`buffer.rs`). The host loop is `handle_events` → `take_effects` → execute → `resolve(req, result)`, plus `set_now`/`tick`/`next_wakeup` for timers. `revision()` and the per-buffer `rev()`/`lines_rev()` tell views what changed.
- `effect.rs`: every side effect the model asks for. Request effects carry a `ReqId`.
- `incoming.rs`: turns each server message into state changes.
- `history.rs`: CHATHISTORY paging with the local-log fallback, history dedupe, and gaps for jumps.
- `search.rs`, `search_panel.rs`: search across the local FTS log and `soju.im/search`, deduped and newest first. soju requires `in=`, so it falls back to one SEARCH per buffer.
- `saved.rs`, `monitor.rs`, `unread.rs`: saved buffers and rejoin, MONITOR, unread/highlight counts and read markers.
- `commands.rs`, `aliases.rs`: slash commands (`run_input`) and user aliases.
- `panels.rs`, `welcome.rs`, `composer.rs`, `rows.rs`, `timeline.rs`, `format.rs`: logic behind the views (sidebar, switcher, channel browser, settings forms, first run, completion, message grouping, mIRC formatting). Keep logic here, not in `hearth-app`.
- `types.rs`: `Settings` (with defaults), `Line` and the core types the app uses.
- `mock.rs`: the scripted session for mock mode and screenshots.

**App (`crates/hearth-app`)**
- `host/`: the global `Hearth` entity that owns the model and outlives windows (on Windows and Linux close-to-tray destroys the window). It pumps engine events, executes effects against `Core`, schedules ticks, and drives the mock.
- `ui/`: the views. `ui/data.rs` is the seam (`UiAction`, view structs); `ui/bind.rs` builds view data from the model. Views take plain data and emit `UiAction`s. Keep them small and reuse `ui/widgets.rs` and `ui/controls.rs`.
- `platform/`: menus, tray, close-to-tray, notifications, dock badge, `open_url` (only http, https, mailto, tel), and the updater (cargo-packager-updater; the public key comes from `HEARTH_UPDATER_PUBKEY` at build time).

## Conventions and gotchas

- **Adding a command** takes these steps:
  1. Implement it on `Core` in `crates/hearth-core/src/app.rs` (async work returns `Pending`).
  2. Add an `Effect` variant in `crates/hearth-model/src/effect.rs` (and a `Reply` if it returns data), and queue it from the model.
  3. Execute it in the app's host (`crates/hearth-app/src/host/mod.rs`), and answer it in `mock.rs` so mock mode keeps working.
- **Settings** are a JSON blob owned by the client state (`Settings` in `crates/hearth-model/src/types.rs`, with defaults; unknown keys are dropped, missing ones defaulted). The engine reads only the keys it needs when settings are applied.
- **The UI never touches the network.** Previews, avatars and inline images are fetched by the engine (with its SSRF guards) and handed to the views as data URLs.
- **Sending from a buffer** goes through the model so the reply comes back to that buffer; it adds a labeled-response label.
- **History dedupe:** `Buffer::merge` dedupes by msgid, then fuzzily (same nick and text within 3 s). Keep `time` tags on everything.
- **Secrets never go in logs, config or test fixtures.**
- **Comments are sparse** and explain *why*. Match the surrounding style; `cargo fmt` with the repo's `rustfmt.toml`.
- **GPUI:** don't update an entity or window from inside its own update or key dispatch; defer (`window.defer`, `cx.defer`). Variable-height virtual lists use `list(ListState, …)`.

## Testing

- **Client state:** feed raw IRC lines with `testing.rs` (`connect(app, caps, isupport)`, `feed(app, lines…)`, `sent(app)`), as the suites in `crates/hearth-model/src/tests/` do. Pin time with `set_now`.
- **Engine:** `crates/hearth-core/tests/*.rs` run the actor against a scripted local TCP server (`tests/common`). Add a test there for protocol changes such as CAP, SASL, batches or labels.
- **App:** `crates/hearth-app/tests/host_mock.rs` drives headless GPUI windows against the mock (typing, keys, overlays); `host_real.rs` runs the real engine against a scripted local server in temp directories.
- **UI changes:** look at them with `cargo xtask screenshots` (renders offscreen, no window) or `HEARTH_MOCK=1 cargo run`. Mock environment variables: `HEARTH_VIEW=search|channels|settings|switcher|shortcuts`, `HEARTH_TAB=<settings tab>`, `HEARTH_SERVER=<id>`, `HEARTH_THEME=light`, `HEARTH_LAYOUT=columns|compact`, `HEARTH_UPDATE=available|error`, `HEARTH_EMPTY=1`, `HEARTH_NOAVATARS=1`, `HEARTH_BUFFER=<buffer>`, `HEARTH_QUERY=<search>`, and `HEARTH_STRESS=1|live` (`#stress`, about 10k lines and 5k members, for performance). `HEARTH_BACKGROUND=1` opens the window without taking focus.
