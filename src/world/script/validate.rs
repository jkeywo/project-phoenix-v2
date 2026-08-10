//! Cross-reference validation for loaded scripts (issue #979, milestone M1).
//!
//! Once every unit has run its top level and the defined-function set is known,
//! this pass proves each collected [`Registration`] names a function that
//! actually exists across the content set. Unresolved handlers become
//! [`WorldFinding`] errors on the **existing** authoring-validation channel, so
//! the atomic activation gate (`world::validate::has_error`) blocks a world
//! whose scripts reference functions that were never defined — nothing spawns
//! partially.
//!
//! The defined set is built from `AST::iter_functions()` (see
//! [`load`](super::load)); handler names are ordinary named functions, so name
//! resolution is exact. Anonymous `anon$…` names are irrelevant here — they are
//! never referenced by name — and the M0 spike's cross-file collision caveat
//! does not apply to named handlers.

use std::collections::BTreeSet;

use crate::world::script::engine::{Registration, ScriptTrigger};
use crate::world::validate::{Severity, SourceLocation, WorldFinding};

/// Category slug for a handler that resolves to no defined function.
pub const UNRESOLVED_SCRIPT_FN: &str = "unresolved-script-fn";

/// Category slug for a `[[trigger]]` that specifies BOTH `script = "fn"` and a
/// declarative `[[trigger.action]]` array — the two front-ends are alternatives,
/// never both on one trigger (issue #980, M2).
pub const TRIGGER_SCRIPT_AND_ACTION: &str = "trigger-script-and-action";

/// Category slug for a `[[comms]]` block that specifies BOTH `script = "fn"` and
/// an inline `[[comms.response]]` dialogue tree — the two front-ends are
/// alternatives, never both on one thread (issue #982, M4). The comms analogue of
/// [`TRIGGER_SCRIPT_AND_ACTION`].
pub const COMMS_SCRIPT_AND_RESPONSE: &str = "comms-script-and-response";

fn unresolved_finding(handler: &str, source_path: &str, context: &str) -> WorldFinding {
    WorldFinding {
        severity: Severity::Error,
        category: UNRESOLVED_SCRIPT_FN,
        message: format!("{context} references undefined function '{handler}'"),
        source: SourceLocation {
            file: source_path.to_string(),
            line: None,
            reference: handler.to_string(),
        },
    }
}

/// Prove every registration's handler resolves against `defined_fns`.
///
/// Returns one error finding per unresolved handler, located at the unit that
/// made the registration.
pub fn validate_registrations(
    registrations: &[Registration],
    defined_fns: &BTreeSet<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    for reg in registrations {
        if !defined_fns.contains(&reg.handler) {
            findings.push(unresolved_finding(
                &reg.handler,
                &reg.source_path,
                &format!("script registration for event '{}'", reg.event),
            ));
        }
    }
    findings
}

/// Prove every Rhai-authored trigger's handler (`on_destroyed("x", "fn")`, …)
/// resolves against `defined_fns` (issue #980, M2).
///
/// A trigger whose handler names no defined function is an error finding on the
/// existing authoring-validation channel, so the atomic activation gate
/// (`world::validate::has_error`) blocks a world whose scripted triggers point at
/// functions that were never defined.
pub fn validate_script_triggers(
    script_triggers: &[ScriptTrigger],
    defined_fns: &BTreeSet<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    for st in script_triggers {
        if !defined_fns.contains(&st.handler) {
            findings.push(unresolved_finding(
                &st.handler,
                &st.source_path,
                "scripted trigger",
            ));
        }
    }
    findings
}

/// Cross-reference the TOML `[[trigger]] script = "fn"` front-end (issue #980, M2)
/// against the compiled script's defined-function set.
///
/// Reads the raw world TOML rather than the parsed `WorldConfig` so this pass
/// owns the whole scripted-trigger contract in the script module, beside the Rhai
/// checks, and sees a `[[trigger]]` that carries BOTH front-ends at once (which
/// the parser silently prefers `script` for). Two error rules, both blocking:
///
/// * **both front-ends** — a trigger with `script = "fn"` AND a non-empty
///   `[[trigger.action]]` array. They are alternatives; specifying both is
///   ambiguous authoring, reported as [`TRIGGER_SCRIPT_AND_ACTION`].
/// * **unresolved handler** — a `script = "fn"` naming no defined function,
///   reported as [`UNRESOLVED_SCRIPT_FN`], exactly like a Rhai registration.
///
/// `world_path` locates the findings; a per-trigger line is not derived (the
/// loader does not carry the raw TOML text — the existing `WorldFinding` allows
/// `line: None`).
pub fn validate_toml_script_triggers(
    world_path: &str,
    world_toml: &toml::Value,
    defined_fns: &BTreeSet<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let Some(triggers) = world_toml.get("trigger").and_then(|t| t.as_array()) else {
        return findings;
    };
    for entry in triggers {
        let Some(handler) = entry.get("script").and_then(|v| v.as_str()) else {
            continue;
        };
        let has_actions = entry
            .get("action")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        if has_actions {
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: TRIGGER_SCRIPT_AND_ACTION,
                message: format!(
                    "trigger specifies both script = '{handler}' and a declarative action \
                     array; the two front-ends are alternatives, not both on one trigger"
                ),
                source: SourceLocation {
                    file: world_path.to_string(),
                    line: None,
                    reference: handler.to_string(),
                },
            });
        }
        if !defined_fns.contains(handler) {
            findings.push(unresolved_finding(handler, world_path, "trigger script"));
        }
    }
    findings
}

/// Cross-reference the TOML `[[comms]] script = "fn"` front-end (issue #982, M4)
/// against the compiled script's defined-function set.
///
/// The comms twin of [`validate_toml_script_triggers`]. Reads the raw world TOML
/// rather than the parsed `WorldConfig` so this pass owns the whole scripted-comms
/// contract in the script module, beside the trigger checks, and sees a
/// `[[comms]]` that carries BOTH front-ends at once (which the parser silently
/// prefers `script` for, dropping the response tree). Two error rules, both
/// blocking:
///
/// * **both front-ends** — a block with `script = "fn"` AND a non-empty
///   `[[comms.response]]` array. They are alternatives; specifying both is
///   ambiguous authoring, reported as [`COMMS_SCRIPT_AND_RESPONSE`].
/// * **unresolved root fn** — a `script = "fn"` naming no defined function,
///   reported as [`UNRESOLVED_SCRIPT_FN`], exactly like a scripted trigger.
///
/// `world_path` locates the findings; a per-block line is not derived (the loader
/// does not carry the raw TOML text — the existing `WorldFinding` allows
/// `line: None`).
pub fn validate_toml_script_comms(
    world_path: &str,
    world_toml: &toml::Value,
    defined_fns: &BTreeSet<String>,
) -> Vec<WorldFinding> {
    let mut findings = Vec::new();
    let Some(entries) = world_toml.get("comms").and_then(|t| t.as_array()) else {
        return findings;
    };
    for entry in entries {
        let Some(root_fn) = entry.get("script").and_then(|v| v.as_str()) else {
            continue;
        };
        let has_responses = entry
            .get("response")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        if has_responses {
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: COMMS_SCRIPT_AND_RESPONSE,
                message: format!(
                    "comms thread specifies both script = '{root_fn}' and an inline \
                     [[comms.response]] dialogue tree; the two front-ends are alternatives, \
                     not both on one thread"
                ),
                source: SourceLocation {
                    file: world_path.to_string(),
                    line: None,
                    reference: root_fn.to_string(),
                },
            });
        }
        if !defined_fns.contains(root_fn) {
            findings.push(unresolved_finding(root_fn, world_path, "comms script"));
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(event: &str, handler: &str, path: &str) -> Registration {
        Registration {
            event: event.to_string(),
            handler: handler.to_string(),
            source_path: path.to_string(),
        }
    }

    fn defined(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_resolved_handler_produces_no_finding() {
        let regs = vec![reg("flag_set:armed", "handle_armed", "w.toml#script.s")];
        let findings = validate_registrations(&regs, &defined(&["handle_armed"]));
        assert!(findings.is_empty());
    }

    #[test]
    fn an_unresolved_handler_is_an_error_finding() {
        let regs = vec![reg("flag_set:armed", "handle_armed", "w.toml#script.s")];
        let findings = validate_registrations(&regs, &defined(&["something_else"]));
        assert_eq!(findings.len(), 1);
        let f = &findings[0];
        assert!(f.is_error());
        assert_eq!(f.category, UNRESOLVED_SCRIPT_FN);
        assert_eq!(f.source.file, "w.toml#script.s");
        assert_eq!(f.source.reference, "handle_armed");
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn each_unresolved_handler_reports_separately() {
        let regs = vec![
            reg("a", "handler_a", "p1"),
            reg("b", "handler_b", "p2"),
            reg("c", "handler_c", "p3"),
        ];
        // Only `handler_b` is defined.
        let findings = validate_registrations(&regs, &defined(&["handler_b"]));
        assert_eq!(findings.len(), 2);
    }

    // ── Rhai trigger front-end handler resolution (issue #980) ────────────────

    fn script_trigger(handler: &str, path: &str) -> ScriptTrigger {
        ScriptTrigger {
            trigger: crate::world::config::scripted_trigger(
                crate::world::config::TriggerCondition::OnWorldLoaded,
            ),
            handler: handler.to_string(),
            source_path: path.to_string(),
        }
    }

    #[test]
    fn a_resolved_scripted_trigger_handler_is_clean() {
        let sts = vec![script_trigger("on_loaded", "w.toml#script.s")];
        assert!(validate_script_triggers(&sts, &defined(&["on_loaded"])).is_empty());
    }

    #[test]
    fn an_unresolved_scripted_trigger_handler_blocks_activation() {
        let sts = vec![script_trigger("missing", "w.toml#script.s")];
        let findings = validate_script_triggers(&sts, &defined(&["something_else"]));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, UNRESOLVED_SCRIPT_FN);
        assert_eq!(findings[0].source.reference, "missing");
        assert!(crate::world::validate::has_error(&findings));
    }

    // ── TOML `[[trigger]] script = "fn"` front-end (issue #980) ───────────────

    fn world(toml_str: &str) -> toml::Value {
        toml::from_str(toml_str).expect("valid toml")
    }

    #[test]
    fn a_resolved_toml_script_trigger_is_clean() {
        let w = world(
            r#"
            [[trigger]]
            condition = "on_destroyed"
            entity = "raider"
            script = "on_raider_dead"
            "#,
        );
        let findings = validate_toml_script_triggers("w.toml", &w, &defined(&["on_raider_dead"]));
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn an_unresolved_toml_script_trigger_is_an_error() {
        let w = world(
            r#"
            [[trigger]]
            condition = "on_world_loaded"
            script = "never_defined"
            "#,
        );
        let findings = validate_toml_script_triggers("w.toml", &w, &defined(&["other"]));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, UNRESOLVED_SCRIPT_FN);
        assert_eq!(findings[0].source.reference, "never_defined");
        assert_eq!(findings[0].source.file, "w.toml");
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn a_trigger_with_both_script_and_actions_is_rejected() {
        // The handler IS defined, so the ONLY finding is the both-front-ends one.
        let w = world(
            r#"
            [[trigger]]
            condition = "on_destroyed"
            entity = "raider"
            script = "on_raider_dead"

            [[trigger.action]]
            type = "complete_objective"
            id = "obj-x"
            "#,
        );
        let findings = validate_toml_script_triggers("w.toml", &w, &defined(&["on_raider_dead"]));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, TRIGGER_SCRIPT_AND_ACTION);
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn a_declarative_trigger_yields_no_script_findings() {
        // No `script` key: this front-end must not touch it at all.
        let w = world(
            r#"
            [[trigger]]
            condition = "on_destroyed"
            entity = "raider"

            [[trigger.action]]
            type = "complete_objective"
            id = "obj-x"
            "#,
        );
        assert!(validate_toml_script_triggers("w.toml", &w, &defined(&[])).is_empty());
    }

    // ── TOML `[[comms]] script = "fn"` front-end (issue #982, M4) ─────────────

    #[test]
    fn a_resolved_toml_script_comms_is_clean() {
        let w = world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            script = "hail_axiom"
            "#,
        );
        let findings = validate_toml_script_comms("w.toml", &w, &defined(&["hail_axiom"]));
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn an_unresolved_toml_script_comms_is_an_error() {
        let w = world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            script = "never_defined"
            "#,
        );
        let findings = validate_toml_script_comms("w.toml", &w, &defined(&["other"]));
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].category, UNRESOLVED_SCRIPT_FN);
        assert_eq!(findings[0].source.reference, "never_defined");
        assert_eq!(findings[0].source.file, "w.toml");
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn a_comms_with_both_script_and_responses_is_rejected() {
        // The root fn IS defined, so the ONLY finding is the both-front-ends one.
        let w = world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            script = "hail_axiom"

            [[comms.response]]
            text = "Acknowledge"
            [[comms.response.action]]
            type = "complete_objective"
            id = "obj-x"
            "#,
        );
        let findings = validate_toml_script_comms("w.toml", &w, &defined(&["hail_axiom"]));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].category, COMMS_SCRIPT_AND_RESPONSE);
        assert!(crate::world::validate::has_error(&findings));
    }

    #[test]
    fn a_declarative_comms_yields_no_script_findings() {
        // No `script` key: this front-end must not touch a plain TOML comms block.
        let w = world(
            r#"
            [[comms]]
            from = "axiom"
            trigger = "on_hailed"
            entity = "axiom"
            message = "Go ahead."

            [[comms.response]]
            text = "Acknowledge"
            [[comms.response.action]]
            type = "complete_objective"
            id = "obj-x"
            "#,
        );
        assert!(validate_toml_script_comms("w.toml", &w, &defined(&[])).is_empty());
    }
}
