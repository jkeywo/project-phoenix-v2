//! The pure, Bevy-free **held-response** vocabulary (issue #1158).
//!
//! One tractor system serves several verbs because the **target** supplies the
//! consequence. The tractor (#1156) supplies the geometry — where a held target
//! rides — and the helm penalty, and knows nothing about which of these it is
//! doing. What being held *does* to a target is authored on the target itself,
//! in its own `[held_response]` table, so a scenario author reaches a new
//! behaviour by authoring the target rather than by adding a verb.
//!
//! # The vocabulary
//!
//! * **follow** (tow) — a derelict rides the operator's rig, the tractor's
//!   default motion and nothing more.
//! * **arrest-decline** (stabilise) — a failing structure's condition decline is
//!   arrested while the beam holds it steady, and it recovers at an authored
//!   rate; the recovered condition crosses the target's OWN authored thresholds
//!   and sets the operational flags a scenario already reads.
//! * **station-keep** (hold in place) — a self-moving craft is held on the
//!   operator's rig without ceasing to be a thing that can be ordered elsewhere.
//! * **formation-keep** (escort) — a self-moving target is held in formation at
//!   an authored offset and distance, distinct from being merely station-kept in
//!   place on the operator's own rig.
//!
//! A target that authors NO `[held_response]` table is merely held in place —
//! the station-keep default — so every derelict and craft written before this
//! existed goes on being held exactly as #1156 held it.
//!
//! # Why this is a module of its own, Bevy-free (rule 10)
//!
//! Two decisions live here and nothing else, the way the coupling module's
//! geometry and verdict do: the OFFSET a held target rides at
//! ([`held_offset`], the one thing that distinguishes formation-keep from the
//! rest) and the condition ADJUSTMENT a held target banks this tick
//! ([`condition_delta`], the one thing arrest-decline does that the others do
//! not). The sibling [`crate::tractor::server`] adapter reads the live world
//! into the scalars these take, calls in, and applies what comes back — feeding
//! the offset to the coupling module's [`crate::tractor::coupled_position`] and
//! the delta to the infrastructure condition queue. It takes `glam::Vec3`, not a
//! Bevy type, so it compiles and is tested with no app, no world and no
//! schedule.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Which named response holding a target invokes (issue #1158).
///
/// The kebab-case names are the authored vocabulary — `kind = "arrest-decline"`
/// on the target's `[held_response]` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeldResponseKind {
    /// A derelict rides the rig under tow — the tractor's default motion.
    Follow,
    /// A degrading structure's decline is arrested while held; its condition
    /// moves at an authored rate.
    ArrestDecline,
    /// A self-moving craft is held on the operator's rig in place.
    StationKeep,
    /// A self-moving target is held in formation at an authored offset and
    /// distance.
    FormationKeep,
}

/// The authored `[held_response]` table on a TARGET entity (issue #1158).
///
/// Every field is a designer's number, read from TOML (AGENTS.md rule 11), and
/// every per-kind field is optional because it belongs to exactly one `kind`:
/// [`Self::validate`] rejects a table that authors a field its kind does not
/// use, or omits one its kind needs, so a mistake is a load error naming the
/// field rather than a hold that quietly does nothing. A target that authors no
/// `[held_response]` at all carries no component and is merely held in place.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldResponseConfig {
    /// Which named response holding this target invokes.
    pub kind: HeldResponseKind,
    /// **arrest-decline only.** Condition points per second the held structure
    /// moves at while the beam is on it, over and above the ordinary decline the
    /// hold arrests. `0.0` holds the structure exactly steady; a positive value
    /// recovers it. Required for arrest-decline, forbidden for every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recover_per_sec: Option<f32>,
    /// **formation-keep only.** The formation bearing in the operator's OWN
    /// frame — a direction the slot lies along, rotated by the operator's
    /// rotation so the formation swings round as the operator turns. Need not be
    /// unit length; its direction is what is read, and [`Self::distance`] sets
    /// how far. Required for formation-keep, forbidden for every other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<[f32; 3]>,
    /// **formation-keep only.** How far along [`Self::offset`] the target rides,
    /// in world units. Required for formation-keep, forbidden for every other
    /// kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<f32>,
}

impl HeldResponseConfig {
    /// Reject a `[held_response]` table whose fields do not match its kind
    /// (issue #1158).
    ///
    /// Called at entity-config parse time so a typo — `recover_per_sec` on a
    /// formation-keep, a missing `distance`, a zero-length formation bearing —
    /// is a load error naming the field, not a hold that silently arrests
    /// nothing or holds a target on top of its operator.
    pub fn validate(&self) -> Result<(), String> {
        match self.kind {
            HeldResponseKind::Follow | HeldResponseKind::StationKeep => {
                self.reject_arrest_fields()?;
                self.reject_formation_fields()?;
            }
            HeldResponseKind::ArrestDecline => {
                self.reject_formation_fields()?;
                let rate = self.recover_per_sec.ok_or_else(|| {
                    "[held_response] kind = \"arrest-decline\" needs a recover_per_sec — the rate \
                     its condition moves at while held (0.0 holds it steady)"
                        .to_string()
                })?;
                if !rate.is_finite() || rate < 0.0 {
                    return Err(format!(
                        "[held_response] recover_per_sec must be a non-negative finite number, \
                         got {rate}"
                    ));
                }
            }
            HeldResponseKind::FormationKeep => {
                self.reject_arrest_fields()?;
                let offset = self.offset.ok_or_else(|| {
                    "[held_response] kind = \"formation-keep\" needs an offset — the formation \
                     bearing in the operator's own frame"
                        .to_string()
                })?;
                for (axis, component) in offset.iter().enumerate() {
                    if !component.is_finite() {
                        return Err(format!(
                            "[held_response] offset component {axis} must be finite, got {component}"
                        ));
                    }
                }
                if Vec3::from(offset).length_squared() == 0.0 {
                    return Err(
                        "[held_response] formation-keep offset must have a direction — a \
                         zero-length bearing would hold the target on top of its operator"
                            .to_string(),
                    );
                }
                let distance = self.distance.ok_or_else(|| {
                    "[held_response] kind = \"formation-keep\" needs a distance — how far along \
                     the offset the target rides"
                        .to_string()
                })?;
                if !distance.is_finite() || distance <= 0.0 {
                    return Err(format!(
                        "[held_response] formation-keep distance must be a positive finite \
                         number, got {distance}"
                    ));
                }
            }
        }
        Ok(())
    }

    fn reject_arrest_fields(&self) -> Result<(), String> {
        if self.recover_per_sec.is_some() {
            return Err(format!(
                "[held_response] recover_per_sec belongs to arrest-decline, not {:?}",
                self.kind
            ));
        }
        Ok(())
    }

    fn reject_formation_fields(&self) -> Result<(), String> {
        if self.offset.is_some() || self.distance.is_some() {
            return Err(format!(
                "[held_response] offset/distance belong to formation-keep, not {:?}",
                self.kind
            ));
        }
        Ok(())
    }

    /// Resolve the authored table into the value the adapter applies.
    ///
    /// Assumes [`Self::validate`] has passed (it runs at load); the defensive
    /// `unwrap_or` defaults never fire on an authored table that survived load,
    /// and only keep a hand-built config from panicking.
    pub fn resolve(&self) -> HeldResponse {
        match self.kind {
            HeldResponseKind::Follow => HeldResponse::Follow,
            HeldResponseKind::StationKeep => HeldResponse::StationKeep,
            HeldResponseKind::ArrestDecline => HeldResponse::ArrestDecline {
                recover_per_sec: self.recover_per_sec.unwrap_or(0.0),
            },
            HeldResponseKind::FormationKeep => HeldResponse::FormationKeep {
                offset: Vec3::from(self.offset.unwrap_or([0.0, 0.0, 0.0])),
                distance: self.distance.unwrap_or(0.0),
            },
        }
    }
}

/// The resolved held-response — what the adapter applies (issue #1158).
///
/// A target with no authored table resolves to nothing here; the adapter treats
/// its absence as [`HeldResponse::StationKeep`], which is why the two decisions
/// below both return the "held in place, condition untouched" answer for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HeldResponse {
    /// A derelict rides the operator's rig under tow.
    Follow,
    /// A degrading structure held steady, recovering at the authored rate.
    ArrestDecline {
        /// Condition points per second the structure moves at while held.
        recover_per_sec: f32,
    },
    /// A self-moving craft held on the operator's rig in place.
    StationKeep,
    /// A self-moving target held in formation at the authored slot, in the
    /// operator's own frame.
    FormationKeep {
        /// The formation bearing in the operator's own frame (need not be unit
        /// length).
        offset: Vec3,
        /// How far along `offset` the target rides.
        distance: f32,
    },
}

/// **The one thing formation-keep changes: where the held target rides.**
///
/// Returns the offset, in the operator's OWN frame, to feed the coupling
/// module's [`crate::tractor::coupled_position`]. Every response but
/// formation-keep rides the operator's authored coupling rig
/// (`operator_coupling_offset`, the `[tractor]` table's `coupling_offset`);
/// formation-keep rides its OWN authored slot — `distance` units along its
/// bearing — which is what makes escort distinct from station-keeping the target
/// in place on the operator's rig.
///
/// The adapter feeds whatever this returns to the same generic
/// `coupled_position`, so the tractor never branches on the held-response: it
/// applies the offset the held target declares.
pub fn held_offset(response: &HeldResponse, operator_coupling_offset: Vec3) -> Vec3 {
    match response {
        HeldResponse::FormationKeep { offset, distance } => offset.normalize_or_zero() * *distance,
        HeldResponse::Follow | HeldResponse::ArrestDecline { .. } | HeldResponse::StationKeep => {
            operator_coupling_offset
        }
    }
}

/// **The one thing arrest-decline changes: the condition banked this tick.**
///
/// Returns the condition adjustment, in points, the adapter queues on the held
/// target for `crate::infrastructure::tick_infrastructure_condition` to apply
/// THIS tick. Only arrest-decline moves the condition track; every other
/// response returns `0.0` and leaves the structure's ordinary decline entirely
/// alone.
///
/// # How arrest-decline arrests
///
/// The infrastructure tick applies the target's ordinary decline every tick —
/// `decay_per_sec * dt` off the top. Arrest-decline cancels exactly that decline
/// (the `decay_per_sec * dt` term, the same product the infra tick computes) and
/// adds the authored recovery (`recover_per_sec * dt`), so the NET movement is
/// the authored rate: `0.0` holds the structure steady, a positive rate recovers
/// it. Releasing the beam stops the adapter queuing anything, so the target's
/// ordinary decline resumes on the very next tick with nothing to arrest it.
///
/// Expressing the arrest as a queued adjustment — rather than reaching into the
/// condition track — is what keeps the recovered condition crossing the target's
/// OWN authored thresholds: the adjustment lands through the one system that
/// owns the flag edges, so a structure recovered across `restores_above` sets
/// the operational flag a scenario already reads, by the same rule a scripted
/// repair does.
pub fn condition_delta(response: &HeldResponse, decay_per_sec: f32, dt: f32) -> f32 {
    match response {
        HeldResponse::ArrestDecline { recover_per_sec } => {
            decay_per_sec * dt + recover_per_sec * dt
        }
        HeldResponse::Follow | HeldResponse::StationKeep | HeldResponse::FormationKeep { .. } => {
            0.0
        }
    }
}

#[cfg(test)]
#[path = "held_response_tests.rs"]
mod tests;
