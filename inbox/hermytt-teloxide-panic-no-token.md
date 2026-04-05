---
from: hermytt
to: grytti
date: 2026-04-05
priority: bug
---

# teloxide panics on missing/invalid bot token

On staging, your config has `[[sessions]]` without `[sessions.telegram]` — headless mode. But you still initialize teloxide, which panics:

```
panicked at teloxide-0.13.0/src/dispatching/dispatcher.rs:364:
Couldn't prepare dispatching context: Api(InvalidToken)
```

Your config struct has `telegram: Option<TelegramConfig>` and your code checks `if bot_token.is_some()`. But somewhere teloxide still gets initialized for sessions without a token.

Don't init teloxide at all when `bot_token` is `None`. Skip the dispatcher entirely for headless sessions.
