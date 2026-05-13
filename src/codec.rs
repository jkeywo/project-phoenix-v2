use crate::messages::{ClientMessage, ServerMessage};

pub trait MessageCodec {
    type Error;
    fn encode_client(&self, msg: &ClientMessage) -> Result<String, Self::Error>;
    fn decode_client(&self, s: &str) -> Result<ClientMessage, Self::Error>;
    fn encode_server(&self, msg: &ServerMessage) -> Result<String, Self::Error>;
    fn decode_server(&self, s: &str) -> Result<ServerMessage, Self::Error>;
}

pub struct JsonCodec;

impl MessageCodec for JsonCodec {
    type Error = serde_json::Error;

    fn encode_client(&self, msg: &ClientMessage) -> Result<String, Self::Error> {
        serde_json::to_string(msg)
    }

    fn decode_client(&self, s: &str) -> Result<ClientMessage, Self::Error> {
        serde_json::from_str(s)
    }

    fn encode_server(&self, msg: &ServerMessage) -> Result<String, Self::Error> {
        serde_json::to_string(msg)
    }

    fn decode_server(&self, s: &str) -> Result<ServerMessage, Self::Error> {
        serde_json::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::*;

    struct PrettyJsonCodec;

    impl MessageCodec for PrettyJsonCodec {
        type Error = serde_json::Error;
        fn encode_client(&self, msg: &ClientMessage) -> Result<String, Self::Error> { serde_json::to_string_pretty(msg) }
        fn decode_client(&self, s: &str) -> Result<ClientMessage, Self::Error> { serde_json::from_str(s) }
        fn encode_server(&self, msg: &ServerMessage) -> Result<String, Self::Error> { serde_json::to_string_pretty(msg) }
        fn decode_server(&self, s: &str) -> Result<ServerMessage, Self::Error> { serde_json::from_str(s) }
    }

    fn assert_client_roundtrip<C: MessageCodec>(codec: &C, msg: ClientMessage)
    where C::Error: std::fmt::Debug {
        let encoded = codec.encode_client(&msg).unwrap();
        let decoded = codec.decode_client(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    fn assert_server_roundtrip<C: MessageCodec>(codec: &C, msg: ServerMessage)
    where C::Error: std::fmt::Debug {
        let encoded = codec.encode_server(&msg).unwrap();
        let decoded = codec.decode_server(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    fn player() -> Player {
        Player { token: "tok".into(), name: "Alice".into(), consoles: vec![], connected: true }
    }

    fn state() -> GameState {
        GameState { phase: GamePhase::Lobby, players: vec![player()], world: None }
    }

    fn empty_ship_stations() -> crate::stations::ShipStations {
        crate::stations::ShipStations::default()
    }

    // ClientMessage round-trips

    #[test]
    fn client_identify() {
        let msg = ClientMessage::Identify { token: "t".into(), name: "Bob".into() };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_name() {
        let msg = ClientMessage::SetName { name: "Carol".into() };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_start_game() {
        let msg = ClientMessage::StartGame;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_toggle_red_alert() {
        let msg = ClientMessage::ToggleRedAlert;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_fore() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Fore) };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_aft() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Aft) };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_port() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Port) };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_starboard() {
        let msg = ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Starboard) };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_radar() {
        let msg = ClientMessage::SetView { mode: ViewMode::Radar };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_science_radar() {
        let msg = ClientMessage::SetView { mode: ViewMode::ScienceRadar };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_system_chart() {
        let msg = ClientMessage::SetView { mode: ViewMode::SystemChart };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_helm_input() {
        let msg = ClientMessage::HelmInput { thrust: 0.75, steering: -0.5 };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_start_impulse_charge() {
        let msg = ClientMessage::StartImpulseCharge;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_cancel_impulse() {
        let msg = ClientMessage::CancelImpulse;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    // ServerMessage round-trips

    #[test]
    fn server_welcome() {
        let msg = ServerMessage::Welcome { state: state(), ship_stations: empty_ship_stations() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_player_joined() {
        let msg = ServerMessage::PlayerJoined { player: player() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_player_left() {
        let msg = ServerMessage::PlayerLeft { token: "tok".into() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_name_changed() {
        let msg = ServerMessage::NameChanged { token: "tok".into(), name: "Dave".into() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_game_started() {
        let msg = ServerMessage::GameStarted;
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_sim_state() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: true,
                view_mode: ViewMode::Camera(ViewDirection::Fore),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                hull_integrity: 100,
                authorized_repair_console: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn sim_snapshot_view_mode_starboard() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::Camera(ViewDirection::Starboard),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                hull_integrity: 100,
                authorized_repair_console: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn sim_snapshot_view_mode_radar() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::Radar,
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                hull_integrity: 100,
                authorized_repair_console: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn sim_snapshot_carries_ship_position_and_yaw() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 12.5,
                ship_z: -8.25,
                ship_yaw: 1.5707,
                hull_integrity: 100,
                authorized_repair_console: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn sim_snapshot_carries_hull_integrity() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                hull_integrity: 75,
                authorized_repair_console: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn sim_snapshot_carries_authorized_repair_console() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                hull_integrity: 75,
                authorized_repair_console: Some(Console::Repair),
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn asteroid_info_with_uuid_round_trips_in_world_setup() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                asteroids: vec![
                    AsteroidInfo {
                        uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
                        x: 12.5,
                        z: -8.0,
                        radius: 2.0,
                        tags: vec![],
                    },
                ],
                asteroid_fields: vec![],
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn world_data_with_asteroids_round_trips_inside_world_setup() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                asteroids: vec![
                    AsteroidInfo { uuid: "a1b2c3d4-e5f6-4789-8abc-def012345678".into(), x: 1.0, z: 2.0, radius: 2.0, tags: vec![] },
                    AsteroidInfo { uuid: "b2c3d4e5-f6a7-4890-9bcd-ef0123456789".into(), x: -3.5, z: 4.25, radius: 1.5, tags: vec![] },
                ],
                asteroid_fields: vec![],
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn welcome_with_world_some_round_trips() {
        let msg = ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![player()],
                world: Some(WorldData {
                    asteroids: vec![AsteroidInfo { uuid: "c3d4e5f6-a7b8-4901-acde-f01234567890".into(), x: 0.0, z: 0.0, radius: 2.0, tags: vec![] }],
                    asteroid_fields: vec![],
                }),
            },
            ship_stations: empty_ship_stations(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_target() {
        let msg = ClientMessage::SetTarget { uuid: "550e8400-e29b-41d4-a716-446655440000".into() };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_target_lock_confirmed() {
        let msg = ServerMessage::TargetLock { uuid: "550e8400-e29b-41d4-a716-446655440000".into(), locked: true };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_target_lock_rejected() {
        let msg = ServerMessage::TargetLock { uuid: "550e8400-e29b-41d4-a716-446655440000".into(), locked: false };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_weapons_update_fire_ready() {
        let msg = ServerMessage::WeaponsUpdate {
            target_uuid: Some("550e8400-e29b-41d4-a716-446655440000".into()),
            fire_ready: true,
            on_cooldown: false,
            torpedo_count: 10,
            fore_port_loaded: true, fore_port_reload_secs: 0.0,
            fore_starboard_loaded: true, fore_starboard_reload_secs: 0.0,
            aft_loaded: true, aft_reload_secs: 0.0,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_weapons_update_no_lock() {
        let msg = ServerMessage::WeaponsUpdate {
            target_uuid: None,
            fire_ready: false,
            on_cooldown: false,
            torpedo_count: 8,
            fore_port_loaded: false, fore_port_reload_secs: 7.5,
            fore_starboard_loaded: true, fore_starboard_reload_secs: 0.0,
            aft_loaded: false, aft_reload_secs: 3.2,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_weapons_update_on_cooldown() {
        let msg = ServerMessage::WeaponsUpdate {
            target_uuid: Some("550e8400-e29b-41d4-a716-446655440000".into()),
            fire_ready: false,
            on_cooldown: true,
            torpedo_count: 0,
            fore_port_loaded: false, fore_port_reload_secs: 10.0,
            fore_starboard_loaded: false, fore_starboard_reload_secs: 5.0,
            aft_loaded: false, aft_reload_secs: 2.0,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_fire_phaser_round_trips() {
        let msg = ClientMessage::FirePhaser;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_beam_started_round_trips() {
        let msg = ServerMessage::BeamStarted { target_uuid: "550e8400-e29b-41d4-a716-446655440000".into() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_beam_ended_round_trips() {
        let msg = ServerMessage::BeamEnded { target_uuid: "550e8400-e29b-41d4-a716-446655440000".into() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_asteroid_destroyed_round_trips() {
        let msg = ServerMessage::AsteroidDestroyed { uuid: "550e8400-e29b-41d4-a716-446655440000".into() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_repair_round_trips() {
        let msg = ClientMessage::Repair { console: crate::messages::Console::Repair };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_repair_with_power_console_round_trips() {
        let msg = ClientMessage::Repair { console: crate::messages::Console::Power };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_repair_state_round_trips() {
        let msg = ServerMessage::RepairState {
            remaining_cooldown_secs: 12.5,
            in_progress: true,
            penalty: false,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_phaser_mode_round_trips() {
        let msg = ClientMessage::SetPhaserMode { mode: crate::messages::PhaserMode::Manual };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_phaser_fired_round_trips() {
        let msg = ServerMessage::PhaserFired {
            bank: crate::messages::PhaserBank::Port,
            target_uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_science_target_round_trips() {
        let msg = ClientMessage::SetScienceTarget { uuid: "entity-uuid-123".into() };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_science_target_suggestion_round_trips() {
        let msg = ServerMessage::ScienceTargetSuggestion { uuid: "entity-uuid-456".into() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_shield_status_round_trips() {
        let msg = ServerMessage::ShieldStatus {
            facings: vec![
                ShieldFacingStatus { label: "Fore".into(), hp: 80, max_hp: 100, online: true, offline_remaining: 0.0 },
                ShieldFacingStatus { label: "Port".into(), hp: 0, max_hp: 100, online: false, offline_remaining: 7.5 },
                ShieldFacingStatus { label: "Aft".into(), hp: 100, max_hp: 100, online: true, offline_remaining: 0.0 },
                ShieldFacingStatus { label: "Starboard".into(), hp: 55, max_hp: 100, online: true, offline_remaining: 0.0 },
            ],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_fire_torpedo_round_trips() {
        let msg = ClientMessage::FireTorpedo {
            tube: crate::messages::TorpedoTube::ForePort,
            target_uuid: Some("550e8400-e29b-41d4-a716-446655440000".into()),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_fire_torpedo_no_target_round_trips() {
        let msg = ClientMessage::FireTorpedo {
            tube: crate::messages::TorpedoTube::Aft,
            target_uuid: None,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_torpedo_launched_round_trips() {
        let msg = ServerMessage::TorpedoLaunched {
            uuid: "torpedo-uuid-1".into(),
            tube: crate::messages::TorpedoTube::ForeStarboard,
            x: 10.5,
            z: -20.0,
            heading: 1.57,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_torpedo_destroyed_round_trips() {
        let msg = ServerMessage::TorpedoDestroyed { uuid: "torpedo-uuid-1".into() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_modifier_added_round_trips() {
        let msg = ServerMessage::ModifierAdded {
            source: crate::messages::ModifierSource::ImpulseDrive,
            slot: crate::messages::ModifierSlot::MaxSpeed,
            bonus: 0.5,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_asteroid_spawned_round_trips() {
        let msg = ServerMessage::AsteroidSpawned {
            uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
            x: 100.0,
            y: 0.0,
            z: -50.0,
            config_path: "assets/entities/asteroid_small.toml".into(),
            max_hp: 30,
            current_hp: 30,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_modifier_added_console_source_round_trips() {
        let msg = ServerMessage::ModifierAdded {
            source: crate::messages::ModifierSource::Console(crate::messages::Console::Science),
            slot: crate::messages::ModifierSlot::RadarRange,
            bonus: 1.0,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_modifier_added_region_source_round_trips() {
        let msg = ServerMessage::ModifierAdded {
            source: crate::messages::ModifierSource::RegionEffect { region_id: "nebula-7".into() },
            slot: crate::messages::ModifierSlot::HullDamageTaken,
            bonus: -0.3,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_modifier_removed_round_trips() {
        let msg = ServerMessage::ModifierRemoved {
            source: crate::messages::ModifierSource::ImpulseDrive,
            slot: crate::messages::ModifierSlot::MaxYawRate,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    // ── New station wire types ────────────────────────────────────────────────

    #[test]
    fn client_select_station_round_trips() {
        let msg = ClientMessage::SelectStation { station: "Captain".into() };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_release_station_round_trips() {
        let msg = ClientMessage::ReleaseStation;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_station_assigned_with_station_round_trips() {
        let msg = ServerMessage::StationAssigned {
            token: "tok".into(),
            station: Some("Captain".into()),
            consoles: vec![Console::CaptainChair],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_station_assigned_spectator_round_trips() {
        let msg = ServerMessage::StationAssigned {
            token: "tok".into(),
            station: None,
            consoles: vec![],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn welcome_with_ship_stations_round_trips() {
        use crate::stations::{ShipStations, StationDef};
        use std::collections::HashMap;
        let mut configs = HashMap::new();
        configs.insert(1u32, vec![
            StationDef {
                name: "Captain".into(),
                description: "The big chair".into(),
                consoles: vec![Console::CaptainChair],
                rank: "Cpt.".into(),
                next: None,
                previous: None,
            },
        ]);
        let ship_stations = ShipStations { configs, min_players: 1, max_players: 1 };
        let msg = ServerMessage::Welcome { state: state(), ship_stations };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn welcome_with_world_none_round_trips() {
        let msg = ServerMessage::Welcome { state: state(), ship_stations: empty_ship_stations() };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg.clone());
        match msg {
            ServerMessage::Welcome { state, .. } => assert!(state.world.is_none()),
            _ => panic!("expected Welcome"),
        }
    }
}
