# Project Phoenix — Entity Authoring

| Field | Value |
|---|---|
| Document | GDD-ENTITY-AUTHORING |
| Status | Working draft |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Generic entity templates, composition, world instances, and TOML authoring contract |
| Authority | Design and authoring overview. `src/entities/config.rs`, the validators, and shipped assets are runtime truth; PASM is architecture truth. |

This document defines the common design model for things that exist in a Phoenix world. It deliberately stops before the detailed ship-system model, which is covered by [Ships and Ship Systems](./ships-and-systems.md), and before scenario choreography, which is covered by [Scenario Authoring](./scenario-authoring.md).

Related documents: [World and Environmental Systems](./world-environmental-systems.md), [AI and Backfill](./ai-and-backfill.md), [Alliance Ships](../content/ships/alliance-ships.md), and [Harrow Ships](../content/ships/harrow-ships.md).

## Design intent

An entity is a reusable bundle of facts and capabilities. Ships, stations, planets, stars, asteroids, regions, infrastructure, civilian traffic, and invisible objective markers all use the same template-and-instance pipeline. A template says what a thing is and can do; a world instance says where this particular copy begins, what it is called in this scenario, when it exists, and which values differ here.

Entity data should create shared simulation state rather than scenario-specific exceptions. A collider is a physical hazard, a faction affects relationships, infrastructure condition affects capacity, a comms range controls contact, and operations declare what another ship may actually do. Scenario scripts observe and change those facts through supported verbs; they do not replace them with narrative-only claims.

## Authoring layers

| Layer | Responsibility | Typical location |
|---|---|---|
| Fragment | Reusable authored fields or keyed array entries shared by several templates. | `assets/entities/fragments/` |
| Entity template | Canonical capability, simulation, and presentation definition for one kind of entity. | `assets/entities/*.toml` |
| World instance | Placement, unique reference identity, spawn timing, and scenario-specific overrides. | `assets/worlds/*.toml` under `[[entity]]` |
| Runtime state | Current hull, position, orders, condition, flags, and other changing facts. | Authoritative host simulation |

Composition is resolved before parsing and spawning. Included fragments merge in declared order and the declaring template wins last. Keyed arrays such as systems, stations, station ratings, shield arcs, weapon banks, torpedo tubes, and doctrine objectives reconcile by stable identity during composition; bare tags union. An entry may be removed from composed content with `_remove = true`, while an empty array clears a whole list. World-instance overrides use the narrower runtime override contract: they may replace values and lists, but `_remove` is rejected.

## Common entity vocabulary

| Concern | TOML surface | Design meaning |
|---|---|---|
| Identity | `name`, `display_name`, `class`, `hull_id`, `tags` | Machine reference, crew-facing proper name, category, registry identity, and behavioural classification. |
| Allegiance | `faction` | Relationship and AI-side membership through a faction UUID. |
| Physical body | `[collider]`, `[hull]`, `[mesh]`, `[star]`, `[planet]` | Collision, damageability, and visible representation. |
| Detection | `[radar_appearance]`, `[target]`, `[comms]` | Whether and how the entity appears, can be selected, and can communicate. |
| Behaviour | `[behaviour]`, `[[behaviour.doctrine]]`, `[ai_profile]`, fine-system policies | Goals, movement/combat intent, and authored AI decisions. The retired behaviour-state FSM is not valid current authoring. |
| Environment | `[shape]`, `[effects]`, `[asteroid_field]` | Areas, fields, hazards, and generated populations. |
| Crew systems | `[infrastructure]`, `[tractor]`, `[dock]`, `[umbilical]`, `[repair.external_dispatch]`, `[scan]`, `[civilian]` | Condition/capacity, real external-system work a hull can perform, facts sensors can derive, and route-following traffic. |
| Presentation | `[[light]]`, `[audio]`, `[cinematic_camera]`, `[reference_grid]` | Renderer and host presentation that does not become a separate gameplay truth. |

Absence is meaningful. No radar appearance means no radar contact; no target section means the entity is not targetable; no tractor/dock/umbilical/external-dispatch configuration means the hull cannot perform that crew-system job; no infrastructure section means it has no authored condition/capacity model; no scan capability means it cannot produce structural readings.

## Template TOML specification

The following is an illustrative shape, not a copy-and-fill requirement. Authors should include only capabilities the entity possesses and should consult the live Rust type for the detailed fields of a chosen capability.

```toml
# Optional reusable fragments, resolved relative to this file.
includes = ["fragments/example-capability.toml"]

# `name` is template identity; `display_name` is crew-facing text or a string id.
name = "entity.example.name"
display_name = "entity.example.display_name"
class = "station"
tags = ["infrastructure", "comms_contact"]
faction = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"

[collider]
shape = "Cylinder"                 # Ball | Capsule | Cylinder
radius = 17.0
length = 0.0
half_height = 7.0                  # Cylinder only
movable = false                    # Ships must explicitly author true.

[hull]
hull_integrity = 300.0             # Simple entities use one pool.

[mesh]
model = "assets/models/example.glb"
scale = 1.0

[radar_appearance]
icon = "station"
colour = [0.3, 0.7, 1.0, 1.0]
size = 17.0

[target]
tags = ["station"]
threat_level = "none"
description = "entity.example.target.description"

[comms]
range = 500.0

[infrastructure]
condition_max = 100.0
hull_damage_share = 0.5

[[infrastructure.capacity]]
id = "transfer_throughput"
amount = 20

[[infrastructure.threshold]]
flag = "transfer_capable"
fails_below = 0.4

[scan]
power_group = "shields"
min_power_level = 2

[[scan.band]]
id = "detailed"
label = "entity.example.scan.band.detailed.label"
max_range = 120.0
condition_step = 0.01
```

Shapes and dimensions must match the visible object closely enough that navigation reads honestly. `movable` is an authored hazard fact, not cosmetic metadata: ships are mobile contacts and terrain is static. Mesh level-of-detail configuration belongs in the model rig sidecar, not `[[mesh.lod]]` in entity TOML.

Complex ships use per-system hull entries rather than a single integrity pool:

```toml
[hull]

[[hull.system_hull]]
system = "engines-port"
max_hp = 40.0

[[hull.system_hull]]
system = "reactor"
max_hp = 55.0
```

## World-instance TOML specification

Every placed entity references a template. `name` is the unique authored reference used by scripts, objectives, comms, and composition validation; it should not be treated as display prose. `display_name` supplies player-facing text where needed.

```toml
[[entity]]
template_path = "assets/entities/example_station.toml"
name = "world.example.entity.station.name"
display_name = "Kepler Transfer Station"
spawn_on = "world_load"            # or game_start where appropriate
when = "flag(example_station_present)"

[entity.transform]
position = [120.0, 0.0, -40.0]
rotation = [0.0, 1.57, 0.0]
scale = [1.0, 1.0, 1.0]

[entity.overrides.infrastructure]
condition = 35.0
```

An override should express a property of this instance, such as damage, local capacity, faction, doctrine, or a scenario role. It should not silently redefine the template’s fundamental identity. If several worlds need the same override, promote it into a new template or a fragment.

## Entity families

| Family | Required design questions |
|---|---|
| Ship | How does it move, survive, sense, communicate, fight or operate, and who or what controls its systems? |
| Station/infrastructure | Is it physical terrain, targetable, destructible, communicative, condition-bearing, capacity-bearing, and repairable? |
| Civilian traffic | Which authored route can it follow, which orders can it accept, and what happens when it cannot complete a leg? |
| Planet/star/moon | What is its true collision extent, visible form, radar treatment, and environmental role? |
| Asteroid/field | Is it one body or a deterministic population, what can damage it, and how does it affect navigation? |
| Region | What shape defines containment, what facts/effects apply inside it, and how is it shown on radar? |
| Objective marker | Is it intentionally non-physical and invisible except through scenario UI? |

## Authoring rules

- Prefer capability presence over entity-type branches. A station with weapons should use weapon capabilities; a ship with infrastructure should use infrastructure state.
- Keep stable machine identifiers separate from player-facing strings.
- Put tunable values in TOML and player-visible interface strings in the string catalogue, except for authored narrative content explicitly carried by the scenario/content format.
- Use faction relations rather than scenario code that special-cases individual hull names.
- Make sensors reveal authoritative facts. Do not give a script a second, contradictory version of condition, capacity, position, or allegiance.
- Give every AI-capable fine system an explicit authored policy or explicit idle declaration. Missing declarations are load errors in production content.
- Use template composition for reusable content and instance overrides for genuinely local variation.

## Acceptance criteria

- A template parses under strict entity validation and resolves every include without cycles or missing files.
- Every referenced model, marker, faction, station, system, power group, route, anchor, and script-visible entity name resolves through its canonical validator.
- Collider mobility and dimensions agree with the designed physical object.
- The entity’s visible, targetable, radar, comms, operations, scan, and infrastructure capabilities are present only when intended.
- A world may place multiple instances without identity collision, and each named instance has a unique reference.
- Scenario logic reads or changes the same authoritative state used by consoles and AI.

## Canonical sources

- `src/entities/config.rs` — live template schema and strict validation.
- `src/entities/include_resolve.rs` and `src/entities/entity_override.rs` — composition and instance-override semantics.
- `src/world/config.rs` — `[[entity]]` instance schema.
- `assets/entities/` — shipped examples.
- `pasm/spec/architecture/ship-entity-configuration.yaml` and `pasm/spec/architecture/world-files.yaml` — intended architecture and decisions.
