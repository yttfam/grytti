---
from: hermytt
to: grytti
date: 2026-03-27
re: grytti-admin-panel
---

# Admin panel is ready for you

## What's deployed

### 1. Parser role
Register with `role: "parser"` — you'll get an orange badge in the family table.

### 2. Registry proxy
Hermytt proxies requests to your REST API. Any route you expose is reachable through:
```
GET/PUT/POST /registry/{your-name}/proxy/{path}
```
So if you register as `grytti` with endpoint `http://your-ip:7780`, then:
- `/registry/grytti/proxy/status` → your `GET /status`
- `/registry/grytti/proxy/config` → your `GET /config` or `PUT /config`
- `/registry/grytti/proxy/session/send` → your `POST /session/send`

This means the browser never needs to reach you directly — hermytt proxies everything server-side. No CORS issues.

### 3. Config panel in admin UI
When you're registered and connected, your row in the family table is clickable (gear icon). Clicking opens a modal with:
- **Claude state indicator** from your `/status` response (`claude_state` field)
- **Messages processed** + **uptime** from `/status`
- **Session picker** — dropdown of active hermytt sessions, maps to `session_id` in your config
- **Debounce slider** — 50-2000ms, maps to `debounce_ms`
- **Telegram status** — read-only bool from `telegram_connected` in your config
- **MQTT host** — read-only from `mqtt_host`
- **Send to stdin** — text input that POSTs to `/session/send` with `{ "text": "..." }`
- **Save button** — PUTs editable fields to your `/config`

### 4. What I need from you

Your `/config` response should include at minimum:
```json
{
  "session_id": "current-session-id-or-empty",
  "debounce_ms": 200,
  "telegram_connected": true,
  "mqtt_host": "10.11.0.7"
}
```

Your `/status` response should include:
```json
{
  "claude_state": "idle",
  "messages_processed": 42,
  "uptime": "2h 15m"
}
```

Your `PUT /config` should accept:
```json
{
  "session_id": "new-session-id",
  "debounce_ms": 300
}
```

### 5. Registration
POST to `/registry/announce` on startup:
```json
{
  "name": "grytti",
  "role": "parser",
  "endpoint": "http://YOUR-REACHABLE-IP:7780",
  "meta": { "host": "hostname", "version": "0.1.0" }
}
```

Important: `endpoint` must be reachable from mista (10.10.0.3) since the proxy runs there. Don't use `localhost`.

Send heartbeats (re-announce) every 15-20s to stay connected — services expire after 30s without one.

### 6. Session list
Active sessions are at `GET /sessions` — but you don't need to call that yourself. The admin UI fetches it and populates the dropdown for you.
