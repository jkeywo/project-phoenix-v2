---
type: AFK
priority: feature-tracer
---

# 009 — Server Page JS Shell

JavaScript running on server.html — owns the PeerJS host lifecycle.

## What to build

Inline `<script>` in `server.html` (or a linked `server.js`):

- Generates a UUID v4 peer ID and creates a PeerJS host peer
- Generates QR code via `qrcode.js` CDN pointing to `client.html#<peer-id>`
- Accepts incoming PeerJS connections
- Routes inbound data → `wasm_receive_message(json)`
- Wires up `set_message_callback` to send outbound JSON to the correct peer(s)
- Self-loopbacks host (captain) messages back into WASM
- Fires `ClientMessage::ClearConsole` equivalent when a connection closes

## Acceptance Criteria

- [ ] `server.html` loads in a browser and creates a PeerJS peer (manual test)
- [ ] QR code displayed in lobby
