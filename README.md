# Hearth

A modern IRC client built with Rust, Tauri 2 and Svelte 5.

## Features

- **Transports:** plain TCP, TLS (rustls) and IRC-over-WebSocket (`text.ircv3.net`). Self-signed certificates can be allowed per server.
- **IRCv3:** CAP 302 with `cap-notify`, SASL PLAIN, SCRAM-SHA-256 (falls back to PLAIN) and EXTERNAL, STS (upgrades to TLS and remembers the policy), `labeled-response`, `server-time`, `message-tags`, `batch`, `echo-message`, `away-notify`, `account-notify`, `extended-join`, `chghost`, `setname`, `multi-prefix`, `userhost-in-names`, `invite-notify`, `draft/chathistory`, `draft/event-playback`, `draft/read-marker`, `draft/message-redaction` (receive and delete your own messages), `+typing`, `+draft/reply`, `+draft/react` and `MONITOR` / `extended-monitor`.
- **soju bouncer support:** one server entry expands into every bouncer network (`soju.im/bouncer-networks`), and networks added or removed on the bouncer show up live. Networks can be added, edited and removed from the server's settings page. Message search uses `soju.im/search` when available. File uploads go through `soju.im/FILEHOST`, by drag and drop, paste or the paperclip button.
- **History:** backlog loads when you join a channel and older pages load as you scroll up. Private conversations from the last 14 days are rediscovered with `CHATHISTORY TARGETS`.
- **Saved scrollback:** messages are stored locally in SQLite (`hearth.db` in the app-data directory), so scrollback survives restarts and works on servers without chathistory. It can be turned off and wiped under Settings → Privacy & data.
- **Search:** ⌘⇧F searches every buffer, one network or the current buffer, using the local full-text index plus bouncer search. Choosing a result jumps to the message.
- **Replies and reactions:** hover a message to reply, react, copy it or delete your own.
- **Link previews** (opt-in): pages and images are fetched by the Rust backend, which refuses local and private addresses at every redirect.
- **Channel browser** for `/list`, with filtering and sorting.
- **Saved buffers:** open channels and private conversations are remembered per network, including channel keys, and restored on launch.
- **Ignore list** with nick or `nick!user@host` wildcard masks. **Per-buffer notifications:** all messages, mentions only, or muted.
- **Watch list:** MONITOR shows online presence for private conversations and announces watched nicks.
- **Tray:** optionally keep running in the menu bar or system tray when the window closes, and mark yourself away after a period in the background.
- **Unread state:** read markers sync between devices, the sidebar shows unread counts and mention badges, a "New" divider marks where you left off, and the dock badge counts mentions.
- **Desktop notifications** for mentions, highlight words and private messages.
- **Composer:**
  - Tab completion for nicks (most recent speakers first), channels and commands.
  - Command suggestions as you type.
  - Input history.
  - Multi-line sends.
  - Long messages split on word boundaries.
- **Rendering:**
  - mIRC formatting, including the extended 99-colour palette and hex colours.
  - Clickable links and `#channels`.
  - Stable per-nick colours computed in OKLCH.
  - Collapsed join/part/quit runs.
  - WHOIS results shown as cards.
  - Collapsible MOTD.
- **Layouts:** "columns", with nicks aligned in a gutter, or "compact", with nicks above grouped messages. Light, dark and system themes. Adjustable text size.
- **Reliability:**
  - Automatic reconnect with exponential backoff and keepalive pings.
  - Channels you were in are rejoined after reconnecting.
  - Outgoing lines are rate-limited on direct connections, and the send queue is capped.
  - CTCP replies are rate-limited.
- **Security:**
  - Passwords are kept in the OS keychain, and existing plain-text passwords are migrated out of the config file.
  - Client certificates (CertFP) can be generated in the app.
  - The webview runs under a strict Content Security Policy.

## Commands

Type `/help` in the app for the full list. The main ones are `/join`, `/part`, `/msg`, `/query`, `/me`, `/notice`, `/nick`, `/topic`, `/whois`, `/away`, `/back`, `/invite`, `/kick`, `/ban`, `/unban`, `/op`, `/deop`, `/voice`, `/devoice`, `/mode`, `/names`, `/ctcp`, `/raw`, `/clear`, `/close`, `/reconnect`, `/quit` and `/list` (opens the channel browser). Also: `/search`, `/ignore` and `/unignore` (no argument lists the ignores), `/watch` and `/unwatch` (alias `/monitor`), and `/notify all|mentions|none` (with `/mute` and `/unmute` as shortcuts). Anything else is sent to the server verbatim.

## Shortcuts

| Keys | Action |
| --- | --- |
| ⌘K | Jump to any buffer (type a `#channel` to join it) |
| ⌥↑ / ⌥↓ | Previous / next buffer (add ⇧ for unread only) |
| ⌘1–9 | Go to the Nth channel |
| ⌘F | Filter messages in the current buffer |
| ⌘⇧F | Search messages |
| ⌘W | Close or leave the buffer |
| ⌘⇧M | Toggle the member list |
| ⌘, | Open Settings |

## Development

```sh
pnpm install
pnpm tauri dev      # run the app
pnpm check          # svelte-check + tsc
pnpm test           # Vitest: state and message handling
cd src-tauri && cargo test && cargo clippy   # unit tests + actor tests against a scripted server
pnpm tauri build    # release bundle
```

When `pnpm dev` runs in a plain browser instead of inside Tauri, Hearth replays a scripted IRC session from `src/lib/mock.ts`. This lets you work on the UI without a server. Query parameters control the preview: `?theme=light`, `?layout=compact`, `?view=settings`, `?view=switcher`, `?view=search`, `?view=channels`, `?tab=privacy` (open a Settings tab; add `&server=<id>` to edit a server) and `?empty` (first-run screen).

CI (`.github/workflows/ci.yml`) runs the checks and tests on Linux, macOS and Windows.

To smoke-test the backend headlessly:

```sh
cd src-tauri
HEARTH_HOST=irc.libera.chat HEARTH_NICK=someone cargo run --example probe
```

## Architecture

### Backend (`src-tauri/src`)

- `irc/message.rs` parses and serializes IRC messages, including tags.
- `irc/transport.rs` reduces TCP, TLS and WebSocket to a line reader and a line writer.
- `irc/client.rs` runs one actor per connection. It handles registration, CAP and SASL, STS, keepalive, reconnects, labeled-response routing, chathistory batches and message logging.
- `irc/scram.rs` implements SCRAM-SHA-256.
- `irc/manager.rs` owns all connections and spawns a child connection for each bouncer network.
- `irc/events.rs` batches events into a single payload per burst before sending them to the UI.
- `store.rs` is the SQLite log. It holds messages with an FTS5 index, saved buffers and STS policies, and a writer thread batches inserts.
- `secrets.rs` stores and loads keychain entries. `certs.rs` generates client certificates and computes their fingerprints.
- `preview.rs` fetches link previews, with SSRF guards and a cache.
- `tests/actor.rs` drives the connection actor against a scripted local server.

### Frontend (`src/lib`)

- `state.svelte.ts` holds all client state (connections, buffers, members) using Svelte 5 runes.
- `incoming.ts` turns server messages into state changes.
- `history.ts` loads chathistory, falls back to the local log and dedupes merged lines.
- `search.ts` searches the local log and the bouncer.
- `saved.ts` saves and restores open buffers.
- `monitor.ts` tracks watched nicks with MONITOR.
- `unread.ts` counts unread messages and highlights and syncs read markers.
- `ignore.ts` matches ignore masks.
- `*.test.ts` are Vitest suites. `test-utils.ts` feeds raw IRC lines into a mocked backend.
- `commands.ts` implements slash commands.
- `format.ts` parses mIRC formatting and finds links.
- `components/` holds the UI.

Config is stored as JSON in the OS app-config directory with `0600` permissions. On macOS that's `~/Library/Application Support/app.hearth.irc/config.json`. Passwords are stored in the OS keychain (service `app.hearth.irc`). If the keychain is unavailable, they stay in the config file instead.
