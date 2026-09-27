//! Named, bounded T5 cross-feature journey (#1553); no performance threshold.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use project_phoenix::core::messages::GamePhase;
use project_phoenix::core::report::MissionReport;
use project_phoenix::headless::{build_headless_app, run, HeadlessArgs};
use project_phoenix::ship_slots::{
    AuthoredShipSlotId, ClaimOutcome, FrozenShipSlots, HullOutcome, ShipSlotReservations,
};
use project_phoenix::world::server::{ObjectiveInstanceManagerRes, ObjectiveManagerRes};

#[test]
fn competing_slot_picks_freeze_into_actual_launch_and_convoy_report() {
    let source = std::fs::read_to_string("assets/worlds/alliance_convoy_escort.toml").unwrap();
    let config = project_phoenix::world::config::parse_world(&source).unwrap();
    // Both arrival orders use the same public reservation law as the lobby.
    // The loser selects another slot, confirms a different hull, then launches.
    for (winner, loser) in [("host-a", "host-b"), ("host-b", "host-a")] {
        let mut reservations = ShipSlotReservations::default();
        assert_eq!(
            reservations.claim(&config.ship_slots, "lead", winner),
            ClaimOutcome::Claimed
        );
        assert_eq!(
            reservations.claim(&config.ship_slots, "lead", loser),
            ClaimOutcome::Occupied
        );
        assert_eq!(
            reservations.confirm_hull(
                &config.ship_slots,
                "lead",
                loser,
                "assets/entities/alliance_destroyer.toml"
            ),
            HullOutcome::NotClaimant
        );
        assert!(reservations.freeze(&config.ship_slots).is_none());
        assert_eq!(
            reservations.confirm_hull(
                &config.ship_slots,
                "lead",
                winner,
                "assets/entities/alliance_cruiser.toml"
            ),
            HullOutcome::Confirmed
        );
        assert_eq!(
            reservations.claim(&config.ship_slots, "port", loser),
            ClaimOutcome::Claimed
        );
        assert_eq!(
            reservations.confirm_hull(
                &config.ship_slots,
                "port",
                loser,
                "assets/entities/alliance_destroyer.toml"
            ),
            HullOutcome::Confirmed
        );
        let frozen = reservations.freeze(&config.ship_slots).unwrap();
        assert_eq!(frozen.0.len(), 2); // starboard/rear remain absent
        assert_eq!(frozen.0[0].claimant.as_deref(), Some(winner));
        assert_eq!(frozen.0[1].claimant.as_deref(), Some(loser));
        // A post-freeze reservation edit cannot mutate the launch snapshot.
        reservations.release_claimant(winner);
        assert_eq!(frozen.0[0].claimant.as_deref(), Some(winner));

        // Shorten travel only. The scenario's ordinary arrival/loss, scoped
        // Objective and report logic still decides the end; no GM is injected.
        let short = source
            .replace(
                "safe_harbour = [0.0, 0.0, -2400.0]",
                "safe_harbour = [0.0, 0.0, -300.0]",
            )
            .replace("speed = 0.18", "speed = 1.0");
        assert_ne!(short, source, "authored lane fixture must still match");
        let path = std::env::temp_dir().join(format!(
            "phoenix_t5_journey_{}_{}.toml",
            std::process::id(),
            winner
        ));
        std::fs::write(&path, short).unwrap();
        let args = HeadlessArgs {
            world_path: path.to_string_lossy().into_owned(),
            ship_path: "assets/entities/alliance_cruiser.toml".into(),
            seed: Some(1553),
            deterministic: true,
            dt: 1.0 / 60.0,
            max_ticks: 5_400,
            ..Default::default()
        };
        let mut app = build_headless_app(&args).unwrap();
        app.insert_resource(frozen.clone());
        app.insert_resource(project_phoenix::lockstep::FleetRoster::new(
            frozen
                .0
                .iter()
                .enumerate()
                .map(|(index, slot)| project_phoenix::lockstep::FleetShip {
                    host: project_phoenix::command_admission::HostSlot(index as u32 + 1),
                    authored_slot_id: Some(slot.slot_id.clone()),
                    ship_path: Some(slot.hull.clone()),
                    crew: vec![],
                })
                .collect(),
            project_phoenix::command_admission::HostSlot(1),
        ));
        run(&mut app, 120);
        assert_eq!(
            *app.world().resource::<State<GamePhase>>().get(),
            GamePhase::InProgress
        );
        let mut ships: Vec<_> = app
            .world_mut()
            .query::<(
                &AuthoredShipSlotId,
                &project_phoenix::entities::spawner::EntityUuid,
                &project_phoenix::entities::spawner::EntityTemplatePath,
            )>()
            .iter(app.world())
            .map(|(slot, uuid, template)| (slot.0.clone(), uuid.0.clone(), template.0.clone()))
            .collect();
        ships.sort();
        assert_eq!(
            ships
                .iter()
                .map(|(slot, _, _)| slot.as_str())
                .collect::<Vec<_>>(),
            ["lead", "port"]
        );
        assert_eq!(
            ships
                .iter()
                .map(|(slot, _, hull)| (slot.as_str(), hull.as_str()))
                .collect::<Vec<_>>(),
            [
                ("lead", "assets/entities/alliance_cruiser.toml"),
                ("port", "assets/entities/alliance_destroyer.toml")
            ]
        );
        let definitions = &app.world().resource::<ObjectiveManagerRes>().0;
        let instances = &app.world().resource::<ObjectiveInstanceManagerRes>().0;
        for (_, uuid, _) in &ships {
            let projected =
                instances.project_snapshots_for_ship(uuid, definitions.sorted_snapshots());
            assert!(projected
                .iter()
                .any(|row| row.id == "escort_convoy::shared_escort"));
        }
        run(&mut app, args.max_ticks);
        assert_eq!(
            *app.world().resource::<State<GamePhase>>().get(),
            GamePhase::GameOver
        );
        let rows = app.world().resource::<MissionReport>().rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "convoy_fate");
        assert!(rows[0]
            .outcome_id
            .starts_with("world.alliance_convoy.report."));
        assert_eq!(app.world().resource::<FrozenShipSlots>(), &frozen);
        std::fs::remove_file(path).unwrap();
    }
}
