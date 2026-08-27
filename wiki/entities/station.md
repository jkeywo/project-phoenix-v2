---
title: Station
type: entity
tags: [station, lobby, roster, rating, ai, human-seeking]
sources: [src/ship/config.rs, src/ship/coordination.rs, src/ship/coordination_systems.rs, src/lobby/stations_config.rs, src/lobby/session.rs, src/ship/components.rs, src/ship/rating_systems.rs, gui/mount-plan.js, assets/entities/alliance_destroyer.toml]
updated: 2026-08-27
---

# Station

A Station is an authored operable surface with a stable `StationId`, console,
ratings, and owned [Systems](./system.md). A direct station is offered as a
claimable lobby seat. An `auxiliary = true` station is mounted and resolved by
the simulation but is not offered as a separate seat.

## Authoring and wire shape

Each hull declares `[[station]]` blocks in its entity TOML. `StationConfig` in
`src/ship/config.rs` parses and validates them. At startup,
`stations_from_ship_config` projects the roster into `StationDef` values for
`Welcome`, including:

- stable id and display metadata;
- optional console URL;
- selectable rating names;
- human-seeking host order and visiting rating;
- whether the station is auxiliary.

The client uses the roster rather than a hardcoded station list. It derives
mount ids and iframe URLs from the station id and authored console URL.

## Ratings and control

A station rating names the systems delegated to AI in its
`automated_systems` list. `ActiveStationRatings` holds the live choice, and
`ShipSystemControlSources` derives each system's human/AI operating policy.
The directly connected holder may select only ratings authored for that
station. `Backfill` is a runtime-only rating used for an unmanned or AFK seat;
it is never a lobby-selectable authored rating.

## Human-seeking and auxiliary stations

A `human_seeking` station retains its own identity, systems, rating, and
console while the coordination resolver chooses where to present it:

1. its active direct holder, if it has one;
2. the first compatible directly held station in its finite `host_order`;
3. AI when no eligible human host exists.

`visiting_rating` defines the baseline capability while hosted. A scenario
detail floor may raise that capability but cannot lower it. A station that is
itself visiting cannot host another visiting station, preventing nested or
transitive authority. `VisitingStationHosts` publishes the resolved placement
so the client shell mounts the hosted surface for the right player.

The Alliance Destroyer demonstrates this shape with auxiliary Navigation,
Comms, and Command stations. Their host order and ratings are hull data, not
special cases in the lobby or client.

## Occupancy

`Player.station` records a direct claim. `SessionManager::holder_for_station`
returns only a connected holder, which is the occupancy rule shared by command
admission and human-seeking resolution. A disconnected player keeps the
station id on their record for reconnect restoration while its systems move to
Backfill control.

## Related

- [Console](./console.md)
- [System](./system.md)
- [Session](./session.md)
- [Stations](../concepts/stations.md)
