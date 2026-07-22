use bevy::prelude::*;

use crate::lobby_handler;
use crate::lobby_handler::CountdownAction;
pub use crate::lobby_handler::Target;
use crate::messages::{
    ClientMessage, DeliveryClass, GamePhase, GameState, ServerMessage, ShipClientConfig, WorldData,
};
use crate::server::asset_preload::AssetPreloadResource;
use crate::session::SessionManager;
use crate::ship::rating;
use crate::ship_plugin::{
    load_ship_config_from_disk, ActiveStationRatings, PendingShipConfig, ShipConfigComponent,
    ShipSystemControlSources,
};
use crate::stations_config::{stations_from_ship_config, ShipStations};

/// Server-authoritative pre-game countdown. When `remaining_secs > 0.0` the
/// lobby is counting down and `pending_phase` is the target after the timer
/// expires. Anyone unreadying, disconnecting, or a new player joining resets
/// this timer (via `CountdownAction::Cancel`).
#[derive(Resource)]
pub struct CountdownTimer {
    pub remaining_secs: f32,
    pub pending_phase: Option<GamePhase>,
}

impl Default for CountdownTimer {
    fn default() -> Self {
        CountdownTimer {
            remaining_secs: 0.0,
            pending_phase: None,
        }
    }
}

/// Cached `GameState` snapshot derived from `Sessions` + `GamePhase` each frame.
/// Renderer systems read this instead of accessing `Sessions` directly.
#[derive(Resource, Clone)]
pub struct GameStateCache(pub GameState);

/// Pending outbound messages produced by lobby systems.
/// Drained each frame by `drain_lobby_outbox`, which runs unconditionally so
/// messages queued on the Lobby→InProgress transition frame (e.g. GameStarted)
/// are not lost.
#[derive(Resource, Default)]
pub struct LobbyOutbox(pub Vec<(Target, ServerMessage)>);

// ── Resources ──────────────────────────────────────────────────────────────

#[derive(Resource)]
pub struct Sessions(pub SessionManager);

/// Bevy resource wrapping the per-ship client config sent in `Welcome`.
/// Populated from the loaded ship TOML by `update_session_with_config`.
///
/// **Legitimately player-only.** This is the subset of ship config that
/// the browser client needs (radar range, chart range, target radii, etc.).
/// Only the LocalShip has a browser client, so a single Resource is
/// sufficient — NPCs do not have consoles that need this data. The full
/// per-ship config lives on the `ShipConfigComponent` per entity.
#[derive(Resource, Default)]
pub struct ShipClientConfigResource(pub ShipClientConfig);

/// Server's authoritative copy of the world layout — populated once during
/// world setup and broadcast to clients via `WorldSetup` after `StartGame`,
/// and replayed inside `Welcome` for mid-game reconnects.
#[derive(Resource, Clone, Default)]
pub struct WorldResource(pub WorldData);

/// Template path of the player ship selected during the host first screen.
/// Set by JS via `wasm_select_ship` before `wasm_init`. Defaults to
/// `"assets/entities/alliance_cruiser.toml"` for legacy worlds that don't
/// expose an `available_ships` list.
#[derive(Resource, Clone)]
pub struct SelectedShipResource(pub String);

// ── Messages (Bevy 0.18 pull-based message system) ─────────────────────────

/// A decoded ClientMessage received from one peer, tagged with the sender's
/// session token.
#[derive(Message, Clone)]
pub struct InboundMessage {
    pub token: String,
    pub msg: ClientMessage,
}

/// A lifecycle event signalled by the transport layer when a peer disconnects.
#[derive(Message, Clone)]
pub struct PlayerDisconnected {
    pub token: String,
}

/// A ServerMessage to be forwarded to one or all peers by the JS bridge.
#[derive(Message, Clone)]
pub struct OutboundMessage {
    pub target: Target,
    pub msg: ServerMessage,
    pub delivery: DeliveryClass,
}

// ── System set ─────────────────────────────────────────────────────────────

/// Ordering anchor for every lobby `Update` system: `handle_disconnect` runs
/// first, then the eight per-variant message systems (Identify / SetName /
/// ReturnToLobby / ConfirmScenario plus the four station-management systems),
/// then `tick_countdown → update_game_state_cache`. Downstream systems that must
/// observe the post-lobby world state order themselves with
/// `.after(LobbySystemSet)`.
#[derive(bevy::ecs::schedule::SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LobbySystemSet;

// ── Plugin ─────────────────────────────────────────────────────────────────

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<bevy::state::app::StatesPlugin>() {
            app.add_plugins(bevy::state::app::StatesPlugin);
        }
        let initial_cache = GameStateCache(GameState {
            phase: GamePhase::Lobby,
            players: vec![],
            world: None,
        });
        app.insert_resource(Sessions(SessionManager::new()))
            .insert_resource(initial_cache)
            .insert_resource(LobbyOutbox::default())
            .insert_resource(ShipClientConfigResource::default())
            .init_resource::<ShipStations>()
            .init_resource::<CountdownTimer>()
            .init_state::<GamePhase>()
            .add_message::<InboundMessage>()
            .add_message::<OutboundMessage>()
            .add_message::<PlayerDisconnected>()
            .add_systems(Startup, update_session_with_config)
            // Ordering scaffold (replaces the former monolithic `process_lobby`
            // chain). `handle_disconnect` must run before the per-variant message
            // systems so that when a stale disconnect and the reconnect `Identify`
            // land in the same frame (a browser refresh), the seat is vacated+saved
            // first and then restored — not the reverse, which would leave the
            // player marked disconnected with their seat cleared. `tick_countdown`
            // runs after the message systems (but before the outbox drain) so
            // countdown broadcasts reach the outbound bus.
            .add_systems(
                Update,
                (handle_disconnect, tick_countdown, update_game_state_cache)
                    .chain()
                    .in_set(LobbySystemSet),
            )
            // Per-variant message systems (issues #733 + #734). Each owns exactly
            // one ClientMessage variant, reading it via its own `MessageReader`
            // cursor. They replace the monolithic `process_lobby`/`process_message`
            // dispatch. All run after `handle_disconnect` and before
            // `tick_countdown`. They carry different phase gates, so they are
            // registered per matching gate:
            //
            // Identify + the four station systems gate on
            // Lobby/Loading/InProgress (claim/release/toggle + mid-game reconnect).
            .add_systems(
                Update,
                (
                    handle_identify_system,
                    handle_select_station_system,
                    handle_release_station_system,
                    handle_set_ready_system,
                    handle_set_station_rating_system,
                )
                    .in_set(LobbySystemSet)
                    .after(handle_disconnect)
                    .before(tick_countdown)
                    .run_if(
                        in_state(GamePhase::Lobby)
                            .or(in_state(GamePhase::Loading))
                            .or(in_state(GamePhase::InProgress)),
                    ),
            )
            // SetName gates on Lobby/Loading (rename before the game starts).
            .add_systems(
                Update,
                handle_set_name_system
                    .in_set(LobbySystemSet)
                    .after(handle_disconnect)
                    .before(tick_countdown)
                    .run_if(in_state(GamePhase::Lobby).or(in_state(GamePhase::Loading))),
            )
            // ReturnToLobby gates on GameOver (the game-over screen's button).
            .add_systems(
                Update,
                handle_return_to_lobby_system
                    .in_set(LobbySystemSet)
                    .after(handle_disconnect)
                    .before(tick_countdown)
                    .run_if(in_state(GamePhase::GameOver)),
            )
            // ConfirmScenario gates on Lobby (scenario picker confirmation)
            // and on the host local-console token (issue #822).
            .add_systems(
                Update,
                handle_confirm_scenario_system
                    .in_set(LobbySystemSet)
                    .after(handle_disconnect)
                    .before(tick_countdown)
                    .run_if(in_state(GamePhase::Lobby)),
            );
    }
}

/// Update the Sessions resource with available consoles from the ship's EntityConfig.
fn update_session_with_config(
    mut ship_stations: ResMut<ShipStations>,
    mut ship_client_config: ResMut<ShipClientConfigResource>,
    pending_ship_config: Option<Res<PendingShipConfig>>,
    selected_ship: Option<Res<SelectedShipResource>>,
) {
    let ship_config_resource = if let Some(pending) = pending_ship_config {
        ShipConfigComponent(pending.0.clone())
    } else {
        load_ship_config_from_disk()
    };
    if ship_stations.stations.is_empty() {
        *ship_stations = stations_from_ship_config(&ship_config_resource.0);
    }

    // Use the selected ship path (from available_ships) or fall back to the
    // legacy default for worlds without an `available_ships` list.
    let config_path = selected_ship
        .as_ref()
        .map(|s| s.0.as_str())
        .unwrap_or("assets/entities/alliance_cruiser.toml");

    if let Some(ship_config) = crate::config_cache::get_config_cache().get(config_path) {
        // Build the client-facing ship config from the same source-of-truth.
        // `HelmConsoleConfig::effective_radar_range()` prefers the structured
        // [helm_console.radar] range when present, falling back to the legacy
        // flat radar_range field, then to the Default.
        let mut next = ShipClientConfig::default();
        if let Some(hc) = &ship_config.helm_console {
            let range = hc.effective_radar_range();
            if range > 0.0 {
                next.helm_radar_range = range;
            }
            // Push the configured impulse charge duration to the client so
            // the helm progress bar advances at the same rate the server
            // is ticking.
            next.impulse_charge_duration = hc.impulse_charge_duration;
        }
        // [repair] block — pushes repair-team timings to the client so the
        // Repair panel can derive its progress-bar durations without knowing
        // server-side constants. Absent block keeps defaults that match the
        // historical hardcoded constants.
        if let Some(rc) = &ship_config.repair {
            if rc.repair_team_count > 0 {
                next.repair_team_count = rc.repair_team_count as u8;
            }
            next.repair_travel_secs = rc.travel_duration_secs;
            next.repair_rate_hp_per_sec = rc.repair_rate_hp_per_sec;
        }
        // [weapons_console] — push phaser banks (id/facing/fire_arc/cooldown
        // only; auto_arc_deg stays server-side) and the beam/arc colours so
        // the Tactical UI can render fire arcs, colour fire buttons, and
        // size the per-bank cooldown bar.
        if let Some(wc) = &ship_config.weapons_console {
            next.phaser_banks = wc
                .phaser_banks
                .iter()
                .map(|b| crate::core::messages::PhaserBankClientConfig {
                    id: b.id.clone(),
                    facing_deg: b.facing_deg,
                    fire_arc_deg: b.fire_arc_deg,
                    // Mirror the server's "zero means absent" fallback so
                    // the client always sees the real cooldown duration.
                    cooldown_secs: if b.cooldown_secs > 0.0 {
                        b.cooldown_secs
                    } else {
                        crate::entity_config::PhaserCombatConfig::DEFAULT_BEAM_COOLDOWN_SECS
                    },
                })
                .collect();
            let empty_color: Vec<f32> = vec![];
            let beam_color_src = wc
                .phaser_banks
                .first()
                .map(|b| &b.beam_color)
                .unwrap_or(&empty_color);
            if beam_color_src.len() == 4 {
                next.phaser_beam_color = [
                    beam_color_src[0],
                    beam_color_src[1],
                    beam_color_src[2],
                    beam_color_src[3],
                ];
            }
            if wc.torpedo_arc_color.len() == 4 {
                next.torpedo_arc_color = [
                    wc.torpedo_arc_color[0],
                    wc.torpedo_arc_color[1],
                    wc.torpedo_arc_color[2],
                    wc.torpedo_arc_color[3],
                ];
            }
        }
        // [torpedoes] — per-tube layout (id/facing/fire_arc).
        if let Some(tc) = &ship_config.torpedoes {
            next.torpedo_tubes = tc
                .tubes
                .iter()
                .map(|t| crate::core::messages::TorpedoTubeClientConfig {
                    id: t.id.clone(),
                    facing_deg: t.facing_deg,
                    fire_arc_deg: t.fire_arc_deg,
                })
                .collect();
        }
        // [weapons_console.blaster_banks] — per-bank layout (id/facing/fire_arc/cooldown).
        // Mirrors the phaser "zero means absent" fallback so clients always see the real
        // cooldown duration. Default cooldown is 3.0 s (matches BlasterBankConfig default).
        if let Some(wc) = &ship_config.weapons_console {
            next.blaster_banks = wc
                .blaster_banks
                .iter()
                .map(|b| crate::core::messages::BlasterBankClientConfig {
                    id: b.id.clone(),
                    facing_deg: b.facing_deg,
                    fire_arc_deg: b.fire_arc_deg,
                    cooldown_secs: if b.cooldown_secs > 0.0 {
                        b.cooldown_secs
                    } else {
                        3.0
                    },
                })
                .collect();
        }
        // Radar shows lists — push the TOML-configured tag filters to the
        // client so each console widget can build its RadarFilter without
        // hardcoding tag names.
        if let Some(hc) = &ship_config.helm_console {
            if let Some(r) = &hc.radar {
                next.helm_radar_shows = r.shows.iter().map(|t| t.as_str().to_string()).collect();
            }
        }
        if let Some(sc) = &ship_config.sensors_console {
            next.sensors_radar_range = sc.long_range_radar.range;
            next.sensors_radar_shows = sc
                .long_range_radar
                .shows
                .iter()
                .map(|t| t.as_str().to_string())
                .collect();
            next.sensors_radar_selects = sc
                .long_range_radar
                .selects
                .iter()
                .map(|t| t.as_str().to_string())
                .collect();
        }
        if let Some(nc) = &ship_config.navigation_console {
            next.nav_chart_shows = nc
                .system_chart
                .shows
                .iter()
                .map(|t| t.as_str().to_string())
                .collect();
            next.nav_chart_selects = nc
                .system_chart
                .selects
                .iter()
                .map(|t| t.as_str().to_string())
                .collect();
            if nc.system_chart.range > 0.0 {
                next.nav_chart_range = nc.system_chart.range;
            }
        }
        if let Some(wc) = &ship_config.weapons_console {
            if let Some(r) = &wc.radar {
                next.tactical_radar_shows =
                    r.shows.iter().map(|t| t.as_str().to_string()).collect();
                next.tactical_radar_selects =
                    r.selects.iter().map(|t| t.as_str().to_string()).collect();
                next.tactical_radar_range = r.range;
            }
        }
        // Ship identity metadata — class, hull_id, power_rating, css.
        next.class = ship_config.class.clone();
        next.hull_id = ship_config.hull_id.clone();
        next.power_rating = ship_config.power_rating;
        next.ship_css = ship_config.css.clone();
        // Station→system membership map: lets the client aggregate per-station
        // hull without knowing the ship layout. Iterate the stations block of
        // the TOML and collect system ids per station.
        if let Some(sc) = ship_config.ship_config.as_ref() {
            next.station_systems = sc
                .stations
                .iter()
                .map(|station| {
                    let system_ids = sc
                        .systems_for_station(&station.id)
                        .map(|sys| sys.id.0.clone())
                        .collect();
                    (station.id.0.clone(), system_ids)
                })
                .collect();
        }
        // Helm capability fields — sourced from [helm_capability] if present.
        // helm_systems: all system ids owned by the helm station.
        if let Some(sc) = ship_config.ship_config.as_ref() {
            let helm_station_id = crate::messages::StationId("helm".into());
            next.helm_systems = sc
                .systems_for_station(&helm_station_id)
                .map(|sys| sys.id.0.clone())
                .collect();
        }
        if let Some(cap) = &ship_config.helm_capability {
            next.vertical_movement_mode = match cap.vertical_movement_mode {
                crate::entity_config::VerticalMovementMode::Planar => "planar".to_string(),
                crate::entity_config::VerticalMovementMode::Bounded => "bounded".to_string(),
                crate::entity_config::VerticalMovementMode::Full3D => "full_3d".to_string(),
            };
            next.impulse_steering_multiplier = cap.impulse.steering_multiplier;
        }
        ship_client_config.0 = next;
    }
}

pub fn update_game_state_cache(
    sessions: Res<Sessions>,
    state: Res<State<GamePhase>>,
    world: Option<Res<WorldResource>>,
    mut cache: ResMut<GameStateCache>,
) {
    if !sessions.is_changed() && !state.is_changed() {
        return;
    }
    let world_data = world.as_ref().map(|w| &w.0);
    cache.0 = lobby_handler::derive_game_state(&sessions.0, state.get(), world_data);
}

// ── Systems ────────────────────────────────────────────────────────────────

/// Per-variant system for `ClientMessage::Identify` (issue #734) — the reconnect
/// handshake. Gated on Lobby/Loading/InProgress so a browser refresh mid-game
/// still receives its `Welcome` and has its seat restored. Every parameter is
/// sourced exactly as the former `process_lobby` Identify path did:
/// `world` from `WorldResource`, `ship_stations` with a `default()` fallback,
/// `ship_config` from `ShipClientConfigResource`, and the ratings SNAPSHOT from
/// either `active_ratings` (ship present) or `pending_ratings()` (pre-spawn).
/// `handle_identify` takes no `preload_complete` (only `SetReady` needed it).
pub fn handle_identify_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    world: Option<Res<WorldResource>>,
    ship_stations: Option<Res<ShipStations>>,
    ship_client_config: Res<ShipClientConfigResource>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let world_data = world.as_ref().map(|w| &w.0);
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::Identify { token, name } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let ratings_snapshot = active_ratings.0.clone();
            let result = lobby_handler::handle_identify(
                token,
                name,
                &mut sessions.0,
                phase.clone(),
                world_data,
                stations,
                &ship_client_config.0,
                &ratings_snapshot,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            // No Ship entity yet (Lobby/Loading) — fall back to whatever
            // ratings players have picked in the lobby so far, so (re)joining
            // clients' Welcome reflects current toggle state.
            let pending_ratings = sessions.0.pending_ratings().clone();
            let result = lobby_handler::handle_identify(
                token,
                name,
                &mut sessions.0,
                phase.clone(),
                world_data,
                stations,
                &ship_client_config.0,
                &pending_ratings,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::SetName` (issue #734). Gated on
/// Lobby/Loading. The result only carries outbound (a `NameChanged` broadcast),
/// but the dual-path `apply_result` call mirrors the other systems for
/// consistency.
pub fn handle_set_name_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::SetName { name } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = lobby_handler::handle_set_name(&ev.token, name, &mut sessions.0);
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = lobby_handler::handle_set_name(&ev.token, name, &mut sessions.0);
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::ReturnToLobby` (issue #734). Gated on
/// GameOver — the game-over screen's "return to lobby" button. `apply_result`
/// routes the phase transition back to `Lobby` plus the cleared-ready / returned
/// broadcasts.
pub fn handle_return_to_lobby_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::ReturnToLobby = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = lobby_handler::handle_return_to_lobby(&mut sessions.0, phase.clone());
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = lobby_handler::handle_return_to_lobby(&mut sessions.0, phase.clone());
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::ConfirmScenario` (issue #734). Gated
/// on Lobby. Needs almost nothing, but still routes its outbound
/// (`ScenarioLoaded`) through `apply_result`. The pure handler additionally
/// gates on the sender token (issue #822): only the host page's
/// `LOCAL_CONSOLE_TOKEN` may confirm a scenario.
pub fn handle_confirm_scenario_system(
    mut inbound: MessageReader<InboundMessage>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::ConfirmScenario = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = lobby_handler::handle_confirm_scenario(&ev.token, phase.clone());
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = lobby_handler::handle_confirm_scenario(&ev.token, phase.clone());
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::SelectStation` (issue #733).
/// Reads its variant off the inbound bus with its own cursor, calls the pure
/// `lobby_handler::handle_select_station`, then applies the result to Bevy
/// resources via `apply_result` — using the same dual-path
/// (real ship entity vs. pre-spawn fallback) handling as the other lobby
/// message systems.
pub fn handle_select_station_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    ship_stations: Option<Res<ShipStations>>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::SelectStation { station } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = lobby_handler::handle_select_station(
                &ev.token,
                station,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = lobby_handler::handle_select_station(
                &ev.token,
                station,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::ReleaseStation` (issue #733).
pub fn handle_release_station_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    ship_stations: Option<Res<ShipStations>>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::ReleaseStation = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = lobby_handler::handle_release_station(
                &ev.token,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = lobby_handler::handle_release_station(
                &ev.token,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::SetReady` (issue #733).
pub fn handle_set_ready_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    ship_stations: Option<Res<ShipStations>>,
    preload: Option<Res<AssetPreloadResource>>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let phase = state.get().clone();
    // Preload gate: same logic as handle_disconnect.
    let preload_complete = if crate::debug_overlay::is_playwright_automation() {
        true
    } else {
        preload
            .as_ref()
            .map(|p| !p.started || p.complete)
            .unwrap_or(true)
    };
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::SetReady { ready } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = lobby_handler::handle_set_ready(
                &ev.token,
                *ready,
                &mut sessions.0,
                phase.clone(),
                preload_complete,
                stations,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = lobby_handler::handle_set_ready(
                &ev.token,
                *ready,
                &mut sessions.0,
                phase.clone(),
                preload_complete,
                stations,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

/// Per-variant system for `ClientMessage::SetStationRating` (issue #733).
/// The pure handler only acts in Lobby/Loading (InProgress rating changes are
/// applied against the live Ship entity by
/// `ship_plugin::handle_station_rating_change`), so the InProgress run is a
/// no-op here — but the system is still gated on InProgress per the user's
/// decision to keep the four station systems on a uniform phase gate.
pub fn handle_set_station_rating_system(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    ship_stations: Option<Res<ShipStations>>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let phase = state.get().clone();
    let events: Vec<_> = inbound.read().cloned().collect();
    for ev in events {
        let ClientMessage::SetStationRating { rating_name } = &ev.msg else {
            continue;
        };
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let result = lobby_handler::handle_set_station_rating(
                &ev.token,
                rating_name,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = lobby_handler::handle_set_station_rating(
                &ev.token,
                rating_name,
                &mut sessions.0,
                phase.clone(),
                stations,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

fn handle_disconnect(
    mut events: MessageReader<PlayerDisconnected>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<crate::server_app::LocalShip>,
    >,
    stations: Option<Res<ShipStations>>,
    preload: Option<Res<AssetPreloadResource>>,
    mut countdown: Option<ResMut<CountdownTimer>>,
) {
    let empty_stations = ShipStations::default();
    let ship_stations = stations.as_deref().unwrap_or(&empty_stations);

    // Preload gate: same logic as handle_set_ready_system.
    let preload_complete = if crate::debug_overlay::is_playwright_automation() {
        true
    } else {
        preload
            .as_ref()
            .map(|p| !p.started || p.complete)
            .unwrap_or(true)
    };

    for ev in events.read() {
        // Apply Backfill rating to the disconnecting player's station so the
        // ship keeps operating without a human at the console.
        // ship_query may return Err if the Ship entity hasn't spawned yet.
        if let Ok((cfg, mut cs, mut active_ratings)) = ship_query.single_mut() {
            let ratings_snapshot = active_ratings.0.clone();
            let result = lobby_handler::process_disconnect_with_stations(
                &ev.token,
                &mut sessions.0,
                ship_stations,
                &cfg.0,
                &mut cs.0,
                &ratings_snapshot,
                state.get().clone(),
                preload_complete,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
                countdown.as_deref_mut(),
            );
        } else {
            let result = lobby_handler::process_disconnect(
                &ev.token,
                &mut sessions.0,
                state.get().clone(),
                preload_complete,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
                countdown.as_deref_mut(),
            );
        }
    }
}

fn apply_result(
    result: lobby_handler::LobbyHandlerResult,
    outbox: &mut ResMut<LobbyOutbox>,
    next_state: &mut ResMut<NextState<GamePhase>>,
    ship_config: Option<&ShipConfigComponent>,
    control_sources: Option<&mut ShipSystemControlSources>,
    active_ratings: &mut ActiveStationRatings,
    mut countdown: Option<&mut CountdownTimer>,
) {
    // Handle countdown actions before the phase transition so the cancel
    // broadcast goes out on the same frame as the unready message.
    if let Some(ref action) = result.countdown_action {
        if let Some(ref mut timer) = countdown {
            match action {
                CountdownAction::Start {
                    secs,
                    pending_phase,
                } if timer.remaining_secs <= 0.0 => {
                    timer.remaining_secs = *secs as f32;
                    timer.pending_phase = Some(pending_phase.clone());
                    outbox.0.push((
                        Target::All,
                        ServerMessage::GameStartCountdown {
                            remaining_secs: *secs,
                        },
                    ));
                }
                CountdownAction::Cancel if timer.remaining_secs > 0.0 => {
                    timer.remaining_secs = 0.0;
                    timer.pending_phase = None;
                    outbox.0.push((
                        Target::All,
                        ServerMessage::GameStartCountdown { remaining_secs: 0 },
                    ));
                }
                _ => {}
            }
        }
    }

    if let Some(new_phase) = result.new_phase {
        next_state.set(new_phase);
    }
    if let Some((station_id, rating_name)) = result.station_rating_update {
        if let (Some(cfg), Some(cs)) = (ship_config, control_sources) {
            rating::apply_rating(&cfg.0, &station_id, &rating_name, &mut cs.0);
        }
        active_ratings.0.insert(station_id, rating_name);
    }
    outbox.0.extend(result.outbound);
}

/// Ticks the pre-game countdown each frame. When the countdown reaches 0,
/// transitions to the pending phase and broadcasts `GameStarted`. Also
/// checks `all_ready()` each frame and cancels the countdown if a player
/// unreadied, disconnected, or a new player joined without readying.
fn tick_countdown(
    time: Res<Time>,
    mut timer: ResMut<CountdownTimer>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    sessions: Option<Res<Sessions>>,
) {
    if timer.remaining_secs <= 0.0 {
        return;
    }

    // Cancel if not all connected players are ready anymore.
    if let Some(ref sessions) = sessions {
        if !sessions.0.all_ready() {
            timer.remaining_secs = 0.0;
            timer.pending_phase = None;
            outbox.0.push((
                Target::All,
                ServerMessage::GameStartCountdown { remaining_secs: 0 },
            ));
            return;
        }
    }

    let prev = timer.remaining_secs;
    timer.remaining_secs -= time.delta_secs();
    if timer.remaining_secs <= 0.0 {
        // Countdown complete — transition.
        timer.remaining_secs = 0.0;
        if let Some(ref phase) = timer.pending_phase {
            next_state.set(phase.clone());
            outbox.0.push((Target::All, ServerMessage::GameStarted));
        }
        timer.pending_phase = None;
    } else {
        // Broadcast when the whole-second display changes.
        let prev_secs = prev.ceil() as u32;
        let now_secs = timer.remaining_secs.ceil() as u32;
        if now_secs != prev_secs {
            outbox.0.push((
                Target::All,
                ServerMessage::GameStartCountdown {
                    remaining_secs: now_secs,
                },
            ));
        }
    }
}

// ── Outbox drain ───────────────────────────────────────────────────────────

/// Plugin that drains [`LobbyOutbox`] into the `OutboundMessage` bus every
/// frame, regardless of the current game phase.
///
/// This phase-agnostic drain is intentional: `tick_countdown` both transitions
/// the phase to `InProgress` *and* queues `GameStarted` in the same frame.
/// A phase-gated drain (such as routing through `LobbyBroadcaster`) would skip
/// the outbox on that transition frame, causing `GameStarted` to be lost.
pub struct LobbyOutboxPlugin;

impl Plugin for LobbyOutboxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, drain_lobby_outbox.after(tick_countdown));
    }
}

pub(crate) fn drain_lobby_outbox(world: &mut World) {
    let entries = std::mem::take(&mut world.resource_mut::<LobbyOutbox>().0);
    for (target, msg) in entries {
        world.write_message(OutboundMessage {
            target,
            msg,
            delivery: DeliveryClass::Reliable,
        });
    }
}

/// Returns a [`LobbyOutboxPlugin`] that drains [`LobbyOutbox`] into the
/// `OutboundMessage` bus each frame.
///
/// This must be registered once (typically in `bridge.rs`) alongside
/// `LobbyPlugin`.
pub fn lobby_outbox_broadcaster() -> LobbyOutboxPlugin {
    LobbyOutboxPlugin
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Outbox(Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut outbox: ResMut<Outbox>) {
        for ev in reader.read() {
            outbox.0.push(ev.clone());
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(lobby_outbox_broadcaster())
            .add_plugins(bevy::time::TimePlugin)
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(1.0),
            ))
            .init_resource::<Outbox>()
            .add_systems(PostUpdate, collect);
        app
    }

    fn push(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let msgs = app.world().resource::<Outbox>().0.clone();
        app.world_mut().resource_mut::<Outbox>().0.clear();
        msgs
    }

    #[test]
    fn identify_arrives_via_inbound_message_and_welcome_is_sent_via_outbound() {
        let mut app = test_app();
        push(
            &mut app,
            "peer-id",
            ClientMessage::Identify {
                token: "t1".into(),
                name: "Alice".into(),
            },
        );
        let out = tick(&mut app);
        assert!(out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::Welcome { .. })));
    }

    #[test]
    fn select_station_works_during_in_progress_phase() {
        use crate::lobby::stations_config::stations_from_ship_config;
        use crate::messages::StationId;
        use crate::ship::config::{ShipConfig, StationConfig, StationRatingConfig};
        use std::collections::HashMap;

        let mut app = test_app();

        // Phase starts at Lobby by default.
        // Add a ship with station config before startup so
        // update_session_with_config sees non-empty stations.
        let ship_config = ShipConfig {
            stations: vec![
                StationConfig {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "Helm station".into(),
                    rank: "Crew".into(),
                    short_code: "H".into(),
                    ratings: vec![StationRatingConfig {
                        name: "Std".into(),
                        automated_systems: vec![],
                        ai_tuning: None,
                    }],
                    console: None,
                },
                StationConfig {
                    id: StationId("tactical".into()),
                    name: "Tactical".into(),
                    description: "Tactical station".into(),
                    rank: "Crew".into(),
                    short_code: "T".into(),
                    ratings: vec![StationRatingConfig {
                        name: "Std".into(),
                        automated_systems: vec![],
                        ai_tuning: None,
                    }],
                    console: None,
                },
            ],
            systems: vec![],
            power_groups: HashMap::new(),
            coordination_lag_secs: 2.0,
        };
        app.world_mut()
            .insert_resource(stations_from_ship_config(&ship_config));
        app.world_mut()
            .insert_resource(ShipClientConfigResource::default());

        // Verify stations are populated
        {
            let stations = app.world().resource::<ShipStations>();
            assert!(
                !stations.stations.is_empty(),
                "ShipStations must be non-empty"
            );
            assert_eq!(stations.stations.len(), 2, "expected 2 stations");
        }

        // Register two players in lobby first.
        // The peer ID (first arg to push) is the session token sent by the bridge,
        // and the Identify message body carries the same token for registration.
        push(
            &mut app,
            "t1",
            ClientMessage::Identify {
                token: "t1".into(),
                name: "Player1".into(),
            },
        );
        push(
            &mut app,
            "t2",
            ClientMessage::Identify {
                token: "t2".into(),
                name: "Player2".into(),
            },
        );
        tick(&mut app);

        // Player1 claims Helm in lobby
        push(
            &mut app,
            "t1",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        let out = tick(&mut app);
        assert!(out.iter().any(|m| {
            matches!(&m.msg, ServerMessage::StationAssigned { token, station, .. }
                if token == "t1" && station == &Some("Helm".into()))
        }));

        // Start the game — both ready triggers countdown
        push(&mut app, "t1", ClientMessage::SetReady { ready: true });
        push(&mut app, "t2", ClientMessage::SetReady { ready: true });
        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::GameStartCountdown { .. })),
            "ready should start countdown"
        );

        // Fast-forward the countdown by advancing the timer directly.
        use crate::lobby::CountdownTimer;
        app.world_mut()
            .resource_mut::<CountdownTimer>()
            .remaining_secs = 0.001;
        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::GameStarted)),
            "countdown expiry must emit GameStarted"
        );

        // Now in InProgress: Player2 claims Tactical (was unclaimed)
        push(
            &mut app,
            "t2",
            ClientMessage::SelectStation {
                station: "Tactical".into(),
            },
        );
        let out = tick(&mut app);
        assert!(
            out.iter().any(|m| {
                matches!(&m.msg, ServerMessage::StationAssigned { token, station, .. }
                    if token == "t2" && station == &Some("Tactical".into()))
            }),
            "SelectStation should work during InProgress phase"
        );
    }

    #[test]
    fn release_station_works_during_in_progress_phase() {
        use crate::lobby::stations_config::stations_from_ship_config;
        use crate::messages::StationId;
        use crate::ship::config::{ShipConfig, StationConfig, StationRatingConfig};
        use std::collections::HashMap;

        let mut app = test_app();

        // Phase starts at Lobby by default.
        let ship_config = ShipConfig {
            stations: vec![StationConfig {
                id: StationId("helm".into()),
                name: "Helm".into(),
                description: "Helm station".into(),
                rank: "Crew".into(),
                short_code: "H".into(),
                ratings: vec![StationRatingConfig {
                    name: "Std".into(),
                    automated_systems: vec![],
                    ai_tuning: None,
                }],
                console: None,
            }],
            systems: vec![],
            power_groups: HashMap::new(),
            coordination_lag_secs: 2.0,
        };
        app.world_mut()
            .insert_resource(stations_from_ship_config(&ship_config));
        app.world_mut()
            .insert_resource(ShipClientConfigResource::default());

        // Register player and claim station in lobby.
        // The peer ID (first arg to push) is the session token sent by the bridge.
        push(
            &mut app,
            "t1",
            ClientMessage::Identify {
                token: "t1".into(),
                name: "Player1".into(),
            },
        );
        tick(&mut app);

        push(
            &mut app,
            "t1",
            ClientMessage::SelectStation {
                station: "Helm".into(),
            },
        );
        tick(&mut app);

        // Start the game — single player ready triggers countdown
        push(&mut app, "t1", ClientMessage::SetReady { ready: true });
        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::GameStartCountdown { .. })),
            "ready should start countdown"
        );

        // Fast-forward the countdown by advancing the timer directly.
        use crate::lobby::CountdownTimer;
        app.world_mut()
            .resource_mut::<CountdownTimer>()
            .remaining_secs = 0.001;
        let out = tick(&mut app);
        assert!(
            out.iter()
                .any(|m| matches!(&m.msg, ServerMessage::GameStarted)),
            "countdown expiry must emit GameStarted"
        );

        // Now in InProgress: Player1 releases Helm
        push(&mut app, "t1", ClientMessage::ReleaseStation);
        let out = tick(&mut app);
        assert!(
            out.iter().any(|m| {
                matches!(&m.msg, ServerMessage::StationAssigned { token, station, .. }
                    if token == "t1" && station.is_none())
            }),
            "ReleaseStation should work during InProgress phase"
        );
    }

    #[test]
    fn selected_ship_resource_populates_ship_stations_via_update_session() {
        use crate::messages::StationId;
        use crate::ship::config::{ShipConfig, StationConfig, StationRatingConfig};
        use std::collections::HashMap;

        let mut app = test_app();

        // Insert PendingShipConfig so update_session_with_config uses it.
        // ShipStations starts empty (init_resource in LobbyPlugin).
        let ship_config = ShipConfig {
            stations: vec![
                StationConfig {
                    id: StationId("helm".into()),
                    name: "Helm".into(),
                    description: "Helm station".into(),
                    rank: "Crew".into(),
                    short_code: "H".into(),
                    ratings: vec![StationRatingConfig {
                        name: "Std".into(),
                        automated_systems: vec![],
                        ai_tuning: None,
                    }],
                    console: None,
                },
                StationConfig {
                    id: StationId("tactical".into()),
                    name: "Tactical".into(),
                    description: "Tactical station".into(),
                    rank: "Crew".into(),
                    short_code: "T".into(),
                    ratings: vec![StationRatingConfig {
                        name: "Std".into(),
                        automated_systems: vec![],
                        ai_tuning: None,
                    }],
                    console: None,
                },
            ],
            systems: vec![],
            power_groups: HashMap::new(),
            coordination_lag_secs: 2.0,
        };
        app.world_mut()
            .insert_resource(crate::ship_plugin::PendingShipConfig(ship_config.clone()));

        // First update runs Startup systems including update_session_with_config
        app.update();

        // Assert stations were populated from PendingShipConfig
        let stations = app.world().resource::<ShipStations>();
        assert_eq!(stations.stations.len(), 2);
        assert_eq!(stations.stations[0].id.0, "helm");
        assert_eq!(stations.stations[0].name, "Helm");
        assert_eq!(stations.stations[1].id.0, "tactical");
        assert_eq!(stations.stations[1].name, "Tactical");
    }
}
