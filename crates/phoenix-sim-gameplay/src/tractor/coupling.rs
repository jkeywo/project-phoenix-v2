//! The pure, Bevy-free heart of the tractor beam (issue #1156).
//!
//! Two things live here and nothing else: the **coupling-position module**
//! ([`coupled_position`]) — given the operator's transform and the authored
//! coupling offset, where the held target sits — and the pure **hold verdict**
//! ([`hold_status`]) that decides, from live scalars the adapter reads off the
//! world, whether the coupling may form this tick and, if not, the one refusal
//! reason the console shows.
//!
//! # Why this is a module of its own, Bevy-free
//!
//! AGENTS.md rule 10: the geometry and the verdict are decided here, in
//! isolation, and unit-tested here; the sibling [`crate::tractor::server`]
//! adapter gathers the real components, calls in, and applies what comes back,
//! deciding nothing itself. The position module is the exact shape lifted and
//! generalised from `operations::server::move_towed_targets` — an offset in the
//! operator's OWN frame, rotated by the operator's post-integration rotation, so
//! a tug that turns swings its load round with it rather than dragging it
//! sideways through the towline.
//!
//! It takes `glam` types, not Bevy ones. Bevy re-exports `glam`, so a
//! `Transform`'s `translation`/`rotation` ARE these types and the adapter passes
//! them straight in; but nothing here imports `bevy`, so the module compiles and
//! is tested with no app, no world and no schedule. `glam` is pinned with the
//! `libm` feature for determinism (see `Cargo.toml`), the same backing
//! `simmath` uses — so this maths agrees bit-for-bit on native and wasm.
//!
//! # The applied review change (mass is NOT here)
//!
//! The coupling-position module takes **transforms and the authored offset
//! only**. A tractor pulling a heavier hull is a helm-*penalty* concern, and the
//! entity `mass` authored by #1154 enters there, in the later slice #1157 —
//! never in the geometry of where the load rides. Keeping mass out of this
//! signature is what stops the two concerns from tangling.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// The authored coupling terms for a hull's tractor beam — its `[tractor]`
/// table (issue #1156).
///
/// Every field is a designer's number, read from TOML: AGENTS.md rule 11, no
/// hardcoded gameplay values. A hull that authors no `[tractor]` table carries
/// no [`crate::tractor::server::TractorBeam`] component and is unchanged in every
/// way. The **power group** the tractor draws from is NOT here — it is the
/// `power_group` field of the tractor `[[system]]` block, the one authoritative
/// place a system names its group — and the adapter resolves it at spawn so the
/// two can never drift.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TractorConfig {
    /// The furthest the locked target may sit from the operator and still be
    /// held, in world units. Drifting past it drops the coupling
    /// ([`TractorRefusal::OutOfRange`]).
    pub range: f32,
    /// Where the held target rides, in the operator's OWN frame — the rig. The
    /// same `[f32; 3]` shape and meaning as a tow's `tow_offset`: a load astern
    /// of the operator authors a negative Z, and the rig swings round as the
    /// operator turns because the offset is rotated by the operator's rotation
    /// in [`coupled_position`].
    pub coupling_offset: [f32; 3],
    /// The lowest power-group level at which the beam holds. Below it the
    /// coupling drops ([`TractorRefusal::Unpowered`]). Authored, not derived
    /// from the group's nominal rung, so a hull can make its tractor cheap or
    /// dear independent of what else shares the group.
    pub min_power_level: u8,
    /// The authored mass → helm-penalty curve (issue #1157). While the beam
    /// holds a target, the operator's top speed AND turn rate are cut by
    /// [`tow_load_penalty`] of that target's authored mass, read from these two
    /// numbers rather than a coefficient inlined anywhere. This is the concern
    /// the coupling-*position* module deliberately kept out of its signature:
    /// where the load rides takes transforms only; how much it drags the tug
    /// takes mass, and lives here beside it.
    pub tow_load: TowLoadCurve,
}

/// The authored curve mapping a held target's mass to the helm movement penalty
/// it exacts (issue #1157) — the `[tractor.tow_load]` sub-table.
///
/// A saturating curve, so a light target is barely felt and a heavy one is
/// severe, both from these two authored numbers (AGENTS.md rule 11 — no
/// coefficient inlined in Rust or JS). The penalty of a held mass `m` is
///
/// ```text
/// max_penalty · m / (m + half_penalty_mass)
/// ```
///
/// which is `0` at zero mass, exactly `max_penalty / 2` at `half_penalty_mass`,
/// and approaches `max_penalty` as the held mass grows without bound. It is the
/// SAME curve for every target: a buoy near zero mass barely registers while a
/// laden freighter saturates toward the cap, with no branch deciding which.
///
/// The penalty is applied as a NEGATIVE bonus on both `MaxSpeed` and
/// `MaxYawRate`, so `max_penalty` is the most of the ship's top speed and turn
/// rate the heaviest tow can ever cost — a `0.75` cap can never take more than
/// three-quarters of either, and the ship can never be dragged to a standstill.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TowLoadCurve {
    /// The held mass at which the penalty reaches half of [`Self::max_penalty`]
    /// — the curve's knee. A hull tows loads around this mass at a moderate
    /// penalty; much lighter is barely felt, much heavier saturates toward the
    /// cap.
    pub half_penalty_mass: f32,
    /// The most of the ship's top speed and turn rate a tow can ever cost, as a
    /// fraction in `[0, 1)` — approached asymptotically as the held mass grows.
    pub max_penalty: f32,
}

impl TowLoadCurve {
    /// Reject an authored `[tractor.tow_load]` curve that could never produce a
    /// sane penalty (issue #1157): a non-positive knee mass (which would make
    /// the curve undefined or negative), or a cap outside `[0, 1)` (a penalty of
    /// `1.0` or more would stop the ship dead, which no tow should).
    pub fn validate(&self) -> Result<(), String> {
        if !self.half_penalty_mass.is_finite() || self.half_penalty_mass <= 0.0 {
            return Err(format!(
                "[tractor.tow_load] half_penalty_mass must be a positive mass, got {}",
                self.half_penalty_mass
            ));
        }
        if !self.max_penalty.is_finite() || self.max_penalty < 0.0 || self.max_penalty >= 1.0 {
            return Err(format!(
                "[tractor.tow_load] max_penalty must be a fraction in [0, 1) — a penalty of 1.0 \
                 would drag the ship to a dead stop, got {}",
                self.max_penalty
            ));
        }
        Ok(())
    }
}

/// **The tow-load penalty.** The fraction of the operator's top speed AND turn
/// rate a held target of the given `mass` costs, read from the authored curve
/// (issue #1157).
///
/// Unlike [`coupled_position`], this takes MASS — it is the helm-penalty concern
/// the coupling-position module deliberately kept out of its signature (#1156),
/// living here in the same pure module so the number is a deterministic function
/// of a folded property rather than a guess. A target that authored no mass
/// carries #1154's `DEFAULT_ENTITY_MASS`, so this returns that mass's penalty
/// for it with no special case.
///
/// Returns a non-negative fraction the adapter applies as a NEGATIVE bonus on
/// `MaxSpeed` and `MaxYawRate`. Monotonically increasing in mass and bounded
/// above by `curve.max_penalty`. `half_penalty_mass` is validated positive, so
/// the denominator is always positive and there is no division by zero.
pub fn tow_load_penalty(mass: f32, curve: &TowLoadCurve) -> f32 {
    let m = mass.max(0.0);
    curve.max_penalty * m / (m + curve.half_penalty_mass)
}

impl TractorConfig {
    /// Reject an authored `[tractor]` table that describes a beam that could
    /// never hold anything (issue #1156). A non-positive range, or a zero
    /// minimum power level (which would let a wholly unpowered beam hold), are
    /// author mistakes whose only other symptom would be a control the crew can
    /// press and that quietly never grips.
    pub fn validate(&self) -> Result<(), String> {
        if self.range.is_nan() || self.range <= 0.0 {
            return Err(format!(
                "[tractor] range must be a positive distance, got {}",
                self.range
            ));
        }
        for (axis, component) in self.coupling_offset.iter().enumerate() {
            if !component.is_finite() {
                return Err(format!(
                    "[tractor] coupling_offset component {axis} must be finite, got {component}"
                ));
            }
        }
        if self.min_power_level == 0 {
            return Err(
                "[tractor] min_power_level must be at least 1 — a beam that holds at level 0 \
                 would never lose its allocation"
                    .to_string(),
            );
        }
        self.tow_load.validate()?;
        Ok(())
    }
}

/// The one reason a tractor coupling did not form (or dropped) this tick
/// (issue #1156), as the console shows it — a `strings.csv` id, never English.
///
/// The umbilical, dock and repair-dispatch slices copy this refusal-plus-
/// `string_id` shape; it mirrors `operations::Ineligibility`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TractorRefusal {
    /// Tactical holds no lock, so the beam has nothing to grip. Both "engaged
    /// with no lock" and "the lock was dropped mid-hold" report this.
    NoLock,
    /// The locked target sits further than the authored `range`.
    OutOfRange,
    /// The tractor's power group is below the authored `min_power_level`.
    Unpowered,
    /// The tractor system is damaged to `Disabled` (or `Destroyed`).
    Disabled,
}

impl TractorRefusal {
    /// The `strings.csv` id the console resolves through `t()`. A `match`, not a
    /// composed `format!("tractor.refused.{...}")`, so `check-strings.mjs` can
    /// see every id a new variant needs a row for.
    pub fn string_id(self) -> &'static str {
        match self {
            TractorRefusal::NoLock => "tractor.refused.no_lock",
            TractorRefusal::OutOfRange => "tractor.refused.out_of_range",
            TractorRefusal::Unpowered => "tractor.refused.unpowered",
            TractorRefusal::Disabled => "tractor.refused.disabled",
        }
    }
}

/// **The coupling-position module.** Where the held target sits, given the
/// operator's transform and the authored coupling offset (issue #1156).
///
/// The whole of the geometry: the offset is in the operator's own frame, so it
/// is rotated by the operator's rotation and added to the operator's
/// translation. Lifted and generalised from the tow rig in
/// `operations::server::move_towed_targets`
/// (`transform.translation + transform.rotation * Vec3::from(capability.tow_offset)`).
///
/// Takes transforms and the offset ONLY. Mass is a later slice's (#1157) helm
/// penalty and never enters here.
pub fn coupled_position(
    operator_translation: Vec3,
    operator_rotation: Quat,
    coupling_offset: Vec3,
) -> Vec3 {
    operator_translation + operator_rotation * coupling_offset
}

/// **The hold verdict.** `Ok(())` when the coupling may form this tick, else the
/// one refusal the console shows (issue #1156).
///
/// Pure: the adapter reads the live world into these scalars and applies the
/// answer. Used at engage time (so "engaging with no lock / out of range /
/// unpowered is refused") and re-run every tick a hold is live (so each
/// interruption drops it).
///
/// # Check order is the console's "most actionable first"
///
/// A knocked-out or unpowered beam cannot grip whatever Tactical points it at,
/// so those are reported before the target-acquisition checks; among the latter,
/// there is no range to a target that was never locked, so `NoLock` precedes
/// `OutOfRange`. When several conditions fail at once the crew are told the one
/// nearest the beam itself.
///
/// `separation` is the distance from the operator to the locked target, or
/// `None` when there is no lock or the locked entity cannot be found — either
/// way there is nothing in range, which is why a missing separation with a
/// present lock still reads as `OutOfRange`.
pub fn hold_status(
    lock: Option<&str>,
    separation: Option<f32>,
    range: f32,
    power_level: u8,
    min_power_level: u8,
    tractor_disabled: bool,
) -> Result<(), TractorRefusal> {
    if tractor_disabled {
        return Err(TractorRefusal::Disabled);
    }
    if power_level < min_power_level {
        return Err(TractorRefusal::Unpowered);
    }
    if lock.is_none() {
        return Err(TractorRefusal::NoLock);
    }
    match separation {
        Some(sep) if sep <= range => Ok(()),
        _ => Err(TractorRefusal::OutOfRange),
    }
}

#[cfg(test)]
#[path = "coupling_tests.rs"]
mod tests;
