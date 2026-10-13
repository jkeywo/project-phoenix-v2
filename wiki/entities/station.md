---
title: Station
type: entity
tags: [station, lobby, roster, rating, ai]
sources: [crates/phoenix-sim-gameplay/src/ship/config.rs, crates/phoenix-simulation/src/lobby/stations_config.rs, crates/phoenix-sim-session/src/lobby/session.rs, crates/phoenix-simulation/src/ship/coordination_systems.rs, crates/phoenix-simulation/src/ship/rating_systems.rs]
updated: 2026-10-13
---

# Station

A Station is an authored crew seat with a stable `StationId`, display metadata,
ratings and optional stance catalogue. `Player.station` records its directly
claimed tenure; native display assignments also name Stations.

A Station hosts one or more [Consoles](console.md). Each Console owns its
[Systems](system.md), its screen and its permitted observations. Detached
human-seeking Consoles replace auxiliary hosted surfaces without adding seats.

`SessionManager::holder_for_station` returns a connected direct holder.
Disconnected players retain their Station identity for reconnect while ordinary
Backfill operates the affected Systems. Human-seeking Console placement tries
its direct home holder before its authored candidate Stations; a hosted tab
never constitutes a new direct holder.

`ActiveStationRatings` retains the crew's selected complexity. Console ratings
can preserve a detached surface's own ladder; `console_system_depths` derives
its owned Systems' AI, simplified or detailed modes. Hosting and ratings feed
`ShipSystemControlSources`; damage remains an independent offline override.
`Backfill` is runtime-only, not an authored lobby rating.

`ShipStations` carries separate seat and Console definitions. The lobby claims
seats, while the client mounts screens from the Console definitions and current
`ConsoleHosts`. Shipped hull rosters remain authored data.

See [Stations](../concepts/stations.md) for session and GM integration.
