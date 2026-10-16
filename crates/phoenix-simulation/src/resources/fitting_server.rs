//! Physical fitting composes the existing team, parts, damage and machinery owners.
//! A desired definition is a request; only a completed, paid stage moves an item.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::console::repair::server::ShipRepairTeams;
use crate::core::messages::{AdmittedCommands, SystemControlPayload as Control, SystemId};
use crate::entities::spawner::EntityUuid;
use crate::infrastructure::InfrastructureCondition;
use crate::resources::fitting::{CompiledFitting, EquipmentLocation, FittingState, RefitClass};
use crate::resources::server::ResourceServices;
use crate::ship::compartment_damage::ShipDamage;
use crate::ship::power::ShipPowerSystem;

#[derive(Component, Clone, Debug)]
pub struct CompiledShipFitting(pub CompiledFitting);
#[derive(Component, Clone, Debug)]
pub struct CompiledRefitFacility(pub phoenix_model::fitting::FittingFacilityConfig);
#[derive(Component, Clone, Debug)]
pub struct FittingPlannerCatalog(pub phoenix_model::system_status::FittingPlannerContext);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefitStage {
    Fetching,
    Delivering,
    Returning,
    Preparing,
    Removing,
    Installing,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefitWork {
    pub request: u64,
    pub slot: String,
    pub item: String,
    pub removed_item: Option<String>,
    pub locker: String,
    pub source_locker: String,
    pub job: phoenix_model::job::JobId,
    pub stage: RefitStage,
    pub progress: f32,
    pub reserved: bool,
    pub waiting: String,
}
impl RefitWork {
    pub fn claim(&self) -> String {
        format!("refit/{}", self.request)
    }
}

#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShipFitting {
    pub equipment: FittingState,
    pub requested: BTreeSet<String>,
    pub work: BTreeMap<String, RefitWork>,
    pub next_request: u64,
    pub last_tick: Option<u64>,
}

pub(crate) fn install(
    config: &crate::entities::config::EntityConfig,
    uuid: &str,
    cmds: &mut EntityCommands,
) {
    if let Some(facility) = &config.refit_facility {
        cmds.insert(CompiledRefitFacility(facility.clone()));
    }
    if let Some(profile) = CompiledFitting::compile(config).expect("validated fitting") {
        let catalog = planner_context(config).expect("fitted catalog");
        let mut equipment = FittingState::from_profile(&profile).expect("validated assemblies");
        equipment.assemblies = equipment
            .assemblies
            .into_values()
            .map(|mut item| {
                item.id = format!("eq-{uuid}-{}", item.id);
                (item.id.clone(), item)
            })
            .collect();
        cmds.insert((
            CompiledShipFitting(profile),
            FittingPlannerCatalog(catalog),
            ShipFitting {
                equipment,
                requested: BTreeSet::new(),
                work: BTreeMap::new(),
                next_request: 1,
                last_tick: None,
            },
        ));
    }
}

pub(crate) fn planner_context(
    config: &crate::entities::config::EntityConfig,
) -> Option<phoenix_model::system_status::FittingPlannerContext> {
    let profile = CompiledFitting::compile(config).ok()??;
    // Accepted EntityConfig contains HashMaps. Its ordinary serde projection
    // also skips parsed ShipConfig and shield arcs, so bind those explicitly.
    // Sort object keys recursively, preserving all authored array order, so
    // independent native/captured loads bind the same complete accepted hull.
    let mut canonical = serde_json::to_value((config, &config.ship_config, &config.shield_arcs))
        .expect("accepted hull serializes");
    canonical.sort_all_objects();
    Some(phoenix_model::system_status::FittingPlannerContext {
        hull_path: String::new(),
        content_id: format!(
            "{:016x}",
            vellum_digest::fnv1a(
                &serde_json::to_vec(&canonical).expect("accepted hull serializes")
            )
        ),
        slots: profile
            .config()
            .slots
            .iter()
            .map(|slot| phoenix_model::system_status::FittingPlannerSlot {
                slot: slot.id.clone(),
                system: slot.system.clone(),
                home_station: config
                    .ship_config
                    .as_ref()
                    .and_then(|ship| {
                        ship.system(&slot.system)
                            .and_then(|system| ship.system_home_station(system))
                    })
                    .map(|station| station.0.clone())
                    .unwrap_or_default(),
                default_equipment: slot.default_equipment.clone(),
                compatible_equipment: profile
                    .config()
                    .equipment
                    .iter()
                    .filter(|equipment| profile.compatible(&slot.id, &equipment.id).is_ok())
                    .map(|equipment| equipment.id.clone())
                    .collect(),
            })
            .collect(),
    })
}

pub(crate) fn register(app: &mut App) {
    use crate::authoritative::{DeclareState, StateClass};
    use crate::command_admission::{ConsumerMatcher, FeedbackAddress, RegisterAdmittedConsumer};
    use crate::core::messages::SystemControlPayloadDiscriminants as Payload;
    use crate::ship::system_registry::{PHASER_BANK_KIND, POWER_REACTOR_KIND, REPAIR_KIND};
    app.declare_state::<CompiledShipFitting>(
        StateClass::Derived,
        "equipment-fitting-and-refitting",
    )
    .declare_state::<ShipFitting>(StateClass::Folded, "equipment-fitting-and-refitting");
    app.declare_state::<CompiledRefitFacility>(
        StateClass::Derived,
        "equipment-fitting-and-refitting",
    );
    app.declare_state::<FittingPlannerCatalog>(
        StateClass::Derived,
        "equipment-fitting-and-refitting",
    );
    for kind in [PHASER_BANK_KIND, POWER_REACTOR_KIND] {
        app.register_admitted_consumer(
            ConsumerMatcher::kind_payload(kind, Payload::RequestEquipment).with_feedback(
                FeedbackAddress::DeclaredKindOnly,
                &[Payload::RequestEquipment],
            ),
        );
    }
    for (payload, feedback) in [
        (
            Payload::DispatchRefit,
            &[Payload::DispatchRefit] as &'static [Payload],
        ),
        (
            Payload::CancelRefit,
            &[Payload::CancelRefit] as &'static [Payload],
        ),
    ] {
        app.register_admitted_consumer(
            ConsumerMatcher::kind_payload(REPAIR_KIND, payload)
                .with_feedback(FeedbackAddress::DeclaredKindOnly, feedback),
        );
    }
    app.add_systems(
        FixedUpdate,
        handle_requests
            .in_set(crate::sim_sets::SimSet::Physics)
            .after(crate::console::repair::server::operate_repair_ai)
            .before(crate::console::repair::server::tick_repair_teams)
            .before(crate::console::repair::dispatch::handle_dispatch_repair_team)
            .after(crate::command_admission::AdmissionSet),
    );
    app.add_systems(
        FixedUpdate,
        // Commit hardware only after this interval's beams have expired. The
        // damage phase follows Physics, so fitting must not add a backwards
        // dependency from a Physics system to the beam lifetime owner.
        tick.in_set(crate::sim_sets::SimSet::Modifiers)
            .after(crate::console::repair::server::tick_repair_teams)
            .after(crate::console::weapons::beam::tick_beams_tick_lifetimes)
            .before(crate::infrastructure::server::tick_infrastructure_condition),
    );
}

fn installed<'a>(
    state: &'a FittingState,
    slot: &str,
) -> Option<&'a crate::resources::fitting::EquipmentAssembly> {
    state.assemblies.values().find(|item| matches!(&item.location, EquipmentLocation::Installed { slot: current } if current == slot))
}

/// Temporary vacancy and safe removal gate new starts; an already burning beam
/// completes through its ordinary lifecycle before removal commits.
pub fn unavailable(profile: &CompiledShipFitting, state: &ShipFitting, system: &SystemId) -> bool {
    profile.0.config().slots.iter().any(|slot| {
        &slot.system == system
            && (installed(&state.equipment, &slot.id).is_none()
                || state.work.contains_key(&slot.id))
    })
}

fn can_report(
    config: &crate::ship::config::ShipConfig,
    recipient: &SystemId,
    source: &SystemId,
) -> bool {
    use phoenix_model::console::ConsoleObservationKind as Kind;
    let Some(recipient) = config.system(recipient) else {
        return false;
    };
    let Some(source) = config.system(source) else {
        return false;
    };
    if recipient.console == source.console && recipient.console.is_some() {
        return true;
    }
    let Some(console) = config
        .consoles
        .iter()
        .find(|console| Some(&console.id) == recipient.console.as_ref())
    else {
        return false;
    };
    let kind = match source.kind.as_str() {
        crate::ship::system_registry::PHASER_BANK_KIND => Kind::PhaserBank,
        crate::ship::system_registry::POWER_REACTOR_KIND => Kind::PowerReactor,
        _ => return false,
    };
    console
        .feeds
        .iter()
        .any(|feed| feed.source == source.id && feed.kind == kind)
}

fn handle_requests(world: &mut World) {
    let mut query = world.query::<(Entity, &EntityUuid, &AdmittedCommands, &CompiledShipFitting)>();
    let mut commands: Vec<_> = query
        .iter(world)
        .flat_map(|(entity, uuid, admitted, _)| {
            admitted
                .0
                .iter()
                .filter(|cmd| {
                    matches!(
                        cmd.payload,
                        Control::RequestEquipment { .. }
                            | Control::DispatchRefit { .. }
                            | Control::CancelRefit { .. }
                    )
                })
                .map(move |cmd| (uuid.0.clone(), entity, cmd.clone()))
        })
        .collect();
    commands.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, entity, cmd) in commands {
        let result = match &cmd.payload {
            Control::RequestEquipment { slot, equipment } => {
                request(world, entity, &cmd.target, slot, equipment)
            }
            Control::DispatchRefit { team_idx, slot } => {
                dispatch(world, entity, &cmd.target, *team_idx as usize, slot)
            }
            Control::CancelRefit { slot } => cancel(world, entity, &cmd.target, slot),
            _ => unreachable!(),
        };
        let feedback = crate::command_admission::feedback::admitted_action_feedback(
            &cmd,
            if result.is_ok() {
                crate::core::messages::ActionFeedbackOutcome::Applied
            } else {
                crate::core::messages::ActionFeedbackOutcome::Refused
            },
        );
        if let (Some(feedback), Some(mut outbound)) = (
            feedback,
            world.get_resource_mut::<Messages<crate::lobby::OutboundMessage>>(),
        ) {
            outbound.write(feedback);
        }
    }
}

fn request(
    world: &mut World,
    entity: Entity,
    target: &SystemId,
    slot: &str,
    equipment: &str,
) -> Result<(), String> {
    let profile = world
        .get::<CompiledShipFitting>(entity)
        .ok_or("no fitting profile")?;
    if &profile.0.slot(slot)?.system != target {
        return Err("request must target the slot's own System".into());
    }
    let mut state = world
        .get::<ShipFitting>(entity)
        .ok_or("no equipment owner")?
        .clone();
    if state.work.contains_key(slot) {
        return Err("cancel existing work before changing its request".into());
    }
    state.equipment.request(&profile.0, slot, equipment)?;
    state.requested.insert(slot.to_owned());
    world.entity_mut(entity).insert(state);
    Ok(())
}

fn repair_owner(
    world: &World,
    entity: Entity,
    target: &SystemId,
    slot: &str,
) -> Result<(), String> {
    let config = &world
        .get::<crate::ship_plugin::ShipConfigComponent>(entity)
        .ok_or("no ship topology")?
        .0;
    if !config
        .system(target)
        .is_some_and(|system| system.kind == crate::ship::system_registry::REPAIR_KIND)
    {
        return Err("only Repair assigns teams".into());
    }
    let profile = &world
        .get::<CompiledShipFitting>(entity)
        .ok_or("no fitting profile")?
        .0;
    if !can_report(config, target, &profile.slot(slot)?.system) {
        return Err("Repair has no permitted fitting report".into());
    }
    Ok(())
}

fn dispatch(
    world: &mut World,
    entity: Entity,
    target: &SystemId,
    team: usize,
    slot: &str,
) -> Result<(), String> {
    repair_owner(world, entity, target, slot)?;
    let profile = world
        .get::<CompiledShipFitting>(entity)
        .ok_or("no fitting profile")?
        .0
        .clone();
    let mut state = world
        .get::<ShipFitting>(entity)
        .ok_or("no equipment owner")?
        .clone();
    if state.next_request == u64::MAX || state.work.contains_key(slot) {
        return Err("slot already claimed or request identity exhausted".into());
    }
    if !state.requested.contains(slot) {
        return Err("no outstanding fitting request".into());
    }
    let definition = state
        .equipment
        .desired
        .get(slot)
        .ok_or("no desired equipment")?
        .clone();
    profile.compatible(slot, &definition)?;
    let room = profile.compartment(slot)?.to_owned();
    if world.get::<EntityUuid>(entity).is_none_or(|uuid| {
        crate::resources::equipment_recovery_server::source_claimed(
            world,
            &uuid.0,
            &profile.slot(slot).expect("compiled slot").system,
        )
    }) {
        return Err("the actual equipment is committed to recovery".into());
    }
    if foreign_repair_claim(world, entity, &profile.slot(slot)?.system) {
        return Err("another ship's repair team owns this physical task".into());
    }
    let item = state
        .equipment
        .assemblies
        .values()
        .filter(|item| {
            item.definition == definition
                && !state.work.values().any(|work| work.item == item.id)
                && matches!(&item.location, EquipmentLocation::Stored { .. })
        })
        .min_by_key(|item| {
            let EquipmentLocation::Stored { locker } = &item.location else {
                unreachable!()
            };
            (
                profile
                    .locker(locker)
                    .is_ok_and(|locker| locker.compartment != room),
                item.id.clone(),
            )
        })
        .ok_or("no actual compatible stored spare")?
        .id
        .clone();
    let EquipmentLocation::Stored {
        locker: source_locker,
    } = &state.equipment.assemblies[&item].location
    else {
        unreachable!()
    };
    let source_locker = source_locker.clone();
    if profile.equipment(&definition)?.work.class == RefitClass::Facility
        && !facility_available(world, entity)
    {
        return Err("a suitable coupled facility is required".into());
    }
    let old = installed(&state.equipment, slot).map(|item| item.id.clone());
    let mut teams = world
        .get::<ShipRepairTeams>(entity)
        .ok_or("no repair teams")?
        .clone();
    let request = state.next_request;
    let job = teams
        .0
        .claim_refit(
            team,
            request,
            profile.slot(slot)?.system.clone(),
            slot.to_owned(),
        )
        .ok_or("team or physical task is already committed")?;
    if !teams
        .0
        .visit_refit_compartment(job, &profile.locker(&source_locker)?.compartment)
    {
        return Err("team cannot reach the actual spare locker".into());
    }
    let locker = receiving_locker(
        &profile,
        &state,
        &room,
        old.as_deref(),
        world.get::<crate::resources::equipment_recovery_server::ShipEquipmentRecovery>(entity),
    )?;
    state.work.insert(
        slot.to_owned(),
        RefitWork {
            request,
            slot: slot.to_owned(),
            item,
            removed_item: old,
            locker,
            source_locker,
            job,
            stage: RefitStage::Fetching,
            progress: 0.0,
            reserved: false,
            waiting: String::new(),
        },
    );
    state.next_request += 1;
    world.entity_mut(entity).insert((state, teams));
    Ok(())
}

/// Space held by actual fitting jobs, in addition to stored assemblies.
/// Recovery must include these claims when offering the same finite lockers.
pub fn reserved_space(profile: &CompiledFitting, state: &ShipFitting, locker: &str) -> u64 {
    state.work.values().fold(0_u64, |total, work| {
        let mut claimed = 0_u64;
        if work.locker == locker
            && !matches!(work.stage, RefitStage::Installing | RefitStage::Returning)
        {
            if let Some(old) = work
                .removed_item
                .as_ref()
                .and_then(|id| state.equipment.assemblies.get(id))
            {
                claimed = claimed.saturating_add(
                    profile
                        .equipment(&old.definition)
                        .map_or(u64::MAX, |definition| u64::from(definition.storage_space)),
                );
            }
        }
        if work.source_locker == locker {
            if let Some(item) = state
                .equipment
                .assemblies
                .get(&work.item)
                .filter(|item| matches!(item.location, EquipmentLocation::InWork { .. }))
            {
                claimed = claimed.saturating_add(
                    profile
                        .equipment(&item.definition)
                        .map_or(u64::MAX, |definition| u64::from(definition.storage_space)),
                );
            }
        }
        total.saturating_add(claimed)
    })
}

fn receiving_locker(
    profile: &CompiledFitting,
    state: &ShipFitting,
    room: &str,
    old: Option<&str>,
    recovery: Option<&crate::resources::equipment_recovery_server::ShipEquipmentRecovery>,
) -> Result<String, String> {
    let required = old
        .map(|id| {
            profile
                .equipment(&state.equipment.assemblies[id].definition)
                .map(|item| item.storage_space)
        })
        .transpose()?
        .unwrap_or(0);
    for locker in profile
        .config()
        .lockers
        .iter()
        .filter(|locker| locker.compartment == room)
    {
        let used: u64 = state.equipment.assemblies.values().filter(|item| matches!(&item.location, EquipmentLocation::Stored { locker: current } if current == &locker.id))
            .map(|item| profile.equipment(&item.definition).map_or(u64::MAX, |definition| definition.storage_space as u64)).fold(0, u64::saturating_add);
        let claimed = reserved_space(profile, state, &locker.id);
        let incoming = recovery.map_or(0, |owner| {
            crate::resources::equipment_recovery_server::reserved_space(owner, &locker.id, profile)
        });
        if used
            .saturating_add(claimed)
            .saturating_add(required as u64)
            .saturating_add(incoming)
            <= locker.space as u64
        {
            return Ok(locker.id.clone());
        }
    }
    Err("no finite local receiving space for the removed assembly".into())
}

/// Facility support is an explicit authored service and actual physical Dock
/// coupling; proximity, a name containing 'starbase', or a UI request grants none.
fn facility_available(world: &World, entity: Entity) -> bool {
    facility_provider(world, entity).is_some()
}
fn facility_provider(world: &World, entity: Entity) -> Option<Entity> {
    let Some(partner) = world
        .get::<crate::dock::DockControl>(entity)
        .and_then(|dock| dock.docked_partner())
    else {
        return None;
    };
    world
        .try_query::<(
            Entity,
            &EntityUuid,
            &CompiledRefitFacility,
            &crate::resources::ports_server::CompiledPortService,
        )>()
        .and_then(|mut query| {
            query
                .iter(world)
                .find_map(|(provider, uuid, facility, ports)| {
                    if provider == entity
                        || uuid.0 != partner
                        || !crate::resources::ports_server::eligible(
                            world, provider, entity, &ports.0,
                        )
                    {
                        return None;
                    }
                    let available = world
                        .get::<ShipDamage>(provider)
                        .is_some_and(|damage| damage.operational(&facility.0.system))
                        && world
                            .get::<crate::ship_plugin::ShipSystemControlSources>(provider)
                            .is_some_and(|sources| !sources.0.is_offline(&facility.0.system))
                        && world.get::<ShipPowerSystem>(provider).is_some();
                    available.then_some(provider)
                })
        })
}

pub(crate) fn foreign_repair_claim(world: &World, entity: Entity, target: &SystemId) -> bool {
    let Some(uuid) = world.get::<EntityUuid>(entity) else {
        return true;
    };
    world.try_query::<(&EntityUuid, &ShipRepairTeams)>().is_some_and(|mut query|
        query.iter(world).filter(|(source,_)| source.0 != uuid.0).any(|(_,teams)| {
            let jobs = teams.0.save_state();
            jobs.external_physical_work.iter().any(|(id,work)| work.target_uuid == uuid.0
                && matches!(&work.target, Some(phoenix_model::damage::RepairWorkTarget::Component { system, .. }) if system == target)
                && jobs.teams.iter().flatten().any(|job| job.id.0 == *id && job.outcome.is_none()))
        }))
}

fn cancel(world: &mut World, entity: Entity, target: &SystemId, slot: &str) -> Result<(), String> {
    repair_owner(world, entity, target, slot)?;
    let mut state = world
        .get::<ShipFitting>(entity)
        .ok_or("no equipment owner")?
        .clone();
    if let Some(work) = state.work.get_mut(slot) {
        work.waiting = "cancelled".into();
    } else if !state.requested.remove(slot) {
        return Err("no fitting work or request".into());
    }
    world.entity_mut(entity).insert(state);
    Ok(())
}

fn recipe(
    profile: &CompiledFitting,
    state: &ShipFitting,
    work: &RefitWork,
) -> Result<phoenix_model::damage::RemedyConfig, String> {
    let equipment = profile.equipment(
        &state
            .equipment
            .assemblies
            .get(&work.item)
            .ok_or("work lost actual item")?
            .definition,
    )?;
    Ok(phoenix_model::damage::RemedyConfig {
        work_secs: equipment.work.work_secs,
        manual_work_fraction: equipment.work.manual_work_fraction,
        structural_hp: None,
        outcome: phoenix_model::damage::RemedyOutcome::Full,
        requirements: equipment.work.requirements.clone(),
    })
}

fn tick(world: &mut World) {
    let now = world
        .get_resource::<crate::sim_tick::SimTick>()
        .map_or(0, |tick| tick.0);
    let dt = world
        .get_resource::<Time>()
        .map_or(0.0, |time| time.delta_secs());
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    let mut query = world.query::<(Entity, &EntityUuid, &ShipFitting)>();
    let mut order: Vec<_> = query
        .iter(world)
        .map(|(entity, uuid, _)| (uuid.0.clone(), entity))
        .collect();
    order.sort_by(|a, b| a.0.cmp(&b.0));
    for (_, entity) in order {
        if world
            .get::<ShipFitting>(entity)
            .is_some_and(|state| state.last_tick == Some(now))
        {
            continue;
        }
        advance(world, entity, now, dt);
    }
}

fn advance(world: &mut World, entity: Entity, now: u64, dt: f32) {
    let Some(profile) = world
        .get::<CompiledShipFitting>(entity)
        .map(|p| p.0.clone())
    else {
        return;
    };
    let Some(mut state) = world.get::<ShipFitting>(entity).cloned() else {
        return;
    };
    let Some(mut teams) = world.get::<ShipRepairTeams>(entity).cloned() else {
        return;
    };
    let Some(mut inventory) = world.get::<InfrastructureCondition>(entity).cloned() else {
        return;
    };
    let Some(services) = world.get::<ResourceServices>(entity).cloned() else {
        return;
    };
    let Some(mut damage) = world.get::<ShipDamage>(entity).cloned() else {
        return;
    };
    let mut power = world.get::<ShipPowerSystem>(entity).cloned();
    let blocked = world
        .get::<crate::ship::access::InteriorAccess>(entity)
        .zip(teams.0.layout())
        .map(|(access, layout)| access.blocked_connections(layout))
        .unwrap_or_default();
    // Redirect a recalled carrier before return travel can finish its JobId.
    // No paid stage or parts consumption occurs during this cleanup route.
    for work in state.work.values_mut() {
        let recalled = teams.0.save_state().teams.iter().flatten().any(|job| {
            job.id == work.job
                && matches!(
                    job.activity,
                    crate::jobs::repair::RepairActivity::Refit {
                        cancelled: true,
                        ..
                    }
                )
        });
        if work.waiting == "cancelled" || recalled || work.stage == RefitStage::Returning {
            work.stage = RefitStage::Returning;
            work.progress = 0.0;
            inventory.0.release_repair_claim(&work.claim());
            work.reserved = false;
            if matches!(
                &state.equipment.assemblies[&work.item].location,
                EquipmentLocation::InWork { .. }
            ) {
                if let Ok(locker) = profile.locker(&work.source_locker) {
                    teams
                        .0
                        .visit_refit_return_compartment(work.job, &locker.compartment);
                }
            }
        }
    }
    teams.0.tick_refits(dt, &blocked);
    let mut flags = Vec::new();
    let slots: Vec<_> = state.work.keys().cloned().collect();
    for slot in slots {
        let mut work = state.work[&slot].clone();
        let Ok(requirements) = recipe(&profile, &state, &work) else {
            continue;
        };
        if matches!(
            &state.equipment.assemblies[&work.item].location,
            EquipmentLocation::InWork { .. }
        ) {
            let actual_room =
                teams
                    .0
                    .refit_location(work.job)
                    .and_then(|location| match location {
                        phoenix_model::interior::TeamLocation::Compartment { compartment } => {
                            Some(compartment.clone())
                        }
                        phoenix_model::interior::TeamLocation::Crossing { from, .. } => {
                            Some(from.clone())
                        }
                        _ => None,
                    });
            if let Some(room) = actual_room {
                if state
                    .equipment
                    .move_in_work(&profile, &work.item, work.job.0, &room)
                    .is_err()
                {
                    continue;
                }
            }
        }
        if work.stage == RefitStage::Returning {
            let stored = matches!(
                &state.equipment.assemblies[&work.item].location,
                EquipmentLocation::Stored { .. }
            );
            if (stored || teams.0.refit_return_effect_enabled(work.job))
                && return_items(
                    &profile,
                    &mut state,
                    &work,
                    world
                        .get::<crate::resources::equipment_recovery_server::ShipEquipmentRecovery>(
                            entity,
                        ),
                )
                .is_ok()
            {
                inventory.0.release_repair_claim(&work.claim());
                teams.0.recall_refit(work.job);
                state.work.remove(&slot);
                state.requested.remove(&slot);
            } else {
                work.waiting = "returning_equipment".into();
                state.work.insert(slot, work);
            }
            continue;
        }
        work.waiting.clear();
        if foreign_repair_claim(
            world,
            entity,
            &profile.slot(&slot).expect("compiled slot").system,
        ) {
            work.waiting = "physical_task_claimed".into();
        } else if !teams.0.refit_effect_enabled(work.job) {
            work.waiting = "team_travelling".into();
        } else if damage.catastrophic() {
            work.waiting = "compartment_unavailable".into();
        } else if work.stage == RefitStage::Fetching {
            let source = &profile
                .locker(&work.source_locker)
                .expect("compiled locker")
                .compartment;
            let mut candidate = state.equipment.clone();
            let mut candidate_teams = teams.clone();
            if candidate
                .pick_up_from_locker(&profile, &work.item, work.job.0, source)
                .is_ok()
                && candidate_teams.0.visit_refit_compartment(
                    work.job,
                    profile.compartment(&slot).expect("compiled slot"),
                )
            {
                state.equipment = candidate;
                teams = candidate_teams;
                work.stage = RefitStage::Delivering;
            } else {
                work.waiting = "equipment_or_space_unavailable".into();
            }
        } else if work.stage == RefitStage::Delivering {
            work.stage = RefitStage::Preparing;
        } else if damage
            .state
            .compartments
            .get(profile.compartment(&slot).unwrap_or(""))
            .is_none_or(|room| room.disabled)
        {
            work.waiting = "compartment_unavailable".into();
        } else if profile
            .equipment(&state.equipment.assemblies[&work.item].definition)
            .is_ok_and(|equipment| equipment.work.class == RefitClass::Facility)
            && !facility_available(world, entity)
        {
            work.waiting = "facility_required".into();
        } else {
            use crate::jobs::repair::RepairStockAction as Parts;
            if work.stage == RefitStage::Preparing {
                if inventory
                    .0
                    .repair_parts(Parts::Reserve, &work.claim(), &requirements, &services.0)
                    .is_err()
                {
                    work.waiting = "missing_parts".into();
                } else {
                    work.reserved = true;
                    work.stage = RefitStage::Removing;
                }
            } else {
                let system = &profile.slot(&slot).expect("compiled slot").system;
                let burning = system.0.strip_prefix("phaser-").is_some_and(|bank| {
                    world
                        .get::<crate::console::weapons::ActiveBeam>(entity)
                        .is_some_and(|beams| beams.is_bank_firing(bank))
                });
                if burning {
                    work.waiting = "safe_shutdown".into();
                } else {
                    let owner = teams.0.owner().clone();
                    let tool_factor =
                        damage.factor(&owner, phoenix_model::damage::DamageBinding::Operation);
                    let enabled = world
                        .get::<crate::ship_plugin::ShipSystemControlSources>(entity)
                        .is_none_or(|sources| !sources.0.is_offline(&owner));
                    let paid = if enabled && tool_factor > 0.0 {
                        power.as_mut().map_or(0.0, |power| {
                            power
                                .0
                                .work_continuous(&owner, dt, 1.0)
                                .map_or(0.0, |grant| grant.fraction * tool_factor)
                        })
                    } else {
                        0.0
                    };
                    let facility_fraction = if profile
                        .equipment(&state.equipment.assemblies[&work.item].definition)
                        .is_ok_and(|definition| definition.work.class == RefitClass::Facility)
                    {
                        facility_provider(world, entity)
                            .and_then(|provider| {
                                let facility =
                                    world.get::<CompiledRefitFacility>(provider)?.0.clone();
                                let factor = world.get::<ShipDamage>(provider)?.factor(
                                    &facility.system,
                                    phoenix_model::damage::DamageBinding::Operation,
                                );
                                world
                                    .get_mut::<ShipPowerSystem>(provider)?
                                    .0
                                    .work_continuous(&facility.system, dt, 1.0)
                                    .ok()
                                    .map(|grant| grant.fraction * factor)
                            })
                            .unwrap_or(0.0)
                    } else {
                        1.0
                    };
                    work.progress += dt
                        * facility_fraction
                        * (requirements.manual_work_fraction
                            + (1.0 - requirements.manual_work_fraction) * paid);
                    if work.progress >= requirements.work_secs {
                        let mut candidate = state.equipment.clone();
                        let mut next_damage = damage.clone();
                        let result = if work.stage == RefitStage::Removing {
                            remove_hardware(&profile, &mut candidate, &mut next_damage, &work)
                        } else {
                            install_hardware(&profile, &mut candidate, &mut next_damage, &work)
                        };
                        if let Ok(()) = result {
                            let mut next_power = power.clone();
                            let staged =
                                stage_capability(&profile, &candidate, next_power.as_mut());
                            if let Ok(effective) = staged {
                                let payment = if work.stage == RefitStage::Removing {
                                    inventory.0.repair_parts(
                                        Parts::Consume,
                                        &work.claim(),
                                        &requirements,
                                        &services.0,
                                    )
                                } else {
                                    Ok(Vec::new())
                                };
                                if let Ok(changes) = payment {
                                    flags.extend(changes);
                                    state.equipment = candidate;
                                    damage = next_damage;
                                    power = next_power;
                                    apply_capability(world, entity, &effective);
                                    if work.stage == RefitStage::Removing {
                                        work.stage = RefitStage::Installing;
                                        work.progress = 0.0;
                                        work.reserved = false;
                                    } else {
                                        teams.0.finish_refit(work.job);
                                        state.work.remove(&slot);
                                        state.requested.remove(&slot);
                                        continue;
                                    }
                                }
                            } else {
                                work.waiting = "unsafe_hardware_transition".into();
                            }
                        } else {
                            work.waiting = "equipment_or_space_unavailable".into();
                        }
                        work.progress = work.progress.min(requirements.work_secs);
                    }
                }
            }
        }
        state.work.insert(slot, work);
    }
    state.last_tick = Some(now);
    world
        .entity_mut(entity)
        .insert((state, teams, inventory, damage));
    if let Some(power) = power {
        world.entity_mut(entity).insert(power);
    }
    let uuid = world.get::<EntityUuid>(entity).map(|uuid| uuid.0.clone());
    if let (Some(uuid), Some(mut runtime)) = (
        uuid,
        world.get_resource_mut::<crate::world::server::WorldContentRuntime>(),
    ) {
        crate::infrastructure::server::mirror_flags(&mut runtime, &flags, &uuid, &None);
    }
}

fn remove_hardware(
    profile: &CompiledFitting,
    state: &mut FittingState,
    damage: &mut ShipDamage,
    work: &RefitWork,
) -> Result<(), String> {
    let slot = profile.slot(&work.slot)?;
    if installed(state, &work.slot).is_some() {
        let condition = damage
            .state
            .components
            .get_mut(&slot.system)
            .and_then(|components| components.get_mut(&slot.operation_component))
            .ok_or("slot has no Operation condition owner")?;
        state.remove_installed(profile, &work.slot, work.job.0, condition.clone())?;
        // A vacant slot has no replaceable hardware fault. The explicit fitting
        // vacancy gates actuation; fixed intake/cooling and structure stay real.
        *condition = phoenix_model::damage::ComponentState {
            emp: None,
            condition: phoenix_model::damage::ComponentCondition::Healthy,
            dose: 0.0,
            last_source: None,
        };
        if let Some(old) = &work.removed_item {
            state.return_to_locker(profile, old, work.job.0, &work.locker)?;
        }
    }
    Ok(())
}

fn install_hardware(
    profile: &CompiledFitting,
    state: &mut FittingState,
    damage: &mut ShipDamage,
    work: &RefitWork,
) -> Result<(), String> {
    let slot = profile.slot(&work.slot)?;
    let condition = state.install_from_work(profile, &work.slot, &work.item, work.job.0)?;
    *damage
        .state
        .components
        .get_mut(&slot.system)
        .and_then(|components| components.get_mut(&slot.operation_component))
        .ok_or("slot has no Operation condition owner")? = condition;
    damage.state.validate(&damage.definition)?;
    Ok(())
}

fn return_items(
    profile: &CompiledFitting,
    state: &mut ShipFitting,
    work: &RefitWork,
    recovery: Option<&crate::resources::equipment_recovery_server::ShipEquipmentRecovery>,
) -> Result<(), String> {
    let mut candidate = state.equipment.clone();
    if matches!(&candidate.assemblies[&work.item].location, EquipmentLocation::InWork { job, .. } if *job == work.job.0)
    {
        candidate.return_to_locker(profile, &work.item, work.job.0, &work.source_locker)?;
    }
    let mut staged = state.clone();
    staged.equipment = candidate.clone();
    let locker = profile.locker(&work.source_locker)?;
    let used = candidate.assemblies.values().filter(|item| matches!(&item.location, EquipmentLocation::Stored { locker: id } if id == &locker.id))
        .map(|item| profile.equipment(&item.definition).map_or(u64::MAX, |definition| u64::from(definition.storage_space))).fold(0_u64, u64::saturating_add);
    let incoming = recovery.map_or(0, |owner| {
        crate::resources::equipment_recovery_server::reserved_space(owner, &locker.id, profile)
    });
    if used
        .saturating_add(reserved_space(profile, &staged, &locker.id))
        .saturating_add(incoming)
        > u64::from(locker.space)
    {
        return Err("actual return locker is full or reserved".into());
    }
    state.equipment = candidate;
    Ok(())
}

pub(crate) fn stage_capability(
    profile: &CompiledFitting,
    state: &FittingState,
    power: Option<&mut ShipPowerSystem>,
) -> Result<crate::entities::config::EntityConfig, String> {
    let effective = profile.effective_config(state)?;
    if let Some(power) = power {
        power.0.reconfigure_energy(
            effective
                .energy
                .clone()
                .ok_or("fitted equipment requires energy hardware")?,
        )?;
    }
    Ok(effective)
}

pub(crate) fn apply_capability(
    world: &mut World,
    entity: Entity,
    effective: &crate::entities::config::EntityConfig,
) {
    if let Some(weapons) = &effective.weapons_console {
        world.entity_mut(entity).insert((
            crate::entities::spawner::WeaponsConsoleSection(weapons.clone()),
            crate::console::weapons::PhaserCombatConfigResource(
                crate::entities::config::PhaserCombatConfig::from_weapons_console(weapons),
            ),
        ));
    }
}

/// The Repair controller gets exactly the explicit reports its Console receives.
/// This is the same observation used by the human work list, never another AI.
pub fn pending_requests(
    profile: &CompiledShipFitting,
    state: &ShipFitting,
    config: &crate::ship::config::ShipConfig,
    recipient: &SystemId,
) -> Vec<String> {
    state
        .requested
        .iter()
        .filter(|slot| {
            !state.work.contains_key(*slot)
                && profile
                    .0
                    .slot(slot)
                    .is_ok_and(|slot| can_report(config, recipient, &slot.system))
                && state.equipment.desired.get(*slot).is_some_and(|desired| {
                    state.equipment.assemblies.values().any(|item| {
                        &item.definition == desired
                            && !state.work.values().any(|work| work.item == item.id)
                            && matches!(&item.location, EquipmentLocation::Stored { .. })
                    })
                })
        })
        .cloned()
        .collect()
}

pub(crate) fn project(
    profile: &CompiledShipFitting,
    state: &ShipFitting,
    config: &crate::ship::config::ShipConfig,
    sources: &crate::ship::control_source::ControlSourceResolver,
    teams: Option<&ShipRepairTeams>,
    status: &mut BTreeMap<SystemId, phoenix_model::system_status::SystemStatus>,
) {
    use phoenix_model::system_status::{FittingChoiceView, FittingSlotView, StatusReasonCode};
    let repair = config
        .systems
        .iter()
        .find(|system| system.kind == crate::ship::system_registry::REPAIR_KIND);
    for slot in &profile.0.config().slots {
        let current = installed(&state.equipment, &slot.id).map(|item| item.definition.clone());
        let work = state.work.get(&slot.id);
        let choices = profile
            .0
            .config()
            .equipment
            .iter()
            .filter(|equipment| profile.0.compatible(&slot.id, &equipment.id).is_ok())
            .map(|equipment| {
                let available = state
                    .equipment
                    .assemblies
                    .values()
                    .filter(|item| {
                        item.definition == equipment.id
                            && !state.work.values().any(|work| work.item == item.id)
                            && matches!(&item.location, EquipmentLocation::Stored { .. })
                    })
                    .count() as u32;
                let (family, statistics) = match &equipment.hardware {
                    crate::resources::fitting::EquipmentHardware::Beam(hardware) => (
                        "beam",
                        BTreeMap::from([
                            ("range".into(), hardware.beam_range.to_string()),
                            ("damage".into(), hardware.beam_damage_per_sec.to_string()),
                            (
                                "energy".into(),
                                hardware.continuous_cost_per_sec.to_string(),
                            ),
                            ("active".into(), hardware.beam_duration_secs.to_string()),
                            ("rest".into(), hardware.cooldown_secs.to_string()),
                        ]),
                    ),
                    crate::resources::fitting::EquipmentHardware::Reactor(hardware) => (
                        "reactor",
                        BTreeMap::from([
                            (
                                "sustained".into(),
                                hardware.sustainable_output_per_sec.to_string(),
                            ),
                            ("peak".into(), hardware.max_output_per_sec.to_string()),
                            ("ramp".into(), hardware.ramp_per_sec.to_string()),
                        ]),
                    ),
                };
                FittingChoiceView {
                    equipment: equipment.id.clone(),
                    available,
                    facility_required: equipment.work.class == RefitClass::Facility,
                    family: family.into(),
                    statistics,
                }
            })
            .collect();
        let view = FittingSlotView {
            slot: slot.id.clone(),
            system: slot.system.clone(),
            installed: current,
            desired: state
                .requested
                .contains(&slot.id)
                .then(|| state.equipment.desired.get(&slot.id).cloned())
                .flatten(),
            choices,
            phase: work
                .map(|work| {
                    if work.waiting.is_empty() {
                        format!("{:?}", work.stage).to_lowercase()
                    } else {
                        work.waiting.clone()
                    }
                })
                .unwrap_or_else(|| "idle".into()),
            progress: work
                .and_then(|work| {
                    recipe(&profile.0, state, work).ok().map(|recipe| {
                        ((work.progress / recipe.work_secs).clamp(0.0, 1.0) * 1000.0) as u16
                    })
                })
                .unwrap_or(0),
            can_request: sources
                .crew_order_policy_for(&slot.system)
                .accept_human_input
                && work.is_none(),
            can_dispatch: false,
            free_teams: Vec::new(),
        };
        if let Some(status) = status.get_mut(&slot.system) {
            if unavailable(profile, state, &slot.system) {
                status.unavailable(StatusReasonCode::EquipmentRemoved, None);
            }
            status.fitting.push(view.clone());
        }
        if let Some(repair) = repair.filter(|repair| can_report(config, &repair.id, &slot.system)) {
            if let Some(status) = status.get_mut(&repair.id) {
                let mut report = view;
                report.can_request = false;
                report.can_dispatch = sources.crew_order_policy_for(&repair.id).accept_human_input
                    && state.requested.contains(&slot.id);
                report.free_teams = teams
                    .map(|teams| {
                        teams
                            .0
                            .free_team_indices(None)
                            .into_iter()
                            .map(|team| team as u32)
                            .collect()
                    })
                    .unwrap_or_default();
                status.fitting.push(report);
            }
        }
    }
}

#[cfg(test)]
#[path = "fitting_server_tests.rs"]
mod tests;
