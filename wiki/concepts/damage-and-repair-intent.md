---
title: Damage And Repair Intent
type: concept
tags: [damage, repair, engineering, station, information]
sources: [pasm/spec/architecture/engineering-damage.yaml, src/console/repair/server.rs, src/console/repair/visibility.rs, gui/console-state.js, gui/components/ph-repair-teams.js, gui/destroyer/engineering.html, gui/cruiser/engineering.html, gui/battleship/repair.html]
updated: 2026-08-13
---

Summary

The intended damage-and-repair model separates information access by role and by repair-team presence. Engineering always sees ship-wide aggregate hull and dedicated `Core` damage detail, but exact non-`Core` internal damage remains hidden until a repair team physically arrives on site.

## Intended information model

- Engineering sees ship-wide aggregate hull.
- Engineering has a separate always-available `Core` detail surface.
- A station owner sees exact local damage for systems they own.
- Engineering does not get exact non-`Core` detail before arrival.
- Sending a team is allowed without a request, but travel time is also an information gate.
- When a team arrives, Engineering gets exact local detail for that station only.
- If the team leaves, that detailed local view disappears.
- If multiple teams are deployed, each team carries its own local revealed context.

## Intended authority model

- Requests for repair are advisory, not required.
- Human stations ask socially; AI-owned stations should emit level-3 repair requests.
- Engineering chooses whether to dispatch a scarce team.
- Dispatch targets are station-level or `Core`, not hidden internal subsystems.
- Once a team arrives, Engineering owns subsystem repair prioritization for that target station.
- That prioritization is local to the on-site team, not global across the whole ship.

## Implementation: the intended model is now enforced host-side

Issue #737 closed the gap this section used to describe: `src/console/repair/visibility.rs`'s `HullVisibility` now projects `RepairBlackboard.system_hull` and `queue_depth` per-recipient before either reaches the wire, rather than leaving role separation to client-side presentation. A station owner sees only its own systems; Engineering sees Core plus any non-Core system with a team physically on site (`Repairing`, not `Travelling`); everyone gets the two ship-wide scalars (`aggregate_hull_fraction`, and `destroyed_hull_fraction` since issue #1014) regardless of which rows they can see. `damageable_systems` (dispatch targets) and `teams` stay whole — Engineering must be able to dispatch to a system it cannot yet see exact detail for. The projection is shared by the live broadcast and the reconnect resync, so reconnecting cannot be used to obtain detail the live path withholds. The state is assembled in `gui/console-state.js` and rendered through the shared `ph-repair-teams` component used by the Destroyer and Cruiser Engineering consoles and the Battleship Repair console; since issue #1013 an on-site team sweeps every damaged system at its station rather than one, so "a team arrives" now reveals detail for however many of the station's systems it visits in turn, not just the one it was originally dispatched to.
