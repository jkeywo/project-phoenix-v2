//! Candidate-only evidence checks over the actual accepted hull/Repair owner.
use super::*;
use bevy::ecs::system::RunSystemOnce;
use phoenix_model::{activity::ActivityOutcome, damage::RepairWorkTarget};
use phoenix_sim_world::world::activities::*;

fn pending_work() -> (World, Entity, PhoenixSnapshot) {
    pending_work_at(false)
}
fn pending_work_at(slot: bool) -> (World, Entity, PhoenixSnapshot) {
    let (mut world, mut actor, _) = fixture();
    let mut base = crate::world::activities::declared_actor_fixture(
        "personnel-ship",
        "assets/entities/personnel_cruiser.toml",
    );
    if slot {
        assert!(world.despawn(actor));
        let path = "assets/entities/personnel_cruiser.toml".to_owned();
        base.ship_slots = vec![crate::world::config::ShipSlotConfig {
            id: "lead".into(),
            label: None,
            ships: vec![crate::world::config::AvailableShipEntry {
                template_path: path.clone(),
                label: None,
            }],
            default_ship: path.clone(),
            unclaimed: phoenix_model::wire::UnclaimedSlotPolicy::Backfill,
        }];
        base.entities[0].spawn_on = crate::world::config::WorldEntitySpawnOn::GameStart;
        base.name_to_uuid.clear();
        world.insert_resource(crate::ship_slots::FrozenShipSlots::from_unclaimed_slots(
            &base.ship_slots,
        ));
        world.insert_resource(crate::lockstep::FleetRoster::default());
        world.insert_resource(crate::lobby::SelectedShipResource(path));
        world.insert_resource(base.clone());
        world
            .run_system_once(crate::server_app::spawn_game_start_entities)
            .unwrap();
        actor = world
            .query::<(Entity, &crate::ship_slots::AuthoredShipSlotId)>()
            .iter(&world)
            .find_map(|(entity, slot)| (slot.0 == "lead").then_some(entity))
            .unwrap();
        let metadata = world.resource::<crate::server_app::GameStartEntityUuids>();
        assert_eq!(metadata.0.len(), 1);
        assert_eq!(
            metadata.0[0].authored_slot_id.as_deref(),
            Some("lead"),
            "actual FleetPlacement writes the existing boot record"
        );
    }
    world.insert_resource(base);
    let owner = SystemId("repair".into());
    let room = world
        .get::<crate::ship::compartment_damage::ShipDamage>(actor)
        .unwrap()
        .definition
        .interior
        .team_homes[0]
        .clone();
    let target = RepairWorkTarget::Compartment {
        compartment: room.clone(),
    };
    let request = crate::core::messages::RepairTarget::Work(target.clone());
    let definition = SituationDefinition {
        id: "restore".into(),
        initial: "useful".into(),
        state: vec![SituationState {
            id: "useful".into(),
            when: SituationCondition::Achieved,
            call: None,
            next: None,
        }],
    };
    let config = SituationConfig {
        id: "restore-home".into(),
        definition: definition.id.clone(),
        request: ActivityRequest::Repair {
            actor: if slot { "slot:lead" } else { "personnel-ship" }.into(),
            owner: owner.clone(),
            team: 0,
            request: request.clone(),
            target,
            goal: RepairGoal::Structure,
        },
    };
    let mut runtime = WorldContentRuntime::default();
    runtime.mission_clock = Some(crate::world::server::MissionClock::new(
        std::time::Duration::ZERO,
    ));
    runtime.situation_definitions = vec![definition];
    runtime.situation_configs = vec![config];
    world.insert_resource(runtime);
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    world.insert_resource(time);
    {
        let mut damage = world
            .get_mut::<crate::ship::compartment_damage::ShipDamage>(actor)
            .unwrap();
        damage.state.compartments.get_mut(&room).unwrap().hp -= 1.0;
    }
    {
        let mut power = world
            .get_mut::<crate::ship::power::ShipPowerSystem>(actor)
            .unwrap();
        let capacity = power
            .0
            .config()
            .consumers
            .iter()
            .find(|row| row.system == owner)
            .unwrap()
            .reserve
            .capacity;
        power
            .0
            .network_mut()
            .consumers
            .get_mut(&owner)
            .unwrap()
            .energy = capacity;
    }
    world
        .get_mut::<crate::core::messages::AdmittedCommands>(actor)
        .unwrap()
        .0
        .push(crate::core::messages::AdmittedCommand {
            target: owner,
            payload: crate::core::messages::SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: request,
            },
            response_token: None,
            feedback_correlation: None,
        });
    world
        .run_system_once(crate::console::repair::dispatch::handle_dispatch_repair_team)
        .unwrap();
    world
        .get_mut::<crate::core::messages::AdmittedCommands>(actor)
        .unwrap()
        .0
        .clear();
    for tick in 21..=2000 {
        world.resource_mut::<SimTick>().0 = tick;
        world
            .run_system_once(crate::console::repair::server::tick_repair_teams)
            .unwrap();
        if world.resource::<WorldContentRuntime>().pending_world_events.iter().any(|event|matches!(event,WorldEvent::Activity(fact) if matches!(fact.outcome,ActivityOutcome::RepairAchieved {..}))) {
            let saved=capture(&world);
            super::super::super::activities::validate(&world,&saved).unwrap();
            return (world,actor,saved);
        }
    }
    panic!("actual on-site paid structural remedy did not commit");
}

#[test]
fn pending_fact_json_and_consumed_history_survive_return_and_team_reassignment() {
    let (mut world, actor, saved) = pending_work();
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    super::super::super::activities::validate(&world, &decoded).unwrap();
    let mut report = RestoreReport::default();
    super::super::super::application::restore_scenario(&mut world, &decoded, &mut report);
    assert!(report.gaps.is_empty());
    let now = world.resource::<SimTick>().0;
    crate::world::activities::acknowledge_pending(
        &mut world.resource_mut::<WorldContentRuntime>(),
        now,
    );
    world
        .get_mut::<crate::console::repair::server::ShipRepairTeams>(actor)
        .unwrap()
        .0
        .recall(0);
    for tick in now + 1..=now + 2000 {
        world.resource_mut::<SimTick>().0 = tick;
        world
            .run_system_once(crate::console::repair::server::tick_repair_teams)
            .unwrap();
        crate::world::activities::acknowledge_pending(
            &mut world.resource_mut::<WorldContentRuntime>(),
            tick,
        );
        if world.resource::<WorldContentRuntime>().situations.instances["restore-home"]
            .settled
            .is_some()
        {
            break;
        }
    }
    assert!(
        world.resource::<WorldContentRuntime>().situations.instances["restore-home"]
            .settled
            .is_some()
    );
    let same = world.resource::<WorldContentRuntime>().situations.clone();
    let mut teams = world
        .get_mut::<crate::console::repair::server::ShipRepairTeams>(actor)
        .unwrap();
    let target = SystemId("power-reactor".into());
    assert!(teams
        .0
        .dispatch_physical_job(0, target.clone(), None, target.0)
        .is_some());
    drop(teams);
    let historical = capture(&world);
    super::super::super::activities::validate(&world, &historical).unwrap();
    assert_eq!(
        world.resource::<WorldContentRuntime>().situations,
        same,
        "slot reuse does not replace credited history"
    );
    assert!(world.despawn(actor));
    let removed = capture(&world);
    super::super::super::activities::validate(&world, &removed).unwrap();
    assert_eq!(
        removed.scenario.as_ref().unwrap().situations,
        same,
        "terminal evidence survives actual actor removal using its existing base-world recipe"
    );
    let mut forgotten = removed;
    forgotten
        .scenario
        .as_mut()
        .unwrap()
        .situations
        .instances
        .clear();
    assert!(
        super::super::super::activities::validate(&world, &forgotten).is_err(),
        "loss cannot erase an already-resolved initial actor's arming frontier"
    );
}

#[test]
fn malformed_whole_candidate_refuses_before_any_clock_entity_or_evidence_write() {
    let (mut world, _, saved) = pending_work();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..8 {
        let mut candidate = saved.clone();
        candidate.tick += 10;
        let scenario = candidate.scenario.as_mut().unwrap();
        match fault {
            0 => scenario.situations.instances.clear(),
            1 => scenario.pending_events.push(
                scenario
                    .pending_events
                    .iter()
                    .find(|event| event.kind == 13)
                    .unwrap()
                    .clone(),
            ),
            2 => {
                scenario
                    .pending_events
                    .iter_mut()
                    .find(|event| event.kind == 13)
                    .unwrap()
                    .activity
                    .as_mut()
                    .unwrap()
                    .scope
                    .identity = phoenix_model::activity::ActivityExecutionIdentity::Physical {
                    job: phoenix_model::job::JobId(999),
                }
            }
            3 => {
                scenario
                    .pending_events
                    .iter_mut()
                    .find(|event| event.kind == 13)
                    .unwrap()
                    .activity
                    .as_mut()
                    .unwrap()
                    .tick = candidate.tick + 1
            }
            4 => {
                scenario
                    .situations
                    .instances
                    .get_mut("restore-home")
                    .unwrap()
                    .accepted
                    .as_mut()
                    .unwrap()
                    .identity = phoenix_model::activity::ActivityExecutionIdentity::Physical {
                    job: phoenix_model::job::JobId(999),
                }
            }
            5 => {
                scenario
                    .situations
                    .instances
                    .get_mut("restore-home")
                    .unwrap()
                    .request = ActivityRequest::Delivery {
                    actor: "personnel-ship".into(),
                    owner: SystemId("logistics".into()),
                    source: "main".into(),
                    destination: "local".into(),
                    commodity: "parts".into(),
                    amount: 1,
                }
            }
            6 => {
                scenario
                    .pending_events
                    .iter_mut()
                    .find(|event| event.kind == 13)
                    .unwrap()
                    .activity = None
            }
            _ => {
                let fact = scenario
                    .pending_events
                    .iter_mut()
                    .find(|event| event.kind == 13)
                    .unwrap()
                    .activity
                    .as_mut()
                    .unwrap();
                if let ActivityOutcome::RepairAchieved {
                    result: phoenix_model::activity::RepairAchievement::Structure { restored_hp },
                    ..
                } = &mut fact.outcome
                {
                    *restored_hp = f64::MAX;
                }
            }
        }
        // Restore performs this global preflight before changing any owner.
        assert!(
            !restore(&mut world, &candidate).is_complete(),
            "fault {fault}"
        );
        assert_eq!(capture(&world), before, "fault {fault}");
        assert_eq!(
            crate::sim_digest::world_digest(&world),
            digest,
            "fault {fault}"
        );
    }
}

#[test]
fn unresolved_declared_conditional_actor_does_not_owe_tomorrows_instance() {
    let (mut world, _, _) = fixture();
    let mut base = crate::world::activities::declared_actor_fixture(
        "future",
        "assets/entities/personnel_cruiser.toml",
    );
    base.entities[0].when = Some("flag(reveal-future)".into());
    let definition = SituationDefinition {
        id: "future".into(),
        initial: "received".into(),
        state: vec![SituationState {
            id: "received".into(),
            when: SituationCondition::Received,
            call: None,
            next: None,
        }],
    };
    let request = SituationConfig {
        id: "future".into(),
        definition: definition.id.clone(),
        request: ActivityRequest::Delivery {
            actor: "future".into(),
            owner: SystemId("logistics".into()),
            source: "hold-ammunition".into(),
            destination: "magazine".into(),
            commodity: "torpedoes".into(),
            amount: 2,
        },
    };
    let mut runtime = WorldContentRuntime::default();
    runtime.mission_clock = Some(crate::world::server::MissionClock::new(
        std::time::Duration::ZERO,
    ));
    runtime.situation_definitions = vec![definition];
    runtime.situation_configs = vec![request];
    world.insert_resource(base);
    world.insert_resource(runtime);
    let saved = capture(&world);
    assert!(saved
        .scenario
        .as_ref()
        .unwrap()
        .situations
        .instances
        .is_empty());
    super::super::super::activities::validate(&world, &saved).unwrap();
}

#[test]
fn actual_launch_slot_history_survives_removed_actor_and_forged_boot_links_write_nothing() {
    let (mut world, actor, _) = pending_work_at(true);
    let now = world.resource::<SimTick>().0;
    crate::world::activities::acknowledge_pending(
        &mut world.resource_mut::<WorldContentRuntime>(),
        now,
    );
    world
        .get_mut::<crate::console::repair::server::ShipRepairTeams>(actor)
        .unwrap()
        .0
        .recall(0);
    for tick in now + 1..=now + 2000 {
        world.resource_mut::<SimTick>().0 = tick;
        world
            .run_system_once(crate::console::repair::server::tick_repair_teams)
            .unwrap();
        crate::world::activities::acknowledge_pending(
            &mut world.resource_mut::<WorldContentRuntime>(),
            tick,
        );
        if world.resource::<WorldContentRuntime>().situations.instances["restore-home"]
            .settled
            .is_some()
        {
            break;
        }
    }
    assert!(
        world.resource::<WorldContentRuntime>().situations.instances["restore-home"]
            .settled
            .is_some()
    );
    assert!(world.despawn(actor));
    let saved = capture(&world);
    let decoded: PhoenixSnapshot =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    super::super::super::activities::validate(&world, &decoded).unwrap();
    let before = capture(&world);
    let digest = crate::sim_digest::world_digest(&world);
    for fault in 0..6 {
        let mut candidate = decoded.clone();
        candidate.tick += 10;
        let boot = candidate.boot_identity.as_mut().unwrap();
        match fault {
            0 => boot.game_start_entity_uuids[0].authored_slot_id = Some("wrong-slot".into()),
            1 => boot.game_start_entity_uuids[0].authored_slot_id = None,
            2 => boot.game_start_entity_uuids[0].entity_uuid = "different-actor".into(),
            3 => {
                let row = boot.game_start_entity_uuids[0].clone();
                boot.game_start_entity_uuids.push(row);
            }
            4 => {
                candidate.frozen_ship_slots.as_mut().unwrap().0[0].hull =
                    "assets/entities/nav_beacon.toml".into()
            }
            _ => candidate
                .scenario
                .as_mut()
                .unwrap()
                .situations
                .instances
                .clear(),
        }
        assert!(
            !restore(&mut world, &candidate).is_complete(),
            "fault {fault}"
        );
        assert_eq!(capture(&world), before, "fault {fault}");
        assert_eq!(
            crate::sim_digest::world_digest(&world),
            digest,
            "fault {fault}"
        );
    }
}

#[test]
fn actor_lost_after_committed_work_retains_history_without_fabricated_settlement() {
    let (mut world, actor, _) = pending_work();
    assert!(world.despawn(actor));
    let saved = capture(&world);
    assert!(
        saved.scenario.as_ref().unwrap().situations.instances["restore-home"]
            .settled
            .is_none()
    );
    super::super::super::activities::validate(&world, &saved).unwrap();
    let now = world.resource::<SimTick>().0;
    crate::world::activities::acknowledge_pending(
        &mut world.resource_mut::<WorldContentRuntime>(),
        now,
    );
    let instance = &world.resource::<WorldContentRuntime>().situations.instances["restore-home"];
    assert!(instance.achieved.is_some());
    assert!(instance.settled.is_none());
    super::super::super::activities::validate(&world, &capture(&world)).unwrap();
}
