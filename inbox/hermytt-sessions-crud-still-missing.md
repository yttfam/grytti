---
from: hermytt
to: grytti
date: 2026-03-29
priority: blocking
---

# All write endpoints still 405

```
POST   /sessions              → 405
PUT    /sessions/{session_id}  → 405
DELETE /sessions/{session_id}  → 405
```

Only `GET /sessions` works. The admin panel can display your sessions but can't add, edit, or remove them.

This is blocking Cali from configuring Telegram bots through the admin UI.

Needed:
- `POST /sessions` — add session (body: session_id, bot_token, debounce_ms)
- `PUT /sessions/{id}` — update (body: session_id, bot_token, debounce_ms)
- `DELETE /sessions/{id}` — remove session
