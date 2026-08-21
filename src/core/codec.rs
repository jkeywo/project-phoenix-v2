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
// host-channel pushes (HUD, lobby, chatter, audio) and the inbound
// `ClientMessage` decode. Bridge / plugin code must call these, never
// `serde_json` directly.

/// Encode a `ViewscreenHudState` to JSON for the HTML viewscreen overlay.
pub fn encode_hud_state(
    s: &crate::messages::ViewscreenHudState,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(s)
}

/// Encode an AI→AI chatter event for the `"chatter"` host channel (issue
/// #818). The wire shape is `{"from_label":…,"to_label":…,"text":…}` — the
/// `__updateChatter` handler in `server.html` reads exactly these keys.
pub fn encode_chatter(
    ev: &crate::console_bridge::AiChatterEvent,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(ev)
}

/// Encode the merged ship + world audio config for `__audioConfig`. Sent once
/// on game start; JS builds its `<audio>` elements and Web Audio graph from it.
pub fn encode_audio_config(
    p: &crate::audio_config::AudioConfigPayload,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(p)
}

/// Encode a one-shot positional audio cue for `__audioCue`. Coordinates are
/// listener-relative — see `audio_config::listener_relative`.
pub fn encode_audio_cue(c: &crate::audio_config::AudioCue) -> Result<String, serde_json::Error> {
    serde_json::to_string(c)
}

/// Decode inbound JSON from the HTML/PeerJS bridge.
///
/// The wire shape is a full `ClientMessage` — every emitter (phone consoles,
/// host-page consoles via `gui/action-map.js`, smoke fixtures) sends the
/// serde envelope directly. The short-form system-control shim that used to
/// live here was retired by issue #822 once no console emitted short form.
pub fn decode_bridge_client_message(s: &str) -> Result<ClientMessage, serde_json::Error> {
    serde_json::from_str(s)
}

/// Encode a `LobbyStatePayload` to JSON for the HTML lobby overlay.
pub fn encode_lobby_state(
    s: &crate::messages::LobbyStatePayload,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(s)
}

// ── Batch inbound decode (issue #602) ───────────────────────────────────────

/// A single decode failure from the bridge inbound drain, with truncated
/// fields for safe logging.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodeError {
    pub token: String,
    pub payload_snippet: String,
}

/// Batch-decode a list of `(token, json)` pairs into successful
/// `ClientMessage` values and `DecodeError` failures. Truncates
/// token to 12 chars and payload snippet to 80 chars at collection time.
pub fn decode_bridge_client_messages(
    entries: Vec<(String, String)>,
) -> (Vec<(String, ClientMessage)>, Vec<DecodeError>) {
    let mut successes = Vec::new();
    let mut failures = Vec::new();
    for (token, json) in entries {
        match decode_bridge_client_message(&json) {
            Ok(msg) => successes.push((token, msg)),
            Err(_) => {
                let truncated_token: String = token.chars().take(12).collect();
                let payload_snippet: String = json.chars().take(80).collect();
                failures.push(DecodeError {
                    token: truncated_token,
                    payload_snippet,
                });
            }
        }
    }
    (successes, failures)
}

// ── Delivery documents (PRD #855) ─────────────────────────────────────────────
//
// The native host serves these over HTTP and the browser host publishes the
// identical bytes through `bridge::wasm_delivery_manifest`. They live here for
// the same reason everything above does: `serde_json` is confined to this
// module (AGENTS.md constraint 1), so a host that wants JSON asks for it here.
//
// Field NAMES for the catalogue entries come from `delivery::payload`, never
// from this file — that is the whole point of that module's ordered entry
// lists, and it is why a new catalogue field cannot reach the browser surface
// while skipping the native one.

fn stamp_json(stamp: &crate::delivery::stamp::DeliveryStamp) -> serde_json::Value {
    serde_json::json!({
        "protocol": stamp.protocol,
        "content_id": stamp.content_id,
        "content_epoch": stamp.content_epoch,
    })
}

fn payload_value_json(value: &crate::delivery::payload::PayloadValue) -> serde_json::Value {
    use crate::delivery::payload::PayloadValue;
    match value {
        PayloadValue::Text(s) => serde_json::Value::String(s.clone()),
        PayloadValue::Number(n) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
    }
}

fn ship_json(ship: &crate::delivery::payload::ShipPayload) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (key, value) in ship.entries() {
        obj.insert((*key).to_string(), payload_value_json(value));
    }
    serde_json::Value::Object(obj)
}

fn scenario_json(scenario: &crate::delivery::payload::ScenarioPayload) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for (key, value) in scenario.entries() {
        obj.insert((*key).to_string(), payload_value_json(value));
    }
    obj.insert(
        crate::delivery::payload::SHIPS_KEY.to_string(),
        serde_json::Value::Array(scenario.ships().iter().map(ship_json).collect()),
    );
    serde_json::Value::Object(obj)
}

/// Encode a host's own version stamp — the body of `/host/stamp.json`.
pub fn encode_delivery_stamp(stamp: &crate::delivery::stamp::DeliveryStamp) -> String {
    stamp_json(stamp).to_string()
}

/// Encode the content manifest + catalogue a host publishes.
pub fn encode_delivery_manifest(manifest: &crate::delivery::DeliveryManifest) -> String {
    serde_json::json!({
        "stamp": stamp_json(&manifest.stamp),
        "manifest_path": manifest.manifest_path,
        "scenarios": manifest
            .scenarios
            .iter()
            .map(scenario_json)
            .collect::<Vec<_>>(),
    })
    .to_string()
}

/// Encode a version-pin refusal — the body of a `409` from either host.
pub fn encode_delivery_refusal(refusal: &crate::delivery::DeliveryRefusal) -> String {
    serde_json::json!({
        "error": refusal.mismatch.code(),
        "detail": refusal.mismatch.detail(),
        "host": stamp_json(&refusal.host),
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::*;
    use std::collections::{BTreeMap, HashMap};
    use strum::IntoEnumIterator;

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
            connected: true,
            ready: false,
            station: None,
            last_rating: None,
            spectator: false,
            afk: false,
        }
    }

    fn state() -> GameState {
        GameState {
            phase: GamePhase::Lobby,
            players: vec![player()],
            world: None,
        }
    }

    fn empty_ship_stations() -> crate::stations_config::ShipStations {
        crate::stations_config::ShipStations::default()
    }

    fn sample_entity_snapshot() -> EntitySnapshot {
        EntitySnapshot {
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
            shield_fraction: None,
            inner_radius: None,
            warp_out_remaining_secs: None,
            radar_size: None,
            region_colour: None,
            half_extents: None,
            radar_icon: None,
            objective_target: false,
            target_tags: Vec::new(),
            threat_level: None,
            target_description: None,
            infrastructure: None,
        }
    }

    /// Issue #1025: the published infrastructure block survives the wire.
    ///
    /// Its own test rather than a field on `sample_entity_snapshot`, because
    /// that sample is an asteroid and an asteroid has no infrastructure — a
    /// populated block on it would pin a shape nothing produces. What matters
    /// here is that the flag list and the capacity list, both tuple-typed,
    /// round-trip in order and with their booleans intact.
    #[test]
    fn a_published_infrastructure_block_round_trips() {
        let snapshot = EntitySnapshot {
            uuid: "550e8400-e29b-41d4-a716-446655440001".into(),
            tags: vec!["station".into()],
            infrastructure: Some(crate::messages::InfrastructureSnapshot {
                condition_fraction: 0.3,
                flags: vec![
                    ("depot_transfer_capable".into(), false),
                    ("depot_docking_capable".into(), true),
                ],
                capacities: vec![("depot_transfer_throughput".into(), 40)],
            }),
            ..EntitySnapshot::default()
        };
        let json = serde_json::to_string(&snapshot).expect("serialises");
        let back: EntitySnapshot = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, snapshot, "the whole block must survive verbatim");

        let bare = EntitySnapshot {
            uuid: "550e8400-e29b-41d4-a716-446655440002".into(),
            ..EntitySnapshot::default()
        };
        let json = serde_json::to_string(&bare).expect("serialises");
        assert!(
            !json.contains("infrastructure"),
            "an entity with no infrastructure must not pay for the field on the wire — every \
             entity shipped today is in this arm, got {json}"
        );
    }

    // ── Table-driven round-trip harness (issue #610) ──────────────────────────
    //
    // One sample message per `ClientMessage` / `ServerMessage` variant. Adding a
    // variant to either enum means adding a table row here — the exhaustiveness
    // tests below (`client_message_table_covers_every_variant` /
    // `server_message_table_covers_every_variant`) fail with the missing
    // variant's name if a row is forgotten, and the round-trip tests
    // (`client_message_table_round_trips` / `server_message_table_round_trips`)
    // exercise every row through both `JsonCodec` and `PrettyJsonCodec`.
    //
    // `ClientMessageDiscriminants` / `ServerMessageDiscriminants` come from
    // `#[derive(strum::EnumDiscriminants)]` on the two enums in `messages.rs`:
    // a fieldless companion enum with a `strum::IntoEnumIterator` impl that is
    // regenerated from the real enum on every build, so it can never drift out
    // of sync with the variant list the way a hand-maintained list could.

    fn client_message_table() -> Vec<(ClientMessageDiscriminants, ClientMessage)> {
        vec![
            (
                ClientMessageDiscriminants::Identify,
                ClientMessage::Identify {
                    token: "t".into(),
                    name: "Bob".into(),
                },
            ),
            (
                ClientMessageDiscriminants::SetName,
                ClientMessage::SetName {
                    name: "Carol".into(),
                },
            ),
            (
                ClientMessageDiscriminants::SelectStation,
                ClientMessage::SelectStation {
                    station: "Captain".into(),
                },
            ),
            (
                ClientMessageDiscriminants::ReleaseStation,
                ClientMessage::ReleaseStation,
            ),
            (
                ClientMessageDiscriminants::SetReady,
                ClientMessage::SetReady { ready: true },
            ),
            (
                ClientMessageDiscriminants::SetSpectator,
                ClientMessage::SetSpectator { spectator: true },
            ),
            (
                ClientMessageDiscriminants::SetAfk,
                ClientMessage::SetAfk { afk: true },
            ),
            (
                ClientMessageDiscriminants::ControlSystem,
                ClientMessage::ControlSystem {
                    target: crate::system_registry::helm_thrust_system_id(),
                    payload: SystemControlPayload::SetThrust { value: 0.75 },
                },
            ),
            (
                ClientMessageDiscriminants::ControlSystem,
                ClientMessage::ControlSystem {
                    target: crate::system_registry::helm_steering_system_id(),
                    payload: SystemControlPayload::SetSteering { value: -0.25 },
                },
            ),
            (
                ClientMessageDiscriminants::SetStationRating,
                ClientMessage::SetStationRating {
                    rating_name: "Assisted".into(),
                },
            ),
            (
                ClientMessageDiscriminants::ReportStationEligibility,
                ClientMessage::ReportStationEligibility {
                    ineligible: vec![crate::messages::StationId("science".into())],
                },
            ),
            (
                ClientMessageDiscriminants::SendCoordination,
                ClientMessage::SendCoordination {
                    // Coordination targets are station-id keys (issue #801) —
                    // console-level routing, not system admission.
                    target: crate::system_registry::tactical_station_key(),
                    payload: CoordinationPayload::FrequencyHint { frequency: 0.33 },
                },
            ),
            (
                ClientMessageDiscriminants::ReturnToLobby,
                ClientMessage::ReturnToLobby,
            ),
            (
                ClientMessageDiscriminants::SelectScenario,
                ClientMessage::SelectScenario {
                    scenario_id: "default".into(),
                },
            ),
            (
                ClientMessageDiscriminants::SelectPlayerShip,
                ClientMessage::SelectPlayerShip {
                    template_path: "assets/entities/alliance_cruiser.toml".into(),
                },
            ),
            (
                ClientMessageDiscriminants::StationVisited,
                ClientMessage::StationVisited {
                    station: crate::messages::StationId("comms".into()),
                },
            ),
            // The two client settings-menu routes (issue #940). Both rows carry
            // the same `#[cfg]` the variants do, so in a demo build the table
            // shrinks with the enum and the exhaustiveness test below still
            // balances — `strum`'s `EnumDiscriminants` copies `cfg` onto the
            // generated discriminant enum, so both sides lose the same names.
            #[cfg(not(phoenix_demo_build))]
            (
                ClientMessageDiscriminants::ToggleDebugFlag,
                ClientMessage::ToggleDebugFlag {
                    flag: crate::messages::DebugFlag::Regions,
                },
            ),
            #[cfg(not(phoenix_demo_build))]
            (
                ClientMessageDiscriminants::TogglePause,
                ClientMessage::TogglePause,
            ),
        ]
    }

    fn server_message_table() -> Vec<(ServerMessageDiscriminants, ServerMessage)> {
        vec![
            (
                ServerMessageDiscriminants::Welcome,
                ServerMessage::Welcome {
                    state: state(),
                    ship_stations: empty_ship_stations(),
                    ship_config: ShipClientConfig::default(),
                    station_ratings: HashMap::new(),
                },
            ),
            (
                ServerMessageDiscriminants::PlayerJoined,
                ServerMessage::PlayerJoined { player: player() },
            ),
            (
                ServerMessageDiscriminants::PlayerLeft,
                ServerMessage::PlayerLeft {
                    token: "tok".into(),
                },
            ),
            (
                ServerMessageDiscriminants::StationAssigned,
                ServerMessage::StationAssigned {
                    token: "tok".into(),
                    station: Some("Captain".into()),
                    station_id: Some(StationId("captain".into())),
                },
            ),
            (
                ServerMessageDiscriminants::ReadyChanged,
                ServerMessage::ReadyChanged {
                    token: "tok".into(),
                    ready: true,
                },
            ),
            (
                ServerMessageDiscriminants::SpectatorChanged,
                ServerMessage::SpectatorChanged {
                    token: "tok".into(),
                    spectator: true,
                },
            ),
            (
                ServerMessageDiscriminants::AfkChanged,
                ServerMessage::AfkChanged {
                    token: "tok".into(),
                    afk: true,
                },
            ),
            (
                ServerMessageDiscriminants::NameChanged,
                ServerMessage::NameChanged {
                    token: "tok".into(),
                    name: "Dave".into(),
                },
            ),
            (
                ServerMessageDiscriminants::GameStarted,
                ServerMessage::GameStarted,
            ),
            (
                ServerMessageDiscriminants::GameStartCountdown,
                ServerMessage::GameStartCountdown { remaining_secs: 5 },
            ),
            (
                ServerMessageDiscriminants::LoadingProgress,
                ServerMessage::LoadingProgress { fraction: 0.5 },
            ),
            (
                ServerMessageDiscriminants::SimState,
                ServerMessage::SimState {
                    snapshot: SimSnapshot {
                        entity_states: vec![EntityStateSnapshot {
                            uuid: "ast-1".into(),
                            position: Some([12.0, 0.0, -5.0]),
                            yaw: Some(0.5),
                            hull_fraction: Some(1.0),
                            shield_fraction: None,
                            flags: vec![],
                            shields: None,
                            shield_freq: None,
                            warp_out_remaining_secs: None,
                        }],
                        station_hosts: vec![StationHostSnapshot {
                            station: StationId("navigation".into()),
                            host: Some(StationId("tactical".into())),
                            rating: "Std".into(),
                        }],
                        station_health: vec![StationHealthSnapshot {
                            station: StationId("navigation".into()),
                            health: Some(0.5),
                        }],
                        station_importance: vec![StationImportanceSnapshot {
                            station: StationId("navigation".into()),
                            unread: true,
                            critical: false,
                        }],
                        control_sources: BTreeMap::from([
                            (SystemId("navigation".into()), "Human".into()),
                            (SystemId("shields-system".into()), "Ai".into()),
                        ]),
                    },
                },
            ),
            (
                ServerMessageDiscriminants::WorldSetup,
                ServerMessage::WorldSetup {
                    world: WorldData {
                        entities: vec![sample_entity_snapshot()],
                        ..Default::default()
                    },
                },
            ),
            (
                ServerMessageDiscriminants::TargetLock,
                ServerMessage::TargetLock {
                    uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
                    locked: true,
                },
            ),
            (
                ServerMessageDiscriminants::WeaponsUpdate,
                ServerMessage::WeaponsUpdate {
                    target_uuid: Some("550e8400-e29b-41d4-a716-446655440000".into()),
                    target_name: Some("Klingon Raider".into()),
                    banks: vec![PhaserBankState {
                        id: "port".to_string(),
                        fire_ready: true,
                        on_cooldown: false,
                        cooldown_remaining: 0.0,
                        readiness: WeaponReadiness::default(),
                    }],
                    tubes: vec![TorpedoTubeState {
                        id: "fore_port".to_string(),
                        loaded: true,
                        reload_secs: 0.0,
                        state: "loaded".into(),
                        progress: 1.0,
                        load_time: 10.0,
                        volley_max: 3,
                        loaded_count: 2,
                        target_count: 3,
                        load_progress: 1.0,
                        readiness: WeaponReadiness::default(),
                        active_barrels: Vec::new(),
                        pattern_step: 0,
                        pattern_len: 0,
                    }],
                    torpedo_count: 10,
                    phaser_mode: PhaserMode::Auto,
                    blasters: vec![],
                    phaser_frequency: 0.5,
                },
            ),
            (
                ServerMessageDiscriminants::BeamStarted,
                ServerMessage::BeamStarted {
                    bank: "port".to_string(),
                    source_uuid: "11111111-1111-1111-1111-111111111111".into(),
                    target_uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
                },
            ),
            (
                ServerMessageDiscriminants::BeamEnded,
                ServerMessage::BeamEnded {
                    bank: "port".to_string(),
                    source_uuid: "11111111-1111-1111-1111-111111111111".into(),
                    target_uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
                },
            ),
            (
                ServerMessageDiscriminants::AsteroidDestroyed,
                ServerMessage::AsteroidDestroyed {
                    uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
                },
            ),
            (
                ServerMessageDiscriminants::PhaserFired,
                ServerMessage::PhaserFired {
                    bank: "port".to_string(),
                    target_uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
                },
            ),
            (
                ServerMessageDiscriminants::RepairState,
                ServerMessage::RepairState {
                    teams: vec![
                        TeamSlot::Idle,
                        TeamSlot::Travelling {
                            system_id: Some(SystemId("helm".into())),
                            display_name: Some("Helm".into()),
                            elapsed: 2.5,
                            priority: None,
                        },
                        TeamSlot::Repairing {
                            system_id: Some(SystemId("tactical".into())),
                            display_name: Some("Tactical".into()),
                            priority: None,
                            priority_system_id: None,
                        },
                        TeamSlot::Returning {
                            remaining: 3.0,
                            system_id: None,
                            display_name: None,
                            queued_system_id: Some(SystemId("tactical".into())),
                            queued_display_name: Some("Tactical".into()),
                        },
                    ],
                },
            ),
            (
                ServerMessageDiscriminants::ShieldStatus,
                ServerMessage::ShieldStatus {
                    facings: vec![ShieldFacingStatus {
                        label: "Fore".into(),
                        hp: 80,
                        max_hp: 100,
                        online: true,
                        offline_remaining: 0.0,
                        is_focused: false,
                        center_deg: 0.0,
                        width_deg: 90.0,
                        arc_id: "fore".into(),
                        priority: 1,
                    }],
                    frequency: 0.5,
                },
            ),
            (
                ServerMessageDiscriminants::TorpedoLaunched,
                ServerMessage::TorpedoLaunched {
                    uuid: "torpedo-uuid-1".into(),
                    tube: "fore_starboard".to_string(),
                    x: 10.5,
                    y: 3.25,
                    z: -20.0,
                    heading: 1.57,
                },
            ),
            (
                ServerMessageDiscriminants::TorpedoDestroyed,
                ServerMessage::TorpedoDestroyed {
                    uuid: "torpedo-uuid-1".into(),
                },
            ),
            (
                ServerMessageDiscriminants::BlasterFired,
                ServerMessage::BlasterFired {
                    bank: "fore".to_string(),
                    source_uuid: "11111111-1111-1111-1111-111111111111".into(),
                    projectile_id: "proj-uuid-1".into(),
                    x: 5.0,
                    z: -10.0,
                    heading: 0.0,
                    visual_scale: 1.0,
                },
            ),
            (
                ServerMessageDiscriminants::BlasterHit,
                ServerMessage::BlasterHit {
                    bank: "fore".to_string(),
                    projectile_id: "proj-uuid-1".into(),
                    target_uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
                },
            ),
            (
                ServerMessageDiscriminants::ModifierAdded,
                ServerMessage::ModifierAdded {
                    source: ModifierSource::PowerGroup(PowerGroupId("sensors".to_string())),
                    slot: ModifierSlot::RadarRange,
                    bonus: 0.5,
                },
            ),
            (
                ServerMessageDiscriminants::ModifierRemoved,
                ServerMessage::ModifierRemoved {
                    source: ModifierSource::ImpulseDrive,
                    slot: ModifierSlot::MaxYawRate,
                },
            ),
            (
                ServerMessageDiscriminants::AsteroidSpawned,
                ServerMessage::AsteroidSpawned {
                    uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
                    x: 100.0,
                    y: 0.0,
                    z: -50.0,
                    config_path: "assets/entities/asteroid_small.toml".into(),
                    max_hp: 30,
                    current_hp: 30,
                    radius: 2.0,
                    radar_icon: Some("asteroid".into()),
                    radar_colour: None,
                    radar_size: None,
                },
            ),
            (
                ServerMessageDiscriminants::PowerState,
                ServerMessage::PowerState {
                    helm: 3,
                    weapons: 2,
                    shields: 4,
                    battery_charge: 65.5,
                    draining: true,
                    locked: false,
                },
            ),
            (
                ServerMessageDiscriminants::EntitySpawned,
                ServerMessage::EntitySpawned {
                    snapshot: sample_entity_snapshot(),
                },
            ),
            (
                ServerMessageDiscriminants::EntityDespawned,
                ServerMessage::EntityDespawned {
                    uuid: "run-entity-001".into(),
                },
            ),
            (
                ServerMessageDiscriminants::StationSpawned,
                ServerMessage::StationSpawned {
                    uuid: "station-1".into(),
                    name: "Deep Space 9".into(),
                    position: [100.0, 0.0, -50.0],
                    shape: "cylinder".into(),
                    radius: 15.0,
                    hull_integrity: 200.0,
                },
            ),
            (
                ServerMessageDiscriminants::StationDestroyed,
                ServerMessage::StationDestroyed {
                    uuid: "station-1".into(),
                },
            ),
            (
                ServerMessageDiscriminants::ObjectiveSummary,
                ServerMessage::ObjectiveSummary {
                    objectives: vec![ObjectiveSnapshot {
                        id: "obj-1".into(),
                        text: "Destroy the convoy".into(),
                        text_params: Default::default(),
                        mandatory: true,
                        status: ObjectiveStatus::Active,
                        targets: vec!["Axiom Station".into()],
                        source: ObjectiveSource::Mission,
                    }],
                },
            ),
            (
                ServerMessageDiscriminants::CommsState,
                ServerMessage::CommsState {
                    messages: vec![CommsMessage {
                        id: "m1".into(),
                        sender_uuid: "station-abc".into(),
                        sender_name: "Starbase 12".into(),
                        subject: "Greetings".into(),
                        body: "Welcome to the sector.".into(),
                        body_params: Default::default(),
                        responses: vec![crate::messages::CommsResponseView {
                            text: "Acknowledged".into(),
                            important: true,
                            available: true,
                        }],
                        selected_response: Some(0),
                        is_read: false,
                        is_orphaned: false,
                        sender_in_range: true,
                        thread_id: "thread-001".into(),
                        is_urgent: false,
                    }],
                    objectives: vec![],
                    contacts: vec![CommsContact {
                        uuid: "station-abc".into(),
                        name: "Starbase 12".into(),
                        in_range: true,
                        is_urgent: false,
                    }],
                },
            ),
            (
                ServerMessageDiscriminants::CommsResponseRejected,
                ServerMessage::CommsResponseRejected {
                    message_id: "m1".into(),
                    response_index: 2,
                },
            ),
            (
                ServerMessageDiscriminants::CivilianOrderRejected,
                ServerMessage::CivilianOrderRejected {
                    target: "world.entity.hauler_kestrel.name".into(),
                    reason: "civilian.order.rejected.unknown_target".into(),
                },
            ),
            (
                ServerMessageDiscriminants::ShipDestroyed,
                ServerMessage::ShipDestroyed,
            ),
            (
                ServerMessageDiscriminants::GameOver,
                ServerMessage::GameOver {
                    reason: "server.game_over.ship_destroyed".into(),
                    outcome: Some("defeat".into()),
                },
            ),
            (
                ServerMessageDiscriminants::ReturnedToLobby,
                ServerMessage::ReturnedToLobby,
            ),
            (
                ServerMessageDiscriminants::ScenarioCatalog,
                ServerMessage::ScenarioCatalog {
                    scenarios: vec![crate::core::messages::ScenarioCatalogWire {
                        id: "default".into(),
                        world: "assets/worlds/default.toml".into(),
                        label: Some("Starbase Alpha".into()),
                        description: None,
                        ships: vec![crate::world::config::AvailableShipEntry {
                            template_path: "assets/entities/alliance_cruiser.toml".into(),
                            label: Some("Cruiser".into()),
                        }],
                    }],
                    locked_scenario: None,
                    locked_ship: None,
                },
            ),
            (
                ServerMessageDiscriminants::RatingChanged,
                ServerMessage::RatingChanged {
                    station_id: StationId("captain".into()),
                    rating_name: "Assisted".into(),
                },
            ),
            (
                ServerMessageDiscriminants::SystemHullUpdate,
                ServerMessage::SystemHullUpdate {
                    entries: vec![SystemHullStatus {
                        system_id: SystemId("helm".into()),
                        display_name: "Helm".into(),
                        current: 25.0,
                        max_hp: 25.0,
                        tier: crate::damage::DamageTier::Operational,
                        debuff_magnitude: 0.0,
                    }],
                    aggregate_fraction: Some(0.75),
                    destroyed_fraction: Some(0.25),
                },
            ),
            (
                ServerMessageDiscriminants::DamageTaken,
                ServerMessage::DamageTaken {
                    hull: 3.5,
                    shield: 10.0,
                },
            ),
            (
                ServerMessageDiscriminants::CoordinationPopup,
                ServerMessage::CoordinationPopup {
                    target: crate::system_registry::helm_station_key(),
                    payload: CoordinationPayload::Alert {
                        title: "Shield down".into(),
                        body: "Fore shield offline".into(),
                    },
                    sender_label: "AI Tactical".into(),
                },
            ),
            (
                ServerMessageDiscriminants::AiChatter,
                ServerMessage::AiChatter {
                    from_label: "Shields".into(),
                    to_label: "Helm".into(),
                    text: "Fore shield offline (12s)".into(),
                },
            ),
            (
                ServerMessageDiscriminants::BlackboardUpdate,
                ServerMessage::BlackboardUpdate {
                    updates: vec![(
                        SystemId("helm".into()),
                        SystemBlackboard::Helm(HelmBlackboard {
                            yaw: 0.785,
                            forward_speed: 75.0,
                            x: 1200.5,
                            z: -800.3,
                            impulse_charge: 0.0,
                            boost_battery: 0.5,
                            boost_active: true,
                            boost_enabled: true,
                            radar_range: 0.0,
                            lateral_speed: 0.0,
                            hostile_weapon_arcs: Vec::new(),
                        }),
                    )],
                },
            ),
            (
                ServerMessageDiscriminants::ShipManual,
                ServerMessage::ShipManual {
                    manual: crate::ship::manual::ShipManualWire {
                        stations: vec![
                            crate::ship::manual::StationManualWire {
                                station_id: StationId("captain".into()),
                                overview: Some("You command the bridge.".into()),
                                sections: vec![],
                            },
                            crate::ship::manual::StationManualWire {
                                station_id: StationId("science".into()),
                                overview: Some("Sensors and shields.".into()),
                                sections: vec![crate::ship::manual::SystemManualSection {
                                    kind: "shields".into(),
                                    metrics: vec![
                                        crate::ship::manual::SystemManualMetric {
                                            code: "max_hp".into(),
                                            value: 100.0,
                                        },
                                        crate::ship::manual::SystemManualMetric {
                                            code: "arcs".into(),
                                            value: 4.0,
                                        },
                                    ],
                                    capabilities: vec![],
                                    automation: vec![
                                        crate::ship::manual::StationRatingAutomation {
                                            rating: "Backfill".into(),
                                            automated_systems: vec![SystemId(
                                                "shield-arc-fore".into(),
                                            )],
                                        },
                                    ],
                                }],
                            },
                            // Helm station: exercises the #773 `capabilities`
                            // list (movement mode as a machine value_code) on
                            // the round-trip so the wire field is covered.
                            crate::ship::manual::StationManualWire {
                                station_id: StationId("helm".into()),
                                overview: Some("Fly the ship.".into()),
                                sections: vec![crate::ship::manual::SystemManualSection {
                                    kind: "helm_thrust".into(),
                                    metrics: vec![crate::ship::manual::SystemManualMetric {
                                        code: "max_speed".into(),
                                        value: 10.0,
                                    }],
                                    capabilities: vec![
                                        crate::ship::manual::SystemManualCapability {
                                            code: "movement_mode".into(),
                                            value_code: "bounded".into(),
                                        },
                                    ],
                                    automation: vec![],
                                }],
                            },
                        ],
                    },
                },
            ),
            (
                ServerMessageDiscriminants::DebugState,
                ServerMessage::DebugState {
                    // Mixed on/off so a pair whose flag and bool were swapped
                    // in the encoding would not still round-trip.
                    flags: crate::messages::DebugFlag::ALL
                        .iter()
                        .map(|f| (*f, *f == crate::messages::DebugFlag::Modifiers))
                        .collect(),
                    // Mixed again, for the same reason: two adjacent bools that
                    // agree cannot catch a transposition.
                    paused: false,
                    god_mode: true,
                },
            ),
        ]
    }

    #[test]
    fn client_message_table_covers_every_variant() {
        let covered: std::collections::HashSet<ClientMessageDiscriminants> =
            client_message_table().into_iter().map(|(d, _)| d).collect();
        for variant in ClientMessageDiscriminants::iter() {
            assert!(
                covered.contains(&variant),
                "ClientMessage variant {variant:?} has no sample row in client_message_table(); \
                 add one so the round-trip harness covers it"
            );
        }
    }

    #[test]
    fn server_message_table_covers_every_variant() {
        let covered: std::collections::HashSet<ServerMessageDiscriminants> =
            server_message_table().into_iter().map(|(d, _)| d).collect();
        for variant in ServerMessageDiscriminants::iter() {
            assert!(
                covered.contains(&variant),
                "ServerMessage variant {variant:?} has no sample row in server_message_table(); \
                 add one so the round-trip harness covers it"
            );
        }
    }

    #[test]
    fn client_message_table_round_trips() {
        for (discriminant, msg) in client_message_table() {
            assert_client_roundtrip(&JsonCodec, msg.clone());
            assert_client_roundtrip(&PrettyJsonCodec, msg.clone());
            assert_eq!(
                ClientMessageDiscriminants::from(&msg),
                discriminant,
                "table row discriminant mismatch for {msg:?}"
            );
        }
    }

    #[test]
    fn server_message_table_round_trips() {
        for (discriminant, msg) in server_message_table() {
            assert_server_roundtrip(&JsonCodec, msg.clone());
            assert_server_roundtrip(&PrettyJsonCodec, msg.clone());
            assert_eq!(
                ServerMessageDiscriminants::from(&msg),
                discriminant,
                "table row discriminant mismatch for {msg:?}"
            );
        }
    }

    // ── Wire-format string pins ────────────────────────────────────────────
    //
    // These assert an exact JSON string rather than just round-trip equality
    // — they pin the on-the-wire shape itself (important for cross-client /
    // JS-side compatibility), which the table-driven harness above does not
    // inherently cover.

    /// `encode_chatter` wire shape pin (issues #818, #975): the `"chatter"`
    /// host channel's JSON must expose exactly `from_label` / `to_label` and the
    /// TYPED `payload` — `__updateChatter` in `server.html` reads these keys and
    /// renders the payload through the shared coordination-popup normaliser. The
    /// host no longer receives a pre-composed sentence.
    #[test]
    fn encode_chatter_wire_shape_matches_js_handler() {
        let ev = crate::console_bridge::AiChatterEvent {
            from_label: "chatter.sender.sensors".into(),
            to_label: "tactical".into(),
            payload: crate::messages::CoordinationPayload::FrequencyHint { frequency: 0.5 },
        };
        let encoded = encode_chatter(&ev).unwrap();
        assert_eq!(
            encoded,
            r#"{"from_label":"chatter.sender.sensors","to_label":"tactical","payload":{"type":"FrequencyHint","data":{"frequency":0.5}}}"#,
            "chatter wire shape must match what __updateChatter parses: from_label / to_label / typed payload"
        );
    }

    /// `encode_chatter` must JSON-escape quotes/backslashes in the labels and in
    /// any text carried inside the payload — the pre-#818 hand-rolled `format!`
    /// encoder did this by hand; serde now owns it. Round-trips through
    /// `serde_json::Value` to prove the output is valid JSON with the original
    /// strings intact.
    #[test]
    fn encode_chatter_escapes_special_characters() {
        let ev = crate::console_bridge::AiChatterEvent {
            from_label: r#"AI "Sensors""#.into(),
            to_label: r"helm\aux".into(),
            payload: crate::messages::CoordinationPayload::Advisory {
                message: "line1\nline2".into(),
            },
        };
        let encoded = encode_chatter(&ev).unwrap();
        let v: serde_json::Value = serde_json::from_str(&encoded).expect("valid JSON");
        assert_eq!(v["from_label"], r#"AI "Sensors""#);
        assert_eq!(v["to_label"], r"helm\aux");
        assert_eq!(v["payload"]["data"]["message"], "line1\nline2");
    }

    #[test]
    fn client_control_system_json_shape_uses_string_ids() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("power-reactor".into()),
            payload: SystemControlPayload::SetPowerGroupAllocation {
                group: PowerGroupId("weapons".into()),
                level: 3,
            },
        };

        let encoded = JsonCodec.encode_client(&msg).unwrap();

        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"power-reactor","payload":{"type":"SetPowerGroupAllocation","data":{"group":"weapons","level":3}}}}"#
        );
    }

    /// Blaster fire command codec round-trip (issue #631).
    ///
    /// `FireBlaster` is a `SystemControlPayload` variant carried by
    /// `ClientMessage::ControlSystem`. This test verifies the JSON shape and
    /// round-trip fidelity for the fire action.
    #[test]
    fn fire_blaster_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("blaster-fore".into()),
            payload: SystemControlPayload::FireBlaster,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        // Pin the on-the-wire JSON shape — JS action-map.js depends on this.
        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"blaster-fore","payload":{"type":"FireBlaster"}}}"#,
            "FireBlaster wire shape must match what action-map.js sends"
        );
    }

    /// `SystemAffinity` round-trips including the `Comms` variant (issue #753).
    ///
    /// `SystemAffinity` is replicated inside `ScoredObjective` on the viewscreen
    /// blackboard, so the new `Comms` variant must survive the wire codec.
    #[test]
    fn system_affinity_comms_variant_round_trips() {
        let affinities = vec![
            SystemAffinity::Helm,
            SystemAffinity::Weapons,
            SystemAffinity::Captain,
            SystemAffinity::Comms,
        ];
        let encoded = serde_json::to_string(&affinities).unwrap();
        let decoded: Vec<SystemAffinity> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, affinities);
        assert!(
            encoded.contains("Comms"),
            "the Comms affinity variant must serialize by name"
        );
    }

    /// ChargeBlasterStart / ChargeBlasterCancel codec round-trips (issue #636).
    #[test]
    fn charge_blaster_start_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("blaster-fore".into()),
            payload: SystemControlPayload::ChargeBlasterStart,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"blaster-fore","payload":{"type":"ChargeBlasterStart"}}}"#,
            "ChargeBlasterStart wire shape must match what action-map.js sends"
        );
    }

    #[test]
    fn charge_blaster_cancel_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("blaster-fore".into()),
            payload: SystemControlPayload::ChargeBlasterCancel,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"blaster-fore","payload":{"type":"ChargeBlasterCancel"}}}"#,
            "ChargeBlasterCancel wire shape must match what action-map.js sends"
        );
    }

    /// Phaser fire as a ControlSystem payload (issue #846).
    #[test]
    fn fire_phaser_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("phaser-fore".into()),
            payload: SystemControlPayload::FirePhaser,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"phaser-fore","payload":{"type":"FirePhaser"}}}"#,
            "FirePhaser wire shape must match what action-map.js sends"
        );
    }

    /// Command stance selection as a ControlSystem payload (issue #1107).
    #[test]
    fn set_station_stance_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("command".into()),
            payload: SystemControlPayload::SetStationStance {
                station: StationId("tactical".into()),
                stance: "tactical-weapons-free".into(),
            },
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"command","payload":{"type":"SetStationStance","data":{"station":"tactical","stance":"tactical-weapons-free"}}}}"#,
            "SetStationStance wire shape must match what action-map.js sends"
        );
    }

    /// Torpedo fire as a ControlSystem payload (issue #846).
    #[test]
    fn fire_torpedo_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::FireTorpedo {
                target_uuid: Some("550e8400-e29b-41d4-a716-446655440000".into()),
            },
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"torpedo-tube-fore-port","payload":{"type":"FireTorpedo","data":{"target_uuid":"550e8400-e29b-41d4-a716-446655440000"}}}}"#,
            "FireTorpedo wire shape must match what action-map.js sends"
        );
    }

    /// Load tube as a ControlSystem payload (issue #846).
    #[test]
    fn load_tube_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-fore-port".into()),
            payload: SystemControlPayload::LoadTube,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"torpedo-tube-fore-port","payload":{"type":"LoadTube"}}}"#,
            "LoadTube wire shape must match what action-map.js sends"
        );
    }

    /// Unload tube as a ControlSystem payload (issue #846).
    #[test]
    fn unload_tube_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("torpedo-tube-aft".into()),
            payload: SystemControlPayload::UnloadTube,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"torpedo-tube-aft","payload":{"type":"UnloadTube"}}}"#,
            "UnloadTube wire shape must match what action-map.js sends"
        );
    }

    /// `CoordinationPayload::TargetDesignation` round-trip, embedded in both
    /// directions of the channel-3 bus (issue #676 — replaces the old direct
    /// `SensorsTargetSuggestion`).
    #[test]
    fn target_designation_coordination_payload_round_trips() {
        let send_msg = ClientMessage::SendCoordination {
            target: crate::system_registry::tactical_station_key(),
            payload: CoordinationPayload::TargetDesignation {
                uuid: "asteroid-42".into(),
                label: "Asteroid".into(),
            },
        };
        assert_client_roundtrip(&JsonCodec, send_msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, send_msg);

        let popup_msg = ServerMessage::CoordinationPopup {
            target: crate::system_registry::tactical_station_key(),
            payload: CoordinationPayload::TargetDesignation {
                uuid: "asteroid-42".into(),
                label: "Asteroid".into(),
            },
            sender_label: "Sensors".into(),
        };
        assert_server_roundtrip(&JsonCodec, popup_msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, popup_msg);
    }

    /// `CoordinationPayload::ArcBearingRequest` round-trip, embedded in both
    /// directions of the channel-3 bus (issue #677 — Weapons asks Helm to
    /// bring the phaser firing arc to bear).
    #[test]
    fn arc_bearing_request_coordination_payload_round_trips() {
        let send_msg = ClientMessage::SendCoordination {
            target: crate::system_registry::helm_station_key(),
            payload: CoordinationPayload::ArcBearingRequest {
                uuid: "hostile-7".into(),
                label: "Raider".into(),
                family: crate::messages::WeaponFamily::Blasters,
                arcs: vec![
                    crate::messages::WeaponEmitterArc {
                        facing_deg: 0.0,
                        arc_deg: 90.0,
                        range: 35.0,
                    },
                    crate::messages::WeaponEmitterArc {
                        facing_deg: 180.0,
                        arc_deg: 60.0,
                        range: 40.0,
                    },
                ],
            },
        };
        assert_client_roundtrip(&JsonCodec, send_msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, send_msg);

        let popup_msg = ServerMessage::CoordinationPopup {
            target: crate::system_registry::helm_station_key(),
            payload: CoordinationPayload::ArcBearingRequest {
                uuid: "hostile-7".into(),
                label: "Raider".into(),
                family: crate::messages::WeaponFamily::Torpedoes,
                arcs: vec![crate::messages::WeaponEmitterArc {
                    facing_deg: 0.0,
                    arc_deg: 45.0,
                    range: 120.0,
                }],
            },
            sender_label: "Weapons".into(),
        };
        assert_server_roundtrip(&JsonCodec, popup_msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, popup_msg);
    }

    /// `CoordinationPayload::ArcBearingWithdraw` round-trip (issue #932):
    /// Weapons withdraws a standing request once its emitting family goes
    /// unusable.
    #[test]
    fn arc_bearing_withdraw_coordination_payload_round_trips() {
        let send_msg = ClientMessage::SendCoordination {
            target: crate::system_registry::helm_station_key(),
            payload: CoordinationPayload::ArcBearingWithdraw {
                family: crate::messages::WeaponFamily::Torpedoes,
            },
        };
        assert_client_roundtrip(&JsonCodec, send_msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, send_msg);

        let popup_msg = ServerMessage::CoordinationPopup {
            target: crate::system_registry::helm_station_key(),
            payload: CoordinationPayload::ArcBearingWithdraw {
                family: crate::messages::WeaponFamily::Blasters,
            },
            sender_label: "Weapons".into(),
        };
        assert_server_roundtrip(&JsonCodec, popup_msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, popup_msg);
    }

    /// `CoordinationPayload::PowerBrownout` round-trip, embedded in both
    /// directions of the channel-3 bus (issue #678).
    #[test]
    fn power_brownout_coordination_payload_round_trips() {
        let send_msg = ClientMessage::SendCoordination {
            target: crate::system_registry::tactical_station_key(),
            payload: CoordinationPayload::PowerBrownout {
                group: "weapons".into(),
                label: "WEAPONS".into(),
                allocated_level: 2,
            },
        };
        assert_client_roundtrip(&JsonCodec, send_msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, send_msg);

        let popup_msg = ServerMessage::CoordinationPopup {
            target: crate::system_registry::tactical_station_key(),
            payload: CoordinationPayload::PowerBrownout {
                group: "weapons".into(),
                label: "WEAPONS".into(),
                allocated_level: 2,
            },
            sender_label: "Power".into(),
        };
        assert_server_roundtrip(&JsonCodec, popup_msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, popup_msg);
    }

    /// `CoordinationPayload::NavigateTo` round-trip, embedded in both
    /// directions of the channel-3 bus (issue #681 — Navigation clears Helm to
    /// follow the ship's waypoint).
    ///
    /// The `generation` is the navigation contract: the waypoint itself is the
    /// shared goal, `process_coordination_lag` latches only this, and it names
    /// which waypoint the Helm is cleared for. The `x` / `z` alongside it are
    /// display-only (issue #977 — the chatter popup formats them, replacing the
    /// English label Rust used to compose). The generation is a `u64`
    /// (not a timestamp) for PRD #620 lockstep determinism, so a value beyond
    /// f64's exact-integer range is used here to pin that it survives the JSON
    /// codec without precision loss — the failure mode a naive `f32`/`f64`
    /// generation would have.
    #[test]
    fn navigate_to_coordination_payload_round_trips() {
        let generation = u64::MAX - 1;
        let send_msg = ClientMessage::SendCoordination {
            target: crate::system_registry::helm_station_key(),
            payload: CoordinationPayload::NavigateTo {
                generation,
                x: 300.0,
                z: -100.0,
            },
        };
        assert_client_roundtrip(&JsonCodec, send_msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, send_msg);

        let popup_msg = ServerMessage::CoordinationPopup {
            target: crate::system_registry::helm_station_key(),
            payload: CoordinationPayload::NavigateTo {
                generation,
                x: 300.0,
                z: -100.0,
            },
            sender_label: "Navigation".into(),
        };
        assert_server_roundtrip(&JsonCodec, popup_msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, popup_msg);
    }

    /// `CoordinationPayload::RepairRequest` round-trip (issue #682 — damaged
    /// system pushes repair request to the Repair console).
    #[test]
    fn repair_request_coordination_payload_round_trips() {
        let send_msg = ClientMessage::SendCoordination {
            target: crate::system_registry::repair_system_id(),
            payload: CoordinationPayload::RepairRequest {
                system_id: crate::messages::SystemId("helm-radar".into()),
                station_id: "helm".into(),
                station_label: "Helm".into(),
                tier: crate::damage::DamageTier::Damaged,
                deficit: Some(12.5),
            },
        };
        assert_client_roundtrip(&JsonCodec, send_msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, send_msg);

        let popup_msg = ServerMessage::CoordinationPopup {
            target: crate::system_registry::repair_system_id(),
            payload: CoordinationPayload::RepairRequest {
                system_id: crate::messages::SystemId("helm-radar".into()),
                station_id: "helm".into(),
                station_label: "Helm".into(),
                tier: crate::damage::DamageTier::Disabled,
                // The wire form of a coarsened popup: tier crosses, exact
                // deficit withheld (issue #737).
                deficit: None,
            },
            sender_label: "Helm System".into(),
        };
        assert_server_roundtrip(&JsonCodec, popup_msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, popup_msg);
    }

    /// `CoordinationPayload::ThreatBearing` round-trip (issue #683 — sensors
    /// warns shields of incoming threat).
    #[test]
    fn threat_bearing_coordination_payload_round_trips() {
        let send_msg = ClientMessage::SendCoordination {
            target: crate::system_registry::shields_system_id(),
            payload: CoordinationPayload::ThreatBearing {
                bearing_rad: 0.698,
                label: "Hostile closing".into(),
            },
        };
        assert_client_roundtrip(&JsonCodec, send_msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, send_msg);

        let popup_msg = ServerMessage::CoordinationPopup {
            target: crate::system_registry::shields_system_id(),
            payload: CoordinationPayload::ThreatBearing {
                bearing_rad: 2.094,
                label: "Incoming torpedo".into(),
            },
            sender_label: "Sensors".into(),
        };
        assert_server_roundtrip(&JsonCodec, popup_msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, popup_msg);
    }

    /// `CoordinationPayload::IntentAdvisory` round-trip (issue #879 — a
    /// backfilled seat's coarsened intent advisory, broadcast to every human
    /// seat on the source ship).
    #[test]
    fn intent_advisory_coordination_payload_round_trips() {
        let popup_msg = ServerMessage::CoordinationPopup {
            target: crate::system_registry::tactical_station_key(),
            payload: CoordinationPayload::IntentAdvisory {
                kind: crate::messages::IntentKind::TargetSwitched,
                subject: Some("Harrow Raider".into()),
                generation: 4,
            },
            sender_label: "Tactical".into(),
        };
        assert_server_roundtrip(&JsonCodec, popup_msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, popup_msg);

        // The subject-less kinds, which serialise without the optional field.
        let bare = ServerMessage::CoordinationPopup {
            target: crate::system_registry::helm_station_key(),
            payload: CoordinationPayload::IntentAdvisory {
                kind: crate::messages::IntentKind::BreakingOff,
                subject: None,
                generation: 5,
            },
            sender_label: "Helm".into(),
        };
        assert_server_roundtrip(&JsonCodec, bare.clone());
        assert_server_roundtrip(&PrettyJsonCodec, bare);
    }

    /// BlasterFired server message round-trip (issue #631, extended #638).
    #[test]
    fn blaster_fired_server_message_round_trips() {
        let msg = ServerMessage::BlasterFired {
            bank: "fore".to_string(),
            source_uuid: "11111111-1111-1111-1111-111111111111".into(),
            projectile_id: "proj-uuid-abc".into(),
            x: 5.0,
            z: -10.0,
            heading: 1.57,
            visual_scale: 1.5,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    /// BlasterFired defaults visual_scale to 1.0 when absent (wire compat).
    #[test]
    fn blaster_fired_defaults_visual_scale_when_absent() {
        // Simulate an older wire message that has no visual_scale field.
        let json = r#"{"type":"BlasterFired","data":{"bank":"fore","source_uuid":"11111111-1111-1111-1111-111111111111","projectile_id":"proj-uuid-abc","x":5.0,"z":-10.0,"heading":1.57}}"#;
        let codec = crate::codec::JsonCodec;
        let decoded: ServerMessage = codec.decode_server(json).unwrap();
        if let ServerMessage::BlasterFired { visual_scale, .. } = decoded {
            assert!(
                (visual_scale - 1.0).abs() < f32::EPSILON,
                "visual_scale must default to 1.0 when absent from wire, got {visual_scale}"
            );
        } else {
            panic!("expected BlasterFired");
        }
    }

    // ── Parameterised text ids ────────────────────────────────────────────

    /// A text id with no parameter table encodes EXACTLY as it did before the
    /// table existed — the whole basis on which this could be added to a shipped
    /// wire without a revision bump, and what makes the change digest-neutral.
    ///
    /// Pinned as a literal rather than as "has no `text_params` key", because
    /// the claim is about bytes: a reader that never heard of the field must see
    /// the same string, in the same order, with the same punctuation.
    #[test]
    fn an_objective_with_no_params_is_byte_identical_to_the_pre_params_wire() {
        let encoded = JsonCodec
            .encode_server(&ServerMessage::ObjectiveSummary {
                objectives: vec![ObjectiveSnapshot {
                    id: "obj-a3-window".into(),
                    text: "world.falling_skyway.objective.window.text".into(),
                    text_params: Default::default(),
                    mandatory: true,
                    status: ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                }],
            })
            .unwrap();

        assert_eq!(
            encoded,
            r#"{"type":"ObjectiveSummary","data":{"objectives":[{"id":"obj-a3-window","text":"world.falling_skyway.objective.window.text","mandatory":true,"status":"Active","source":"Mission"}]}}"#,
            "an objective naming a figure-free string must encode as it always did"
        );
    }

    /// The same for a comms body, which carries its table under `body_params`.
    #[test]
    fn a_comms_message_with_no_params_carries_no_params_key() {
        let msg = crate::messages::CommsMessage::injected(
            "m1".into(),
            "u1".into(),
            "entity.skyway_control.name".into(),
            "world.falling_skyway.comms.window_closes".into(),
            Default::default(),
            vec![],
            "t1".into(),
            true,
            false,
        );
        let encoded = serde_json::to_string(&msg).unwrap();
        assert!(
            !encoded.contains("body_params"),
            "an empty table must not appear on the wire at all, got {encoded}"
        );
    }

    /// A non-empty table rides beside the id, and its keys are in sorted order —
    /// the `BTreeMap` property the encoding's determinism rests on. A `HashMap`
    /// would pass an "is the key present" assertion and still emit these three
    /// names in a different order on a different run.
    #[test]
    fn objective_text_params_ride_the_wire_in_sorted_key_order() {
        let params = ["shortfall", "available", "claimed"]
            .into_iter()
            .enumerate()
            .map(|(i, k)| (k.to_string(), i.to_string()))
            .collect();
        let encoded = JsonCodec
            .encode_server(&ServerMessage::ObjectiveSummary {
                objectives: vec![ObjectiveSnapshot {
                    id: "obj".into(),
                    text: "some.id".into(),
                    text_params: params,
                    mandatory: true,
                    status: ObjectiveStatus::Active,
                    targets: vec![],
                    source: crate::messages::ObjectiveSource::Mission,
                }],
            })
            .unwrap();

        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        let obj = &value["data"]["objectives"][0];
        assert_eq!(
            obj.as_object()
                .expect("objective is an object")
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<&str>>(),
            std::collections::BTreeSet::from([
                "id",
                "text",
                "text_params",
                "mandatory",
                "status",
                "source"
            ]),
            "the params table is the only key a figure-carrying objective adds"
        );

        // Sorted, not insertion order: the map was built shortfall/available/claimed.
        let rendered = encoded
            .split_once("\"text_params\":")
            .expect("text_params is present")
            .1;
        assert!(
            rendered.starts_with(r#"{"available":"1","claimed":"2","shortfall":"0"}"#),
            "keys must serialise in sorted order, got {rendered}"
        );

        // And it survives the round trip it will actually make.
        let decoded = JsonCodec.decode_server(&encoded).unwrap();
        let ServerMessage::ObjectiveSummary { objectives } = decoded else {
            panic!("expected ObjectiveSummary");
        };
        assert_eq!(objectives[0].text_params["shortfall"], "0");
    }

    /// A payload written by a peer that predates the field still decodes — the
    /// `serde(default)` half of the contract.
    #[test]
    fn an_objective_without_the_params_key_still_decodes() {
        let legacy = r#"{"type":"ObjectiveSummary","data":{"objectives":[{"id":"o","text":"t","mandatory":false,"status":"Active","source":"Mission"}]}}"#;
        let ServerMessage::ObjectiveSummary { objectives } =
            JsonCodec.decode_server(legacy).unwrap()
        else {
            panic!("expected ObjectiveSummary");
        };
        assert!(objectives[0].text_params.is_empty());
    }

    // ── GameOver carries the authored outcome (PRD #1023 module 4) ────────

    /// The whole surface of the message the game-over screen reads. `outcome`
    /// is written even when it is `null`, so the client tests one shape.
    #[test]
    fn game_over_wire_keys_are_reason_and_outcome() {
        let encoded = JsonCodec
            .encode_server(&ServerMessage::GameOver {
                reason: "world.falling_skyway.ending.held".into(),
                outcome: Some("victory".into()),
            })
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        assert_eq!(
            value["data"]
                .as_object()
                .expect("GameOver data is an object")
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<&str>>(),
            std::collections::BTreeSet::from(["reason", "outcome"]),
            "the ending's whole surface: what happened, and which side it was"
        );
        assert_eq!(value["data"]["outcome"], "victory");

        // Still written when there is no declared side, because a key that
        // came and went would make absence and defeat look alike to a client
        // testing for the field rather than its value.
        let undeclared = JsonCodec
            .encode_server(&ServerMessage::GameOver {
                reason: "r".into(),
                outcome: None,
            })
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&undeclared).unwrap();
        assert!(value["data"].as_object().unwrap().contains_key("outcome"));
        assert!(value["data"]["outcome"].is_null());
    }

    #[test]
    fn game_over_outcome_round_trips_and_defaults_when_absent() {
        for outcome in [Some("victory".to_string()), Some("defeat".into()), None] {
            assert_server_roundtrip(
                &JsonCodec,
                ServerMessage::GameOver {
                    reason: "server.game_over.ship_destroyed".into(),
                    outcome: outcome.clone(),
                },
            );
        }

        // A peer still sending the pre-#1023 `{reason}` shape decodes as an
        // undeclared ending rather than failing the message.
        let legacy = r#"{"type":"GameOver","data":{"reason":"Ship destroyed"}}"#;
        match JsonCodec.decode_server(legacy).unwrap() {
            ServerMessage::GameOver { reason, outcome } => {
                assert_eq!(reason, "Ship destroyed");
                assert_eq!(outcome, None);
            }
            other => panic!("expected GameOver, got {other:?}"),
        }
    }

    // ── Human-seeking hosts on the wire (issue #984) ──────────────────────

    /// `host_station` is how the resolved seek reaches a console, and it rides
    /// the seeking system's own blackboard. The key is always written — the
    /// client's push router recognises a seeking system BY that key, so a key
    /// that disappeared on `None` would make "the seek let go" unroutable.
    #[test]
    fn seeking_blackboards_always_carry_a_host_station_key() {
        let comms = serde_json::to_value(crate::messages::SystemBlackboard::Comms(
            crate::messages::CommsBlackboard::default(),
        ))
        .unwrap();
        // Adjacently tagged (`{"kind":…,"data":…}`) — the shape
        // gui/sim-state.js unwraps and gui/dirty-consoles.js inspects.
        assert_eq!(comms["kind"], "Comms");
        assert!(
            comms["data"]
                .as_object()
                .expect("a comms blackboard is an object")
                .contains_key("host_station"),
            "an unhosted comms blackboard still names the field"
        );
        assert!(comms["data"]["host_station"].is_null());

        let nav = serde_json::to_value(crate::messages::SystemBlackboard::Navigation(
            crate::messages::NavigationBlackboard {
                host_station: Some(StationId("engineering".into())),
                ..Default::default()
            },
        ))
        .unwrap();
        assert_eq!(nav["kind"], "Navigation");
        assert_eq!(nav["data"]["host_station"], "engineering");
    }

    #[test]
    fn seeking_blackboards_default_their_host_when_a_peer_omits_it() {
        // The pre-#984 shape: every other field, no `host_station`.
        let legacy = r#"{"messages":[],"objectives":[],"contacts":[]}"#;
        let bb: crate::messages::CommsBlackboard = serde_json::from_str(legacy).unwrap();
        assert_eq!(bb.host_station, None);
    }

    /// BlasterHit server message round-trip (issue #631).
    #[test]
    fn blaster_hit_server_message_round_trips() {
        let msg = ServerMessage::BlasterHit {
            bank: "fore".to_string(),
            projectile_id: "proj-uuid-abc".into(),
            target_uuid: "550e8400-e29b-41d4-a716-446655440000".into(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    /// Torpedo volley target command codec round-trip (issue #632).
    ///
    /// `SetTorpedoVolleyTarget` is a `SystemControlPayload` variant carried by
    /// `ClientMessage::ControlSystem`. The target SystemId addresses a specific
    /// torpedo tube (e.g. `"torpedo-tube-fore-port"`).
    #[test]
    fn set_torpedo_volley_target_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: crate::system_registry::torpedo_tube_system_id("fore_port")
                .expect("fore_port resolves"),
            payload: SystemControlPayload::SetTorpedoVolleyTarget { count: 3 },
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        // Pin the on-the-wire JSON shape — action-map.js depends on this.
        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"torpedo-tube-fore-port","payload":{"type":"SetTorpedoVolleyTarget","data":{"count":3}}}}"#,
            "SetTorpedoVolleyTarget wire shape must match what action-map.js sends"
        );
    }

    /// SetRedAlert command round-trip (issue #748).
    ///
    /// `SetRedAlert { active }` is a `SystemControlPayload` variant carried by
    /// `ClientMessage::ControlSystem` targeting `red-alert`. Both the captain
    /// UI and the Captain AI send the desired end state, so the wire shape must
    /// match what `gui/action-map.js` sends.
    #[test]
    fn set_red_alert_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("red-alert".into()),
            payload: SystemControlPayload::SetRedAlert { active: true },
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        // Pin the on-the-wire JSON shape — action-map.js depends on this.
        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"red-alert","payload":{"type":"SetRedAlert","data":{"active":true}}}}"#,
            "SetRedAlert wire shape must match what action-map.js sends"
        );

        // The inactive request must round-trip identically.
        let off = ClientMessage::ControlSystem {
            target: SystemId("red-alert".into()),
            payload: SystemControlPayload::SetRedAlert { active: false },
        };
        assert_client_roundtrip(&JsonCodec, off.clone());
        assert_client_roundtrip(&PrettyJsonCodec, off);
    }

    /// SetRepairPriority command round-trip (issue #739).
    #[test]
    fn set_repair_priority_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("repair".into()),
            payload: SystemControlPayload::SetRepairPriority {
                team_idx: 1,
                priority: 2,
            },
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        // Pin the on-the-wire JSON shape — action-map.js depends on this.
        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"repair","payload":{"type":"SetRepairPriority","data":{"team_idx":1,"priority":2}}}}"#,
            "SetRepairPriority wire shape must match what action-map.js sends"
        );
    }

    /// SetRepairTargetPriority command round-trip (issue #1015) — the repair
    /// console's damaged-systems taps. Unlike `SetRepairPriority` above it
    /// carries no ordinal at all: the host resolves which team's sweep covers
    /// the named system and pins that system directly, because #737 hides
    /// most of the candidates from the console.
    #[test]
    fn set_repair_target_priority_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId("repair".into()),
            payload: SystemControlPayload::SetRepairTargetPriority {
                system_id: SystemId("helm-engine-port".into()),
            },
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        // Pin the on-the-wire JSON shape — repair-dispatch.js depends on this.
        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"repair","payload":{"type":"SetRepairTargetPriority","data":{"system_id":"helm-engine-port"}}}}"#,
            "SetRepairTargetPriority wire shape must match what repair-dispatch.js sends"
        );
    }

    /// The console-facing half of issue #1015: the pin the host resolved a tap
    /// to rides home on the `Repairing` slot, so `normalizeTeamSlot` in
    /// `gui/console-state.js` can highlight the tapped row.
    ///
    /// `#[serde(default)]` on the new field is what keeps a pre-#1015 snapshot
    /// (issue #862 restores `TeamSlot`s verbatim) loadable, so the absent-field
    /// decode is pinned here too.
    #[test]
    fn repairing_slot_carries_its_priority_pin() {
        let slot = TeamSlot::Repairing {
            system_id: Some(SystemId("helm-engine-port".into())),
            display_name: Some("Port Engine".into()),
            priority: Some(2),
            priority_system_id: Some(SystemId("helm-engine-starboard".into())),
        };
        let encoded = serde_json::to_string(&slot).unwrap();
        assert_eq!(
            encoded,
            r#"{"Repairing":{"system_id":"helm-engine-port","display_name":"Port Engine","priority":2,"priority_system_id":"helm-engine-starboard"}}"#,
            "the repair console reads `priority_system_id` off this exact shape"
        );
        assert_eq!(serde_json::from_str::<TeamSlot>(&encoded).unwrap(), slot);

        let legacy = r#"{"Repairing":{"system_id":"helm","display_name":"Helm","priority":1}}"#;
        assert_eq!(
            serde_json::from_str::<TeamSlot>(legacy).unwrap(),
            TeamSlot::Repairing {
                system_id: Some(SystemId("helm".into())),
                display_name: Some("Helm".into()),
                priority: Some(1),
                priority_system_id: None,
            },
            "a slot serialised before #1015 must still decode"
        );
    }

    /// ToggleGodMode command round-trip (issue #900). Sent from
    /// `bridge::drain_god_mode_toggle` (not JS directly — the wasm export
    /// keeps its old zero-argument signature) under `LOCAL_CONSOLE_TOKEN`, but
    /// the wire shape it produces is pinned here the same way every other
    /// `ControlSystem` payload is.
    #[test]
    fn toggle_god_mode_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId(crate::ship::system_registry::GOD_MODE_SYSTEM_ID.into()),
            payload: SystemControlPayload::ToggleGodMode,
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"god-mode","payload":{"type":"ToggleGodMode"}}}"#,
            "ToggleGodMode wire shape must stay pinned"
        );
    }

    /// The operation start/abort commands (issue #1026).
    ///
    /// Both target the real, station-owned `captain` system rather than an
    /// operations system id of their own — an operation is something a ship
    /// does, not a thing aboard it — so the wire shape pinned here is what gives
    /// them the ordinary station-tenure admission check.
    #[test]
    fn operation_start_and_abort_control_systems_round_trip() {
        let start = ClientMessage::ControlSystem {
            target: SystemId(crate::ship::system_registry::CAPTAIN_SYSTEM_ID.into()),
            payload: SystemControlPayload::StartOperation {
                verb: crate::operations::OperationVerb::Stabilise,
                target_uuid: "00000000-0000-8000-8000-000000000042".into(),
            },
        };
        assert_client_roundtrip(&JsonCodec, start.clone());
        assert_client_roundtrip(&PrettyJsonCodec, start.clone());
        let encoded = JsonCodec.encode_client(&start).unwrap();
        assert!(
            encoded.contains(r#""verb":"stabilise""#),
            "the verb crosses in its authored snake_case spelling — the same one the TOML \
             `verb` field and the script effect use: {encoded}"
        );

        let abort = ClientMessage::ControlSystem {
            target: SystemId(crate::ship::system_registry::CAPTAIN_SYSTEM_ID.into()),
            payload: SystemControlPayload::AbortOperation,
        };
        assert_client_roundtrip(&JsonCodec, abort.clone());
        let encoded = JsonCodec.encode_client(&abort).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"captain","payload":{"type":"AbortOperation"}}}"#,
            "AbortOperation wire shape must stay pinned"
        );
    }

    /// The operations blackboard (issue #1026), and the claim that adding it
    /// broke nobody.
    #[test]
    fn system_blackboard_operations_round_trips_and_is_additive() {
        let bb = SystemBlackboard::Operations(crate::messages::OperationsBlackboard {
            capabilities: vec![crate::messages::CapabilityOffer {
                verb: "stabilise".into(),
                label: "operation.verb.stabilise".into(),
            }],
            active: Some(crate::messages::ActiveOperationSnapshot {
                id: 3,
                verb: "stabilise".into(),
                verb_label: "operation.verb.stabilise".into(),
                target_uuid: "00000000-0000-8000-8000-000000000042".into(),
                target_name: Some("world.entity.skyhook_depot.name".into()),
                progress: 0.25,
                state: "stalled".into(),
                reason: Some("operation.refused.out_of_range".into()),
                // Issue #1027: a hazard band stretching the work.
                rate_percent: 40,
            }),
            refusal: None,
        });

        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains(r#""kind":"Operations""#), "got: {json}");
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);

        // A ship that authored `[operations]` and is running nothing pays for
        // neither the hold nor the refusal.
        let idle = SystemBlackboard::Operations(crate::messages::OperationsBlackboard::default());
        let json = serde_json::to_string(&idle).unwrap();
        assert!(
            !json.contains("active") && !json.contains("reason") && !json.contains("refusal"),
            "an idle operations blackboard must carry no absent-field noise: {json}"
        );
        assert_eq!(
            serde_json::from_str::<SystemBlackboard>(&json).unwrap(),
            idle
        );

        // ADDITIVE ON THE WIRE. A blackboard payload minted before this variant
        // existed still decodes, because the enum's other variants are
        // untouched and every field this one adds is its own.
        let legacy = r#"{"kind":"Captain","data":{"red_alert":false,"view_direction":"fore",
            "hull_integrity_pct":100.0}}"#;
        assert!(
            matches!(
                serde_json::from_str::<SystemBlackboard>(legacy).unwrap(),
                SystemBlackboard::Captain(_)
            ),
            "adding a variant must not move any other variant's decoding"
        );
        // …and the operations blackboard itself decodes from a payload carrying
        // only the fields the first shipped version had, so a later field is
        // additive on the same terms.
        let minimal = r#"{"kind":"Operations","data":{}}"#;
        assert_eq!(
            serde_json::from_str::<SystemBlackboard>(minimal).unwrap(),
            SystemBlackboard::Operations(crate::messages::OperationsBlackboard::default()),
            "every field on the blackboard is `#[serde(default)]`, so a payload that predates \
             any one of them decodes rather than being refused whole"
        );

        // Issue #1027 added `rate_percent` to the live hold. A payload minted
        // before it existed has to decode as the NORMAL rate rather than as
        // zero — a hold that a stale host reported as stopped would read to the
        // crew as an operation that had died.
        let pre_1027 = r#"{"kind":"Operations","data":{"capabilities":[],"active":{
            "id":1,"verb":"stabilise","verb_label":"operation.verb.stabilise",
            "target_uuid":"depot-1","progress":0.5,"state":"holding"}}}"#;
        let SystemBlackboard::Operations(decoded) =
            serde_json::from_str::<SystemBlackboard>(pre_1027).unwrap()
        else {
            panic!("an operations payload decodes as one");
        };
        assert_eq!(
            decoded.active.expect("the hold decodes").rate_percent,
            100,
            "an absent rate is full speed, which is what every payload written before hazard \
             bands existed meant"
        );

        let msg = ServerMessage::BlackboardUpdate {
            updates: vec![(
                SystemId(crate::operations::OPERATIONS_BLACKBOARD_KEY.into()),
                bb,
            )],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    /// The scan command (issue #1032).
    ///
    /// It targets the real, station-owned `sensors` system rather than a scan
    /// system id of its own — the suite is the thing aboard the ship that can be
    /// damaged and commanded, the reading is not — so the wire shape pinned here
    /// is what gives it the ordinary station-tenure admission check, the same one
    /// `SetScienceTarget` takes.
    #[test]
    fn scan_target_control_system_round_trips() {
        let msg = ClientMessage::ControlSystem {
            target: SystemId(crate::ship::system_registry::SENSORS_SYSTEM_ID.into()),
            payload: SystemControlPayload::ScanTarget {
                uuid: "00000000-0000-8000-8000-000000000042".into(),
            },
        };
        assert_client_roundtrip(&JsonCodec, msg.clone());
        assert_client_roundtrip(&PrettyJsonCodec, msg.clone());

        let encoded = JsonCodec.encode_client(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"sensors","payload":{"type":"ScanTarget","data":{"uuid":"00000000-0000-8000-8000-000000000042"}}}}"#,
            "ScanTarget wire shape must stay pinned"
        );
    }

    /// The scan blackboard (issue #1032), and the derivation-purity guarantee
    /// stated where the payload is actually made: **as the reading's whole key
    /// set**.
    ///
    /// `pasm/spec/design/simulation-differentiation.yaml` says a sensor readout
    /// must not be "scripted exposition dressed as sensor output". In a language
    /// with no reflection, the enforceable form of that is the assertion below:
    /// a reading has exactly these ten keys, every one of them a quantity read
    /// off the subject's condition track (or its content identity, for `mass` —
    /// issue #1154) or a `strings.csv` id an author wrote against a quantity —
    /// and none of them a result, a summary, a narration or a description. A
    /// `scan_text` field would have to be added here, in a diff, moving this
    /// test.
    #[test]
    fn system_blackboard_scan_round_trips_and_carries_no_field_for_authored_prose() {
        use crate::messages::{ScanBlackboard, ScanReadingSnapshot};
        use std::collections::BTreeSet;

        let reading = ScanReadingSnapshot {
            subject_uuid: "00000000-0000-8000-8000-000000000042".into(),
            subject_name: "world.entity.skyhook.name".into(),
            band: "detailed".into(),
            band_label: "entity.alliance_destroyer.scan.band.detailed.label".into(),
            taken_at_tick: 900,
            condition_fraction: 0.31,
            condition_step: 0.01,
            mass: 250_000.0,
            flags: vec![("world.skyhook.transfer.label".into(), false)],
            capacities: vec![("world.skyhook.berths.label".into(), 4)],
        };

        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&reading).unwrap()).unwrap();
        assert_eq!(
            value
                .as_object()
                .expect("a reading is an object")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<&str>>(),
            BTreeSet::from([
                "subject_uuid",
                "subject_name",
                "band",
                "band_label",
                "taken_at_tick",
                "condition_fraction",
                "condition_step",
                "mass",
                "flags",
                "capacities",
            ]),
            "the reading's whole surface — every key is a measured quantity, its \
             content identity, or a label an author wrote against one, and there \
             is nowhere for a written-out scan result to ride"
        );

        let bb = SystemBlackboard::Scan(ScanBlackboard {
            capable: true,
            reading: Some(reading),
            refusal: None,
        });
        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains(r#""kind":"Scan""#), "got: {json}");
        assert_eq!(serde_json::from_str::<SystemBlackboard>(&json).unwrap(), bb);

        // A refused scan carries the reason and no stale reading.
        let refused = SystemBlackboard::Scan(ScanBlackboard {
            capable: true,
            reading: None,
            refusal: Some(crate::science::ScanRefusal::OutOfRange.string_id().into()),
        });
        let json = serde_json::to_string(&refused).unwrap();
        assert!(
            !json.contains("reading") && json.contains("scan.refusal.out_of_range"),
            "a refusal replaces the reading rather than sitting beside it: {json}"
        );
        assert_eq!(
            serde_json::from_str::<SystemBlackboard>(&json).unwrap(),
            refused
        );

        // ADDITIVE ON THE WIRE: adding this variant moved no other variant's
        // decoding, and every field on it is `#[serde(default)]`.
        assert_eq!(
            serde_json::from_str::<SystemBlackboard>(r#"{"kind":"Scan","data":{}}"#).unwrap(),
            SystemBlackboard::Scan(ScanBlackboard::default())
        );

        let msg = ServerMessage::BlackboardUpdate {
            updates: vec![(SystemId(crate::science::SCAN_BLACKBOARD_KEY.into()), bb)],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    /// The dossier blackboard (issue #1030), and the hidden-truth guarantee
    /// stated where the payload is actually made: **as the payload's whole key
    /// set**.
    ///
    /// This is the wire half of `dossier::projection`'s own tests. In a language
    /// with no reflection, "structurally absent, not filtered at render time"
    /// means the serialised dossier has exactly these five keys and a fact
    /// exactly two — so a field a secret could ride in would have to be added
    /// here, in a diff, moving this test. A render-time filter would not satisfy
    /// it: the assertion is over the type's whole surface.
    #[test]
    fn system_blackboard_dossiers_round_trips_and_carries_no_field_for_a_secret() {
        use crate::messages::{
            DossierBlackboard, DossierEvidenceSnapshot, DossierFactSnapshot, DossierSnapshot,
            DossierValue,
        };
        use std::collections::BTreeSet;

        let dossier = DossierSnapshot {
            uuid: "00000000-0000-8000-8000-000000000042".into(),
            name: "world.entity.skyhook.name".into(),
            summary: "world.entity.skyhook.description".into(),
            facts: vec![
                DossierFactSnapshot {
                    label: crate::dossier::FACT_FACTION.into(),
                    value: DossierValue::Text("faction.federation.display_name".into()),
                },
                DossierFactSnapshot {
                    label: crate::dossier::FACT_COMMS.into(),
                    value: DossierValue::Flag(true),
                },
                DossierFactSnapshot {
                    label: crate::dossier::FACT_CONDITION.into(),
                    value: DossierValue::Fraction(0.5),
                },
                DossierFactSnapshot {
                    label: "world.skyhook.berths.label".into(),
                    value: DossierValue::Count(4),
                },
            ],
            evidence: Vec::new(),
        };

        // The key set, read off the type rather than off this instance.
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&dossier).unwrap()).unwrap();
        let keys: BTreeSet<&str> = value
            .as_object()
            .expect("a dossier is an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from(["uuid", "name", "summary", "facts", "evidence"]),
            "the dossier's whole surface — there is nowhere for hidden truth to ride"
        );
        for fact in value["facts"].as_array().unwrap() {
            let keys: BTreeSet<&str> = fact
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                BTreeSet::from(["label", "value"]),
                "a fact is a label and a value; there is no third column"
            );
        }
        assert_eq!(
            value["facts"][2]["value"],
            serde_json::json!({"kind": "fraction", "value": 0.5}),
            "a value is TAGGED, so the panel formats a percentage rather than guessing"
        );

        let bb = SystemBlackboard::Dossiers(DossierBlackboard {
            subjects: vec![dossier],
        });
        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains(r#""kind":"Dossiers""#), "got: {json}");
        assert_eq!(serde_json::from_str::<SystemBlackboard>(&json).unwrap(), bb);

        // The #1031 seam, pinned by #1030 before anything wrote to it and kept
        // BYTE-FOR-BYTE here now that something does: appending entries was
        // additive, and this literal is the proof — the slice that filled the
        // list did not have to move a character of it.
        let with_evidence = r#"{"kind":"Dossiers","data":{"subjects":[{"uuid":"u","name":"n",
            "facts":[],"evidence":[{"text":"world.x.evidence","provenance":"scan",
            "gathered_at_tick":900}]}]}}"#;
        assert_eq!(
            serde_json::from_str::<SystemBlackboard>(with_evidence).unwrap(),
            SystemBlackboard::Dossiers(DossierBlackboard {
                subjects: vec![DossierSnapshot {
                    uuid: "u".into(),
                    name: "n".into(),
                    summary: String::new(),
                    facts: Vec::new(),
                    evidence: vec![DossierEvidenceSnapshot {
                        text: "world.x.evidence".into(),
                        provenance: "scan".into(),
                        gathered_at_tick: 900,
                    }],
                }],
            })
        );

        // Every field is `#[serde(default)]`, so a payload minted before any one
        // of them decodes rather than being refused whole.
        assert_eq!(
            serde_json::from_str::<SystemBlackboard>(r#"{"kind":"Dossiers","data":{}}"#).unwrap(),
            SystemBlackboard::Dossiers(DossierBlackboard::default())
        );

        // An evidence ENTRY's whole key set, asserted the same way a fact's is
        // (issue #1031). Three columns and no fourth: what was learned, how, and
        // when. There is deliberately no "actual value" beside the reported one
        // and no confidence score — a scenario that misleads the crew authors the
        // misleading finding and the contradiction as two entries they can
        // compare, which is why inspecting this payload cannot reveal anything
        // they were not shown.
        let entry: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&DossierEvidenceSnapshot {
                text: "world.x.evidence".into(),
                provenance: "dialogue".into(),
                gathered_at_tick: 900,
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            entry
                .as_object()
                .expect("an entry is an object")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<&str>>(),
            BTreeSet::from(["text", "provenance", "gathered_at_tick"]),
        );

        // And the provenance crosses as the SCRIPT's own name, so the panel's
        // PROVENANCE_LABELS keys, a scenario's `provenance: "scan"` and a save
        // are one vocabulary rather than three.
        for provenance in crate::dossier::EvidenceProvenance::ALL {
            assert_eq!(
                serde_json::to_string(&provenance).unwrap(),
                format!("\"{}\"", provenance.as_str())
            );
        }

        let msg = ServerMessage::BlackboardUpdate {
            updates: vec![(SystemId(crate::dossier::DOSSIER_BLACKBOARD_KEY.into()), bb)],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    /// The phone settings menu's wire shapes (issue #940), pinned because
    /// `gui/settings-panel.js` hand-builds them: the two messages it sends and
    /// the `DebugState` read-back it folds.
    ///
    /// The two client messages are `#[cfg]`-gated with the variants they name;
    /// the read-back is not, because it is reported in every build.
    #[test]
    fn client_settings_menu_wire_shapes_are_pinned() {
        use crate::messages::DebugFlag;

        #[cfg(not(phoenix_demo_build))]
        {
            let msg = ClientMessage::ToggleDebugFlag {
                flag: DebugFlag::Regions,
            };
            assert_client_roundtrip(&JsonCodec, msg.clone());
            assert_eq!(
                JsonCodec.encode_client(&msg).unwrap(),
                r#"{"type":"ToggleDebugFlag","data":{"flag":"Regions"}}"#,
                "ToggleDebugFlag wire shape must match what settings-panel.js sends"
            );

            // A unit variant, so the adjacently-tagged encoding omits `data`
            // entirely — exactly what `connection-manager.js` puts on the wire
            // when `send()` is given no data, which is how `settings-panel.js`
            // sends it and how `ReleaseStation` has always been sent. Pinned
            // because the JS builds this by hand.
            let pause = ClientMessage::TogglePause;
            assert_client_roundtrip(&JsonCodec, pause.clone());
            assert_eq!(
                JsonCodec.encode_client(&pause).unwrap(),
                r#"{"type":"TogglePause"}"#,
                "TogglePause wire shape must match what settings-panel.js sends"
            );
            // An explicit null is the same message; an empty object is NOT, and
            // the JS must not start sending one.
            assert!(JsonCodec
                .decode_client(r#"{"type":"TogglePause","data":null}"#)
                .is_ok());
            assert!(
                JsonCodec
                    .decode_client(r#"{"type":"TogglePause","data":{}}"#)
                    .is_err(),
                "`data: {{}}` is not this message — settings-panel.js sends no data at all"
            );
        }

        let report = ServerMessage::DebugState {
            flags: vec![(DebugFlag::Regions, true), (DebugFlag::Modifiers, false)],
            paused: true,
            god_mode: true,
        };
        assert_server_roundtrip(&JsonCodec, report.clone());
        assert_eq!(
            JsonCodec.encode_server(&report).unwrap(),
            r#"{"type":"DebugState","data":{"flags":[["Regions",true],["Modifiers",false]],"paused":true,"god_mode":true}}"#,
            "DebugState wire shape must match what settings-panel.js folds"
        );
    }

    // ── The demo build's missing routes (issue #940) ─────────────────────────
    //
    // **These are the gate tests, and they are the only ones that ask the
    // question the way an attacker would: over the wire.** Everything else
    // about the client settings menu is checked through a Rust predicate, and a
    // predicate can only be consulted by code that chose to consult it. These
    // two decode a raw JSON frame — exactly what `bridge.rs` hands the codec
    // when a phone speaks — and assert it is understood in a dev build and
    // meaningless in a demo one.
    //
    // HONESTLY, WHAT THESE SEE, and what they leave to their neighbour: they
    // pin the ROUTE to the cfg. A variant that quietly loses its `#[cfg]`
    // decodes in the demo run and fails here; one that gains a stray `#[cfg]`
    // fails the dev run. What they cannot see is the cfg itself being wrong —
    // they ask `is_demo_cfg()` the same question the code under test asks, so
    // a `build.rs` that stopped tracking `PHOENIX_DEMO_BUILD` would move both
    // sides together and these would still pass. That half is
    // `build_flags::the_cfg_gate_and_the_runtime_flag_agree`, which compares
    // the cfg against the `option_env!` read of the same variable. The two
    // tests are in the same CI step for exactly that reason; neither is
    // sufficient alone. (Checked, not assumed: inverting `build.rs` turns that
    // test red under `PHOENIX_DEMO_BUILD=true`.)

    /// A demo binary cannot be told to draw a debug overlay by a phone: the
    /// variant is not compiled, so the frame does not parse.
    #[test]
    fn the_client_debug_route_is_absent_from_a_demo_build() {
        let decoded =
            JsonCodec.decode_client(r#"{"type":"ToggleDebugFlag","data":{"flag":"Regions"}}"#);
        assert_eq!(
            decoded.is_ok(),
            !crate::build_flags::is_demo_cfg(),
            "ToggleDebugFlag must decode in a dev build and be an unknown \
             message in a demo build — a hidden Debug/Cheat tab is a forgeable \
             UI fact, so the wire shape has to go too"
        );
    }

    /// A demo binary cannot be paused by a phone. This is the one that matters
    /// in play: a demo is N strangers on N phones, any one of whom could
    /// otherwise freeze the mission for everyone, repeatedly, with nothing in
    /// the drain checking station, captaincy or `GamePhase`.
    ///
    /// The host's own pause (issue #939) is a different path — a `wasm_*`
    /// export the host page calls directly — and is deliberately untouched in
    /// every build.
    #[test]
    fn the_client_pause_route_is_absent_from_a_demo_build() {
        for frame in [
            r#"{"type":"TogglePause"}"#,
            r#"{"type":"TogglePause","data":null}"#,
        ] {
            assert_eq!(
                JsonCodec.decode_client(frame).is_ok(),
                !crate::build_flags::is_demo_cfg(),
                "a demo build must not understand {frame} from any phone"
            );
        }
    }

    /// `TorpedoTubeState` with non-default volley fields round-trips (issue #632).
    #[test]
    fn torpedo_tube_state_volley_fields_round_trip() {
        use crate::messages::{PhaserMode, TorpedoTubeState};
        let msg = ServerMessage::WeaponsUpdate {
            target_uuid: None,
            target_name: None,
            banks: vec![],
            tubes: vec![TorpedoTubeState {
                id: "fore_port".to_string(),
                loaded: true,
                reload_secs: 3.5,
                state: "loading".into(),
                progress: 0.65,
                load_time: 10.0,
                volley_max: 4,
                loaded_count: 2,
                target_count: 4,
                load_progress: 0.65,
                readiness: WeaponReadiness::default(),
                // Patterned attack in progress (issue #766): step 2 of 3,
                // barrel 1 firing this round.
                active_barrels: vec![1],
                pattern_step: 2,
                pattern_len: 3,
            }],
            torpedo_count: 8,
            phaser_mode: PhaserMode::Auto,
            blasters: vec![],
            phaser_frequency: 0.5,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    /// `TorpedoTubeState` patterned-attack fields round-trip (issue #766).
    #[test]
    fn torpedo_tube_state_pattern_fields_round_trip() {
        use crate::messages::{PhaserMode, TorpedoTubeState, WeaponReadiness};
        let msg = ServerMessage::WeaponsUpdate {
            target_uuid: None,
            target_name: None,
            banks: vec![],
            tubes: vec![TorpedoTubeState {
                id: "fore-centre".to_string(),
                loaded: true,
                reload_secs: 0.0,
                state: "loaded".into(),
                progress: 1.0,
                load_time: 3.0,
                volley_max: 3,
                loaded_count: 3,
                target_count: 3,
                load_progress: 1.0,
                readiness: WeaponReadiness::default(),
                // Patterned attack: step 2 of 3, barrel 1 active this round.
                active_barrels: vec![1],
                pattern_step: 2,
                pattern_len: 3,
            }],
            torpedo_count: 27,
            phaser_mode: PhaserMode::Manual,
            blasters: vec![],
            phaser_frequency: 0.5,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    /// Shared weapon readiness contract (issue #764): each family's per-instance
    /// state carries a `WeaponReadiness` that round-trips with its blocking
    /// reason + range/arc intact, across all three families in one message.
    #[test]
    fn weapon_readiness_contract_round_trips_for_all_families() {
        use crate::messages::{
            BlasterBankState, PhaserBankState, PhaserMode, TorpedoTubeState, WeaponBlockReason,
            WeaponReadiness,
        };
        let msg = ServerMessage::WeaponsUpdate {
            target_uuid: Some("550e8400-e29b-41d4-a716-446655440000".into()),
            target_name: Some("Raider".into()),
            banks: vec![PhaserBankState {
                id: "port".into(),
                fire_ready: false,
                on_cooldown: false,
                cooldown_remaining: 0.0,
                readiness: WeaponReadiness {
                    ready: false,
                    blocking_reason: WeaponBlockReason::OutOfArc,
                    target_range: Some(42.0),
                    target_arc: Some(120.0),
                },
            }],
            tubes: vec![TorpedoTubeState {
                id: "fore".into(),
                loaded: false,
                reload_secs: 3.0,
                state: "loading".into(),
                progress: 0.5,
                load_time: 10.0,
                volley_max: 2,
                loaded_count: 0,
                target_count: 2,
                load_progress: 0.5,
                readiness: WeaponReadiness {
                    ready: false,
                    blocking_reason: WeaponBlockReason::Loading,
                    target_range: Some(100.0),
                    target_arc: Some(10.0),
                },
                active_barrels: Vec::new(),
                pattern_step: 0,
                pattern_len: 0,
            }],
            torpedo_count: 4,
            phaser_mode: PhaserMode::Manual,
            blasters: vec![BlasterBankState {
                id: "starboard".into(),
                fire_ready: false,
                on_cooldown: false,
                cooldown_remaining: 0.0,
                pending_volley: 2,
                charge_progress: 0.0,
                has_charge: false,
                readiness: WeaponReadiness {
                    ready: true,
                    blocking_reason: WeaponBlockReason::Ready,
                    target_range: Some(12.5),
                    target_arc: Some(3.0),
                },
                // Patterned attack in progress (issue #765): step 1 of 3,
                // barrels 0 and 2 firing simultaneously.
                active_barrels: vec![0, 2],
                pattern_step: 1,
                pattern_len: 3,
            }],
            phaser_frequency: 0.5,
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    /// Regression: the on-the-wire JSON for `LoadingProgress` must place
    /// `fraction` directly under `data`, not under `data.data`.
    ///
    /// Earlier the variant carried a nested `data: LoadingProgress` field,
    /// which combined with `#[serde(content = "data")]` produced
    /// `{"type":"LoadingProgress","data":{"data":{"fraction":0.5}}}` —
    /// the JS handlers in `server.html` and `client.html` read
    /// `parsed.data?.fraction` and got `undefined` (an object has no
    /// `fraction`), so the loading bar stuck at 0 % for the entire
    /// duration of `GamePhase::Loading`.
    #[test]
    fn server_loading_progress_wire_format() {
        let msg = ServerMessage::LoadingProgress { fraction: 0.5 };
        let encoded = JsonCodec.encode_server(&msg).unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"LoadingProgress","data":{"fraction":0.5}}"#,
            "LoadingProgress wire format must put `fraction` at data.fraction (not data.data.fraction); JS clients depend on this exact layout",
        );
    }

    #[test]
    fn entity_snapshot_shield_fraction_is_present_as_a_number_on_the_wire() {
        // (#471) shield_fraction: Some(0.0..=1.0) must appear as a bare number
        // on the wire, not e.g. wrapped or stringified.
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![EntitySnapshot {
                    shield_fraction: Some(0.42),
                    ..sample_entity_snapshot()
                }],
                ..Default::default()
            },
        };
        let json = JsonCodec.encode_server(&msg).expect("encode");
        assert!(
            json.contains("\"shield_fraction\":0.42"),
            "wire must contain shield_fraction=0.42, got: {json}"
        );
    }

    #[test]
    fn entity_snapshot_shield_fraction_none_is_omitted_from_wire() {
        // (#471) When shield_fraction is None, the field should be entirely
        // absent from the JSON wire format.
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![sample_entity_snapshot()],
                ..Default::default()
            },
        };
        let json = JsonCodec.encode_server(&msg).expect("encode");
        assert!(
            !json.contains("shield_fraction"),
            "shield_fraction=None must be omitted from wire, got: {json}"
        );
    }

    #[test]
    fn entity_snapshot_radar_size_none_is_omitted_from_json() {
        let msg = ServerMessage::WorldSetup {
            world: WorldData {
                entities: vec![sample_entity_snapshot()],
                ..Default::default()
            },
        };
        let encoded = JsonCodec.encode_server(&msg).expect("encode");
        assert!(
            !encoded.contains("radar_size"),
            "None radar_size must be omitted from JSON, got: {}",
            encoded
        );
    }

    // ── Version-skew tests: envelope decode path (issue #610) ─────────────
    //
    // These pin the CURRENT decode behaviour of the `ClientMessage` /
    // `ServerMessage` envelopes (`#[serde(tag = "type", content = "data")]`,
    // no `#[serde(deny_unknown_fields)]`) for two version-skew scenarios:
    // an unrecognised field on a known variant's payload, and a completely
    // unknown `type` tag. No policy change is made here — this is
    // documentation-by-test of whatever serde already does. If the pinned
    // behaviour ever changes intentionally, update the doc comments below.

    /// Pins current behaviour: an unrecognised field inside a known
    /// variant's `data` payload is silently ignored by serde on the client
    /// decode path (no `#[serde(deny_unknown_fields)]` on `ClientMessage`).
    /// A newer client sending an extra field to an older server (or vice
    /// versa) will not fail to decode — the extra field is simply dropped.
    /// If this changes (e.g. `deny_unknown_fields` is added), update this
    /// comment and the assertion below.
    #[test]
    fn client_decode_unknown_field_in_known_variant_is_ignored() {
        let json = r#"{"type":"SetReady","data":{"ready":true,"totally_unknown_field":42}}"#;
        let decoded = JsonCodec.decode_client(json);
        assert_eq!(
            decoded.expect("unknown field must not fail decode"),
            ClientMessage::SetReady { ready: true }
        );
    }

    /// Pins current behaviour: a `type` tag that does not match any
    /// `ClientMessage` variant is a hard decode error — serde's internally
    /// tagged enum representation has no wildcard/fallback arm. A client on
    /// a newer wire format that introduces a brand-new variant will produce
    /// an unrecoverable decode error on an older server, not a silently
    /// dropped message. If this changes (e.g. an explicit `Unknown` catch-all
    /// variant is introduced), update this comment.
    #[test]
    fn client_decode_unknown_type_tag_is_decode_error() {
        let json = r#"{"type":"TotallyMadeUpVariant","data":{}}"#;
        let decoded = JsonCodec.decode_client(json);
        assert!(
            decoded.is_err(),
            "an unknown `type` tag must fail to decode, got: {decoded:?}"
        );
    }

    /// Pins current behaviour: an unrecognised field inside a known
    /// variant's `data` payload is silently ignored by serde on the server
    /// decode path (no `#[serde(deny_unknown_fields)]` on `ServerMessage`).
    /// An older cached client meeting a newer server payload shape (extra
    /// fields added to an existing variant) will not error — it just won't
    /// see the new field. If this policy ever changes intentionally, update
    /// this comment.
    #[test]
    fn server_decode_unknown_field_in_known_variant_is_ignored() {
        let json =
            r#"{"type":"LoadingProgress","data":{"fraction":0.5,"totally_unknown_field":"x"}}"#;
        let decoded = JsonCodec.decode_server(json);
        assert_eq!(
            decoded.expect("unknown field must not fail decode"),
            ServerMessage::LoadingProgress { fraction: 0.5 }
        );
    }

    /// Pins current behaviour: a `type` tag that does not match any
    /// `ServerMessage` variant is a hard decode error, for the same reason
    /// as the client-side case above. A cached older client that receives a
    /// message using a brand-new server-only variant it doesn't know about
    /// will fail to decode that message outright (and must handle/log the
    /// error), rather than silently ignoring it. If this changes, update
    /// this comment.
    #[test]
    fn server_decode_unknown_type_tag_is_decode_error() {
        let json = r#"{"type":"TotallyMadeUpVariant","data":{}}"#;
        let decoded = JsonCodec.decode_server(json);
        assert!(
            decoded.is_err(),
            "an unknown `type` tag must fail to decode, got: {decoded:?}"
        );
    }

    // ── Bridge decode helper tests (decode_bridge_client_message) ──────────
    //
    // The short-form system-control shim was retired by issue #822: every
    // emitter now sends the full `ClientMessage` envelope, so the bridge
    // decode is a plain serde decode.

    #[test]
    fn decode_bridge_client_message_accepts_full_client_message() {
        let msg =
            decode_bridge_client_message(r#"{"type":"SetReady","data":{"ready":true}}"#).unwrap();

        assert_eq!(msg, ClientMessage::SetReady { ready: true });
    }

    /// Post-#822 the bridge no longer rewrites bare short-form payloads such
    /// as `{"type":"SetThrust",...}` — they are a hard decode error, exactly
    /// like any other unknown `type` tag.
    #[test]
    fn decode_bridge_client_message_rejects_short_form_payloads() {
        for json in [
            r#"{"type":"SetThrust","data":{"value":0.5}}"#,
            r#"{"type":"StartImpulseCharge"}"#,
            r#"{"type":"Hail","data":{"target_uuid":"s1"}}"#,
        ] {
            assert!(
                decode_bridge_client_message(json).is_err(),
                "short-form payload must no longer decode: {json}"
            );
        }
    }

    /// Comms control payloads round-trip inside the `ControlSystem` envelope
    /// (issue #822 — pins the shapes `gui/action-map.js` now emits after the
    /// short-form shim was retired).
    #[test]
    fn comms_control_system_payloads_round_trip() {
        let payloads = vec![
            SystemControlPayload::Hail {
                target_uuid: "starbase-1".into(),
            },
            SystemControlPayload::SelectCommsMessage {
                message_id: "m1".into(),
            },
            SystemControlPayload::RespondToMessage {
                message_id: "m1".into(),
                response_index: 0,
            },
            SystemControlPayload::ClearComms,
            SystemControlPayload::ShowOnScreen {
                message_id: "m1".into(),
            },
        ];
        for payload in payloads {
            let msg = ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload,
            };
            assert_client_roundtrip(&JsonCodec, msg.clone());
            assert_client_roundtrip(&PrettyJsonCodec, msg);
        }

        // Pin one wire shape exactly — action-map.js `hail` depends on this.
        let encoded = JsonCodec
            .encode_client(&ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: SystemControlPayload::Hail {
                    target_uuid: "starbase-1".into(),
                },
            })
            .unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"comms","payload":{"type":"Hail","data":{"target_uuid":"starbase-1"}}}}"#,
            "Hail wire shape must match what action-map.js sends"
        );
    }

    /// Navigation waypoint payloads round-trip inside the `ControlSystem`
    /// envelope (issue #822 — pins the shapes `gui/action-map.js` now emits).
    #[test]
    fn navigation_control_system_payloads_round_trip() {
        for payload in [
            SystemControlPayload::SetNavigationWaypoint {
                x: 12.5,
                z: -8.0,
                source_uuid: None,
            },
            SystemControlPayload::SetNavigationWaypoint {
                x: 12.5,
                z: -8.0,
                source_uuid: Some("station-alpha".into()),
            },
            SystemControlPayload::ClearNavigationWaypoint,
        ] {
            let msg = ClientMessage::ControlSystem {
                target: crate::system_registry::navigation_system_id(),
                payload,
            };
            assert_client_roundtrip(&JsonCodec, msg.clone());
            assert_client_roundtrip(&PrettyJsonCodec, msg);
        }

        // Pin the unit-payload wire shape — action-map.js
        // `clear_navigation_waypoint` depends on this.
        let encoded = JsonCodec
            .encode_client(&ClientMessage::ControlSystem {
                target: crate::system_registry::navigation_system_id(),
                payload: SystemControlPayload::ClearNavigationWaypoint,
            })
            .unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"navigation","payload":{"type":"ClearNavigationWaypoint"}}}"#,
            "ClearNavigationWaypoint wire shape must match what action-map.js sends"
        );
    }

    /// Civilian orders (issue #1028) round-trip inside the `ControlSystem`
    /// envelope, targeting the same `navigation` system the waypoint payloads do.
    ///
    /// All three verbs, and both flavours of `divert`, because the order is a
    /// *nested* enum on the payload — the one shape in this envelope where a
    /// serde tag sits inside a serde tag — and a rename on either level would
    /// break traffic control while every other navigation payload kept working.
    #[test]
    fn civilian_order_payloads_round_trip() {
        use crate::civilian::CivilianOrder;
        for order in [
            CivilianOrder::Hold,
            CivilianOrder::divert_to_route("depot_run"),
            CivilianOrder::divert_to_anchor("holding_point"),
            CivilianOrder::dock_at("world.entity.skyhook_depot.name"),
        ] {
            let msg = ClientMessage::ControlSystem {
                target: crate::system_registry::navigation_system_id(),
                payload: SystemControlPayload::OrderCivilian {
                    target: "world.entity.hauler_kestrel.name".into(),
                    order,
                },
            };
            assert_client_roundtrip(&JsonCodec, msg.clone());
            assert_client_roundtrip(&PrettyJsonCodec, msg);
        }

        // Pin the wire shape the nav console's order controls send.
        let encoded = JsonCodec
            .encode_client(&ClientMessage::ControlSystem {
                target: crate::system_registry::navigation_system_id(),
                payload: SystemControlPayload::OrderCivilian {
                    target: "hauler".into(),
                    order: CivilianOrder::divert_to_route("depot_run"),
                },
            })
            .unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"navigation","payload":{"type":"OrderCivilian","data":{"target":"hauler","order":{"verb":"divert","route":"depot_run"}}}}}"#,
            "the civilian order wire shape must match what action-map.js sends"
        );
    }

    /// Vertical thrust (issue #744) round-trips through the `ControlSystem`
    /// envelope, targeting `helm-vertical-thrust`. AI-only on the wire, but the
    /// payload must survive the codec like every other admitted command.
    #[test]
    fn vertical_thrust_control_system_payload_round_trips() {
        for vertical in [-1.0_f32, -0.25, 0.0, 0.6, 1.0] {
            let msg = ClientMessage::ControlSystem {
                target: crate::system_registry::vertical_thrust_system_id(),
                payload: SystemControlPayload::VerticalThrustInput { vertical },
            };
            assert_client_roundtrip(&JsonCodec, msg.clone());
            assert_client_roundtrip(&PrettyJsonCodec, msg);
        }

        // Pin the wire shape.
        let encoded = JsonCodec
            .encode_client(&ClientMessage::ControlSystem {
                target: crate::system_registry::vertical_thrust_system_id(),
                payload: SystemControlPayload::VerticalThrustInput { vertical: 0.5 },
            })
            .unwrap();
        assert_eq!(
            encoded,
            r#"{"type":"ControlSystem","data":{"target":"helm-vertical-thrust","payload":{"type":"VerticalThrustInput","data":{"vertical":0.5}}}}"#,
            "VerticalThrustInput wire shape must be stable"
        );
    }

    // ── ModifierSource / FlagKind / EntityTag / RegionEffectKind ───────────
    // (not ClientMessage/ServerMessage envelope tests — out of scope for the
    // table-driven harness, kept as-is)

    #[test]
    fn modifier_source_world_hash_and_eq() {
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
            crate::messages::FlagKind::CommsJammed,
            crate::messages::FlagKind::SensorBlind,
        ] {
            let json = serde_json::to_string(flag).unwrap();
            let decoded: crate::messages::FlagKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*flag, decoded);
        }
    }

    #[test]
    fn shield_facing_status_with_arc_geometry_round_trips() {
        let msg = ShieldFacingStatus {
            label: "Fore".into(),
            hp: 150,
            max_hp: 150,
            online: true,
            offline_remaining: 0.0,
            is_focused: true,
            center_deg: 0.0,
            width_deg: 90.0,
            arc_id: "fore".into(),
            priority: 3,
        };
        let encoded = serde_json::to_string(&msg).unwrap();
        let decoded: ShieldFacingStatus = serde_json::from_str(&encoded).unwrap();
        assert_eq!(msg, decoded);
        // Wire compat: pre-#514 payloads without center_deg/width_deg/arc_id
        // must still deserialize with the defaults filled in.
        let legacy_json =
            r#"{"label":"Fore","hp":100,"max_hp":100,"online":true,"offline_remaining":0.0}"#;
        let legacy_decoded: ShieldFacingStatus = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(legacy_decoded.center_deg, 0.0);
        assert_eq!(legacy_decoded.width_deg, 90.0);
        assert_eq!(legacy_decoded.arc_id, "");
        assert!(!legacy_decoded.is_focused);
        assert_eq!(legacy_decoded.priority, 1);
    }

    #[test]
    fn coordination_frequency_hint_round_trips() {
        let payload = CoordinationPayload::FrequencyHint { frequency: 0.33 };
        let json = serde_json::to_string(&payload).unwrap();
        let decoded: CoordinationPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn coordination_frequency_hint_boundary_values_round_trip() {
        for f in [0.0f32, 1.0f32] {
            let payload = CoordinationPayload::FrequencyHint { frequency: f };
            let json = serde_json::to_string(&payload).unwrap();
            let decoded: CoordinationPayload = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, payload);
        }
    }

    // ── RegionEffectKind serde round-trips (moved from regions/effects.rs #524) ──

    fn region_effect_round_trip(effect: crate::regions::effects::RegionEffectKind) {
        let json = serde_json::to_string(&effect).unwrap();
        let decoded: crate::regions::effects::RegionEffectKind =
            serde_json::from_str(&json).unwrap();
        assert_eq!(effect, decoded);
    }

    #[test]
    fn region_effect_damage_zone_round_trips() {
        region_effect_round_trip(crate::regions::effects::RegionEffectKind::DamageZone {
            dps: 15.0,
            shield_pierce: 0.0,
        });
    }

    #[test]
    fn region_effect_slow_zone_round_trips() {
        use crate::regions::effects::RegionEffectKind::SlowZone;
        region_effect_round_trip(SlowZone {
            thrust_modifier: Some(0.5),
            yaw_rate_modifier: Some(-0.3),
        });
        region_effect_round_trip(SlowZone {
            thrust_modifier: Some(0.5),
            yaw_rate_modifier: None,
        });
        region_effect_round_trip(SlowZone {
            thrust_modifier: None,
            yaw_rate_modifier: Some(-0.3),
        });
        region_effect_round_trip(SlowZone {
            thrust_modifier: None,
            yaw_rate_modifier: None,
        });
    }

    #[test]
    fn region_effect_blocks_impulse_round_trips() {
        region_effect_round_trip(crate::regions::effects::RegionEffectKind::BlocksImpulse);
    }

    #[test]
    fn region_effect_radar_dampening_round_trips() {
        region_effect_round_trip(crate::regions::effects::RegionEffectKind::RadarDampening {
            multiplier: 0.3,
        });
    }

    #[test]
    fn region_effect_comms_jam_round_trips() {
        region_effect_round_trip(crate::regions::effects::RegionEffectKind::CommsJam);
    }

    #[test]
    fn region_effect_sensor_blind_round_trips() {
        region_effect_round_trip(crate::regions::effects::RegionEffectKind::SensorBlind);
    }

    #[test]
    fn region_effect_nebula_fog_round_trips() {
        use crate::regions::effects::RegionEffectKind::NebulaFog;
        region_effect_round_trip(NebulaFog {
            color: [0.25, 0.08, 0.32],
            density: 0.008,
        });
        region_effect_round_trip(NebulaFog {
            color: [0.5, 0.1, 0.2],
            density: 0.015,
        });
    }

    #[test]
    fn region_effect_negative_and_zero_values_round_trip() {
        use crate::regions::effects::RegionEffectKind::{DamageZone, RadarDampening, SlowZone};
        region_effect_round_trip(DamageZone {
            dps: -5.0,
            shield_pierce: 0.0,
        });
        region_effect_round_trip(SlowZone {
            thrust_modifier: Some(-1.0),
            yaw_rate_modifier: None,
        });
        region_effect_round_trip(DamageZone {
            dps: 0.0,
            shield_pierce: 0.0,
        });
        region_effect_round_trip(RadarDampening { multiplier: 0.0 });
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

    // ── Comms wire version-skew (field-level, not envelope-level) ──────────

    #[test]
    fn comms_contact_missing_in_range_defaults_to_true() {
        let json = r#"{"uuid":"x","name":"X"}"#;
        let contact: CommsContact = serde_json::from_str(json).unwrap();
        assert!(
            contact.in_range,
            "in_range should default to true for backward compat"
        );
    }

    #[test]
    fn comms_message_missing_sender_in_range_defaults_to_true() {
        let json = r#"{"id":"m","sender_uuid":"s","sender_name":"S","subject":"x","body":"y","responses":[],"selected_response":null,"is_read":false}"#;
        let msg: CommsMessage = serde_json::from_str(json).unwrap();
        assert!(
            msg.sender_in_range,
            "sender_in_range should default to true for backward compat"
        );
    }

    #[test]
    fn comms_message_missing_thread_id_defaults_to_empty() {
        let json = r#"{"id":"m","sender_uuid":"s","sender_name":"S","subject":"x","body":"y","responses":[],"selected_response":null,"is_read":false}"#;
        let msg: CommsMessage = serde_json::from_str(json).unwrap();
        assert!(
            msg.thread_id.is_empty(),
            "thread_id should default to empty string for backward compat"
        );
    }

    #[test]
    fn comms_response_view_missing_flags_default_important_false_available_true() {
        // A pre-#761 wire payload carries responses as bare `{ "text": ... }`
        // objects. `important` must default to false and `available` to true.
        let json = r#"{"text":"Acknowledge"}"#;
        let view: crate::messages::CommsResponseView = serde_json::from_str(json).unwrap();
        assert_eq!(view.text, "Acknowledge");
        assert!(!view.important, "important must default to false");
        assert!(view.available, "available must default to true");
    }

    #[test]
    fn comms_response_view_round_trips_important_and_available() {
        let view = crate::messages::CommsResponseView {
            text: "Fire everything".into(),
            important: true,
            available: false,
        };
        let json = serde_json::to_string(&view).unwrap();
        let back: crate::messages::CommsResponseView = serde_json::from_str(&json).unwrap();
        assert_eq!(view, back);
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
            ServerMessage::CommsState {
                messages, contacts, ..
            } => {
                assert_eq!(messages.len(), 1);
                assert!(
                    messages[0].sender_in_range,
                    "sender_in_range must default to true"
                );
                assert_eq!(contacts.len(), 1);
                assert!(contacts[0].in_range, "in_range must default to true");
            }
            other => panic!("expected CommsState, got {other:?}"),
        }
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
            assert!(
                snapshot.control_sources.is_empty(),
                "control_sources must default empty for an older compatible host"
            );
        } else {
            panic!("expected SimState");
        }
    }

    // ── HTML console bridge (de)serialisation ─────────────────────────────

    #[test]
    fn encode_hud_state_round_trips() {
        let state = ViewscreenHudState {
            heading: 90,
            hull_pct: 75,
            condition: "ALERT".into(),
            red_alert: true,
            engine_thrust: 0.0,
            phaser_firing: true,
            game_over_message: None,
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
            engine_thrust: 0.0,
            phaser_firing: false,
            game_over_message: None,
        };
        let json = encode_hud_state(&state).expect("encode hud");
        assert!(json.contains("\"heading\":0"), "got: {json}");
        assert!(json.contains("\"hull_pct\":100"), "got: {json}");
        assert!(json.contains("\"condition\":\"NOMINAL\""), "got: {json}");
        assert!(json.contains("\"red_alert\":false"), "got: {json}");
    }

    // ── SystemBlackboard tag-shape tests (not envelope round-trips) ────────

    #[test]
    fn system_blackboard_helm_serde_fields() {
        let bb = SystemBlackboard::Helm(HelmBlackboard {
            yaw: 1.5,
            forward_speed: 42.0,
            x: 10.0,
            z: -20.0,
            impulse_charge: 0.3,
            boost_battery: 0.8,
            boost_active: false,
            boost_enabled: true,
            radar_range: 0.0,
            lateral_speed: 0.0,
            hostile_weapon_arcs: Vec::new(),
        });
        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains("\"kind\":\"Helm\""), "got: {json}");
        assert!(json.contains("\"yaw\":1.5"), "got: {json}");
        assert!(json.contains("\"forward_speed\":42.0"), "got: {json}");
        assert!(json.contains("\"impulse_charge\":0.3"), "got: {json}");
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);
    }

    /// The hostile weapon-arc overlay payload (issue #874) round-trips.
    ///
    /// Populated deliberately: the empty case is what `#[serde(default)]` plus
    /// `skip_serializing_if` already cover, and the field only earns its place
    /// on the wire when it carries sectors.
    #[test]
    fn system_blackboard_helm_hostile_weapon_arcs_round_trip() {
        use crate::messages::{HostileWeaponArc, HostileWeaponArcContact};
        let bb = SystemBlackboard::Helm(HelmBlackboard {
            hostile_weapon_arcs: vec![HostileWeaponArcContact {
                uuid: "1f6b4c8e-0000-4000-8000-000000000001".into(),
                x: 120.0,
                z: -45.5,
                arcs: vec![
                    HostileWeaponArc {
                        bearing_deg: -30.0,
                        half_angle_deg: 45.0,
                        range: 800.0,
                    },
                    HostileWeaponArc {
                        bearing_deg: 150.0,
                        half_angle_deg: 60.0,
                        range: 500.0,
                    },
                ],
            }],
            ..Default::default()
        });
        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains("\"hostile_weapon_arcs\""), "got: {json}");
        assert!(json.contains("\"bearing_deg\":-30.0"), "got: {json}");
        assert!(json.contains("\"half_angle_deg\":45.0"), "got: {json}");
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);
    }

    /// An empty arc list stays OFF the wire — the red-alert gate must cost
    /// nothing when it is closed, which is the common case.
    #[test]
    fn system_blackboard_helm_omits_empty_hostile_weapon_arcs() {
        let bb = SystemBlackboard::Helm(HelmBlackboard::default());
        let json = serde_json::to_string(&bb).unwrap();
        assert!(!json.contains("hostile_weapon_arcs"), "got: {json}");
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);
    }

    /// `SystemBlackboard::Repair` round-trip, tag shape and full envelope
    /// (issue #737).
    ///
    /// The repair blackboard is the only blackboard carrying gated damage
    /// detail, and #737 added two wire fields to it: `QueueEntryPreview
    /// ::station_id` — the bucket the host projection decides entitlement from,
    /// which the client also keys its queue rows by — and
    /// `RepairBlackboard::aggregate_hull_fraction`, the one whole-ship figure
    /// every recipient may have now that `system_hull` is a projection and can
    /// no longer be summed into one. Both are new on the wire; neither had any
    /// round-trip coverage. `queue_depth` and `system_hull` are populated here
    /// because the empty-vec case is what `#[serde(default)]` already covers.
    ///
    /// Issue #1014 added a third: `destroyed_hull_fraction`, the companion
    /// whole-ship scalar for capability at the `Destroyed` tier. It is
    /// `#[serde(default)]` like the other two, so a payload predating it decodes
    /// to `None` rather than failing — pinned below.
    #[test]
    fn system_blackboard_repair_round_trips() {
        fn hull(id: &str, current: f32, tier: crate::damage::DamageTier) -> SystemHullStatus {
            SystemHullStatus {
                system_id: SystemId(id.into()),
                display_name: id.into(),
                current,
                max_hp: 100.0,
                tier,
                debuff_magnitude: 0.25,
            }
        }

        let bb = SystemBlackboard::Repair(RepairBlackboard {
            teams: vec![],
            travel_duration_secs: 5.0,
            system_hull: vec![
                hull("core", 40.0, crate::damage::DamageTier::Damaged),
                hull("repair", 100.0, crate::damage::DamageTier::Operational),
            ],
            damageable_systems: vec![SystemId("core".into()), SystemId("helm-radar".into())],
            queue_depth: vec![
                QueueEntryPreview {
                    station_id: "core".into(),
                    station_label: "Core".into(),
                    tier: crate::damage::DamageTier::Damaged,
                    deficit: 60.0,
                },
                QueueEntryPreview {
                    station_id: "helm".into(),
                    station_label: "Helm".into(),
                    tier: crate::damage::DamageTier::Disabled,
                    deficit: 90.0,
                },
            ],
            aggregate_hull_fraction: Some(0.75),
            destroyed_hull_fraction: Some(0.2),
        });

        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains("\"kind\":\"Repair\""), "got: {json}");
        assert!(json.contains("\"station_id\":\"core\""), "got: {json}");
        assert!(json.contains("\"station_id\":\"helm\""), "got: {json}");
        assert!(
            json.contains("\"aggregate_hull_fraction\":0.75"),
            "got: {json}"
        );
        assert!(
            json.contains("\"destroyed_hull_fraction\":0.2"),
            "got: {json}"
        );
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);

        // A payload written before #1014 carries no `destroyed_hull_fraction`;
        // `#[serde(default)]` must decode it to `None` rather than reject the
        // whole blackboard.
        let legacy = r#"{"kind":"Repair","data":{"teams":[],"travel_duration_secs":5.0,
            "system_hull":[],"damageable_systems":[],"queue_depth":[],
            "aggregate_hull_fraction":0.75}}"#;
        let decoded_legacy: SystemBlackboard = serde_json::from_str(legacy).unwrap();
        let SystemBlackboard::Repair(legacy_bb) = decoded_legacy else {
            panic!("expected a Repair blackboard");
        };
        assert_eq!(legacy_bb.destroyed_hull_fraction, None);
        assert_eq!(legacy_bb.aggregate_hull_fraction, Some(0.75));

        // ...and through the envelope it actually ships in. Post-#737 this is
        // sent per token (`Target::Token`), not broadcast, but the encoding is
        // the same one the resync path reuses.
        let msg = ServerMessage::BlackboardUpdate {
            updates: vec![(SystemId("repair".into()), bb)],
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg);
    }

    #[test]
    fn system_blackboard_weapons_serde_fields() {
        let bb = SystemBlackboard::Weapons(WeaponsBlackboard {
            target_uuid: Some("truth-uuid".into()),
            locked_target: Some("intent-uuid".into()),
            target_name: Some("Raider".into()),
            torpedo_count: 4,
            ..Default::default()
        });
        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains("\"kind\":\"Weapons\""), "got: {json}");
        assert!(
            json.contains("\"target_uuid\":\"truth-uuid\""),
            "got: {json}"
        );
        assert!(
            json.contains("\"locked_target\":\"intent-uuid\""),
            "got: {json}"
        );
        assert!(json.contains("\"torpedo_count\":4"), "got: {json}");
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);
    }

    #[test]
    fn weapons_blackboard_legacy_wire_shape_defaults_locked_target() {
        // Pre-#697 payloads carry no `locked_target`; they must still decode,
        // defaulting the AI-intent field to None.
        let legacy_json = r#"{"kind":"Weapons","data":{
            "target_uuid":"truth-uuid",
            "target_name":null,
            "banks":[],
            "tubes":[],
            "torpedo_count":0,
            "phaser_mode":"Manual"
        }}"#;
        let decoded: SystemBlackboard = serde_json::from_str(legacy_json).unwrap();
        match decoded {
            SystemBlackboard::Weapons(bb) => {
                assert_eq!(bb.target_uuid.as_deref(), Some("truth-uuid"));
                assert_eq!(bb.locked_target, None);
            }
            other => panic!("expected Weapons blackboard, got {other:?}"),
        }
    }

    #[test]
    fn system_blackboard_phaser_bank_serde_fields() {
        let bb = SystemBlackboard::PhaserBank(PhaserBankBlackboard {
            is_online: true,
            on_cooldown: false,
            cooldown_remaining: 0.0,
            fire_ready: true,
        });
        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains("\"kind\":\"PhaserBank\""), "got: {json}");
        assert!(json.contains("\"fire_ready\":true"), "got: {json}");
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);
    }

    #[test]
    fn system_blackboard_torpedo_magazine_serde_fields() {
        let bb = SystemBlackboard::TorpedoMagazine(TorpedoMagazineBlackboard {
            is_online: false,
            torpedoes_remaining: 3,
            capacity: 10,
            torpedoes_in_flight: 0,
        });
        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains("\"kind\":\"TorpedoMagazine\""), "got: {json}");
        assert!(json.contains("\"is_online\":false"), "got: {json}");
        assert!(json.contains("\"torpedoes_remaining\":3"), "got: {json}");
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);
    }

    #[test]
    fn system_blackboard_power_reactor_serde_fields() {
        let bb = SystemBlackboard::PowerReactor(PowerReactorBlackboard {
            total_allocation: 5,
            max_allocation: 8,
            is_online: false,
            draining: true,
        });
        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains("\"kind\":\"PowerReactor\""), "got: {json}");
        assert!(json.contains("\"is_online\":false"), "got: {json}");
        assert!(json.contains("\"total_allocation\":5"), "got: {json}");
        assert!(json.contains("\"draining\":true"), "got: {json}");
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);
    }

    #[test]
    fn system_blackboard_power_battery_serde_fields() {
        let bb = SystemBlackboard::PowerBattery(PowerBatteryBlackboard {
            charge: 15.0,
            capacity: 100.0,
            is_online: false,
            emergency_threshold: 0.25,
        });
        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains("\"kind\":\"PowerBattery\""), "got: {json}");
        assert!(json.contains("\"is_online\":false"), "got: {json}");
        assert!(json.contains("\"charge\":15"), "got: {json}");
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);
    }

    #[test]
    fn system_blackboard_shield_arc_serde_fields() {
        let bb = SystemBlackboard::ShieldArc(ShieldArcBlackboard {
            label: "Aft".into(),
            hp: 0,
            max_hp: 75,
            is_online: false,
            is_focused: false,
            offline_remaining: 4.5,
            center_deg: 180.0,
            width_deg: 90.0,
        });
        let json = serde_json::to_string(&bb).unwrap();
        assert!(json.contains("\"kind\":\"ShieldArc\""), "got: {json}");
        assert!(json.contains("\"label\":\"Aft\""), "got: {json}");
        assert!(json.contains("\"hp\":0"), "got: {json}");
        assert!(json.contains("\"max_hp\":75"), "got: {json}");
        assert!(json.contains("\"is_online\":false"), "got: {json}");
        assert!(json.contains("\"is_focused\":false"), "got: {json}");
        assert!(json.contains("\"offline_remaining\":4.5"), "got: {json}");
        assert!(json.contains("\"center_deg\":180"), "got: {json}");
        assert!(json.contains("\"width_deg\":90"), "got: {json}");
        let decoded: SystemBlackboard = serde_json::from_str(&json).unwrap();
        assert_eq!(bb, decoded);
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
            torpedo_armed: true,
        };
        let json = serde_json::to_string(&blip).unwrap();
        let decoded: RadarBlip = serde_json::from_str(&json).unwrap();
        assert_eq!(blip, decoded);
        assert!(json.contains("\"icon\":\"ship\""), "got: {json}");
        assert!(json.contains("\"objective_target\":true"), "got: {json}");
        assert!(json.contains("\"name\":\"Pirate Raider\""), "got: {json}");
        assert!(json.contains("\"torpedo_armed\":true"), "got: {json}");
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
        // Issue #957: an old payload carries no capability claim, and the
        // absence must read as "not known to be torpedo-armed" rather than
        // badging every legacy contact.
        assert!(!blip.torpedo_armed);
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

    // ── issue #616 (parent #516): SystemId-keyed hull + Power group additive shapes ──
    // These tests cover the new-shape wire types introduced alongside the
    // legacy `Console`-keyed shapes. Publishers emit both; consumers may read
    // either. Legacy payloads without the new fields must still deserialize.

    #[test]
    fn system_hull_status_round_trips() {
        let status = SystemHullStatus {
            system_id: SystemId("phaser-fore".into()),
            display_name: "Phaser Bank (Fore)".into(),
            current: 42.5,
            max_hp: 100.0,
            tier: crate::damage::DamageTier::Damaged,
            debuff_magnitude: 0.15,
        };
        let json = serde_json::to_string(&status).unwrap();
        let decoded: SystemHullStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, decoded);
        assert!(
            json.contains("\"system_id\":\"phaser-fore\""),
            "got: {json}"
        );
        assert!(
            json.contains("\"display_name\":\"Phaser Bank (Fore)\""),
            "got: {json}"
        );
    }

    #[test]
    fn system_hull_status_debuff_defaults_when_absent() {
        // Legacy payload without debuff_magnitude must still deserialize.
        let json = r#"{"system_id":"helm","display_name":"Helm","current":10.0,"max_hp":25.0,"tier":"Damaged"}"#;
        let decoded: SystemHullStatus = serde_json::from_str(json).unwrap();
        assert_eq!(decoded.debuff_magnitude, 0.0);
        assert_eq!(decoded.system_id, SystemId("helm".into()));
    }

    #[test]
    fn system_hull_update_legacy_wire_shape_defaults_destroyed_fraction() {
        // A payload written before #1014 carries no `destroyed_fraction`;
        // `#[serde(default)]` must decode it to `None` rather than reject the
        // whole message.
        let legacy =
            r#"{"type":"SystemHullUpdate","data":{"entries":[],"aggregate_fraction":0.75}}"#;
        let decoded = JsonCodec.decode_server(legacy).unwrap();
        let ServerMessage::SystemHullUpdate {
            aggregate_fraction,
            destroyed_fraction,
            ..
        } = decoded
        else {
            panic!("expected SystemHullUpdate");
        };
        assert_eq!(aggregate_fraction, Some(0.75));
        assert_eq!(destroyed_fraction, None);
    }

    #[test]
    fn system_hull_update_legacy_wire_shape_defaults_aggregate_fraction() {
        // Same pre-existing gap, one field further back: a payload that also
        // predates `aggregate_fraction` must default it to `None` too.
        let legacy = r#"{"type":"SystemHullUpdate","data":{"entries":[]}}"#;
        let decoded = JsonCodec.decode_server(legacy).unwrap();
        let ServerMessage::SystemHullUpdate {
            aggregate_fraction,
            destroyed_fraction,
            ..
        } = decoded
        else {
            panic!("expected SystemHullUpdate");
        };
        assert_eq!(aggregate_fraction, None);
        assert_eq!(destroyed_fraction, None);
    }

    #[test]
    fn team_slot_new_wire_shape_decodes_without_legacy_console_field() {
        // Post-#619 wire form: no `console` / `queued` fields at all.
        // Unknown fields (if a legacy payload sends them) are silently
        // ignored by serde since the struct no longer declares them.
        let new_json = r#"{
            "type": "RepairState",
            "data": {
                "teams": [
                    { "Travelling": { "system_id": "helm", "display_name": "Helm", "elapsed": 0.5 } },
                    { "Repairing": { "system_id": "phaser-fore", "display_name": "Phaser Bank (Fore)" } },
                    { "Returning": { "remaining": 1.0, "queued_system_id": "tactical", "queued_display_name": "Tactical" } }
                ]
            }
        }"#;
        let decoded = JsonCodec.decode_server(new_json).unwrap();
        match decoded {
            ServerMessage::RepairState { teams } => {
                match &teams[0] {
                    TeamSlot::Travelling {
                        system_id,
                        display_name,
                        ..
                    } => {
                        assert_eq!(*system_id, Some(SystemId("helm".into())));
                        assert_eq!(display_name.as_deref(), Some("Helm"));
                    }
                    other => panic!("expected Travelling, got {other:?}"),
                }
                match &teams[1] {
                    TeamSlot::Repairing {
                        system_id,
                        display_name,
                        ..
                    } => {
                        assert_eq!(*system_id, Some(SystemId("phaser-fore".into())));
                        assert_eq!(display_name.as_deref(), Some("Phaser Bank (Fore)"));
                    }
                    other => panic!("expected Repairing, got {other:?}"),
                }
                match &teams[2] {
                    TeamSlot::Returning {
                        queued_system_id,
                        queued_display_name,
                        ..
                    } => {
                        assert_eq!(*queued_system_id, Some(SystemId("tactical".into())));
                        assert_eq!(queued_display_name.as_deref(), Some("Tactical"));
                    }
                    other => panic!("expected Returning, got {other:?}"),
                }
            }
            other => panic!("expected RepairState, got {other:?}"),
        }
    }

    #[test]
    fn power_blackboard_legacy_wire_shape_defaults_groups_field() {
        // Pre-#616 blackboard payload without `groups` must still deserialize
        // (post-#516 sub-PR-follow-up the `consoles` field is gone from the
        // struct entirely; the legacy field is now an "unknown field" that
        // serde silently ignores, and `groups` defaults to the empty vec).
        // `locked` joined `consoles` in that ignored set when issue #952
        // retired the brownout lock; `draining` defaults to false in its place.
        let legacy_json = r#"{
            "kind": "Power",
            "data": {
                "consoles": [],
                "total": 0,
                "total_max": 8,
                "battery_charge": 0.0,
                "battery_max": 100.0,
                "locked": false
            }
        }"#;
        let decoded: SystemBlackboard = serde_json::from_str(legacy_json).unwrap();
        match decoded {
            SystemBlackboard::Power(bb) => {
                assert!(bb.groups.is_empty(), "groups defaults to empty vec");
                assert!(
                    !bb.draining,
                    "a pre-#952 payload's `locked` must not be read as `draining` — \
                     they are different questions"
                );
            }
            other => panic!("expected Power blackboard, got {other:?}"),
        }
    }

    #[test]
    fn power_group_entry_round_trips() {
        // PowerGroupEntry became a dedicated struct after the parent-issue
        // #516 cleanup (previously a type alias for the deleted
        // `PowerConsoleEntry`). Sanity: it constructs and round-trips through
        // JSON with the same field shape.
        let entry = PowerGroupEntry {
            id: "helm".into(),
            label: "HELM".into(),
            level: 1,
            commanded_level: 3,
            min_level: 2,
            max_level: 4,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let decoded: PowerGroupEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, entry);
        // Named explicitly: the round trip above would pass just as well if
        // `min_level` were dropped on encode and refilled by its default, and
        // the whole point of issue #1004 is that the AUTHORED floor reaches the
        // client rather than a value the client guessed.
        assert!(
            json.contains(r#""min_level":2"#),
            "the authored floor must be ON the wire, not reconstructed: {json}"
        );

        // A legacy payload has neither field. Both must still decode, to
        // DIFFERENT defaults — they are different questions:
        //   - `commanded_level` (pre-#952) has no meaningful absent value, so
        //     its bare `#[serde(default)]` 0 reads as "unknown" and the client
        //     steps from `level`.
        //   - `min_level` (pre-#1004) does: any server that omitted it was
        //     already clamping to GROUP_LEVEL_MIN, so it defaults to 1. A 0
        //     here would draw the phantom rung this field exists to remove.
        let legacy: PowerGroupEntry =
            serde_json::from_str(r#"{"id":"helm","label":"HELM","level":2,"max_level":4}"#)
                .unwrap();
        assert_eq!(legacy.commanded_level, 0);
        assert_eq!(
            legacy.min_level,
            crate::ship::config::default_min_power_level(),
            "a pre-#1004 entry must decode to the engine's floor, not to 0"
        );
        assert_eq!(legacy.min_level, 1);
    }

    // ── Batch inbound decode (issue #602) ────────────────────────────────

    #[test]
    fn decode_bridge_client_messages_passes_valid_json() {
        let entries = vec![(
            "t1".into(),
            r#"{"type":"SetReady","data":{"ready":true}}"#.into(),
        )];
        let (successes, failures) = decode_bridge_client_messages(entries);
        assert_eq!(successes.len(), 1);
        assert!(failures.is_empty());
        assert_eq!(successes[0].0, "t1");
        assert_eq!(successes[0].1, ClientMessage::SetReady { ready: true });
    }

    #[test]
    fn decode_bridge_client_messages_logs_garbage_with_truncated_fields() {
        let entries = vec![
            ("t1".into(), "{{{bogus}}}".into()),
            (
                "this-is-a-very-long-token-value-that-exceeds-twelve".into(),
                "x".repeat(200),
            ),
        ];
        let (successes, failures) = decode_bridge_client_messages(entries);
        assert!(successes.is_empty());
        assert_eq!(failures.len(), 2);
        // Token truncated to 12 chars
        assert_eq!(failures[0].token, "t1");
        assert_eq!(failures[1].token, "this-is-a-ve");
        // Payload truncated to 80 chars
        assert_eq!(failures[0].payload_snippet.len(), 11);
        assert_eq!(failures[1].payload_snippet.len(), 80);
    }

    #[test]
    fn decode_bridge_client_messages_mixed_valid_and_invalid() {
        let entries = vec![
            (
                "t1".into(),
                r#"{"type":"SetReady","data":{"ready":true}}"#.into(),
            ),
            ("t2".into(), "{{{garbage}}}".into()),
        ];
        let (successes, failures) = decode_bridge_client_messages(entries);
        assert_eq!(successes.len(), 1);
        assert_eq!(failures.len(), 1);
        assert_eq!(successes[0].0, "t1");
        assert_eq!(failures[0].token, "t2");
    }

    // ── station_systems round-trip (issue #625) ───────────────────────────

    #[test]
    fn ship_client_config_station_systems_round_trips() {
        // Build a config that carries a station→system map.
        let mut station_systems = HashMap::new();
        station_systems.insert(
            "helm".to_string(),
            vec!["helm".to_string(), "helm-engine-port".to_string()],
        );
        station_systems.insert("tactical".to_string(), vec!["tactical".to_string()]);
        let config = ShipClientConfig {
            station_systems,
            blaster_banks: vec![BlasterBankClientConfig {
                id: "fore".into(),
                facing_deg: 0.0,
                fire_arc_deg: 120.0,
                cooldown_secs: 2.5,
            }],
            ..ShipClientConfig::default()
        };
        let msg = ServerMessage::Welcome {
            state: state(),
            ship_stations: empty_ship_stations(),
            ship_config: config.clone(),
            station_ratings: HashMap::new(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg.clone());
        // Verify the station_systems field survives the round-trip.
        let json = JsonCodec.encode_server(&msg).unwrap();
        let decoded = JsonCodec.decode_server(&json).unwrap();
        if let ServerMessage::Welcome { ship_config, .. } = decoded {
            assert_eq!(ship_config.station_systems, config.station_systems);
            assert_eq!(ship_config.blaster_banks, config.blaster_banks);
        } else {
            panic!("expected Welcome");
        }
    }

    #[test]
    fn ship_client_config_station_systems_defaults_empty_when_missing() {
        // Old server payloads without station_systems should decode cleanly.
        // Build a minimal Welcome message, encode it, strip the station_systems
        // key, then re-decode — the #[serde(default)] must fill in an empty map.
        let msg = ServerMessage::Welcome {
            state: state(),
            ship_stations: empty_ship_stations(),
            ship_config: ShipClientConfig::default(),
            station_ratings: HashMap::new(),
        };
        let full_json = JsonCodec.encode_server(&msg).unwrap();
        // Remove the station_systems entry to simulate an old server payload.
        let stripped = full_json.replace(",\"station_systems\":{}", "");
        let decoded = JsonCodec.decode_server(&stripped).unwrap();
        if let ServerMessage::Welcome { ship_config, .. } = decoded {
            assert!(
                ship_config.station_systems.is_empty(),
                "station_systems defaults to empty map"
            );
        } else {
            panic!("expected Welcome");
        }
    }

    #[test]
    fn ship_client_config_helm_capability_round_trips() {
        // Build a config that carries helm capability fields.
        let config = ShipClientConfig {
            helm_systems: vec![
                "helm-thrust".to_string(),
                "helm-steering".to_string(),
                "helm-impulse".to_string(),
                "helm-boost".to_string(),
                "helm-lateral-thrust".to_string(),
            ],
            vertical_movement_mode: "bounded".to_string(),
            impulse_steering_multiplier: 0.1,
            ..ShipClientConfig::default()
        };
        let msg = ServerMessage::Welcome {
            state: state(),
            ship_stations: empty_ship_stations(),
            ship_config: config.clone(),
            station_ratings: HashMap::new(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg.clone());
        // Verify the helm capability fields survive the round-trip.
        let json = JsonCodec.encode_server(&msg).unwrap();
        let decoded = JsonCodec.decode_server(&json).unwrap();
        if let ServerMessage::Welcome { ship_config, .. } = decoded {
            assert_eq!(ship_config.helm_systems, config.helm_systems);
            assert_eq!(
                ship_config.vertical_movement_mode,
                config.vertical_movement_mode
            );
            assert_eq!(
                ship_config.impulse_steering_multiplier,
                config.impulse_steering_multiplier
            );
        } else {
            panic!("expected Welcome");
        }
    }

    // ── station_tutorials round-trip (issue #916) ─────────────────────────

    #[test]
    fn ship_client_config_station_tutorials_round_trip() {
        // One overlay per shipped trigger kind, exercising every optional
        // field of the trigger vocabulary. Content fields are strings.csv ids
        // (structured codes on the wire — never composed English).
        let mut station_tutorials = HashMap::new();
        station_tutorials.insert(
            "helm".to_string(),
            vec![
                TutorialOverlayWire {
                    id: "helm-welcome".into(),
                    trigger: TutorialTriggerWire {
                        kind: "first_visit".into(),
                        control: None,
                        path: None,
                        op: None,
                        value: None,
                    },
                    title: "entity.test.station.helm.tutorial.welcome.title".into(),
                    text: "entity.test.station.helm.tutorial.welcome.text".into(),
                    anchor: Some("helm-radar".into()),
                    priority: 0,
                },
                TutorialOverlayWire {
                    id: "helm-boost".into(),
                    trigger: TutorialTriggerWire {
                        kind: "state".into(),
                        control: Some("set_boost".into()),
                        path: Some("boost_battery".into()),
                        op: Some("gte".into()),
                        value: Some(1.0),
                    },
                    title: "entity.test.station.helm.tutorial.boost.title".into(),
                    text: "entity.test.station.helm.tutorial.boost.text".into(),
                    anchor: None,
                    priority: 10,
                },
            ],
        );
        let config = ShipClientConfig {
            station_tutorials,
            ..ShipClientConfig::default()
        };
        let msg = ServerMessage::Welcome {
            state: state(),
            ship_stations: empty_ship_stations(),
            ship_config: config.clone(),
            station_ratings: HashMap::new(),
        };
        assert_server_roundtrip(&JsonCodec, msg.clone());
        assert_server_roundtrip(&PrettyJsonCodec, msg.clone());
        let json = JsonCodec.encode_server(&msg).unwrap();
        let decoded = JsonCodec.decode_server(&json).unwrap();
        if let ServerMessage::Welcome { ship_config, .. } = decoded {
            assert_eq!(ship_config.station_tutorials, config.station_tutorials);
        } else {
            panic!("expected Welcome");
        }
    }

    #[test]
    fn ship_client_config_station_tutorials_default_empty_when_missing() {
        // A Welcome from a build predating #916 carries no station_tutorials
        // key at all (skip_serializing_if on the sender side too) — the field
        // must decode as an empty map, not fail.
        let msg = ServerMessage::Welcome {
            state: state(),
            ship_stations: empty_ship_stations(),
            ship_config: ShipClientConfig::default(),
            station_ratings: HashMap::new(),
        };
        let json = JsonCodec.encode_server(&msg).unwrap();
        assert!(
            !json.contains("station_tutorials"),
            "empty map must be skipped on encode"
        );
        let decoded = JsonCodec.decode_server(&json).unwrap();
        if let ServerMessage::Welcome { ship_config, .. } = decoded {
            assert!(ship_config.station_tutorials.is_empty());
        } else {
            panic!("expected Welcome");
        }
    }
}
