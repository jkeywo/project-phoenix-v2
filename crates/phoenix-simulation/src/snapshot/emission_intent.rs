//! Exact optional accepted equipment ownership, shared by both restore doors.
use super::*;
use phoenix_sim_gameplay::ship::emission_profile::profile;

fn validate_row(world: &World, live: Option<Entity>, row: &EntityState) -> Result<(), String> {
    let mut warnings = Vec::new();
    let accepted = row
        .spawn
        .as_ref()
        .map(|origin| {
            origin
                .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
                .ok_or("emission intent candidate source cannot resolve")
        })
        .transpose()?;
    if !warnings.is_empty() {
        return Err("emission intent candidate recipe is not exact".into());
    }
    let config = match accepted.as_ref() {
        Some(config) => config.ship_config.as_ref(),
        None => live
            .and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity))
            .map(|config| &config.0),
    };
    let expected = config.map(profile).transpose()?.flatten();
    match (&expected, &row.emission_intent) {
        (None, None) => {}
        (Some(_), Some(state)) => {
            state.validate(config.ok_or("missing actual emission owner graph")?)?
        }
        _ => return Err("saved emission intent owner differs from accepted equipment".into()),
    }
    for (key, board) in &row.blackboards {
        let crate::core::messages::SystemBlackboard::Command(board) = board else {
            continue;
        };
        if let Some(view) = &board.emission {
            if row.emission_intent.as_ref().is_none_or(|state| {
                view.owner != state.owner
                    || view.intent != state.intent
                    || key != &state.owner.0
                    || board.command_system_id != state.owner
            }) {
                return Err(
                    "saved emission publication differs from its actual Command intent".into(),
                );
            }
        }
    }
    Ok(())
}
pub(super) fn validate_entity(
    world: &World,
    entity: Entity,
    row: &EntityState,
) -> Result<(), String> {
    validate_row(world, Some(entity), row)
}
pub(super) fn validate_roster(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    for row in &snapshot.entities {
        let live = world.try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
            .and_then(|mut query| query.iter(world).find_map(|(entity, id)| (id.0 == row.uuid).then_some(entity)));
        validate_row(world, live, row).map_err(|reason| (row.uuid.clone(), reason))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::command::emission::ShipEmissionIntent;
    #[test]
    fn exact_required_owner_and_publication_apply_to_disabled_rows_and_candidate_recipes() {
        let config = crate::entities::include_resolve::load_entity_config(
            "assets/entities/emission_profile_cruiser.toml",
        )
        .unwrap();
        let ship = config.ship_config.unwrap();
        let (owner, profile) = profile(&ship).unwrap().unwrap();
        let state = phoenix_sim_gameplay::ship::emission_profile::EmissionIntentState::new(
            owner.clone(),
            &profile,
        );
        let mut world = World::new();
        let entity = world
            .spawn((
                EntityUuid("quiet-ship".into()),
                crate::ship_plugin::ShipConfigComponent(ship),
                ShipEmissionIntent(state.clone()),
                bevy::ecs::entity_disabling::Disabled,
            ))
            .id();
        let row = EntityState {
            uuid: "quiet-ship".into(),
            emission_intent: Some(state),
            ..Default::default()
        };
        validate_entity(&world, entity, &row).unwrap();
        let mut saved = PhoenixSnapshot {
            entities: vec![row.clone()],
            ..Default::default()
        };
        validate_roster(&world, &saved).unwrap();
        for fault in 0..3 {
            let mut bad = row.clone();
            match fault {
                0 => bad.emission_intent = None,
                1 => bad.emission_intent.as_mut().unwrap().owner = SystemId("foreign".into()),
                _ => bad.blackboards.push((
                    "foreign".into(),
                    crate::core::messages::SystemBlackboard::Command(
                        crate::core::messages::CommandBlackboard {
                            command_system_id: owner.clone(),
                            emission: Some(phoenix_model::emission_profile::EmissionIntentView {
                                owner: owner.clone(),
                                intent: profile.initial,
                                controllable: true,
                            }),
                            ..Default::default()
                        },
                    ),
                )),
            }
            assert!(validate_entity(&world, entity, &bad).is_err());
            saved.entities[0] = bad;
            assert!(validate_roster(&world, &saved).is_err());
        }
        // A candidate's accepted recipe takes precedence over the old live fit.
        let mut foreign = row.clone();
        foreign.spawn = Some(crate::world::spawn_origin::SpawnOrigin {
            template_path: "assets/entities/identity_cruiser.toml".into(),
            name: "quiet-ship".into(),
            ..Default::default()
        });
        assert!(validate_entity(&world, entity, &foreign).is_err());
        let mut absent = row;
        absent.spawn = Some(crate::world::spawn_origin::SpawnOrigin {
            template_path: "assets/entities/emission_profile_cruiser.toml".into(),
            name: "quiet-ship".into(),
            ..Default::default()
        });
        world.despawn(entity);
        validate_row(&world, None, &absent).unwrap();
        absent.emission_intent = None;
        assert!(validate_row(&world, None, &absent).is_err());
    }
}
