---
title: Comms Range
type: concept
tags: [comms, range, hail, authority, contacts]
sources: [crates/phoenix-model/src/comms_history.rs, crates/phoenix-simulation/src/comms/mod.rs, crates/phoenix-simulation/src/comms/range.rs, crates/phoenix-simulation/src/comms/component.rs, crates/phoenix-simulation/src/comms/roster.rs, crates/phoenix-simulation/src/comms/server.rs, crates/phoenix-simulation/src/console/comms/server.rs, crates/phoenix-model/src/messages.rs, crates/phoenix-sim-gameplay/src/entities/config.rs, crates/phoenix-simulation/src/entities/spawner.rs, gui/comms-state.js, assets/entities/alliance_destroyer.toml]
updated: 2026-10-15
---

# Comms Range

Comms endpoints opt in with an authored `[comms]` block. A contact is reachable when both endpoints support range and their distance is within the effective link range. Hailability and physical range are separate authored facts.

`crates/phoenix-simulation/src/comms/range.rs` contains the pure distance/range calculation. `crates/phoenix-simulation/src/comms/server.rs` maintains each ship's authoritative contact roster and range flags from live transforms. `crates/phoenix-simulation/src/console/comms/server.rs` enforces the same flags when a hail or reply command is applied, so stale or malicious client state cannot bypass range.

`CommsContact.in_range` describes the current roster entry. Each `CommsMessage.sender_in_range` snapshots the sender's status when the message enters the inbox, allowing the UI and Backfill policy to explain why a thread can or cannot continue.

Entities with no `[comms]` endpoint are not contacts. Missing range state is denied while range enforcement is active; fixtures/worlds that explicitly run without range use the documented inactive mode rather than fabricated flags.

`CommsRuntime.open_hails` uses Model `comms_history::HailHistory` to retain the
issuing ship and target together. Fleet Human/ordinary AI hail consumers use the
issuing slot's physical gate; clearing and AI candidacy retirement affect only
that source. Snapshot capture/restore and the ordered digest retain the same
source-scoped history. The local roster remains a presentation projection.

## Related

- [Comms Panel](./comms-panel.md)
- [World Data](../entities/world-data.md)
