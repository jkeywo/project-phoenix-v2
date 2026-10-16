//! One disposable child's prepared initial fit. No Live entry or refit job.
use super::{rehearsal_request::*, test_protocol::Launch};
use crate::entities::loader::TemplateLoader;
use crate::resources::fitting_server::{CompiledShipFitting, FittingPlannerCatalog, ShipFitting};
use crate::ship::power::ShipPowerSystem;
use bevy::prelude::*;
use std::collections::BTreeMap;

#[derive(Resource)]
pub struct PreparedRehearsal {
    request: RehearsalRequest,
    choices: BTreeMap<String, String>,
    accepted: crate::entities::config::EntityConfig,
    pub(crate) attempted: bool,
    pub(crate) error: Option<String>,
    pub(crate) target: Option<Entity>,
    pub(crate) initialized_tick: Option<u64>,
}

/// Only captured bytes are used here; no cache/filesystem fallback is available.
pub fn prepare(
    files: BTreeMap<String, String>,
    launch: &Launch,
) -> Result<Option<PreparedRehearsal>, String> {
    let Some(request) = launch.rehearsal.as_ref() else {
        return Ok(None);
    };
    request.validate(&launch.selection)?;
    if launch.breakpoint.is_some() {
        return Err(
            "A rehearsal owns its finite window; a Test breakpoint cannot replace it".into(),
        );
    }
    let report = super::test_source::validate_selection(files.clone(), &launch.selection);
    if !report.accepted {
        return Err("Captured rehearsal source did not pass Test validation".into());
    }
    let source = super::Sources(files);
    let accepted = source
        .load_template(&launch.selection.ship)
        .ok_or("Captured rehearsal hull did not resolve")?;
    let mut context = crate::resources::fitting_server::planner_context(&accepted)
        .ok_or("Captured rehearsal hull has no accepted fitting profile")?;
    context.hull_path = launch.selection.ship.clone();
    if accepted
        .ship_config
        .as_ref()
        .is_none_or(|ship| ship.stations.len() != 6)
    {
        return Err("The first fitting rehearsal requires the six-seat Cruiser".into());
    }
    let choices = request.choices(&context)?;
    // The normal player-hull path merges row overrides. The first profiles do
    // not permit them to quietly change the favourite's bound source context.
    let world = source
        .0
        .get(&launch.selection.world)
        .and_then(|text| crate::world::config::parse_world(text).ok())
        .ok_or("Captured rehearsal World did not resolve")?;
    let slot_row = world
        .entities
        .iter()
        .find(|row| row.id.as_deref() == Some("rehearsal"))
        .ok_or("Rehearsal World is missing its explicit controlled row")?;
    if slot_row.overrides.is_some() || slot_row.template_path != CRUISER {
        return Err("Rehearsal player row must retain the accepted Cruiser fitting context".into());
    }
    Ok(Some(PreparedRehearsal {
        request: request.clone(),
        choices,
        accepted,
        attempted: false,
        error: None,
        target: None,
        initialized_tick: None,
    }))
}

pub fn install(app: &mut App, prepared: Option<PreparedRehearsal>) {
    if let Some(prepared) = prepared {
        use crate::authoritative::{DeclareState, StateClass};
        app.declare_state::<PreparedRehearsal>(
            StateClass::TestInfra,
            "equipment-fitting-and-refitting",
        )
        .insert_resource(prepared);
    }
}

/// Runs only after the real GameStart spawn and its explicit deferred boundary.
/// All fallible work completes against clones before touching actual equipment.
pub(crate) fn initialize(world: &mut World) {
    if !world.contains_resource::<PreparedRehearsal>() {
        return;
    }
    if world.resource::<PreparedRehearsal>().attempted {
        return;
    }
    let result = stage(world);
    let tick = world.resource::<crate::sim_tick::SimTick>().0;
    let mut prepared = world.resource_mut::<PreparedRehearsal>();
    prepared.attempted = true;
    match result {
        Ok(entity) => {
            prepared.target = Some(entity);
            prepared.initialized_tick = Some(tick);
        }
        Err(error) => {
            prepared.error = Some(error);
            // Refuse before the child's first machinery interval. No default
            // fit is allowed to operate under the selected favourite's label.
            world.resource_mut::<super::test_clock::TestClock>().paused = true;
            world
                .resource_mut::<crate::gm::actions::action::SimulationPaused>()
                .0 = true;
        }
    }
}

fn stage(world: &mut World) -> Result<Entity, String> {
    if !world.contains_resource::<super::test_clock::DisposableTest>() {
        return Err("Fitting initialization requires a disposable Test child".into());
    }
    if !world
        .get_resource::<crate::ship_slots::FrozenShipSlots>()
        .is_some_and(|slots| {
            slots
                .0
                .iter()
                .filter(|slot| {
                    slot.slot_id == "rehearsal" && slot.hull == CRUISER && slot.claimant.is_some()
                })
                .count()
                == 1
        })
    {
        return Err("Rehearsal target is not the actual frozen controlled slot".into());
    }
    let mut query = world.query::<(Entity, &crate::ship_slots::AuthoredShipSlotId)>();
    let targets: Vec<_> = query
        .iter(world)
        .filter(|(_, slot)| slot.0 == "rehearsal")
        .map(|(entity, _)| entity)
        .collect();
    let [entity] = targets.as_slice() else {
        return Err("Rehearsal must bind exactly one actual frozen ship slot".into());
    };
    let entity = *entity;
    let prepared = world.resource::<PreparedRehearsal>();
    let profile = world
        .get::<CompiledShipFitting>(entity)
        .ok_or("Actual fitting profile is absent")?
        .clone();
    let mut fitting = world
        .get::<ShipFitting>(entity)
        .ok_or("Actual fitting state is absent")?
        .clone();
    let mut power = world
        .get::<ShipPowerSystem>(entity)
        .ok_or("Actual finite Power owner is absent")?
        .clone();
    let catalog = world
        .get::<FittingPlannerCatalog>(entity)
        .ok_or("Actual fitting catalog is absent")?;
    if world
        .get::<crate::entities::spawner::EntityTemplatePath>(entity)
        .is_none_or(|path| path.0 != CRUISER)
        || catalog.0.content_id != prepared.request.plan.content_id
        || fitting.last_tick.is_some()
        || !fitting.requested.is_empty()
        || !fitting.work.is_empty()
        || world
            .get::<crate::ship::compartment_damage::ShipDamage>(entity)
            .is_none()
        || world
            .get::<crate::infrastructure::InfrastructureCondition>(entity)
            .is_none()
        || world
            .get::<crate::entities::spawner::WeaponsConsoleSection>(entity)
            .is_none()
        || world
            .get::<crate::console::weapons::PhaserCombatConfigResource>(entity)
            .is_none()
    {
        return Err(
            "Actual rehearsal target is incomplete, already worked or differently bound".into(),
        );
    }
    let initial = crate::resources::energy_network::EnergyNetworkState::new(
        prepared
            .accepted
            .energy
            .as_ref()
            .ok_or("Accepted energy hardware is absent")?,
        prepared
            .accepted
            .ship_config
            .as_ref()
            .ok_or("Accepted ship definition is absent")?,
    )?;
    if power.0.capture_continuation().network != initial
        || prepared.accepted.energy.as_ref() != Some(power.0.config())
    {
        return Err(
            "Rehearsal cannot replace an already operating or differently fitted Power owner"
                .into(),
        );
    }
    for slot in &profile.0.config().slots {
        let current = fitting
            .equipment
            .installed(&slot.id)
            .ok_or("Initial fitting assembly is absent")?;
        if current.definition != slot.default_equipment {
            return Err("Rehearsal must start from the accepted default assemblies".into());
        }
        let item_id = current.id.clone();
        let choice = prepared
            .choices
            .get(&slot.id)
            .ok_or("Normalized choice is absent")?;
        profile.0.compatible(&slot.id, choice)?;
        fitting
            .equipment
            .assemblies
            .get_mut(&item_id)
            .expect("inspected actual assembly")
            .definition = choice.clone();
        fitting
            .equipment
            .desired
            .insert(slot.id.clone(), choice.clone());
    }
    fitting.equipment.validate(&profile.0)?;
    let before = power.0.capture_continuation();
    let effective = crate::resources::fitting_server::stage_capability(
        &profile.0,
        &fitting.equipment,
        Some(&mut power),
    )?;
    if before != power.0.capture_continuation() {
        return Err("Hypothetical equipment must preserve actual initial reserves and heat".into());
    }
    world.entity_mut(entity).insert((fitting, power));
    crate::resources::fitting_server::apply_capability(world, entity, &effective);
    Ok(entity)
}

#[cfg(test)]
#[path = "rehearsal_tests.rs"]
mod tests;
