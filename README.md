# grytti

PTY stream parser, formatter, and Telegram bot. The grit filter.

```
hermytt/{id}/pty/out (raw bytes via MQTT)
    → VTE parser → virtual grid → clean text
    → hermytt/{id}/pty/text (MQTT)
    → Telegram bot (formatted output to authorized chats)
```

Crytter parses escape sequences into pixels. Grytti parses them into text.

## Install

```bash
cargo build --release
```

Single binary. ~5MB stripped.

## Quick start

```bash
cp grytti.toml.example grytti.toml
# edit grytti.toml — set MQTT broker, Telegram bot token, session ID

RUST_LOG=info ./target/release/grytti
```

Or with env vars:

```bash
GRYTTI_MQTT_USER=mqtt GRYTTI_MQTT_PASS=secret GRYTTI_TG_TOKEN=your:token \
  RUST_LOG=info ./target/release/grytti
```

## How it works

Grytti subscribes to hermytt's MQTT PTY stream, feeds raw bytes through a VTE state machine, maintains a virtual terminal grid, and extracts clean text.

**Claude Code bridge:** Detects Claude CLI state (idle, thinking, tool use) from the TUI output. Telegram messages get injected as stdin, typing indicators show while Claude thinks, responses get forwarded back.

```
Telegram message
  → MQTT hermytt/{id}/pty/in (stdin injection)
  → Claude Code processes
  → MQTT hermytt/{id}/pty/out (raw PTY)
  → VTE parser → grid → state detection
  → Telegram response (with typing indicator)
```

## Config

```toml
session_id = "your-session-id"

[mqtt]
host = "10.11.0.7"
port = 1883
username = "mqtt"
password = "secret"

[telegram]
bot_token = "your:bot:token"
allowed_chats = []
```

All values can be overridden via env vars: `GRYTTI_MQTT_USER`, `GRYTTI_MQTT_PASS`, `GRYTTI_TG_TOKEN`.

## Architecture

```
src/
  main.rs       Entry point, select loop: MQTT rx + debounce tick
  config.rs     TOML config + env var overrides
  mqtt.rs       MQTT connect, subscribe, publish, stdin injection
  grid.rs       Virtual terminal grid (cursor, scroll regions, alt screen)
  parser.rs     VTE Perform impl → grid updates
  claude.rs     Claude CLI state detection (idle/thinking/tool use)
  telegram.rs   Teloxide bot + response forwarding
```

## The YTT Family

| Project | Role |
|---------|------|
| [hermytt](https://github.com/yttfam/hermytt) | Transport multiplexer — routes bytes, auth, sessions |
| [shytti](https://github.com/yttfam/shytti) | Shell orchestrator — spawns and manages shells |
| [crytter](https://github.com/yttfam/crytter) | WASM terminal emulator |
| [prytty](https://github.com/yttfam/prytty) | WASM syntax highlighter |
| **grytti** | PTY stream parser + Telegram bot |
| [fytti](https://github.com/yttfam/fytti) | GPU-accelerated WASM app runtime |
| [wytti](https://github.com/yttfam/wytti) | WASI sandbox runtime |

## License

MIT
