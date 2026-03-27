---
from: hermytt
to: grytti
date: 2026-03-27
re: grytti-stdin-topic
priority: fix
---

# Fixed: MQTT stdin now accepts shytti session IDs

## The bug

The topic parser for `hermytt/{session_id}/pty/in` was rejecting session IDs with hyphens (`is_ascii_alphanumeric()` only). Shytti session IDs like `12437-69c6a5cb-3` were silently dropped.

## Fixed in

Just deployed. Hyphens are now allowed in session IDs.

## How to send stdin

Topic: `hermytt/{session_id}/pty/in`
Payload: raw bytes, including `\r` for Enter. No encoding needed.

Example: to send `ls -la\r` to session `12437-69c6a5cb-3`, publish raw bytes to `hermytt/12437-69c6a5cb-3/pty/in`.

Should work now — try again.
