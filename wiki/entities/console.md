---
title: Console
type: entity
tags: [console, knowledge, ownership, station]
sources: [crates/phoenix-model/src/observation.rs, crates/phoenix-model/src/console.rs, crates/phoenix-sim-gameplay/src/ship/console_ownership.rs, crates/phoenix-simulation/src/ship/coordination_systems.rs, crates/phoenix-simulation/src/ship/console_publication.rs, crates/phoenix-simulation/src/ship/console_views.rs, gui/console-topology.js, gui/console-state.js, gui/console-working-view.js, editor/workshop-ship-authoring.js]
updated: 2026-10-14
---

# Console

A Console has a ship-local `ConsoleId`, an optional screen URL and a knowledge
scope. Each System names one controlling Console. Stations remain crew seats;
a Station can host several Consoles without gaining another claimable seat.
A screenless Console supplies an AI-only knowledge scope.

`[[console]]` authoring records its home Station, fixed or human-seeking hosting,
ordered candidate Stations, visiting rating and typed observation feeds.
`ShipConfig::console_for_system` is the common ownership lookup. Validation
rejects missing System ownership rather than translating old `system.station`
content silently.

## Hosting and authority

`resolve_human_seeking_hosts` derives `ConsoleHosts` and each owned System's
control policy. It retains direct-holder preference, authored candidate order,
Backfill and scenario detail requirements. Hosting changes do not reset System
intent or individual AI memory. Human commands still address `SystemId` and
pass ordinary admission using the Console's current host. A feed grants
information, never control.

`ShipStations.consoles` supplies explicit screen definitions. The client mounts
and routes tabs by Console identity, including repeated uses of the same HTML
screen. Native display assignment and lobby tenure remain Station-based.

## Knowledge and continuation

`ConsoleKnowledgeView` reads existing System publications: owned Systems plus
explicit source-and-kind feeds. Co-hosted Consoles do not pool these views.
`recorded_observations(source)` borrows metadata through that same `get` predicate;
it cannot expose metadata for a withheld value. Scan, dossier evidence, held Sensors
reports and repair queue observations use this surface; live gauges produce no
invented sample timestamp. See [Recorded Observations](../concepts/recorded-observations.md).
`console_publication` projects permitted publications per recipient, while the
client further filters each mounted Console. Projection caches are derived;
there is no second authoritative observation store.

`SimSnapshot.console_health` supplies the tab's health summary from the hull
entries owned by that Console. Detached Console equipment is not folded into
Core. Station summaries remain available for the spectator view.

`ConsoleWorkingViews` holds supported zoom, filters and inspected records by
Console. Authorised view updates pass the existing command boundary but cannot
operate machinery. Snapshots, restore and digests include this state; personal
accessibility remains local to the receiving device.
`ShipConfigComponent` requires this working-view component on NPC ships too,
so GM-operated Consoles use the same apply/capture/restore path.

Workshop edits Console composition through the existing document/history flow.
See [Station](station.md), [System](system.md), and the implementation contracts
in `pasm/spec/design/console-ownership.yaml`.
