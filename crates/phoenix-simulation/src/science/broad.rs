//! Physical broad instrument: paid returns become observer-local records.
//! Controllers and publishers consume BroadTracks, never these scene inputs.
use crate::entities::spawner::{EntityName, EntityTagsSection, EntityUuid, FactionComponent};
use crate::ship::{
    power::ShipPowerSystem,
    state::{ShipPhysics, ShipRedAlert},
};
use crate::ship_plugin::ShipSystemControlSources;
use bevy::prelude::*;
use phoenix_model::{identity::SystemId, sensing::BroadConfig};
pub use phoenix_sim_gameplay::science::tracks::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct BroadInstrumentProfile {
    pub profile: BroadConfig,
    pub radar: crate::radar_config::RadarConfig,
    pub kind: String,
}
#[derive(Component, Clone, Debug)]
pub struct BroadProfile(pub BTreeMap<SystemId, BroadInstrumentProfile>);
#[derive(Component, Clone, Debug)]
pub struct BroadTracks(pub BTreeMap<SystemId, BroadTrackState>);
#[derive(Component, Clone, Debug)]
pub struct SensingControls(pub phoenix_model::sensing::SensingControlBindings);

pub fn combat_owner(controls: Option<&SensingControls>, fitted: bool) -> Option<SystemId> {
    controls.map_or_else(
        || (!fitted).then(crate::ship::system_registry::tactical_radar_system_id),
        |controls| controls.0.combat_lock.clone(),
    )
}
pub fn science_owner(controls: Option<&SensingControls>, fitted: bool) -> Option<SystemId> {
    controls.map_or_else(
        || (!fitted).then(crate::ship::system_registry::sensors_system_id),
        |controls| controls.0.science_target.clone(),
    )
}
impl BroadProfile {
    pub fn from_config(
        config: &crate::entities::config::EntityConfig,
    ) -> Result<Option<Self>, String> {
        let Some(sensing) = &config.sensing else {
            return Ok(None);
        };
        if sensing.broad.is_empty() {
            return Ok(None);
        }
        phoenix_sim_gameplay::science::sensing::validate_sensing(sensing, config)?;
        let profiles = sensing
            .broad
            .iter()
            .map(|profile| {
                Ok((
                    profile.system.clone(),
                    BroadInstrumentProfile {
                        kind: config
                            .ship_config
                            .as_ref()
                            .and_then(|ship| ship.system(&profile.system))
                            .expect("validated actual instrument")
                            .kind
                            .clone(),
                        profile: profile.clone(),
                        radar: phoenix_sim_gameplay::science::sensing::broad_radar(
                            &profile.system,
                            config,
                        )?,
                    },
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(Some(Self(profiles)))
    }
}
impl BroadTracks {
    pub fn new(profile: &BroadProfile) -> Self {
        Self(
            profile
                .0
                .iter()
                .map(|(id, instrument)| (id.clone(), BroadTrackState::new(&instrument.profile)))
                .collect(),
        )
    }
}
pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(profile) = &config.sensing {
        commands.insert(SensingControls(profile.control.clone()));
    }
    if let Some(profile) =
        BroadProfile::from_config(config).expect("accepted broad instrument composition")
    {
        let mut state = BroadTracks::new(&profile);
        for sharing in &config
            .sensing
            .as_ref()
            .expect("broad sensing profile")
            .sharing
        {
            let owner = state
                .0
                .get_mut(&sharing.system)
                .expect("validated sharing owner");
            owner.share_own_faction = sharing.share_own_faction;
            owner.receive_shared = sharing.receive;
        }
        commands.insert((profile, state));
    }
}
pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<BroadProfile>(StateClass::Derived, "independent-broad-scanner")
        .declare_state::<SensingControls>(StateClass::Derived, "sensing-control-ownership")
        .declare_state::<BroadTracks>(StateClass::Folded, "independent-broad-scanner");
    app.add_systems(
        FixedUpdate,
        tick.in_set(crate::sim_sets::SimSet::Modifiers)
            .after(crate::ship::energy_runtime::tick_power_system),
    );
}

#[allow(clippy::type_complexity)]
pub(crate) fn tick(
    fixed: Res<Time<Fixed>>,
    tick: Res<crate::sim_tick::SimTick>,
    paths: crate::regions::signal_path::RegionSignalQuery,
    runtime: Option<Res<crate::world::server::WorldContentRuntime>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    world: Option<Res<crate::lobby::server::WorldResource>>,
    ordnance: Query<&crate::server_app::TorpedoSystemResource>,
    scene: Query<(
        Option<&EntityUuid>,
        Option<&crate::server_app::AsteroidUuid>,
        &Transform,
        Option<&ShipPhysics>,
        Option<&EntityName>,
        Option<&FactionComponent>,
        Option<&EntityTagsSection>,
        Has<crate::server_app::Ship>,
        Option<&ShipPowerSystem>,
        Option<&ShipRedAlert>,
        Option<&crate::ship::disablement::DisablementObservation>,
        Option<&crate::entities::spawner::EntityTarget>,
        Option<&crate::ship::shields::ShipShields>,
    )>,
    mut observers: Query<(
        &EntityUuid,
        &Transform,
        Option<&ShipPhysics>,
        &BroadProfile,
        &mut BroadTracks,
        &ShipPowerSystem,
        &ShipSystemControlSources,
        Option<&crate::ship::compartment_damage::ShipDamage>,
        &crate::ship_plugin::ShipConfigComponent,
        Option<&crate::modifiers::ShipModifiers>,
        &crate::server_app::ShipSystemBlackboards,
    )>,
) {
    let dt = fixed.timestep().as_secs_f32();
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    // Streamed rocks retain their existing actor identity and authored tags in
    // WorldResource. Positions still come from the actual physical actor.
    let snapshots: BTreeMap<_, _> = world
        .as_ref()
        .into_iter()
        .flat_map(|world| &world.0.entities)
        .map(|row| (row.uuid.as_str(), row))
        .collect();
    let mut observers: Vec<_> = observers.iter_mut().collect();
    observers.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    for (
        observer,
        transform,
        physics,
        profile,
        mut state,
        power,
        sources,
        damage,
        ship_config,
        modifiers,
        boards,
    ) in observers
    {
        for (system, instrument) in &profile.0 {
            let profile = &instrument.profile;
            let state = state
                .0
                .get_mut(system)
                .expect("each installed broad instrument owns its state");
            let installed = sources.0.entries().any(|(id, _)| id == &profile.system);
            let usable = installed
                && !sources.0.is_offline(&profile.system)
                && sources.0.source_for(&profile.system)
                    != crate::ship::control_source::ControlSource::Offline;
            let operation = damage.map_or(1.0, |damage| {
                damage.factor(
                    &profile.system,
                    phoenix_model::damage::DamageBinding::Operation,
                )
            });
            // Power already pays each fitted broad consumer once in Physics. The
            // observation producer never spends a second cost or uses incoming flow.
            let paid = if usable {
                power.0.last_work_fraction(&profile.system).unwrap_or(0.0) * operation
            } else {
                0.0
            };
            let origin = physical_position(physics, transform);
            let mut measurements = Vec::new();
            if paid > 0.0 {
                let radar = &instrument.radar;
                let range = radar.range
                    * modifiers.map_or(1.0, |modifiers| {
                        modifiers.get(
                            &match ship_config.0.system(system).map(|s| s.kind.as_str()) {
                                Some(crate::ship::system_registry::HELM_RADAR_KIND) => {
                                    crate::core::messages::ModifierSlot::HelmRadarRange
                                }
                                Some(crate::ship::system_registry::TACTICAL_RADAR_KIND) => {
                                    crate::core::messages::ModifierSlot::RadarRange
                                }
                                _ => crate::core::messages::ModifierSlot::SensorRadarRange,
                            },
                        )
                    });
                if range.is_finite() && range > 0.0 {
                    // Existing projectiles are not scene entities. Their one
                    // authoritative owner supplies positions to the same paid
                    // instrument, with no guidance/order/velocity shortcut.
                    if radar
                        .shows
                        .contains(&phoenix_model::tags::EntityTag::Missile)
                    {
                        for projectile in ordnance.iter().flat_map(|owner| &owner.0.in_flight) {
                            let position = Vec3::new(projectile.x, projectile.y, projectile.z);
                            if !position.is_finite()
                                || position.distance(origin) > range
                                || runtime.as_ref().is_some_and(|runtime| {
                                    crate::gm::operations::contact::mode(
                                        &runtime.contact_overrides,
                                        &observer.0,
                                        &projectile.uuid,
                                    ) == crate::gm::operations::contact::ContactMode::Conceal
                                })
                            {
                                continue;
                            }
                            let support = paths.transmission(
                                origin,
                                position,
                                phoenix_sim_gameplay::regions::signal_path::SignalClass::Sensors,
                            );
                            if support > 0.0 {
                                measurements.push(BroadMeasurement {
                                    elevation: Some(
                                        phoenix_model::sensing_tracks::ObservedElevation {
                                            height: position.y,
                                            error: profile.position_error,
                                            velocity: None,
                                        },
                                    ),
                                    source: projectile.uuid.clone(),
                                    observation: None,
                                    selectable: radar
                                        .selects
                                        .contains(&phoenix_model::tags::EntityTag::Missile),
                                    position: [position.x, position.z],
                                    support,
                                    identity: None,
                                    classification: Some("missile".into()),
                                    assessment: None,
                                });
                            }
                        }
                    }
                    for (
                        id,
                        rock,
                        transform,
                        physics,
                        name,
                        faction,
                        tags,
                        ship,
                        power,
                        alert,
                        disabled,
                        target,
                        shields,
                    ) in &scene
                    {
                        let Some(id) = id
                            .map(|id| id.0.as_str())
                            .or_else(|| rock.map(|id| id.0.as_str()))
                        else {
                            continue;
                        };
                        let tags = tags.map(|tags| tags.0.as_slice()).or_else(|| {
                            rock.and_then(|_| snapshots.get(id).map(|row| row.tags.as_slice()))
                        });
                        if id == observer.0 {
                            continue;
                        }
                        let report = runtime.as_ref().and_then(|runtime| {
                            runtime
                                .contact_information
                                .reports
                                .get(&observer.0)?
                                .get(id)
                        });
                        let mode = runtime.as_ref().map_or(Default::default(), |runtime| {
                            crate::gm::operations::contact::mode(
                                &runtime.contact_overrides,
                                &observer.0,
                                id,
                            )
                        });
                        if mode == crate::gm::operations::contact::ContactMode::Conceal {
                            continue;
                        }
                        // GM reports are an observer's Sensors picture. Another
                        // independently fitted instrument does not inherit it.
                        if ship_config.0.system(system).is_some_and(|system| {
                            system.kind == crate::ship::system_registry::SENSOR_RADAR_KIND
                        }) {
                            if let Some(report) = report {
                                if let Some(measurement) =
                                    reported_measurement(system, id, report, target, &radar.selects)
                                {
                                    measurements.push(measurement);
                                }
                                continue;
                            }
                        }
                        if !tags.is_some_and(|tags| {
                            tags.iter().any(|tag| {
                                radar
                                    .shows
                                    .iter()
                                    .any(|allowed| allowed.as_str() == tag.as_str())
                            })
                        }) {
                            continue;
                        }
                        let position = physical_position(physics, transform);
                        if !position.is_finite() || position.distance(origin) > range {
                            continue;
                        }
                        let support = paths.transmission(
                            origin,
                            position,
                            phoenix_sim_gameplay::regions::signal_path::SignalClass::Sensors,
                        );
                        if support <= 0.0 {
                            continue;
                        }
                        let classification = if ship {
                            "ship"
                        } else {
                            tags.and_then(|tags| {
                                [
                                    "station", "asteroid", "planet", "moon", "star", "cargo",
                                    "region",
                                ]
                                .into_iter()
                                .find(|known| tags.iter().any(|tag| tag.as_str() == *known))
                            })
                            .unwrap_or("contact")
                        };
                        // Classification changes interpretation of an already
                        // detected return; it never supplies detection itself.
                        let supplied_identity = runtime.as_ref().and_then(|runtime| {
                            runtime.contact_classifications.get(&observer.0)?.get(id)
                        });
                        let identity_name = supplied_identity
                            .map(|identity| identity.label.as_str())
                            .or_else(|| name.map(|name| name.0.as_str()))
                            .filter(|name| !name.is_empty());
                        measurements.push(BroadMeasurement {
                            elevation: Some(phoenix_model::sensing_tracks::ObservedElevation {
                                height: position.y,
                                error: profile.position_error,
                                velocity: None,
                            }),
                            source: id.to_owned(),
                            observation: None,
                            selectable: target.is_some_and(|target| {
                                crate::entities::tags::matches_any(
                                    &crate::entities::tags::parse_tags(&target.0.tags),
                                    &radar.selects,
                                )
                            }) || rock.is_some()
                                && snapshots.get(id).is_some_and(|snapshot| {
                                    crate::entities::tags::matches_any(
                                        &crate::entities::tags::parse_tags(&snapshot.target_tags),
                                        &radar.selects,
                                    )
                                }),
                            position: [position.x, position.z],
                            support,
                            identity: identity_name.map(|name| TrackIdentity {
                                name: name.to_owned(),
                                faction: supplied_identity
                                    .is_none()
                                    .then(|| faction.map(|faction| faction.0.to_string()))
                                    .flatten(),
                                references: world_config.as_ref().map_or_else(Vec::new, |world| {
                                    let view =
                                        ship_config.0.observations_for_system(system, &boards.0);
                                    let Some(crate::core::messages::SystemBlackboard::Viewscreen(
                                        board,
                                    )) = view
                                        .get(&crate::ship::system_registry::viewscreen_system_id())
                                    else {
                                        return Vec::new();
                                    };
                                    routine_references(
                                        name,
                                        &world.entities,
                                        board.scored_objectives.iter().flat_map(|objective| {
                                            objective.snapshot.targets.iter().map(String::as_str)
                                        }),
                                    )
                                }),
                            }),
                            classification: Some(classification.into()),
                            assessment: Some(TrackAssessment {
                                protection: Some(phoenix_model::sensing_tracks::ObservedShieldCover {
                                    incoming_sector: phoenix_sim_gameplay::science::tracks::shield_observation_sector(
                                        [origin.x, origin.z], [position.x, position.z]),
                                    blocked: shields.is_some_and(|shields| shields.0.hp_facing_attacker(
                                        origin.x, origin.z, position.x, position.z,
                                        physics.map_or(0.0, |physics| physics.yaw)) > 0),
                                }),
                                alert: alert.map(|alert| alert.0),
                                disabled: disabled.map(|disabled| disabled.disabled),
                                weapons_restrained: power.and_then(|power| {
                                    power
                                        .0
                                        .has_group(&crate::core::messages::PowerGroupId(
                                            "weapons".into(),
                                        ))
                                        .then(|| power.0.weapons_restrained())
                                }),
                            }),
                        });
                    }
                }
            }
            if paid > 0.0
                && ship_config.0.system(system).is_some_and(|system| {
                    system.kind == crate::ship::system_registry::SENSOR_RADAR_KIND
                })
            {
                if let Some(runtime) = runtime.as_ref() {
                    for ghost in runtime
                        .contact_information
                        .ghosts
                        .get(&observer.0)
                        .into_iter()
                        .flat_map(|rows| rows.values())
                    {
                        let presentation =
                            crate::gm::operations::information::ghost_uuid(&observer.0, &ghost.id);
                        if crate::gm::operations::contact::mode(
                            &runtime.contact_overrides,
                            &observer.0,
                            &presentation,
                        ) == crate::gm::operations::contact::ContactMode::Conceal
                        {
                            continue;
                        }
                        if let Some(sample) = ghost_measurement(system, ghost, state, tick.0) {
                            measurements.push(sample);
                        }
                    }
                }
            }
            // Equal observed labels are not proof that two returns are the
            // same authored actor. Keep both returns, withholding correlation.
            let mut labels = BTreeMap::<String, usize>::new();
            for measurement in &measurements {
                if let Some(identity) = &measurement.identity {
                    *labels.entry(identity.name.clone()).or_default() += 1;
                }
            }
            for measurement in &mut measurements {
                if let Some(identity) = &mut measurement.identity {
                    if labels.get(&identity.name).copied().unwrap_or(0) != 1 {
                        identity.references.clear();
                    }
                }
            }
            state
                .advance(profile, tick.0, dt, paid.clamp(0.0, 1.0), &measurements)
                .expect("compiled broad profile and fixed physical observation must validate");
            phoenix_sim_gameplay::science::sharing::record_observer_pose(
                state,
                tick.0,
                [origin.x, origin.z],
            );
        }
    }
}
fn ghost_measurement(
    owner: &SystemId,
    ghost: &crate::gm::operations::information::GhostContact,
    state: &BroadTrackState,
    tick: u64,
) -> Option<BroadMeasurement> {
    use phoenix_model::observation::*;
    let source = synthetic_source(&ghost.id).ok()?;
    let position = [
        ghost.position_mm[0] as f32 / 1000.0,
        ghost.position_mm[2] as f32 / 1000.0,
    ];
    let current = state
        .current
        .get(&source)
        .and_then(|id| state.tracks.get(id));
    let same_evidence = |track: &&BroadTrack| {
        track.position == position
            && track
                .identity
                .as_ref()
                .is_some_and(|identity| identity.name == ghost.label)
            && track
                .observation
                .as_ref()
                .is_some_and(|observation| observation.subject == ObservationSubject::Unknown)
    };
    let held = if current.is_some() {
        current.filter(same_evidence)
    } else {
        // Loss of current support clears routing, not the held sample's clock.
        // Reuse only an unambiguous retained report; source IDs never associate
        // a lost observed actor, and synthetic points have no actor to route.
        let mut retained = state.tracks.values().filter(same_evidence);
        retained.next().filter(|_| retained.next().is_none())
    };
    let observation = held
        .and_then(|track| track.observation.as_ref())
        .filter(|observation| observation.subject == ObservationSubject::Unknown)
        .cloned()
        .unwrap_or_else(|| {
            ObservationMetadata::at(
                tick,
                ObservationSubject::Unknown,
                ObservationProvenance {
                    kind: ObservationSource::SensorsReport,
                    system: Some(owner.clone()),
                    label: "console.sensors.report_source".into(),
                },
                ObservationUncertainty::Unspecified,
            )
        });
    Some(BroadMeasurement {
        elevation: None,
        source,
        observation: Some(observation),
        selectable: false,
        position,
        support: 1.0,
        identity: Some(TrackIdentity {
            name: ghost.label.clone(),
            faction: None,
            references: Vec::new(),
        }),
        classification: None,
        assessment: None,
    })
}
fn reported_measurement(
    owner: &SystemId,
    source: &str,
    state: &crate::gm::operations::information::reports::ReportState,
    target: Option<&crate::entities::spawner::EntityTarget>,
    selects: &[crate::entities::tags::EntityTag],
) -> Option<BroadMeasurement> {
    use phoenix_model::observation::*;
    let sample = state.presented.as_ref()?;
    Some(BroadMeasurement {
        elevation: None,
        source: source.to_owned(),
        observation: Some(ObservationMetadata::at(
            sample.observed_tick,
            ObservationSubject::Entity {
                uuid: source.into(),
            },
            ObservationProvenance {
                kind: ObservationSource::SensorsReport,
                system: Some(owner.clone()),
                label: "console.sensors.report_source".into(),
            },
            if state.policy.position_step_mm > 0 {
                ObservationUncertainty::Quantized {
                    field: "position".into(),
                    step: state.policy.position_step_mm as f32 / 1000.0,
                }
            } else {
                ObservationUncertainty::Unspecified
            },
        )),
        selectable: target.is_some_and(|target| {
            crate::entities::tags::matches_any(
                &crate::entities::tags::parse_tags(&target.0.tags),
                selects,
            )
        }),
        position: [
            sample.position_mm[0] as f32 / 1000.0,
            sample.position_mm[2] as f32 / 1000.0,
        ],
        support: 1.0,
        identity: (!state.policy.hide_identity && sample.name != "console.sensors.basic_contact")
            .then(|| TrackIdentity {
                name: sample.name.clone(),
                faction: None,
                references: Vec::new(),
            }),
        classification: None,
        assessment: None,
    })
}
fn routine_references<'a>(
    observed_name: &str,
    records: &[crate::world::config::WorldEntity],
    requested: impl Iterator<Item = &'a str>,
) -> Vec<String> {
    let mut matching = records
        .iter()
        .filter(|record| record.display_text() == observed_name);
    let Some(record) = matching.next().filter(|_| matching.next().is_none()) else {
        return Vec::new();
    };
    let mut references: Vec<_> = requested
        .filter(|reference| {
            record.name.as_deref() == Some(*reference) || record.id.as_deref() == Some(*reference)
        })
        .map(str::to_owned)
        .collect();
    references.sort();
    references.dedup();
    references
}
pub(crate) fn physical_position(physics: Option<&ShipPhysics>, transform: &Transform) -> Vec3 {
    physics.map_or(transform.translation, |physics| {
        Vec3::new(physics.x, physics.y, physics.z)
    })
}
#[cfg(test)]
#[path = "broad_tests.rs"]
mod tests;
