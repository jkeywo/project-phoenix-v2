//! Pure client-side simulation-state model.
//!
//! Mirrors the parts of `SimSnapshot` the captain UI needs to render
//! (red alert state, current view mode), updated by inbound `SimState`
//! messages, and exposes `ClientMessage` builders for the captain
//! buttons. Bevy-free so it can be exhaustively unit-tested on native.

use bevy::prelude::Resource;

use crate::messages::{ClientMessage, ServerMessage, ViewDirection, ViewMode, WorldData, PhaserMode, ShieldFacingStatus};
use crate::entity_tags::EntityTag;
use crate::radar_config::RadarConfig;
use crate::radar::{ScienceRadarView, compute_science_radar_view};

/// Range used by the Science console system chart — large enough to show the
/// full solar system layout.
pub const SYSTEM_CHART_RANGE: f32 = 500.0;

/// Returns the `RadarConfig` for the Science console System Chart tab.
///
/// Uses a large detection range and filters for navigational entities only:
/// stars, planets, and asteroid field rings.
pub fn system_chart_config() -> RadarConfig {
    RadarConfig {
        range: SYSTEM_CHART_RANGE,
        shows: vec![EntityTag::Star, EntityTag::Planet, EntityTag::AsteroidField],
    }
}

/// Compute the Science console System Chart view from the current client state.
///
/// Non-interactive: returns dots and rings for navigational entities (stars,
/// planets, asteroid fields) within `SYSTEM_CHART_RANGE` of the ship.
/// Individual asteroids are excluded (they are not navigational features).
pub fn compute_system_chart_view(state: &ClientSimState) -> ScienceRadarView {
    let config = system_chart_config();
    compute_science_radar_view(
        &state.world.asteroids,
        &state.world.asteroid_fields,
        state.ship_x,
        state.ship_z,
        state.ship_yaw,
        &config,
    )
}

/// Subset of `SimSnapshot` the client UI needs. Reset to defaults on
/// `Welcome` (which also clears `LobbyState`) and refreshed every time
/// a `SimState` message arrives.
#[derive(Clone, Debug, PartialEq, Resource)]
pub struct ClientSimState {
    pub red_alert: bool,
    pub view_mode: ViewMode,
    pub ship_x:   f32,
    pub ship_z:   f32,
    pub ship_yaw: f32,
    /// Static world snapshot replayed on `WorldSetup` and on `Welcome`
    /// (when the server includes it). Used by the helm radar.
    pub world: WorldData,
    /// Seconds remaining on the active repair action (if `repair_in_progress`)
    /// or penalty cooldown (if `repair_penalty`).
    pub repair_cooldown_secs: f32,
    /// True while this console is performing an authorized repair.
    pub repair_in_progress: bool,
    /// True while this player has an unauthorized-repair penalty cooldown.
    pub repair_penalty: bool,
    /// Current phaser firing mode.
    pub phaser_mode: PhaserMode,
    /// UUID of the last asteroid hit by a phaser shot (cleared on new shot).
    pub last_phaser_target: Option<String>,
    /// The most recent science target suggestion received from the server
    /// (None until a Science officer designates a target).
    pub science_target_suggestion: Option<String>,
    /// Latest shield facing snapshots received from the server.
    /// Empty until the first `ShieldStatus` message is received.
    pub shield_facings: Vec<ShieldFacingStatus>,
}

impl Default for ClientSimState {
    fn default() -> Self {
        Self {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x:   0.0,
            ship_z:   0.0,
            ship_yaw: 0.0,
            world: WorldData::default(),
            repair_cooldown_secs: 0.0,
            repair_in_progress: false,
            repair_penalty: false,
            phaser_mode: PhaserMode::Auto,
            last_phaser_target: None,
            science_target_suggestion: None,
            shield_facings: Vec::new(),
        }
    }
}

impl ClientSimState {
    /// Apply a single inbound `ServerMessage`. Drives both the captain
    /// console state (red alert, view mode) and the helm console state
    /// (ship pose, world snapshot for the radar).
    pub fn apply(&mut self, msg: &ServerMessage) {
        match msg {
            ServerMessage::SimState { snapshot } => {
                self.red_alert = snapshot.red_alert;
                self.view_mode = snapshot.view_mode.clone();
                self.ship_x   = snapshot.ship_x;
                self.ship_z   = snapshot.ship_z;
                self.ship_yaw = snapshot.ship_yaw;
            }
            ServerMessage::WorldSetup { world } => {
                self.world = world.clone();
            }
            ServerMessage::Welcome { state } => {
                let preserved_world = state.world.clone().unwrap_or_default();
                *self = Self::default();
                self.world = preserved_world;
            }
            ServerMessage::RepairState { remaining_cooldown_secs, in_progress, penalty } => {
                self.repair_cooldown_secs = *remaining_cooldown_secs;
                self.repair_in_progress = *in_progress;
                self.repair_penalty = *penalty;
            }
            ServerMessage::PhaserFired { target_uuid, .. } => {
                self.last_phaser_target = Some(target_uuid.clone());
            }
            ServerMessage::ScienceTargetSuggestion { uuid } => {
                self.science_target_suggestion = Some(uuid.clone());
            }
            ServerMessage::ShieldStatus { facings } => {
                self.shield_facings = facings.clone();
            }
            _ => {}
        }
    }

    /// True iff the captain's view direction selector should highlight
    /// the given direction. Radar mode highlights nothing in the cross.
    pub fn is_active_camera_direction(&self, direction: &ViewDirection) -> bool {
        matches!(&self.view_mode, ViewMode::Camera(d) if d == direction)
    }
}

/// `ClientMessage` to send when the captain presses a direction button
/// in the view-selector cross.
pub fn message_for_direction_press(direction: ViewDirection) -> ClientMessage {
    ClientMessage::SetView { mode: ViewMode::Camera(direction) }
}

/// `ClientMessage` to send when the captain presses the Red Alert toggle.
pub fn red_alert_toggle_message() -> ClientMessage {
    ClientMessage::ToggleRedAlert
}

/// `ClientMessage` for the helm "On Screen" button: switches the server
/// viewscreen to radar mode.
pub fn on_screen_message() -> ClientMessage {
    ClientMessage::SetView { mode: ViewMode::Radar }
}

/// `ClientMessage` for the Repair button: sends a repair request to the server.
pub fn repair_message() -> ClientMessage {
    ClientMessage::Repair { console: crate::messages::Console::Helm }
}

/// `ClientMessage` to send when the Science officer taps an entity on their
/// long-range radar to suggest it as a target to the Weapons console.
pub fn set_science_target_message(uuid: String) -> ClientMessage {
    ClientMessage::SetScienceTarget { uuid }
}

/// A single shield arc as rendered in the 2D top-down status diagram.
///
/// The arc is a pie slice centred on the ship sprite. `start_angle` and
/// `end_angle` are in radians, measured clockwise from "up" (forward) in
/// screen space (matching Bevy/CSS canvas convention where 0 = top, π/2 = right).
///
/// `fill_fraction` is in `[0.0, 1.0]` — the arc fills from the centre outward
/// to `max_radius * fill_fraction`. When the facing is offline the fraction is
/// 0.0.
#[derive(Clone, Debug, PartialEq)]
pub struct ShieldArcView {
    /// Human-readable label (e.g. "Fore", "Port").
    pub label: String,
    pub hp: i32,
    pub max_hp: i32,
    pub online: bool,
    /// Fraction of the maximum arc radius to fill, in `[0.0, 1.0]`.
    pub fill_fraction: f32,
    /// Start angle in radians (clockwise from up).
    pub start_angle: f32,
    /// End angle in radians (clockwise from up).
    pub end_angle: f32,
}

/// Compute the list of `ShieldArcView`s for the Science Console shield diagram.
///
/// Each facing occupies an equal pie-slice of the full circle. Facing 0 is
/// centred on forward (top of the diagram); indices increase clockwise so that
/// the standard 4-facing layout is Fore(top), Port(left), Aft(bottom),
/// Starboard(right).
///
/// `fill_fraction` is `hp / max_hp` when online; `0.0` when offline.
pub fn shield_status_view(facings: &[ShieldFacingStatus]) -> Vec<ShieldArcView> {
    let n = facings.len();
    if n == 0 {
        return Vec::new();
    }
    use std::f32::consts::TAU;
    let arc = TAU / n as f32;
    let half_arc = arc / 2.0;

    facings
        .iter()
        .enumerate()
        .map(|(i, f)| {
            // Centre of this facing's arc, clockwise from up.
            // Facing 0 → centred on 0 (top / forward).
            // Facing 1 → centred on arc (clockwise from facing 0).
            let centre_angle = i as f32 * arc;
            let start_angle = centre_angle - half_arc;
            let end_angle = centre_angle + half_arc;
            let fill_fraction = if f.online && f.max_hp > 0 {
                (f.hp as f32 / f.max_hp as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            ShieldArcView {
                label: f.label.clone(),
                hp: f.hp,
                max_hp: f.max_hp,
                online: f.online,
                fill_fraction,
                start_angle,
                end_angle,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{AsteroidInfo, Console, GamePhase, GameState, Player, SimSnapshot};

    fn snap(red_alert: bool, view_mode: ViewMode) -> SimSnapshot {
        SimSnapshot { red_alert, view_mode, ship_x: 0.0, ship_z: 0.0, ship_yaw: 0.0, hull_integrity: 100, authorized_repair_console: None }
    }

    fn snap_pose(x: f32, z: f32, yaw: f32) -> SimSnapshot {
        SimSnapshot {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x: x,
            ship_z: z,
            ship_yaw: yaw,
            hull_integrity: 100,
            authorized_repair_console: None,
        }
    }

    #[test]
    fn default_sim_state_is_calm_and_facing_forward() {
        let s = ClientSimState::default();
        assert!(!s.red_alert);
        assert_eq!(s.view_mode, ViewMode::Camera(ViewDirection::Fore));
        assert_eq!(s.ship_x, 0.0);
        assert_eq!(s.ship_z, 0.0);
        assert_eq!(s.ship_yaw, 0.0);
        assert!(s.world.asteroids.is_empty());
    }

    #[test]
    fn sim_state_message_updates_red_alert_and_view_mode() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::SimState {
            snapshot: snap(true, ViewMode::Camera(ViewDirection::Aft)),
        });
        assert!(s.red_alert);
        assert_eq!(s.view_mode, ViewMode::Camera(ViewDirection::Aft));
    }

    #[test]
    fn sim_state_message_updates_ship_pose() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::SimState { snapshot: snap_pose(12.5, -7.25, 1.5) });
        assert_eq!(s.ship_x, 12.5);
        assert_eq!(s.ship_z, -7.25);
        assert_eq!(s.ship_yaw, 1.5);
    }

    #[test]
    fn world_setup_message_populates_world_data() {
        let mut s = ClientSimState::default();
        let world = WorldData {
            asteroids: vec![
                AsteroidInfo { uuid: "a".into(), x:  3.0, z:  4.0, radius: 2.0, tags: vec![] },
                AsteroidInfo { uuid: "b".into(), x: -1.5, z:  0.0, radius: 1.0, tags: vec![] },
            ],
            asteroid_fields: vec![],
        };
        s.apply(&ServerMessage::WorldSetup { world: world.clone() });
        assert_eq!(s.world, world);
    }

    #[test]
    fn welcome_resets_sim_state_but_preserves_world_when_present() {
        let mut s = ClientSimState {
            red_alert: true,
            view_mode: ViewMode::Radar,
            ship_x: 9.0, ship_z: 9.0, ship_yaw: 1.0,
            world: WorldData::default(),
            repair_cooldown_secs: 0.0,
            repair_in_progress: false,
            repair_penalty: false,
            phaser_mode: PhaserMode::Auto,
            last_phaser_target: None,
            science_target_suggestion: None,
            shield_facings: Vec::new(),
        };
        let world = WorldData {
            asteroids: vec![AsteroidInfo { uuid: "c".into(), x: 1.0, z: 2.0, radius: 0.5, tags: vec![] }],
            asteroid_fields: vec![],
        };
        s.apply(&ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::InProgress,
                players: vec![],
                world: Some(world.clone()),
            },
        });
        // Everything except `world` must reset to defaults.
        assert!(!s.red_alert);
        assert_eq!(s.view_mode, ViewMode::default());
        assert_eq!(s.ship_x, 0.0);
        assert_eq!(s.ship_z, 0.0);
        assert_eq!(s.ship_yaw, 0.0);
        assert_eq!(s.world, world, "world from Welcome must be retained");
    }

    #[test]
    fn welcome_without_world_clears_world_to_default() {
        let mut s = ClientSimState {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x: 0.0, ship_z: 0.0, ship_yaw: 0.0,
            world: WorldData {
                asteroids: vec![AsteroidInfo { uuid: "d".into(), x: 0.0, z: 0.0, radius: 1.0, tags: vec![] }],
                asteroid_fields: vec![],
            },
            repair_cooldown_secs: 0.0,
            repair_in_progress: false,
            repair_penalty: false,
            phaser_mode: PhaserMode::Auto,
            last_phaser_target: None,
            science_target_suggestion: None,
            shield_facings: Vec::new(),
        };
        s.apply(&ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::Lobby,
                players: vec![],
                world: None,
            },
        });
        assert_eq!(s, ClientSimState::default());
    }

    #[test]
    fn unrelated_messages_do_not_disturb_sim_state() {
        let mut s = ClientSimState {
            red_alert: true,
            view_mode: ViewMode::Camera(ViewDirection::Port),
            ship_x: 5.0, ship_z: 6.0, ship_yaw: 0.7,
            world: WorldData {
                asteroids: vec![AsteroidInfo { uuid: "e".into(), x: 0.0, z: 0.0, radius: 1.0, tags: vec![] }],
                asteroid_fields: vec![],
            },
            repair_cooldown_secs: 0.0,
            repair_in_progress: false,
            repair_penalty: false,
            phaser_mode: PhaserMode::Auto,
            last_phaser_target: None,
            science_target_suggestion: None,
            shield_facings: Vec::new(),
        };
        let before = s.clone();
        s.apply(&ServerMessage::PlayerJoined {
            player: Player { token: "x".into(), name: "Y".into(), consoles: vec![Console::Helm], connected: true },
        });
        assert_eq!(s, before);
    }

    #[test]
    fn is_active_camera_direction_only_matches_in_camera_mode() {
        let mut s = ClientSimState::default();
        assert!( s.is_active_camera_direction(&ViewDirection::Fore));
        assert!(!s.is_active_camera_direction(&ViewDirection::Aft));

        s.view_mode = ViewMode::Camera(ViewDirection::Port);
        assert!( s.is_active_camera_direction(&ViewDirection::Port));
        assert!(!s.is_active_camera_direction(&ViewDirection::Fore));

        s.view_mode = ViewMode::Radar;
        for d in [ViewDirection::Fore, ViewDirection::Aft, ViewDirection::Port, ViewDirection::Starboard] {
            assert!(!s.is_active_camera_direction(&d), "Radar mode highlights no cross arrow");
        }
    }

    #[test]
    fn direction_press_builds_set_view_camera_message() {
        let msg = message_for_direction_press(ViewDirection::Starboard);
        assert_eq!(
            msg,
            ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Starboard) },
        );
    }

    #[test]
    fn red_alert_toggle_message_is_toggle_red_alert() {
        assert_eq!(red_alert_toggle_message(), ClientMessage::ToggleRedAlert);
    }

    #[test]
    fn on_screen_message_is_set_view_radar() {
        assert_eq!(
            on_screen_message(),
            ClientMessage::SetView { mode: ViewMode::Radar },
        );
    }

    #[test]
    fn science_target_suggestion_updates_state() {
        let mut s = ClientSimState::default();
        assert!(s.science_target_suggestion.is_none());
        s.apply(&ServerMessage::ScienceTargetSuggestion { uuid: "entity-abc".into() });
        assert_eq!(s.science_target_suggestion, Some("entity-abc".into()));
    }

    #[test]
    fn science_target_suggestion_is_overwritten_by_newer_message() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::ScienceTargetSuggestion { uuid: "first".into() });
        s.apply(&ServerMessage::ScienceTargetSuggestion { uuid: "second".into() });
        assert_eq!(s.science_target_suggestion, Some("second".into()));
    }

    #[test]
    fn welcome_clears_science_target_suggestion() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::ScienceTargetSuggestion { uuid: "some-entity".into() });
        s.apply(&ServerMessage::Welcome {
            state: GameState { phase: GamePhase::Lobby, players: vec![], world: None },
        });
        assert!(s.science_target_suggestion.is_none());
    }

    #[test]
    fn set_science_target_message_builder_produces_correct_message() {
        let msg = set_science_target_message("entity-uuid-42".into());
        assert_eq!(msg, ClientMessage::SetScienceTarget { uuid: "entity-uuid-42".into() });
    }

    // ── system_chart_config ──────────────────────────────────────────────

    #[test]
    fn system_chart_config_has_large_range() {
        let cfg = system_chart_config();
        assert!(cfg.range >= 200.0, "system chart range {:.0} should be large (≥200)", cfg.range);
    }

    #[test]
    fn system_chart_config_shows_star_planet_asteroid_field() {
        use crate::entity_tags::EntityTag;
        let cfg = system_chart_config();
        assert!(cfg.shows.contains(&EntityTag::Star),         "must show stars");
        assert!(cfg.shows.contains(&EntityTag::Planet),       "must show planets");
        assert!(cfg.shows.contains(&EntityTag::AsteroidField),"must show asteroid fields");
    }

    #[test]
    fn system_chart_config_does_not_show_individual_asteroids() {
        use crate::entity_tags::EntityTag;
        let cfg = system_chart_config();
        assert!(!cfg.shows.contains(&EntityTag::Asteroid), "individual asteroids are not navigational");
    }

    // ── compute_system_chart_view ────────────────────────────────────────

    fn state_with_field(x: f32, z: f32) -> ClientSimState {
        use crate::messages::{AsteroidField, WorldData};
        let mut s = ClientSimState::default();
        s.world = WorldData {
            asteroids: vec![],
            asteroid_fields: vec![
                AsteroidField {
                    uuid: "field-1".into(),
                    x, z,
                    inner_radius: 10.0,
                    outer_radius: 30.0,
                    tags: vec!["asteroid_field".into()],
                }
            ],
        };
        s
    }

    #[test]
    fn system_chart_view_empty_world_produces_empty_view() {
        let s = ClientSimState::default();
        let view = compute_system_chart_view(&s);
        assert!(view.dots.is_empty());
        assert!(view.rings.is_empty());
    }

    #[test]
    fn system_chart_view_includes_asteroid_field_ring_within_range() {
        let s = state_with_field(0.0, -100.0);
        let view = compute_system_chart_view(&s);
        assert_eq!(view.rings.len(), 1, "asteroid field within range should appear as a ring");
        assert_eq!(view.rings[0].uuid, "field-1");
    }

    #[test]
    fn system_chart_view_excludes_asteroid_field_beyond_range() {
        // Place field far outside SYSTEM_CHART_RANGE.
        let far = SYSTEM_CHART_RANGE + 200.0;
        let s = state_with_field(far, 0.0);
        let view = compute_system_chart_view(&s);
        assert!(view.rings.is_empty(), "field beyond system chart range must be excluded");
    }

    #[test]
    fn system_chart_view_excludes_individual_asteroids() {
        use crate::messages::{AsteroidInfo, WorldData};
        let mut s = ClientSimState::default();
        s.world = WorldData {
            asteroids: vec![
                AsteroidInfo { uuid: "a1".into(), x: 0.0, z: -50.0, radius: 2.0, tags: vec!["asteroid".into()] }
            ],
            asteroid_fields: vec![],
        };
        let view = compute_system_chart_view(&s);
        assert!(view.dots.is_empty(), "individual asteroids must not appear on system chart");
    }

    #[test]
    fn system_chart_view_ring_position_respects_ship_pose() {
        // Field directly ahead at 100 units (ship at origin, yaw=0 → forward is -Z).
        let s = state_with_field(0.0, -100.0);
        let view = compute_system_chart_view(&s);
        assert_eq!(view.rings.len(), 1);
        // Ring centre should be at roughly (0, positive) in radar space.
        assert!(view.rings[0].centre_y > 0.0, "field ahead should map to positive radar_y");
        let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-3, "expected {b}, got {a}");
        close(view.rings[0].centre_x, 0.0);
        close(view.rings[0].centre_y, 100.0 / SYSTEM_CHART_RANGE);
    }

    // ── shield state in ClientSimState ───────────────────────────────────

    fn make_facing(label: &str, hp: i32, max_hp: i32, online: bool) -> ShieldFacingStatus {
        ShieldFacingStatus {
            label: label.into(),
            hp,
            max_hp,
            online,
            offline_remaining: if online { 0.0 } else { 5.0 },
        }
    }

    #[test]
    fn shield_facings_default_to_empty() {
        let s = ClientSimState::default();
        assert!(s.shield_facings.is_empty());
    }

    #[test]
    fn shield_status_message_updates_facings() {
        let mut s = ClientSimState::default();
        let facings = vec![
            make_facing("Fore", 80, 100, true),
            make_facing("Port", 50, 100, true),
            make_facing("Aft", 0, 100, false),
            make_facing("Starboard", 100, 100, true),
        ];
        s.apply(&ServerMessage::ShieldStatus { facings: facings.clone() });
        assert_eq!(s.shield_facings, facings);
    }

    #[test]
    fn second_shield_status_message_overwrites_first() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::ShieldStatus {
            facings: vec![make_facing("Fore", 90, 100, true)],
        });
        s.apply(&ServerMessage::ShieldStatus {
            facings: vec![make_facing("Fore", 70, 100, true)],
        });
        assert_eq!(s.shield_facings.len(), 1);
        assert_eq!(s.shield_facings[0].hp, 70);
    }

    #[test]
    fn welcome_resets_shield_facings() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::ShieldStatus {
            facings: vec![make_facing("Fore", 80, 100, true)],
        });
        s.apply(&ServerMessage::Welcome {
            state: GameState { phase: GamePhase::Lobby, players: vec![], world: None },
        });
        assert!(s.shield_facings.is_empty());
    }

    // ── shield_status_view ───────────────────────────────────────────────

    #[test]
    fn empty_facings_produce_empty_view() {
        let view = shield_status_view(&[]);
        assert!(view.is_empty());
    }

    #[test]
    fn four_facings_produce_four_arcs() {
        let facings = vec![
            make_facing("Fore", 100, 100, true),
            make_facing("Port", 100, 100, true),
            make_facing("Aft", 100, 100, true),
            make_facing("Starboard", 100, 100, true),
        ];
        let view = shield_status_view(&facings);
        assert_eq!(view.len(), 4);
    }

    #[test]
    fn full_hp_online_facing_has_fill_fraction_one() {
        let facings = vec![make_facing("Fore", 100, 100, true)];
        let view = shield_status_view(&facings);
        assert!((view[0].fill_fraction - 1.0).abs() < 1e-4);
    }

    #[test]
    fn half_hp_online_facing_has_fill_fraction_half() {
        let facings = vec![make_facing("Fore", 50, 100, true)];
        let view = shield_status_view(&facings);
        assert!((view[0].fill_fraction - 0.5).abs() < 1e-4);
    }

    #[test]
    fn offline_facing_has_fill_fraction_zero() {
        let facings = vec![make_facing("Aft", 0, 100, false)];
        let view = shield_status_view(&facings);
        assert!((view[0].fill_fraction).abs() < 1e-4);
    }

    #[test]
    fn facing_zero_is_centred_on_top_forward() {
        use std::f32::consts::TAU;
        let facings = vec![make_facing("Fore", 100, 100, true)];
        let view = shield_status_view(&facings);
        // For n=1, arc = TAU, half_arc = TAU/2.
        // start = -TAU/2, end = +TAU/2. Centre is 0 (forward / top).
        let centre = (view[0].start_angle + view[0].end_angle) / 2.0;
        assert!(centre.abs() < 1e-4, "facing 0 centre should be 0 (forward), got {centre}");
        let _ = TAU; // used above
    }

    #[test]
    fn four_facing_arcs_cover_full_circle_without_gap() {
        use std::f32::consts::TAU;
        let facings = vec![
            make_facing("Fore", 100, 100, true),
            make_facing("Port", 100, 100, true),
            make_facing("Aft", 100, 100, true),
            make_facing("Starboard", 100, 100, true),
        ];
        let view = shield_status_view(&facings);
        // Each arc should span TAU/4.
        for arc in &view {
            let span = arc.end_angle - arc.start_angle;
            assert!((span - TAU / 4.0).abs() < 1e-4, "arc span should be TAU/4, got {span}");
        }
        // Arcs should tile seamlessly: end[i] == start[i+1].
        for i in 0..3 {
            assert!((view[i].end_angle - view[i + 1].start_angle).abs() < 1e-4);
        }
    }

    #[test]
    fn arc_view_labels_match_facing_labels() {
        let facings = vec![
            make_facing("Fore", 100, 100, true),
            make_facing("Port", 80, 100, true),
            make_facing("Aft", 0, 100, false),
            make_facing("Starboard", 100, 100, true),
        ];
        let view = shield_status_view(&facings);
        assert_eq!(view[0].label, "Fore");
        assert_eq!(view[1].label, "Port");
        assert_eq!(view[2].label, "Aft");
        assert_eq!(view[3].label, "Starboard");
    }

    #[test]
    fn arc_view_hp_and_online_match_facing_status() {
        let facings = vec![
            make_facing("Fore", 75, 100, true),
            make_facing("Aft", 0, 100, false),
        ];
        let view = shield_status_view(&facings);
        assert_eq!(view[0].hp, 75);
        assert_eq!(view[0].max_hp, 100);
        assert!(view[0].online);
        assert_eq!(view[1].hp, 0);
        assert!(!view[1].online);
    }
}
