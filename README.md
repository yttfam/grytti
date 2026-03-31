# grytti

PTY stream parser and MQTT text bridge. The grit filter.

```
hermytt/{id}/pty/out (raw bytes via MQTT)
    → VTE parser → virtual grid → clean text
    → hermytt/{id}/pty/text (MQTT)
    → Telegram bot (optional)
```

Crytter parses escape sequences into pixels. Grytti parses them into text.

## Install

```bash
cargo build --release
```

Single binary. ~5.8MB stripped.

## Quick start

```bash
cp grytti.toml.example grytti.toml
# edit grytti.toml — set MQTT broker, add sessions

RUST_LOG=info ./target/release/grytti
```

## How it works

Grytti subscribes to hermytt's MQTT PTY stream, feeds raw bytes through a VTE state machine, maintains a virtual terminal grid, and publishes clean text back to MQTT.

### Two modes

**Headless (agent mode)** — MQTT in, MQTT out. No Telegram, no human needed. Agents read `hermytt/{id}/pty/text` and write to `hermytt/{id}/pty/in`. Pure machine-to-machine.

**Telegram bridge** — adds a Telegram bot frontend. Messages from Telegram get injected as stdin. Responses forwarded back with typing indicators. Auto-login flow handles Claude Code OAuth.

```
# Agent mode
hermytt/{id}/pty/out → grytti → hermytt/{id}/pty/text
                                 hermytt/{id}/pty/in ← other agent

# Telegram mode
Telegram message → grytti → hermytt/{id}/pty/in
hermytt/{id}/pty/out → grytti → Telegram response
```

### Claude Code detection

Detects Claude CLI state from TUI output:
- **Idle** — `❯` prompt visible, ready for input
- **Thinking** — spinner active, typing indicator sent to Telegram
- **Not logged in** — auto-triggers login flow, sends OAuth URL to Telegram
- **Process detection** — distinguishes Claude Code from raw shell

### Shell mode

Also works with raw shell sessions. Extracts command output between shell prompts and forwards to Telegram (or publishes to MQTT).

## Config

### Multi-session with Telegram

```toml
[mqtt]
host = "10.11.0.7"
port = 1883
username = "mqtt"
password = "secret"

# Session with Telegram bot (for humans)
[[sessions]]
session_id = "1-69c8e367-7180"
[sessions.telegram]
bot_token = "your:bot:token"

# Session with a different bot
[[sessions]]
session_id = "2-69c8e36e-7180"
[sessions.telegram]
bot_token = "another:bot:token"

# Headless session (for agents — no Telegram)
[[sessions]]
session_id = "agent-42"
```

### Single session (backwards compatible)

```toml
session_id = "your-session-id"

[mqtt]
host = "localhost"

[telegram]
bot_token = "your:bot:token"
```

### API config (optional)

```toml
[api]
bind = "0.0.0.0"
port = 7780
hermytt_registry = "http://10.10.0.3:7777"
hermytt_token = "your-hermytt-token"
endpoint = "http://10.10.0.3:7780"
```

Env var overrides: `GRYTTI_MQTT_USER`, `GRYTTI_MQTT_PASS`.

## REST API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/status` | GET | Uptime, claude state, process, messages |
| `/sessions` | GET | List all sessions |
| `/sessions` | POST | Create session (with or without bot_token) |
| `/sessions/:id` | GET | Session detail + last response |
| `/sessions/:id` | PUT | Update session config |
| `/sessions/:id` | DELETE | Remove session |
| `/sessions/:id/send` | POST | Inject stdin `{"text": "..."}` |
| `/sessions/:id/snapshot` | GET | Raw grid text |

## Architecture

```
src/
  main.rs       Entry point, MQTT event loop + debounce ticker
  config.rs     TOML config + env var overrides
  mqtt.rs       MQTT connect, subscribe, publish, stdin injection
  grid.rs       Virtual terminal grid (cursor, scroll regions, alt screen)
  parser.rs     VTE state machine → grid updates
  claude.rs     Claude CLI state detection + response extraction
  telegram.rs   Teloxide bot + response forwarding (optional)
  login.rs      Auto-login flow (OAuth URL → Telegram → code → stdin)
  api.rs        Axum REST API + hermytt registry
```

## Deploy

```bash
./deploy.sh   # cross-compile for linux-musl, scp to mista, restart systemd
```

## The YTT Family

| Project | Role |
|---------|------|
| [hermytt](https://github.com/yttfam/hermytt) | Transport multiplexer — routes bytes, auth, sessions |
| [shytti](https://github.com/yttfam/shytti) | Shell orchestrator — spawns and manages shells |
| [crytter](https://github.com/yttfam/crytter) | WASM terminal emulator |
| [prytty](https://github.com/yttfam/prytty) | WASM syntax highlighter |
| **grytti** | PTY stream parser + text bridge |
| [fytti](https://github.com/yttfam/fytti) | GPU-accelerated WASM app runtime |
| [wytti](https://github.com/yttfam/wytti) | WASI sandbox runtime |

## License

MIT
