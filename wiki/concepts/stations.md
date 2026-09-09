---
title: Stations
type: concept
tags: [stations, lobby, ratings, authority, backfill, puppeting, human-seeking]
sources: [src/lobby/stations_config.rs, src/lobby/session.rs, src/lobby/result_application.rs, src/lobby/crew_replication.rs, src/ship/config.rs, src/ship/rating_systems.rs, src/command_admission/policy.rs, src/gm_puppet.rs, src/gm_action.rs, gui/gm-station-puppet.js, gui/console-state.js, gui/console-core.js, assets/entities/alliance_destroyer.toml]
updated: 2026-09-09
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

Lobby message results reach both loaded-Ship components through
`LobbyResultApplier::apply` in `src/lobby/result_application.rs`. Before the
LocalShip exists, pending Lobby choices remain in `SessionManager`; no temporary
rating component is created and discarded by each message system.

In an active fleet, `PendingCrewRatingChanges` stages local crew requests for
ordinary admission. `apply_assigned_station_rating` changes the live rating and
control sources on the same agreed tick on every peer, including the ship host.
Reconnect and AFK bookkeeping project the latest choice from `PendingCommands`
plus unsent requests; they never activate local AI before that tick. Mid-game
rating input retains its Input schedule position and is submitted next tick.


An active GM puppet is a temporary control overlay, not tenure. It can be added
only while the Station is Backfill, suppresses AI only for that Station, and
does not alter `Player.station`. Releasing the last GM reapplies the ordinary
live rating; a disconnected holder therefore returns to Backfill, while a
holder who has reconnected remains Human. The affected Station's authentic
console receives crew-public operator and latest-activity feedback from the
authoritative snapshot projection.

The GM's local Station projection stops at the same raw boundary as a player:
`WorldResource` entities plus absolute live `SimState` entity fields, current
ship pose, waypoint, objectives and tagged blackboards, plus the exact complete
`ShipClientConfig` produced for an ordinary `Welcome`. The browser runs these
through the ordinary `ClientSimState`, radar-region and console-state builders.
Consequently authored ranges, filters, arcs, hull identity, tutorials and
assist gaps reach a puppeted spatial Station's authentic Helm, Sensors or
Navigation interface with current world data; it does not render an origin-only
or partially configured GM approximation. Correlated actions retain the
originating iframe identity. Admission-time refusals return immediately;
accepted actions remain unprojected Pending until the ordinary System consumer
reports its terminal result. That exact-once canonical result returns through
`__updateActionFeedback` so accessible controls settle Applied or Refused
instead of treating admission as successful execution. If the browser ingress
queue refuses synchronously, the same correlation settles locally as Refused.
The presentation tracker is capacity- and time-bounded; an accepted command
whose terminal feedback does not arrive becomes visibly TimedOut, while any
later result is ignored rather than mutating the terminal occurrence.

Takeover and command grants are revalidated when their canonical boundary is
actually applied, not only when the owner sequences them. A delayed takeover
is Refused if the holder reconnected first; same-tick command/release grants
retain their exact owner order. GM host loss removes only that GM's membership,
preserving equal peers and restoring Backfill immediately when the last leaves.
An active takeover or command stamped after that operator's agreed loss
boundary is durably Refused from canonical mesh state, even if it was sequenced
before the transport disappeared. Recovery advances a journalled slot
generation, so those old grants stay stale after rejoin while new-generation
grants apply; the generation is part of snapshot, digest and replay state. A
grant stamped exactly at an unapplied loss boundary retains the established
PreUpdate-before-FixedUpdate order and is then cleaned up by the loss.

## Admission

`station_for_system` in `src/command_admission/policy.rs` resolves a command target through the ship's authored `[[system]]` entry. For a `human_seeking` system, the live host map wins over the authored home station. Shield arcs resolve through their synthesised fine-system entries, with a narrow legacy fixture fallback. Unknown or ownerless systems do not acquire human authority.

A GM-issued Station command resolves the same payload-aware effective target
and live availability before it reaches System consumers. Thus a Helm iframe's
`SetView { Radar }` addressed to `viewscreen` is authorized by the authored
Helm Radar System, while a forged Science Radar view is refused as outside
Helm. GM attribution is recorded at that boundary for activity, then stripped
from the ordinary `AdmittedCommand`; console handlers still see the same
`SystemControlPayload` they receive from a player or AI. An accepted command
waiting for the tick buffer is snapshotted and folded until delivery.

Never cast `SystemId` to `StationId` because the strings happen to match on one hull. Station tenure and system identity are different types and can diverge on auxiliary layouts.

## Related

- [Station entity](../entities/station.md)
- [System entity](../entities/system.md)
- [System Addressing](./coarse-system-migration.md)
- [AI Ship Unification](./ai-ship-unification.md)
