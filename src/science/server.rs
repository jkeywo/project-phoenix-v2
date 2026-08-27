//! Bevy adapter for the science scan (issue #1032).
//!
//! One component, two systems, and no decisions of its own. What a scan says is
//! [`super::scan`]'s to derive; everything here gathers the live inputs that
//! pure sibling cannot reach — where the ship is, what the grid is holding,
//! which hazard band it is sitting in, what the subject's condition track
//! currently says — stores what comes back, and publishes it.
//!
//! # The command rides a real, station-owned system
//!
//! [`tick_scans`] reads `AdmittedCommands` for the **`sensors`** system, which
//! is a genuine `[[system]]` a station owns, a console can be damaged out of,
//! and admission already gates on station tenure. So a scan takes exactly the
//! path `SetScienceTarget` takes, and admission — not this file — decides who
//! may ask. Nothing here branches on whether a human or the ship's own AI sent
//! it (AGENTS.md rule 6); by the time a command is in `AdmittedCommands` there
//! is nothing left on it that could say.
//!
//! `sensors` is also the system the destroyer's **captain** station owns
//! (`gui/destroyer/captain.html` resolves its sensor panels from it), which is
//! why the readout lands on that console without a second routing rule.
//!
//! # Why this runs in `SimSet::Modifiers` rather than `SimSet::Input`
//!
//! A scan is instantaneous — there is no hold to open on one tick and advance
//! on the next — and every input it takes is a *this tick* reading:
//!
//! * the ship's `Transform`, which `sync_ship_position` mirrors out of
//!   `ShipPhysics` back in `SimSet::Physics`, exactly as the tractor tick
//!   reads it;
//! * `RegionMembership`, recomputed in `SimSet::Physics`;
//! * and the subject's condition track, which
//!   [`tick_infrastructure_condition`](crate::infrastructure::tick_infrastructure_condition)
//!   advances in this same set — so this system is explicitly ordered
//!   `.after` it. A scan taken on the tick a repair team finishes reads the
//!   repaired number, which is the only answer a crew watching both consoles
//!   would accept.
//!
//! `AdmittedCommands` is cleared and refilled once per tick, before
//! `SimSet::Input`, so a `Modifiers` reader sees the whole tick's set.
//!
//! # The one place a reading becomes something a scenario can see (issue #1038)
//!
//! [`tick_scans`] is also where [`scanned_flag`] is mirrored into the base-world
//! flag store, for [`tick_infrastructure_condition`]'s reason spelled out at
//! length in `infrastructure/server.rs`: a fact is only observable if the code
//! that produces it is also the code that mirrors it, so there is **one write
//! site** rather than a second system re-deriving "has this been scanned" from
//! the record a tick later. Like a threshold crossing, the flag is written here
//! and a `FlagSet` is pushed onto `WorldContentRuntime::pending_world_events`,
//! which `collect_world_events` drains at the top of the next tick's
//! `SimSet::Physics` — so an `on_flag_set` hook fires one tick after the reading
//! lands, on the same one-tick bridge #1025's crossings ride.
//!
//! Nothing new is registered by that: the flag store is state the world plugin
//! already owns, snapshots and censuses. See [`scanned_flag`] for why the flag
//! exists when the reading is already stored, and why it latches.
//!
//! # Determinism
//!
//! Ships are walked in UUID order, never archetype order, and the subject is
//! looked up by UUID rather than by whichever entity a query happened to yield
//! first — the same rule [`crate::sim_digest`], #1025 and #1026 apply to their
//! own walks.

use bevy::prelude::*;

use crate::core::messages::{
    AdmittedCommands, InfrastructureSnapshot, PowerGroupId, ScanBlackboard, ScanReadingSnapshot,
    SystemBlackboard, SystemControlPayload, SystemId,
};
use crate::dossier::SubjectCondition;
use crate::entities::spawner::{EntityName, EntityUuid};
use crate::infrastructure::InfrastructureCondition;
use crate::logging::LogFilterConfig;
use crate::regions::effects::RegionEffectName;
use crate::science::scan::{
    derive, scanned_flag, ScanConditions, ScanConfig, ScanReading, ScanRefusal, ScanSubject,
};
use crate::ship::power::ShipPowerSystem;
use crate::world::content::WorldEvent;
use crate::world::server::WorldContentRuntime;

/// The blackboard channel key a ship's last scan is published under.
///
/// **Not a system id.** No `[[system]]` block declares it, no station owns it,
/// it registers no `ControlSource` and no `ControlSystem` message may target it
/// — the thing that can be commanded and damaged is `sensors`, and that is what
/// the scan command targets. This is where the *result* is carried, and it is
/// a channel for `operations`' reason: the blackboard map and the
/// `BlackboardUpdate` wire message are typed as `SystemId`.
///
/// A field on `SensorsBlackboard` was the obvious alternative and was rejected:
/// that payload is the radar's live configuration, republished as contacts
/// move, and hanging a rarely-changing reading off it would re-broadcast the
/// reading every time a blip did.
pub const SCAN_BLACKBOARD_KEY: &str = "scan";

/// The blackboard channel key as a [`SystemId`].
pub fn scan_blackboard_key() -> SystemId {
    SystemId(SCAN_BLACKBOARD_KEY.to_string())
}

/// Everything one ship knows about scanning.
///
/// Authoritative per-ship simulation state. The **reading** is the part that
/// cannot be re-derived: it is what the crew saw when they looked, at the
/// fidelity their range and power bought them at that tick, and no amount of
/// re-folding the world afterwards recovers it — the structure has moved on
/// since, which is the entire point of taking a reading rather than watching a
/// gauge. A run that scanned the skyhook and a run that did not are different
/// runs, exactly as #1031's evidence log is.
///
/// One component rather than two because the three fields move together and are
/// read together: the publisher wants all of them and a save has to restore all
/// of them.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct ShipScanRecord {
    /// The hull's authored `[scan]` table, as spawned.
    pub config: ScanConfig,
    /// The last reading this ship took, retained until the next one replaces
    /// it. Retained past the moment of the scan because a console that emptied
    /// the instant the sweep finished would be a console nobody could read.
    pub last: Option<ScanReading>,
    /// Why the most recent scan returned nothing, if it did. Cleared by the
    /// next scan that succeeds, so a refusal and a reading are never both
    /// current.
    pub refusal: Option<ScanRefusal>,
}

/// The mutable part of a ship's scan record, as a save carries it.
///
/// The authored `config` is deliberately **not** in here: it is re-derived
/// from the hull's template on the tick the ship spawns, and a
/// save whose hull's `[scan]` table has since changed is refused as
/// content-moved long before this is read — so writing it would put content
/// into a save that `content_digest` is the thing answerable for.
///
/// The two fields that ARE here have to come back **together**: restore the
/// reading without the refusal and a resumed console shows an answer beside a
/// stale complaint; restore the refusal without the reading and a crew who had
/// just scanned the skyhook resume having apparently failed to.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScanSaveState {
    /// The last reading taken, whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<ScanReading>,
    /// Why the last scan returned nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<ScanRefusal>,
}

impl ShipScanRecord {
    /// Project the record onto what a save carries.
    pub fn save_state(&self) -> ScanSaveState {
        ScanSaveState {
            last: self.last.clone(),
            refusal: self.refusal,
        }
    }

    /// Take a save's state back, leaving the spawned `[scan]` table alone.
    pub fn restore(&mut self, state: &ScanSaveState) {
        self.last = state.last.clone();
        self.refusal = state.refusal;
    }
}

/// Registers the scan systems. Added by `WorldPlugin` alongside
/// `InfrastructurePlugin`, because what it reads is its state.
pub struct SciencePlugin;

impl Plugin for SciencePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                // After the condition tick, so a scan taken on the tick a
                // repair lands reads the repaired number. See the module doc.
                tick_scans
                    .in_set(crate::sim_sets::SimSet::Modifiers)
                    .after(crate::infrastructure::tick_infrastructure_condition),
                publish_scan_blackboard.in_set(crate::sim_sets::SimSet::Publish),
            ),
        );
    }
}

/// Take every scan asked for this tick.
///
/// Per ship, in UUID order: pull this tick's admitted `ScanTarget` commands off
/// the `sensors` system, resolve the named subject, gather the live conditions,
/// and store whatever the pure derivation returns.
///
/// A hull that authored no `[scan]` table still owes the console an answer when
/// it is asked to scan, so it gets the record holding
/// [`ScanRefusal::NotCapable`] rather than having the command dropped on the
/// floor.
///
/// Every reading that comes back also raises the subject's [`scanned_flag`] —
/// see the module docs for why the mirror is written here and nowhere else.
#[allow(clippy::too_many_arguments)]
pub fn tick_scans(
    tick: Option<Res<crate::sim_tick::SimTick>>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    membership: Option<Res<crate::regions::server::RegionMembership>>,
    mut ships: Query<
        (
            Entity,
            &EntityUuid,
            &Transform,
            &AdmittedCommands,
            Option<&ShipPowerSystem>,
            Option<&mut ShipScanRecord>,
        ),
        With<crate::server_app::Ship>,
    >,
    subjects: Query<(
        &EntityUuid,
        &Transform,
        Option<&EntityName>,
        Option<&InfrastructureCondition>,
        // The world's own `[[entity]] id` — the handle #1038's mirror flag is
        // keyed on, because a scenario can type it and a minted UUID is not
        // something any author has ever seen. See `scanned_flag`.
        Option<&crate::entities::spawner::EntityId>,
        // Authored mass (issue #1154). `Option` rather than required: every
        // entity `spawn_entity` produces carries this unconditionally, but a
        // handful of test fixtures build a subject entity by hand without
        // going through the real spawner, and `subject_mass` below falls
        // those back to the same documented default a bare TOML gets — never
        // to zero.
        Option<&crate::entities::spawner::EntityMass>,
    )>,
    region_effects: Query<&crate::entities::spawner::RegionEffectsSection>,
    mut commands: Commands,
    log: Option<Res<LogFilterConfig>>,
) {
    let now_tick = tick.map(|t| t.0).unwrap_or(0);

    // UUID order, not archetype order: two hosts must take the same ship's
    // scans in the same sequence, because each one overwrites that ship's
    // single stored reading.
    let mut rows: Vec<(String, Entity)> = ships
        .iter()
        .map(|(entity, uuid, ..)| (uuid.0.clone(), entity))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.index().cmp(&b.1.index())));

    for (_, entity) in rows {
        let Ok((entity, _, transform, admitted, power, record)) = ships.get_mut(entity) else {
            continue;
        };
        let requested: Vec<String> = admitted
            .for_target(crate::ship::system_registry::SENSORS_SYSTEM_ID)
            .filter_map(|cmd| match &cmd.payload {
                SystemControlPayload::ScanTarget { uuid } => Some(uuid.clone()),
                _ => None,
            })
            .collect();
        if requested.is_empty() {
            continue;
        }
        let Some(mut record) = record else {
            // No `[scan]` table at all. Insert the record holding the refusal
            // rather than saying nothing; it lands a tick later, which no
            // console can see.
            commands.entity(entity).insert(ShipScanRecord {
                refusal: Some(ScanRefusal::NotCapable),
                ..Default::default()
            });
            continue;
        };
        let effects = operator_region_effects(membership.as_deref(), &region_effects, entity);
        let ship_pos = transform.translation;

        for target_uuid in requested {
            let found = subjects.iter().find(|(uuid, ..)| uuid.0 == target_uuid);
            let Some((_, subject_transform, name, condition, authored_id, mass)) = found else {
                record.last = None;
                record.refusal = Some(ScanRefusal::NoSuchTarget);
                crate::pwarn!(
                    log,
                    crate::logging::LogCat::Sensors,
                    entity = entity,
                    "scan refused: no entity in this world answers to '{target_uuid}'"
                );
                continue;
            };
            let subject = ScanSubject {
                uuid: target_uuid.clone(),
                name: name.map(|n| n.0.clone()).unwrap_or_default(),
                condition: subject_condition(condition),
                mass: subject_mass(mass),
            };
            let conditions = ScanConditions {
                distance: ship_pos.distance(subject_transform.translation),
                // No power grid means no power *constraint* — the ceiling is
                // absent, not zero. The reading #1026 takes for a bare fixture,
                // and for the same reason: failing a hull for a component it
                // never had would make every test rig unscannable.
                power_level: match power {
                    Some(power) => power
                        .0
                        .level_for(&PowerGroupId(record.config.power_group.clone())),
                    None => u8::MAX,
                },
                power_locked: power.map(|power| power.0.locked()).unwrap_or(false),
                region_effects: effects.clone(),
            };

            match derive(&record.config, &subject, &conditions, now_tick) {
                Ok(reading) => {
                    crate::pdebug!(
                        log,
                        crate::logging::LogCat::Sensors,
                        entity = entity,
                        "scan of {target_uuid} returned at band {} ({:.0} units)",
                        reading.band,
                        conditions.distance
                    );
                    record.last = Some(reading);
                    record.refusal = None;
                    if let Some(authored_id) = authored_id {
                        mirror_scanned(runtime.as_deref_mut(), &authored_id.0, &log);
                    }
                }
                Err(refusal) => {
                    crate::pdebug!(
                        log,
                        crate::logging::LogCat::Sensors,
                        entity = entity,
                        "scan of {target_uuid} refused: {}",
                        refusal.string_id()
                    );
                    record.last = None;
                    record.refusal = Some(refusal);
                }
            }
        }
    }
}

/// Latch "this crew have read that structure" into the base-world flag store,
/// and queue the world event a scenario hangs its beat on.
///
/// The transition is decided from the store's own `(before, after)` rather than
/// from "this is a scan", which is
/// [`mirror_flags`](crate::infrastructure::server) exactly: re-scanning the same
/// structure is an ordinary thing for a crew to do and must not emit a second
/// `FlagSet` for a bit that was already up, in the same way a re-append of a
/// finding is a no-op in [`EvidenceLog`](crate::dossier::EvidenceLog).
///
/// A world with no `WorldContentRuntime` — every bare-`App` fixture — takes the
/// `None` arm and scans exactly as it did before this existed.
fn mirror_scanned(
    runtime: Option<&mut WorldContentRuntime>,
    subject_id: &str,
    log: &Option<Res<LogFilterConfig>>,
) {
    let Some(runtime) = runtime else {
        return;
    };
    let flag = scanned_flag(subject_id);
    let (before, after) = runtime.flags.set_flag(&flag);
    if (before != 0) == (after != 0) {
        return;
    }
    crate::pdebug!(
        log,
        crate::logging::LogCat::Sensors,
        "scan mirror: {flag} raised — this crew have now read {subject_id}"
    );
    runtime.pending_world_events.push(WorldEvent::FlagSet {
        name: flag,
        origin_layer: None,
    });
}

/// The subject's published condition track, paired with the labels its scenario
/// authored — **and the only path a condition reaches a scan by**.
///
/// [`InfrastructureSnapshot::from_state`] is #1025's publish gate, so a
/// structure the scenario keeps off the wire yields `None` here and the
/// derivation refuses it. Lifted out as its own function so that gate is one
/// readable line rather than a clause inside the tick, and so it is visibly the
/// same construction `dossier::server` makes.
fn subject_condition(condition: Option<&InfrastructureCondition>) -> Option<SubjectCondition> {
    let infra = condition?;
    let published = InfrastructureSnapshot::from_state(&infra.0)?;
    Some(SubjectCondition::from_published(
        &published,
        |flag| {
            infra
                .0
                .thresholds()
                .iter()
                .find(|t| t.flag == flag)
                .and_then(|t| t.label.clone())
        },
        |capacity| {
            infra
                .0
                .capacities()
                .iter()
                .find(|c| c.id == capacity)
                .and_then(|c| c.label.clone())
        },
    ))
}

/// The subject's authored mass (issue #1154), off its `EntityMass` component.
///
/// Falls back to [`crate::entities::config::DEFAULT_ENTITY_MASS`] — never to
/// `0.0` — for the handful of test fixtures that build a subject entity by
/// hand rather than through [`crate::entities::spawner::spawn_entity`], the
/// only path that inserts the component. Every entity the real spawner
/// produces carries `EntityMass` unconditionally, so this arm is untaken in
/// production.
fn subject_mass(mass: Option<&crate::entities::spawner::EntityMass>) -> f32 {
    mass.map(|m| m.0)
        .unwrap_or(crate::entities::config::DEFAULT_ENTITY_MASS)
}

/// Which authored region effects the scanning hull is standing in,
/// deduplicated and in a fixed order.
///
/// Sorted by declaration order rather than by whichever region entity the
/// membership set happened to yield first, so that two hosts that spawned the
/// same bands in different orders hand the pure module the same list.
fn operator_region_effects(
    membership: Option<&crate::regions::server::RegionMembership>,
    region_effects: &Query<&crate::entities::spawner::RegionEffectsSection>,
    operator: Entity,
) -> Vec<RegionEffectName> {
    let Some(regions) = membership.and_then(|m| m.inside.get(&operator)) else {
        return Vec::new();
    };
    let mut names: Vec<RegionEffectName> = regions
        .iter()
        .filter_map(|region| region_effects.get(*region).ok())
        .flat_map(|effects| {
            effects
                .0
                .iter()
                .map(crate::regions::effects::region_effect_name)
        })
        .collect();
    names.sort_by_key(|name| {
        RegionEffectName::ALL
            .iter()
            .position(|candidate| candidate == name)
            .unwrap_or(usize::MAX)
    });
    names.dedup();
    names
}

/// Publish each scanning ship's last reading.
///
/// Only ships that carry [`ShipScanRecord`] publish one, so a world whose hulls
/// author no `[scan]` puts exactly the payload on the wire it did before this
/// existed. Written only when the picture actually changed —
/// `ShipSystemBlackboards` feeds the diffed `BlackboardUpdate` broadcast, so a
/// ship that has not scanned since last tick costs nothing.
pub fn publish_scan_blackboard(
    mut ships: Query<(
        &ShipScanRecord,
        &mut crate::server_app::ShipSystemBlackboards,
    )>,
) {
    for (record, mut blackboards) in ships.iter_mut() {
        let blackboard = SystemBlackboard::Scan(ScanBlackboard {
            // A hull with no bands can be asked and refused, which the console
            // renders as "no scan capability" rather than as an empty box.
            capable: !record.config.bands.is_empty(),
            reading: record.last.as_ref().map(ScanReadingSnapshot::from_reading),
            refusal: record.refusal.map(|r| r.string_id().to_string()),
        });
        let key = scan_blackboard_key();
        if blackboards.0.get(&key) != Some(&blackboard) {
            blackboards.0.insert(key, blackboard);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::science::scan::ScanBandConfig;

    const SHIP: &str = "ship-1";
    /// The depot's minted UUID — what a command and a reading join on.
    const DEPOT: &str = "depot-1";
    /// The depot's authored `[[entity]] id` — deliberately NOT its UUID, so the
    /// mirror-flag tests below fail if the key ever slips back onto the UUID no
    /// scenario author can type.
    const DEPOT_ID: &str = "skyway_depot";
    /// The depot's authored mass (issue #1154) — a distinctive number so a test
    /// asserting on it cannot pass by accident against
    /// `entity_config::DEFAULT_ENTITY_MASS`.
    const DEPOT_MASS: f32 = 180_000.0;

    fn suite() -> ScanConfig {
        ScanConfig {
            power_group: "shields".into(),
            min_power_level: 1,
            bands: vec![
                ScanBandConfig {
                    id: "detailed".into(),
                    label: "world.probe.band.detailed.label".into(),
                    max_range: 500.0,
                    condition_step: 0.01,
                    report_thresholds: true,
                    report_capacities: true,
                },
                ScanBandConfig {
                    id: "coarse".into(),
                    label: "world.probe.band.coarse.label".into(),
                    max_range: 3000.0,
                    condition_step: 0.25,
                    report_thresholds: true,
                    report_capacities: false,
                },
            ],
            degraded_by: Vec::new(),
            interference_bands: 1,
        }
    }

    fn depot_condition(condition: f32) -> InfrastructureCondition {
        use crate::infrastructure::{
            CapacityConfig, InfrastructureConfig, InfrastructureState, ThresholdConfig,
        };
        InfrastructureCondition(InfrastructureState::from_config(&InfrastructureConfig {
            condition_max: 100.0,
            condition: Some(condition),
            capacities: vec![CapacityConfig {
                id: "depot_berths".into(),
                amount: 4,
                label: Some("world.probe.capacity.berths.label".into()),
                ceiling: None,
            }],
            thresholds: vec![ThresholdConfig {
                flag: "depot_transfer_capable".into(),
                capacity: None,
                fails_below: 0.4,
                restores_above: None,
                label: Some("world.probe.threshold.transfer.label".into()),
            }],
            ..InfrastructureConfig::default()
        }))
    }

    /// A minimal app: the two systems, one scanning ship at the origin and one
    /// depot 200 units away.
    fn app_with(config: ScanConfig, condition: f32, depot_x: f32) -> (App, Entity) {
        let mut app = App::new();
        app.add_systems(Update, (tick_scans, publish_scan_blackboard).chain());
        // The world's flag store, so the mirror (issue #1038) has somewhere to
        // land. Every test in this module reads it or ignores it; the one below
        // that builds its own bare `App` deliberately leaves it out, which is
        // the `None` arm every fixture in the crate takes.
        app.insert_resource(WorldContentRuntime::default());
        let ship = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(SHIP.to_string()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                AdmittedCommands::default(),
                ShipScanRecord {
                    config,
                    ..Default::default()
                },
                crate::server_app::ShipSystemBlackboards::default(),
            ))
            .id();
        app.world_mut().spawn((
            EntityUuid(DEPOT.to_string()),
            crate::entities::spawner::EntityId(DEPOT_ID.to_string()),
            Transform::from_xyz(depot_x, 0.0, 0.0),
            EntityName("world.probe.entity.depot.name".to_string()),
            depot_condition(condition),
            crate::entities::spawner::EntityMass(DEPOT_MASS),
        ));
        (app, ship)
    }

    fn ask_for_scan(app: &mut App, ship: Entity, uuid: &str) {
        app.world_mut()
            .get_mut::<AdmittedCommands>(ship)
            .expect("the ship has an admitted set")
            .0
            .push(crate::core::messages::AdmittedCommand {
                target: SystemId(crate::ship::system_registry::SENSORS_SYSTEM_ID.to_string()),
                payload: SystemControlPayload::ScanTarget {
                    uuid: uuid.to_string(),
                },
                response_token: None,
            });
    }

    fn record(app: &App, ship: Entity) -> ShipScanRecord {
        app.world()
            .get::<ShipScanRecord>(ship)
            .expect("the ship keeps a scan record")
            .clone()
    }

    /// **AC1.** The command arrives through `AdmittedCommands` on the `sensors`
    /// system and produces a reading off the target's real condition.
    #[test]
    fn an_admitted_scan_command_reads_the_targets_live_condition_track() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        ask_for_scan(&mut app, ship, DEPOT);
        app.update();

        let reading = record(&app, ship).last.expect("a reading came back");
        assert_eq!(reading.subject_uuid, DEPOT);
        assert_eq!(reading.subject_name, "world.probe.entity.depot.name");
        assert_eq!(reading.band, "detailed");
        assert!(
            (reading.condition_fraction - 0.62).abs() < 1e-6,
            "62 of 100 points, read at whole-percent fidelity: {}",
            reading.condition_fraction
        );
        assert_eq!(
            reading.flags,
            vec![("world.probe.threshold.transfer.label".to_string(), true)]
        );
        assert_eq!(
            reading.capacities,
            vec![("world.probe.capacity.berths.label".to_string(), 4)]
        );
    }

    /// Issue #1154: the reading carries the subject's authored mass, off its
    /// `EntityMass` component — end to end, from the spawned entity through
    /// `AdmittedCommands` to the stored `ShipScanRecord`.
    #[test]
    fn a_scan_reading_reports_the_subjects_authored_mass() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        ask_for_scan(&mut app, ship, DEPOT);
        app.update();

        let reading = record(&app, ship).last.expect("a reading came back");
        assert_eq!(reading.mass, DEPOT_MASS);
    }

    /// A subject spawned without an `EntityMass` component (every real hull has
    /// one — this only happens to a hand-built test fixture) still reads out a
    /// real, positive mass rather than zero: [`subject_mass`] falls back to the
    /// same documented default an unauthored TOML gets.
    #[test]
    fn a_subject_with_no_entity_mass_component_falls_back_to_the_documented_default() {
        let mut app = App::new();
        app.add_systems(Update, (tick_scans, publish_scan_blackboard).chain());
        app.insert_resource(WorldContentRuntime::default());
        let ship = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(SHIP.to_string()),
                Transform::from_xyz(0.0, 0.0, 0.0),
                AdmittedCommands::default(),
                ShipScanRecord {
                    config: suite(),
                    ..Default::default()
                },
                crate::server_app::ShipSystemBlackboards::default(),
            ))
            .id();
        // No EntityMass component — the case the fallback in `subject_mass`
        // exists for.
        app.world_mut().spawn((
            EntityUuid(DEPOT.to_string()),
            Transform::from_xyz(200.0, 0.0, 0.0),
            EntityName("world.probe.entity.depot.name".to_string()),
            depot_condition(62.0),
        ));
        ask_for_scan(&mut app, ship, DEPOT);
        app.update();

        let reading = record(&app, ship).last.expect("a reading came back");
        assert_eq!(reading.mass, crate::entities::config::DEFAULT_ENTITY_MASS);
        assert!(reading.mass > 0.0, "the fallback must never be zero");
    }

    /// **AC2 through the ECS.** The SAME command, on a depot whose condition
    /// something moved in between, reads out differently — with no content edit
    /// and no change to the scanning hull.
    #[test]
    fn mutating_the_subjects_condition_changes_what_the_next_scan_says() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        ask_for_scan(&mut app, ship, DEPOT);
        app.update();
        let before = record(&app, ship).last.expect("a reading");

        // The storm takes thirty points off it. Nothing else in the world moves.
        {
            let mut q = app.world_mut().query::<&mut InfrastructureCondition>();
            let mut condition = q
                .iter_mut(app.world_mut())
                .next()
                .expect("the depot carries a track");
            condition.0.degrade(30.0);
        }
        ask_for_scan(&mut app, ship, DEPOT);
        app.update();
        let after = record(&app, ship).last.expect("a second reading");

        assert!(
            (before.condition_fraction - 0.62).abs() < 1e-6
                && (after.condition_fraction - 0.32).abs() < 1e-6,
            "the two readings are {} and {} — the readout is the track",
            before.condition_fraction,
            after.condition_fraction
        );
        assert_eq!(
            after.flags,
            vec![("world.probe.threshold.transfer.label".to_string(), false)],
            "…and the operational flag the drop knocked out reads out as down"
        );
    }

    /// **AC5 through the ECS.** Fly out and the same command comes back
    /// coarser, off the hull's own authored bands.
    #[test]
    fn the_same_scan_from_further_out_comes_back_at_a_coarser_band() {
        let (mut app, ship) = app_with(suite(), 62.0, 2_400.0);
        ask_for_scan(&mut app, ship, DEPOT);
        app.update();

        let reading = record(&app, ship).last.expect("a reading");
        assert_eq!(reading.band, "coarse");
        assert_eq!(
            reading.condition_fraction, 0.5,
            "0.62 to the nearest quarter"
        );
        assert!(
            reading.capacities.is_empty(),
            "the coarse band does not claim to count berths"
        );
    }

    /// **AC1's refusal half, through the ECS.** A target with no condition
    /// track at all is refused, and the refusal — not a stale reading — is what
    /// the console gets.
    #[test]
    fn scanning_something_with_no_condition_track_is_refused_with_a_reason() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        let rock = app
            .world_mut()
            .spawn((
                EntityUuid("rock-9".to_string()),
                Transform::from_xyz(50.0, 0.0, 0.0),
            ))
            .id();
        assert!(app.world().get::<InfrastructureCondition>(rock).is_none());

        ask_for_scan(&mut app, ship, DEPOT);
        app.update();
        assert!(record(&app, ship).last.is_some(), "precondition: a reading");

        ask_for_scan(&mut app, ship, "rock-9");
        app.update();
        let record = record(&app, ship);
        assert_eq!(record.refusal, Some(ScanRefusal::NoReadableCondition));
        assert!(
            record.last.is_none(),
            "a refusal clears the previous reading rather than leaving one on \
             screen beside a complaint about a different target"
        );
    }

    /// A uuid nothing answers to is its own refusal, distinct from a target
    /// that exists and has nothing to read.
    #[test]
    fn scanning_a_uuid_no_entity_answers_to_is_refused_as_no_such_target() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        ask_for_scan(&mut app, ship, "not-in-this-world");
        app.update();
        assert_eq!(record(&app, ship).refusal, Some(ScanRefusal::NoSuchTarget));
    }

    /// **The leak rule through the ECS.** A `publish = false` structure is
    /// refused with the identical reason a bare rock gets, and nothing about
    /// its real 31 points reaches the record or the wire.
    #[test]
    fn a_structure_the_scenario_keeps_off_the_wire_cannot_be_scanned() {
        use crate::infrastructure::{InfrastructureConfig, InfrastructureState};

        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        app.world_mut().spawn((
            EntityUuid("sealed-1".to_string()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            EntityName("world.probe.entity.sealed.name".to_string()),
            InfrastructureCondition(InfrastructureState::from_config(&InfrastructureConfig {
                condition_max: 100.0,
                condition: Some(31.0),
                publish: false,
                ..InfrastructureConfig::default()
            })),
        ));

        ask_for_scan(&mut app, ship, "sealed-1");
        app.update();
        let record = record(&app, ship);
        assert_eq!(
            record.refusal,
            Some(ScanRefusal::NoReadableCondition),
            "the same answer an unreadable rock gets"
        );
        assert!(record.last.is_none(), "and the withheld 0.31 is on nothing");
    }

    /// A hull with no `[scan]` table still answers when it is asked.
    #[test]
    fn a_hull_that_cannot_scan_is_refused_by_name_rather_than_ignored() {
        let mut app = App::new();
        app.add_systems(Update, tick_scans);
        let ship = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                EntityUuid(SHIP.to_string()),
                Transform::default(),
                AdmittedCommands::default(),
            ))
            .id();
        ask_for_scan(&mut app, ship, DEPOT);
        app.update();
        assert_eq!(
            app.world()
                .get::<ShipScanRecord>(ship)
                .expect("the refusal record is inserted")
                .refusal,
            Some(ScanRefusal::NotCapable)
        );
    }

    /// The reading reaches the wire under the `scan` channel, with the string
    /// ids the console resolves and no English on any of them.
    #[test]
    fn the_reading_is_published_under_the_scan_blackboard_channel() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        ask_for_scan(&mut app, ship, DEPOT);
        app.update();

        let boards = app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(ship)
            .expect("the ship publishes");
        let bb = match boards.0.get(&scan_blackboard_key()) {
            Some(SystemBlackboard::Scan(bb)) => bb.clone(),
            other => panic!("expected a scan blackboard, got {other:?}"),
        };
        assert!(bb.capable);
        assert!(bb.refusal.is_none());
        let reading = bb.reading.expect("the reading is on the wire");
        assert_eq!(reading.band_label, "world.probe.band.detailed.label");
        assert_eq!(reading.subject_name, "world.probe.entity.depot.name");
        assert_eq!(reading.condition_step, 0.01);
    }

    /// A refusal reaches the wire as its own `strings.csv` id.
    #[test]
    fn a_refusal_is_published_as_a_string_id_rather_than_as_prose() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        ask_for_scan(&mut app, ship, "not-in-this-world");
        app.update();

        let boards = app
            .world()
            .get::<crate::server_app::ShipSystemBlackboards>(ship)
            .expect("the ship publishes");
        let bb = match boards.0.get(&scan_blackboard_key()) {
            Some(SystemBlackboard::Scan(bb)) => bb.clone(),
            other => panic!("expected a scan blackboard, got {other:?}"),
        };
        assert_eq!(bb.refusal.as_deref(), Some("scan.refusal.no_such_target"));
        assert!(bb.reading.is_none());
    }

    // ── The mirror flag (issue #1038) ───────────────────────────────────────

    fn runtime(app: &App) -> &WorldContentRuntime {
        app.world().resource::<WorldContentRuntime>()
    }

    /// Every `FlagSet` this run has queued, in order.
    fn queued_flag_sets(app: &App) -> Vec<String> {
        runtime(app)
            .pending_world_events
            .iter()
            .filter_map(|e| match e {
                WorldEvent::FlagSet { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    /// **Issue #1038's engine seam.** A reading that comes back raises the
    /// subject's own `scan.<id>.taken` in the world flag store and queues the
    /// `FlagSet` a scenario's `on_flag_set` hook fires from — keyed on the
    /// world's authored id, which is the only spelling an author can write.
    #[test]
    fn a_reading_that_comes_back_raises_the_subjects_scanned_flag() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        assert_eq!(runtime(&app).flags.counter(&scanned_flag(DEPOT_ID)), 0);

        ask_for_scan(&mut app, ship, DEPOT);
        app.update();

        assert_eq!(
            runtime(&app).flags.counter(&scanned_flag(DEPOT_ID)),
            1,
            "the crew have now read this structure, and a script can ask so"
        );
        assert_eq!(queued_flag_sets(&app), vec![scanned_flag(DEPOT_ID)]);
        assert_eq!(
            runtime(&app).flags.counter(&scanned_flag(DEPOT)),
            0,
            "and nothing is keyed on the minted UUID, which no scenario can type"
        );
    }

    /// A structure the world authored no `id` for is one no scenario can name,
    /// so it is read and nothing is mirrored. Never a `scan..taken`.
    #[test]
    fn a_structure_with_no_authored_id_is_scannable_and_mirrors_nothing() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        app.world_mut().spawn((
            EntityUuid("anonymous-1".to_string()),
            Transform::from_xyz(150.0, 0.0, 0.0),
            EntityName("world.probe.entity.anonymous.name".to_string()),
            depot_condition(44.0),
        ));

        ask_for_scan(&mut app, ship, "anonymous-1");
        app.update();

        assert!(
            record(&app, ship).last.is_some(),
            "the reading still comes back — the console is owed an answer"
        );
        assert!(queued_flag_sets(&app).is_empty());
        assert_eq!(runtime(&app).flags.counter("scan..taken"), 0);
    }

    /// It LATCHES. A second reading of the same structure does not queue a
    /// second event — an ordinary re-scan must not re-fire a beat — and reading
    /// something else afterwards does not unlearn the first.
    #[test]
    fn re_scanning_raises_nothing_twice_and_scanning_elsewhere_unlearns_nothing() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        app.world_mut().spawn((
            EntityUuid("depot-2".to_string()),
            crate::entities::spawner::EntityId("ladder_depot".to_string()),
            Transform::from_xyz(120.0, 0.0, 0.0),
            EntityName("world.probe.entity.other.name".to_string()),
            depot_condition(55.0),
        ));

        ask_for_scan(&mut app, ship, DEPOT);
        app.update();
        ask_for_scan(&mut app, ship, DEPOT);
        app.update();
        ask_for_scan(&mut app, ship, "depot-2");
        app.update();

        assert_eq!(
            queued_flag_sets(&app),
            vec![scanned_flag(DEPOT_ID), scanned_flag("ladder_depot")],
            "one event per structure the crew have read, however often they read it"
        );
        assert_eq!(
            runtime(&app).flags.counter(&scanned_flag(DEPOT_ID)),
            1,
            "the console's `last` reading has moved on to the other depot; what \
             the crew KNOW they have looked at has not"
        );
    }

    /// A refusal raises nothing. Being told there is nothing to read is not
    /// having read it, and a scenario that fired its comparison off a refused
    /// scan would be firing off "the player pressed the button".
    #[test]
    fn a_refused_scan_raises_no_flag_for_the_thing_it_could_not_read() {
        use crate::infrastructure::{InfrastructureConfig, InfrastructureState};

        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        app.world_mut().spawn((
            EntityUuid("sealed-1".to_string()),
            crate::entities::spawner::EntityId("sealed_depot".to_string()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            EntityName("world.probe.entity.sealed.name".to_string()),
            InfrastructureCondition(InfrastructureState::from_config(&InfrastructureConfig {
                condition_max: 100.0,
                condition: Some(31.0),
                publish: false,
                ..InfrastructureConfig::default()
            })),
        ));

        ask_for_scan(&mut app, ship, "sealed-1");
        app.update();
        ask_for_scan(&mut app, ship, "not-in-this-world");
        app.update();

        assert_eq!(
            runtime(&app).flags.counter(&scanned_flag("sealed_depot")),
            0
        );
        assert!(
            queued_flag_sets(&app).is_empty(),
            "neither refusal is an act of reading"
        );
    }

    /// The save projection carries the reading and the refusal and leaves the
    /// authored table behind.
    #[test]
    fn the_save_projection_carries_the_reading_and_not_the_authored_table() {
        let (mut app, ship) = app_with(suite(), 62.0, 200.0);
        ask_for_scan(&mut app, ship, DEPOT);
        app.update();

        let live = record(&app, ship);
        let saved = live.save_state();
        assert_eq!(saved.last, live.last);
        assert!(saved.refusal.is_none());

        let mut restored = ShipScanRecord {
            config: suite(),
            ..Default::default()
        };
        restored.restore(&saved);
        assert_eq!(restored, live, "the whole record comes back");
    }
}
