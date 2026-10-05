// Pure Rust module for managing mission objectives.
// No Bevy dependency. Owns all objective state for the running simulation.
//
// PRD #342: legacy multi-scenario layering is gone. Objectives live for the
// duration of the session. Completed/failed objectives are retained until
// explicitly cleared.
//
// The public surface is intentionally narrow:
//   - `ObjectiveManager::add` — register a new active objective (backward compat)
//   - `ObjectiveManager::add_full` — register with directive + utility config
//   - `ObjectiveManager::add_full_with_params` — the above plus a table of
//     runtime values to interpolate into the objective's text

//   - `ObjectiveManager::complete` — transition active → completed
//   - `ObjectiveManager::fail` — transition active → failed
//   - `ObjectiveManager::sorted_snapshots` — sorted view (mandatory first)
//   - `ObjectiveManager::scored_pool` — utility-scored pool for AI (issue #571)
//   - `ObjectiveManager::is_dirty` / `ObjectiveManager::mark_clean` — change tracking
//     so callers can push `ObjectiveSummary` only on change
//   - `ObjectiveManager::drain_transitions` — the ordered per-tick log of every
//     mutation the mutators above made, for the mission-timeline recorder (#1338)

/// Canonical authoring vocabulary and validation for objective Directives.
/// Entity doctrine and World actions keep their existing TOML field names, but
/// both adapt into this one typed contract before a runtime `AiDirective` is
/// built (issue #1268).
pub mod directive;

// ── Utility scoring types ──────────────────────────────────────────────────

// ── Internal record ────────────────────────────────────────────────────────

// ── Transition log (issue #1338) ───────────────────────────────────────────

// ── Manager ────────────────────────────────────────────────────────────────

// ── Unit Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "objectives_tests.rs"]
mod tests;

pub use phoenix_sim_contracts::objective_utility::*;

pub use phoenix_sim_world::objectives::*;

pub use phoenix_sim_gameplay::objectives::*;
