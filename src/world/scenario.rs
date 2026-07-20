// Pure scenario-load decision layer (issue #821).
//
// Pure Rust module — no Bevy. `PendingScenarioLoad` queues world TOML paths to
// merge additively into the live `WorldContentRuntime`; the
// `apply_pending_scenario_loads` applier in `world::server` resolves the TOML
// (I/O) and performs the merges, while this module decides what each branch —
// duplicate path, TOML not yet fetched, parse failure, success — means.
//
// Note: this path is currently dormant — no production trigger action enqueues
// into `PendingScenarioLoad` (see the resource's doc comment in
// `world::server`); it is retained as the merge-side plumbing.
//
// The parse + trigger-state derivation core is shared with the layer-load path
// via `world::layers::parse_world_triggers`.

use crate::world::config::WorldConfig;
use crate::world::content::TriggerState;
use crate::world::layers::parse_world_triggers;

/// Decision produced by [`evaluate_scenario_load`] for one queued path.
#[derive(Debug, Default)]
pub struct ScenarioLoadResult {
    /// Trigger states parsed from the scenario TOML for the applier to merge
    /// into the live runtime (empty on any non-success branch).
    pub new_trigger_states: Vec<TriggerState>,
    /// Parsed config for the impure comms merge (`merge_world_comms`).
    pub scenario_config: Option<Box<WorldConfig>>,
    /// TOML not yet available (WASM fetch in flight) — re-queue the path for
    /// the next frame.
    pub requeue: bool,
    /// Record the path in `loaded_scenario_paths`. True on success *and* on
    /// parse failure (a broken file must not be retried), false when skipped
    /// as a duplicate or re-queued.
    pub mark_loaded: bool,
    /// Failure-path messages for the applier to log (`error` level).
    pub warnings: Vec<String>,
}

/// Evaluate one queued scenario path.
///
/// `already_loaded` is the applier's `loaded_scenario_paths.contains` check
/// (done before TOML resolution so a duplicate never touches the WASM fetch
/// queue). `toml_str` is `None` when the TOML is not yet available.
pub fn evaluate_scenario_load(
    path: &str,
    already_loaded: bool,
    toml_str: Option<&str>,
) -> ScenarioLoadResult {
    if already_loaded {
        return ScenarioLoadResult::default();
    }
    let Some(toml_str) = toml_str else {
        return ScenarioLoadResult {
            requeue: true,
            ..Default::default()
        };
    };
    match parse_world_triggers(toml_str) {
        Err(e) => ScenarioLoadResult {
            mark_loaded: true,
            warnings: vec![format!("failed to parse {path}: {e}")],
            ..Default::default()
        },
        Ok((scenario_config, new_trigger_states)) => ScenarioLoadResult {
            new_trigger_states,
            scenario_config: Some(Box::new(scenario_config)),
            mark_loaded: true,
            requeue: false,
            warnings: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENARIO_TOML: &str = r#"
[global]
seed = 7

[[trigger]]
condition = "on_world_loaded"

  [[trigger.action]]
  type = "set_flag"
  name = "scenario_armed"
"#;

    #[test]
    fn duplicate_path_is_skipped_without_marking() {
        let result = evaluate_scenario_load("worlds/s1.toml", true, Some(SCENARIO_TOML));
        assert!(result.new_trigger_states.is_empty());
        assert!(result.scenario_config.is_none());
        assert!(!result.requeue);
        assert!(!result.mark_loaded, "a skipped duplicate is already marked");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn missing_toml_requeues_for_next_frame() {
        let result = evaluate_scenario_load("worlds/s1.toml", false, None);
        assert!(result.requeue);
        assert!(!result.mark_loaded);
        assert!(result.new_trigger_states.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn parse_failure_marks_loaded_and_warns() {
        // A broken file must be marked loaded so it is never retried.
        let result = evaluate_scenario_load("worlds/broken.toml", false, Some("nope ["));
        assert!(result.mark_loaded);
        assert!(!result.requeue);
        assert!(result.new_trigger_states.is_empty());
        assert!(result.scenario_config.is_none());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].starts_with("failed to parse worlds/broken.toml:"));
    }

    #[test]
    fn successful_parse_yields_triggers_and_config() {
        let result = evaluate_scenario_load("worlds/s1.toml", false, Some(SCENARIO_TOML));
        assert_eq!(result.new_trigger_states.len(), 1);
        assert!(result.scenario_config.is_some());
        assert!(result.mark_loaded);
        assert!(!result.requeue);
        assert!(result.warnings.is_empty());
    }
}
