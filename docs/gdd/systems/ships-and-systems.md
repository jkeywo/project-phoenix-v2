# Project Phoenix — Ships and Ship Systems

| Field | Value |
|---|---|
| Document | GDD-SHIPS-SYSTEMS |
| Status | Working draft |
| Owner | Unassigned |
| Last updated | 2026-08-18 |
| Scope | Generic ship design, stations, fine systems, damage, power, control, and TOML contract |
| Authority | Generic design overview. Live Rust schemas, validators, shipped hulls, and PASM remain canonical. |

This document explains how to design a Phoenix ship without prescribing a particular faction or hull. Alliance and Harrow content are catalogued separately in [Alliance Ships](../content/ships/alliance-ships.md) and [Harrow Ships](../content/ships/harrow-ships.md).

Related documents: [Station Experiences](./station-experiences.md), [AI and Backfill](./ai-and-backfill.md), [Difficulty and Balance](../foundation/difficulty-balance-playtesting.md), [Onboarding and Accessibility](../foundation/onboarding-accessibility.md), [Campaign Continuity and Persistence](../foundation/campaign-continuity.md), and [Future Modes](../future/future-modes.md).

Detailed mechanics: [Movement and Helm](../mechanics/movement.md), [Targeting and Weapons](../mechanics/targeting-weapons.md), [Damage, Diagnosis and Repair](../mechanics/damage-repair.md), [Power and Resource Network](../mechanics/power-resource-network.md), [Shields](../mechanics/shields.md), [Sensors and Epistemics](../mechanics/sensors-epistemics.md), [Navigation and Relative Motion](../mechanics/navigation-relative-motion.md), [Comms and Commitments](../mechanics/comms-commitments.md), and [External Operations](../mechanics/external-operations.md).

## Ship design goal

A ship is a connected operating problem, not a bag of independent minigames. Movement changes firing solutions and hazard exposure; damage removes capability and creates repair priorities; power creates trade-offs between propulsion, weapons, and shields; sensors and comms turn state into crew knowledge; station topology determines who must coordinate with whom.

The same authored hull can be flown by a full crew, a partial crew, no human crew, or NPC AI. Human and AI operators issue the same system-control commands after admission. Player identity is attached to the selected runtime instance, not baked into the hull template, so a world-spawned copy of a player-capable hull remains an ordinary NPC.

## Topology

```text
Ship template
  ├─ physical and presentation configuration
  ├─ stations: crew-facing bundles and rating choices
  ├─ systems: authoritative fine capabilities
  ├─ station ownership: which player holds each bundle
  ├─ control source: Human or Backfill AI per system
  ├─ power groups: shared allocation pressure
  └─ damage hull: capability-bearing sections
```

A station is a crew-facing workload bundle. A system is the smallest authoritative controllable capability. Systems belong to stations for ordinary control, but the relationship is data rather than code: compact hulls may put helm, sensors, shields, comms, and power on two broad stations, while a large hull may expose each as a separate job.

## Stations and player count

The possible player range is `0–Max Players per Ship`, where the maximum is normally the number of claimable stations. Recommended player count belongs to the scenario because workload depends on the situation as well as the hull.

Each station authors an ordered set of ratings. A rating names the systems automated for that holder and may supply AI tuning. A fully manual rating automates little; simpler ratings automate more. An unclaimed or disconnected station keeps its holder relationship but its systems fall to Backfill control until the player reconnects or the station is reclaimed.

Human-seeking systems such as comms or navigation may walk an authored station order to find any human-held console before accepting AI. The order must be a complete permutation with the owning station first. This preserves the principle that any available human is preferred without creating an invisible allow-list.

## Fine-system catalogue

The registry is extensible; these are the established design families rather than a promise that every hull carries every system.

| Family | Typical fine systems | Shared design pressure |
|---|---|---|
| Command | Captain, red alert, viewscreen | Priorities, alert posture, shared presentation |
| Helm | Thrust, steering, joystick, engines, impulse, boost, lateral thrust, helm radar | Position, facing, speed, hazard avoidance, commitment to manoeuvres |
| Tactical | Tactical radar, phaser control/banks, blaster banks, torpedo magazine/tubes | Target handoff, firing arcs, range, frequency, ammunition, timing |
| Science | Sensors, sensor radar, scan | Contact interpretation, frequency hints, structural facts, evidence |
| Defence | Shields and authored shield arcs | Facing, focus trade-offs, regeneration, damage prevention |
| Engineering | Power reactor/battery, repair | Allocation, reserve, brownout risk, damaged-system priority |
| Operations | Navigation, comms, external operations | Routes, hails, commitments, towing, stabilisation, escort, transfer |

## Ship-state interactions

### Movement

Helm configuration defines speed, reverse speed, acceleration, deceleration, yaw authority, low-speed turning, boost, impulse, lateral/vertical capability, radar, and engine presentation. These values should create a recognisable flight role. A light hull may change the geometry of a fight quickly; a heavy hull may need to plan turns and use range or arcs deliberately.

### Weapons

Weapon banks are independent systems with authored arcs, markers, range, cadence, damage, visuals, frequency behaviour, power group, and AI policy. Phaser-style beams and blaster-style shots may occupy different tactical roles. Torpedoes add a finite magazine, loading, tubes, launch geometry, flight behaviour, and distinct coordination pressure. Weapon reach is not increased by reactor allocation; power affects authored performance slots, not arbitrary scenario exceptions.

### Shields

Shields use authored arcs and a common base configuration. Focus may strengthen one facing while weakening others. Incoming direction therefore matters to Helm, Tactical, Shields, and Repair. A hull may use two broad arcs, four directional arcs, or another authored arrangement; arc order is load-bearing and must remain stable through composition.

### Power

The established groups are `helm`, `weapons`, and `shields`. Each group has an allowed level range and a default. The reactor defines capacity, charge/discharge rates, the sustainable allocation total, the maximum commanded total, and an emergency threshold. Commanding above the sustainable total consumes reserve; crossing the emergency threshold can brown out the ship and lock groups low. The design purpose is a legible temporary trade, not routine arithmetic maintenance.

### Damage and repair

Player ships normally divide hull integrity across system-bearing sections. Damage degrades the corresponding capabilities and becomes visible through engineering and station status. Repair teams travel and restore sections over time; dispatch capacity and travel delay make the priority consequential. NPC or simple entities may use one hull pool where detailed internal damage would add no meaningful decision.

### Sensors, navigation, comms, and external operations

Sensors reveal contacts and authored facts; navigation makes routes and spatial objectives actionable; comms carries hails, choices, promises, and status; operations perform physical work such as stabilising, towing, escorting, transferring, or field repair. These should converge on shared targets and state so one officer’s discovery changes another officer’s options.

## Ship TOML skeleton

This skeleton shows the relationships among sections. Detailed weapon, radar, AI-policy, audio, and PFX fields are intentionally omitted here; the live schema and an existing hull are the correct references when implementing one.

```toml
name = "entity.example_ship.name"
display_name = "entity.example_ship.display_name"
class = "destroyer"
hull_id = "AEV-000"
power_rating = 70
tags = ["ship", "comms_contact"]
faction = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"

[collider]
shape = "Capsule"
radius = 4.0
length = 12.0
movable = true

[hull]

[[hull.system_hull]]
system = "engines-port"
max_hp = 30.0

[[hull.system_hull]]
system = "reactor"
max_hp = 40.0

[helm_console]
max_speed = 18.0
max_reverse_speed = 6.0
acceleration = 6.0
deceleration = 7.0
max_yaw_rate = 0.6
low_speed_turn_boost = 0.35

[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
marker = "weapon_fore"
facing_deg = 0.0
fire_arc_deg = 140.0
auto_arc_deg = 140.0
beam_range = 120.0
beam_damage_per_sec = 3.0
beam_duration_secs = 4.0
cooldown_secs = 5.0

[torpedoes]
count = 12

[[torpedoes.tubes]]
id = "tube-port"
marker = "torpedo_port"

[shields_console]
focus_bonus_max_hp = 50

[shields_console.base]
num_facings = 2
max_hp = 80
regen_per_sec = 2.5
offline_duration = 8.0

[[shield_arc]]
id = "fore"
label = "shield_arc.fore.label"
center_deg = 0.0
width_deg = 180.0
hull_max_hp = 6.0

[[shield_arc]]
id = "aft"
label = "shield_arc.aft.label"
center_deg = 180.0
width_deg = 180.0
hull_max_hp = 5.0

[power]
capacity = 70.0
rates = [1.0, 1.0, 1.0, 1.0, 1.0, 1.0]
sustainable_total = 6
max_commanded_total = 8
emergency_threshold = 10.0

[power_groups.helm]
label = "ship.power_group.helm"
default_level = 2
min_level = 1
max_level = 4

[power_groups.weapons]
label = "ship.power_group.weapons"
default_level = 2
min_level = 1
max_level = 4

[power_groups.shields]
label = "ship.power_group.shields"
default_level = 2
min_level = 1
max_level = 4
```

## Station and system TOML

```toml
[[station]]
id = "tactical"
name = "station.tactical.name"
description = "station.tactical.description"
rank = "station.tactical.rank"
short_code = "TAC"
console = "gui/weapons-console.html"
manual_overview = "Operate targeting, energy weapons and torpedoes."

[[station.rating]]
name = "Manual"
automated_systems = []

[[station.rating]]
name = "Assisted"
automated_systems = ["phaser-fore", "torpedo-magazine"]

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
station = "tactical"
power_group = "weapons"

[[system]]
id = "comms"
kind = "comms"
station = "captain"
human_seeking = true
seek_order = ["captain", "tactical"]
```

`id` is stable identity and `kind` selects the registered behaviour. Several instances may share one kind. `ai_only = true` is reserved for capabilities that should never become a direct crew control. `config` carries kind-specific TOML only where the registered system consumes it.

## AI authoring

Every AI-capable fine system must declare an inline policy/selector or explicitly declare itself idle. Policies are ordered rules over host-provided facts and parameters that emit registered channel/verb actions; selectors rank candidates. Content must not assume that an unknown fact name will be caught automatically, so new policies require deliberate review and an exercised scenario.

The top-level `[[behaviour.doctrine]]` objective list remains live for NPC travel and combat intent, but it is transitional architecture rather than a second place to author direct actuator logic. Do not reintroduce the retired behaviour state machine.

## Hull-design method

1. State the hull fantasy and the decisions it should create.
2. Choose its movement, durability, range, arcs, ammunition, and operational capability as one coherent role.
3. Define the fine-system set and power groups.
4. Bundle systems into stations for the intended maximum crew, then author ratings for smaller crews.
5. Define damage sections and repair capacity so loss of capability is readable and recoverable.
6. Author human and AI control for the same systems.
7. Verify the hull as player-controlled, partially crewed, zero-crew Backfill, and world-spawned NPC where applicable.
8. Balance through authored scenarios and headless runs rather than isolated stat comparison alone.

## Acceptance criteria

- Every station, rating automation entry, system, power group, hull section, weapon bank, tube, shield arc, and marker reference validates.
- All AI-capable systems have explicit policy or idle declarations.
- No required capability depends on a particular crew size; vacant systems remain operable through Backfill.
- The ship has a clear role visible through movement, survival, armament, and workload—not only through prose.
- Damage, power, position, target choice, and scenario operations create cross-station consequences.
- A world-spawned copy does not inherit player-only identity.
- Recommended player count is recorded by each scenario that offers the hull.

## Canonical sources

- `src/ship/config.rs` and `src/system_registry.rs` — stations, ratings, system instances, power groups, and system kinds.
- `src/entities/config.rs` — hull capability sections and strict AI-policy validation.
- `pasm/spec/architecture/station-system-authority.yaml`, `data-driven-fine-system-ai.yaml`, `weapons.yaml`, `shields.yaml`, `engineering-damage.yaml`, and `power-modifiers-regions.yaml`.
- `assets/entities/alliance_*.toml` and `assets/entities/ship_harrow_*.toml` — shipped hull examples.
