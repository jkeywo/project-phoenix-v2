//! Complete schedules on two differently projected hosts: Dynasty's authored
//! strike cycle uses ordinary Backfill commands and remains fleet deterministic.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::command_admission::HostSlot;
use project_phoenix::core::messages::{ClientMessage, StationId, SystemControlPayload, SystemId};
use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::lobby::{InboundMessage, Sessions};
use project_phoenix::lockstep::{
    join_fleet, FleetRoster, FleetShip, FleetSlotOf, MeshInbox, MeshOutbox,
};
use project_phoenix::ship::helm_ai::HelmEnginesAiPolicyState;
use project_phoenix::ship::power::ShipPowerSystem;
use project_phoenix::sim_tick::SimTick;
use std::collections::BTreeSet;

const DYNASTY: &str = "assets/entities/dynasty_player_cruiser.toml";
const ALLIANCE: &str = "assets/entities/alliance_cruiser.toml";

fn host(local: HostSlot, crewed: bool) -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: "assets/worlds/probe_fleet_duel.toml".into(),
        ship_path: if local == HostSlot(1) {
            DYNASTY
        } else {
            ALLIANCE
        }
        .into(),
        max_ticks: 100_000,
        seed: Some(1_539_039),
        deterministic: true,
        ..Default::default()
    })
    .unwrap();
    let stations = ["command", "sensors", "damage-control"];
    if crewed && local == HostSlot(1) {
        let mut sessions = app.world_mut().resource_mut::<Sessions>();
        for station in stations {
            sessions.0.register(station.into(), station.into()).unwrap();
            sessions
                .0
                .set_station(station, Some(StationId(station.into())));
        }
    }
    let roster = FleetRoster::new(
        vec![
            FleetShip {
                host: HostSlot(1),
                ship_path: Some(DYNASTY.into()),
                authored_slot_id: None,
                crew: if crewed {
                    stations
                        .into_iter()
                        .map(|s| (StationId(s.into()), "Std".into()))
                        .collect()
                } else {
                    vec![]
                },
            },
            FleetShip {
                host: HostSlot(2),
                ship_path: Some(ALLIANCE.into()),
                authored_slot_id: None,
                crew: vec![],
            },
        ],
        local,
    );
    let delay = project_phoenix::lockstep::authored_delay(app.world());
    assert!(join_fleet(app.world_mut(), roster, delay));
    app
}

fn step(hosts: &mut [App; 2]) {
    let frames = hosts
        .each_mut()
        .map(|app| app.world_mut().resource_mut::<MeshOutbox>().drain());
    for i in 0..2 {
        for frame in &frames[1 - i] {
            hosts[i]
                .world_mut()
                .resource_mut::<MeshInbox>()
                .push(frame.clone());
        }
    }
    for app in hosts.iter_mut() {
        run(app, 1);
    }
    assert_eq!(
        hosts[0].world().resource::<SimTick>().0,
        hosts[1].world().resource::<SimTick>().0
    );
    if world_digest(hosts[0].world()) != world_digest(hosts[1].world()) {
        for (index, host) in hosts.iter().enumerate() {
            let path = std::env::temp_dir().join(format!("dynasty-peer-{index}.json"));
            std::fs::write(
                &path,
                serde_json::to_vec_pretty(&project_phoenix::snapshot::capture(host.world()))
                    .unwrap(),
            )
            .unwrap();
            eprintln!(
                "peer {index}: {:?}; snapshot {}",
                project_phoenix::sim_digest::digest_stages(host.world()),
                path.display()
            );
        }
    }
    assert_eq!(
        world_digest(hosts[0].world()),
        world_digest(hosts[1].world()),
        "tick {}",
        hosts[0].world().resource::<SimTick>().0
    );
}

#[test]
fn authored_dynasty_cycles_with_backfill_and_three_humans_on_both_peers() {
    for crewed in [false, true] {
        let mut hosts = [host(HostSlot(1), crewed), host(HostSlot(2), crewed)];
        let mut phases = BTreeSet::new();
        let mut enabled_before = false;
        let mut enables = 0;
        let mut depleted = false;
        let mut max_charge = 0.0_f32;
        for frame in 0..12_000 {
            if crewed && frame == 10 {
                let world = hosts[0].world_mut();
                let mut ships = world.query::<(
                    &FleetSlotOf,
                    &project_phoenix::ship_plugin::ShipConfigComponent,
                    &project_phoenix::ship_plugin::ShipSystemControlSources,
                )>();
                let (_, config, sources) = ships
                    .iter(world)
                    .find(|(slot, _, _)| slot.0 == HostSlot(1))
                    .unwrap();
                for station in ["command", "sensors", "damage-control"] {
                    assert!(
                        config
                            .0
                            .systems_for_station(&StationId(station.into()))
                            .any(|system| sources.0.source_for(&system.id)
                                == project_phoenix::ship::control_source::ControlSource::Human),
                        "{station} must be a real human Station"
                    );
                }
                hosts[0]
                    .world_mut()
                    .resource_mut::<Messages<InboundMessage>>()
                    .write(InboundMessage {
                        token: "command".into(),
                        msg: ClientMessage::ControlSystem {
                            target: SystemId("red-alert".into()),
                            payload: SystemControlPayload::SetRedAlert { active: true },
                        },
                    });
            }
            step(&mut hosts);
            let world = hosts[0].world_mut();
            let mut q =
                world.query::<(&FleetSlotOf, &ShipPowerSystem, &HelmEnginesAiPolicyState)>();
            for (slot, power, phase) in q.iter(world) {
                if slot.0 != HostSlot(1) {
                    continue;
                }
                phases.insert(phase.0.current.clone());
                max_charge = max_charge.max(power.0.battery_charge);
                let boost = power.0.strike_boost().unwrap();
                if boost.enabled && !enabled_before {
                    enables += 1;
                }
                if enabled_before && !boost.enabled {
                    assert!(
                        boost.depleted,
                        "automatic shutdown must come from an unpaid emitted attack"
                    );
                    depleted = true;
                }
                enabled_before = boost.enabled;
            }
            if enables >= 2 && depleted && phases.contains("attack") && phases.contains("recovery")
            {
                break;
            }
        }
        assert!(
            max_charge >= 60.0,
            "crewed={crewed}, charge={max_charge}, phases={phases:?}"
        );
        assert!(
            depleted && enables >= 2,
            "crewed={crewed}, enables={enables}, depleted={depleted}, phases={phases:?}"
        );
        assert!(
            phases.contains("charge")
                && phases.contains("approach")
                && phases.contains("attack")
                && phases.contains("recovery"),
            "{phases:?}"
        );
    }
}
