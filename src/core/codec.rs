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

// ── HTML console bridge (de)serialisation (ADR-0001 / PRD #419) ────────────
//
// These are the sanctioned `serde_json` surface for the HTML bridge: the
// viewscreen HUD push, the generic console-state push, and the inbound
// `__sendAction` decode. Bridge / plugin code must call these, never
// `serde_json` directly.

/// Encode a `ViewscreenHudState` to JSON for the HTML viewscreen overlay.
pub fn encode_hud_state(
    s: &crate::messages::ViewscreenHudState,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(s)
}

/// Encode any serialisable console-state struct to JSON for `__updateConsole`.
pub fn encode_console_state<T: serde::Serialize>(s: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string(s)
}

/// Decode a `window.__sendAction` envelope into a typed `UiAction`. The
/// envelope's extra `console` field is ignored by serde.
pub fn decode_ui_action(s: &str) -> Result<crate::messages::UiAction, serde_json::Error> {
    serde_json::from_str(s)
}

/// Encode a `LobbyStatePayload` to JSON for the HTML lobby overlay.
pub fn encode_lobby_state(
    s: &crate::messages::LobbyStatePayload,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::*;
    use std::collections::HashMap;

    struct PrettyJsonCodec;

    impl MessageCodec for PrettyJsonCodec {
        type Error = serde_json::Error;
        fn encode_client(&self, msg: &ClientMessage) -> Result<String, Self::Error> {
            serde_json::to_string_pretty(msg)
        }
        fn decode_client(&self, s: &str) -> Result<ClientMessage, Self::Error> {
            serde_json::from_str(s)
        }
        fn encode_server(&self, msg: &ServerMessage) -> Result<String, Self::Error> {
            serde_json::to_string_pretty(msg)
        }
        fn decode_server(&self, s: &str) -> Result<ServerMessage, Self::Error> {
            serde_json::from_str(s)
        }
    }

    fn assert_client_roundtrip<C: MessageCodec>(codec: &C, msg: ClientMessage)
    where
        C::Error: std::fmt::Debug,
    {
        let encoded = codec.encode_client(&msg).unwrap();
        let decoded = codec.decode_client(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    fn assert_server_roundtrip<C: MessageCodec>(codec: &C, msg: ServerMessage)
    where
        C::Error: std::fmt::Debug,
    {
        let encoded = codec.encode_server(&msg).unwrap();
        let decoded = codec.decode_server(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    fn player() -> Player {
        Player {
            token: "tok".into(),
            name: "Alice".into(),
            consoles: vec![],
            connected: true,
        }
    }

    fn state() -> GameState {
        GameState {
            phase: GamePhase::Lobby,
            players: vec![player()],
            complexity: HashMap::new(),
            world: None,
        }
    }

    fn empty_ship_stations() -> crate::stations_config::ShipStations {
        crate::stations_config::ShipStations::default()
    }

    // ClientMessage round-trips

    #[test]
    fn client_identify() {
        let msg = ClientMessage::Identify {
            token: "t".into(),
            name: "Bob".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_name() {
        let msg = ClientMessage::SetName {
            name: "Carol".into(),
        };
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
        let msg = ClientMessage::SetView {
            mode: ViewMode::Camera(ViewDirection::Fore),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_aft() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::Camera(ViewDirection::Aft),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_port() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::Camera(ViewDirection::Port),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_starboard() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::Camera(ViewDirection::Starboard),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_radar() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::Radar,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_science_radar() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::ScienceRadar,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_system_chart() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::SystemChart,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_navigation_chart() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::NavigationChart,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_comms() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::Comms,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_helm_input() {
        let msg = ClientMessage::HelmInput {
            thrust: 0.75,
            steering: -0.5,
        };
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

    #[test]
    fn client_set_navigation_waypoint() {
        let msg = ClientMessage::SetNavigationWaypoint { x: 120.0, z: -45.0 };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_clear_navigation_waypoint() {
        let msg = ClientMessage::ClearNavigationWaypoint;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    // ServerMessage round-trips

    #[test]
    fn server_welcome() {
        let msg = ServerMessage::Welcome {
            state: state(),
            ship_stations: empty_ship_stations(),
            ship_config: ShipClientConfig::default(),
        };
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
        let msg = ServerMessage::PlayerLeft {
            token: "tok".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_name_changed() {
        let msg = ServerMessage::NameChanged {
            token: "tok".into(),
            name: "Dave".into(),
        };
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
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![],
                radar_state: RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
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
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![],
                radar_state: RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
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
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![],
                radar_state: RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
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
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![],
                radar_state: RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn sim_snapshot_round_trips_without_hull_integrity() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![],
                radar_state: RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn sim_snapshot_with_navigation_waypoint_round_trips() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![],
                radar_state: RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: Some(WaypointSnapshot { x: 12.5, z: -8.0 }),
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn entity_snapshot_as_asteroid_round_trips_in_world_setup() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![EntitySnapshot {
                    uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
                    id: None,
                    name: None,
                    position: Some([12.5, 0.0, -8.0]),
                    tags: vec!["asteroid".into()],
                    shape: None,
                    radius: Some(2.0),
                    colour: None,
                    yaw: None,
                    hull_fraction: None,
                    inner_radius: None,
                    warp_out_remaining_secs: None,
                    radar_world_size: None,
                    half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    }],
                ..Default::default()
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn world_data_with_multiple_entities_round_trips() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![
                    EntitySnapshot {
                        uuid: "a1b2c3d4-e5f6-4789-8abc-def012345678".into(),
                        id: None,
                        name: None,
                        position: Some([1.0, 0.0, 2.0]),
                        tags: vec!["asteroid".into()],
                        shape: None,
                        radius: Some(2.0),
                        colour: None,
                        yaw: None,
                        hull_fraction: None,
                        inner_radius: None,
                        warp_out_remaining_secs: None,
                        radar_world_size: None,
                    half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    },
                    EntitySnapshot {
                        uuid: "b2c3d4e5-f6a7-4890-9bcd-ef0123456789".into(),
                        id: None,
                        name: None,
                        position: Some([-3.5, 0.0, 4.25]),
                        tags: vec!["asteroid".into()],
                        shape: None,
                        radius: Some(1.5),
                        colour: None,
                        yaw: None,
                        hull_fraction: None,
                        inner_radius: None,
                        warp_out_remaining_secs: None,
                        radar_world_size: None,
                    half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    },
                ],
                ..Default::default()
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
                complexity: HashMap::new(),
                world: Some(WorldData {
                    entities: vec![EntitySnapshot {
                        uuid: "c3d4e5f6-a7b8-4901-acde-f01234567890".into(),
                        id: None,
                        name: None,
                        position: Some([0.0, 0.0, 0.0]),
                        tags: vec!["asteroid".into()],
                        shape: None,
                        radius: Some(2.0),
                        colour: None,
                        yaw: None,
                        hull_fraction: None,
                        inner_radius: None,
                        warp_out_remaining_secs: None,
                        radar_world_size: None,
                        half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    }],
                    ..Default::default()
                }),
            },
            ship_stations: empty_ship_stations(),
            ship_config: ShipClientConfig::default(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_target() {
        let msg = ClientMessage::SetTarget {
            uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_target_lock_confirmed() {
        let msg = ServerMessage::TargetLock {
            uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
            locked: true,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_target_lock_rejected() {
        let msg = ServerMessage::TargetLock {
            uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
            locked: false,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_weapons_update_fire_ready() {
        use crate::messages::{PhaserBankState, TorpedoTubeState};
        let msg = ServerMessage::WeaponsUpdate {
            target_uuid: Some("550e8400-e29b-41d4-a716-446655440000".into()),
            target_name: Some("Klingon Raider".into()),
            banks: vec![PhaserBankState {
                id: "port".to_string(),
                fire_ready: true,
                on_cooldown: false,
                cooldown_remaining: 0.0,
            }],
            tubes: vec![
                TorpedoTubeState { id: "fore_port".to_string(), loaded: true, reload_secs: 0.0 },
                TorpedoTubeState { id: "fore_starboard".to_string(), loaded: true, reload_secs: 0.0 },
                TorpedoTubeState { id: "aft".to_string(), loaded: true, reload_secs: 0.0 },
            ],
            torpedo_count: 10,
            phaser_mode: crate::messages::PhaserMode::Auto,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_weapons_update_no_lock() {
        use crate::messages::{PhaserBankState, TorpedoTubeState};
        let msg = ServerMessage::WeaponsUpdate {
            target_uuid: None,
            target_name: None,
            banks: vec![PhaserBankState {
                id: "port".to_string(),
                fire_ready: false,
                on_cooldown: false,
                cooldown_remaining: 0.0,
            }],
            tubes: vec![
                TorpedoTubeState { id: "fore_port".to_string(), loaded: false, reload_secs: 7.5 },
                TorpedoTubeState { id: "fore_starboard".to_string(), loaded: true, reload_secs: 0.0 },
                TorpedoTubeState { id: "aft".to_string(), loaded: false, reload_secs: 3.2 },
            ],
            torpedo_count: 8,
            phaser_mode: crate::messages::PhaserMode::Manual,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_weapons_update_on_cooldown() {
        use crate::messages::{PhaserBankState, TorpedoTubeState};
        let msg = ServerMessage::WeaponsUpdate {
            target_uuid: Some("550e8400-e29b-41d4-a716-446655440000".into()),
            target_name: None,
            banks: vec![PhaserBankState {
                id: "port".to_string(),
                fire_ready: false,
                on_cooldown: true,
                cooldown_remaining: 1.5,
            }],
            tubes: vec![
                TorpedoTubeState { id: "fore_port".to_string(), loaded: false, reload_secs: 10.0 },
                TorpedoTubeState { id: "fore_starboard".to_string(), loaded: false, reload_secs: 5.0 },
                TorpedoTubeState { id: "aft".to_string(), loaded: false, reload_secs: 2.0 },
            ],
            torpedo_count: 0,
            phaser_mode: crate::messages::PhaserMode::Manual,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_fire_phaser_round_trips() {
        let msg = ClientMessage::FirePhaser { bank: "port".to_string() };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_beam_started_round_trips() {
        let msg = ServerMessage::BeamStarted {
            bank: "port".to_string(),
            source_uuid: "11111111-1111-1111-1111-111111111111".into(),
            target_uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_beam_ended_round_trips() {
        let msg = ServerMessage::BeamEnded {
            bank: "port".to_string(),
            source_uuid: "11111111-1111-1111-1111-111111111111".into(),
            target_uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_asteroid_destroyed_round_trips() {
        let msg = ServerMessage::AsteroidDestroyed {
            uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_dispatch_repair_team_round_trips() {
        use crate::messages::Console;
        let msg = ClientMessage::DispatchRepairTeam {
            team_idx: 0,
            console: Console::Helm,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_repair_state_round_trips() {
        use crate::messages::{Console, TeamSlot};
        let msg = ServerMessage::RepairState {
            teams: vec![
                TeamSlot::Idle,
                TeamSlot::Travelling {
                    console: Console::Helm,
                    elapsed: 2.5,
                },
                TeamSlot::Repairing {
                    console: Console::Tactical,
                },
                TeamSlot::Returning {
                    remaining: 3.0,
                    queued: None,
                },
            ],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_phaser_mode_round_trips() {
        let msg = ClientMessage::SetPhaserMode {
            mode: crate::messages::PhaserMode::Manual,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_phaser_fired_round_trips() {
        let msg = ServerMessage::PhaserFired {
            bank: "port".to_string(),
            target_uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_science_target_round_trips() {
        let msg = ClientMessage::SetScienceTarget {
            uuid: "entity-uuid-123".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_sensors_target_round_trips() {
        let msg = ClientMessage::SetSensorsTarget {
            uuid: "entity-uuid-sensors-123".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_science_target_suggestion_round_trips() {
        let msg = ServerMessage::ScienceTargetSuggestion {
            uuid: "entity-uuid-456".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_sensors_target_suggestion_round_trips() {
        let msg = ServerMessage::SensorsTargetSuggestion {
            uuid: "entity-uuid-sensors-456".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_view_sensors_radar() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::SensorsRadar,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_shield_status_round_trips() {
        let msg = ServerMessage::ShieldStatus {
            facings: vec![
                ShieldFacingStatus {
                    label: "Fore".into(),
                    hp: 80,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
                ShieldFacingStatus {
                    label: "Port".into(),
                    hp: 0,
                    max_hp: 100,
                    online: false,
                    offline_remaining: 7.5,
                    is_focused: false,
                },
                ShieldFacingStatus {
                    label: "Aft".into(),
                    hp: 100,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
                ShieldFacingStatus {
                    label: "Starboard".into(),
                    hp: 55,
                    max_hp: 100,
                    online: true,
                    offline_remaining: 0.0,
                    is_focused: false,
                },
            ],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_shield_focus_some_round_trips() {
        let msg = ClientMessage::SetShieldFocus {
            facing: Some(ViewDirection::Fore),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_shield_focus_none_round_trips() {
        let msg = ClientMessage::SetShieldFocus { facing: None };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn shield_facing_status_with_focus_round_trips() {
        let msg = ShieldFacingStatus {
            label: "Fore".into(),
            hp: 150,
            max_hp: 150,
            online: true,
            offline_remaining: 0.0,
            is_focused: true,
        };
        let encoded = serde_json::to_string(&msg).unwrap();
        let decoded: ShieldFacingStatus = serde_json::from_str(&encoded).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn client_fire_torpedo_round_trips() {
        let msg = ClientMessage::FireTorpedo {
            tube: "fore_port".to_string(),
            target_uuid: Some("550e8400-e29b-41d4-a716-446655440000".into()),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_fire_torpedo_no_target_round_trips() {
        let msg = ClientMessage::FireTorpedo {
            tube: "aft".to_string(),
            target_uuid: None,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_torpedo_launched_round_trips() {
        let msg = ServerMessage::TorpedoLaunched {
            uuid: "torpedo-uuid-1".into(),
            tube: "fore_starboard".to_string(),
            x: 10.5,
            z: -20.0,
            heading: 1.57,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_torpedo_destroyed_round_trips() {
        let msg = ServerMessage::TorpedoDestroyed {
            uuid: "torpedo-uuid-1".into(),
        };
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
            radius: 2.0,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_modifier_added_console_source_round_trips() {
        let msg = ServerMessage::ModifierAdded {
            source: crate::messages::ModifierSource::Console(crate::messages::Console::Sensors),
            slot: crate::messages::ModifierSlot::RadarRange,
            bonus: 1.0,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_modifier_added_region_source_round_trips() {
        let msg = ServerMessage::ModifierAdded {
            source: crate::messages::ModifierSource::RegionEffect {
                uuid: uuid::Uuid::from_u128(7),
            },
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

    #[test]
    fn server_modifier_world_source_round_trips() {
        // ModifierAdded with World source
        let msg = ServerMessage::ModifierAdded {
            source: crate::messages::ModifierSource::World {
                id: "default".into(),
                tag: "speed_boost".into(),
            },
            slot: crate::messages::ModifierSlot::MaxSpeed,
            bonus: 0.25,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);

        // ModifierRemoved with World source
        let msg = ServerMessage::ModifierRemoved {
            source: crate::messages::ModifierSource::World {
                id: "default".into(),
                tag: "speed_boost".into(),
            },
            slot: crate::messages::ModifierSlot::MaxSpeed,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn modifier_source_world_hash_and_eq() {
        use crate::messages::ModifierSource;
        use std::collections::HashSet;

        let a = ModifierSource::World {
            id: "s1".into(),
            tag: "t1".into(),
        };
        let b = ModifierSource::World {
            id: "s1".into(),
            tag: "t1".into(),
        };
        let c = ModifierSource::World {
            id: "s1".into(),
            tag: "t2".into(),
        };
        let d = ModifierSource::World {
            id: "s2".into(),
            tag: "t1".into(),
        };

        // Same (id, tag) → equal
        assert_eq!(a, b);

        // Different tag → not equal
        assert_ne!(a, c);

        // Different id → not equal
        assert_ne!(a, d);

        // HashSet deduplication: same pair stored once
        let mut set = HashSet::new();
        set.insert(a.clone());
        set.insert(b.clone());
        assert_eq!(set.len(), 1);

        // Different tag stored separately
        set.insert(c);
        assert_eq!(set.len(), 2);

        // Different id stored separately
        set.insert(d);
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn flag_kind_round_trips() {
        for flag in &[
            crate::flag_kind::FlagKind::CommsJammed,
            crate::flag_kind::FlagKind::SensorBlind,
        ] {
            let json = serde_json::to_string(flag).unwrap();
            let decoded: crate::flag_kind::FlagKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*flag, decoded);
        }
    }

    // ── New station wire types ────────────────────────────────────────────────

    #[test]
    fn client_select_station_round_trips() {
        let msg = ClientMessage::SelectStation {
            station: "Captain".into(),
        };
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
        use crate::stations_config::{ShipStations, StationDef};
        use std::collections::HashMap;
        let mut configs = HashMap::new();
        configs.insert(
            1u32,
            vec![StationDef {
                name: "Captain".into(),
                description: "The big chair".into(),
                consoles: vec![Console::CaptainChair],
                rank: "Cpt.".into(),
                short_code: "CAP".into(),
                next: None,
                previous: None,
            }],
        );
        let ship_stations = ShipStations {
            configs,
            min_players: 1,
            max_players: 1,
            complexity_presets: std::collections::HashMap::new(),
        };
        let msg = ServerMessage::Welcome {
            state: state(),
            ship_stations,
            ship_config: ShipClientConfig::default(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn welcome_with_world_none_round_trips() {
        let msg = ServerMessage::Welcome {
            state: state(),
            ship_stations: empty_ship_stations(),
            ship_config: ShipClientConfig::default(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg.clone());
        match msg {
            ServerMessage::Welcome { state, .. } => assert!(state.world.is_none()),
            _ => panic!("expected Welcome"),
        }
    }

    #[test]
    fn welcome_with_custom_repair_timings_round_trips() {
        // Verify the [repair] block fields ride through Welcome intact —
        // changing values in the ship TOML must reach the client byte-equal.
        let ship_config = ShipClientConfig {
            helm_radar_range: 700.0,
            repair_travel_secs: 9.0,
            repair_rate_hp_per_sec: 1.5,
            impulse_charge_duration: 4.5,
            ..ShipClientConfig::default()
        };
        let msg = ServerMessage::Welcome {
            state: state(),
            ship_stations: empty_ship_stations(),
            ship_config,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg.clone());
        match msg {
            ServerMessage::Welcome { ship_config, .. } => {
                assert_eq!(ship_config.repair_travel_secs, 9.0);
                assert_eq!(ship_config.repair_rate_hp_per_sec, 1.5);
                assert_eq!(ship_config.helm_radar_range, 700.0);
                assert_eq!(ship_config.impulse_charge_duration, 4.5);
            }
            _ => panic!("expected Welcome"),
        }
    }

    #[test]
    fn client_increase_power_round_trips() {
        let msg = ClientMessage::IncreasePower {
            console: Console::Helm,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_decrease_power_round_trips() {
        let msg = ClientMessage::DecreasePower {
            console: Console::Sensors,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_sensors_console_round_trips() {
        let msg = ClientMessage::SetComplexity {
            console: Console::Sensors,
            preset_name: "Low".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_shields_console_round_trips() {
        let msg = ClientMessage::SetComplexity {
            console: Console::Shields,
            preset_name: "Std".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_navigation_console_round_trips() {
        let msg = ClientMessage::SetComplexity {
            console: Console::Navigation,
            preset_name: "Std".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_power_state_round_trips() {
        let msg = ServerMessage::PowerState {
            helm: 3,
            weapons: 2,
            sensors: 4,
            battery_charge: 65.5,
            locked: false,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_power_state_locked_round_trips() {
        let msg = ServerMessage::PowerState {
            helm: 1,
            weapons: 1,
            sensors: 1,
            battery_charge: 0.0,
            locked: true,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn sim_snapshot_with_power_levels_round_trips() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: true,
                view_mode: ViewMode::Radar,
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                forward_speed: 0.0,
                power_levels: (4, 2, 1),
                flags: vec![],
                entity_states: vec![],
                radar_state: RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn sim_snapshot_with_default_power_levels_serializes_correctly() {
        let with_defaults = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![],
                radar_state: RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
            },
        };
        // Encoding then decoding should preserve (2, 2, 2) for power_levels.
        let encoded = serde_json::to_string(&with_defaults).unwrap();
        let decoded: ServerMessage = serde_json::from_str(&encoded).unwrap();
        assert_eq!(with_defaults, decoded);
    }

    // ── EntitySnapshot / EntityState / RadarState tests ──────────────────

    #[test]
    fn entity_snapshot_minimal_round_trips() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![EntitySnapshot {
                    uuid: "u1".into(),
                    id: None,
                    name: None,
                    position: None,
                    tags: vec!["asteroid".into()],
                    shape: None,
                    radius: None,
                    colour: None,
                    yaw: None,
                    hull_fraction: None,
                    inner_radius: None,
                    warp_out_remaining_secs: None,
                    radar_world_size: None,
                    half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    }],
                ..Default::default()
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn entity_snapshot_full_fields_round_trips() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![EntitySnapshot {
                    uuid: "u1".into(),
                    id: Some("station-alpha".into()),
                    name: Some("Station Alpha".into()),
                    position: Some([10.5, 0.0, -20.3]),
                    tags: vec!["station".into(), "ship".into()],
                    shape: Some("sphere".into()),
                    radius: Some(5.0),
                    colour: Some([0.2, 0.5, 0.8]),
                    yaw: Some(1.57),
                    hull_fraction: Some(0.85),
                    inner_radius: Some(2.0),
                    warp_out_remaining_secs: None,
                    radar_world_size: None,
                    half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    }],
                ..Default::default()
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn entity_snapshot_warp_out_remaining_secs_round_trips() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![EntitySnapshot {
                    uuid: "npc-1".into(),
                    id: None,
                    name: None,
                    position: Some([5.0, 0.0, 10.0]),
                    tags: vec![],
                    shape: None,
                    radius: None,
                    colour: None,
                    yaw: None,
                    hull_fraction: None,
                    inner_radius: None,
                    warp_out_remaining_secs: Some(3.5),
                    radar_world_size: None,
                    half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    }],
                ..Default::default()
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn entity_snapshot_radar_world_size_round_trips() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![EntitySnapshot {
                    uuid: "u-radar".into(),
                    id: None,
                    name: None,
                    position: Some([1.0, 0.0, 2.0]),
                    tags: vec!["ship".into()],
                    shape: None,
                    radius: Some(3.0),
                    colour: None,
                    yaw: None,
                    hull_fraction: None,
                    inner_radius: None,
                    warp_out_remaining_secs: None,
                    radar_world_size: Some(12.5),
                    half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    }],
                ..Default::default()
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn entity_snapshot_radar_world_size_none_is_omitted_from_json() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![EntitySnapshot {
                    uuid: "u-noradar".into(),
                    id: None,
                    name: None,
                    position: Some([0.0, 0.0, 0.0]),
                    tags: vec![],
                    shape: None,
                    radius: Some(1.0),
                    colour: None,
                    yaw: None,
                    hull_fraction: None,
                    inner_radius: None,
                    warp_out_remaining_secs: None,
                    radar_world_size: None,
                    half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    }],
                ..Default::default()
            },
        };
        let encoded = JsonCodec.encode_server(&msg).expect("encode");
        assert!(
            !encoded.contains("radar_world_size"),
            "None radar_world_size must be omitted from JSON, got: {}",
            encoded
        );
    }

    #[test]
    fn entity_snapshot_with_multiple_entities_round_trips() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![
                    EntitySnapshot {
                        uuid: "ast-1".into(),
                        id: None,
                        name: None,
                        position: Some([0.0, 0.0, -25.0]),
                        tags: vec!["asteroid".into()],
                        shape: None,
                        radius: Some(2.0),
                        colour: None,
                        yaw: None,
                        hull_fraction: None,
                        inner_radius: None,
                        warp_out_remaining_secs: None,
                        radar_world_size: None,
                    half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    },
                    EntitySnapshot {
                        uuid: "field-1".into(),
                        id: None,
                        name: None,
                        position: Some([50.0, 0.0, -100.0]),
                        tags: vec!["asteroid_field".into()],
                        shape: None,
                        radius: Some(30.0),
                        colour: None,
                        yaw: None,
                        hull_fraction: None,
                        inner_radius: Some(10.0),
                        warp_out_remaining_secs: None,
                        radar_world_size: None,
                    half_extents: None,
                    radar_icon: None,
                    objective_target: false,
                    target_tags: Vec::new(),
                    threat_level: None,
                    target_description: None,
                    },
                ],
                ..Default::default()
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn entity_state_snapshot_minimal_round_trips() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![EntityStateSnapshot {
                    uuid: "ast-1".into(),
                    position: Some([12.0, 0.0, -5.0]),
                    yaw: Some(0.5),
                    hull_fraction: Some(1.0),
                    flags: vec![],
                    shields: None,
                    warp_out_remaining_secs: None,
                }],
                radar_state: RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn entity_state_snapshot_minimal_fields_round_trips() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![EntityStateSnapshot {
                    uuid: "ast-2".into(),
                    position: None,
                    yaw: None,
                    hull_fraction: None,
                    flags: vec![],
                    shields: None,
                    warp_out_remaining_secs: None,
                }],
                radar_state: RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn radar_state_snapshot_custom_values_round_trips() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![],
                radar_state: RadarStateSnapshot {
                    helm_range: 40.0,
                    tactical_range: 55.0,
                    science_long_range: 180.0,
                    science_system_map: 600.0,
                },
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn sim_snapshot_with_multiple_entity_states_round_trips() {
        let msg = ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: true,
                view_mode: ViewMode::Radar,
                ship_x: 10.0,
                ship_z: -20.0,
                ship_yaw: 1.0,
                forward_speed: 0.0,
                power_levels: (3, 2, 1),
                flags: vec![crate::flag_kind::FlagKind::SensorBlind],
                entity_states: vec![
                    EntityStateSnapshot {
                        uuid: "e1".into(),
                        position: Some([5.0, 0.0, -10.0]),
                        yaw: Some(0.0),
                        hull_fraction: Some(1.0),
                        flags: vec![],
                        shields: None,
                        warp_out_remaining_secs: None,
                    },
                    EntityStateSnapshot {
                        uuid: "e2".into(),
                        position: Some([20.0, 0.0, -30.0]),
                        yaw: None,
                        hull_fraction: Some(0.5),
                        flags: vec![crate::flag_kind::FlagKind::CommsJammed],
                        shields: None,
                        warp_out_remaining_secs: None,
                    },
                ],
                radar_state: RadarStateSnapshot {
                    helm_range: 50.0,
                    tactical_range: 60.0,
                    science_long_range: 200.0,
                    science_system_map: 500.0,
                },
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn entity_tag_round_trips() {
        for tag in &[
            EntityTag::Asteroid,
            EntityTag::Ship,
            EntityTag::AsteroidField,
            EntityTag::Star,
            EntityTag::Planet,
            EntityTag::Region,
        ] {
            let json = serde_json::to_string(tag).unwrap();
            let decoded: EntityTag = serde_json::from_str(&json).unwrap();
            assert_eq!(*tag, decoded);
        }
    }

    // ── SetComplexity / ComplexityChanged round-trips ──────────────────

    #[test]
    fn client_set_complexity_round_trips() {
        let msg = ClientMessage::SetComplexity {
            console: Console::Helm,
            preset_name: "Low".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_complexity_changed_round_trips() {
        let msg = ServerMessage::ComplexityChanged {
            console: Console::Tactical,
            preset_name: "Std".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn set_complexity_with_captain_chair_console_round_trips() {
        let msg = ClientMessage::SetComplexity {
            console: Console::CaptainChair,
            preset_name: "Std".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    // ── EntitySpawned / EntityDespawned round-trips ──────────────────────

    #[test]
    fn server_entity_spawned_round_trips() {
        let msg = ServerMessage::EntitySpawned {
            snapshot: EntitySnapshot {
                uuid: "run-entity-001".into(),
                id: Some("station-alpha".into()),
                name: None,
                position: Some([100.0, 0.0, -200.0]),
                tags: vec!["station".into(), "ship".into()],
                shape: Some("sphere".into()),
                radius: Some(5.0),
                colour: Some([0.2, 0.5, 0.8]),
                yaw: Some(1.57),
                hull_fraction: Some(0.85),
                inner_radius: Some(2.0),
                warp_out_remaining_secs: None,
                radar_world_size: None,
                half_extents: None,
            radar_icon: None,
            objective_target: false,
            target_tags: Vec::new(),
            threat_level: None,
            target_description: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_entity_spawned_minimal_round_trips() {
        let msg = ServerMessage::EntitySpawned {
            snapshot: EntitySnapshot {
                uuid: "run-minimal".into(),
                id: None,
                name: None,
                position: Some([10.0, 0.0, 20.0]),
                tags: vec![],
                shape: None,
                radius: None,
                colour: None,
                yaw: None,
                hull_fraction: None,
                inner_radius: None,
                warp_out_remaining_secs: None,
                radar_world_size: None,
                half_extents: None,
            radar_icon: None,
            objective_target: false,
            target_tags: Vec::new(),
            threat_level: None,
            target_description: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_entity_despawned_round_trips() {
        let msg = ServerMessage::EntityDespawned {
            uuid: "run-entity-001".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    // ── SetPhaserFrequency round-trip ────────────────────────────────────

    #[test]
    fn client_set_phaser_frequency_round_trips() {
        let msg = ClientMessage::SetPhaserFrequency { frequency: 0.75 };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_phaser_frequency_zero_round_trips() {
        let msg = ClientMessage::SetPhaserFrequency { frequency: 0.0 };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_set_phaser_frequency_one_round_trips() {
        let msg = ClientMessage::SetPhaserFrequency { frequency: 1.0 };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    // ── FrequencyHint round-trip ──────────────────────────────────────────

    #[test]
    fn server_frequency_hint_round_trips() {
        let msg = ServerMessage::FrequencyHint { frequency: 0.33 };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_frequency_hint_boundary_values_round_trip() {
        for f in [0.0f32, 1.0f32] {
            let msg = ServerMessage::FrequencyHint { frequency: f };
            assert_server_roundtrip(&JsonCodec, msg.clone());
            assert_server_roundtrip(&PrettyJsonCodec, msg);
        }
    }

    // ── StationSpawned / StationDestroyed round-trips ─────────────────────

    #[test]
    fn server_station_spawned_round_trips() {
        let msg = ServerMessage::StationSpawned {
            uuid: "station-1".into(),
            name: "Deep Space 9".into(),
            position: [100.0, 0.0, -50.0],
            shape: "cylinder".into(),
            radius: 15.0,
            hull_integrity: 200.0,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_station_destroyed_round_trips() {
        let msg = ServerMessage::StationDestroyed {
            uuid: "station-1".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_station_spawned_sphere_and_torus_shapes_round_trip() {
        for shape in ["sphere", "torus"] {
            let msg = ServerMessage::StationSpawned {
                uuid: "s".into(),
                name: "Test".into(),
                position: [0.0, 0.0, 0.0],
                shape: shape.into(),
                radius: 10.0,
                hull_integrity: 100.0,
            };
            assert_server_roundtrip(&JsonCodec, msg.clone());
        }
    }

    // ── ObjectiveSummary round-trip ────────────────────────────────────────

    #[test]
    fn server_objective_summary_round_trips() {
        let msg = ServerMessage::ObjectiveSummary {
            objectives: vec![
                crate::messages::ObjectiveSnapshot {
                    id: "obj-1".into(),
                    text: "Destroy the convoy".into(),
                    mandatory: true,
                    status: crate::messages::ObjectiveStatus::Active,
                    targets: vec!["Axiom Station".into(), "Research Outpost".into()],
                },
                crate::messages::ObjectiveSnapshot {
                    id: "obj-2".into(),
                    text: "Scan the debris".into(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Completed,
                    targets: vec![],
                },
                crate::messages::ObjectiveSnapshot {
                    id: "obj-2".into(),
                    text: "Scan the debris".into(),
                    mandatory: false,
                    status: crate::messages::ObjectiveStatus::Completed,
                    targets: vec![],
                },
            ],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_objective_summary_empty_objectives_round_trips() {
        let msg = ServerMessage::ObjectiveSummary { objectives: vec![] };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_objective_summary_failed_status_round_trips() {
        let msg = ServerMessage::ObjectiveSummary {
            objectives: vec![crate::messages::ObjectiveSnapshot {
                id: "obj-fail".into(),
                text: "Save the station".into(),
                mandatory: true,
                status: crate::messages::ObjectiveStatus::Failed,
                targets: vec![],
            }],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
    }

    // ── Comms wire round-trips ─────────────────────────────────────────────

    #[test]
    fn client_hail_round_trips() {
        let msg = ClientMessage::Hail {
            target_uuid: "station-uuid-123".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_select_comms_message_round_trips() {
        let msg = ClientMessage::SelectCommsMessage {
            message_id: "msg-1".into(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_respond_to_message_round_trips() {
        let msg = ClientMessage::RespondToMessage {
            message_id: "msg-1".into(),
            response_index: 2,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_clear_comms_round_trips() {
        let msg = ClientMessage::ClearComms;
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn client_show_on_screen_round_trips() {
        let msg = ClientMessage::ShowOnScreen {
            message_id: "msg-abc-123".to_string(),
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_comms_state_empty_round_trips() {
        let msg = ServerMessage::CommsState {
            messages: vec![],
            objectives: vec![],
            contacts: vec![],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_comms_state_with_message_round_trips() {
        let msg = ServerMessage::CommsState {
            messages: vec![crate::messages::CommsMessage {
                id: "m1".into(),
                sender_uuid: "station-abc".into(),
                sender_name: "Starbase 12".into(),
                subject: "Greetings".into(),
                body: "Welcome to the sector.".into(),
                responses: vec!["Acknowledged".into(), "Request docking".into()],
                selected_response: Some(0),
                is_read: false,
                is_orphaned: false,
                sender_in_range: true,
                thread_id: "thread-001".into(),
                is_urgent: false,
            }],
            objectives: vec![],
            contacts: vec![crate::messages::CommsContact {
                uuid: "station-abc".into(),
                name: "Starbase 12".into(),
                in_range: true,
                is_urgent: false,
            }],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_comms_state_with_orphaned_message_round_trips() {
        let msg = ServerMessage::CommsState {
            messages: vec![crate::messages::CommsMessage {
                id: "m2".into(),
                sender_uuid: "convoy-uuid".into(),
                sender_name: "Convoy".into(),
                subject: "Distress".into(),
                body: "We are under attack!".into(),
                responses: vec![],
                selected_response: None,
                is_read: false,
                is_orphaned: true,
                sender_in_range: true,
                thread_id: "thread-002".into(),
                is_urgent: false,
            }],
            objectives: vec![],
            contacts: vec![],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn server_comms_state_with_range_flags_round_trips() {
        let msg = ServerMessage::CommsState {
            messages: vec![crate::messages::CommsMessage {
                id: "m3".into(),
                sender_uuid: "raider-1".into(),
                sender_name: "Raider".into(),
                subject: "Surrender".into(),
                body: "...".into(),
                responses: vec!["No".into()],
                selected_response: None,
                is_read: false,
                is_orphaned: false,
                sender_in_range: false,
                thread_id: "thread-003".into(),
                is_urgent: false,
            }],
            objectives: vec![],
            contacts: vec![crate::messages::CommsContact {
                uuid: "raider-1".into(),
                name: "Raider".into(),
                in_range: false,
                is_urgent: false,
            }],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn comms_contact_missing_in_range_defaults_to_true() {
        let json = r#"{"uuid":"x","name":"X"}"#;
        let contact: crate::messages::CommsContact = serde_json::from_str(json).unwrap();
        assert!(contact.in_range, "in_range should default to true for backward compat");
    }

    #[test]
    fn comms_message_missing_sender_in_range_defaults_to_true() {
        let json = r#"{"id":"m","sender_uuid":"s","sender_name":"S","subject":"x","body":"y","responses":[],"selected_response":null,"is_read":false}"#;
        let msg: crate::messages::CommsMessage = serde_json::from_str(json).unwrap();
        assert!(msg.sender_in_range, "sender_in_range should default to true for backward compat");
    }

    #[test]
    fn comms_message_missing_thread_id_defaults_to_empty() {
        let json = r#"{"id":"m","sender_uuid":"s","sender_name":"S","subject":"x","body":"y","responses":[],"selected_response":null,"is_read":false}"#;
        let msg: crate::messages::CommsMessage = serde_json::from_str(json).unwrap();
        assert!(msg.thread_id.is_empty(), "thread_id should default to empty string for backward compat");
    }

    #[test]
    fn comms_state_payload_with_no_range_flags_defaults_both_to_true() {
        // A pre-feature server payload contains neither `in_range` on contacts
        // nor `sender_in_range` on messages. Both must deserialize as true so
        // older clients/servers interoperate.
        let json = r#"{
            "type":"CommsState",
            "data":{
                "messages":[{"id":"m1","sender_uuid":"s","sender_name":"S","subject":"x","body":"y","responses":[],"selected_response":null,"is_read":false,"is_orphaned":false}],
                "objectives":[],
                "contacts":[{"uuid":"c1","name":"C"}]
            }
        }"#;
        let msg = JsonCodec.decode_server(json).expect("decode");
        match msg {
            ServerMessage::CommsState { messages, contacts, .. } => {
                assert_eq!(messages.len(), 1);
                assert!(messages[0].sender_in_range, "sender_in_range must default to true");
                assert_eq!(contacts.len(), 1);
                assert!(contacts[0].in_range, "in_range must default to true");
            }
            other => panic!("expected CommsState, got {other:?}"),
        }
    }

    // ── EntityStateSnapshot with shields wire extension ───────────────────

    #[test]
    fn entity_state_snapshot_with_shields_round_trips() {
        use crate::messages::ShieldFacingStatus;
        let msg = ServerMessage::SimState {
            snapshot: crate::messages::SimSnapshot {
                red_alert: false,
                view_mode: crate::messages::ViewMode::default(),
                ship_x: 0.0,
                ship_z: 0.0,
                ship_yaw: 0.0,
                forward_speed: 0.0,
                power_levels: (2, 2, 2),
                flags: vec![],
                entity_states: vec![crate::messages::EntityStateSnapshot {
                    uuid: "ship-1".into(),
                    position: Some([50.0, 0.0, 0.0]),
                    yaw: Some(0.0),
                    hull_fraction: Some(0.75),
                    flags: vec![],
                    shields: Some(vec![
                        ShieldFacingStatus {
                            label: "Fore".into(),
                            hp: 0,
                            max_hp: 100,
                            online: false,
                            offline_remaining: 10.0,
                            is_focused: false,
                        },
                        ShieldFacingStatus {
                            label: "Aft".into(),
                            hp: 100,
                            max_hp: 100,
                            online: true,
                            offline_remaining: 0.0,
                            is_focused: false,
                        },
                        ShieldFacingStatus {
                            label: "Port".into(),
                            hp: 50,
                            max_hp: 100,
                            online: true,
                            offline_remaining: 0.0,
                            is_focused: false,
                        },
                        ShieldFacingStatus {
                            label: "Starboard".into(),
                            hp: 80,
                            max_hp: 100,
                            online: true,
                            offline_remaining: 0.0,
                            is_focused: false,
                        },
                    ]),
                    warp_out_remaining_secs: None,
                }],
                radar_state: crate::messages::RadarStateSnapshot::default(),
                impulse_charge_progress: 0.0,
                engine_thrust: 0.0,
                console_hull: vec![],
                navigation_waypoint: None,
            },
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn ship_destroyed_round_trips() {
        let msg = ServerMessage::ShipDestroyed;
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn damage_taken_shield_only_round_trips() {
        let msg = ServerMessage::DamageTaken {
            hull: 0.0,
            shield: 12.5,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn damage_taken_hull_only_round_trips() {
        let msg = ServerMessage::DamageTaken {
            hull: 8.0,
            shield: 0.0,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn damage_taken_both_fields_round_trips() {
        let msg = ServerMessage::DamageTaken {
            hull: 3.5,
            shield: 10.0,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn entity_state_snapshot_without_shields_field_defaults_to_none() {
        // Shields field omitted from JSON → deserializes to None
        let json = r#"{"type":"SimState","data":{"snapshot":{"red_alert":false,"view_mode":{"kind":"Camera","data":"Fore"},"ship_x":0.0,"ship_z":0.0,"ship_yaw":0.0,"hull_integrity":1.0,"power_levels":[2,2,2],"flags":[],"entity_states":[{"uuid":"e1","flags":[]}],"radar_state":{"helm_range":50.0,"tactical_range":60.0,"science_long_range":200.0,"science_system_map":500.0}}}}"#;
        let decoded: ServerMessage = JsonCodec.decode_server(json).unwrap();
        if let ServerMessage::SimState { snapshot } = decoded {
            assert_eq!(snapshot.entity_states.len(), 1);
            assert!(
                snapshot.entity_states[0].shields.is_none(),
                "shields must default to None when absent"
            );
        } else {
            panic!("expected SimState");
        }
    }

    // ── HTML console bridge (de)serialisation ─────────────────────────────

    #[test]
    fn decode_ui_action_fire_torpedo_envelope() {
        // Full envelope as produced by window.__sendAction; the extra
        // `console` field is ignored by serde.
        let json = r#"{"action":"fire_torpedo","console":"Tactical","tube":"fore","target_uuid":null}"#;
        let action = decode_ui_action(json).expect("decode fire_torpedo");
        assert_eq!(
            action,
            UiAction::FireTorpedo {
                tube: "fore".into(),
                target_uuid: None
            }
        );
    }

    #[test]
    fn decode_ui_action_fire_torpedo_with_target() {
        let json = r#"{"action":"fire_torpedo","console":"Tactical","tube":"fore","target_uuid":"abc-123"}"#;
        let action = decode_ui_action(json).expect("decode fire_torpedo");
        assert_eq!(
            action,
            UiAction::FireTorpedo {
                tube: "fore".into(),
                target_uuid: Some("abc-123".into())
            }
        );
    }

    #[test]
    fn decode_ui_action_fire_torpedo_omitted_target_defaults_none() {
        // target_uuid omitted entirely → defaults to None via #[serde(default)].
        let json = r#"{"action":"fire_torpedo","console":"Tactical","tube":"aft"}"#;
        let action = decode_ui_action(json).expect("decode fire_torpedo");
        assert_eq!(
            action,
            UiAction::FireTorpedo {
                tube: "aft".into(),
                target_uuid: None
            }
        );
    }

    #[test]
    fn decode_ui_action_fire_phaser_envelope() {
        let json = r#"{"action":"fire_phaser","console":"Tactical","bank":"port"}"#;
        let action = decode_ui_action(json).expect("decode fire_phaser");
        assert_eq!(action, UiAction::FirePhaser { bank: "port".into() });
    }

    #[test]
    fn encode_hud_state_round_trips() {
        let state = ViewscreenHudState {
            heading: 90,
            hull_pct: 75,
            condition: "ALERT".into(),
            red_alert: true,
        };
        let json = encode_hud_state(&state).expect("encode hud");
        let decoded: ViewscreenHudState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn encode_hud_state_emits_snake_case_fields() {
        let state = ViewscreenHudState {
            heading: 0,
            hull_pct: 100,
            condition: "NOMINAL".into(),
            red_alert: false,
        };
        let json = encode_hud_state(&state).expect("encode hud");
        assert!(json.contains("\"heading\":0"), "got: {json}");
        assert!(json.contains("\"hull_pct\":100"), "got: {json}");
        assert!(json.contains("\"condition\":\"NOMINAL\""), "got: {json}");
        assert!(json.contains("\"red_alert\":false"), "got: {json}");
    }

    #[test]
    fn encode_console_state_round_trips_helm() {
        let state = HelmConsoleState {
            heading: 045.0,
            speed: 75.0,
            x: 1200.5,
            z: -800.3,
            yaw: 0.785,
            impulse_charge_progress: 0.0,
            on_screen: false,
        };
        let json = encode_console_state(&state).expect("encode helm console");
        let decoded: HelmConsoleState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn encode_console_state_round_trips_weapons() {
        let state = WeaponsConsoleState {
            target_uuid: Some("tgt-1".into()),
            target_name: None,
            banks: vec![PhaserBankState {
                id: "port".into(),
                fire_ready: true,
                on_cooldown: false,
                cooldown_remaining: 0.0,
            }],
            tubes: vec![TorpedoTubeState {
                id: "fore".into(),
                loaded: true,
                reload_secs: 0.0,
            }],
            torpedo_count: 6,
            phaser_mode: PhaserMode::Auto,
            phaser_arcs: Vec::new(),
            torpedo_arcs: Vec::new(),
            blips: Vec::new(),
            regions: Vec::new(),
        };
        let json = encode_console_state(&state).expect("encode console");
        let decoded: WeaponsConsoleState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn encode_console_state_round_trips_captain() {
        use crate::messages::{CaptainConsoleState, ObjectiveSnapshot, ObjectiveStatus};
        let state = CaptainConsoleState {
            red_alert: true,
            view_direction: "Starboard".into(),
            objectives: vec![ObjectiveSnapshot {
                id: "obj-1".into(),
                text: "Destroy the pirate base".into(),
                mandatory: true,
                status: ObjectiveStatus::Active,
                targets: vec![],
            }],
            hull_integrity_pct: 87.5,
            game_status: "RED ALERT — All hands to battlestations.".into(),
        };
        let json = encode_console_state(&state).expect("encode captain console");
        // Verify field names match what the HTML JS reads.
        assert!(json.contains("\"view_direction\":\"Starboard\""), "got: {json}");
        assert!(json.contains("\"red_alert\":true"), "got: {json}");
        assert!(json.contains("\"hull_integrity_pct\":87.5"), "got: {json}");
        let decoded: CaptainConsoleState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn radar_blip_with_new_fields_round_trips() {
        let blip = RadarBlip {
            uuid: "abc-123".into(),
            radar_x: 0.5,
            radar_y: -0.3,
            scaled_radius: 0.02,
            kind: "ship".into(),
            icon: "ship".into(),
            color: [1.0, 0.502, 0.376],
            objective_target: true,
            name: Some("Pirate Raider".into()),
            selectable: true,
            threat_level: Some("medium".into()),
            description: Some("A pirate vessel".into()),
            target_tags: vec!["ship".into(), "pirate".into()],
        };
        let json = serde_json::to_string(&blip).unwrap();
        let decoded: RadarBlip = serde_json::from_str(&json).unwrap();
        assert_eq!(blip, decoded);
        assert!(json.contains("\"icon\":\"ship\""), "got: {json}");
        assert!(json.contains("\"objective_target\":true"), "got: {json}");
        assert!(json.contains("\"name\":\"Pirate Raider\""), "got: {json}");
    }

    #[test]
    fn radar_blip_new_fields_default_when_absent() {
        // JSON without the new fields (as emitted by pre-#445 server)
        let json = r#"{"uuid":"old-uuid","radar_x":0.1,"radar_y":0.2,"scaled_radius":0.01,"kind":"asteroid"}"#;
        let blip: RadarBlip = serde_json::from_str(json).unwrap();
        assert_eq!(blip.icon, "");
        assert_eq!(blip.color, [0.0, 0.0, 0.0]);
        assert!(!blip.objective_target);
        assert!(blip.name.is_none());
    }

    #[test]
    fn radar_region_round_trips() {
        let region = RadarRegion {
            uuid: "region-1".into(),
            x: 100.0,
            z: -200.0,
            shape: "sphere".into(),
            radius: Some(50.0),
            inner_radius: None,
            outer_radius: Some(50.0),
            half_extents: None,
            yaw: None,
            color: [1.0, 0.0, 0.0],
            name: Some("Danger Zone".into()),
        };
        let json = serde_json::to_string(&region).unwrap();
        let decoded: RadarRegion = serde_json::from_str(&json).unwrap();
        assert_eq!(region, decoded);
    }

    #[test]
    fn radar_region_box_round_trips() {
        let region = RadarRegion {
            uuid: "region-box".into(),
            x: 0.0,
            z: 0.0,
            shape: "box".into(),
            radius: None,
            inner_radius: None,
            outer_radius: None,
            half_extents: Some([40.0, 30.0]),
            yaw: Some(0.785),
            color: [0.0, 1.0, 0.5],
            name: None,
        };
        let json = serde_json::to_string(&region).unwrap();
        let decoded: RadarRegion = serde_json::from_str(&json).unwrap();
        assert_eq!(region, decoded);
    }

    #[test]
    fn decode_ui_action_increase_power() {
        let json = r#"{"action":"increase_power","console":"Power","target":"Helm"}"#;
        let action = decode_ui_action(json).expect("decode increase_power");
        assert_eq!(action, UiAction::IncreasePower { target: Console::Helm });
    }

    #[test]
    fn decode_ui_action_decrease_power() {
        let json = r#"{"action":"decrease_power","console":"Power","target":"Tactical"}"#;
        let action = decode_ui_action(json).expect("decode decrease_power");
        assert_eq!(action, UiAction::DecreasePower { target: Console::Tactical });
    }

    #[test]
    fn decode_ui_action_dispatch_repair_team() {
        let json = r#"{"action":"dispatch_repair_team","console":"Repair","team_idx":1,"target":"Helm"}"#;
        let action = decode_ui_action(json).expect("decode dispatch_repair_team");
        assert_eq!(action, UiAction::DispatchRepairTeam { team_idx: 1, target: Console::Helm });
    }

    #[test]
    fn power_console_state_round_trips() {
        use crate::messages::{PowerConsoleEntry, PowerConsoleState};
        let state = PowerConsoleState {
            consoles: vec![
                PowerConsoleEntry { id: "Helm".into(),     label: "HELM".into(),    level: 3, max_level: 4 },
                PowerConsoleEntry { id: "Tactical".into(), label: "WEAPONS".into(), level: 2, max_level: 4 },
                PowerConsoleEntry { id: "Sensors".into(),  label: "SENSORS".into(), level: 2, max_level: 4 },
            ],
            total: 7,
            total_max: 8,
            battery_charge: 80.0,
            battery_max: 100.0,
            locked: false,
        };
        let json = encode_console_state(&state).expect("encode power console state");
        let decoded: PowerConsoleState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn power_console_state_locked_round_trips() {
        use crate::messages::{PowerConsoleEntry, PowerConsoleState};
        let state = PowerConsoleState {
            consoles: vec![
                PowerConsoleEntry { id: "Helm".into(),     label: "HELM".into(),    level: 1, max_level: 4 },
                PowerConsoleEntry { id: "Tactical".into(), label: "WEAPONS".into(), level: 1, max_level: 4 },
                PowerConsoleEntry { id: "Sensors".into(),  label: "SENSORS".into(), level: 1, max_level: 4 },
            ],
            total: 3,
            total_max: 8,
            battery_charge: 0.0,
            battery_max: 100.0,
            locked: true,
        };
        let json = encode_console_state(&state).expect("encode locked power console state");
        let decoded: PowerConsoleState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn repair_console_state_round_trips() {
        use crate::messages::{ConsoleHullStatus, RepairConsoleState, TeamSlot};
        let state = RepairConsoleState {
            teams: vec![
                TeamSlot::Idle,
                TeamSlot::Travelling { console: Console::Helm, elapsed: 1.5 },
            ],
            console_hull: vec![
                ConsoleHullStatus { console: Console::Helm,     current: 20.0, max_hp: 25.0 },
                ConsoleHullStatus { console: Console::Tactical, current: 25.0, max_hp: 25.0 },
            ],
            travel_duration_secs: 5.0,
            damageable_consoles: vec![Console::Helm, Console::Tactical],
        };
        let json = encode_console_state(&state).expect("encode repair console state");
        let decoded: RepairConsoleState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn repair_console_state_all_team_slots_round_trip() {
        use crate::messages::{RepairConsoleState, TeamSlot};
        let state = RepairConsoleState {
            teams: vec![
                TeamSlot::Idle,
                TeamSlot::Travelling { console: Console::Helm, elapsed: 2.0 },
                TeamSlot::Repairing  { console: Console::Tactical },
                TeamSlot::Returning  { remaining: 3.0, queued: Some(Console::Power) },
            ],
            console_hull: vec![],
                navigation_waypoint: None,
            travel_duration_secs: 5.0,
            damageable_consoles: vec![Console::Helm, Console::Tactical, Console::Power],
        };
        let json = encode_console_state(&state).expect("encode repair console state all slots");
        let decoded: RepairConsoleState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn shields_console_state_round_trips() {
        use crate::messages::{ShieldFacingStatus, ShieldsConsoleState};
        let state = ShieldsConsoleState {
            facings: vec![
                ShieldFacingStatus { label: "Fore".into(), hp: 100, max_hp: 100, online: true, offline_remaining: 0.0, is_focused: true },
                ShieldFacingStatus { label: "Port".into(), hp: 72, max_hp: 100, online: true, offline_remaining: 0.0, is_focused: false },
                ShieldFacingStatus { label: "Aft".into(), hp: 0, max_hp: 100, online: false, offline_remaining: 8.0, is_focused: false },
                ShieldFacingStatus { label: "Starboard".into(), hp: 88, max_hp: 100, online: true, offline_remaining: 0.0, is_focused: false },
            ],
            hull_integrity_pct: 78.0,
            focused_facing: Some("Fore".into()),
            grid_status: "GRID NOMINAL".into(),
            target_bearing: Some(272.0),
        };
        let json = encode_console_state(&state).expect("encode shields console state");
        let decoded: ShieldsConsoleState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn shields_console_state_no_target_no_focus_round_trips() {
        use crate::messages::ShieldsConsoleState;
        let state = ShieldsConsoleState {
            facings: vec![],
            hull_integrity_pct: 100.0,
            focused_facing: None,
            grid_status: "GRID OFFLINE".into(),
            target_bearing: None,
        };
        let json = encode_console_state(&state).expect("encode shields console state no target");
        let decoded: ShieldsConsoleState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn shields_console_state_json_field_names() {
        use crate::messages::{ShieldFacingStatus, ShieldsConsoleState};
        let state = ShieldsConsoleState {
            facings: vec![
                ShieldFacingStatus { label: "Fore".into(), hp: 100, max_hp: 100, online: true, offline_remaining: 0.0, is_focused: true },
            ],
            hull_integrity_pct: 100.0,
            focused_facing: Some("Fore".into()),
            grid_status: "GRID NOMINAL".into(),
            target_bearing: None,
        };
        let json = encode_console_state(&state).expect("encode shields console state");
        assert!(json.contains("\"facings\":"), "got: {json}");
        assert!(json.contains("\"hull_integrity_pct\":"), "got: {json}");
        assert!(json.contains("\"focused_facing\":"), "got: {json}");
        assert!(json.contains("\"grid_status\":"), "got: {json}");
        assert!(json.contains("\"target_bearing\":"), "got: {json}");
    }

    #[test]
    fn weapons_console_state_with_regions_round_trips() {
        let state = WeaponsConsoleState {
            target_uuid: None,
            target_name: None,
            banks: Vec::new(),
            tubes: Vec::new(),
            torpedo_count: 0,
            phaser_mode: PhaserMode::Auto,
            phaser_arcs: Vec::new(),
            torpedo_arcs: Vec::new(),
            blips: vec![RadarBlip {
                uuid: "ent-1".into(),
                radar_x: 0.2,
                radar_y: 0.4,
                scaled_radius: 0.05,
                kind: "asteroid".into(),
                icon: "asteroid".into(),
                color: [0.478, 0.753, 1.0],
                objective_target: false,
                name: None,
                selectable: false,
                threat_level: None,
                description: None,
                target_tags: Vec::new(),
            }],
            regions: vec![RadarRegion {
                uuid: "zone-1".into(),
                x: 50.0,
                z: 50.0,
                shape: "torus".into(),
                radius: Some(80.0),
                inner_radius: Some(20.0),
                outer_radius: Some(80.0),
                half_extents: None,
                yaw: None,
                color: [0.5, 0.2, 1.0],
                name: Some("Ion Storm".into()),
            }],
        };
        let json = encode_console_state(&state).expect("encode");
        let decoded: WeaponsConsoleState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, decoded);
    }
}
