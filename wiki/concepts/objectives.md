---
title: Objectives
type: concept
tags: [world, objectives, ai, captain, gui, authoring, gm, activity]
sources: [src/gm_objective.rs, src/snapshot.rs, src/sim_digest.rs, src/objectives.rs, src/objectives/directive.rs, src/entities/config.rs, src/world/config.rs, src/world/script/effects.rs, src/world/server.rs, src/world/dispatch.rs, src/core/balance.rs, src/gm_activity.rs, src/console/comms/server.rs, src/console/captain/server.rs, src/console/weapons/torpedo.rs, src/console/weapons/blackboard.rs, src/server/radar.rs, src/gui/radar.rs, src/gm_projection.rs, gui/console-state.js, src/ship/helm_ai/mod.rs, src/ship/helm_ai/impulse.rs, src/ai/core.rs, assets/worlds/combat_test.toml]
updated: 2026-09-07
---

# Objectives

World triggers, comms responses and authored GM palette activation create mission objectives. Each carries player text, status, targets, an optional AI directive, utility configuration, and a `Mission` or `Doctrine` source.

## Flow

1. Entity `[[behaviour.doctrine]]` entries and World/scripted `add_objective` actions project their existing fields into `objectives::directive::AuthoredDirective`.
2. The shared contract validates kind, field ownership and requirements, applies established defaults, and performs the only conversion to `AiDirective`.
3. `add_objective`, `complete_objective`, and `fail_objective` actions mutate the session-lifetime `ObjectiveManager`.
4. Each actual Active, Completed, or Failed transition emits an unconditional
   lifecycle fact with its stable id and authored targets. The shared action
   dispatcher and the independent Helm-AI Reach completion path use the same
   fact; idempotent repeats and layer-unload removal emit nothing. The bounded
   local GM activity feed projects these facts without entering peer transport.
5. Active objectives are utility-scored from base priority, mandatory bonus, world conditions, modifiers, zero gates, and an optional Captain boost.
6. Each ship's viewscreen blackboard carries its scored pool. Backfill Helm,
   Weapons/Tactical, Comms, Navigation, Sensors, Engineering, and Repair consume
   positive directives selected for their `SystemAffinity`.
7. Captain and Comms apply the same player-facing visibility rule: mission
   objectives remain visible at any score, while doctrine objectives appear
   only at positive utility. Ship-specific GUI objective lists render those
   projections.

## GM activation and recipient scope

The mission panel activates only `[[gm_objective_palette]]` entries. Each row
reuses normal Objective parsing, including directive, utility, targets and text
parameters. Complete and fail operate only on an active retained Objective.
The canonical action retains its id, verb, operator and exact recipient scope;
same-correlation repeats do not create another lifecycle transition.
Palette identities and recipient lists use the same byte, character and count
bounds as the canonical action. Resolved names and retained records are checked
again before the panel advertises an available control.

Recipients are immutable ship UUIDs resolved from authored names at activation.
They are distinct from subject `targets`, which still identify contacts and
markers. An empty recipient list keeps legacy all-ship visibility. A scoped
Objective still has one status, resolved for its whole intended scope; Captain,
Comms, AI scoring, Command stances and GM crew replicas filter it by ship.
Missing recipients refuse activation rather than widening it to all ships.

The shared `WorldData` Objective target bit is a global lifecycle cache hint.
`objectives::project_entity_targets` replaces it at the viewscreen, Tactical and
GM crew boundaries from each recipient's current Objective list, without changing
shared or folded state. The shared JavaScript radar and region builders do the
same for initial, spawned and reconnect metadata, so empty or terminal lists
remove stale annotations and hide unassigned synthetic Objective markers.
The native radar renderer takes its centre, rendered sources and auto-fit
inputs only from the current widget's bridge children. Hidden widgets may
retain older sources, but cannot reintroduce their Objective markers, regions,
labels or rings after the active projection completes or removes an Objective.

Normal scripts and GM controls call `gm_objective::apply_command`, preserving
ordinary BalanceEvent facts and narrative transitions. Runtime layers retain
palette provenance and ownership, so unloading removes their rows and Objectives.
Snapshot format 28 preserves complete records in insertion order; digest folds
those records and restore replaces them without replaying fictional transitions.
Each active layer's ordered Objective ownership list also travels with its
captured flags and contributes to the digest, so a later unload retracts the same
records, directives and stances after restore. Operational activity rows attach
the recorded recipients as semantic ship references and Ship links; cached names
or UUID fallbacks keep those rows addressable by the activity feed's ship filter.

## Directive authoring contract

`DirectiveKind` owns the complete vocabulary: `None`, `Patrol`, `Destroy`,
`Reach`, `Retreat`, `Hail`, `Scan`, `Dock`, `Tow`, `Stabilise`, `Escort`,
`Transfer`, `FieldRepair`, and `Order`. The authoring surfaces deliberately keep
their existing shapes. World actions share `target` and `route`; doctrine keeps
its dedicated `directive_target`, `directive_hail_target`,
`directive_scan_target`, `directive_dock_target`, `directive_operate_target`,
`directive_order_target`, and `directive_order_route`. Their adapters only map
those names onto canonical Anchors, Loop, Target, Anchor, and Route slots.

The shared interpreter rejects missing required values, whitespace-empty
scalar values, blank elements inside a nonempty text list, fields owned by
another kind, unknown kinds, and unknown keys. World
TOML captures unknown action keys during deserialization, while the Rhai
`add_objective(#{ ... })` adapter carries unknown map keys into the same
contract; neither surface can silently discard a Directive typo.

`Reach` and `Retreat` require an anchor. `Hail`, `Scan`, `Dock`, and the five
operate directives require a target; `Order` requires both target and route.
Two historical defaults remain behavioral truth: an untargeted `Destroy` asks
target selection to choose a visible hostile, and a Patrol with no anchors and
no loop resolves to a hold, including an explicitly empty `[]` anchor list. A
nonempty list containing a blank anchor is malformed rather than a hold. Runtime
scoring, name-to-UUID target resolution, and per-system behavioral matches
remain downstream. A runtime-created doctrine entry that bypasses entity-load
validation is rejected by the same interpreter and omitted from scoring;
malformed data cannot panic the authoritative tick. Other runtime consumers do
not reopen the raw authoring catalogue: the Helm impulse default receives the
already-scored typed Directive, and torpedo conservation canonically parses
doctrine before counting only valid, nonblank targeted `Destroy` directives.

## Runtime ownership and visibility

- `CaptainPriorityBoost` is keyed by ship scope. A Captain's selection reorders
  that ship's objective consumers only.
- Captain and Comms both use `is_visible_objective`; neither exposes zero-score
  doctrine while both retain mission objectives.
- GM palette parsing rejects duplicate ids and malformed authored directives;
  incoming layers with colliding palette ids refuse activation. The older
  declarative action-list reference walker is empty after the Rhai migration;
  it does not validate arbitrary scripted Objective declarations.
- A loaded world layer owns the objectives it authored. Unloading it removes
  those objectives plus priority and route-cursor state that names them.
- Backfill Comms consumes positive `Hail` directives and emits the same admitted
  Comms action used by a player.
