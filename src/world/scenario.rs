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
// Since issue #985 a merged scenario contributes NOTHING but the record that it
// was loaded: the `[[trigger]]` and `[[comms]]` blocks it used to merge no
// longer parse (a world that still authors either is refused by `parse_world`),
// and a layer's `[[entity]]` blocks travel through `world::layers`, not here.
// The dedup / requeue / parse-failure decisions are kept because they are the
// merge-side plumbing script-in-layers (#1045) plugs into.

use crate::world::config::parse_world;

/// Decision produced by [`evaluate_scenario_load`] for one queued path.
#[derive(Debug, Default)]
pub struct ScenarioLoadResult {
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
    match parse_world(toml_str) {
        Err(e) => ScenarioLoadResult {
            mark_loaded: true,
            warnings: vec![format!("failed to parse {path}: {e}")],
            ..Default::default()
        },
        Ok(_) => ScenarioLoadResult {
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
"#;

    #[test]
    fn duplicate_path_is_skipped_without_marking() {
        let result = evaluate_scenario_load("worlds/s1.toml", true, Some(SCENARIO_TOML));
        assert!(!result.requeue);
        assert!(!result.mark_loaded, "a skipped duplicate is already marked");
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn missing_toml_requeues_for_next_frame() {
        let result = evaluate_scenario_load("worlds/s1.toml", false, None);
        assert!(result.requeue);
        assert!(!result.mark_loaded);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn parse_failure_marks_loaded_and_warns() {
        // A broken file must be marked loaded so it is never retried.
        let result = evaluate_scenario_load("worlds/broken.toml", false, Some("nope ["));
        assert!(result.mark_loaded);
        assert!(!result.requeue);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].starts_with("failed to parse worlds/broken.toml:"));
    }

    #[test]
    fn successful_parse_marks_loaded_without_warnings() {
        let result = evaluate_scenario_load("worlds/s1.toml", false, Some(SCENARIO_TOML));
        assert!(result.mark_loaded);
        assert!(!result.requeue);
        assert!(result.warnings.is_empty());
    }
}
