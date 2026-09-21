#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::command_admission::HostSlot;
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::lockstep::FleetSlotOf;
use project_phoenix::server_app::{LocalShip, Ship};
use project_phoenix::ship_slots::{
    AuthoredShipSlotId, FrozenShipSlots, LaunchSource, LaunchedSlot,
};

const WORLD: &str = "tests/fixtures/worlds/multi_ship_slots_direct_launch.toml";
const HULL: &str = "assets/entities/alliance_cruiser.toml";

#[test]
fn headless_direct_launch_freezes_backfill_defaults_and_omits_absent_slots() {
    let args = HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: HULL.into(),
        seed: Some(1_522),
        max_ticks: 30,
        ..Default::default()
    };
    let mut app = build_headless_app(&args).expect("seeded multi-ship fixture builds");
    assert_eq!(
        app.world().resource::<FrozenShipSlots>().0,
        vec![
            LaunchedSlot {
                slot_id: "lead".into(),
                hull: HULL.into(),
                claimant: None,
                source: LaunchSource::Backfill,
            },
            LaunchedSlot {
                slot_id: "wing".into(),
                hull: HULL.into(),
                claimant: None,
                source: LaunchSource::Backfill,
            },
        ],
        "the production headless entry freezes authored policy without an injected roster"
    );

    run(&mut app, args.max_ticks);
    let mut query = app
        .world_mut()
        .query::<(&AuthoredShipSlotId, &FleetSlotOf, Has<LocalShip>, &Ship)>();
    let mut rows: Vec<_> = query
        .iter(app.world())
        .map(|(slot, host, local, _)| (slot.0.clone(), host.0, local))
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        vec![
            ("lead".into(), HostSlot::SOLO, true),
            ("wing".into(), HostSlot(2), false),
        ],
        "the direct host keeps its first Backfill hull local while Absent reserve never spawns"
    );
}
