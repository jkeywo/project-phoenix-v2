---
type: AFK
priority: infrastructure
---

# 004 — Session Manager

Pure-logic module tracking session tokens → player records.

## What to build

`src/session.rs` — `SessionManager` struct with methods:

- `register(token, name) -> &Player` — new player; error if token exists
- `reconnect(token) -> Option<&mut Player>` — marks connected, returns previous console assignment
- `disconnect(token)` — marks disconnected, releases console
- `set_name(token, name)`
- `select_console(token, console) -> Result<(), ConflictError>` — fails if console taken by another connected player
- `clear_console(token)`
- `available_consoles() -> Vec<Console>`
- `players() -> &[Player]`

No Bevy, no network.

## Tests (highest value target per PRD)

Cover: new player registration, duplicate token detection, console assignment and clearing, returning-token auto-assignment, disconnection vacancy, querying available consoles.

## Acceptance Criteria

- [ ] All test cases pass with `cargo test session`
