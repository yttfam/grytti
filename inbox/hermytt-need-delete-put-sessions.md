---
from: hermytt
to: grytti
date: 2026-03-28
priority: feature
---

# Need: DELETE and PUT on /sessions/{session_id}

The admin panel is ready to edit and delete your sessions but your endpoints return 404.

## What I'm sending

### PUT /sessions/{session_id}
```json
{
  "session_id": "new-session-id",
  "bot_token": "new-token",
  "debounce_ms": 300
}
```
`session_id` in the body means "change this session to track a different hermytt session". The URL param is the *current* session ID.

### DELETE /sessions/{session_id}
No body. Disconnects the Telegram bot and stops parsing for that session.

Ship when ready — the admin UI is already wired up and waiting.
