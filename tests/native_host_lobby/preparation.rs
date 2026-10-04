use super::*;
use project_phoenix::{
    content_ledger, gm_presentation::sound::LiveSoundCatalog, lockstep::FleetRoster,
    sim_rng::SimRng, world_id::WorldIdMint,
};

struct Before {
    entities: Vec<Entity>,
    rng: project_phoenix::sim_rng::SimRngState,
    mint: project_phoenix::world_id::WorldIdMintState,
    sounds: String,
    slots: Option<project_phoenix::ship_slots::FrozenShipSlots>,
    stations: Vec<project_phoenix::lobby::stations_config::StationDef>,
}
impl Before {
    fn capture(app: &mut App) -> Self {
        let source = project_phoenix::gm_presentation::sound::BUNDLED
            .replace("id = \"weapons\"", "id = \"retained-sentinel\"");
        app.insert_resource(LiveSoundCatalog::capture(Some(source)).unwrap().0);
        project_phoenix::sim_rng::install(
            app.world_mut(),
            SimRng::new(987654, project_phoenix::sim_rng::SeedSource::Cli),
        );
        Self {
            entities: app
                .world_mut()
                .query::<Entity>()
                .iter(app.world())
                .collect(),
            rng: app.world().resource::<SimRng>().state(),
            mint: app.world().resource::<WorldIdMint>().state(),
            sounds: format!("{:?}", app.world().resource::<LiveSoundCatalog>().0),
            slots: app
                .world()
                .get_resource::<project_phoenix::ship_slots::FrozenShipSlots>()
                .cloned(),
            stations: app
                .world()
                .resource::<project_phoenix::lobby::stations_config::ShipStations>()
                .stations
                .clone(),
        }
    }
    fn assert_refused(&self, app: &mut App) {
        assert_eq!(
            self.entities,
            app.world_mut()
                .query::<Entity>()
                .iter(app.world())
                .collect::<Vec<_>>()
        );
        assert_eq!(self.rng, app.world().resource::<SimRng>().state());
        assert_eq!(self.mint, app.world().resource::<WorldIdMint>().state());
        assert_eq!(
            self.sounds,
            format!("{:?}", app.world().resource::<LiveSoundCatalog>().0)
        );
        assert_eq!(
            self.slots.as_ref(),
            app.world()
                .get_resource::<project_phoenix::ship_slots::FrozenShipSlots>()
        );
        assert_eq!(
            self.stations,
            app.world()
                .resource::<project_phoenix::lobby::stations_config::ShipStations>()
                .stations
        );
        assert!(!app.world().contains_resource::<WorldConfig>());
        assert!(!app
            .world()
            .contains_resource::<project_phoenix::lobby::server::SelectedShipResource>());
        assert!(!app
            .world()
            .contains_resource::<project_phoenix::ship_plugin::PendingShipConfig>());
        assert!(!app
            .world()
            .contains_resource::<project_phoenix::gm_projection::GameMasterPeer>());
        assert!(!content_ledger::is_frozen());
        assert!(content_ledger::snapshot().is_empty());
        assert_eq!(
            app.world().resource::<LobbySelection>().0,
            scenario_arbiter::ScenarioSelection::default()
        );
    }
}

#[test]
fn stale_catalogue_slot_refuses_before_install_and_allows_retry() {
    let preload = preload();
    let mut cfg = lobby_config();
    // The published slot is stale: the current file has four differently named slots.
    cfg.catalog = catalog_plus(
        "stale-slots",
        "assets/worlds/cruiser_elimination.toml",
        &[
            "assets/entities/alliance_cruiser.toml".into(),
            "assets/entities/dynasty_player_cruiser.toml".into(),
        ],
    );
    let mut app = build_native_host_app(&cfg, &preload).unwrap();
    pump(&mut app, 4);
    let before = Before::capture(&mut app);
    select(
        &mut app,
        "host-a",
        "stale-slots",
        "assets/entities/alliance_cruiser.toml",
    );
    // Exercise Admission and native loading in one production fixed pass, without
    // advancing the separate clock/mint boundary around this attempted load.
    app.world_mut().run_schedule(FixedUpdate);
    before.assert_refused(&mut app);
    let (id, hull) = pick();
    select(&mut app, "host-b", &id, &hull);
    pump(&mut app, 90);
    assert!(app.world().contains_resource::<WorldConfig>());
}

#[test]
fn incompatible_standalone_gm_refuses_before_materialization_and_allows_retry() {
    let preload = preload();
    let mut app = build_native_host_app(&lobby_config(), &preload).unwrap();
    let mut role = NativeSessionRoleState::default();
    assert!(role.request(NativeSessionRole::StandaloneGameMaster));
    app.insert_resource(role);
    pump(&mut app, 4);
    let incompatible = FleetRoster::solo_game_master("existing-gm").unwrap();
    app.insert_resource(incompatible.clone());
    let before = Before::capture(&mut app);
    app.world_mut().write_message(InboundMessage {
        token: "gm".into(),
        msg: ClientMessage::SelectScenario {
            scenario_id: pick().0,
        },
    });
    app.world_mut().run_schedule(FixedUpdate);
    before.assert_refused(&mut app);
    assert_eq!(app.world().resource::<FleetRoster>(), &incompatible);
    assert!(!app.world().resource::<NativeSessionRoleState>().committed());
    app.insert_resource(FleetRoster::default());
    let gm_roster_before = app
        .world()
        .resource::<project_phoenix::gm_roster::GmRoster>()
        .clone();
    app.add_systems(
        RuntimeWorldLoad,
        (|world: &mut World| {
            assert_eq!(
                world.resource::<FleetRoster>(),
                &FleetRoster::default(),
                "materialization leaves prepared fleet input intact"
            );
            assert_eq!(
                world.resource::<project_phoenix::gm_roster::GmRoster>(),
                &project_phoenix::gm_roster::GmRoster::default(),
                "materialization leaves prepared operator input intact"
            );
            assert!(!world.contains_resource::<project_phoenix::lockstep::FleetLockstep>());
        })
        .after(project_phoenix::world::materialization::WorldMaterialization),
    );
    app.world_mut().write_message(InboundMessage {
        token: "gm".into(),
        msg: ClientMessage::SelectScenario {
            scenario_id: pick().0,
        },
    });
    pump(&mut app, 90);
    assert!(app.world().contains_resource::<WorldConfig>());
    assert_eq!(
        app.world()
            .resource::<FleetRoster>()
            .gm_operator(project_phoenix::command_admission::HostSlot::SOLO),
        Some("gm-1")
    );
    // Materialization does not change the inputs captured for the prepared binding.
    assert!(gm_roster_before.operators().iter().all(|old| app
        .world()
        .resource::<project_phoenix::gm_roster::GmRoster>()
        .operators()
        .iter()
        .any(|row| row.id == old.id)));
    assert!(app.world().resource::<NativeSessionRoleState>().committed());
}
