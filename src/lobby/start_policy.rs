//! Pure collective lobby-start policy (issue #1290).
//!
//! A ship host knows only its own crew. The browser host mesh combines one
//! [`ReadinessTally`] per ship with the public GM roster and grants one start
//! to every simulation peer. This module owns the value invariants and the
//! final, independently-testable policy; transport routing remains in the
//! browser and the fixed-tick application remains in `lobby::server`.

use serde::{Deserialize, Serialize};

use crate::gm_roster::GmRoster;

/// A bounded readiness count for one participant cohort.
///
/// Both fields are `u32`, so hostile JSON cannot smuggle an imprecise or
/// unbounded JavaScript number through a later Rust boundary. Construction
/// also preserves the important invariant that the ready subset cannot be
/// larger than the connected set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessTally {
    pub connected: u32,
    pub ready: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadinessTallyError {
    ReadyExceedsConnected,
    TotalOverflow,
}

impl ReadinessTally {
    pub fn try_new(connected: u32, ready: u32) -> Result<Self, ReadinessTallyError> {
        if ready > connected {
            return Err(ReadinessTallyError::ReadyExceedsConnected);
        }
        Ok(Self { connected, ready })
    }

    pub fn all_ready(self) -> bool {
        self.connected > 0 && self.ready == self.connected
    }

    pub fn checked_add(self, other: Self) -> Result<Self, ReadinessTallyError> {
        let connected = self
            .connected
            .checked_add(other.connected)
            .ok_or(ReadinessTallyError::TotalOverflow)?;
        let ready = self
            .ready
            .checked_add(other.ready)
            .ok_or(ReadinessTallyError::TotalOverflow)?;
        Self::try_new(connected, ready)
    }
}

/// The authenticated kind of start request the policy is evaluating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartTrigger<'a> {
    Automatic,
    Forced { operator_id: &'a str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartPolicyReason {
    NoParticipants,
    NotReady,
    ValidationFailed,
    GmNotConnected,
    InvalidReadiness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartPolicyDecision {
    Start { forced_by: Option<String> },
    Wait { reason: StartPolicyReason },
    Refused { reason: StartPolicyReason },
}

/// Decide a collective start from any number of ship-cohort tallies and the
/// one public GM roster shared by every peer.
///
/// A force request authenticates as a *connected* GM before it can skip the
/// readiness check. It never skips validation. Automatic start requires at
/// least one connected crew member or GM, which keeps an empty/AI-only lobby
/// from launching merely because every member of the empty set is ready.
pub fn evaluate_start_policy(
    crew: impl IntoIterator<Item = ReadinessTally>,
    gms: &GmRoster,
    validation_passed: bool,
    trigger: StartTrigger<'_>,
) -> StartPolicyDecision {
    let mut participants = ReadinessTally::default();
    for tally in crew {
        let Ok(next) = ReadinessTally::try_new(tally.connected, tally.ready)
            .and_then(|_| participants.checked_add(tally))
        else {
            return StartPolicyDecision::Refused {
                reason: StartPolicyReason::InvalidReadiness,
            };
        };
        participants = next;
    }
    let Ok(participants) = participants.checked_add(gms.readiness_tally()) else {
        return StartPolicyDecision::Refused {
            reason: StartPolicyReason::InvalidReadiness,
        };
    };

    match trigger {
        StartTrigger::Forced { operator_id } => {
            if !gms.is_connected(operator_id) {
                return StartPolicyDecision::Refused {
                    reason: StartPolicyReason::GmNotConnected,
                };
            }
            if !validation_passed {
                return StartPolicyDecision::Refused {
                    reason: StartPolicyReason::ValidationFailed,
                };
            }
            StartPolicyDecision::Start {
                forced_by: Some(operator_id.to_string()),
            }
        }
        StartTrigger::Automatic => {
            if !validation_passed {
                return StartPolicyDecision::Refused {
                    reason: StartPolicyReason::ValidationFailed,
                };
            }
            if participants.connected == 0 {
                return StartPolicyDecision::Wait {
                    reason: StartPolicyReason::NoParticipants,
                };
            }
            if participants.ready != participants.connected {
                return StartPolicyDecision::Wait {
                    reason: StartPolicyReason::NotReady,
                };
            }
            StartPolicyDecision::Start { forced_by: None }
        }
    }
}

/// Standalone native hosts own both cohorts locally. An enabled GM whose
/// screen is unavailable remains in the roster and blocks automatic launch.
/// Fleet hosts continue using their canonical collective start grant instead.
pub fn local_lobby_ready(crew: ReadinessTally, gms: &GmRoster) -> bool {
    gms.operators().iter().all(|gm| gm.connected && gm.ready)
        && matches!(
            evaluate_start_policy([crew], gms, true, StartTrigger::Automatic),
            StartPolicyDecision::Start { .. }
        )
}

/// Maximum accepted fixed-tick start id. This is a protocol bound, not a
/// gameplay value; it follows the public operator/session id bound.
pub const MAX_START_GRANT_ID_CHARS: usize = 64;

/// Largest logical tick JavaScript can carry without losing integer identity.
///
/// Start grants cross the browser host mesh as JSON numbers. Rust's `u64`
/// range is wider than JavaScript's exact integer range, so accepting a larger
/// value would let two peers decode the same apparent grant onto different
/// ticks before either reached this validation seam.
pub const MAX_SAFE_START_APPLY_TICK: u64 = (1_u64 << 53) - 1;

/// A host-mesh start grant delivered identically to every simulation peer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartGrant {
    pub id: String,
    pub mode: StartGrantMode,
    pub operator_id: Option<String>,
    /// The exact [`crate::sim_tick::SimTick`] on which every simulation peer
    /// applies this immutable decision. Zero is the owner-edge proposal
    /// sentinel: the Rust fixed-tick admission system replaces it with a safe
    /// future boundary before the grant enters a TickFrame.
    #[serde(default)]
    pub apply_tick: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartGrantMode {
    Automatic,
    Forced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartGrantError {
    InvalidId,
    InvalidAttribution,
    InvalidApplyTick,
}

impl StartGrant {
    /// Validate the exact browser grant shape and return its monotonic sequence.
    /// IDs are rendered as `start-N` on the public mesh, preserving a readable
    /// idempotency key without sacrificing a bounded monotonic comparison.
    pub fn validate(&self) -> Result<u64, StartGrantError> {
        if self.id.is_empty() || self.id.chars().count() > MAX_START_GRANT_ID_CHARS {
            return Err(StartGrantError::InvalidId);
        }
        let sequence = self
            .id
            .strip_prefix("start-")
            .and_then(|tail| tail.parse::<u64>().ok())
            .filter(|sequence| *sequence > 0)
            .ok_or(StartGrantError::InvalidId)?;
        if self.apply_tick > MAX_SAFE_START_APPLY_TICK {
            return Err(StartGrantError::InvalidApplyTick);
        }

        match (self.mode, self.operator_id.as_deref()) {
            (StartGrantMode::Automatic, None) => Ok(sequence),
            (StartGrantMode::Forced, Some(id))
                if !id.is_empty()
                    && id.chars().count() <= crate::gm_roster::MAX_GM_OPERATOR_ID_CHARS =>
            {
                Ok(sequence)
            }
            _ => Err(StartGrantError::InvalidAttribution),
        }
    }

    pub fn trigger(&self) -> StartTrigger<'_> {
        match self.mode {
            StartGrantMode::Automatic => StartTrigger::Automatic,
            StartGrantMode::Forced => StartTrigger::Forced {
                operator_id: self
                    .operator_id
                    .as_deref()
                    .expect("validated forced grants carry an operator id"),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartGrantStatus {
    Applied,
    NoOp,
    Refused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StartGrantReason {
    AlreadyStarted,
    ValidationFailed,
    ReadinessChanged,
    GmNotConnected,
    FleetNotManaged,
    InvalidGrant,
    UnauthorizedGrant,
    UnsafeApplyTick,
    ConflictingGrant,
    MissedApplyTick,
}

/// Fixed-tick result returned to the local host page. Its field names mirror
/// the mesh force-result envelope so the operator surface needs one renderer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartGrantResult {
    /// Logical tick at which the fixed-tick policy produced this terminal
    /// result. The browser may not drain it until PostUpdate, after one or more
    /// fixed steps have advanced `SimTick`.
    pub tick: u64,
    pub status: StartGrantStatus,
    pub operator_id: Option<String>,
    pub reason: Option<StartGrantReason>,
    pub grant_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gm_roster::{GmOperator, GmRoster};

    fn gm(id: &str, connected: bool, ready: bool) -> GmOperator {
        GmOperator {
            id: id.into(),
            name: id.into(),
            connected,
            ready,
        }
    }

    fn roster(rows: Vec<GmOperator>) -> GmRoster {
        GmRoster::try_new(rows).unwrap()
    }

    #[test]
    fn zero_participants_waits_and_stationless_crew_counts_like_any_crew() {
        assert_eq!(
            evaluate_start_policy([], &GmRoster::default(), true, StartTrigger::Automatic),
            StartPolicyDecision::Wait {
                reason: StartPolicyReason::NoParticipants
            }
        );
        assert_eq!(
            evaluate_start_policy(
                [ReadinessTally::try_new(1, 1).unwrap()],
                &GmRoster::default(),
                true,
                StartTrigger::Automatic,
            ),
            StartPolicyDecision::Start { forced_by: None }
        );
    }

    #[test]
    fn every_connected_crew_cohort_and_gm_must_be_ready() {
        let gms = roster(vec![gm("gm-1", true, true), gm("gm-2", false, false)]);
        assert_eq!(
            evaluate_start_policy(
                [
                    ReadinessTally::try_new(2, 2).unwrap(),
                    ReadinessTally::try_new(1, 0).unwrap(),
                ],
                &gms,
                true,
                StartTrigger::Automatic,
            ),
            StartPolicyDecision::Wait {
                reason: StartPolicyReason::NotReady
            }
        );
        assert_eq!(
            evaluate_start_policy(
                [
                    ReadinessTally::try_new(2, 2).unwrap(),
                    ReadinessTally::try_new(1, 1).unwrap(),
                ],
                &gms,
                true,
                StartTrigger::Automatic,
            ),
            StartPolicyDecision::Start { forced_by: None }
        );
    }

    #[test]
    fn gm_only_auto_start_excludes_disconnected_gms() {
        let gms = roster(vec![gm("gm-1", true, true), gm("gm-2", false, true)]);
        assert_eq!(
            evaluate_start_policy([], &gms, true, StartTrigger::Automatic),
            StartPolicyDecision::Start { forced_by: None }
        );
    }

    #[test]
    fn every_connected_gm_is_an_equal_force_authority() {
        let gms = roster(vec![gm("gm-1", true, false), gm("gm-2", true, false)]);
        for id in ["gm-1", "gm-2"] {
            assert_eq!(
                evaluate_start_policy(
                    [ReadinessTally::try_new(2, 0).unwrap()],
                    &gms,
                    true,
                    StartTrigger::Forced { operator_id: id },
                ),
                StartPolicyDecision::Start {
                    forced_by: Some(id.to_string())
                }
            );
        }
    }

    #[test]
    fn unknown_disconnected_and_ship_identities_cannot_force() {
        let gms = roster(vec![gm("gm-1", false, false)]);
        for id in ["gm-1", "ship-1", "unknown"] {
            assert_eq!(
                evaluate_start_policy(
                    [ReadinessTally::try_new(1, 0).unwrap()],
                    &gms,
                    true,
                    StartTrigger::Forced { operator_id: id },
                ),
                StartPolicyDecision::Refused {
                    reason: StartPolicyReason::GmNotConnected
                }
            );
        }
    }

    #[test]
    fn validation_failure_blocks_auto_and_force() {
        let gms = roster(vec![gm("gm-1", true, true)]);
        assert_eq!(
            evaluate_start_policy([], &gms, false, StartTrigger::Automatic),
            StartPolicyDecision::Refused {
                reason: StartPolicyReason::ValidationFailed
            }
        );
        assert_eq!(
            evaluate_start_policy(
                [],
                &gms,
                false,
                StartTrigger::Forced {
                    operator_id: "gm-1"
                },
            ),
            StartPolicyDecision::Refused {
                reason: StartPolicyReason::ValidationFailed
            }
        );
    }

    #[test]
    fn tally_and_grant_shapes_enforce_their_invariants() {
        assert_eq!(
            ReadinessTally::try_new(1, 2),
            Err(ReadinessTallyError::ReadyExceedsConnected)
        );
        let automatic = StartGrant {
            id: "start-7".into(),
            mode: StartGrantMode::Automatic,
            operator_id: None,
            apply_tick: 412,
        };
        assert_eq!(automatic.validate(), Ok(7));
        assert_eq!(
            StartGrant {
                operator_id: Some("gm-1".into()),
                ..automatic.clone()
            }
            .validate(),
            Err(StartGrantError::InvalidAttribution)
        );
        assert_eq!(
            StartGrant {
                id: "start-8".into(),
                mode: StartGrantMode::Forced,
                operator_id: Some("gm-1".into()),
                apply_tick: 412,
            }
            .validate(),
            Ok(8)
        );
        assert_eq!(
            StartGrant {
                apply_tick: MAX_SAFE_START_APPLY_TICK + 1,
                ..automatic
            }
            .validate(),
            Err(StartGrantError::InvalidApplyTick)
        );
    }
}
