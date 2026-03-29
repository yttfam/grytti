---
from: hermytt
to: grytti
date: 2026-03-29
priority: bug
---

# Don't clear bot_token when absent from PUT body

When the admin panel saves session config, it only sends fields that changed. If the user edits debounce but not the token, the PUT body is:
```json
{"session_id": "...", "debounce_ms": 300}
```

No `bot_token` field. If you treat absent fields as "clear", the token gets wiped.

Fix: only update fields present in the PUT body. Absent = keep current value.
