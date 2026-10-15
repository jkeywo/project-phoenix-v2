---
title: WeaponsPlugin
type: concept
tags: [weapons, tactical, phaser, torpedo, blaster, targeting, ai]
sources: [crates/phoenix-sim-gameplay/src/weapons/pdc.rs, crates/phoenix-simulation/src/console/weapons/pdc.rs, assets/entities/pdc_cruiser.toml, crates/phoenix-simulation/src/console/weapons/assignments.rs, crates/phoenix-simulation/src/console/weapons/readiness.rs, crates/phoenix-simulation/src/console/weapons/round_work.rs, crates/phoenix-simulation/src/snapshot/weapon_targets.rs, crates/phoenix-model/src/weapon_operation.rs, crates/phoenix-sim-gameplay/src/weapons/operation.rs, crates/phoenix-model/src/turret.rs, crates/phoenix-sim-gameplay/src/weapons/turret.rs, crates/phoenix-simulation/src/weapons/mounts.rs, crates/phoenix-simulation/src/snapshot/mounts.rs, crates/phoenix-simulation/src/entities/ship_spawn.rs, crates/phoenix-simulation/src/console/weapons/mod.rs, crates/phoenix-simulation/src/console/weapons/server.rs, crates/phoenix-simulation/src/console/weapons/beam.rs, crates/phoenix-simulation/src/console/weapons/torpedo.rs, crates/phoenix-simulation/src/console/weapons/blaster.rs, crates/phoenix-simulation/src/console/weapons/blackboard.rs, crates/phoenix-sim-gameplay/src/console/weapons/shared.rs, tests/projectile_ship_traversal.rs, crates/phoenix-simulation/src/console/helm/server.rs, crates/phoenix-simulation/src/weapons/, crates/phoenix-simulation/src/server_app/registration.rs, crates/phoenix-simulation/src/server_app/world_setup.rs, gui/sim-state.js, gui/console-state.js, gui/weapon-cooldown.js, pasm/spec/architecture/weapons.yaml, crates/phoenix-sim-gameplay/src/console/weapons/beam.rs, crates/phoenix-sim-gameplay/src/ship/damage.rs]
updated: 2026-10-15
---

# WeaponsPlugin

`console/weapons/assignments.rs` retains per-System group and individual intent.
Its `WeaponAimInputs` resolves a current permitted measured point for the actual
weapon Console; lost feeds preserve orders while withholding aim. Explicit Hold
does not borrow the primary target. `snapshot/weapon_targets.rs` validates saved
owners, prepared group membership and reference shapes before restore writes.
`console/weapons/readiness.rs` applies the same intent, measured geometry, target
filters and actual turret bearing to core, fine and reconnect projections.

`console/weapons/round_work.rs` resolves a managed Local store to its real Capacity
endpoint and stages finite rounds with Power per emitted barrel. Other stock
claims and queued physical debits remain unavailable. Missing named barrel
markers preserve their indices and cannot substitute the ship centre.
Beam impact uses the physical ray result, including friendly blockers; a missed
measured ray spends its work without damaging the actor named by the contact.

Gameplay `ActiveBeam::complete_bank` ends an ordinary phaser cycle and applies its recorded pending cooldown. Exceptional relight uses the existing end path. Shared damage selection retains authored system order and the original RNG draw count.

The Tactical/weapons module follows the pure-root plus server-adapter convention. `crates/phoenix-simulation/src/console/weapons/mod.rs` declares and re-exports the module tree; `crates/phoenix-simulation/src/console/weapons/server.rs` owns `WeaponsPlugin`, shared resources, registration, and Tactical target selection.

Weapon consumers and projections read each ship's equipment components.
Missing components produce no mounts or ammunition and never borrow the global
compatibility resources. Torpedo resource-only harnesses run only with no Ship
entities; their magazine claims must also be unaddressed. Explicitly authored
legacy weapon configurations retain their existing construction rules.

## Module layout

| File | Responsibility |
|---|---|
| `crates/phoenix-simulation/src/console/weapons/server.rs` | Plugin assembly, shared state, admitted-consumer registration, Tactical AI target selection, and the Tactical-owned receiver for delivered frequency hints. |
| `crates/phoenix-simulation/src/console/weapons/beam.rs` | Phaser banks, target-lock application, auto-fire, beam lifecycle, and damage. |
| `crates/phoenix-simulation/src/console/weapons/torpedo.rs` | Tube load/fire commands, target snapshots, torpedo lifecycle, and detonation. |
| `crates/phoenix-simulation/src/console/weapons/blaster.rs` | NPC blaster charge/fire and hit application. |
| `crates/phoenix-simulation/src/console/weapons/blackboard.rs` | Weapons/Tactical radar blackboards, `WeaponsUpdate`, and its broadcaster. |
| `crates/phoenix-sim-gameplay/src/console/weapons/shared.rs` | Cross-family helpers and one-tick handoff resources. |
| `crates/phoenix-simulation/src/console/weapons/server_tests.rs` | The large plugin integration suite included from `crates/phoenix-simulation/src/console/weapons/server.rs`. |

Pure weapon state machines used by these adapters live in Gameplay
`crates/phoenix-sim-gameplay/src/weapons/`; Simulation retains compatibility
exports. `mount.rs` owns shared geometry and damage delivery; `pattern.rs` owns
common barrel planning. The beam, bolt and guided adapters consume those
mechanisms without changing their cycle state, impact-facing policies or
admitted command vocabulary. `entities/config/weapon_definition.rs` compiles
the common authored form into the same strategy parameters at content load.

The closed weapon configuration now supports `BeamOperation::Fixed` (default)
and `Sustained`, carried by portable beam hardware and the existing active beam
slot. The pure `beam_work_seconds` helper bounds fixed work by remaining duration
and sustained work by the real tick. Sustained configuration rejects fixed-cycle
jitter. Simulation launch/stop and continuation joins remain the integrator's
pending executor work at this source checkpoint.

Optional bolt `rounds` references the bank's actual same-room Local Capacity
store. Gameplay `weapons/operation.rs` reserves rounds before the caller stages
Power, then consumes or releases that exact claim. `tick_with_work` calls admission
with each actual barrel index, skipping unavailable or refused listed barrels;
a due attempted step advances even if none fires. Missing origins do not borrow
another barrel. `recoil_delta` returns the opposite world XZ impulse. These
helpers keep quantities in the existing Stock owner.

`WeaponDefinition.targets` compiles to each supported bank/tube and portable
beam hardware as `Option<Vec<EntityTag>>`. The pure `target_allowed` helper
keeps caller-provided ordinary-domain eligibility for None, rejects every target
for an explicit empty list, and otherwise matches only held observed tags.
Unknown classification does not become a Ship or Missile fact. The authored
`targets=false` override clears an inherited filter while the runtime still
stores None; Workshop's Legacy policy writes that tombstone. These are selection
rules, and cannot make friendly or civilian physical collateral immune.



`phoenix-model/src/turret.rs` describes the bounded `Turret` System component.
Gameplay `weapons/turret.rs` owns finite sector traversal and actual bearing;
Simulation `weapons/mounts.rs` stages its local energy and one per-System state
map. The mount borrows the owning capability's existing target and availability.
It keeps no target, enable latch or AI runtime of its own. Missing intent returns
toward the authored home through paid motion. The passive producer rotates its
existing field of view using the same bearing, without narrowing reception to a
weapon's aim tolerance. `snapshot/mounts.rs` preflights the complete candidate
owner set and continuation frontier before any restore write. Workshop's existing
System-component form edits these fields through the normal accepted transaction.

## Command symmetry

Each authored bank, tube, and Tactical control is a fine `SystemId`. Human controls and AI hosts emit identical admitted payloads. In particular, `ai_target_selection` emits `SetTarget`; `handle_set_target` is the sole writer of the authoritative Tactical selection. Auto-fire reads that selection but does not bypass target admission.

The Tactical selector ranks the ship's visible/scored objective surface deterministically. A named positive operate/destroy directive can nominate its exact live target; ordinary acquisition remains bounded by the authored radar reach and hostility rules.

## Coordination

Weapons resolves its fine Tactical/Helm Systems to explicit owning-Station
addresses before enqueue. An out-of-arc target produces an
`ArcBearingRequest` addressed to Helm with the selected usable family's exact
arc geometry. AI Helm's owning receiver preserves that geometry; a delivered
withdrawal clears the standing request even if a different family most recently
held it. A delayed `FrequencyHint` addressed to
Tactical reaches `receive_tactical_coordination`; that Tactical-owned receiver
rechecks the live control policy before latching the unchanged frequency for
the existing next-tick applier. The payload never selects or widens its own
recipient.

## Projectile identity

Immediate torpedo launches in `handle_fire_torpedo` and blaster ticks in
`tick_blaster_system` visit ships in UUID order before consuming the shared
Projectile mint. This matches the burst-launch traversal in `tick_torpedoes`.
Each ship retains its admitted-command order and authored bank/barrel order.
The entity index breaks ties only for legacy fixtures without a stable UUID.
`tests/projectile_ship_traversal.rs` moves a ship through an inert archetype
without changing its state and compares actual projectile identities and full
traces across both storage orders and fresh default/default/pinned processes.
## Damage pipelines

Beam processing is phased so read-only shooter/LOS preparation completes before damage and lifetime updates. Torpedo processing similarly builds one target snapshot before advancing every ship's torpedoes. Both paths route shields and hull damage through the shared damage model, publish lifecycle events through `SimOutbox`, and leave the LocalShip present when defeat is latched.

The magazine consumes load claims after that tick's projectile lifecycle. A newly
reserved round therefore starts with its full authored load time and advances
on the next tick. `tests/reconnect_snapshot_boundary.rs` checks that boundary
and the shared 10 Hz Weapons cache while exercising an ordinary load/unload
and reconnect; a reconnect does not reset that cache.

Weapon reach, arcs, cooldowns, load times, payloads, colours, and AI policy parameters come from entity TOML. `crates/phoenix-simulation/src/server_app/world_setup.rs` projects the selected hull's authored configuration into runtime components/resources; renderer-only beam appearance stays separate from hit authority.

## Publication

Publish systems build per-system blackboards plus the Tactical radar aggregate. `weapons_update_broadcaster` sends the LocalShip summary at 10 Hz to the projected weapons holder. Client panels render the authored bank/tube ids and arcs from `ShipClientConfig`; they do not own firing state.

Tactical radar sorts its emitted blips and region overlays by UUID. The initial
world reconciliation and ECS archetype traversal can enumerate the same data
in different orders; neither order determines the wire payload or keyboard
cycling order. The publisher's regression reverses both storage sources and
compares the complete projection.

`gui/sim-state.js` retains the Welcome's Phaser and Blaster bank configuration. `gui/console-state.js` uses `gui/weapon-cooldown.js` to join authored cooldown durations onto live bank states by bank id, separately for each weapon family, before the shared Tactical renderer passes them to the controls. The same helper supplies the bounded remaining-time percentage for both progress bars; the `tactical-weapons-projection` component in `pasm/spec/architecture/weapons.yaml` records the presentation contract.

## Related

- [Weapons Runtime](./weapons-intent.md)
- [Radar Projection](./radar-projection.md)
- [Broadcaster Seam](./broadcaster-seam.md)
- [Information-Parity Audit](./information-parity-audit.md)

Sustained beam operation is frozen on the existing active bank slot at launch. Its finite duration value stays unchanged while ordinary paid work continues; unavailable control/equipment, shutdown or failed funding ends it through the same cooldown and BeamEnded lifecycle as an explicit Stop. Snapshots require the live slot's operation and preserve fractional damage and its bought cooldown; presentation continues to render the restored live bank directly.

The optional `pdc_cruiser` uses the existing finite Broadside battery, paid local
rounds and ordinary per-bank control. Its explicit Missile filter and radar
selection policy require an acquired current classified contact, including its
dated elevation; the fixed full-coverage mount does not claim finite pitch
traversal. All current Turret mechanics constrain azimuth only; a permitted
vertical lead has no separately configured pitch actuator. Full XYZ range still
gates emission and readiness. Straight bolts keep XYZ, pitch, physical payload, speed and visual
scale on the emitted record. The same emission loop has planar and measured XYZ
adapters; `advance_flight_only` needs no surviving launcher configuration.

Physical bolt contact merges the earliest actual Rapier Collider shape cast
with swept guided-round spheres, without faction immunity or a selected-target
exception. Interception debits the actual guided vector's durability and removes
a destroyed round without its detonation payload. Interval-local previous bolt
position is consumed once during Damage and omitted from settled snapshots and
digests. Restore starts with no pending segment; the next Physics step derives
it from persisted current XYZ. Detached flight carriers use the same vectors.
