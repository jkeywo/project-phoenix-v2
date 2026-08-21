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
mod tests {
    use super::*;
    use crate::infrastructure::condition::{
        InfrastructureConfig, InfrastructureState, ThresholdConfig,
    };

    fn cfg(kind: HeldResponseKind) -> HeldResponseConfig {
        HeldResponseConfig {
            kind,
            recover_per_sec: None,
            offset: None,
            distance: None,
        }
    }

    /// A failing structure that loses 6 condition points a second and whose
    /// `holding` flag falls below 40 % and returns at 60 %.
    fn failing_structure(start: f32) -> InfrastructureState {
        InfrastructureState::from_config(&InfrastructureConfig {
            condition_max: 100.0,
            condition: Some(start),
            decay_per_sec: 6.0,
            thresholds: vec![ThresholdConfig {
                label: None,
                flag: "holding".to_string(),
                fails_below: 0.4,
                restores_above: Some(0.6),
            }],
            ..Default::default()
        })
    }

    /// One ordinary (unheld) tick: the infra tick's automatic decline.
    fn decline_one_tick(state: &mut InfrastructureState, dt: f32) {
        let decay = state.decay_per_sec() * dt;
        if decay > 0.0 {
            state.degrade(decay);
        }
    }

    /// One HELD tick: the infra tick's automatic decline THEN the held-response
    /// adjustment, exactly as `tick_infrastructure_condition` applies them (decay
    /// first, queued adjustments second) in one tick.
    fn held_one_tick(state: &mut InfrastructureState, response: &HeldResponse, dt: f32) {
        let decay_per_sec = state.decay_per_sec();
        decline_one_tick(state, dt);
        let delta = condition_delta(response, decay_per_sec, dt);
        state.apply_delta(delta);
    }

    // ── arrest-decline against the condition track ───────────────────────────

    #[test]
    fn arrest_decline_holds_a_failing_structure_steady_at_a_zero_rate() {
        // recover_per_sec = 0: the decline is cancelled and nothing added, so a
        // structure that would lose six points a second holds exactly still.
        let response = HeldResponse::ArrestDecline {
            recover_per_sec: 0.0,
        };
        let mut state = failing_structure(50.0);
        for _ in 0..120 {
            held_one_tick(&mut state, &response, 1.0 / 60.0);
        }
        assert!(
            (state.condition() - 50.0).abs() < 0.5,
            "two held seconds must arrest the decline, leaving the structure where it started \
             (would be 50 - 12 = 38 unheld), got {}",
            state.condition()
        );
    }

    #[test]
    fn arrest_decline_recovers_at_the_authored_rate_and_crosses_the_targets_own_threshold() {
        // Start below the failure point so the flag is down, then hold and
        // recover at a rate that carries it up across the authored restore point.
        let response = HeldResponse::ArrestDecline {
            recover_per_sec: 20.0,
        };
        let mut state = failing_structure(37.0);
        assert_eq!(
            state.flag("holding"),
            Some(false),
            "precondition: 37 % starts below the 40 % failure point, flag down"
        );
        let mut crossings = 0;
        for _ in 0..120 {
            let before = state.decay_per_sec() * (1.0 / 60.0);
            decline_one_tick(&mut state, 1.0 / 60.0);
            let _ = before;
            let delta = condition_delta(&response, 6.0, 1.0 / 60.0);
            crossings += state.apply_delta(delta).len();
        }
        // Net +20/s for two seconds off 37 → ~77, clamped under the ceiling.
        assert!(
            (state.condition() - 77.0).abs() < 1.0,
            "arrest-decline nets the authored +20/s over the arrested decline, got {}",
            state.condition()
        );
        assert_eq!(
            state.flag("holding"),
            Some(true),
            "…and the recovered condition crossed the target's own 60 % restore point, setting \
             the operational flag a scenario reads"
        );
        assert_eq!(
            crossings, 1,
            "the crossing is reported exactly once, on the tick it carries over the restore point"
        );
    }

    #[test]
    fn releasing_arrest_decline_resumes_the_ordinary_decline_on_the_next_tick() {
        let response = HeldResponse::ArrestDecline {
            recover_per_sec: 0.0,
        };
        let mut state = failing_structure(50.0);
        for _ in 0..60 {
            held_one_tick(&mut state, &response, 1.0 / 60.0);
        }
        let held_value = state.condition();
        assert!(
            (held_value - 50.0).abs() < 0.5,
            "held for a second it stayed put, got {held_value}"
        );
        // Release: no more held ticks, only the ordinary decline.
        for _ in 0..60 {
            decline_one_tick(&mut state, 1.0 / 60.0);
        }
        assert!(
            state.condition() < held_value - 5.0,
            "a released structure resumes its ordinary decline — six points off in the second \
             after release, got a drop from {held_value} to {}",
            state.condition()
        );
    }

    // ── follow / station-keep leave the condition track alone ────────────────

    #[test]
    fn follow_and_station_keep_do_not_touch_the_condition_track() {
        for response in [HeldResponse::Follow, HeldResponse::StationKeep] {
            assert_eq!(
                condition_delta(&response, 6.0, 1.0 / 60.0),
                0.0,
                "{response:?} banks no condition — holding a derelict or station-keeping a craft \
                 is a geometry response, not a condition one"
            );
            // A structure held under one of these goes on declining ordinarily:
            // the response arrests nothing.
            let mut state = failing_structure(50.0);
            for _ in 0..120 {
                held_one_tick(&mut state, &response, 1.0 / 60.0);
            }
            assert!(
                (state.condition() - 38.0).abs() < 0.5,
                "{response:?} does not arrest the decline: 50 - 12 = 38, got {}",
                state.condition()
            );
        }
    }

    // ── formation-keep is a geometry response, distinct from station-keep ─────

    #[test]
    fn formation_keep_banks_no_condition_but_rides_its_own_authored_slot() {
        let response = HeldResponse::FormationKeep {
            offset: Vec3::new(0.0, 0.0, 1.0),
            distance: 200.0,
        };
        assert_eq!(
            condition_delta(&response, 6.0, 1.0 / 60.0),
            0.0,
            "formation-keep is a geometry response and leaves the condition track alone"
        );
        // Same declining structure, held in formation: it keeps declining.
        let mut state = failing_structure(50.0);
        for _ in 0..120 {
            held_one_tick(&mut state, &response, 1.0 / 60.0);
        }
        assert!(
            (state.condition() - 38.0).abs() < 0.5,
            "formation-keep arrests nothing, got {}",
            state.condition()
        );
    }

    #[test]
    fn formation_keep_rides_its_own_slot_where_the_others_ride_the_operator_rig() {
        let operator_rig = Vec3::new(0.0, 0.0, -120.0);
        // Station-keep / follow / arrest-decline all ride the operator's rig.
        for response in [
            HeldResponse::StationKeep,
            HeldResponse::Follow,
            HeldResponse::ArrestDecline {
                recover_per_sec: 3.0,
            },
        ] {
            assert_eq!(
                held_offset(&response, operator_rig),
                operator_rig,
                "{response:?} rides the operator's authored coupling rig"
            );
        }
        // Formation-keep rides its OWN slot: 200 units along +Z, distinct from
        // the operator's 120-astern rig.
        let formation = HeldResponse::FormationKeep {
            offset: Vec3::new(0.0, 0.0, 5.0),
            distance: 200.0,
        };
        let slot = held_offset(&formation, operator_rig);
        assert!(
            (slot - Vec3::new(0.0, 0.0, 200.0)).length() < 1e-3,
            "formation-keep rides 200 units along its (un-normalised) +Z bearing, got {slot:?}"
        );
        assert_ne!(
            slot, operator_rig,
            "…which is distinct from station-keeping it in place on the operator's rig"
        );
    }

    // ── config validation ────────────────────────────────────────────────────

    #[test]
    fn arrest_decline_requires_a_recover_rate_and_forbids_formation_fields() {
        let mut c = cfg(HeldResponseKind::ArrestDecline);
        assert!(
            c.validate().is_err(),
            "arrest-decline with no recover_per_sec is a load error"
        );
        c.recover_per_sec = Some(5.0);
        c.validate().expect("arrest-decline with a rate is valid");
        c.offset = Some([0.0, 0.0, 1.0]);
        assert!(
            c.validate().is_err(),
            "a formation offset on an arrest-decline is a load error"
        );
    }

    #[test]
    fn formation_keep_requires_a_non_zero_offset_and_a_positive_distance() {
        let mut c = cfg(HeldResponseKind::FormationKeep);
        assert!(
            c.validate().is_err(),
            "no offset, no distance: a load error"
        );
        c.offset = Some([0.0, 0.0, 0.0]);
        c.distance = Some(200.0);
        assert!(
            c.validate().is_err(),
            "a zero-length bearing would hold the target on its operator"
        );
        c.offset = Some([0.0, 0.0, 1.0]);
        c.distance = Some(0.0);
        assert!(c.validate().is_err(), "a non-positive distance is rejected");
        c.distance = Some(200.0);
        c.validate().expect("a real bearing and distance validate");
        c.recover_per_sec = Some(1.0);
        assert!(
            c.validate().is_err(),
            "a recover rate on a formation-keep is a load error"
        );
    }

    #[test]
    fn follow_and_station_keep_forbid_every_per_kind_field() {
        for kind in [HeldResponseKind::Follow, HeldResponseKind::StationKeep] {
            let mut c = cfg(kind);
            c.validate().expect("a bare follow/station-keep validates");
            c.recover_per_sec = Some(1.0);
            assert!(
                c.validate().is_err(),
                "{kind:?} authors no recover_per_sec — that belongs to arrest-decline"
            );
            let mut c = cfg(kind);
            c.offset = Some([0.0, 0.0, 1.0]);
            assert!(
                c.validate().is_err(),
                "{kind:?} authors no formation offset"
            );
        }
    }

    #[test]
    fn the_vocabulary_round_trips_through_toml() {
        let authored = r#"
kind = "arrest-decline"
recover_per_sec = 8.0
"#;
        let parsed: HeldResponseConfig = toml::from_str(authored).expect("arrest-decline parses");
        assert_eq!(parsed.kind, HeldResponseKind::ArrestDecline);
        assert_eq!(parsed.recover_per_sec, Some(8.0));
        parsed.validate().expect("valid");

        let formation = r#"
kind = "formation-keep"
offset = [0.0, 0.0, 60.0]
distance = 60.0
"#;
        let parsed: HeldResponseConfig = toml::from_str(formation).expect("formation-keep parses");
        assert_eq!(parsed.kind, HeldResponseKind::FormationKeep);
        assert_eq!(parsed.offset, Some([0.0, 0.0, 60.0]));
        assert_eq!(parsed.distance, Some(60.0));
        parsed.validate().expect("valid");

        // The bare vocabulary — a target that says only which response it wants.
        let station = toml::from_str::<HeldResponseConfig>("kind = \"station-keep\"")
            .expect("station-keep parses");
        assert_eq!(station.kind, HeldResponseKind::StationKeep);
        station.validate().expect("valid");
    }

    #[test]
    fn an_unknown_field_is_a_parse_error_rather_than_a_silently_ignored_typo() {
        let err = toml::from_str::<HeldResponseConfig>("kind = \"follow\"\nrecovr_per_sec = 5.0")
            .expect_err("a misspelt field must not be swallowed");
        assert!(
            err.to_string().contains("recovr_per_sec"),
            "the error must name the offending field, got {err}"
        );
    }
}
