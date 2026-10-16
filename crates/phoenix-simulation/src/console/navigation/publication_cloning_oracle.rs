// Test-only e13490cc publication path, retaining original deep copies as an oracle.
use super::*;
use std::collections::BTreeMap;

pub(super) fn publish(
    transit: Option<Res<crate::console::helm::transit::StaticTransitScopes>>,
    knowledge: crate::console::navigation::knowledge::Environment,
    ship_config: Res<crate::lobby::server::ShipClientConfigResource>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    // The civilian traffic picture (issue #1028). Read-only, so this system
    // stays a pure publisher; the state itself is advanced by
    // `civilian::tick_civilian_traffic` in `SimSet::Input`.
    civilians_q: Query<(
        &crate::entities::spawner::EntityUuid,
        Option<&crate::entities::spawner::EntityName>,
        &crate::civilian::CivilianSection,
        &crate::civilian::CivilianTraffic,
    )>,
    mut ship_q: Query<
        (
            Entity,
            &NavigationWaypoint,
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
            &mut crate::server_app::ShipSystemBlackboards,
            // Where the human seek landed this system, if anywhere (issue
            // #984). `Option` because only `LocalShip` carries the component,
            // and optional access filters no archetype — the matched set, and
            // so the iteration order, is exactly what it was.
            Option<&crate::ship::components::HumanSeekingHosts>,
            Option<&super::sharing::NavigationSharing>,
            Option<&crate::ship_plugin::ShipSystemControlSources>,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&sharing::NavigationSharingProfile>,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    let cfg = &ship_config.0;
    // Built once for the whole publish rather than per ship: it is the same
    // world picture for everyone, and UUID-sorted so archetype order never
    // reaches the wire.
    let civilians = civilian_traffic_rows(&civilians_q, world_config.as_deref());
    for (entity, waypoint, is_local, mut bbs, hosts, sharing, sources, config, profile) in
        ship_q.iter_mut()
    {
        let Some(nav_id) = navigation_owner(config, profile) else {
            continue;
        };
        let navigation_waypoint = waypoint.snapshot();
        let formation = config
            .zip(knowledge.actor(entity))
            .and_then(|(config, actor)| {
                crate::console::navigation::formation::view(
                    crate::console::navigation::formation::requests(config, &bbs, &nav_id, actor),
                    &knowledge.formation_readings(entity, config, &bbs, &nav_id),
                    waypoint.mode(),
                )
            });
        let picture = knowledge.picture(entity, config, &nav_id, Some(&bbs), profile);
        let visible_civilians = if picture.configured {
            let received = config.map(|config| {
                let view = config.0.observations_for_system(&nav_id, &bbs.0);
                bbs.0
                    .keys()
                    .filter_map(|key| view.get(key).map(|value| (key.clone(), value.clone())))
                    .collect()
            });
            cloning_civilian_rows(&picture, &civilians_q, received.as_ref(), &nav_id)
        } else {
            civilians.clone()
        };
        let mut bb = if is_local {
            NavigationBlackboard {
                course_generation: waypoint.generation(),
                formation: formation.clone(),
                charted_points: picture
                    .contacts
                    .values()
                    .filter_map(|contact| contact.charted_point())
                    .collect(),
                charted_course: match waypoint.mode() {
                    Some(WaypointMode::ChartedXYZ { course }) => Some(course.clone()),
                    _ => None,
                },
                transit_routes: transit
                    .as_deref()
                    .map_or_else(Vec::new, |scopes| scopes.views()),
                nav_chart_range: cfg.nav_chart_range,
                nav_chart_shows: cfg.nav_chart_shows.clone(),
                nav_chart_selects: cfg.nav_chart_selects.clone(),
                navigation_waypoint,
                civilians: visible_civilians,
                host_station: hosts.and_then(|h| h.0.get(&nav_id).cloned()),
                sharing: None,
                received_chart: Vec::new(),
            }
        } else {
            NavigationBlackboard {
                course_generation: waypoint.generation(),
                formation: formation.clone(),
                charted_points: picture
                    .contacts
                    .values()
                    .filter_map(|contact| contact.charted_point())
                    .collect(),
                charted_course: match waypoint.mode() {
                    Some(WaypointMode::ChartedXYZ { course }) => Some(course.clone()),
                    _ => None,
                },
                transit_routes: transit
                    .as_deref()
                    .map_or_else(Vec::new, |scopes| scopes.views()),
                navigation_waypoint,
                ..Default::default()
            }
        };
        if let Some(sharing) = sharing {
            bb.sharing = Some(sharing.0.sharing_view(
                sources.is_some_and(|sources| super::sharing::operating(sources, &sharing.0.owner)),
            ));
            bb.received_chart = sharing.0.projection();
        }
        bbs.0
            .insert(nav_id.clone(), SystemBlackboard::Navigation(bb));
    }
}

fn cloning_civilian_rows(
    picture: &crate::console::navigation::knowledge::Picture,
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
