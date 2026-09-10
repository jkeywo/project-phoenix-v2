//! How long a GM placement has stood inside a player ship's sensor horizon
//! (issue #1443, PRD #1420 story 4).
//!
//! # The rule this module makes true
//!
//! A GM may take back a placement they have just made — but only while nobody
//! could have seen it. PRD #1420 fixes the cutoff at **two cumulative
//! simulation seconds inside the sensor range of any player ship**, and every
//! word of that is load-bearing:
//!
//! * **cumulative**, not continuous. Leaving the range does not reset the
//!   count; a hull that dips in and out for a second at a time reaches the
//!   cutoff on its second dip.
//! * **simulation** seconds, counted as fixed steps. A paused world contributes
//!   exactly nothing, because a step that never starts cannot count — the same
//!   reasoning [`crate::gm_attention::observe_idle_npcs`] states for the idle
//!   stopwatch, and the reason neither differences a live tick counter.
//! * **union** coverage. Three player ships watching one raider is still one
//!   second per second: [`exposed`] asks whether ANY of them is in range, so an
//!   overlap can never count twice.
//! * **any player ship**, not "the local one". Nothing here reads
//!   [`crate::server_app::LocalShip`] or the local
//!   `ShipClientConfigResource` — both are peer-local, and this counter is
//!   folded into the authoritative digest, so a value that depended on which
//!   hull a peer happened to be flying would be a divergence by construction.
//!
//! Concealment and identification are deliberately NOT consulted. A
//! [`crate::gm_contact::ContactMode::Conceal`] override changes what a console
//! DRAWS; it does not change that the ship was there to be found, and undo is
//! about whether a crew could have seen a thing at all, not about whether they
//! noticed it. Firing and damage are likewise not a second cutoff (PRD #1420 is
//! explicit): they are already exposure, because a ship close enough to shoot
//! is inside somebody's range.
//!
//! # Why the latch is stored rather than recomputed
//!
//! [`SpawnExposure::latched`] is a permanent refusal. Once it closes, no later
//! world state re-opens it: the entity may leave, may be concealed, may become
//! unreachable, and the answer stays no. That cannot be derived from a restored
//! world — the evidence is *history* — so it is captured in the snapshot and
//! folded into the digest alongside the counter that produced it.

use std::collections::BTreeMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::entities::spawner::{EntityTagsSection, EntityUuid};
use crate::ship::state::ShipPhysics;

/// The cutoff, in cumulative simulation seconds.
///
/// A safety rule rather than a designer tunable: PRD #1420 fixes it, and a
/// world that could widen it would be a world where a GM can take back
/// something a crew has been looking at for ten seconds. It is a named constant
/// so the one number appears once, in the module that enforces it.
pub const GM_SPAWN_EXPOSURE_LIMIT_SECS: f32 = 2.0;

/// The cutoff as an exact whole number of simulation ticks at `hz`.
///
/// Rounded to the nearest tick and floored at one, exactly as
/// [`crate::gm_attention::GmAttentionSettings::idle_grace_ticks`] converts its
/// authored grace: the count is in steps, so the boundary has to be too, and a
/// cutoff that rounded to zero would refuse every undo the instant it was
/// placed. A non-finite or non-positive `hz` cannot reach here from a loaded
/// world (`parse_world` bounds `[global] sim_tick_hz`), and falls back to the
/// shipped rate rather than to a degenerate boundary.
pub fn exposure_limit_ticks(hz: f32) -> u64 {
    let hz = if hz.is_finite() && hz > 0.0 {
        f64::from(hz)
    } else {
        f64::from(crate::entities::config::GlobalConfig::default().sim_tick_hz)
    };
    ((f64::from(GM_SPAWN_EXPOSURE_LIMIT_SECS) * hz).round() as u64).max(1)
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// One GM placement's exposure record.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnExposure {
    /// Fixed steps observed with this placement inside some player ship's
    /// sensor range.
    ///
    /// COUNTED, never differenced against [`crate::sim_tick::SimTick`]: a
    /// difference would run through a pause and would take its answer from
    /// whichever frame happened to read the counter.
    #[serde(default)]
    pub ticks: u64,
    /// Whether the permanent refusal has closed. Never cleared.
    #[serde(default, skip_serializing_if = "is_false")]
    pub latched: bool,
}

/// What undo may still do about one GM placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GmSpawnUndoEligibility {
    /// Under the cutoff. The ordinary safe-removal checks decide the rest.
    Eligible,
    /// The cutoff has been reached. Permanently refused.
    Exposed,
    /// Nothing ever watched this placement, so there is no evidence either way
    /// and no honest answer but "not this build's to reverse".
    Unwatched,
}

/// Every GM placement this run has watched, and how long each has been in
/// somebody's sensor range.
///
/// Authoritative: [`crate::gm_action::apply_due_actions`] reads it to decide
/// whether an inverse may run, it is captured in
/// [`crate::snapshot::PhoenixSnapshot`] and it is folded into
/// [`crate::sim_digest::world_digest`] — but, for
/// [`crate::gm_faction::GmFactionOverrides`]' reason, only once a GM has
/// actually placed something, so a run that never uses the surface keeps the
/// digest it had before this landed.
///
/// A `BTreeMap` keyed by the placement's deterministic scenario name
/// ([`crate::gm_spawn::PendingGmSpawn::derive_name`]) rather than by uuid: the
/// name is decided at the canonical apply tick, in `PreUpdate`, while the uuid
/// is not minted until the ordinary trigger pipeline drains the arm in
/// `FixedUpdate`. Keying on something that does not exist yet would leave a
/// window in which a placement was unwatched.
///
/// Bounded by the journal that feeds it: a name is only ever added by an
/// Applied [`crate::gm_action::GmAction::SpawnPaletteEntity`], and
/// [`crate::gm_action::MAX_GM_ACTIONS_PER_RUN`] caps how many of those a run
/// can hold.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmSpawnExposure {
    #[serde(default)]
    watched: BTreeMap<String, SpawnExposure>,
}

impl GmSpawnExposure {
    pub fn is_empty(&self) -> bool {
        self.watched.is_empty()
    }

    pub fn len(&self) -> usize {
        self.watched.len()
    }

    /// Begin watching one placement. Idempotent — re-arming an existing name
    /// must never restart its clock or re-open its latch.
    pub fn watch(&mut self, name: &str) {
        self.watched.entry(name.to_string()).or_default();
    }

    /// This placement's record, if one is being kept.
    pub fn get(&self, name: &str) -> Option<SpawnExposure> {
        self.watched.get(name).copied()
    }

    /// Every watched placement, in stable name order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &SpawnExposure)> {
        self.watched.iter()
    }

    /// What undo may still do about `name`.
    pub fn eligibility(&self, name: &str) -> GmSpawnUndoEligibility {
        match self.watched.get(name) {
            None => GmSpawnUndoEligibility::Unwatched,
            Some(entry) if entry.latched => GmSpawnUndoEligibility::Exposed,
            Some(_) => GmSpawnUndoEligibility::Eligible,
        }
    }

    /// Record one observed step for `name`.
    ///
    /// `exposed` is the union answer for the whole fleet, already collapsed by
    /// the caller, so an overlap arrives here as one step and not as several.
    /// A latched record is never touched again: the count stops at the boundary
    /// it crossed, which keeps the saved number bounded and keeps the moment
    /// the latch closed readable.
    pub fn observe(&mut self, name: &str, exposed: bool, limit_ticks: u64) {
        let Some(entry) = self.watched.get_mut(name) else {
            return;
        };
        if entry.latched || !exposed {
            return;
        }
        entry.ticks = entry.ticks.saturating_add(1);
        entry.latched = entry.ticks >= limit_ticks;
    }
}

/// One placement's exposure as the GM page reads it (issue #1443).
///
/// Milliseconds rather than ticks: the page has no business knowing the
/// simulation's step rate, and "1.4 s of 2.0 s" is the sentence a GM needs.
/// Presentation only — the authoritative facts are [`SpawnExposure`] and the
/// canonical journal, and the reducer re-answers all of it at the apply tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmSpawnExposureStatus {
    /// Cumulative simulation time inside some player ship's sensor range.
    pub exposed_ms: u64,
    /// The cutoff, so the page never hardcodes two seconds of its own.
    pub limit_ms: u64,
    /// Whether the permanent refusal has already closed.
    pub latched: bool,
}

impl GmSpawnExposureStatus {
    /// Present one record at `hz`, clamped so a saturated counter cannot report
    /// more than the cutoff it stopped at.
    pub fn new(entry: SpawnExposure, hz: f32) -> Self {
        let limit_ticks = exposure_limit_ticks(hz);
        let per_tick_ms = f64::from(GM_SPAWN_EXPOSURE_LIMIT_SECS) * 1000.0 / limit_ticks as f64;
        let exposed = (entry.ticks.min(limit_ticks) as f64 * per_tick_ms).round();
        Self {
            exposed_ms: exposed.max(0.0) as u64,
            limit_ms: (f64::from(GM_SPAWN_EXPOSURE_LIMIT_SECS) * 1000.0).round() as u64,
            latched: entry.latched,
        }
    }
}

/// The read-only surfaces [`observe_gm_spawn_exposure`] scans, bundled as one
/// [`SystemParam`]: the player-crewed hulls whose sensors decide exposure, and
/// every identified entity's live position.
///
/// The two overlap (a player hull appears in both) and both are read-only, so
/// Bevy's access check is satisfied and neither needs a filter to exclude the
/// other.
#[derive(SystemParam)]
pub struct GmExposureSurfaces<'w, 's> {
    /// Every player-crewed hull: live position, its OWN authored sensor horizon
    /// and the damage modifiers currently on it.
    ///
    /// "Player-crewed" is the authored `player` tag that
    /// `server_app::world_setup::player_ship_identity` injects onto each fleet
    /// hull at the game-start spawn, or a live [`crate::lockstep::FleetSlotOf`]
    /// seat — the same two facts [`crate::gm_despawn::removable`] protects a
    /// fleet hull by. Both are replicated; neither is peer-local.
    pub players: Query<
        'w,
        's,
        (
            &'static ShipPhysics,
            Option<&'static crate::ai::server::AiProfile>,
            Option<&'static crate::modifiers::ShipModifiers>,
            Option<&'static EntityTagsSection>,
            Has<crate::lockstep::FleetSlotOf>,
        ),
        With<crate::server_app::Ship>,
    >,
    /// Every identified entity's live position. A ship's authoritative position
    /// is on [`ShipPhysics`] (the spawn `Transform` goes stale), which is why
    /// both are read.
    pub placed: Query<
        'w,
        's,
        (
            &'static EntityUuid,
            &'static Transform,
            Option<&'static ShipPhysics>,
        ),
    >,
}

/// Whether `point` is inside any player ship's sensor horizon.
///
/// One `any()`, which is what makes the union free: two overlapping ships and
/// one ship give the same answer, so a step can never be counted twice.
pub fn exposed(point: Vec3, ranges: &[(Vec3, f32)]) -> bool {
    ranges
        .iter()
        .any(|(origin, range)| origin.distance(point) <= *range)
}

/// Advance every watched GM placement's exposure by exactly one simulation
/// step.
///
/// Registered in `FixedLast`, `.before(advance_sim_tick)`, for
/// [`crate::gm_attention::observe_idle_npcs`]' reasons: once per fixed step
/// rather than once per rendered frame, and a paused or stalled world withholds
/// the step entirely so pause needs no special case here.
///
/// Ungated. This is authoritative state that the digest compares, so it must
/// run on every peer whether or not that peer is showing a GM anything.
pub fn observe_gm_spawn_exposure(
    world: Option<Res<crate::world::config::WorldConfig>>,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    surfaces: GmExposureSurfaces,
    mut exposure: ResMut<GmSpawnExposure>,
) {
    // Nothing placed, nothing to watch. Kept first so a world with no GM at all
    // never walks its entity table for this.
    if exposure.is_empty() {
        return;
    }
    let Some(runtime) = runtime.as_deref() else {
        return;
    };
    let hz = world.as_deref().map_or_else(
        || crate::entities::config::GlobalConfig::default().sim_tick_hz,
        |world| world.global.sim_tick_hz,
    );
    let limit = exposure_limit_ticks(hz);
    // The fleet's sensor horizons, collected once per step. `effective_sensor_range`
    // is the simulation's own answer to "how far can THIS hull see", so exposure
    // and the sensors that produce it cannot drift apart.
    let fallback = crate::core::messages::default_sensors_radar_range();
    let default_modifiers = crate::modifiers::ShipModifiers::default();
    let ranges: Vec<(Vec3, f32)> = surfaces
        .players
        .iter()
        .filter(|(_, _, _, tags, fleet)| {
            *fleet
                || tags.is_some_and(|tags| {
                    tags.0
                        .iter()
                        .any(|tag| tag == crate::entities::tags::EntityTag::Player.as_str())
                })
        })
        .map(|(physics, profile, modifiers, _, _)| {
            (
                Vec3::new(physics.x, physics.y, physics.z),
                crate::ship::sensors::effective_sensor_range(
                    profile,
                    fallback,
                    modifiers.unwrap_or(&default_modifiers),
                ),
            )
        })
        .collect();
    // Resolve only the names actually being watched, so the entity table is
    // walked once with a small wanted-set rather than mapped in full.
    let wanted: BTreeMap<&str, &String> = exposure
        .watched
        .keys()
        .filter_map(|name| {
            runtime
                .name_to_uuid
                .get(name)
                .map(|uuid| (uuid.as_str(), name))
        })
        .collect();
    let mut positions: BTreeMap<&String, Vec3> = BTreeMap::new();
    for (uuid, transform, physics) in &surfaces.placed {
        if let Some(name) = wanted.get(uuid.0.as_str()) {
            positions.insert(
                name,
                physics.map_or(transform.translation, |physics| {
                    Vec3::new(physics.x, physics.y, physics.z)
                }),
            );
        }
    }
    // A placement whose entity is not in the world right now — still armed and
    // undrained, already removed, or unloaded with its layer — simply does not
    // accumulate. Its record is kept: the latch is permanent and the journal row
    // that owns it never goes away either.
    let observed: Vec<(String, bool)> = positions
        .into_iter()
        .map(|(name, point)| (name.clone(), exposed(point, &ranges)))
        .collect();
    for (name, seen) in observed {
        exposure.observe(&name, seen, limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cutoff_is_two_simulation_seconds_at_whatever_rate_the_world_runs() {
        assert_eq!(exposure_limit_ticks(60.0), 120);
        assert_eq!(exposure_limit_ticks(30.0), 60);
        assert_eq!(exposure_limit_ticks(240.0), 480);
        // Never zero: a degenerate rate must not refuse every undo outright.
        assert_eq!(exposure_limit_ticks(0.0), 120);
        assert_eq!(exposure_limit_ticks(f32::NAN), 120);
    }

    #[test]
    fn overlapping_ships_count_one_step_not_several() {
        let point = Vec3::new(10.0, 0.0, 0.0);
        let two = [
            (Vec3::ZERO, 100.0),
            (Vec3::new(20.0, 0.0, 0.0), 100.0),
            (Vec3::new(900.0, 0.0, 0.0), 5.0),
        ];
        let mut both = GmSpawnExposure::default();
        both.watch("gm_raider_1");
        both.observe("gm_raider_1", exposed(point, &two), 120);
        let mut one = GmSpawnExposure::default();
        one.watch("gm_raider_1");
        one.observe("gm_raider_1", exposed(point, &two[..1]), 120);
        assert_eq!(both.get("gm_raider_1"), one.get("gm_raider_1"));
        assert_eq!(both.get("gm_raider_1").unwrap().ticks, 1);
    }

    #[test]
    fn leaving_the_range_holds_the_count_instead_of_resetting_it() {
        let mut exposure = GmSpawnExposure::default();
        exposure.watch("gm_raider_1");
        for _ in 0..40 {
            exposure.observe("gm_raider_1", true, 120);
        }
        for _ in 0..500 {
            exposure.observe("gm_raider_1", false, 120);
        }
        assert_eq!(exposure.get("gm_raider_1").unwrap().ticks, 40);
        assert_eq!(
            exposure.eligibility("gm_raider_1"),
            GmSpawnUndoEligibility::Eligible
        );
        for _ in 0..80 {
            exposure.observe("gm_raider_1", true, 120);
        }
        assert_eq!(
            exposure.eligibility("gm_raider_1"),
            GmSpawnUndoEligibility::Exposed,
            "cumulative, not continuous: two separate spells reach the cutoff"
        );
    }

    #[test]
    fn the_latch_never_re_opens() {
        let mut exposure = GmSpawnExposure::default();
        exposure.watch("gm_raider_1");
        for _ in 0..120 {
            exposure.observe("gm_raider_1", true, 120);
        }
        assert!(exposure.get("gm_raider_1").unwrap().latched);
        for _ in 0..10_000 {
            exposure.observe("gm_raider_1", false, 120);
        }
        assert_eq!(exposure.get("gm_raider_1").unwrap().ticks, 120);
        assert!(exposure.get("gm_raider_1").unwrap().latched);
    }

    #[test]
    fn re_arming_a_watched_name_never_restarts_its_clock() {
        let mut exposure = GmSpawnExposure::default();
        exposure.watch("gm_raider_1");
        for _ in 0..119 {
            exposure.observe("gm_raider_1", true, 120);
        }
        exposure.watch("gm_raider_1");
        assert_eq!(exposure.get("gm_raider_1").unwrap().ticks, 119);
        assert!(!exposure.get("gm_raider_1").unwrap().latched);
    }

    #[test]
    fn an_unwatched_placement_is_not_reported_as_exposed() {
        let exposure = GmSpawnExposure::default();
        assert_eq!(
            exposure.eligibility("gm_raider_9"),
            GmSpawnUndoEligibility::Unwatched
        );
    }

    #[test]
    fn the_published_status_reads_in_milliseconds_of_the_stated_cutoff() {
        let status = GmSpawnExposureStatus::new(
            SpawnExposure {
                ticks: 60,
                latched: false,
            },
            60.0,
        );
        assert_eq!(status.exposed_ms, 1000);
        assert_eq!(status.limit_ms, 2000);
        assert!(!status.latched);
        let closed = GmSpawnExposureStatus::new(
            SpawnExposure {
                ticks: 120,
                latched: true,
            },
            60.0,
        );
        assert_eq!(closed.exposed_ms, 2000);
        assert!(closed.latched);
    }
}
