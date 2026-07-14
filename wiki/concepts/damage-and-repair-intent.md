---
title: Damage And Repair Intent
type: concept
tags: [damage, repair, engineering, station, information]
sources: [pasm/spec/architecture/engineering-damage.yaml, src/console/repair/server.rs, gui/console-state.js, gui/engineering-console.html]
updated: 2026-07-14
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

## Current implementation tension

The shipped repair UI currently derives a global `system_hull` list and `dispatch_targets` directly from the repair blackboard, which means Engineering has broader pre-arrival visibility than intended. See [gui/console-state.js](/C:/Coding/project-phoenix-v2/gui/console-state.js) and [gui/engineering-console.html](/C:/Coding/project-phoenix-v2/gui/engineering-console.html).
