//! The Bevy adapter for debris hazards (issue #1347): the per-entity component,
//! the fixed-tick drift, and the four world flags a scenario hangs its beat on.
//!
//! Split from [`super::threat`] the way `tractor::server` is split from
//! `tractor::coupling` (rule 10): the geometry is pure and unit-tested in
//! isolation, and this file is the part that has to know about entities,
//! transforms and the world flag store.
//!
//! # Three things move here, and only three
//!
//! * **The rock drifts.** [`tick_debris_drift`] integrates each contact's
//!   authored velocity into its `Transform`. Nothing else in the simulation
//!   moves a plain entity — the tractor moves a *held* one — so this is the
//!   whole of debris motion, in one function, in UUID order.
//! * **The plot updates.** [`tick_debris_state`] takes the assessments science
//!   pushed this tick, latches them onto the contacts they name, and raises the
//!   authored flags: *read*, *confirmed*, *urgent*, *struck*.
//! * **Nothing else.** No damage is applied here, no objective is posted, no
//!   entity is destroyed and no message is shown. Every one of those is a
//!   consequence, and issue #1347 puts consequences in the scenario: the world
//!   file's `on_flag_set` handlers own them, so a designer retunes what a strike
//!   costs without editing Rust.
//!
//! # Why the confirmation comes from the SCAN and not from the geometry
//!
//! This module knows perfectly well, every tick, which rocks are on course. It
//! deliberately does not act on that. [`DebrisThreat::confirmed`] rises only
//! when an assessment that science actually took says so, because the whole beat
//! is that somebody has to look: a Backfilled Tactical that engaged rocks the
//! simulation privately knew about would make the Sensors seat decorative, and
//! a human Tactical who locks an unread contact must not have the AI agree with
//! them for reasons the crew were never told.

use bevy::prelude::*;

use crate::effect_queue::EffectQueue;
use crate::entities::spawner::{EntityName, EntityUuid};
use crate::logging::LogFilterConfig;
use crate::world::content::WorldEvent;
use crate::world::server::WorldContentRuntime;

use super::threat::{DebrisAssessment, DebrisConfig, DebrisSaveState};

/// One assessment, resolved by `science::server::tick_scans` and applied here
/// (issue #1347).
///
/// A queue rather than a direct write for [`EffectQueue`]'s own reason: the scan
/// tick is where the reading is composed — that is where the suite, the range
/// and the power all are — but the contact's authoritative threat state belongs
/// on the contact, and `tick_scans` holds the subject query read-only.
#[derive(Clone, Debug, PartialEq)]
pub struct DebrisAssessed {
    /// The assessed contact's UUID — the row key this lands on.
    pub subject_uuid: String,
    /// The tick the reading was taken on, carried so a consumer can dead-reckon
    /// the crew's own number forward rather than inventing a fresh one.
    pub taken_at_tick: u64,
    /// What the reading said.
    pub assessment: DebrisAssessment,
}

/// A moving hazard and everything the crew have worked out about it.
///
/// The authored half (`config`) never changes after spawn; the rest is this
/// run's history of the contact and is what a save has to carry.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct DebrisThreat {
    /// The entity's authored `[debris]` table, as spawned.
    pub config: DebrisConfig,
    /// The last assessment taken of this contact, retained until a fresh one
    /// replaces it. `None` until somebody looks — which is the state the whole
    /// beat turns on.
    pub assessment: Option<DebrisAssessment>,
    /// The `SimTick` that assessment was taken on, so a consumer dead-reckons
    /// the crew's own seconds-to-impact forward instead of reading the
    /// simulation's private truth.
    pub assessed_at_tick: u64,
    /// Whether an assessment has ever come back for this contact. Latches: a
    /// re-scan does not un-read a rock, exactly as `science::scan::scanned_flag`
    /// latches.
    pub assessed: bool,
    /// Whether an assessment has said this contact is on course to strike its
    /// protected asset. **The authoritative threat state** every consumer reads.
    /// Latches, for `assessed`'s reason: a confirmed rock that has been shoved
    /// off course is a thing that happened, and clearing it silently would
    /// retire an objective the crew are still looking at.
    pub confirmed: bool,
    /// Whether a confirmed contact has crossed inside its authored urgency
    /// window. Latches.
    pub urgent: bool,
    /// Whether this contact has reached its protected asset. Latches, and stops
    /// the drift: a rock that has arrived does not arrive twice.
    ///
    /// Also the record a scenario's terminal handlers separate on: a mass that
    /// has landed was not intercepted, however it is broken up afterwards — see
    /// `debris_pending` in `assets/worlds/falling_skyway.toml`.
    pub struck: bool,
    /// The crew's own seconds-to-impact, carried forward to THIS tick.
    ///
    /// Republished every tick by [`tick_debris_state`] from
    /// [`seconds_to_impact_at`](Self::seconds_to_impact_at), so every consumer
    /// reads one number that one system owns rather than each re-deriving it
    /// from `assessed_at_tick` against its own idea of how long a tick is.
    /// `None` until somebody looks, and `None` again when the reading said this
    /// contact never arrives.
    pub reckoned_secs_to_impact: Option<f32>,
}

impl DebrisThreat {
    /// Build the component a `[debris]` table spawns.
    pub fn new(config: DebrisConfig) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }

    /// The half of this contact a save carries — see [`DebrisSaveState`].
    ///
    /// The `translation` is passed in rather than read off the component
    /// because it lives on the entity's `Transform`, which is where
    /// [`tick_debris_drift`] writes it; this method is the one place that says
    /// which fields of the pair travel, and the caller supplies the half it can
    /// see.
    pub fn save_state(&self, translation: Vec3) -> DebrisSaveState {
        DebrisSaveState {
            translation: [translation.x, translation.y, translation.z],
            assessment: self.assessment.clone(),
            assessed_at_tick: self.assessed_at_tick,
            assessed: self.assessed,
            confirmed: self.confirmed,
            urgent: self.urgent,
            struck: self.struck,
        }
    }

    /// Put a saved contact back, latches and all.
    ///
    /// The authored `config` is deliberately untouched — it was re-derived from
    /// the entity template when the restore respawned the mass, and a save that
    /// carried it would be carrying content. `reckoned_secs_to_impact` is left
    /// for [`tick_debris_state`] to republish on the first tick after the
    /// resume, for the same reason it is not saved: it is derived, and this
    /// record holds everything it is derived from.
    ///
    /// The **`Transform`** is not written here. A component's restore cannot
    /// reach its entity's other components, so `snapshot::restore` writes the
    /// translation beside this call, exactly as it writes a resumed ship's
    /// transform beside its `ShipPhysics`.
    pub fn restore(&mut self, state: &DebrisSaveState) {
        self.assessment = state.assessment.clone();
        self.assessed_at_tick = state.assessed_at_tick;
        self.assessed = state.assessed;
        self.confirmed = state.confirmed;
        self.urgent = state.urgent;
        self.struck = state.struck;
        self.reckoned_secs_to_impact = None;
    }

    /// Whether this contact is worth pointing an instrument at right now
    /// (issue #1347) — the test the Sensors seat works a debris field by.
    ///
    /// True while nobody has read it, and true again once the crew's reading has
    /// aged past the contact's authored [`reassess_secs`](DebrisConfig::
    /// reassess_secs). An authored `0.0` therefore means "read once", which is
    /// what makes a seat move on down a field instead of staring at the first
    /// rock: the contact it has just read stops asking for attention, and the
    /// next unknown one is the best candidate the selector can see.
    ///
    /// A STRUCK contact never wants assessing — there is nothing left to learn
    /// about a rock that has already landed — and that test lives here rather
    /// than at the call site so every consumer inherits it.
    pub fn needs_assessment(&self, now_tick: u64, secs_per_tick: f32) -> bool {
        if self.struck {
            return false;
        }
        if !self.assessed {
            return true;
        }
        let cadence = self.config.reassess_secs;
        // `> 0.0` rather than `!= 0.0` so a nonsensical negative (which
        // `validate` already refuses at load) fails closed to "read once"
        // instead of re-scanning every tick forever.
        if !matches!(cadence.partial_cmp(&0.0), Some(std::cmp::Ordering::Greater)) {
            return false;
        }
        let age = now_tick.saturating_sub(self.assessed_at_tick) as f32 * secs_per_tick;
        age >= cadence
    }

    /// The crew's own seconds-to-impact, carried forward to `now_tick` at the
    /// authored tick rate (issue #1347).
    ///
    /// **Dead reckoning, not a fresh reading.** The number a consumer gets is
    /// the one the crew took, minus the time that has passed since they took it
    /// — which is what a tactical plot has always shown, and what makes an old
    /// assessment visibly old rather than silently wrong. `None` when nobody has
    /// looked, or when the reading said this contact never arrives.
    ///
    /// `secs_per_tick` is the caller's fixed-step length rather than a constant,
    /// because the tick rate is authored (`[global] sim_tick_hz`) and a hard
    /// 30 Hz here would quietly disagree with a scenario that authored anything
    /// else. Every caller passes `Time::delta_secs()` read inside `FixedUpdate`,
    /// which IS that step.
    pub fn seconds_to_impact_at(&self, now_tick: u64, secs_per_tick: f32) -> Option<f32> {
        let assessment = self.assessment.as_ref()?;
        let held = assessment.seconds_to_impact?;
        let elapsed = now_tick.saturating_sub(self.assessed_at_tick) as f32 * secs_per_tick;
        let remaining = held - elapsed;
        Some(if remaining > 0.0 { remaining } else { 0.0 })
    }
}

/// Registers the debris systems (issue #1347). Added by `WorldPlugin` alongside
/// `SciencePlugin`, because what promotes a contact to a threat is a scan.
pub struct DebrisPlugin;

impl Plugin for DebrisPlugin {
    fn build(&self, app: &mut App) {
        // The assessment queue this plugin owns and drains (issue #1223's
        // contract): pushed by `science::server::tick_scans`, taken in full by
        // `tick_debris_state` every tick, so it is structurally empty at the
        // fold point and inert to both the digest and the snapshot.
        app.init_resource::<EffectQueue<DebrisAssessed>>();
        {
            use crate::authoritative::{DeclareState, StateClass};
            // `digest-exclusion-classes`
            // (`pasm/spec/architecture/deterministic-simulation.yaml`) is the
            // entity that records the cleared-at-fold reason class, and it is
            // what every sibling drained queue points at — see
            // `civilian::server` and `console::captain::server`. The debris
            // slice's OWN authoritative record is `debris-threat-state`, which
            // `DebrisThreat` names; this queue is not that state, it is the
            // one-tick pipe into it.
            app.declare_state::<EffectQueue<DebrisAssessed>>(
                StateClass::ClearedAtFold,
                "digest-exclusion-classes",
            );
        }
        app.add_systems(
            FixedUpdate,
            (
                // Drift first, so this tick's assessment is taken against this
                // tick's position rather than last tick's. The `.before` edge is
                // what MAKES that true rather than merely intending it: both
                // systems sit in `Modifiers`, and both touch a debris contact's
                // `Transform` — this one mutably, `tick_scans` read-only through
                // the `subjects` query it hands to `debris_subject`. Conflicting
                // access with no edge is an ambiguity the multi-threaded executor
                // resolves however it likes, so without this the projection
                // stamped into `ScanReading::debris` — and with it
                // `seconds_to_impact`, the deadline Tactical ranks on, and the
                // tick the `urgent_flag` rises — could be taken against last
                // tick's position on one host and this tick's on another.
                tick_debris_drift
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .before(crate::science::server::tick_scans),
                // Then the plot, after the scan that may have moved it. The
                // explicit `after` is what makes an assessment land on the same
                // tick the crew asked for it, rather than one behind.
                tick_debris_state
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(crate::science::server::tick_scans)
                    .after(tick_debris_drift),
            ),
        );
    }
}

/// Integrate every debris contact's authored drift into its transform.
///
/// UUID order, for `tick_scans`' reason: two hosts must move the same rock the
/// same distance in the same sequence, and raw query order is archetype order.
/// A contact that has already struck is frozen — see [`DebrisThreat::struck`].
pub fn tick_debris_drift(
    time: Option<Res<Time>>,
    mut debris: Query<(Entity, &EntityUuid, &mut Transform, &DebrisThreat)>,
) {
    let delta_secs = time.map(|t| t.delta_secs()).unwrap_or(0.0);
    if delta_secs <= 0.0 {
        return;
    }
    let mut rows: Vec<(String, Entity)> = debris
        .iter()
        .filter(|(_, _, _, threat)| !threat.struck)
        .map(|(entity, uuid, _, _)| (uuid.0.clone(), entity))
        .collect();
    if rows.is_empty() {
        return;
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    for (_, entity) in rows {
        let Ok((_, _, mut transform, threat)) = debris.get_mut(entity) else {
            continue;
        };
        let drift = threat.config.drift;
        if drift == [0.0, 0.0, 0.0] {
            continue;
        }
        transform.translation.x += drift[0] * delta_secs;
        transform.translation.y += drift[1] * delta_secs;
        transform.translation.z += drift[2] * delta_secs;
    }
}

/// Apply this tick's assessments and raise the authored flags.
///
/// Four transitions, each an ordinary world flag so a scenario reads them with
/// the vocabulary it already has (`on_flag_set(..)`), and each raised at most
/// once because [`FlagStore::set_flag`](crate::world::flags::FlagStore::set_flag)
/// reports its own `(before, after)` — a re-scan of a contact the crew already
/// read emits nothing, exactly as re-scanning a depot does.
///
/// The IMPACT test is the simulation's own truth rather than the crew's: a rock
/// arrives whether anybody was watching or not, and a scenario that only
/// punished the crews who looked would be teaching the wrong lesson.
pub fn tick_debris_state(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    time: Option<Res<Time>>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    mut queue: ResMut<EffectQueue<DebrisAssessed>>,
    mut debris: Query<(Entity, &EntityUuid, &Transform, &mut DebrisThreat)>,
    // The protected assets, resolved by their authored `[[entity]] name` — the
    // handle a scenario types. Deliberately the same table `tick_scans` resolves
    // a subject through, so the two agree about what "the depot" means.
    protected: Query<(&Transform, &EntityName)>,
    log: Option<Res<LogFilterConfig>>,
) {
    let now_tick = tick.map(|t| t.0).unwrap_or(0);
    let secs_per_tick = time.map(|t| t.delta_secs()).unwrap_or(0.0);
    let assessed = if queue.0.is_empty() {
        Vec::new()
    } else {
        std::mem::take(&mut queue.0)
    };
    if debris.is_empty() {
        return;
    }

    let mut rows: Vec<(String, Entity)> = debris
        .iter()
        .map(|(entity, uuid, _, _)| (uuid.0.clone(), entity))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    // Raised names are collected and applied together AFTER the walk, so the
    // flag store is borrowed once rather than per contact, and so the world
    // events land in contact-UUID order however the queue arrived.
    let mut raise: Vec<(String, &'static str, String)> = Vec::new();

    for (uuid, entity) in rows {
        let Ok((_, _, transform, mut threat)) = debris.get_mut(entity) else {
            continue;
        };

        // ── What the crew learned this tick ─────────────────────────────────
        //
        // The LAST assessment of this contact in the queue wins: two ships
        // reading the same rock on one tick is an ordinary thing for a fleet to
        // do, and the queue is already in a deterministic order because
        // `tick_scans` walks its ships in UUID order.
        if let Some(record) = assessed.iter().rfind(|a| a.subject_uuid == uuid) {
            threat.assessment = Some(record.assessment.clone());
            threat.assessed_at_tick = record.taken_at_tick;
            if !threat.assessed {
                threat.assessed = true;
                raise.push((uuid.clone(), "read", threat.config.assessed_flag.clone()));
            }
            if record.assessment.on_collision_course && !threat.confirmed {
                threat.confirmed = true;
                raise.push((
                    uuid.clone(),
                    "confirmed",
                    threat.config.confirmed_flag.clone(),
                ));
            }
        }

        // ── Running out of room ─────────────────────────────────────────────
        //
        // Urgency is read off the CREW's number, dead-reckoned — a rock nobody
        // has assessed is never urgent, however close it is, because urgency is
        // a statement about the firing problem and there is no firing problem
        // until somebody has posed one.
        //
        // The reckoned number is republished here, on the tick it is true, so
        // this system is its ONE owner: the Tactical selector reads the field
        // rather than re-deriving it against its own idea of how long a tick is.
        let remaining = threat.seconds_to_impact_at(now_tick, secs_per_tick);
        // A `Mut` read before the write, so a contact whose deadline has not
        // moved this tick costs no change-detection mark.
        if threat.reckoned_secs_to_impact != remaining {
            threat.reckoned_secs_to_impact = remaining;
        }
        if threat.confirmed && !threat.urgent {
            if let Some(remaining) = remaining {
                if remaining <= threat.config.urgent_secs {
                    threat.urgent = true;
                    raise.push((uuid.clone(), "urgent", threat.config.urgent_flag.clone()));
                }
            }
        }

        // ── Arrival ─────────────────────────────────────────────────────────
        if threat.struck || threat.config.protected_target.is_empty() {
            continue;
        }
        let Some(asset) = protected
            .iter()
            .find(|(_, name)| name.0 == threat.config.protected_target)
            .map(|(tf, _)| tf.translation)
        else {
            // The asset left the world — destroyed, or never spawned. The rock
            // keeps drifting and can no longer strike anything, which is the
            // honest outcome: there is nothing there to hit.
            continue;
        };
        let dx = transform.translation.x - asset.x;
        let dz = transform.translation.z - asset.z;
        let radius = threat.config.impact_radius;
        if dx * dx + dz * dz <= radius * radius {
            threat.struck = true;
            raise.push((uuid.clone(), "struck", threat.config.impact_flag.clone()));
        }
    }

    if raise.is_empty() {
        return;
    }
    let Some(runtime) = runtime.as_deref_mut() else {
        return;
    };
    for (uuid, transition, flag) in raise {
        if flag.is_empty() {
            continue;
        }
        let (before, after) = runtime.flags.set_flag(&flag);
        if (before != 0) == (after != 0) {
            continue;
        }
        crate::pdebug!(
            log,
            crate::logging::LogCat::Sensors,
            "debris {uuid} {transition}: {flag} raised"
        );
        runtime.pending_world_events.push(WorldEvent::FlagSet {
            name: flag,
            origin_layer: None,
        });
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
