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
    let hull = crate::world::dispatch::TemplateLoader::load_template(
        &crate::entities::loader::WasmTemplateLoader,
        &frozen.hull,
    )
    .ok_or("cannot resolve activity frozen hull")?;
    Ok(Some((
        row.entity_uuid.clone(),
        crate::server_app::player_hull_config(source, authored.overrides.as_ref(), Some(&hull)),
    )))
}

pub(super) fn validate(world: &World, snapshot: &PhoenixSnapshot) -> Result<(), String> {
    let Some(stored) = snapshot.scenario.as_ref() else {
        if world
            .get_resource::<WorldContentRuntime>()
            .is_some_and(|runtime| !runtime.situation_configs.is_empty())
        {
            return Err("configured situations require their saved World owner".into());
        }
        return Ok(());
    };
    let Some(runtime) = world.get_resource::<WorldContentRuntime>() else {
        return if stored.situations.instances.is_empty()
            && !stored.pending_events.iter().any(|event| event.kind == 13)
        {
            Ok(())
        } else {
            Err("activity evidence requires its World owner".into())
        };
    };
    stored
        .situations
        .validate(&runtime.situation_definitions, snapshot.tick)?;
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
        let authored = runtime
            .situation_configs
            .iter()
            .find(|config| &config.id == id)
            .ok_or("unknown authored situation instance")?;
        world
            .get_resource::<crate::world::config::WorldConfig>()
            .ok_or("activity history requires its accepted base-world definition")?
            .validate_situation_actor(authored.request.actor())?;
        let actor = instance.request.actor();
        let live = world
            .try_query::<(Entity, &EntityUuid)>()
            .and_then(|mut query| {
                query
                    .iter(world)
                    .find_map(|(entity, uuid)| (uuid.0 == actor).then_some(entity))
            });
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
                                    phase: crate::modifiers::repair_teams::TeamSlot::Returning { .. },
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
        // bootstrap equipment; source origins cover captured runtime spawns.
        let config = row
            .and_then(|row| row.spawn.as_ref())
            .and_then(|origin| {
                origin.resolve(
                    &crate::entities::loader::WasmTemplateLoader,
                    &mut Vec::new(),
                )
            })
            .or_else(|| slot_source.as_ref().map(|(_, config)| config.clone()))
            .or_else(|| {
                // Terminal evidence survives actor removal. Reuse the SAME accepted
                // base-world template+overrides resolver, never a live damage row.
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
            });
        match &instance.request {
            ActivityRequest::Repair { target, .. } => {
                let ship = live
                    .and_then(|entity| world.get::<crate::ship_plugin::ShipConfigComponent>(entity))
                    .map(|config| config.0.clone())
                    .or_else(|| {
                        config
                            .as_ref()
                            .and_then(|config| config.ship_config.clone())
                    })
                    .ok_or("cannot resolve activity Repair equipment")?;
                let damage = live
                    .and_then(|entity| {
                        world.get::<crate::ship::compartment_damage::ShipDamage>(entity)
                    })
                    .cloned()
                    .or_else(|| {
                        config.as_ref().and_then(|config| {
                            crate::ship::compartment_damage::ShipDamage::from_config(config)
                                .ok()
                                .flatten()
                        })
                    })
                    .ok_or("cannot resolve activity Repair definition")?;
                let (team_owner, count) = live
                    .and_then(|entity| {
                        world.get::<crate::console::repair::server::ShipRepairTeams>(entity)
                    })
                    .map(|teams| {
                        (
                            teams.0.job_state().owner.clone(),
                            teams.0.job_state().teams.len(),
                        )
                    })
                    .or_else(|| {
                        config.as_ref().and_then(|config| {
                            config.repair.as_ref().map(|repair| {
                                (
                                    crate::ship::system_registry::repair_system_id(),
                                    repair.repair_team_count as usize,
                                )
                            })
                        })
                    })
                    .ok_or("cannot resolve activity Repair team definition")?;
                if !crate::world::activities::repair_binding_supported(
                    &instance.request,
                    &ship,
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
                let profile = live
                    .and_then(|entity| {
                        world.get::<crate::resources::logistics_server::CompiledShipLogistics>(
                            entity,
                        )
                    })
                    .map(|profile| profile.0.clone())
                    .or_else(|| {
                        config.as_ref().and_then(|config| {
                            crate::resources::logistics_profile::compile_entity_logistics(config)
                                .ok()
                                .flatten()
                        })
                    })
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
