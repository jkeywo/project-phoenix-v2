use bevy::prelude::*;

use crate::lobby_handler;
pub use crate::lobby_handler::Target;
use crate::messages::{
    ClientMessage, GamePhase, GameState, ServerMessage, ShipClientConfig, WorldData,
};
use crate::session::SessionManager;
use crate::ship::rating;
use crate::ship_plugin::{ActiveStationRatings, ShipConfigComponent, ShipSystemControlSources};
use crate::simulation::Ship;
use crate::stations_config::{stations_from_ship_config, ShipStations};

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
#[derive(Resource, Default)]
pub struct ShipClientConfigResource(pub ShipClientConfig);

/// Server's authoritative copy of the world layout — populated once during
/// world setup and broadcast to clients via `WorldSetup` after `StartGame`,
/// and replayed inside `Welcome` for mid-game reconnects.
#[derive(Resource, Clone, Default)]
pub struct WorldResource(pub WorldData);

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
}

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
            complexity: std::collections::HashMap::new(),
            world: None,
        });
        app.insert_resource(Sessions(SessionManager::new()))
            .insert_resource(initial_cache)
            .insert_resource(LobbyOutbox::default())
            .insert_resource(ShipClientConfigResource::default())
            .init_resource::<ShipStations>()
            .init_state::<GamePhase>()
            .add_message::<InboundMessage>()
            .add_message::<OutboundMessage>()
            .add_message::<PlayerDisconnected>()
            .add_systems(Startup, update_session_with_config)
            // Order matters: `handle_disconnect` must run before `process_lobby`
            // so that when a stale disconnect and the reconnect `Identify` land
            // in the same frame (a browser refresh), the seat is vacated+saved
            // first and then restored — not the reverse, which would leave the
            // player marked disconnected with their seat cleared.
            .add_systems(
                Update,
                (handle_disconnect, process_lobby, update_game_state_cache).chain(),
            );
    }
}

/// Update the Sessions resource with available consoles from the ship's EntityConfig.
fn update_session_with_config(
    mut ship_stations: ResMut<ShipStations>,
    mut ship_client_config: ResMut<ShipClientConfigResource>,
) {
    let ship_config_resource = crate::ship_plugin::load_ship_config_from_disk();
    if ship_stations.stations.is_empty() {
        *ship_stations = stations_from_ship_config(&ship_config_resource.0);
    }

    // Get the config cache from thread-local storage
    if let Some(ship_config) =
        crate::config_cache::get_config_cache().get("assets/entities/player_ship.toml")
    {
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

pub fn process_lobby(
    mut inbound: MessageReader<InboundMessage>,
    mut sessions: ResMut<Sessions>,
    state: Res<State<GamePhase>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    world: Option<Res<WorldResource>>,
    ship_stations: Option<Res<ShipStations>>,
    ship_client_config: Res<ShipClientConfigResource>,
    preload: Option<Res<crate::server::asset_preload::AssetPreloadResource>>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<Ship>,
    >,
) {
    // During the Lobby phase this system owns the inbound queue and handles
    // every message type. Outside the lobby the simulation systems own it, so
    // here we handle *only* the reconnect handshake (`Identify`) — a browser
    // refresh mid-game must still receive its `Welcome` and have its seat
    // restored. All other message types are left for the in-game systems.
    // (Bevy `MessageReader`s have independent cursors, so reading here never
    // hides messages from those systems.)
    //
    // During `Loading` the lobby system also handles inbound messages (reconnect,
    // Identify, etc.) while the asset pre-cache runs.
    let accepts_all = state.get() == &GamePhase::Lobby || state.get() == &GamePhase::Loading;
    let default_stations = ShipStations::default();
    let stations = ship_stations
        .as_ref()
        .map(|s| s.as_ref())
        .unwrap_or(&default_stations);
    let world_data = world.as_ref().map(|w| &w.0);
    // Preload gate: only Engage when the asset pre-cache has finished.
    // - If the resource is missing entirely (test app, native build that
    //   hasn't run `begin_asset_preload`), treat as complete.
    // - If the resource exists but `!started`, treat as complete too — the
    //   preload system runs every `Update` and may not have observed the
    //   lobby state yet on the first frame. This avoids a deadlock where
    //   `process_lobby` would refuse Engage forever waiting for a system
    //   that never runs.
    // - In Playwright automation mode the renderer plugins are skipped
    //   (MinimalPlugins), and asset preloading can never complete because
    //   the renderer asset pipeline isn't available. Treat preload as
    //   complete unconditionally in this environment.
    // - Otherwise gate on `preload.complete`, which is set by
    //   `poll_asset_preload` once all icons, sidecars, sub-worlds, and GLBs
    //   reach a terminal LoadState (Loaded or Failed).
    let preload_complete = if crate::debug_overlay::is_playwright_automation() {
        true
    } else {
        preload
            .as_ref()
            .map(|p| !p.started || p.complete)
            .unwrap_or(true)
    };
    // Collect all messages first to avoid borrow conflicts with ship_query.
    let events: Vec<_> = inbound
        .read()
        .filter(|ev| accepts_all || matches!(ev.msg, ClientMessage::Identify { .. }))
        .cloned()
        .collect();
    for ev in events {
        if let Some((ship_config, mut control_sources, mut active_ratings)) =
            ship_query.iter_mut().next()
        {
            let ratings_snapshot = active_ratings.0.clone();
            let result = lobby_handler::process_message(
                &ev.token,
                &ev.msg,
                &mut sessions.0,
                state.get().clone(),
                world_data,
                stations,
                &ship_client_config.0,
                preload_complete,
                &ratings_snapshot,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(ship_config),
                Some(&mut control_sources),
                &mut *active_ratings,
            );
        } else {
            let empty_ratings = std::collections::HashMap::new();
            let result = lobby_handler::process_message(
                &ev.token,
                &ev.msg,
                &mut sessions.0,
                state.get().clone(),
                world_data,
                stations,
                &ship_client_config.0,
                preload_complete,
                &empty_ratings,
            );
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
            );
        }
    }
}

fn handle_disconnect(
    mut events: MessageReader<PlayerDisconnected>,
    mut sessions: ResMut<Sessions>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut ship_query: Query<
        (
            &ShipConfigComponent,
            &mut ShipSystemControlSources,
            &mut ActiveStationRatings,
        ),
        With<Ship>,
    >,
    stations: Option<Res<ShipStations>>,
) {
    let empty_stations = ShipStations::default();
    let ship_stations = stations.as_deref().unwrap_or(&empty_stations);
    for ev in events.read() {
        // Apply Backfill rating to the disconnecting player's station so the
        // ship keeps operating without a human at the console.
        // ship_query may return Err if the Ship entity hasn't spawned yet.
        if let Some((cfg, mut cs, mut active_ratings)) = ship_query.iter_mut().next() {
            let ratings_snapshot = active_ratings.0.clone();
            let result = lobby_handler::process_disconnect_with_stations(
                &ev.token,
                &mut sessions.0,
                ship_stations,
                &cfg.0,
                &mut cs.0,
                &ratings_snapshot,
            );
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                Some(cfg),
                Some(&mut cs),
                &mut active_ratings,
            );
        } else {
            let result = lobby_handler::process_disconnect(&ev.token, &mut sessions.0);
            let mut fallback_ratings = ActiveStationRatings::default();
            apply_result(
                result,
                &mut outbox,
                &mut next_state,
                None,
                None,
                &mut fallback_ratings,
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
) {
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

// ── Outbox drain ───────────────────────────────────────────────────────────

/// Plugin that drains [`LobbyOutbox`] into the `OutboundMessage` bus every
/// frame, regardless of the current game phase.
///
/// This phase-agnostic drain is intentional: `process_lobby` both transitions
/// the phase to `InProgress` *and* queues `GameStarted` in the same frame.
/// A phase-gated drain (such as routing through `LobbyBroadcaster`) would skip
/// the outbox on that transition frame, causing `GameStarted` to be lost.
pub struct LobbyOutboxPlugin;

impl Plugin for LobbyOutboxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, drain_lobby_outbox.after(process_lobby));
    }
}

fn drain_lobby_outbox(world: &mut World) {
    let entries = std::mem::take(&mut world.resource_mut::<LobbyOutbox>().0);
    for (target, msg) in entries {
        world.write_message(OutboundMessage { target, msg });
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
}
