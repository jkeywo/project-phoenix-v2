---
phase: code-review
reviewed: 2026-05-16T18:30:00Z
depth: standard
files_reviewed: 20
files_reviewed_list:
  - src/core/messages.rs
  - src/lobby/server.rs
  - src/lobby/handler.rs
  - src/server_app.rs
  - src/server/renderer.rs
  - src/server/viewscreen_border.rs
  - src/core/broadcast/lobby.rs
  - src/core/broadcast/sim.rs
  - src/world/server.rs
  - src/console/captain/server.rs
  - src/console/weapons/server.rs
  - src/console/helm/client.rs
  - src/console/science/client.rs
  - src/client/app.rs
  - src/client_sim.rs
  - src/console_ai/server.rs
  - src/ai/server.rs
  - src/ship_state.rs
  - src/console/power/plugin.rs
  - Cargo.toml
findings:
  critical: 0
  warning: 0
  info: 1
  total: 1
status: clean
---

# Phase: Code Review Report

**Reviewed:** 2026-05-16T18:30:00Z  
**Depth:** standard  
**Files Reviewed:** 20  
**Status:** clean (issues found: 1 info-level)

## Summary

Reviewed Issue #269 "Migrate phase gating to `States<GamePhase>`" against all 10 acceptance criteria. The migration is complete and correct. The old `CurrentPhase` resource struct has been eliminated, all `phase.0` direct-access patterns have been replaced with `State<GamePhase>` / `NextState<GamePhase>` reads and writes, and the SimSet chain is properly gated by `.run_if(in_state(GamePhase::InProgress))`. A single minor issue was found: a stale doc comment referencing the old resource name.

## Acceptance Criteria Verification

### AC-1: `GamePhase` derives `States, Hash, Eq, Clone, Debug, Default` ✅

**File:** `src/core/messages.rs:238-243`

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, Default, States)]
pub enum GamePhase {
    #[default]
    Lobby,
    InProgress,
}
```

All required derives are present. The enum is properly `#[default]`-annotated on `Lobby`, matching Bevy's `States` trait contract.

### AC-2: `app.init_state::<GamePhase>()` registered ✅

**File:** `src/lobby/server.rs:70-71`

The code uses the equivalent manual pattern:

```rust
.insert_resource(State::new(GamePhase::Lobby))
.insert_resource(NextState::<GamePhase>::Unchanged)
```

This is functionally identical to `init_state::<GamePhase>()` — both insert the `State<T>` and `NextState<T>` resources. Bevy's built-in state transition system (active via `App::new()`) handles the transition from `NextState::Set(T)` → `State(T)` each frame.

### AC-3: `CurrentPhase` resource struct removed ✅

**Project-wide grep for `CurrentPhase` (excluding `CurrentPhaserMode`):**

- No `struct CurrentPhase` exists anywhere in `src/`
- No resource registration of `CurrentPhase` exists
- All 13 grep matches are either `CurrentPhaserMode` (a different resource for phaser mode) or a stale comment (see Info section)
- The `Phase` field in `lobby/handler.rs` `derive_game_state` now takes `&GamePhase` directly

### AC-4: `phase.0 !=` guards removed ✅

**Project-wide grep for `phase.0 !=` returned ZERO results.**

All phase comparisons now use `state.get() != &GamePhase::Xxx` (via `Res<State<GamePhase>>`).

### AC-5: Lobby-phase guard uses `State<GamePhase>` ✅

**File:** `src/lobby/server.rs:119`

```rust
if state.get() != &GamePhase::Lobby {
    return;
}
```

The guard is still present (it must be, since `process_lobby` runs unconditionally in Update), but it now uses `State<GamePhase>` via `Res<State<GamePhase>>` parameter, not the old `CurrentPhase.0` pattern.

### AC-6: `.run_if(in_state(GamePhase::InProgress))` on SimSet chain ✅

**File:** `src/server_app.rs:140-146`

```rust
app.configure_sets(Update, (
    crate::sim_sets::SimSet::Input,
    crate::sim_sets::SimSet::Physics,
    crate::sim_sets::SimSet::Damage,
    crate::sim_sets::SimSet::Modifiers,
    crate::sim_sets::SimSet::Broadcast,
).chain().run_if(in_state(GamePhase::InProgress)).after(crate::lobby::process_lobby))
```

The SimSet chain is properly gated by `in_state(GamePhase::InProgress)`, preventing all simulation systems from running during the Lobby phase.

### AC-7: `phase.0 =` replaced with `NextState<GamePhase>` ✅

**Project-wide grep for `phase.0 =` returned ZERO results.**

All phase transitions use `next_state.set(new_phase)` in `apply_result()` (`src/lobby/server.rs:161`):

```rust
fn apply_result(
    result: lobby_handler::LobbyHandlerResult,
    outbox: &mut ResMut<LobbyOutbox>,
    next_state: &mut ResMut<NextState<GamePhase>>,
) {
    if let Some(new_phase) = result.new_phase {
        next_state.set(new_phase);
    }
    outbox.0.extend(result.outbound);
}
```

### AC-8: `OnEnter(GamePhase::InProgress)` for start-of-game systems ✅

**File:** `src/server_app.rs:169-172`

```rust
.add_systems(OnEnter(GamePhase::InProgress), (
    spawn_game_start_entities,
    render_spawned_entities,
))
```

Both systems fire exactly once when the state transitions from `Lobby` to `InProgress`.

### AC-9: `cargo test` passes ✅

Already verified by user. The 1600+ test suite compiles and passes.

### AC-10: Systems not redundantly gated ✅

Systems within SimSet do **not** have redundant manual phase guards (e.g., no `if phase.0 != GamePhase::InProgress { return; }` inside systems in the SimSet chain). The chain-level `.run_if` is the single gate.

Additional renderer systems also use `in_state(GamePhase::InProgress)` where appropriate:
- `src/server/renderer.rs:91-99` — `hull_camera`, `draw_radar_overlay`, `draw_beam_vfx`, `tick_ripples`, `sync_torpedo_entities`, `draw_warp_exit_markers` all use `.run_if(in_state(GamePhase::InProgress))`

## Info

### IN-01: Stale comment referencing `CurrentPhase`

**File:** `src/core/broadcast/sim.rs:207`

```rust
/// - `CurrentPhase` is set to `InProgress` so the dispatch gate passes.
```

This doc comment in the `dispatch_app` test helper still references the old `CurrentPhase` resource name. Since the old gating was removed from `dispatch_sim_broadcasts` (as noted in the comment on line 223: "No State<GamePhase> setup needed for these tests"), the comment is misleading. The current `SimBroadcaster` dispatch no longer gates on game phase at all — that responsibility moved to the SimSet chain.

**Fix:** Update the comment to: `/// Game phase gating is handled by the SimSet chain — not needed here.`

---

_Reviewed: 2026-05-16T18:30:00Z_  
_Reviewer: the agent (gsd-code-reviewer)_  
_Depth: standard_
