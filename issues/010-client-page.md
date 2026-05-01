---
type: AFK
priority: feature-tracer
---

# 010 — Client Page Application

Standalone HTML + JS on client.html — the phone console.

## What to build

`client.html` with inline/linked JS:

- Reads peer ID from URL fragment (`#<peer-id>`)
- Loads/generates session token from `localStorage`
- Generates random space-themed name if none stored
- Lobby UI: editable name input, console picker (show taken/available), Engage button (captain only), Join In Progress button
- Connects to host peer via PeerJS on page load
- Sends `ClientMessage` JSON values on user actions
- Updates UI in response to incoming `ServerMessage` events
- Console UI (in-game): Red Alert toggle button for captain; simple status display for others
- Reconnect: sends `Identify` on connect to reclaim previous session

## Acceptance Criteria

- [ ] Page loads in mobile browser, connects to host (manual test)
- [ ] Name persists across refresh
