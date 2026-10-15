//! Whole-candidate Scenario evidence validation. History is not current work.
use super::*;
use phoenix_model::activity::ActivityOutcome;
use phoenix_sim_world::world::activities::ActivityRequest;

fn slot_binding(
    world: &World,
    snapshot: &PhoenixSnapshot,
    slot: &str,
) -> Result<Option<(String, crate::entities::config::EntityConfig)>, String> {
    let Some(boot) = snapshot.boot_identity.as_ref() else {
        return Ok(None);
    };
    let base = world
        .get_resource::<crate::world::config::WorldConfig>()
        .ok_or("missing accepted base-world slot definition")?;
    super::preflight::validate_boot_identity_for_world(boot, base)
        .map_err(|error| format!("invalid activity launch identity: {error:?}"))?;
    let rows: Vec<_> = boot
        .game_start_entity_uuids
        .iter()
        .filter(|row| row.authored_slot_id.as_deref() == Some(slot))
        .collect();
    if rows.len() > 1 {
        return Err("duplicate activity launch-slot identity".into());
    }
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let frozen = snapshot
        .frozen_ship_slots
        .as_ref()
        .and_then(|slots| slots.0.iter().find(|row| row.slot_id == slot))
        .ok_or("activity launch slot lacks its frozen hull choice")?;
    let accepted = base
        .effective_ship_slots()
        .into_iter()
        .find(|row| row.id == slot)
        .ok_or("unknown activity launch slot")?;
    if !accepted
        .ships
        .iter()
        .any(|ship| ship.template_path == frozen.hull)
    {
        return Err("activity launch hull is outside its authored slot choices".into());
    }
    let authored = base
        .entities
        .get(row.authored_index as usize)
        .ok_or("missing activity launch row")?;
    let source = crate::entities::loader::resolve_entity_via(
        authored,
        &crate::entities::config_cache::get_config_cache(),
        &crate::entities::loader::WasmTemplateLoader,
    )
    .map_err(|error| format!("cannot resolve activity launch row: {error}"))?;
    let hull = crate::entities::loader::TemplateLoader::load_template(
        &crate::entities::loader::WasmTemplateLoader,
        &frozen.hull,
    )
    .ok_or("cannot resolve activity frozen hull")?;
    Ok(Some((
        row.entity_uuid.clone(),
        crate::server_app::player_hull_config(source, authored.overrides.as_ref(), Some(&hull)),
    )))
}

/// Derive only the candidate's selected source graph, never the live choice.
fn planning_frontier(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<
    (
        Vec<phoenix_sim_world::world::activities::SituationDefinition>,
        Vec<phoenix_sim_world::world::activities::SituationConfig>,
    ),
    String,
> {
    let runtime = world
        .get_resource::<WorldContentRuntime>()
        .ok_or("missing World activity owner")?;
    let stored = snapshot
        .scenario
        .as_ref()
        .ok_or("missing saved World activity owner")?;
    let mut definitions = world
        .get_resource::<crate::world::config::WorldConfig>()
        .map_or_else(
            || runtime.situation_definitions.clone(),
            |base| base.situation_definitions.clone(),
        );
    let mut configs = runtime.situation_configs.clone();
    let base = world.get_resource::<crate::world::config::WorldConfig>();
    let compiled = base.and_then(|base| base.compiled_planning.as_ref());
    match (&stored.plan_choice, compiled) {
        (Some(choice), Some(compiled)) => {
            let base = base.ok_or("missing plan base-world definition")?;
            let actor = compiled.actor_reference()?;
            base.validate_situation_actor(actor)?;
            let valid = if let Some(slot) = actor.strip_prefix("slot:") {
                slot_binding(world, snapshot, slot)?
                    .is_some_and(|(uuid, _)| uuid == choice.actor_uuid)
            } else {
                base.name_to_uuid.get(actor) == Some(&choice.actor_uuid)
            };
            if !valid {
                return Err("saved plan actor disagrees with accepted bootstrap identity".into());
            }
            let graph = compiled.validate_history(choice, &stored.situations, snapshot.tick)?;
            for definition in graph.definitions.values() {
                if definitions.iter().any(|row| row.id == definition.id) {
                    return Err("duplicate plan Scenario identity".into());
                }
                definitions.push(definition.clone());
            }
            // Keep authored actor references for the existing immutable actor /
            // historical equipment checks below; its request is bound once.
            validate_plan_publications(snapshot, choice, &graph)?;
            configs.extend(graph.leaves.values().cloned());
        }
        (Some(_), None) => return Err("saved plan has no accepted imported source closure".into()),
        (None, Some(compiled)) if stored.mission_elapsed.is_some() => {
            let base = base.ok_or("missing plan base-world definition")?;
            let actor = compiled.actor_reference()?;
            let initially_resolved = if let Some(slot) = actor.strip_prefix("slot:") {
                slot_binding(world, snapshot, slot)?.is_some()
            } else {
                base.entities
                    .iter()
                    .enumerate()
                    .find(|(_, row)| row.name.as_deref() == Some(actor))
                    .is_some_and(|(index, row)| {
                        (row.spawn_on == crate::world::config::WorldEntitySpawnOn::Immediate
                            && row.when.is_none()
                            && row.when_predicate.is_none())
                            || snapshot.boot_identity.as_ref().is_some_and(|boot| {
                                boot.game_start_entity_uuids
                                    .iter()
                                    .any(|row| row.authored_index as usize == index)
                            })
                    })
            };
            if initially_resolved {
                return Err(
                    "resolved maintenance plan lost its saved method/release frontier".into(),
                );
            }
        }
        _ => {}
    }
    if stored.plan_choice.is_none()
        && snapshot
            .entities
            .iter()
            .flat_map(|row| &row.blackboards)
            .any(|(_, board)| match board {
                crate::core::messages::SystemBlackboard::Repair(board) => {
                    !board.planned_requests.is_empty()
                }
                crate::core::messages::SystemBlackboard::Logistics(board) => {
                    !board.planned_requests.is_empty()
                }
                _ => false,
            })
    {
        return Err("planned publication lacks its saved World choice".into());
    }
    Ok((definitions, configs))
}

pub(super) fn validate(world: &World, snapshot: &PhoenixSnapshot) -> Result<(), String> {
    let Some(stored) = snapshot.scenario.as_ref() else {
        if world
            .get_resource::<WorldContentRuntime>()
            .is_some_and(|runtime| {
                !runtime.situation_configs.is_empty() || runtime.plan_choice.is_some()
            })
            || world
                .get_resource::<crate::world::config::WorldConfig>()
                .is_some_and(|base| base.planning.is_some())
        {
            return Err("configured situations require their saved World owner".into());
        }
        return Ok(());
    };
    let Some(runtime) = world.get_resource::<WorldContentRuntime>() else {
        return if stored.situations.instances.is_empty()
            && stored.plan_choice.is_none()
            && !stored.pending_events.iter().any(|event| event.kind == 13)
        {
            Ok(())
        } else {
            Err("activity evidence requires its World owner".into())
        };
    };
    let (definitions, configs) = planning_frontier(world, snapshot)?;
    stored.situations.validate(&definitions, snapshot.tick)?;
    let pending = stored
        .pending_events
        .iter()
        .filter(|event| event.kind == 13)
        .map(|event| {
            event
                .activity
                .clone()
                .ok_or("missing pending activity payload".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    crate::world::activities::validate_pending(&stored.situations, &pending, snapshot.tick)?;
    if stored
        .pending_events
        .iter()
        .any(|event| event.kind != 13 && event.activity.is_some())
    {
        return Err("activity payload has a non-activity event tag".into());
    }
    if !stored.situations.instances.is_empty() && !world.contains_resource::<SimTick>() {
        return Err("situations require the actual logical clock".into());
    }
    for (id, instance) in &stored.situations.instances {
        let authored = configs
            .iter()
            .find(|config| &config.id == id)
            .ok_or("unknown authored situation instance")?;
        world
            .get_resource::<crate::world::config::WorldConfig>()
            .ok_or("activity history requires its accepted base-world definition")?
            .validate_situation_actor(authored.request.actor())?;
        let actor = instance.request.actor();
        // A forged saved name map must not redirect an immutable binding.
        // Bootstrap mints the base world's named identities before spawning.
        let named = world
            .get_resource::<crate::world::config::WorldConfig>()
            .and_then(|config| config.name_to_uuid.get(authored.request.actor()))
            .or_else(|| runtime.name_to_uuid.get(authored.request.actor()))
            .map(String::as_str);
        let slot = authored.request.actor().strip_prefix("slot:");
        let slot_source = slot
            .map(|slot| slot_binding(world, snapshot, slot))
            .transpose()?
            .flatten();
        let valid_actor = authored.request.actor() == actor
            || named == Some(actor)
            || slot_source.as_ref().is_some_and(|(uuid, _)| uuid == actor);
        let mut expected = authored.request.clone();
        match &mut expected {
            ActivityRequest::Repair {
                actor: expected, ..
            }
            | ActivityRequest::Delivery {
                actor: expected, ..
            } => *expected = actor.into(),
        }
        if !valid_actor
            || instance.definition != authored.definition
            || instance.request != expected
        {
            return Err("saved situation disagrees with immutable authored bindings".into());
        }
        let row = snapshot.entities.iter().find(|row| row.uuid == actor);
        // A terminal fact (pending or consumed) permits the ordinary physical
        // owner to have removed/reassigned its row. Never demand today's slot
        // still contain yesterday's completed job.
        let terminal = instance.settled.is_some()
            || pending.iter().any(|fact| {
                fact.instance == *id
                    && !matches!(fact.outcome, ActivityOutcome::RepairAchieved { .. })
            });
        // Loss of a declared actor preserves own accepted intent/history, but
        // cannot invent a terminal settlement. Present actors must still carry
        // their exact active job; absent actors use immutable provenance below.
        if let (Some(scope), Some(row)) = (instance.accepted.as_ref().filter(|_| !terminal), row) {
            match &instance.request {
                ActivityRequest::Repair { team, .. } => {
                    let job = row
                        .repair
                        .as_ref()
                        .and_then(|repair| repair.jobs.as_ref())
                        .and_then(|jobs| jobs.teams.get(*team as usize))
                        .and_then(Option::as_ref)
                        .ok_or("unsettled activity lost its accepted Repair job")?;
                    if job.id != scope.job || job.owner != scope.owner || job.outcome.is_some() {
                        return Err("unsettled activity disagrees with saved Repair job".into());
                    }
                    for fact in instance
                        .achieved
                        .iter()
                        .chain(pending.iter().filter(|fact| fact.instance == *id))
                    {
                        if let ActivityOutcome::RepairAchieved { stage, .. } = fact.outcome {
                            let physical = row
                                .repair
                                .as_ref()
                                .and_then(|repair| repair.jobs.as_ref())
                                .and_then(|jobs| jobs.physical_work.get(&scope.job.0));
                            if let Some(physical) = physical {
                                if stage >= physical.stage {
                                    return Err(
                                        "repair evidence exceeds actual committed stage frontier"
                                            .into(),
                                    );
                                }
                            } else if !matches!(
                                &job.activity,
                                crate::jobs::repair::RepairActivity::Internal {
                                    phase: crate::core::messages::TeamSlot::Returning { .. },
                                    ..
                                }
                            ) {
                                return Err(
                                    "active achieved repair lacks its physical stage frontier"
                                        .into(),
                                );
                            }
                        }
                    }
                }
                ActivityRequest::Delivery {
                    source,
                    destination,
                    amount,
                    ..
                } => {
                    let batch = row
                        .logistics
                        .as_ref()
                        .and_then(|logistics| logistics.batches.get(&scope.job))
                        .ok_or("unsettled activity lost its accepted delivery batch")?;
                    if &batch.source != source
                        || &batch.original_destination != destination
                        || batch.amount != *amount
                    {
                        return Err("activity delivery disagrees with actual batch binding".into());
                    }
                }
            }
        }
        // Immutable equipment validation also covers retained history. Prefer
        // bootstrap equipment; explicit origins must resolve to that same recipe.
        let accepted = slot_source
            .as_ref()
            .map(|(_, config)| config.clone())
            .or_else(|| {
                let authored_entity = world
                    .get_resource::<crate::world::config::WorldConfig>()?
                    .entities
                    .iter()
                    .find(|entity| entity.name.as_deref() == Some(authored.request.actor()))?;
                crate::entities::loader::resolve_entity_via(
                    authored_entity,
                    &crate::entities::config_cache::get_config_cache(),
                    &crate::entities::loader::WasmTemplateLoader,
                )
                .ok()
            })
            .ok_or("cannot resolve accepted immutable activity equipment")?;
        let config = if let Some(origin) = row.and_then(|row| row.spawn.as_ref()) {
            let mut warnings = Vec::new();
            let candidate = origin
                .resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings)
                .ok_or("cannot resolve explicit candidate activity origin")?;
            if !warnings.is_empty() {
                return Err("candidate activity origin did not resolve exactly".into());
            }
            {
                // First-consumer actors are fixed base/slot recipes. Compare
                // immutable machinery only; presentation name/placement is not
                // an extra capability, and current damage is never consulted.
                let recipe = |config: &crate::entities::config::EntityConfig| {
                    serde_json::to_value((
                        &config.ship_config,
                        &config.interior,
                        &config.damage,
                        &config.repair,
                        &config.logistics,
                        &config.energy,
                    ))
                };
                if recipe(&candidate).map_err(|error| error.to_string())?
                    != recipe(&accepted).map_err(|error| error.to_string())?
                {
                    return Err(
                        "candidate activity origin replaces the accepted immutable equipment"
                            .into(),
                    );
                }
            }
            candidate
        } else {
            accepted
        };
        match &instance.request {
            ActivityRequest::Repair { target, .. } => {
                let ship = config
                    .ship_config
                    .as_ref()
                    .ok_or("cannot resolve activity Repair equipment")?;
                let damage = crate::ship::compartment_damage::ShipDamage::from_config(&config)?
                    .ok_or("cannot resolve activity Repair definition")?;
                let repair = config
                    .repair
                    .as_ref()
                    .ok_or("cannot resolve activity Repair team definition")?;
                let team_owner = crate::ship::system_registry::repair_system_id();
                let count = repair.repair_team_count as usize;
                if !crate::world::activities::repair_binding_supported(
                    &instance.request,
                    ship,
                    &damage,
                    &team_owner,
                    count,
                ) {
                    return Err("activity names unsupported Repair equipment or goal scope".into());
                }
                let valid = match target {
                    phoenix_model::damage::RepairWorkTarget::Compartment { compartment } => {
                        damage.definition.compartment(compartment).is_some()
                    }
                    phoenix_model::damage::RepairWorkTarget::Component { system, component } => {
                        damage
                            .definition
                            .component(&phoenix_model::damage::ComponentRef {
                                system: system.clone(),
                                component: component.clone(),
                            })
                            .is_some()
                    }
                };
                if !valid {
                    return Err("activity names unsupported physical Repair target".into());
                }
                // Historical facts are validated against supported immutable
                // remedies, not the actor's subsequently damaged condition.
                for fact in instance
                    .achieved
                    .iter()
                    .chain(pending.iter().filter(|fact| fact.instance == *id))
                {
                    if let ActivityOutcome::RepairAchieved { remedy, result, .. } = &fact.outcome {
                        let supported=match target {
                            phoenix_model::damage::RepairWorkTarget::Compartment {compartment}=>damage.definition.compartment(compartment).is_some_and(|room|&room.structural_remedy==remedy && matches!(result,phoenix_model::activity::RepairAchievement::Structure {restored_hp} if *restored_hp<=f64::from(room.max_hp))),
                            phoenix_model::damage::RepairWorkTarget::Component {system,component}=>damage.definition.component(&phoenix_model::damage::ComponentRef {system:system.clone(),component:component.clone()}).is_some_and(|component| match result {
                                phoenix_model::activity::RepairAchievement::Component {after:phoenix_model::damage::ComponentCondition::Healthy,..}=>&component.full_remedy==remedy,
                                phoenix_model::activity::RepairAchievement::Component {after:phoenix_model::damage::ComponentCondition::Patched {factor},..}=>component.patch_remedy.as_ref()==Some(remedy)&&damage.definition.config.remedies.get(remedy).is_some_and(|recipe|matches!(recipe.outcome,phoenix_model::damage::RemedyOutcome::Patch {factor:actual} if actual==*factor)),
                                _=>false,
                            }),
                        };
                        if !supported {
                            return Err(
                                "activity repair result disagrees with supported immutable remedy"
                                    .into(),
                            );
                        }
                    }
                }
            }
            ActivityRequest::Delivery { .. } => {
                let profile =
                    crate::resources::logistics_profile::compile_entity_logistics(&config)?
                        .ok_or("cannot resolve activity delivery equipment")?;
                if !crate::world::activities::delivery_binding_supported(
                    &instance.request,
                    &profile,
                ) {
                    return Err(
                        "activity delivery disagrees with actual immutable endpoints".into(),
                    );
                }
            }
        }
    }
    if stored.mission_elapsed.is_some() {
        for config in &runtime.situation_configs {
            if stored.situations.instances.contains_key(&config.id) {
                continue;
            }
            if let Some(base) = world.get_resource::<crate::world::config::WorldConfig>() {
                base.validate_situation_actor(config.request.actor())?;
                if let Some(slot) = config.request.actor().strip_prefix("slot:") {
                    if slot_binding(world, snapshot, slot)?.is_some() {
                        return Err(
                            "resolved launch-slot activity lost its saved arming frontier".into(),
                        );
                    }
                }
                let known_initial = base
                    .entities
                    .iter()
                    .enumerate()
                    .find(|(_, entity)| entity.name.as_deref() == Some(config.request.actor()))
                    .is_some_and(|(index, entity)| {
                        (entity.spawn_on == crate::world::config::WorldEntitySpawnOn::Immediate
                            && entity.when.is_none()
                            && entity.when_predicate.is_none())
                            || snapshot.boot_identity.as_ref().is_some_and(|boot| {
                                boot.game_start_entity_uuids
                                    .iter()
                                    .any(|row| row.authored_index as usize == index)
                            })
                    });
                if known_initial {
                    return Err(
                        "initially resolved activity actor lost its saved arming frontier".into(),
                    );
                }
            }
            // Future/unspawned actors owe no instance yet. A currently present,
            // supported actor has passed the ordinary consumers' arming seam.
            for row in &snapshot.entities {
                let live = world
                    .try_query::<(Entity, &EntityUuid)>()
                    .and_then(|mut query| {
                        query
                            .iter(world)
                            .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
                    });
                let named = world
                    .get_resource::<crate::world::config::WorldConfig>()
                    .and_then(|world| world.name_to_uuid.get(config.request.actor()))
                    .or_else(|| runtime.name_to_uuid.get(config.request.actor()));
                let matched = config.request.actor() == row.uuid
                    || named == Some(&row.uuid)
                    || config
                        .request
                        .actor()
                        .strip_prefix("slot:")
                        .is_some_and(|id| {
                            live.and_then(|entity| {
                                world.get::<crate::ship_slots::AuthoredShipSlotId>(entity)
                            })
                            .is_some_and(|slot| slot.0 == id)
                        });
                if !matched {
                    continue;
                }
                let supported = match &config.request {
                    ActivityRequest::Repair { .. } => live
                        .and_then(|entity| {
                            Some((
                                world.get::<crate::ship_plugin::ShipConfigComponent>(entity)?,
                                world.get::<crate::ship::compartment_damage::ShipDamage>(entity)?,
                                world.get::<crate::console::repair::server::ShipRepairTeams>(
                                    entity,
                                )?,
                            ))
                        })
                        .is_some_and(|(ship, damage, teams)| {
                            crate::world::activities::repair_binding_supported(
                                &config.request,
                                &ship.0,
                                damage,
                                &teams.0.job_state().owner,
                                teams.0.job_state().teams.len(),
                            )
                        }),
                    ActivityRequest::Delivery { .. } => live
                        .and_then(|entity| {
                            world.get::<crate::resources::logistics_server::CompiledShipLogistics>(
                                entity,
                            )
                        })
                        .is_some_and(|profile| {
                            crate::world::activities::delivery_binding_supported(
                                &config.request,
                                &profile.0,
                            )
                        }),
                };
                if supported {
                    return Err("resolved supported situation lost its saved armed instance".into());
                }
            }
        }
    }
    Ok(())
}

/// Cached mission instructions may predate acceptance by one publication step,
/// but can never invent another actor, command, method, or original release tick.
fn validate_plan_publications(
    snapshot: &PhoenixSnapshot,
    choice: &phoenix_sim_world::world::planning::PlanChoice,
    graph: &phoenix_sim_world::world::planning::Decomposition,
) -> Result<(), String> {
    use crate::core::messages::SystemBlackboard;
    let released = choice
        .released
        .iter()
        .map(|(node, tick)| {
            (
                graph.leaves[node].id.clone(),
                (&graph.leaves[node].request, *tick),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for row in &snapshot.entities {
        for (owner, board) in &row.blackboards {
            let mut ids = std::collections::BTreeSet::new();
            match board {
                SystemBlackboard::Repair(board) => {
                    if board.planned_requests.len() > phoenix_sim_world::world::planning::MAX_LEAVES
                    {
                        return Err("planned Repair publication exceeds bound".into());
                    }
                    for request in &board.planned_requests {
                        let Some((
                            ActivityRequest::Repair {
                                owner: expected,
                                team,
                                request: command,
                                target,
                                goal,
                                ..
                            },
                            tick,
                        )) = released.get(&request.id).copied()
                        else {
                            return Err("Repair publication has no released primitive".into());
                        };
                        if row.uuid != choice.actor_uuid
                            || expected.0.as_str() != owner.as_str()
                            || !ids.insert(&request.id)
                            || request.released_tick != tick
                            || u32::from(request.team) != *team
                            || &request.request != command
                            || &request.target != target
                            || request.goal != *goal
                        {
                            return Err(
                                "Repair publication disagrees with released mission instruction"
                                    .into(),
                            );
                        }
                    }
                }
                SystemBlackboard::Logistics(board) => {
                    if board.planned_requests.len() > phoenix_sim_world::world::planning::MAX_LEAVES
                    {
                        return Err("planned delivery publication exceeds bound".into());
                    }
                    for request in &board.planned_requests {
                        let Some((
                            ActivityRequest::Delivery {
                                owner: expected,
                                source,
                                destination,
                                commodity,
                                amount,
                                ..
                            },
                            tick,
                        )) = released.get(&request.id).copied()
                        else {
                            return Err("delivery publication has no released primitive".into());
                        };
                        if row.uuid != choice.actor_uuid
                            || expected.0.as_str() != owner.as_str()
                            || !ids.insert(&request.id)
                            || request.released_tick != tick
                            || &request.source != source
                            || &request.destination != destination
                            || &request.commodity != commodity
                            || request.amount != *amount
                        {
                            return Err(
                                "delivery publication disagrees with released mission instruction"
                                    .into(),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Single-row application leaves the World owner untouched. It therefore may
/// alter a bound actor's body, but cannot rewind its physical job frontiers.
/// Full-roster restore uses its candidate World and apply_entity_state_with.
pub(crate) fn validate_single_row(world: &World, row: &EntityState) -> Result<(), String> {
    let Some(runtime) = world.get_resource::<WorldContentRuntime>() else {
        return Ok(());
    };
    let bound = runtime
        .situations
        .instances
        .values()
        .filter(|instance| instance.request.actor() == row.uuid && instance.accepted.is_some())
        .collect::<Vec<_>>();
    if bound.is_empty() {
        return Ok(());
    }
    let entity = world
        .try_query::<(Entity, &EntityUuid)>()
        .and_then(|mut query| {
            query
                .iter(world)
                .find_map(|(entity, uuid)| (uuid.0 == row.uuid).then_some(entity))
        })
        .ok_or("body-only inverse of accepted World activity requires full-roster restore")?;
    if bound
        .iter()
        .any(|instance| matches!(instance.request, ActivityRequest::Repair { .. }))
    {
        let actual = world
            .get::<crate::console::repair::server::ShipRepairTeams>(entity)
            .ok_or("bound activity lacks current Repair owner")?
            .0
            .save_state();
        if row.repair.as_ref().and_then(|repair| repair.jobs.as_ref()) != Some(&actual) {
            return Err(
                "single-row restore would rewind Repair beneath accepted World activity".into(),
            );
        }
    }
    if bound
        .iter()
        .any(|instance| matches!(instance.request, ActivityRequest::Delivery { .. }))
    {
        let actual = &world
            .get::<crate::resources::logistics_server::ShipLogistics>(entity)
            .ok_or("bound activity lacks current Logistics owner")?
            .0;
        if row.logistics.as_ref() != Some(actual) {
            return Err(
                "single-row restore would rewind Logistics beneath accepted World activity".into(),
            );
        }
    }
    Ok(())
}
