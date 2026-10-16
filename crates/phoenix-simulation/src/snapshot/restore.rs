//! Restore.
use super::*;

/// What a restore actually managed to do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreReport {
    /// Captured entity rows that found a home — bootstrapped or built.
    pub entities_restored: usize,
    /// How many of those this restore had to **build** because the bootstrapped
    /// world had no such entity (issue #863).
    ///
    /// Counted separately because the two are different claims about the same
    /// number: a restore that matched everything is a resume of a run the fresh
    /// app could have replayed, and a restore that built ships is a resume of one
    /// it could not. A host that wants to know whether the save is carrying the
    /// run or merely correcting it reads this.
    pub entities_spawned: usize,
    pub asteroids_restored: usize,
    /// Entities the bootstrap spawned that the capture did not have. Despawned
    /// — a resumed world must not carry a ship the save never saw.
    pub despawned: usize,
    pub gaps: Vec<RestoreGap>,
}

impl RestoreReport {
    /// Whether every captured row found a home. A clean restore is the only
    /// one whose digest can be expected to match the capture's.
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }
}

/// Write a captured payload back over a bootstrapped world.
///
/// The world handed in must already be the *same scenario*, freshly built and
/// run far enough that its authored entities exist — see the module docs on why
/// this is an overwrite and not a spawn. `Run::scenario` and `Run::seed` are
/// what tell a host which world that is.
pub fn restore(world: &mut World, snapshot: &PhoenixSnapshot) -> RestoreReport {
    let mut report = RestoreReport::default();
    if let Some(reason) = snapshot.comms.as_ref().and_then(|comms| {
        phoenix_model::comms_channel::validate_transmissions(&comms.inbox, snapshot.tick).err()
    }) {
        report.gaps.push(RestoreGap::ResourceStateInvalid {
            uuid: "comms".into(),
            reason: reason.into(),
        });
        return report;
    }
    if let Err(reason) = super::comms_requests::validate(world, snapshot) {
        report.gaps.push(RestoreGap::ResourceStateInvalid {
            uuid: "comms".into(),
            reason,
        });
        return report;
    }
    if let Err((uuid, reason)) = super::crew_presence::validate(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let (Some(boot), Some(config)) = (
        snapshot.boot_identity.as_ref(),
        world.get_resource::<crate::world::config::WorldConfig>(),
    ) {
        if boot
            .game_start_entity_uuids
            .iter()
            .any(|row| row.authored_slot_id.is_some())
        {
            if let Err(error) = super::preflight::validate_boot_identity_for_world(boot, config) {
                report.gaps.push(RestoreGap::ResourceStateInvalid {
                    uuid: "world/launch-slots".into(),
                    reason: format!("{error:?}"),
                });
                return report;
            }
        }
    }
    if let Err(reason) = super::opportunity_comms::validate(snapshot) {
        report.gaps.push(RestoreGap::ResourceStateInvalid {
            uuid: "scenario/opportunity-comms".into(),
            reason,
        });
        return report;
    }
    if let Err(reason) = super::activities::validate(world, snapshot) {
        report.gaps.push(RestoreGap::ResourceStateInvalid {
            uuid: "scenario/activities".into(),
            reason,
        });
        return report;
    }
    if let Err((uuid, reason)) = super::vessel_ownership::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::security::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::personnel::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Some(reason) = snapshot
        .comms
        .as_ref()
        .and_then(|comms| comms.open_hails.validate().err())
    {
        report.gaps.push(RestoreGap::ResourceStateInvalid {
            uuid: "comms".into(),
            reason: reason.into(),
        });
        return report;
    }
    if let Err((uuid, reason)) = super::comms_capture::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::emission_intent::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::comms_media::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::concealment::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::identity::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::protection::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::jamming::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::helm_movement::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::craft::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::transit::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::flight_carriers::validate(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::guided_rounds::validate_roster(snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::guided_source::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::weapon_targets::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::emp::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::mounts::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if snapshot.scenario.as_ref().is_some_and(|scenario| {
        scenario.pending_events.iter().any(|event| {
            event.kind == 12 && super::application::world_event_from_record(event).is_none()
        })
    }) {
        report.gaps.push(RestoreGap::ResourceStateInvalid {
            uuid: "world".into(),
            reason: "invalid pending enquiry event".into(),
        });
        return report;
    }
    if let Err((uuid, reason)) = super::spatial_fields::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Some(gap) = super::preflight::mission_clock_gap(world, snapshot) {
        report.gaps.push(gap);
        return report;
    }
    if let Err((uuid, reason)) = super::historical_clock::validate(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }
    if let Err((uuid, reason)) = super::fitting::validate_roster(world, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
        return report;
    }

    // Layer ASTs and handlers are derived state, and trigger progress is stored
    // by index. Refuse the whole overwrite while the fresh bootstrap still has
    // an extra/missing/reordered layer (or a change that can create one next
    // tick), rather than restoring callbacks into one composition and letting
    // the next layer activation append a duplicate set underneath them.
    if let Some(gap) = layer_topology_gap(world, snapshot) {
        report.gaps.push(gap);
        return report;
    }

    let present: Vec<_> = world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
        .map(|mut query| {
            query
                .iter(world)
                .map(|(entity, uuid)| (entity, uuid.0.clone()))
                .collect()
        })
        .unwrap_or_default();
    for row in &snapshot.entities {
        if let Some((entity, _)) = present.iter().find(|(_, uuid)| *uuid == row.uuid) {
            if let Err(reason) = super::preflight::validate_entity_resources(world, *entity, row) {
                report.gaps.push(RestoreGap::ResourceStateInvalid {
                    uuid: row.uuid.clone(),
                    reason,
                });
            }
            if let Err(reason) = validate_entity_repair_jobs(world, *entity, row) {
                report.gaps.push(RestoreGap::RepairJobsInvalid {
                    uuid: row.uuid.clone(),
                    reason,
                });
            }
        }
    }
    super::materialization::preflight_missing_entities(world, snapshot, &mut report);
    if !report.gaps.is_empty() {
        return report;
    }
    let prior_craft_guard = crate::entities::craft::restore_guard(world, true);
    let prior_guard = crate::console::weapons::flight_carrier::suppress(world, true);
    restore_entities(world, snapshot, &mut report);
    crate::console::weapons::flight_carrier::suppress(world, prior_guard);
    world.flush();
    crate::entities::craft::restore_guard(world, prior_craft_guard);
    if report.gaps.iter().any(|gap| {
        matches!(
            gap,
            RestoreGap::RepairJobsInvalid { .. } | RestoreGap::ResourceStateInvalid { .. }
        )
    }) {
        return report;
    }
    super::personnel::apply(world, snapshot);
    restore_npc_frequency_matches(world, snapshot, &mut report);
    restore_coordination_staging(world, snapshot, &mut report);
    restore_asteroids(world, snapshot, &mut report);
    restore_run_scope(world, snapshot, &mut report);
    restore_ai_cadence(world);
    rebuild_power_modifiers(world);
    // Radar capability is derived from restored HP and the GM latch. Input
    // consumers run before the next Modifiers phase, so rebuild it now.
    {
        use bevy::ecs::system::RunSystemOnce;
        let _ = world.run_system_once(crate::modifiers::coordination::apply_radar_damage_modifiers);
        // Physics precedes the next Modifiers phase. Restore the drive cap now,
        // including removal of an active bootstrap bonus from an idle save.
        let _ = world.run_system_once(crate::modifiers::coordination::translate_impulse_modifiers);
    }
    restore_ai_world_snapshot(world, snapshot);
    restore_motion_plans(world, snapshot);
    crate::regions::server::rebuild_region_projections(world);
    {
        use bevy::ecs::system::RunSystemOnce;
        let _ = world.run_system_once(crate::science::server::publish_scan_blackboard);
        let _ = world.run_system_once(crate::science::passive::publish);
        let _ = world.run_system_once(crate::ship::sensors::publish_sensor_radar_blackboard);
        let _ = world.run_system_once(crate::console::helm::server::publish_helm_blackboard);
        let _ = world
            .run_system_once(crate::console::navigation::server::publish_navigation_blackboard);
        let _ = world.run_system_once(
            crate::console::weapons::blackboard::publish_tactical_radar_blackboard,
        );
        let _ = world.run_system_once(crate::console::command::server::publish_command_blackboard);
    }
    super::sensing_publications::restore_held(world, snapshot);
    if let Some(mut media) = world.get_resource_mut::<crate::comms::media_mesh::MediaMesh>() {
        media.invalidate();
    }
    if let Some(mut edge) = world.get_resource_mut::<crate::comms::media_edge::MediaEdge>() {
        edge.invalidate();
    }
    // Device consent never resumes. Retain the live generation high-water so
    // queued pre-restore packets cannot match a fresh explicit start.
    if let Some(mut leases) = world.get_resource_mut::<crate::comms::media_session::MediaLeases>() {
        leases.invalidate_all();
    }
    if let Some(mut carriers) =
        world.get_resource_mut::<crate::comms::media_control::PaidMediaCarriers>()
    {
        carriers.clear();
    }

    report
}
