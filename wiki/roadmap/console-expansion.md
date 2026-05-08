---
title: Console Expansion
type: roadmap
tags: [roadmap, console, captain, helm, weapons, engineering, science, comms]
sources: [PRD-066, docs/3.md, docs/8.md]
updated: 2026-05-08
---

# Console Expansion

Path from today's two consoles to the full six-station bridge crew.

## Today

Two consoles ship in `main`:

- **[Captain](../entities/captain-console.md)** — Red Alert toggle, View Mode selector, Engage button.
- **[Helm](../entities/helm-console.md)** — Thrust + steering. The only console that moves the ship.

Both follow the [Console Plugin Pattern](../concepts/console-plugin-pattern.md): one Bevy plugin per console on the client, mirrored by message-handling logic on the server.

## Next

**[Weapons](../entities/bridge-crew-stations-planned.md) and [Engineering](../entities/bridge-crew-stations-planned.md)** land together in [PRD #66](../sources/prd-066-weapons-and-engineering.md):

- Weapons: target-lock radar, phaser fire control.
- Engineering: hull integrity bar, breakdown queue, repair button.
- Every existing console (Captain, Helm) also gains a Repair button — wrong console = 30 s penalty cooldown.

This is the first PRD that touches every previously-shipped console. It validates the "console plugin retrofit" workflow.

## After PRD #66

### Science — [Draft 3](../sources/design-03-science-console.md)

Three tabs: long-range radar (zoomable rings), impulse drive (three-state bar with charge-up time), system chart (planets + warp targets). Drives content onto the viewscreen — first console other than Captain to do so.

Open question: viewscreen authority model. See [Open Architectural Questions](./open-architectural-questions.md).

### Comms — [Draft 8](../sources/design-08-comms-console.md)

Stub. No design yet.

## Pattern lessons

Each new console reinforces the architecture:

- **One plugin per console** keeps client code segregated. New plugin = new file, no edits to existing plugins.
- **Server is authority.** The console sends `ClientMessage`s and receives `ServerMessage`s. No client-to-client traffic.
- **Pure functions for derived data.** Helm reuses `radar_dots`. Future Weapons radar should reuse the same iterator with a wider range.
- **The Console enum is load-bearing.** Adding `Weapons`, `Engineering`, `Science`, `Comms` variants in `src/shared/messages.rs` is the first edit for any new console PRD. Every test that exhaustively matches `Console` must be updated.

## Multi-console clients

Today one player picks one console. The architecture allows one player to occupy multiple consoles simultaneously (`Session` holds a `Vec<Console>` of assignments today). This becomes important when crew counts are low — one human covers Helm + Weapons. Per-console message subscription ([Architecture Improvement Notes](../sources/notes-architecture-improvements.md)) becomes the union of subscriptions across that player's consoles.

## Cross-references

- Entity: [Console](../entities/console.md), [Bridge Crew Stations (planned)](../entities/bridge-crew-stations-planned.md)
- Concept: [Console Plugin Pattern](../concepts/console-plugin-pattern.md)
- Roadmap: [Combat & Damage](./combat-and-damage.md), [Open Architectural Questions](./open-architectural-questions.md)
