# Hearth

A modern IRC client built with Rust, Tauri 2 and Svelte 5.

## Features

- **Transports:** plain TCP, TLS (rustls) and IRC-over-WebSocket (`text.ircv3.net`). Self-signed certificates can be allowed per server.
- **IRCv3:** CAP 302 with `cap-notify`, SASL PLAIN, `server-time`, `message-tags`, `batch`, `echo-message`, `away-notify`, `account-notify`, `extended-join`, `chghost`, `setname`, `multi-prefix`, `userhost-in-names`, `invite-notify`, `draft/chathistory`, `draft/event-playback`, `draft/read-marker`, `draft/message-redaction` and `+typing`.
- **soju bouncer support:** one server entry expands into every bouncer network (`soju.im/bouncer-networks`), and networks added or removed on the bouncer show up live. File uploads go through `soju.im/FILEHOST`, by drag and drop, paste or the paperclip button.
- **History:** backlog loads when you join a channel and older pages load as you scroll up. Private conversations from the last 14 days are rediscovered with `CHATHISTORY TARGETS`.
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
  - Outgoing lines are rate-limited on direct connections.

## Commands

Type `/help` in the app for the full list. The main ones are `/join`, `/part`, `/msg`, `/query`, `/me`, `/notice`, `/nick`, `/topic`, `/whois`, `/away`, `/back`, `/invite`, `/kick`, `/ban`, `/unban`, `/op`, `/deop`, `/voice`, `/devoice`, `/mode`, `/names`, `/ctcp`, `/raw`, `/clear`, `/close`, `/reconnect`, `/quit` and `/list`. Anything else is sent to the server verbatim.

## Shortcuts

| Keys | Action |
| --- | --- |
| ⌘K | Jump to any buffer (type a `#channel` to join it) |
| ⌥↑ / ⌥↓ | Previous / next buffer (add ⇧ for unread only) |
| ⌘1–9 | Go to the Nth channel |
| ⌘F | Filter messages in the current buffer |
| ⌘W | Close or leave the buffer |
| ⌘⇧M | Toggle the member list |
| ⌘, | Open Settings |

## Development

```sh
pnpm install
pnpm tauri dev      # run the app
pnpm check          # svelte-check + tsc
cd src-tauri && cargo test && cargo clippy
pnpm tauri build    # release bundle
```

When `pnpm dev` runs in a plain browser instead of inside Tauri, Hearth replays a scripted IRC session from `src/lib/mock.ts`. This lets you work on the UI without a server. Query parameters control the preview: `?theme=light`, `?layout=compact`, `?view=settings`, `?view=switcher` and `?empty` (first-run screen).

To smoke-test the backend headlessly:

```sh
cd src-tauri
HEARTH_HOST=irc.libera.chat HEARTH_NICK=someone cargo run --example probe
```

## Architecture

### Backend (`src-tauri/src`)

- `irc/message.rs` parses and serializes IRC messages, including tags.
- `irc/transport.rs` reduces TCP, TLS and WebSocket to a line reader and a line writer.
- `irc/client.rs` runs one actor per connection. It handles registration, CAP and SASL, keepalive, reconnects and chathistory batches.
- `irc/manager.rs` owns all connections and spawns a child connection for each bouncer network.
- `irc/events.rs` batches events into a single payload per burst before sending them to the UI.

### Frontend (`src/lib`)

- `state.svelte.ts` holds all client state (connections, buffers, members, unread counts, history) using Svelte 5 runes.
- `commands.ts` implements slash commands.
- `format.ts` parses mIRC formatting and finds links.
- `components/` holds the UI.

Config, including server credentials, is stored as JSON in the OS app-config directory with `0600` permissions. On macOS that's `~/Library/Application Support/app.hearth.irc/config.json`.
