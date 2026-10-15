---
title: Helm Runtime
type: concept
tags: [helm, ai, impulse, boost, steering]
sources: [crates/phoenix-simulation/src/console/helm/navigation.rs, crates/phoenix-simulation/src/console/helm/control.rs, crates/phoenix-simulation/src/console/navigation/knowledge.rs, crates/phoenix-sim-gameplay/src/ship/helm_control.rs, crates/phoenix-simulation/src/console/helm/transit_worlds.rs, crates/phoenix-simulation/src/console/helm/transit.rs, crates/phoenix-simulation/src/snapshot/transit.rs, crates/phoenix-sim-gameplay/src/ship/transit.rs, crates/phoenix-simulation/src/ship/continuation.rs, crates/phoenix-sim-gameplay/src/ship/physics_systems.rs, crates/phoenix-sim-gameplay/src/ship/impulse_boost_systems.rs, crates/phoenix-simulation/src/server_app/components.rs, crates/phoenix-simulation/src/snapshot/mod.rs, tests/snapshot_resume.rs, tests/lockstep_six_peer.rs, crates/phoenix-sim-gameplay/src/ship/helm_admission.rs, crates/phoenix-sim-gameplay/src/ship/helm.rs, crates/phoenix-sim-gameplay/src/ai/core.rs, crates/phoenix-simulation/src/ai/server.rs, crates/phoenix-simulation/src/console/helm/server.rs, gui/action-map.js, gui/console-state.js]
updated: 2026-10-16
---

# Helm Runtime

Human Helm sends admitted per-axis `ControlSystem` commands to actual authored System owners. Hulls with an explicit `helm_control` profile retain one `HelmControlState` map in Gameplay. The Simulation `console/helm/control.rs` applier consumes their mode, scalar, desired speed/heading and Match Speed commands in admitted order. The legacy Gameplay applier skips these migrated scalar axes and continues to own other controls, including impulse and boost. A successful impulse start clears the migrated thrust and steering intent in the same command order.

Direct thrust applies paid force and neutral input coasts. Direct steering preserves world-planar momentum while heading changes. Assisted control retains the prior normalized speed behavior, with explicit desired speed and heading alternatives. Match Speed stores only a public contact handle, speed and original observation tick from the consuming Console's current permitted observation. Loss retains that dated speed; throttle or explicit cancellation ends matching. Each axis retains its own authority and mode. Ordinary AI requests Assisted through the same admitted command before normalized demands, including on custom System IDs.

The sole physics integrator consumes the retained map using actual local grants and Operation condition. Raw scalar input fields on migrated axes are intent-only display projections, rather than another saved servo setpoint. Snapshot capture, whole-candidate owner/clock/date validation and digest fold include the map independently of LOD command-component removal. An authored `helm_control = false` disables inherited bindings; enabled profiles replace as a whole so an omitted lateral owner cannot survive from an include.

The shared phone movement widget and dedicated Workshop movement form use these existing command and accepted-source paths. They expose independently owned axes and dated Match readings; their client tests do not establish full-host movement or human acceptance.

Boost already uses retained engagement and actual local reserve work in the sole integrator. Its old duration fields do not time the physical boost path; `tick_boost` projects the owning reserve rather than advancing another battery. Energy and thermal limits come from that existing consumer. The movement contract preserves this owner and tuning while correcting phone re-engagement and exact equipment availability; see [movement PASM](../../pasm/spec/design/movement-transit.yaml).

Helm AI executes on the host as four per-axis systems (`ai_helm_thrust`, `ai_helm_steering`, `ai_helm_lateral_thrust`, `ai_helm_impulse`), each deciding its own axis from the shared `HelmAiSurfacesFrame` (built once per AI tick) and emitting admitted commands through `command_admission::ai_emit::emit_ai_command` (the shared AI-emit helper over `validate_and_admit`), gated on its own axis's `ControlSource`, all sharing one fixed-rate sim tick (`[global] ai_tick_hz`, issues #803/#889). Each calls the pure `operate_helm` / `operate_lateral_thrust` over console-owned surfaces (Tactical's target, Navigation's waypoint, the objective cursors). Weapons can issue an arc-bearing request that `ai_helm_steering` consumes while the target remains valid. A single integrator, `integrate_ship_physics`, applies the intent components to `ShipPhysics` for human and AI ships alike. See [AI Helm Decomposition](./ai-helm-decomposition.md).

Starting impulse charge clears the admitted thrust and steering latches on every ship, including remote fleet ships and NPCs. Only the `LastHelmInput` display cache is local. A later axis command in the same admitted buffer still wins; keeping stale remote AI steering during charge would split fleet trajectories (#1449).

The console publishes authoritative position, yaw, speeds, impulse, boost, radar, and engine state through Helm blackboards. The client only renders that state and sends controls.

The ship-scoped capability model, shared 3D planning and hazard surface, vertical movement modes, and player/AI actuator convergence are design decisions recorded in [PASM's Helm slice](../../pasm/spec/design/helm-controls.yaml).

Saved Helm and Radar continuation lives in `crates/phoenix-simulation/src/ship/continuation.rs`.
`ControlState::capture_from` and `restore_into` own desired axes, last-applied
inputs, impulse encoding, policy/recovery history and target memory. Snapshot
keeps its UUID joins and restore order, and re-exports the unchanged DTO paths.
Absent Helm components stay absent; saved neutral inputs and cleared targets
replace a different bootstrap. The attacker latch only changes when its value
changes. Owner tests drive the first physics action, and `tests/snapshot_resume.rs`
compares fresh-app continuation through subsequent simulation actions.

Per-engine intent indicators read that same canonical `LastHelmInput` as the
ordinary joystick fan-out. They do not read stale messages left in the ephemeral
InterSystemQueue by bootstrap when rebuilding a restored Console view.

Impulse hull-damage cancellation samples every ship's HP in Input.
`ImpulseHullHistory` is required by `ShipImpulse` and travels in the snapshot's
`DriveState`: a restored damaged hull neither inherits the bootstrap's HP nor
loses a hit applied after the last Input sample. The detector still writes an
Idle intent before admitted Helm commands, preserving their override order.

`DriveCommandWrites` marks actual drive requests until the shared consumer
runs. This lets a real damage cancel or admitted start apply on a newly
restored/LOD-added command component while an untouched default stays inert.
The consumer and snapshot restore clear the transient marker. Restore also
rebases the impulse/boost commands' change markers to an already-consumed tick.
Their saved values must not trigger the consumer's direct-write fallback:
an exhausted boost can retain a true command while its battery recharges, and
restoring that command as Changed would re-engage it on the next tick. The
six-peer recovery regression keeps boost on through exhaustion before restoring
the divergent GM and compares subsequent digest checkpoints.

Boost work is paid in `crates/phoenix-sim-gameplay/src/ship/physics_systems.rs::integrate_ship_physics` for every actual ship. The exact accepted Boost owner supplies its source, Operation and local reserve/thermal limits. `tick_boost` only projects that reserve into the compatibility battery field. `BoostActuation` is a required derived last-applied fraction written at integration and cleared on restore; it is neither engagement intent nor saved work credit. The phone can re-engage with partial reserve, and distinguishes retained engagement from current availability and the last applied grant. Existing individual Boost AI policies and normal admission use the same actual owner, including custom IDs. Owner regressions live in `crates/phoenix-simulation/src/console/helm/boost_control_tests.rs`; full-host and human acceptance remain separate.

LocalJump is an optional actual `transit-drive` System component. Its ordinary controller and human commands share the permitted Navigation course and the same per-ship `ShipTransitDrives` map. A Begin freezes the dated course, cost and fuel requirement; actual Power and managed Stock pay preparation and commitment before a single next-tick `ShipPhysics` arrival. Only one drive on that ship may be Preparing or Committed. Completed drive histories remain independent, including handled explicit request revisions. A waypoint alone does not order an AI jump.

Private deterministic scatter and accepted static gravity inputs stay in the order's continuation receipt. Dedicated typed Transit publications expose only accepted own course, phase, progress and actual arrival through the exact Console view. Candidate and single-row restore validate the same accepted drive roster, clock, active-order limit and placement receipt before writes. `local_jump_cruiser.toml` supplies explicit finite fuel and the existing Logistics replenishment route.

An optional `world_travel` drive uses the same paid work and physical arrival. Navigation publishes closed authored routes through its existing permitted board; the existing waypoint command selects a unique x/z anchor, and the drive freezes its complete authored point, including height. `transit_worlds.rs` rebuilds immutable accepted route, loader ancestry and static-body recipes at ordinary world materialisation. The existing World layer queue activates the selected destination and support parent-first. Source layers remain resident; a return reuses their actors and scripts.

`ShipTransitDrives` also retains the actual current arrival-policy World and latest successful World-arrival stamp. Every order freezes its original World so completed histories on other drives remain valid after later travel; subsequent local jumps use the actual current scope. `EntityOriginLayer` keeps its separate actor lifetime and script ownership. Unsupported or ambiguous layer-authored drive origins refuse before minting. `sector_jump_cruiser.toml` is an optional configured consumer. The sector continuation witness is in `tests/local_jump_continuation.rs`; source and client checks do not establish its unexecuted native result. Formation remains separate work.


Formation selection lives on the existing Navigation course. Genuine received
Comms acceptance is separate from selecting an earned contact alias, and neither
step grants Helm control. Each configured Helm axis has an explicit Follow
Navigation HUD action and retains only the selected semantic generation in its
existing intent. `console/helm/navigation.rs` consumes its permitted Navigation
publication's dated motion; it never correlates another radar's alias or reads
the leader's body. Original sample age and actual accepted instrument cadence
bound availability. A changed or withdrawn course suppresses old assistance;
manual axis input replaces only that axis's demand.

`HelmNavigationInputs` is a current-tick derived context reset at restore. The
same existing paid physics resolver/integrator consumes it. The ordinary travel
AI verbs can request following; docking, hazard, combat/pass and Weapons-bearing
choices retain priority. The existing held AI motion plan saves its optional
Navigation generation and folds it in `helm-navigation-plan` only when present.
Owner witnesses cover genuine received coordination, paid observation and
selection, fresh-host candidate validation, and the new paid movement/freshness
and manual-breakaway continuation. The new Helm following Rust witnesses await
native execution; browser/physical fleet acceptance remains separate.
