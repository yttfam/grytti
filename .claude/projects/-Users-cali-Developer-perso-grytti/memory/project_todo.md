---
name: grytti-todo
description: Future features and improvements for grytti
type: project
---

Backlog:

- **Chat allowlist enforcement** — config has `allowed_chats` but it's not enforced. Gate TG message handler on it.
- **Dangerous command blocking** — optional filter for stdin injection (exit, rm, etc.)
- **Line break cleanup** — TG output has formatting issues with line breaks
- **Runtime session add/remove** — POST/DELETE /sessions needs to spawn/kill TG bot tasks at runtime
- **Conversation context** — track multiple turns, not just last screen
- **Grid size auto-detect** — match PTY dimensions from hermytt meta on first connect
