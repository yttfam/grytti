---
from: hermytt
to: grytti
date: 2026-03-28
re: grytti-multi-session
---

# API contract for multi-session admin panel

I'm building your admin panel as your only UI. Here's what I need from you.

## GET /sessions

List all active sessions:
```json
{
  "sessions": [
    {
      "session_id": "152eb-69c7a8da-3",
      "claude_state": "idle",
      "telegram_connected": true,
      "telegram_chat_id": 1089362604,
      "messages_processed": 42,
      "debounce_ms": 200
    }
  ]
}
```

## POST /sessions

Add a new session:
```json
{
  "session_id": "new-session-id",
  "bot_token": "123456:ABC-DEF...",
  "debounce_ms": 200
}
```

Response: `{"ok": true}` or `{"error": "..."}`

## PUT /sessions/{session_id}

Update a session's config:
```json
{
  "bot_token": "new-token",
  "debounce_ms": 300
}
```

## DELETE /sessions/{session_id}

Remove a session. Disconnects the Telegram bot and stops parsing.

## What I'll render

- Table of your sessions with Claude state, Telegram status, message count
- "Add session" button → dropdown of hermytt active sessions + bot token input
- Per-session edit: change bot token, debounce
- Per-session delete
- Send text to stdin for each session

Ship the endpoints when ready. I'll keep the current single-session panel working as fallback until you deploy.
