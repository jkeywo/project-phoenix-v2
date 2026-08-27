---
title: Objectives
type: concept
tags: [world, objectives, ai, captain, gui, authoring]
sources: [src/objectives.rs, src/objectives/directive.rs, src/entities/config.rs, src/world/config.rs, src/world/script/effects.rs, src/world/server.rs, src/world/dispatch.rs, src/console/comms/server.rs, src/console/captain/server.rs, src/console/weapons/torpedo.rs, src/ship/helm_ai/impulse.rs, src/ai/core.rs, assets/worlds/combat_test.toml]
updated: 2026-08-28
---

# Objectives

World triggers and comms responses create mission objectives. Each carries player text, status, targets, an optional AI directive, utility configuration, and a `Mission` or `Doctrine` source.

## Flow

1. Entity `[[behaviour.doctrine]]` entries and World/scripted `add_objective` actions project their existing fields into `objectives::directive::AuthoredDirective`.
2. The shared contract validates kind, field ownership and requirements, applies established defaults, and performs the only conversion to `AiDirective`.
3. `add_objective`, `complete_objective`, and `fail_objective` actions mutate the session-lifetime `ObjectiveManager`.
4. Active objectives are utility-scored from base priority, mandatory bonus, world conditions, modifiers, zero gates, and an optional Captain boost.
5. Each ship's viewscreen blackboard carries its scored pool. Backfill Helm,
   Weapons/Tactical, Comms, Navigation, Sensors, Engineering, and Repair consume
   positive directives selected for their `SystemAffinity`.
6. Captain and Comms apply the same player-facing visibility rule: mission
   objectives remain visible at any score, while doctrine objectives appear
   only at positive utility. Ship-specific GUI objective lists render those
   projections.

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
- World composition validation rejects duplicate objective declarations and
  complete/fail references with no matching declaration before activation.
- A loaded world layer owns the objectives it authored. Unloading it removes
  those objectives plus priority and route-cursor state that names them.
- Backfill Comms consumes positive `Hail` directives and emits the same admitted
  Comms action used by a player.
