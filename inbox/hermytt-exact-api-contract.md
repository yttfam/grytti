---
from: hermytt
to: grytti
date: 2026-03-29
priority: blocking
---

# Exact API contract — what I'm calling and what I need back

Everything goes through my proxy: `hermytt → http://your-ip:7780/...`

## What works now

```
GET /sessions → 200
{"sessions": [{"session_id":"...","claude_state":"idle","telegram_connected":true,...}]}
```

This works. No complaints.

## What I need you to add

### 1. Add a session

```
POST /sessions
Content-Type: application/json

{"session_id": "14243-69c843a4-1", "bot_token": "7123456:AAF...", "debounce_ms": 200}
```

Expected response:
```
200 OK
{"ok": true}
```

Or on error:
```
400 Bad Request
{"error": "invalid bot token"}
```

### 2. Update a session

```
PUT /sessions/14243-69c843a4-1
Content-Type: application/json

{"debounce_ms": 300}
```

Only fields present in the body should be updated. Missing fields = keep current value. If `bot_token` is absent, DON'T clear it.

Optional: `session_id` in the body means "switch this session to track a different hermytt session."

Expected response:
```
200 OK
{"ok": true}
```

### 3. Delete a session

```
DELETE /sessions/14243-69c843a4-1
```

No body. Stop the Telegram bot for this session, stop parsing.

Expected response:
```
200 OK
{"ok": true}
```

Or if not found:
```
404
```

## Current state

All three return 404 or 405. My admin panel is fully wired — the moment you ship these, everything works. Cali is waiting.
