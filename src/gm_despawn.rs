//! Authored safe-removal policy and the shared scripted-removal lifecycle.
//!
//! Permission is the ordinary authored `gm_removable` tag, never a browser
//! assertion. Fleet hulls and foundational world geometry remain protected
//! even when mislabeled. Runtime hazards need their normal spawn provenance.
use crate::entities::spawner::{EntitySpawnOrigin, EntityTagsSection, EntityUuid};
use crate::gm_action::GmActionRefusalReason;
use crate::gm_despawn_undo::GmRemovalReference;
use bevy::prelude::*;

pub type RemovalQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static EntityUuid,
        Option<&'static EntityTagsSection>,
        Has<crate::lockstep::FleetSlotOf>,
        Has<crate::server_app::Ship>,
        Has<EntitySpawnOrigin>,
        Option<&'static crate::entities::spawner::RegionEffectsSection>,
        Has<crate::entities::spawner::AsteroidFieldSection>,
        Has<crate::entities::spawner::RegionShapeSection>,
    ),
>;

/// One policy shared by projection and the apply-boundary gate.
pub fn removable(
    tags: Option<&EntityTagsSection>,
    fleet: bool,
    ship: bool,
    spawned: bool,
    hazard: bool,
    field: bool,
) -> bool {
    let has = |tag: &str| tags.is_some_and(|tags| tags.0.iter().any(|t| t == tag));
    if fleet
        || field
        || !has("gm_removable")
        || [
            "player",
            "star",
            "planet",
            "moon",
            "asteroid_field",
            "trigger_volume",
            "objective_marker",
        ]
        .iter()
        .any(|tag| has(tag))
    {
        return false;
    }
    // A region never becomes removable merely by also being tagged a structure.
    if has("region") || hazard {
        return spawned && hazard;
    }
    ship || has("structure") || has("station")
}

pub fn validate_target(query: &RemovalQuery, target: &str) -> Result<(), GmActionRefusalReason> {
    let mut matches = query.iter().filter(|(uuid, ..)| uuid.0 == target);
    let Some((_, tags, fleet, ship, spawned, effects, field, shape)) = matches.next() else {
        return Err(GmActionRefusalReason::UnknownEntity);
    };
    if matches.next().is_some()
        || (shape && effects.is_none_or(|effects| effects.0.is_empty()))
        || !removable(
            tags,
            fleet,
            ship,
            spawned,
            effects.is_some_and(|effects| !effects.0.is_empty()),
            field,
        )
    {
        return Err(GmActionRefusalReason::ProtectedEntity);
    }
    Ok(())
}

/// Shared by ordinary `DestroyEntity` and accepted GM removals. Called after
/// the trigger cascade has observed Destroyed, with no peer-local query gates.
/// Authored name/group mappings are historical facts used by destruction
/// predicates and deliberately survive. Historical Comms messages also survive;
/// live locks, hail permission and support-system partners do not.
///
/// # Why it reports the overrides it cleared
///
/// The returned list is the bounded inverse data a GM removal's undo is built on
/// (issue #1444): the attributed GM contact-knowledge overrides an undo puts
/// back. It is produced by the ONE walk that does the clearing rather than by a
/// read-only mirror of it, so a clear added here cannot quietly stop being
/// restored by the GM who is about to reverse the removal.
///
/// The live console links this walk also releases - radar selections, weapon
/// locks, tows, docking, transports, dispatched repair crews, anchored
/// waypoints, Comms presence, task activations - are deliberately NOT reported.
/// Nothing restores them and nothing names them individually, and half of them
/// are derived from state kept outside the #894 digest boundary on purpose; see
/// [`crate::gm_despawn_undo`], which would have to fold and save anything this
/// function handed it. Ordinary scripted and combat destruction drops the value.
#[must_use]
pub fn remove_entity(world: &mut World, entity: Entity) -> Vec<GmRemovalReference> {
    let mut cleared: Vec<GmRemovalReference> = Vec::new();
    let Some(uuid) = world.get::<EntityUuid>(entity).map(|uuid| uuid.0.clone()) else {
        return cleared;
    };
    if let Some(mut content) = world.get_resource_mut::<crate::world::server::WorldContentRuntime>()
    {
        // The removed entity's OWN observations, and every other observer's
        // row about it. Both are attributed GM decisions, so both are recorded
        // for the inverse rather than merely dropped.
        for (target, mode) in content.contact_overrides.remove(&uuid).unwrap_or_default() {
            cleared.push(GmRemovalReference {
                observer: uuid.clone(),
                target,
                mode,
            });
        }
        for (observer, rows) in content.contact_overrides.iter_mut() {
            if let Some(mode) = rows.remove(&uuid) {
                cleared.push(GmRemovalReference {
                    observer: observer.clone(),
                    target: uuid.clone(),
                    mode,
                });
            }
        }
        content.contact_overrides.retain(|_, rows| !rows.is_empty());
    }
    // Close presentation activations in stable slot order, including starts
    // still queued this tick. Independent component clears below commute.
    use crate::core::task_lifecycle::{TaskLifecycleRequest, TaskLifecycles, TaskTerminalReason};
    use crate::effect_queue::EffectQueue;
    let mut slots = std::collections::BTreeMap::new();
    if let Some(tasks) = world.get_resource::<TaskLifecycles>() {
        for active in tasks.active() {
            if active.key.target.as_deref() == Some(&uuid) {
                slots.insert(active.key.slot.clone(), TaskTerminalReason::TargetDestroyed);
            } else if active.key.slot.operator == uuid {
                // Removing the actor withdraws its work; its subject survived.
                slots.insert(active.key.slot.clone(), TaskTerminalReason::OrderWithdrawn);
            }
        }
    }
    if let Some(mut queue) = world.get_resource_mut::<EffectQueue<TaskLifecycleRequest>>() {
        for request in &queue.0 {
            if let TaskLifecycleRequest::Start { slot, target } = request {
                if target.as_deref() == Some(&uuid) {
                    slots.insert(slot.clone(), TaskTerminalReason::TargetDestroyed);
                } else if slot.operator == uuid {
                    slots.insert(slot.clone(), TaskTerminalReason::OrderWithdrawn);
                }
            }
        }
        for (slot, reason) in slots {
            queue.0.push(TaskLifecycleRequest::End { slot, reason });
        }
    }
    for mut selection in world
        .query::<&mut crate::console::weapons::TacticalRadarSelection>()
        .iter_mut(world)
    {
        if selection.0.as_deref() == Some(&uuid) {
            selection.0 = None;
        }
    }
    for mut selection in world
        .query::<&mut crate::ship::sensors::SensorRadarSelection>()
        .iter_mut(world)
    {
        if selection.0.as_deref() == Some(&uuid) {
            selection.0 = None;
        }
    }
    for (mut beam, cooldown) in world
        .query::<(
            &mut crate::console::weapons::ActiveBeam,
            Option<&mut crate::console::weapons::PhaserCooldown>,
        )>()
        .iter_mut(world)
    {
        let banks: Vec<_> = beam
            .live_banks()
            .filter(|(_, slot)| slot.target_uuid == uuid)
            .map(|(bank, _)| bank.clone())
            .collect();
        let mut cooldown = cooldown;
        for bank in banks {
            if let Some(slot) = beam.end_bank(bank.as_str()) {
                if let Some(cooldown) = cooldown.as_deref_mut() {
                    cooldown.start_bank(bank.as_str(), slot.pending_cooldown_secs);
                }
            }
        }
    }
    for mut tractor in world
        .query::<&mut crate::tractor::TractorBeam>()
        .iter_mut(world)
    {
        if tractor.coupled_target.as_deref() == Some(&uuid) {
            tractor.coupled_target = None;
            tractor.engaged = false;
        }
    }
    for (mut dock, umbilical) in world
        .query::<(
            &mut crate::dock::DockControl,
            Option<&mut crate::umbilical::TransferUmbilical>,
        )>()
        .iter_mut(world)
    {
        if dock.available_target.as_deref() == Some(&uuid) {
            dock.available_target = None;
        }
        if dock.docking_target.as_deref() == Some(&uuid) {
            dock.docking_target = None;
            dock.engaged = false;
            dock.docked = false;
            dock.undock_target = None;
            if let Some(mut umbilical) = umbilical {
                umbilical.running = false;
                umbilical.activation_target = None;
                umbilical.partner_level = None;
            }
        }
    }
    for mut transport in world
        .query::<&mut crate::transporter::Transporter>()
        .iter_mut(world)
    {
        if transport.selected_contact.as_deref() == Some(&uuid)
            || transport.active_contact.as_deref() == Some(&uuid)
        {
            transport.selected_contact = None;
            transport.active_contact = None;
            transport.transporting = false;
            transport.progress = 0.0;
            transport.last_refusal = Some(crate::transporter::TransportRefusal::NoContact);
        }
    }
    for mut repair in world
        .query::<&mut crate::console::repair::external_server::ExternalRepairDispatch>()
        .iter_mut(world)
    {
        if repair.dispatched_target.as_deref() == Some(&uuid) {
            repair.release(Some(
                crate::console::repair::external::ExternalRepairRefusal::NoTarget,
            ));
        }
    }
    for mut waypoint in world
        .query::<&mut crate::console::navigation::server::NavigationWaypoint>()
        .iter_mut(world)
    {
        if matches!(waypoint.mode(), Some(crate::console::navigation::server::WaypointMode::Anchored { source_uuid, .. }) if source_uuid == &uuid)
        {
            waypoint.clear();
        }
    }
    for mut torpedoes in world
        .query::<&mut crate::console::weapons::TorpedoSystemResource>()
        .iter_mut(world)
    {
        for torpedo in &mut torpedoes.0.in_flight {
            if torpedo.target_uuid.as_deref() == Some(&uuid) {
                torpedo.target_uuid = None;
            }
        }
        for burst in &mut torpedoes.0.burst_states {
            if burst.target_uuid.as_deref() == Some(&uuid) {
                burst.target_uuid = None;
            }
        }
    }
    let message_ids = world
        .get_resource_mut::<crate::comms::server::CommsInboxRes>()
        .map(|mut inbox| inbox.0.orphan_sender(&uuid))
        .unwrap_or_default();
    if let Some(mut comms) = world.get_resource_mut::<crate::comms::server::CommsRuntime>() {
        let before = comms.contacts.len();
        comms.contacts.retain(|contact| contact.uuid != uuid);
        comms.needs_broadcast |= comms.contacts.len() != before;
        comms.open_hails.remove(&uuid);
        comms.range_flags.remove(&uuid);
        for flags in comms.fleet_range_flags.values_mut() {
            flags.remove(&uuid);
        }
        comms
            .active_dialogues
            .retain(|id, _| !message_ids.contains(id));
        comms
            .pending_ai_responses
            .retain(|key, _| !message_ids.contains(&key.message_id));
    }
    let names: std::collections::BTreeSet<_> = world
        .get_resource::<crate::world::server::WorldContentRuntime>()
        .map(|runtime| {
            runtime
                .name_to_uuid
                .iter()
                .filter(|(_, id)| *id == &uuid)
                .map(|(name, _)| name.clone())
                .collect()
        })
        .unwrap_or_default();
    if let Some(mut scripts) = world.get_resource_mut::<crate::world::server::WorldScriptRuntime>()
    {
        scripts
            .pending_comms_opens
            .retain(|open| open.from != uuid && !names.contains(&open.from));
    }
    if let Some(mut tokens) = world.get_resource_mut::<crate::ai::server::AiTokenRegistry>() {
        tokens.unregister_by_bevy_entity(entity);
    }
    if let Some(mut layers) = world.get_resource_mut::<crate::world::server::WorldLayerMap>() {
        for layer in layers.0.values_mut() {
            layer.spawned_entities.retain(|e| *e != entity);
        }
    }
    world.despawn(entity);
    // Sorted and de-duplicated before it leaves. This list is folded into the
    // deterministic digest by the capture that keeps it, and map iteration order
    // is an implementation detail no two peers may be required to share.
    cleared.sort();
    cleared.dedup();
    cleared
}
