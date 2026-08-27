---
title: Comms Range
type: concept
tags: [comms, range, hail, authority, contacts]
sources: [src/comms/mod.rs, src/comms/range.rs, src/comms/component.rs, src/comms/roster.rs, src/comms/server.rs, src/console/comms/server.rs, src/core/messages.rs, src/entities/config.rs, src/entities/spawner.rs, gui/comms-state.js, assets/entities/alliance_destroyer.toml]
updated: 2026-08-27
---

# Comms Range

Comms endpoints opt in with an authored `[comms]` block. A contact is reachable when both endpoints support range and their distance is within the effective link range. Hailability and physical range are separate authored facts.

`src/comms/range.rs` contains the pure distance/range calculation. `src/comms/server.rs` maintains each ship's authoritative contact roster and range flags from live transforms. `src/console/comms/server.rs` enforces the same flags when a hail or reply command is applied, so stale or malicious client state cannot bypass range.

`CommsContact.in_range` describes the current roster entry. Each `CommsMessage.sender_in_range` snapshots the sender's status when the message enters the inbox, allowing the UI and Backfill policy to explain why a thread can or cannot continue.

Entities with no `[comms]` endpoint are not contacts. Missing range state is denied while range enforcement is active; fixtures/worlds that explicitly run without range use the documented inactive mode rather than fabricated flags.

## Related

- [Comms Panel](./comms-panel.md)
- [World Data](../entities/world-data.md)
