---
title: Console
type: entity
tags: [console, role, lobby, station]
sources: [src/core/messages.rs, src/ship/config.rs, src/lobby/stations_config.rs, src/command_admission/policy.rs, gui/mount-plan.js, gui/console-resolver.js, gui/action-map.js]
updated: 2026-08-27
---

# Console

A Console is the browser surface authored for a [Station](./station.md). It is
not a server-side enum: a hull's `[[station]]` blocks declare the station id,
display metadata, ratings, and optional `console` URL, and `Welcome` carries
that roster to the client.

`gui/mount-plan.js` turns each mountable station into stable section and iframe
ids (`<station>-ui` and `<station>-iframe`). Tactical retains the sole legacy
DOM alias, `weapons-ui` / `weapons-iframe`. `gui/console-resolver.js` resolves
the authored URL, and stations without a resolvable URL are not mounted.

## Identity and authority

- `StationId` identifies the operable seat or hosted surface.
- `SystemId` identifies the fine-grained capability being controlled.
- `Player.station` records the directly claimed lobby seat.
- `ClientMessage::ControlSystem { target, payload }` is the common command
  envelope used by human consoles and AI controllers.

Command admission resolves the sender's station, the target system's owner,
the active station rating, any visiting-station host, and the game-phase rule.
Console HTML never grants authority by hiding or showing a control.

## Composition

Station rosters and console composition vary by hull. A station can own several
systems and present them in one page; an auxiliary human-seeking station can be
mounted as a hosted tab without becoming a separately claimable lobby seat.
The server-supplied roster and station-host snapshot are therefore the source
of truth. Client code must not assume a fixed number or fixed list of consoles.

To add or change a console, update the hull's station/system authoring, provide
or reuse a panel URL, route its actions through `gui/action-map.js`, and add the
matching server handler or system-kind policy. The mount plan needs no per-hull
registry entry.

## Related

- [Station](./station.md)
- [System](./system.md)
- [Session](./session.md)
- [Console UI Authoring Library](../concepts/console-ui-library.md)
