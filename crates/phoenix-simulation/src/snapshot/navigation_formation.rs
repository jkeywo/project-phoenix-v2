//! Retained course ownership and genuine received coordination before writes.
use super::*;

fn validate(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
    inbox: &[crate::core::messages::CommsMessage],
    tick: u64,
) -> Result<(), String> {
    let Some(saved) = &row.navigation_waypoint else {
        return Ok(());
    };
    let Some(course) = &saved.formation else {
        return Ok(());
    };
    if !course.valid_at(tick)
        || saved.generation == 0
        || saved.source_uuid.is_some()
        || saved.position != course.measurement.slot_point(&course.request)
    {
        return Err("saved Formation course has invalid position, identity or clock".into());
    }
    let (config, profile) = if let Some(entity) = live {
        (
            world
                .get::<crate::ship_plugin::ShipConfigComponent>(entity)
                .cloned(),
            world
                .get::<crate::console::navigation::sharing::NavigationSharingProfile>(entity)
                .cloned(),
        )
    } else if let Some(origin) = &row.spawn {
        let accepted = origin
            .resolve(
                &crate::entities::loader::WasmTemplateLoader,
                &mut Vec::new(),
            )
            .ok_or("cannot resolve actual Formation Navigation equipment")?;
        let profile = accepted.navigation_console.as_ref().and_then(|nav| {
            nav.sharing.as_ref().map(|sharing| {
                crate::console::navigation::sharing::NavigationSharingProfile {
                    owner: sharing.system.clone(),
                    radar: nav.system_chart.clone(),
                }
            })
        });
        (
            accepted
                .ship_config
                .map(crate::ship_plugin::ShipConfigComponent),
            profile,
        )
    } else {
        (None, None)
    };
    let config = config.ok_or("Formation requires actual authored Navigation composition")?;
    if crate::console::navigation::server::navigation_owner(Some(&config), profile.as_ref())
        .as_ref()
        != Some(&course.owner)
    {
        return Err("saved Formation course does not belong to its actual Navigation owner".into());
    }
    let held = inbox
        .iter()
        .find(|message| message.id == course.request.message_id)
        .ok_or("saved Formation course lacks its actual retained received request")?;
    // Reconstruct only this canonical received message for the same typed
    // permission check used by the live consumer. Never trust saved GUI rows.
    let mut boards = crate::server_app::ShipSystemBlackboards::default();
    for comms in config
        .0
        .systems
        .iter()
        .filter(|system| system.kind == crate::ship::system_registry::COMMS_KIND)
    {
        boards.0.insert(
            comms.id.clone(),
            crate::core::messages::SystemBlackboard::Comms(
                crate::core::messages::CommsBlackboard {
                    messages: vec![held.clone()],
                    ..Default::default()
                },
            ),
        );
    }
    if !crate::console::navigation::formation::requests(&config, &boards, &course.owner, &row.uuid)
        .contains(&course.request)
    {
        return Err(
            "saved Formation course differs from its genuinely heard/accepted permitted request"
                .into(),
        );
    }
    let publications: std::collections::HashMap<_, _> = row
        .blackboards
        .iter()
        .map(|(key, value)| (crate::core::messages::SystemId(key.clone()), value.clone()))
        .collect();
    let permitted = config
        .0
        .observations_for_system(&course.owner, &publications);
    let conflicting_sample =
        row.broad
            .iter()
            .flat_map(|states| states.iter())
            .any(|(instrument, state)| {
                permitted
                    .get(instrument)
                    .and_then(crate::science::contact_routing::observations)
                    .is_some()
                    && state.tracks.values().any(|track| {
                        track.contact_id == course.leader
                            && track.observed_tick == course.measurement.observed_tick
                            && (track.position != course.measurement.position
                                || track.velocity != Some(course.measurement.velocity))
                    })
            });
    if conflicting_sample {
        return Err("saved Formation cache disagrees with its retained canonical sample".into());
    }
    // A newer canonical track may already have replaced the dated sample used
    // by the preceding Navigation phase. It cannot rewrite that retained cache.
    let retained_alias =
        row.broad
            .iter()
            .flat_map(|states| states.iter())
            .any(|(instrument, state)| {
                permitted
                    .get(instrument)
                    .and_then(crate::science::contact_routing::observations)
                    .is_some()
                    && state.tracks.values().any(|track| {
                        track.contact_id == course.leader
                            && track.observed_tick >= course.measurement.observed_tick
                    })
            });
    // Carried actors do not run operational Navigation refresh. Their dated
    // intent survives ordinary retained-track aging without becoming executable.
    // The actual custody graph is validated by its existing whole-roster owner.
    let carried = row.probe_custody.as_ref().is_some_and(|custody| {
        matches!(
            custody.0,
            phoenix_model::craft::CraftLocation::Carried { .. }
        )
    });
    if !retained_alias && !course.paused && !carried {
        return Err(
            "saved Formation leader lacks a retained permitted public contact record".into(),
        );
    }
    Ok(())
}
pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        if row
            .navigation_waypoint
            .as_ref()
            .is_none_or(|waypoint| waypoint.formation.is_none())
        {
            continue;
        }
        if !world.contains_resource::<SimTick>()
            || !world.contains_resource::<Time<bevy::time::Fixed>>()
        {
            return Err((
                row.uuid.clone(),
                "Formation continuation requires actual logical and fixed clocks".into(),
            ));
        }
        let live = world.try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| query.iter(world).find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity)));
        validate(
            world,
            live,
            row,
            snapshot
                .comms
                .as_ref()
                .map_or(&[], |comms| comms.inbox.as_slice()),
            snapshot.tick,
        )
        .map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}
pub(super) fn validate_live(
    world: &World,
    entity: Entity,
    row: &EntityState,
) -> Result<(), String> {
    if row
        .navigation_waypoint
        .as_ref()
        .is_none_or(|waypoint| waypoint.formation.is_none())
    {
        return Ok(());
    }
    let tick = world
        .get_resource::<SimTick>()
        .ok_or("Formation continuation requires its logical clock")?
        .0;
    let inbox = world
        .get_resource::<crate::comms::server::CommsInboxRes>()
        .ok_or("Formation continuation requires its actual retained Comms owner")?;
    validate(world, Some(entity), row, &inbox.0.messages(), tick)
}
