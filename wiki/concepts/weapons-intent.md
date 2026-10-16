---
title: Weapons Intent
type: concept
tags: [weapons, tactical, phaser, blaster, torpedo, replication, reconnect]
sources: [crates/phoenix-simulation/src/console/weapons/mod.rs, crates/phoenix-simulation/src/console/weapons/server.rs, crates/phoenix-simulation/src/console/weapons/blackboard.rs, crates/phoenix-simulation/src/console/weapons/beam.rs, crates/phoenix-simulation/src/console/weapons/blaster.rs, crates/phoenix-simulation/src/console/weapons/torpedo.rs, crates/phoenix-simulation/src/core/broadcast/lifecycle.rs, crates/phoenix-sim-gameplay/src/weapons/blaster.rs, crates/phoenix-sim-gameplay/src/weapons/torpedo.rs, crates/phoenix-sim-gameplay/src/entities/config.rs, gui/components/ph-phasers-controls.js, gui/components/ph-blasters-controls.js, gui/components/ph-torpedo-controls.js, crates/phoenix-sim-gameplay/src/weapons/mount.rs, crates/phoenix-sim-gameplay/src/weapons/pattern.rs, crates/phoenix-sim-gameplay/src/entities/config/weapon_definition.rs, gui/components/weapon-target-controls.js, gui/stations/weapon-target-actions.js, editor/workshop-weapon-groups.js, gui/workshop-weapon-groups-panel.js, crates/phoenix-simulation/src/console/weapons/readiness.rs, gui/sensor-tracks.js, crates/phoenix-simulation/src/console/weapons/guidance_link.rs, crates/phoenix-simulation/src/console/weapons/guidance_ui.rs, crates/phoenix-sim-gameplay/src/weapons/guided_orders.rs, gui/components/guidance-controls.js, gui/stations/guidance-actions.js, crates/phoenix-simulation/src/console/weapons/illumination.rs, crates/phoenix-sim-gameplay/src/weapons/illumination.rs, gui/components/illumination-controls.js, editor/workshop-guided-profile.js, gui/workshop-guided-profile.js, crates/phoenix-sim-gameplay/src/weapons/shared_loaders.rs, crates/phoenix-simulation/src/console/weapons/shared_loaders.rs, editor/workshop-shared-loaders.js, gui/workshop-shared-loaders.js]
updated: 2026-10-16
---

# Weapons Intent

Tactical controls three independent, ship-authored weapon families through admitted `ControlSystem` commands.

Shipped armament is authored as `[[weapon]]` entries with a common
`weapon.mount`, a typed `weapon.delivery`, markers/patterns and its ordinary AI
declaration. `strategy_id` preserves the existing bank/tube identity where the
composition entry uses a unique name. Entity loading validates and compiles
these definitions once into the beam, bolt or guided strategy parameters;
there is no second runtime weapon store. Family-specific authored blocks remain
supported parameter shorthand and cannot duplicate the same strategy identity.

Gameplay `weapons/mount.rs` owns the common planar geometry and impact payload
contract. The existing `weapons/pattern.rs` resolves schedules and barrel
sequences in stable authored order. Bolts use timed firing steps; torpedoes use
the sequence for origins while loaded rounds and burst cadence retain authority
over ammunition. These shared mechanisms introduce no new combat controller.

- **Phasers:** sustained, target-locked beams. A ready bank must have its target inside its arc and range; damage accumulates during the beam, then that bank cools down.
- **Blasters:** straight, non-homing projectile volleys. Each bank predicts target motion at firing time, may require holding to charge, then fires its configured volley; bolts have their own speed, range-derived lifetime, collision radius, damage, and shield pierce.
- **Torpedoes:** guided, proximity-detonating projectiles. Tubes have arcs and load state, share a magazine, and snapshot damage/pierce at launch.

The Alliance Cruiser fore and aft banks use their authored fixed firing arcs;
the fore bank has no optional Turret component or traverse constraint.

Preparation and sustained beams spend affordable continuous local energy; an
instant firing step pays every simultaneous barrel/projectile before ammo,
cooldown or effects change. Pending torpedo burst rounds remain physical onboard
commitments until emitted. Zero intake does not silence a charged weapon.
Explicit Tactical restraint and scenario hold/release orders share one canonical
intent; AI reads it through its permitted Console publication.

Torpedo ammunition now uses [shared resource stocks](resource-stores.md): loading claims a round, completion consumes it into the tube, and cancellation releases it. The existing readout remains available magazine quantity. Configured docked replenishment honours tube-held space.

All three are ship-specific TOML capabilities. Tactical receives configured arcs plus live bank/tube state; the current weapons target is shared by player controls, Backfill, the Tactical blackboard, and the viewscreen projection. Current limitations and future work belong in PASM and GitHub rather than this navigation page.

## Client projection lifecycle

`weapons_update_broadcaster` computes the current `WeaponsUpdate` at 10 Hz and sends it only to the holder of the ship's authored Weapons Station (`Audience::HoldingWeapons`). `WeaponsPlugin` owns both the corresponding `LastWeaponsUpdate` delta cache and the `WeaponsUpdateFirstTick` first-tick suppression state, classifies both as cache state, and registers their per-run reset plus the reconnect projector under the stable `weapons` lifecycle key.

A mid-round reconnect receives that same live computation only when its session currently holds the Weapons Station. The targeted projection does not read or mutate either publisher cache, so it cannot force a resend or change the next delta seen by another connected player.

## Tactical operator actions

The collapsed weapon-assignment controls preserve the simple primary-target
journey. Each owned Human bank or tube can hold fire, choose a current permitted
contact, follow its prepared group, or return to the primary target. A group
choice sends one correlated command per currently controllable member; it keeps
an individual override while updating the underlying group choice. Delegated
and foreign machinery remains read-only. The browser preserves focused pickers
and drafts across snapshots and never rewrites received assignment state.
`gui/components/weapon-target-controls.js` presents this view;
`gui/stations/weapon-target-actions.js` checks the exact owner and contact before
using the ordinary action map.

Workshop's prepared-group form edits `weapons_console.group` through the accepted
source document and history. `editor/workshop-weapon-groups.js` validates actual
weapon membership, materialises inherited edits explicitly, and writes an empty
replacement when clearing inherited groups. Ordinary System edits refuse
dangling members. This authors groups for a subsequent Preview/Test or load;
it does not regroup installed machinery during play.

The four hull variants share one context-scoped Tactical action catalogue. Target choice, phaser mode/fire, blaster charge/fire/cancel, and torpedo volley/fire controls send the pre-existing console actions through the normal action map and admitted command path. A visible blaster press sends exactly one `ChargeBlasterStart` semantic action (instant banks fire from it; charged banks complete on the host); release sends no legacy alias action. Parameter choices are checked against the live Tactical projection, and unqualified keyboard/gamepad shortcuts choose the same ready bank or tube a visible control would, while bank/tube readiness, ammunition, cooldown, and Combat Lock remain server facts. Correlated Tactical commands receive their terminal feedback only from the owning consumer; target selection never patches the browser state optimistically.

Weapon publishers and reconnect use `console/weapons/readiness.rs` to resolve each
bank's retained assignment through its permitted current observation and actual
mount bearing. A held or unavailable target does not fall through to the primary
lock. `PhaserBankState.firing` reports the actual active beam separately from
post-shot rest, so the existing phaser control exposes Stop only for an owned
manual bank that is still firing.

`gui/sensor-tracks.js` projects acquired Broad shield cover as one observed
incoming world bearing and blocked/open result with the original assessment
tick. Sensors and Tactical display that held ray in their existing contact detail
rows. It supplies no shield health, complete facing map or protected frequency.


## Own launched-round guidance

`ShipGuidanceKnowledge` records only identities actually emitted by this source,
their tube/known launch endurance, local issued orders and heard receipts. It is
separate from actual physical flight. Hidden interception or loss of link cannot
remove a source row or reveal remote health/pose. Pending delayed emissions use
the original public launch command keyed by the first actual emitted ID.

Actual tube System details expose those own launch records plus exact permitted
current XYZ target choices. `guidance_ui.rs` does not enumerate surviving remote
rounds. Shared status controls use correlated exact-tube commands and preserve
focused native choices. Delegated summaries are read-only. A receipt is labelled
heard, with application and survival unknown; it is never labelled accepted.

The existing ordinary tube AI can forward a newer original observation for its
last explicit own target through the same admitted paid receiver handler. It
does not repeatedly pay for the same reading or revive an explicit stop. Human
and AI delivery follow the current flight advance, so a newly heard order can
steer on the next advance. Original sample/provenance ages remain unchanged.
Native receiver/continuation and independent illumination acceptance are tracked
separately from the focused JavaScript controls tests.

## Independent Science illumination

An authored `illumination` System combines an emitter, actual power input and
optional shared turret. Its generic Boolean/EntityTarget channels are the one
intent used by controls, ordinary per-System state tree, mount and save state.
The source must pay continuous work with available equipment and a ready mount
using its own permitted dated XYZ observation. An explicitly opted-in frozen
receiver may accept that reading only when its existing physical target matches
and original age, range and signal path remain valid. Assistance changes no
Tactical selection, payload, launch permission or endurance. Lost emission leaves
only the previous dated point; source feedback reports its own paid interval,
with no receiver count, survival or application result.

The optional `illumination_cruiser.toml` supplies Science equipment and an
empty-target-only request tree. `Illuminate {target}` is an ordinary Sensors
mission/doctrine request subject to the existing reading frontier. Shared
System details expose exact-owner target/enabled controls. Workshop edits the
composed emitter and shared guided flight/seeker/receiver profile through its
normal source validation and document history; explicit `false` disables inherited
seeker/receiver equipment. Already emitted rounds retain their frozen profiles.

Optional `torpedoes.loader_groups` share a finite load-work rate between actual
guided tubes. `weapons/shared_loaders.rs` computes stable priority order and a
fresh interval budget; the existing tube timers, magazine claims and paid work
remain their sole physical owners. Independent tubes preserve their loading
path. The existing magazine claim consumer arbitrates grouped manual and
standing readiness requests together, using actual supply and its ordinary grant
policy. Shared magazine Operation availability limits group progress.

`SetTorpedoLoadPriority` addresses one actual tube. Its own publication carries
priority and handling wait information, without other members' private state.
The Tactical tube surface uses current permitted control authority. Workshop
edits only the actual group leaf through native accepted document transactions;
initial tube priority is available in the guided weapon form. Snapshot tube rows
retain grouped priorities and validate required/forbidden presence; the ordinary
resource-stock fold includes them. Derived waiting receipts clear on restore.
The optional `shared_loader_cruiser.toml` is the reference fit. Native execution
and device acceptance remain separately recorded integration evidence.

Cruiser standard beam impact is authored in `alliance_cruiser.toml` and its
matching default fitting in `fragments/fitting/alliance_cruiser.toml`. Both banks
use the same penetration: lower penetration deposits more damage in the struck
compartment, while higher penetration carries more energy through it. This is
separate from beam DPS. The pulse fitting retains its distinct impact profile.
