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

use crate::world::script::engine::Registration;
use crate::world::validate::{Severity, SourceLocation, WorldFinding};

/// Category slug for a handler that resolves to no defined function.
pub const UNRESOLVED_SCRIPT_FN: &str = "unresolved-script-fn";

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
            findings.push(WorldFinding {
                severity: Severity::Error,
                category: UNRESOLVED_SCRIPT_FN,
                message: format!(
                    "script registration for event '{}' references undefined function '{}'",
                    reg.event, reg.handler
                ),
                source: SourceLocation {
                    file: reg.source_path.clone(),
                    line: None,
                    reference: reg.handler.clone(),
                },
            });
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
}
