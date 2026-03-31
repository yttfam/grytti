# Contributing to grytti

## Getting started

```bash
git clone https://github.com/yttfam/grytti
cd grytti
cargo build
```

Requires Rust 2021 edition (stable 1.85+).

## Running locally

```bash
cp grytti.toml.example grytti.toml
# edit grytti.toml — MQTT broker, session IDs. Telegram bot token optional.
RUST_LOG=info cargo run
```

Needs a running hermytt instance with MQTT enabled and an active PTY session.

## Project structure

```
src/
  main.rs       Entry point, MQTT event loop + debounce ticker
  config.rs     TOML config with env var overrides
  mqtt.rs       MQTT client: subscribe, publish, stdin injection
  grid.rs       Virtual terminal grid (cells, cursor, scroll regions, alt screen)
  parser.rs     VTE state machine performer → grid updates
  claude.rs     Claude CLI TUI state detection
  telegram.rs   Teloxide bot handler + response forwarding
```

## Key concepts

**Grid:** 2D array of cells tracking cursor position, scroll regions, and alternate screen buffer. Enough fidelity to reconstruct text from Claude Code's React Ink TUI.

**State detection:** Scans grid snapshot for Claude CLI markers — spinner symbols + words (thinking), `⏺` (response), `❯` (idle prompt), `esc to interrupt` (working). Also detects shell prompts for raw shell sessions.

**Two modes:** Sessions with a Telegram bot token get a TG frontend (for humans). Sessions without run headless (for agents) — MQTT text bridge only.

**Debounce:** 200ms quiet period before publishing. Prevents flooding during spinner animations (~60fps redraws).

## Style

- Keep it simple. No over-engineering.
- `cargo clippy` clean.
- No unsafe.
- Small binary, fast startup, low memory.

## Pull requests

- One concern per PR.
- Describe what and why, not how.
- Tests for new features and bug fixes.
