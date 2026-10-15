//! Ephemeral Navigation inputs from its exact Console picture and public chart.
//! Private associations route legitimate requests; they never identify a name.
use super::sharing::NavigationSharingProfile;
use crate::{
    core::messages::SystemId,
    science::broad::{BroadTracks, SensingControls},
    server_app::ShipSystemBlackboards,
    ship_plugin::ShipConfigComponent,
};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use std::collections::BTreeMap;

#[derive(Component, Clone, Debug)]
pub struct NavigationChart(pub crate::radar_config::RadarConfig);

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    commands: &mut EntityCommands,
) {
    if let Some(nav) = &config.navigation_console {
        commands.insert(NavigationChart(nav.system_chart.clone()));
    }
}
pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<NavigationChart>(StateClass::Derived, "navigation-permitted-chart");
}

#[derive(SystemParam)]
pub struct Environment<'w, 's> {
    world: Option<Res<'w, crate::lobby::WorldResource>>,
    runtime: Option<Res<'w, crate::world::server::WorldContentRuntime>>,
    tick: Option<Res<'w, crate::sim_tick::SimTick>>,
    fixed: Option<Res<'w, Time<Fixed>>>,
    scopes: Query<
        'w,
        's,
        (
            Option<&'static BroadTracks>,
            Option<&'static SensingControls>,
            Option<&'static NavigationChart>,
            Option<&'static crate::entities::spawner::EntityUuid>,
            Option<&'static crate::ship::state::ShipPhysics>,
            Option<&'static Transform>,
            Option<&'static crate::science::broad::BroadProfile>,
        ),
    >,
}
impl Environment<'_, '_> {
    pub(crate) fn now(&self) -> Option<u64> {
        self.tick.as_ref().map(|tick| tick.0)
    }
    pub(crate) fn actor(&self, entity: Entity) -> Option<&str> {
        self.scopes.get(entity).ok()?.3.map(|uuid| uuid.0.as_str())
    }
    pub(crate) fn formation_readings(
        &self,
        entity: Entity,
        config: &ShipConfigComponent,
        boards: &ShipSystemBlackboards,
        owner: &SystemId,
    ) -> BTreeMap<String, super::formation::LeaderReading> {
        let mut readings = BTreeMap::new();
        let mut ambiguous = std::collections::BTreeSet::new();
        let (Some(now), Some(fixed)) = (self.now(), self.fixed.as_ref()) else {
            return readings;
        };
        let dt = fixed.timestep().as_secs_f64();
        if !dt.is_finite() || dt <= 0.0 {
            return readings;
        }
        let Ok((Some(tracks), _, _, Some(uuid), _, _, Some(profiles))) = self.scopes.get(entity)
        else {
            return readings;
        };
        let permitted = config.0.observations_for_system(owner, &boards.0);
        for (instrument, state) in &tracks.0 {
            let Some(profile) = profiles.0.get(instrument) else {
                continue;
            };
            let maximum_age = f64::from(profile.profile.sample_interval_secs) * 2.0 + dt;
            let Some(records) = permitted
                .get(instrument)
                .and_then(crate::science::contact_routing::observations)
            else {
                continue;
            };
            for record in records {
                if record.predicted
                    || !record.selectable
                    || record.observed_tick > now
                    || (now - record.observed_tick) as f64 * dt > maximum_age
                {
                    continue;
                }
                let Some(source) = state.resolve_current(&record.contact_id) else {
                    continue;
                };
                if self.runtime.as_ref().is_some_and(|runtime| {
                    crate::gm::operations::contact::mode(
                        &runtime.contact_overrides,
                        &uuid.0,
                        source,
                    ) == crate::gm::operations::contact::ContactMode::Conceal
                }) {
                    continue;
                }
                let Some(velocity) = record.velocity else {
                    continue;
                };
                let measurement = phoenix_model::formation::FormationMeasurement {
                    position: record.position,
                    velocity,
                    observed_tick: record.observed_tick,
                };
                if !measurement.valid_at(now) {
                    continue;
                }
                let reading = super::formation::LeaderReading {
                    name: record
                        .identity
                        .as_ref()
                        .map(|identity| identity.name.clone()),
                    measurement,
                };
                if readings
                    .get(&record.contact_id)
                    .is_some_and(|held| held != &reading)
                {
                    ambiguous.insert(record.contact_id.clone());
                }
                readings.insert(record.contact_id.clone(), reading);
            }
        }
        for alias in ambiguous {
            readings.remove(&alias);
        }
        readings
    }

    pub(crate) fn picture(
        &self,
        entity: Entity,
        config: Option<&ShipConfigComponent>,
        owner: &SystemId,
        boards: Option<&ShipSystemBlackboards>,
        sharing: Option<&NavigationSharingProfile>,
    ) -> Picture {
        let (tracks, controls, chart, uuid, physics, transform, _) =
            self.scopes.get(entity).unwrap_or_default();
        let pose = physics
            .map(|physics| Vec3::new(physics.x, physics.y, physics.z))
            .or_else(|| transform.map(|transform| transform.translation));
        Picture::capture(
            config,
            owner,
            boards,
            tracks,
            controls,
            uuid.map(|uuid| uuid.0.as_str()),
            self.runtime.as_deref(),
            chart,
            sharing,
            pose.and(self.world.as_deref()),
            pose.unwrap_or(Vec3::ZERO),
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Contact {
    /// Physical route stays in the host, never used to match an unknown name.
    pub route: String,
    pub key: String,
    pub name: Option<String>,
    pub references: Vec<String>,
    pub position: [f32; 3],
    pub charted: bool,
}
#[derive(Default)]
pub(crate) struct Picture {
    pub configured: bool,
    pub contacts: BTreeMap<String, Contact>,
}

impl Picture {
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        config: Option<&ShipConfigComponent>,
        owner: &SystemId,
        boards: Option<&ShipSystemBlackboards>,
        tracks: Option<&BroadTracks>,
        controls: Option<&SensingControls>,
        observer: Option<&str>,
        runtime: Option<&crate::world::server::WorldContentRuntime>,
        chart: Option<&NavigationChart>,
        sharing: Option<&NavigationSharingProfile>,
        world: Option<&crate::lobby::WorldResource>,
        pose: Vec3,
    ) -> Self {
        use crate::ship::system_registry::*;
        let mut picture = Self {
            configured: tracks.is_some()
                || controls.is_some()
                || config.is_some_and(|config| {
                    config.0.systems.iter().any(|system| {
                        matches!(
                            system.kind.as_str(),
                            HELM_RADAR_KIND
                                | TACTICAL_RADAR_KIND
                                | SENSOR_RADAR_KIND
                                | SENSORS_KIND
                                | PASSIVE_RECEIVER_KIND
                        )
                    })
                }),
            ..Self::default()
        };
        let concealed = |source: &str| {
            runtime.is_some_and(|runtime| {
                observer.is_none_or(|observer| {
                    crate::gm::operations::contact::mode(
                        &runtime.contact_overrides,
                        observer,
                        source,
                    ) == crate::gm::operations::contact::ContactMode::Conceal
                })
            })
        };
        if let (Some(config), Some(boards), Some(tracks)) = (config, boards, tracks) {
            let permitted = config.0.observations_for_system(owner, &boards.0);
            for (instrument, state) in &tracks.0 {
                let Some(records) = permitted
                    .get(instrument)
                    .and_then(crate::science::contact_routing::observations)
                else {
                    continue;
                };
                for record in records
                    .iter()
                    .filter(|record| record.selectable && !record.predicted)
                {
                    let Some(route) = state.resolve_current(&record.contact_id) else {
                        continue;
                    };
                    if concealed(route) || record.position.iter().any(|value| !value.is_finite()) {
                        continue;
                    }
                    picture.contacts.insert(
                        record.contact_id.clone(),
                        Contact {
                            route: route.into(),
                            key: record.contact_id.clone(),
                            name: record
                                .identity
                                .as_ref()
                                .map(|identity| identity.name.clone()),
                            references: record
                                .identity
                                .as_ref()
                                .map_or_else(Vec::new, |identity| identity.references.clone()),
                            position: [record.position[0], pose.y, record.position[1]],
                            charted: false,
                        },
                    );
                }
            }
        }
        let radar = chart
            .map(|chart| &chart.0)
            .or_else(|| sharing.map(|sharing| &sharing.radar));
        if let Some(radar) = radar.filter(|radar| radar.range.is_finite() && radar.range > 0.0) {
            for entity in world.into_iter().flat_map(|world| &world.0.entities) {
                if concealed(&entity.uuid)
                    || !entity
                        .tags
                        .iter()
                        .any(|tag| tag == crate::console_delivery::CHARTED_TAG)
                    || !entity
                        .tags
                        .iter()
                        .any(|tag| radar.shows.iter().any(|allowed| allowed.as_str() == tag))
                {
                    continue;
                }
                let Some(position) = entity
                    .position
                    .filter(|position| position.iter().all(|value| value.is_finite()))
                else {
                    continue;
                };
                if pose.distance(Vec3::from_array(position)) > radar.range {
                    continue;
                }
                picture.contacts.insert(
                    entity.uuid.clone(),
                    Contact {
                        route: entity.uuid.clone(),
                        key: entity.uuid.clone(),
                        name: entity.name.clone(),
                        references: Vec::new(),
                        position,
                        charted: true,
                    },
                );
            }
        }
        picture
    }
    pub fn request(&self, requested: &str) -> Option<&Contact> {
        if let Some(contact) = self.contacts.get(requested) {
            return Some(contact);
        }
        let mut matches = self.contacts.values().filter(|contact| {
            contact.key == requested
                || contact.name.as_deref() == Some(requested)
                || contact
                    .references
                    .iter()
                    .any(|reference| reference == requested)
        });
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }
    /// Resolve only a declared interaction choice. A Dock destination is another
    /// public reference, so its opaque spelling must resolve in the SAME picture
    /// before translating back to the authored command destination.
    pub fn canonical_order(
        &self,
        requested: &crate::civilian::CivilianOrder,
        options: &[crate::civilian::traffic::CivilianOrderOption],
    ) -> Option<crate::civilian::CivilianOrder> {
        options
            .iter()
            .find_map(|option| match (requested, &option.order) {
                (
                    crate::civilian::CivilianOrder::Dock {
                        structure: requested,
                    },
                    crate::civilian::CivilianOrder::Dock {
                        structure: authored,
                    },
                ) => {
                    let actual = self.request(requested)?;
                    let accepted = self.request(authored)?;
                    (actual.key == accepted.key).then(|| option.order.clone())
                }
                _ => (requested == &option.order).then(|| option.order.clone()),
            })
    }
    /// Prefer a retained explicit handle, otherwise require one route candidate.
    pub fn routed(&self, reference: &str) -> Option<&Contact> {
        if let Some(contact) = self.contacts.get(reference) {
            return Some(contact);
        }
        let mut matches = self
            .contacts
            .values()
            .filter(|contact| contact.route == reference);
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }
}

/// Identified measured contacts expose fixed authored interaction choices, not
/// live itinerary/compliance. Explicit permitted Navigation claims retain their
/// supplied fields; this publisher never refreshes those facts from an actor.
pub(crate) fn civilian_rows(
    picture: &Picture,
    civilians: &Query<(
        &crate::entities::spawner::EntityUuid,
        Option<&crate::entities::spawner::EntityName>,
        &crate::civilian::CivilianSection,
        &crate::civilian::CivilianTraffic,
    )>,
    received: Option<&std::collections::HashMap<SystemId, crate::core::messages::SystemBlackboard>>,
    owner: &SystemId,
) -> Vec<crate::core::messages::CivilianTrafficSnapshot> {
    use crate::core::messages::{
        CivilianOrderOptionSnapshot, CivilianTrafficSnapshot, SystemBlackboard,
    };
    let mut rows = BTreeMap::new();
    if let Some(received) = received {
        // Only another explicitly readable publication is a received claim;
        // never turn our own previous publication into fresh telemetry.
        let mut claims: Vec<_> = received
            .iter()
            .filter(|(source, _)| *source != owner)
            .collect();
        claims.sort_by_key(|(source, _)| *source);
        for (_, board) in claims {
            if let SystemBlackboard::Navigation(board) = board {
                for row in &board.civilians {
                    if let Some(contact) = picture
                        .contacts
                        .get(&row.uuid)
                        .filter(|contact| !contact.charted && contact.name.is_some())
                    {
                        let mut claim = row.clone();
                        claim.name = contact.name.clone().unwrap_or_default();
                        rows.entry(contact.key.clone()).or_insert(claim);
                    }
                }
            }
        }
    }
    for contact in picture
        .contacts
        .values()
        .filter(|contact| !contact.charted && contact.name.is_some())
    {
        let Some((_, _, section, _)) = civilians.iter().find(|(uuid, ..)| uuid.0 == contact.route)
        else {
            continue;
        };
        rows.entry(contact.key.clone())
            .or_insert_with(|| CivilianTrafficSnapshot {
                uuid: contact.key.clone(),
                name: contact.name.clone().unwrap_or_default(),
                route: String::new(),
                leg: 0,
                legs: 0,
                order: String::new(),
                order_destination: String::new(),
                compliance: String::new(),
                reason: String::new(),
                order_options: section
                    .0
                    .order_options
                    .iter()
                    .map(|option| CivilianOrderOptionSnapshot {
                        id: option.id.clone(),
                        label: option.label.clone(),
                        order: option.order.clone(),
                    })
                    .collect(),
            });
    }
    rows.into_values().collect()
}
