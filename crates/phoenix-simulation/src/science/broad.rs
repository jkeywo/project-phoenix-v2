//! Physical broad instrument: paid returns become observer-local records.
//! Controllers and publishers consume BroadTracks, never these scene inputs.
use crate::entities::spawner::{EntityName, EntityTagsSection, EntityUuid, FactionComponent};
use crate::ship::{
    power::ShipPowerSystem,
    state::{ShipPhysics, ShipRedAlert},
};
use crate::ship_plugin::ShipSystemControlSources;
use bevy::prelude::*;
use phoenix_model::sensing::BroadConfig;
pub use phoenix_sim_gameplay::science::tracks::*;

#[derive(Component, Clone, Debug)]
pub struct BroadProfile(pub BroadConfig);
#[derive(Component, Clone, Debug)]
pub struct BroadTracks(pub BroadTrackState);

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(profile) = config
        .sensing
        .as_ref()
        .and_then(|sensing| sensing.broad.as_ref())
    {
        commands.insert((
            BroadProfile(profile.clone()),
            BroadTracks(BroadTrackState::new(profile)),
        ));
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
        Option<&crate::gm::operations::information::reports::SensorsObservationConfig>,
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
        radar,
        modifiers,
    ) in observers
    {
        let profile = &profile.0;
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
        if let Some(radar) = radar.filter(|_| paid > 0.0) {
            let range = radar.0.range
                * modifiers.map_or(1.0, |modifiers| {
                    modifiers.get(&crate::core::messages::ModifierSlot::SensorRadarRange)
                });
            if range.is_finite() && range > 0.0 {
                for (id, transform, physics, name, faction, tags, ship, power, alert, disabled) in
                    &scene
                {
                    if id.0 == observer.0
                        || !tags.is_some_and(|tags| {
                            tags.0.iter().any(|tag| {
                                radar
                                    .0
                                    .shows
                                    .iter()
                                    .any(|allowed| allowed.as_str() == tag.as_str())
                            })
                        })
                    {
                        continue;
                    }
                    let concealed = runtime.as_ref().is_some_and(|runtime| {
                        crate::gm::operations::contact::mode(
                            &runtime.contact_overrides,
                            &observer.0,
                            &id.0,
                        ) == crate::gm::operations::contact::ContactMode::Conceal
                            || crate::gm::operations::information::reports::contains(
                                &runtime.contact_information.reports,
                                &observer.0,
                                &id.0,
                            )
                    });
                    if concealed {
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
                                "station", "asteroid", "planet", "moon", "star", "cargo", "region",
                            ]
                            .into_iter()
                            .find(|known| tags.0.iter().any(|tag| tag.as_str() == *known))
                        })
                        .unwrap_or("contact")
                    };
                    measurements.push(BroadMeasurement {
                        source: id.0.clone(),
                        position: [position.x, position.z],
                        support,
                        identity: name.filter(|name| !name.0.is_empty()).map(|name| {
                            TrackIdentity {
                                name: name.0.clone(),
                                faction: faction.map(|faction| faction.0.to_string()),
                            }
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
            .0
            .advance(profile, tick.0, dt, paid.clamp(0.0, 1.0), &measurements)
            .expect("compiled broad profile and fixed physical observation must validate");
    }
}
fn physical_position(physics: Option<&ShipPhysics>, transform: &Transform) -> Vec3 {
    physics.map_or(transform.translation, |physics| {
        Vec3::new(physics.x, physics.y, physics.z)
    })
}
#[cfg(test)]
#[path = "broad_tests.rs"]
mod tests;
