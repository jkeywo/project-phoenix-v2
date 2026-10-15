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
}
#[derive(Component, Clone, Debug)]
pub struct BroadProfile(pub BTreeMap<SystemId, BroadInstrumentProfile>);
#[derive(Component, Clone, Debug)]
pub struct BroadTracks(pub BTreeMap<SystemId, BroadTrackState>);
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
    if let Some(profile) =
        BroadProfile::from_config(config).expect("accepted broad instrument composition")
    {
        let state = BroadTracks::new(&profile);
        commands.insert((profile, state));
    }
}
pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<BroadProfile>(StateClass::Derived, "independent-broad-scanner")
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
    scene: Query<(
        &EntityUuid,
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
    )>,
) {
    let dt = fixed.timestep().as_secs_f32();
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
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
                    for (
                        id,
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
                    ) in &scene
                    {
                        if id.0 == observer.0 {
                            continue;
                        }
                        let report = runtime.as_ref().and_then(|runtime| {
                            runtime
                                .contact_information
                                .reports
                                .get(&observer.0)?
                                .get(&id.0)
                        });
                        let mode = runtime.as_ref().map_or(Default::default(), |runtime| {
                            crate::gm::operations::contact::mode(
                                &runtime.contact_overrides,
                                &observer.0,
                                &id.0,
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
                                if let Some(measurement) = reported_measurement(
                                    system,
                                    &id.0,
                                    report,
                                    target,
                                    &radar.selects,
                                ) {
                                    measurements.push(measurement);
                                }
                                continue;
                            }
                        }
                        if !tags.is_some_and(|tags| {
                            tags.0.iter().any(|tag| {
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
                                .find(|known| tags.0.iter().any(|tag| tag.as_str() == *known))
                            })
                            .unwrap_or("contact")
                        };
                        // Classification changes interpretation of an already
                        // detected return; it never supplies detection itself.
                        let supplied_identity = runtime.as_ref().and_then(|runtime| {
                            runtime.contact_classifications.get(&observer.0)?.get(&id.0)
                        });
                        let identity_name = supplied_identity
                            .map(|identity| identity.label.as_str())
                            .or_else(|| name.map(|name| name.0.as_str()))
                            .filter(|name| !name.is_empty());
                        measurements.push(BroadMeasurement {
                            source: id.0.clone(),
                            observation: None,
                            selectable: target.is_some_and(|target| {
                                crate::entities::tags::matches_any(
                                    &crate::entities::tags::parse_tags(&target.0.tags),
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
                            }),
                            classification: Some(classification.into()),
                            assessment: Some(TrackAssessment {
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
            state
                .advance(profile, tick.0, dt, paid.clamp(0.0, 1.0), &measurements)
                .expect("compiled broad profile and fixed physical observation must validate");
        }
    }
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
        source: source.to_owned(),
        observation: Some(ObservationMetadata::at(
            sample.observed_tick,
            ObservationSubject::Unknown,
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
            }),
        classification: None,
        assessment: None,
    })
}
fn physical_position(physics: Option<&ShipPhysics>, transform: &Transform) -> Vec3 {
    physics.map_or(transform.translation, |physics| {
        Vec3::new(physics.x, physics.y, physics.z)
    })
}
#[cfg(test)]
#[path = "broad_tests.rs"]
mod tests;
