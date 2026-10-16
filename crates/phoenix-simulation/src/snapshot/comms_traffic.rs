//! Candidate-only traffic intent/publication consistency before any restore write.
//! No clock refresh, private alert fallback or proof of erased emission history.
use super::*;
use phoenix_model::{comms_channel::CommsRadioConfig, messages::SystemBlackboard};

fn bindings(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
) -> Result<
    (
        Option<CommsRadioConfig>,
        Option<crate::ship::config::ShipConfig>,
    ),
    String,
> {
    if let Some(origin) = &row.spawn {
        let mut warnings = Vec::new();
        let accepted = origin
            .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
            .ok_or("traffic candidate recipe cannot resolve")?;
        if !warnings.is_empty() {
            return Err("traffic candidate recipe is not exact".into());
        }
        return Ok((
            accepted.comms.and_then(|comms| comms.radio),
            accepted.ship_config,
        ));
    }
    Ok((
        live.and_then(|entity| world.get::<crate::comms::radio::RadioProfile>(entity))
            .map(|radio| radio.0.clone()),
        live.and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity))
            .map(|ship| ship.0.clone()),
    ))
}

fn validate_row(
    world: &World,
    live: Option<Entity>,
    row: &EntityState,
    now: u64,
) -> Result<(), String> {
    let (radio, ship) = bindings(world, live, row)?;
    match (
        radio
            .as_ref()
            .and_then(|radio| radio.traffic.as_ref().map(|profile| (radio, profile))),
        &row.comms_traffic,
    ) {
        (None, None) => {}
        (Some((radio, profile)), Some(saved)) if saved.owner == radio.system => {
            saved.validate(profile, &crate::comms::traffic::channels(radio), now)?;
        }
        _ => return Err("saved traffic owner differs from exact accepted Radio equipment".into()),
    }
    for (source, board) in &row.blackboards {
        if let SystemBlackboard::Captain(board) = board {
            if let Some(observation) = &board.alert_observation {
                let ship = ship
                    .as_ref()
                    .ok_or("dated alert requires actual accepted ship ownership")?;
                let alert = crate::comms::traffic::unique_system(
                    ship,
                    crate::ship::system_registry::RED_ALERT_KIND,
                )
                .ok_or("dated alert owner is ambiguous")?;
                let publisher = crate::comms::traffic::unique_system(
                    ship,
                    crate::ship::system_registry::CAPTAIN_KIND,
                )
                .ok_or("dated alert publication is ambiguous")?;
                let sampled = observation
                    .observed_tick
                    .ok_or("dated alert sample has no producer clock")?;
                if source != &publisher.0
                    || board.red_alert_system_id != alert
                    || row.red_alert != Some(board.red_alert)
                    || observation != &crate::comms::traffic::alert_observation(sampled, &alert)
                    || sampled > now
                {
                    return Err("saved alert publication contradicts its accepted owner or original publication frontier".into());
                }
            }
        }
        let SystemBlackboard::Comms(board) = board else {
            continue;
        };
        let Some(view) = &board.traffic else {
            continue;
        };
        let radio = radio
            .as_ref()
            .filter(|radio| radio.system.0 == *source)
            .ok_or("traffic publication is not the actual Radio owner")?;
        let profile = radio
            .traffic
            .as_ref()
            .ok_or("traffic publication lacks accepted equipment")?;
        let state = row
            .comms_traffic
            .as_ref()
            .ok_or("traffic publication lacks canonical owner")?;
        let channels = crate::comms::traffic::channels(radio);
        let slots = profile
            .listening
            .as_ref()
            .map_or(channels.len() as u16, |profile| {
                profile.simultaneous_channels
            });
        let selected: Vec<_> = state
            .configured_receive_channels(profile, &channels)?
            .into_iter()
            .map(str::to_owned)
            .collect();
        if view.channels != channels
            || view.priority != state.priority
            || view.simultaneous_channels != slots
            || view.last_report != state.last_report
            || view.report_supported != profile.alert.is_some()
            || view.controllable
            || (!view.monitored.is_empty() && view.monitored != selected)
            || (view.observed_tick.is_none() && !view.monitored.is_empty())
            || view.observed_tick.is_some_and(|tick| tick > now)
        {
            return Err(
                "traffic publication contradicts canonical intent or its dated coverage shape"
                    .into(),
            );
        }
    }
    Ok(())
}

pub(super) fn validate_entity(
    world: &World,
    entity: Entity,
    row: &EntityState,
    now: u64,
) -> Result<(), String> {
    validate_row(world, Some(entity), row, now)
}

pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        let live = world.try_query_filtered::<(Entity,&EntityUuid),Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| query.iter(world).find_map(|(entity,id)| (id.0 == row.uuid).then_some(entity)));
        validate_row(world, live, row, snapshot.tick)
            .map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}
