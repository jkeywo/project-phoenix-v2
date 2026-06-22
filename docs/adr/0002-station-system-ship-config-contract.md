# ADR-0002: Station/System Ship Config Contract

**Status:** Accepted  
**Date:** 2026-06-22  
**Issue:** [#488](https://github.com/jkeywo/project-phoenix-v2/issues/488)  
**Parent PRD:** [#487 - Station / Console / System architecture redesign](https://github.com/jkeywo/project-phoenix-v2/issues/487)

---

## Context

The current player ship model uses fixed monolithic consoles as several
different concepts at once: lobby assignment, GUI ownership, authority,
AI delegation, damage, and power. `assets/entities/player_ship.toml` also
defines a per-player-count `[stations]` promotion/demotion graph, so crew
layout changes require maintaining parallel rosters and `next`/`previous`
links for each player count.

PRD #487 replaces that model with three layers:

- **Station** - the fixed roster seat a player can claim.
- **Console** - the single cohesive GUI owned by a station.
- **System** - a fine-grained capability instance on the ship.

This ADR locks the foundational contract for the future ship-config schema and
wire addressing model. It is intentionally a contract decision, not the runtime
migration. Until the loader migration lands, `player_ship.toml` continues to use
the old schema and the game continues to run through the existing console-based
runtime.

---

## Decisions

### 1. Stable identifiers

Stations, systems, and power groups are addressed by stable designer-authored
string ids:

```rust
pub struct StationId(pub String);
pub struct SystemId(pub String);
pub struct PowerGroupId(pub String);
```

These ids are:

- ship-local authoring keys;
- stable across save/load and wire messages;
- unique within their scope (`SystemId` is ship-wide unique);
- not player session tokens;
- not world entity UUIDs.

The seed player ship should derive initial ids from existing semantic names,
`id` values, and rig `marker` values. Prefer readable lower-kebab ids over
UUIDs. Examples:

```text
captain
helm
repair
power
shields
sensors-radar
navigation-chart
comms
phaser-fore
phaser-aft
torpedo-magazine
torpedo-tube-fore-port
torpedo-tube-fore-starboard
torpedo-tube-aft
viewscreen
```

For existing per-instance weapons, use the clearest stable seed:

- `marker = "phasers_fore"` -> `phaser-fore`
- `id = "fore_port"` -> `torpedo-tube-fore-port`

### 2. Fixed station roster

The new `[[station]]` roster replaces the old per-player-count `[stations]`
layout and its `next`/`previous` promotion graph.

```toml
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons, shields, and threat response."
rank = "Ltn."
short_code = "TAC"
console = "tactical"
```

Station fields:

| Field | Required | Meaning |
|---|---:|---|
| `id` | yes | Stable `StationId`; unique within the ship. |
| `name` | yes | Human-facing station name. |
| `description` | yes | Lobby/help text. |
| `rank` | yes | Rank label shown for players at this station. |
| `short_code` | no | Compact UI label, e.g. `TAC`; defaults to empty. |
| `console` | yes | Single cohesive GUI shell owned by the station. |

Removed from the new model:

- `min_players`
- `max_players`
- `[[stations.N]]`
- `consoles = [...]`
- `next`
- `previous`

Systems assigned to a station are discovered from `[[system]].station`, not
duplicated on the station.

### 3. System instances

Systems are the unit of capability, control, damage, and effective power.
System kinds are code/registry-bound; TOML instantiates and wires existing
kinds but does not define new behavior.

```toml
[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"
power_group = "weapons"
marker = "phasers_fore"

[system.config]
facing_deg = 0
fire_arc_deg = 270
auto_arc_deg = 180
beam_range = 25
beam_damage_per_sec = 5
beam_duration_secs = 6
cooldown_secs = 6
beam_color = [1, 0.4, 0.1, 1]
```

System fields:

| Field | Required | Meaning |
|---|---:|---|
| `id` | yes | Stable ship-wide-unique `SystemId`. |
| `kind` | yes | Registered system kind. |
| `station` | conditional | Owning `StationId`; omit only for AI-only ownerless systems. |
| `ai_only` | conditional | Must be `true` when `station` is omitted. |
| `power_group` | conditional | `PowerGroupId`; required for powered system kinds. |
| `marker` | no | Optional rig-marker seed/link for mounted systems. |
| `[system.config]` | kind-specific | Existing behavior tuning for this system instance. |

Ownerless systems are represented by omitting `station` and setting
`ai_only = true`.

```toml
[[system]]
id = "viewscreen"
kind = "viewscreen"
ai_only = true
power_group = "ops"
```

Rules:

```text
station omitted + ai_only = true  -> valid ownerless Core system
station omitted + ai_only = false -> invalid
station = "core"                  -> invalid; Core is not a claimable station
```

`Core` is a repair/control destination for ownerless infrastructure, not a
station id.

### 4. Per-station rating tables

Ratings are explicit per-station tables. They are not derived from
`assets/complexity/*.toml`.

```toml
[[station]]
id = "tactical"
name = "Tactical"
description = "Weapons, shields, and threat response."
rank = "Ltn."
short_code = "TAC"
console = "tactical"

[[station.rating]]
name = "Assisted"
automated_systems = [
  "torpedo-magazine",
  "torpedo-tube-fore-port",
  "torpedo-tube-fore-starboard",
  "torpedo-tube-aft",
]

[[station.rating]]
name = "Manual"
automated_systems = []
```

Rules:

- Rating names are scoped to one station.
- Each rating explicitly lists the station-owned systems automated at that
  rating.
- Counts and names may vary by station.
- An unclaimed or disconnected station automates all of its systems directly,
  independent of a named rating.
- A loader must reject rating entries that reference unknown systems or systems
  not owned by that station.

The old complexity TOMLs remain legacy UI/AI tuning assets until the migration
removes them. The new rating table is the source of truth for human-vs-AI
control ownership.

### 5. Power groups

Power groups are operator-facing aggregations. Membership is declared on each
system with `power_group`; optional group metadata lives in `[power_groups.*]`.

```toml
[power_groups.weapons]
label = "Weapons"
default_level = 2
min_level = 1
max_level = 4

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"
power_group = "weapons"
```

The power operator changes a group allocation. The power system resolves that
group into member `SystemId`s and emits per-system effective allocation
messages/state updates. Downstream systems consume their own effective power
state and do not need to know which group they belong to.

### 6. Control wire envelope

The future control wire uses one generic target-addressed envelope with a typed
payload enum:

```rust
ClientMessage::ControlSystem {
    target: SystemId,
    payload: SystemControlPayload,
}
```

The payload expresses the command; the envelope expresses which system instance
receives it. Examples:

```rust
SystemControlPayload::HelmInput {
    thrust: 0.75,
    steering: -0.5,
}

SystemControlPayload::FirePhaser

SystemControlPayload::SetPowerGroupAllocation {
    group: PowerGroupId("weapons".into()),
    level: 3,
}

SystemControlPayload::DispatchRepairTeam {
    team_idx: 0,
    target: RepairTarget::Station(StationId("tactical".into())),
}
```

Repair dispatch targets the repair system as the command receiver, but the
repair destination is one of:

```rust
pub enum RepairTarget {
    Station(StationId),
    Core,
}
```

Power allocation targets the power/reactor system as the command receiver, but
the payload names a `PowerGroupId`. The power system then resolves the group to
member systems.

### 7. Codec contract

The existing codec contract is unchanged:

- `serde_json` remains isolated to `src/core/codec.rs`, except for the already
  sanctioned HTML bridge surfaces in that module.
- New wire types derive `Serialize` and `Deserialize`.
- Round-trip tests for new message variants live in `src/core/codec.rs`.
- No binary-format, transport, or PeerJS topology change is implied by this
  ADR.

### 8. Compatibility boundary for #488

Issue #488 must not migrate live runtime configuration.

Allowed in #488:

- this ADR;
- additive wire identity/control types;
- codec round-trip tests;
- wiki/index updates documenting the contract.

Not allowed in #488:

- editing `assets/entities/player_ship.toml` to the new schema;
- changing the current station loader;
- changing lobby assignment behavior;
- changing game start behavior;
- changing console AI behavior;
- changing client/server routing behavior.

---

## Consequences

- Future loader work has a precise schema and validation target.
- Future UI/control work can emit one uniform system-targeted control envelope.
- Runtime remains stable while the migration is split into smaller PRs.
- The old `Console` enum remains in place until each runtime slice is migrated
  and the legacy message variants are retired.

---

## Follow-up Work

1. Build an additive ship config loader/verifier for `[[station]]`, `[[system]]`,
   ratings, power groups, and ownerless AI-only validation.
2. Build a system registry keyed by system kind.
3. Convert current consoles to coarse systems one at a time.
4. Migrate lobby/session state from `Player.consoles` to `Player.station`.
5. Migrate HTML console actions to `ControlSystem`.
6. Remove old per-player-count `[stations]` once the fixed roster is live.
