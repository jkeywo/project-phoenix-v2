//! Initial custody commits in the actual spawn transaction, never a live frame.
use super::*;
use bevy::ecs::entity_disabling::Disabled;

/// Called before the ordinary identity mint/physical spawn. Selection cannot
/// reinterpret a carried actor as a crew hull or silently discard its recipe.
pub(crate) fn initial_location(
    world: &crate::world::config::WorldConfig,
    row: &crate::world::config::WorldEntity,
    config: &EntityConfig,
    crew_placement: bool,
) -> Result<Option<CraftLocation>, String> {
    let Some(custody) = &row.carried_by else {
        return Ok(None);
    };
    if crew_placement || config.probe.is_none() {
        return Err(
            "initial carried craft cannot be a selected crew hull or unsupported probe".into(),
        );
    }
    let parent = world
        .name_to_uuid
        .get(&custody.parent)
        .ok_or("initial carried craft has no accepted parent identity")?;
    let location = CraftLocation::Carried {
        parent_uuid: parent.clone(),
        bay: custody.bay.clone(),
    };
    Ok(Some(location))
}

/// Reuse the actual earlier GameStart boot record, including its saved UUID on
/// resume. The freshly registered World name must not override that identity.
pub(crate) fn initial_game_start_location(
    world: &crate::world::config::WorldConfig,
    row: &crate::world::config::WorldEntity,
    config: &EntityConfig,
    crew_placement: bool,
    previous_boot: &[crate::snapshot::GameStartEntityUuid],
) -> Result<Option<CraftLocation>, String> {
    let mut location = initial_location(world, row, config, crew_placement)?;
    let Some(custody) = &row.carried_by else {
        return Ok(location);
    };
    let (index, parent) = world
        .entities
        .iter()
        .enumerate()
        .find(|(_, parent)| parent.name.as_deref() == Some(custody.parent.as_str()))
        .ok_or("initial carried parent has no declared row")?;
    if parent.spawn_on == crate::world::config::WorldEntitySpawnOn::GameStart {
        let index =
            u32::try_from(index).map_err(|_| "initial parent row exceeds boot identity bound")?;
        let actual = previous_boot
            .iter()
            .find(|record| record.authored_index == index)
            .ok_or("initial carried parent was not accepted at this GameStart")?;
        if let Some(CraftLocation::Carried { parent_uuid, .. }) = &mut location {
            parent_uuid.clone_from(&actual.entity_uuid);
        }
    }
    Ok(location)
}

pub(crate) fn stage_initial(
    commands: &mut Commands,
    entity: Entity,
    location: Option<CraftLocation>,
) {
    let Some(location) = location else {
        return;
    };
    // Inactivity is inserted with the body itself, before ordinary Rapier,
    // sensing, energy or render queries can see an operational free child.
    commands
        .entity(entity)
        .insert((ProbeCustody(location.clone()), Disabled, Visibility::Hidden));
    commands.queue(move |world:&mut World| {
        let result=(|| {
            let CraftLocation::Carried{parent_uuid,bay}=&location else{return Err("initial custody must be Carried".into());};
            let parent=runtime::entity_for(world,parent_uuid).ok_or("initial carried parent was not materialized")?;
            let config=&world.get::<crate::ship_plugin::ShipConfigComponent>(parent).ok_or("initial parent lacks accepted equipment")?.0;
            let binding=config.system(bay)
                .filter(|system|system.kind==crate::ship::system_registry::CRAFT_BAY_KIND)
                .and_then(|system|phoenix_sim_gameplay::ship::composition::craft_bay_composition(&system.components))
                .ok_or("initial parent lacks actual supported bay")?;
            let limit=binding.config.capacity_space;
            if world.get::<ShipBayJobs>(parent).is_none_or(|jobs|!jobs.0.contains_key(bay)) {
                return Err("initial parent lacks its canonical bay Job owner".into());
            }
            let mut query=world.query_filtered::<(&PhysicalProbe,&ProbeCustody),Allow<Disabled>>();
            let occupied=query.iter(world).filter(|(_,custody)|
                matches!(&custody.0,CraftLocation::Carried{parent_uuid:actual,bay:owner} if actual==parent_uuid && owner==bay))
                .try_fold(0_u64,|sum,(probe,_)|sum.checked_add(probe.0.space));
            if occupied.is_none_or(|occupied|occupied>limit) {return Err("initial carried allocation exceeds actual bay".into());}
            set_carried(world,entity,parent_uuid,bay)
        })();
        if let Err(error)=result {
            error!("initial carried probe refused: {error}");
            // Activation has already checked the complete accepted recipe. A
            // defensive missing actual owner still cannot leave a free/orphan
            // child. This is refused materialization, not a gameplay death.
            let _=crate::gm::operations::despawn::remove_entity(world,entity);
        }
    });
}
