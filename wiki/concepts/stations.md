---
title: Stations
type: concept
tags: [stations, lobby, ratings, authority, backfill, human-seeking]
sources: [src/lobby/stations_config.rs, src/lobby/session.rs, src/ship/config.rs, src/ship/rating_systems.rs, src/command_admission/policy.rs, assets/entities/alliance_destroyer.toml]
updated: 2026-08-27
---

# Stations

A station is a designer-authored crew seat. Its stable `StationId` determines tenure and coordination addressing; its station config supplies display copy, console URL, selectable ratings, and optional human-seeking behaviour. Systems are separate fine-grained nouns and name their owning station in ship TOML.

## Fixed roster

Each hull declares a fixed `[[station]]` roster in its entity TOML. `ShipConfig` validates the station and system graph. `stations_from_ship_config` projects it into `ShipStations`, the lobby-facing resource:

- `StationDef.id`, `name`, `description`, `rank`, and `short_code` describe the seat;
- `console` selects the client panel, with a generic fallback when absent;
- `ratings` lists lobby-selectable ratings in authored order;
- `human_seeking`, `host_order`, `visiting_rating`, and `auxiliary` define an auxiliary station that can be hosted by another occupied seat.

`ShipStations` is one `Vec<StationDef>`; a token absent from
`StationAssignments` is unseated (or explicitly a Spectator in session state).

## Tenure, ratings, and Backfill

`Player.station` is authoritative tenure. The holder chooses among the station's authored lobby ratings. `Backfill` is runtime-only: it is never offered as a lobby selection, and represents a vacant/disconnected station whose systems are operated by AI until the holder reconnects or the seat is claimed again.

`ActiveStationRatings` and `ShipSystemControlSources` derive each fine system's current human/AI policy from that roster and live session state. Downstream consumers do not branch on human versus AI; both origins emit the same admitted commands.

## Admission

`station_for_system` in `src/command_admission/policy.rs` resolves a command target through the ship's authored `[[system]]` entry. For a `human_seeking` system, the live host map wins over the authored home station. Shield arcs resolve through their synthesised fine-system entries, with a narrow legacy fixture fallback. Unknown or ownerless systems do not acquire human authority.

Never cast `SystemId` to `StationId` because the strings happen to match on one hull. Station tenure and system identity are different types and can diverge on auxiliary layouts.

## Related

- [Station entity](../entities/station.md)
- [System entity](../entities/system.md)
- [System Addressing](./coarse-system-migration.md)
- [AI Ship Unification](./ai-ship-unification.md)
