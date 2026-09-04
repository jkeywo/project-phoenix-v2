//! The pure, Bevy-free heart of the controlled-demolition operation (issue
//! #1350, PRD #1337).
//!
//! Clearing a Falling Skyway obstruction is a four-stage act that no single
//! system owns: Tactical dispatches Security to place charges (`src/security/`),
//! Engineering holds the target with tractor control where the mass needs it
//! (`src/tractor/`), Security withdraws, and then Tactical DETONATES. The first
//! three stages are the existing team-dispatch, tractor-hold and withdrawal
//! machinery, unchanged. This module is the fourth — the authoritative operation
//! state that ties the others together and the one-shot detonation act itself.
//!
//! # Why this is a module of its own, Bevy-free (rule 10)
//!
//! Detonation READS Security's team states and the tractor's coupling and RAISES
//! the world flag a scenario hangs its four outcomes off — but it is owned by
//! neither: a team is not detonated, a beam is not detonated, an obstruction is.
//! Everything here — the operation's authored terms, the refusal vocabulary, the
//! detonation verdict and the four-outcome decision — is a plain function of
//! scalars the sibling [`crate::demolition::server`] adapter reads out of the
//! live world and applies the answer of. The split the tractor keeps between
//! `coupling` and `server`, and Security between `teams` and `server`.
//!
//! # The engine never learns a scenario's names
//!
//! What a safe demolition leaves behind, what an unsupported one costs, who the
//! premature one kills and what ordinary weapons fire scatters are the scenario's
//! business, authored entirely in TOML off the four flags this module raises.
//! Nothing here, and nothing in the adapter, branches on a world entity's name:
//! the Falling Skyway's obstruction and a later mission's blocked hatch are the
//! same code path with different `[demolition_target]` tables.

use serde::{Deserialize, Serialize};

/// How a completed detonation turned out (issue #1350) — the three outcomes the
/// detonation ACT itself can produce.
///
/// The fourth outcome the issue names — ordinary weapons fire producing the
/// least-controlled debris — is NOT reached through here at all: no charges were
/// ever placed and no detonation command was ever sent, so it is the scenario's
/// own `on_destroyed` handler firing on a target killed by gunnery, which needs
/// no engine support beyond what combat already provides. This enum is only the
/// outcomes of a deliberate detonation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DemolitionOutcome {
    /// Charges placed, the team clear, and the mass stabilised where the target
    /// required it: the obstruction comes apart with limited debris.
    Safe,
    /// Charges placed and the team clear, but a target that needed a tractor hold
    /// was detonated without one: more debris than a supported shot.
    Unsupported,
    /// Detonated while a Security team was still committed to the target — the
    /// crew had not withdrawn. Casualties.
    Premature,
}

impl DemolitionOutcome {
    /// Every outcome, in declaration order.
    pub const ALL: [DemolitionOutcome; 3] = [
        DemolitionOutcome::Safe,
        DemolitionOutcome::Unsupported,
        DemolitionOutcome::Premature,
    ];

    /// The stable snake_case id written on the wire and shown in telemetry.
    pub fn as_str(self) -> &'static str {
        match self {
            DemolitionOutcome::Safe => "safe",
            DemolitionOutcome::Unsupported => "unsupported",
            DemolitionOutcome::Premature => "premature",
        }
    }
}

/// The one reason a detonation was refused (issue #1350), as the console shows it
/// — a `strings.csv` id, never English. Mirrors
/// [`crate::security::SecurityRefusal`].
///
/// A team that has not withdrawn is deliberately NOT a refusal: detonating early
/// is a real, allowed act with a real consequence ([`DemolitionOutcome::Premature`]).
/// The crew are SHOWN the team-safety state and left to decide — the engine does
/// not treat "team clear" as a gate, only "charges placed" and "not already
/// detonated". Refusing a premature shot would hide the choice the issue makes
/// the whole point of the beat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DemolitionRefusal {
    /// The Security System that fires the charges is damaged out.
    Disabled,
    /// Nothing in the world answers to the named target, or what does authors no
    /// `[demolition_target]` table at all.
    NoSuchTarget,
    /// Charges have not been placed here yet — there is nothing to detonate.
    NotCharged,
    /// The charges have already been fired. One detonation per operation.
    AlreadyDetonated,
}

impl DemolitionRefusal {
    /// Every refusal, in declaration order — the coverage guard for the
    /// `strings.csv` rows.
    pub const ALL: [DemolitionRefusal; 4] = [
        DemolitionRefusal::Disabled,
        DemolitionRefusal::NoSuchTarget,
        DemolitionRefusal::NotCharged,
        DemolitionRefusal::AlreadyDetonated,
    ];

    /// The `strings.csv` id the console resolves through `t()`. A `match`, not a
    /// composed string, so `check-strings.mjs` sees every id a new variant needs.
    pub fn string_id(self) -> &'static str {
        match self {
            DemolitionRefusal::Disabled => "demolition.detonate.refused.disabled",
            DemolitionRefusal::NoSuchTarget => "demolition.detonate.refused.no_such_target",
            DemolitionRefusal::NotCharged => "demolition.detonate.refused.not_charged",
            DemolitionRefusal::AlreadyDetonated => "demolition.detonate.refused.already_detonated",
        }
    }
}

/// The authored `[demolition_target]` table on a world entity that can be cleared
/// by controlled demolition (issue #1350).
///
/// Every field is a machine id or a designer's flag name, read from TOML: rule
/// 11, no hardcoded gameplay values, no English. The flags are the ONLY interface
/// between the engine and the scenario — the engine raises them, the scenario's
/// script hangs every debris fragment, casualty and infrastructure consequence
/// off them. An entity that authors no `[demolition_target]` carries no
/// [`crate::demolition::server::DemolitionTarget`] component and cannot be
/// detonated at all — which is why every shipped hull and every existing world is
/// untouched by this slice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DemolitionConfig {
    /// The world flag whose SET state means "charges are placed here". Authored to
    /// equal the `outcome_flag` of this same entity's `[[security_target.action]]
    /// action = "place_charges"` block, so a completed placement is what arms the
    /// detonation — the two halves meet at one flag name and the engine learns the
    /// dependency from the TOML, never from a hardcoded pairing.
    pub charges_flag: String,
    /// The world flag this operation raises on ANY successful detonation,
    /// whatever its outcome. The "spent" marker the refusal gate reads to keep a
    /// second detonation idempotent, and the beat a scenario hangs its generic
    /// "the charges went off" telemetry off.
    pub detonated_flag: String,
    /// The world flag a [`DemolitionOutcome::Safe`] detonation raises.
    pub safe_flag: String,
    /// The world flag a [`DemolitionOutcome::Unsupported`] detonation raises.
    pub unsupported_flag: String,
    /// The world flag a [`DemolitionOutcome::Premature`] detonation raises.
    pub premature_flag: String,
    /// Whether this mass needs a tractor hold to come apart cleanly. When true, a
    /// detonation with no live coupling is [`DemolitionOutcome::Unsupported`]
    /// rather than [`DemolitionOutcome::Safe`]; when false, stabilisation is not
    /// part of this target's problem and a clean shot is always safe. Defaults to
    /// false — a target that says nothing needs no hold.
    #[serde(default)]
    pub stabilization_required: bool,
    /// A `strings.csv` id for the warning the console shows beside the detonate
    /// control — what the crew are about to set off. Never English (rule 11).
    /// `None` for an operation that needs no warning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl DemolitionConfig {
    /// Reject a `[demolition_target]` table that could never resolve (issue
    /// #1350): a blank flag name (a consequence nothing can hang off), or the same
    /// name used for two of the four flags (an outcome that would silently also
    /// fire another's consequence). Each is an author mistake whose only other
    /// symptom would be a detonate control that clears the wrong thing, or
    /// nothing.
    pub fn validate(&self) -> Result<(), String> {
        let named = [
            ("charges_flag", &self.charges_flag),
            ("detonated_flag", &self.detonated_flag),
            ("safe_flag", &self.safe_flag),
            ("unsupported_flag", &self.unsupported_flag),
            ("premature_flag", &self.premature_flag),
        ];
        for (field, value) in named {
            if value.trim().is_empty() {
                return Err(format!(
                    "[demolition_target] {field} must name a world flag, never be blank"
                ));
            }
        }
        for (index, (field, value)) in named.iter().enumerate() {
            if let Some((other, _)) = named[..index].iter().find(|(_, v)| v == value) {
                return Err(format!(
                    "[demolition_target] {field} and {other} both name the flag '{value}'; each of \
                     the five flags must be distinct, or one outcome silently fires another's \
                     consequence"
                ));
            }
        }
        if self
            .warning
            .as_deref()
            .is_some_and(|id| id.trim().is_empty())
        {
            return Err(
                "[demolition_target] warning must be a strings.csv id, or be omitted".to_string(),
            );
        }
        Ok(())
    }

    /// The world flag a given outcome raises on this target.
    pub fn outcome_flag(&self, outcome: DemolitionOutcome) -> &str {
        match outcome {
            DemolitionOutcome::Safe => &self.safe_flag,
            DemolitionOutcome::Unsupported => &self.unsupported_flag,
            DemolitionOutcome::Premature => &self.premature_flag,
        }
    }
}

/// **The detonation verdict.** `Ok(())` when a detonation may FORM against this
/// target this instant, else the one refusal the console shows (issue #1350).
///
/// Pure: the adapter reads the live world into these scalars and applies the
/// answer. Note what is NOT here — whether the team has withdrawn, and whether
/// the target is stabilised. Neither refuses the shot; they DECIDE its outcome
/// (see [`resolve_outcome`]). The only things that stop a detonation forming are
/// the System being knocked out, the target not existing, the charges not being
/// placed, and the charges already having been fired.
///
/// # Check order is the console's "most actionable first"
///
/// A knocked-out System fires nothing, so it is reported before anything about a
/// particular target; a target that does not exist is reported before anything
/// about its state; among the state checks, "not charged yet" is the ordinary
/// not-ready reason and "already detonated" the spent one, so the ready-state
/// check comes before the spent one.
pub fn detonation_status(
    target_known: bool,
    charged: bool,
    detonated: bool,
    disabled: bool,
) -> Result<(), DemolitionRefusal> {
    if disabled {
        return Err(DemolitionRefusal::Disabled);
    }
    if !target_known {
        return Err(DemolitionRefusal::NoSuchTarget);
    }
    if detonated {
        return Err(DemolitionRefusal::AlreadyDetonated);
    }
    if !charged {
        return Err(DemolitionRefusal::NotCharged);
    }
    Ok(())
}

/// **The four-outcome decision.** Which [`DemolitionOutcome`] a detonation that
/// has passed [`detonation_status`] produces, from the two facts that were
/// deliberately left OUT of the refusal gate (issue #1350).
///
/// The order is a priority: a team still on the target is the gravest state, so
/// it decides the outcome regardless of stabilisation — a premature shot kills
/// the crew whether or not the mass was held. Only once the team is clear does
/// the stabilisation dependency choose between the clean and the messy shot, and
/// only when the target authored that it needs one.
pub fn resolve_outcome(
    team_clear: bool,
    stabilization_required: bool,
    stabilized: bool,
) -> DemolitionOutcome {
    if !team_clear {
        DemolitionOutcome::Premature
    } else if stabilization_required && !stabilized {
        DemolitionOutcome::Unsupported
    } else {
        DemolitionOutcome::Safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> DemolitionConfig {
        DemolitionConfig {
            charges_flag: "x_charged".to_string(),
            detonated_flag: "x_detonated".to_string(),
            safe_flag: "x_safe".to_string(),
            unsupported_flag: "x_unsupported".to_string(),
            premature_flag: "x_premature".to_string(),
            stabilization_required: false,
            warning: None,
        }
    }

    #[test]
    fn valid_config_passes() {
        assert!(base_config().validate().is_ok());
    }

    #[test]
    fn blank_flag_is_refused_at_load() {
        let mut config = base_config();
        config.safe_flag = "   ".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn duplicate_flag_names_are_refused_at_load() {
        let mut config = base_config();
        config.unsupported_flag = config.safe_flag.clone();
        let err = config.validate().unwrap_err();
        assert!(err.contains("unsupported_flag"));
        assert!(err.contains("safe_flag"));
    }

    #[test]
    fn blank_warning_is_refused_but_absent_one_is_fine() {
        let mut config = base_config();
        config.warning = Some(String::new());
        assert!(config.validate().is_err());
        config.warning = None;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn outcome_flag_maps_each_outcome_to_its_authored_flag() {
        let config = base_config();
        assert_eq!(config.outcome_flag(DemolitionOutcome::Safe), "x_safe");
        assert_eq!(
            config.outcome_flag(DemolitionOutcome::Unsupported),
            "x_unsupported"
        );
        assert_eq!(
            config.outcome_flag(DemolitionOutcome::Premature),
            "x_premature"
        );
    }

    #[test]
    fn disabled_system_refuses_before_anything_else() {
        // Disabled wins even when the target is unknown and uncharged.
        assert_eq!(
            detonation_status(false, false, false, true),
            Err(DemolitionRefusal::Disabled)
        );
    }

    #[test]
    fn unknown_target_is_refused() {
        assert_eq!(
            detonation_status(false, true, false, false),
            Err(DemolitionRefusal::NoSuchTarget)
        );
    }

    #[test]
    fn uncharged_target_cannot_be_detonated() {
        assert_eq!(
            detonation_status(true, false, false, false),
            Err(DemolitionRefusal::NotCharged)
        );
    }

    #[test]
    fn already_detonated_is_idempotently_refused() {
        assert_eq!(
            detonation_status(true, true, true, false),
            Err(DemolitionRefusal::AlreadyDetonated)
        );
    }

    #[test]
    fn a_charged_undetonated_target_may_fire() {
        assert_eq!(detonation_status(true, true, false, false), Ok(()));
    }

    #[test]
    fn team_still_present_is_always_premature() {
        // Regardless of stabilisation, a team on the target dies.
        assert_eq!(
            resolve_outcome(false, true, true),
            DemolitionOutcome::Premature
        );
        assert_eq!(
            resolve_outcome(false, false, false),
            DemolitionOutcome::Premature
        );
    }

    #[test]
    fn clear_and_stabilised_is_safe() {
        assert_eq!(resolve_outcome(true, true, true), DemolitionOutcome::Safe);
    }

    #[test]
    fn clear_but_unheld_when_required_is_unsupported() {
        assert_eq!(
            resolve_outcome(true, true, false),
            DemolitionOutcome::Unsupported
        );
    }

    #[test]
    fn stabilisation_not_required_is_safe_without_a_hold() {
        // A target that does not need holding is safe with no coupling.
        assert_eq!(resolve_outcome(true, false, false), DemolitionOutcome::Safe);
    }

    #[test]
    fn every_refusal_has_a_distinct_string_id() {
        let mut ids: Vec<&str> = DemolitionRefusal::ALL
            .iter()
            .map(|r| r.string_id())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), DemolitionRefusal::ALL.len());
    }
}
