//! Pure client-side simulation-state model.
//!
//! Mirrors the parts of `SimSnapshot` the captain UI needs to render
//! (red alert state, current view mode), updated by inbound `SimState`
//! messages, and exposes `ClientMessage` builders for the captain
//! buttons. Bevy-free so it can be exhaustively unit-tested on native.

use bevy::prelude::Resource;
use std::collections::HashMap;

use crate::messages::{ClientMessage, Console, ServerMessage, Shape, TeamSlot, ViewDirection, ViewMode, WorldData, PhaserMode, ShieldFacingStatus, TorpedoTube, ModifierSlot, ModifierSource};
use crate::entity_tags::EntityTag;
use crate::radar_config::RadarConfig;
use crate::radar::{ScienceRadarView, compute_science_radar_view};

/// Range used by the Science console system chart — large enough to show the
/// full solar system layout.
pub const SYSTEM_CHART_RANGE: f32 = 500.0;

/// Range used by the Helm console radar — tactical situational awareness.
pub const HELM_RADAR_RANGE: f32 = 50.0;

/// Range used by the Weapons console radar — target acquisition range.
pub const WEAPONS_RADAR_RANGE: f32 = 60.0;

/// Maximum range for the Science console long-range radar at full power.
///
/// This is hardcoded to the max-power value.
/// TODO: integrate with the power system so that Science radar range scales
///       with allocated power once the Engineering/Power Console PRD is
///       implemented.
pub const SCIENCE_RADAR_RANGE: f32 = 200.0;

/// Returns the `RadarConfig` for the Helm console radar.
///
/// Short-range situational awareness: shows asteroids within `HELM_RADAR_RANGE`.
pub fn helm_radar_config() -> RadarConfig {
    RadarConfig {
        range: HELM_RADAR_RANGE,
        shows: vec![EntityTag::Asteroid],
    }
}

/// Returns the `RadarConfig` for the Weapons console radar.
///
/// Extended target-acquisition range: shows asteroids within
/// `WEAPONS_RADAR_RANGE` so the Tactical officer can lock targets
/// that are just beyond the helm's view.
pub fn weapons_radar_config() -> RadarConfig {
    RadarConfig {
        range: WEAPONS_RADAR_RANGE,
        shows: vec![EntityTag::Asteroid],
    }
}

/// Returns the `RadarConfig` for the Science console long-range radar.
///
/// Hardcoded to the maximum-power range (`SCIENCE_RADAR_RANGE`). Shows
/// asteroids, ships, and asteroid fields — the entities relevant for
/// long-range situational awareness.
///
/// TODO: scale `range` by the Science console's allocated power level once
///       the Engineering/Power Console PRD is implemented.
pub fn science_radar_config() -> RadarConfig {
    RadarConfig {
        range: SCIENCE_RADAR_RANGE,
        shows: vec![EntityTag::Asteroid, EntityTag::Ship, EntityTag::AsteroidField],
    }
}

/// Compute the Science console long-range radar view from the current client state.
///
/// Filters to asteroids, ships, and asteroid fields within `SCIENCE_RADAR_RANGE`.
/// Normalises all coordinates to `[-1, 1]` using `science_radar_config().range`.
pub fn compute_science_long_range_radar_view(state: &ClientSimState) -> ScienceRadarView {
    let config = science_radar_config();
    compute_science_radar_view(
        &state.world.asteroids,
        &state.world.asteroid_fields,
        state.ship_x,
        state.ship_z,
        state.ship_yaw,
        &config,
    )
}

/// Compute the Helm console radar view from the current client state.
///
/// Filters to asteroids within `HELM_RADAR_RANGE`.  Normalises all
/// coordinates to `[-1, 1]` using `helm_radar_config().range`.
pub fn compute_helm_radar_view(state: &ClientSimState) -> ScienceRadarView {
    let config = helm_radar_config();
    compute_science_radar_view(
        &state.world.asteroids,
        &state.world.asteroid_fields,
        state.ship_x,
        state.ship_z,
        state.ship_yaw,
        &config,
    )
}

/// Compute the Weapons console radar view from the current client state.
///
/// Filters to asteroids within `WEAPONS_RADAR_RANGE`.  Normalises all
/// coordinates to `[-1, 1]` using `weapons_radar_config().range`.
pub fn compute_weapons_radar_view(state: &ClientSimState) -> ScienceRadarView {
    let config = weapons_radar_config();
    compute_science_radar_view(
        &state.world.asteroids,
        &state.world.asteroid_fields,
        state.ship_x,
        state.ship_z,
        state.ship_yaw,
        &config,
    )
}

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
    /// True when the phaser can fire (target in range/arc, no cooldown).
    /// Updated by `WeaponsUpdate` messages from the server.
    pub fire_ready: bool,
    /// True while the phaser bank is on post-fire cooldown.
    /// Updated by `WeaponsUpdate` messages from the server.
    pub on_cooldown: bool,
    /// Remaining torpedoes in the magazine.
    pub torpedo_count: u32,
    /// Whether the fore-port tube is loaded and ready.
    pub fore_port_loaded: bool,
    /// Seconds until the fore-port tube is ready (0.0 when loaded).
    pub fore_port_reload_secs: f32,
    /// Whether the fore-starboard tube is loaded and ready.
    pub fore_starboard_loaded: bool,
    /// Seconds until the fore-starboard tube is ready (0.0 when loaded).
    pub fore_starboard_reload_secs: f32,
    /// Whether the aft tube is loaded and ready.
    pub aft_loaded: bool,
    /// Seconds until the aft tube is ready (0.0 when loaded).
    pub aft_reload_secs: f32,
    /// In-flight torpedoes: (uuid, x, z, heading, tube).
    /// Updated by `TorpedoLaunched` and `TorpedoDestroyed` messages.
    pub torpedoes_in_flight: Vec<(String, f32, f32, f32, TorpedoTube)>,
    /// Active modifier table: maps `(source, slot)` → bonus value.
    /// Updated by `ModifierAdded` and `ModifierRemoved` messages. Cleared on `Welcome`.
    pub modifiers: HashMap<(ModifierSource, ModifierSlot), f32>,
    /// The shape of the repair icon currently shown on this player's console.
    /// `None` means no icon is shown (no breakdown involving this console and
    /// no decoy). Updated by `ShowRepairIcon` / `ClearRepairIcon` messages.
    pub repair_icon: Option<Shape>,
    /// Latest power allocation levels from SimSnapshot.
    /// Updated every `SimState` broadcast. Tuple is (Helm, Weapons, Science).
    pub power_levels: (u8, u8, u8),
    /// Latest PowerState from the Power console's dedicated 10Hz broadcast.
    pub power_state_payload: Option<(u8, u8, u8, f32, bool)>,
    /// Current state of the three repair teams, updated by `RepairState` messages.
    pub repair_teams: [TeamSlot; 3],
    /// The current breakdown (console + shape) or `None` when queue is empty.
    pub current_breakdown: Option<(Console, Shape)>,
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
            fire_ready: false,
            on_cooldown: false,
            torpedo_count: 10,
            fore_port_loaded: true,
            fore_port_reload_secs: 0.0,
            fore_starboard_loaded: true,
            fore_starboard_reload_secs: 0.0,
            aft_loaded: true,
            aft_reload_secs: 0.0,
            torpedoes_in_flight: Vec::new(),
            modifiers: HashMap::new(),
            repair_icon: None,
            power_levels: (2, 2, 2),
            power_state_payload: None,
            repair_teams: [TeamSlot::Idle, TeamSlot::Idle, TeamSlot::Idle],
            current_breakdown: None,
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
                self.power_levels = snapshot.power_levels;
            }
            ServerMessage::WorldSetup { world } => {
                self.world = world.clone();
            }
            ServerMessage::Welcome { state, .. } => {
                let preserved_world = state.world.clone().unwrap_or_default();
                *self = Self::default();
                self.world = preserved_world;
            }            ServerMessage::RepairState { remaining_cooldown_secs, in_progress, penalty, teams, current_breakdown } => {
                self.repair_cooldown_secs = *remaining_cooldown_secs;
                self.repair_in_progress = *in_progress;
                self.repair_penalty = *penalty;
                self.repair_teams = *teams;
                self.current_breakdown = current_breakdown.clone();
            }
            ServerMessage::PhaserFired { target_uuid, .. } => {
                self.last_phaser_target = Some(target_uuid.clone());
            }
            ServerMessage::WeaponsUpdate {
                fire_ready, on_cooldown,
                torpedo_count,
                fore_port_loaded, fore_port_reload_secs,
                fore_starboard_loaded, fore_starboard_reload_secs,
                aft_loaded, aft_reload_secs,
                ..
            } => {
                self.fire_ready = *fire_ready;
                self.on_cooldown = *on_cooldown;
                self.torpedo_count = *torpedo_count;
                self.fore_port_loaded = *fore_port_loaded;
                self.fore_port_reload_secs = *fore_port_reload_secs;
                self.fore_starboard_loaded = *fore_starboard_loaded;
                self.fore_starboard_reload_secs = *fore_starboard_reload_secs;
                self.aft_loaded = *aft_loaded;
                self.aft_reload_secs = *aft_reload_secs;
            }
            ServerMessage::ScienceTargetSuggestion { uuid } => {
                self.science_target_suggestion = Some(uuid.clone());
            }
            ServerMessage::ShieldStatus { facings } => {
                self.shield_facings = facings.clone();
            }
            ServerMessage::TorpedoLaunched { uuid, tube, x, z, heading } => {
                self.torpedoes_in_flight.push((uuid.clone(), *x, *z, *heading, *tube));
            }
            ServerMessage::TorpedoDestroyed { uuid } => {
                self.torpedoes_in_flight.retain(|(id, ..)| id != uuid);
            }
            ServerMessage::ModifierAdded { source, slot, bonus } => {
                self.modifiers.insert((source.clone(), slot.clone()), *bonus);
            }
            ServerMessage::ModifierRemoved { source, slot } => {
                self.modifiers.remove(&(source.clone(), slot.clone()));
            }
            ServerMessage::ShowRepairIcon { shape } => {
                self.repair_icon = Some(*shape);
            }
            ServerMessage::ClearRepairIcon => {
                self.repair_icon = None;
            }
            ServerMessage::PowerState { helm, weapons, science, battery_charge, locked } => {
                self.power_state_payload = Some((*helm, *weapons, *science, *battery_charge, *locked));
            }
            _ => {}
        }
    }

    /// True iff the captain's view direction selector should highlight
    /// the given direction. Radar mode highlights nothing in the cross.
    pub fn is_active_camera_direction(&self, direction: &ViewDirection) -> bool {
        matches!(&self.view_mode, ViewMode::Camera(d) if d == direction)
    }

    /// Returns the bonus value for the given `(source, slot)` pair, if present.
    pub fn modifier_bonus(&self, source: &ModifierSource, slot: &ModifierSlot) -> Option<f32> {
        self.modifiers.get(&(source.clone(), slot.clone())).copied()
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
    ClientMessage::Repair { shape: Shape::Triangle }
}

/// `ClientMessage` to fire a torpedo from the given tube with an optional homing target.
pub fn fire_torpedo_message(tube: TorpedoTube, target_uuid: Option<String>) -> ClientMessage {
    ClientMessage::FireTorpedo { tube, target_uuid }
}

/// `ClientMessage` to fire the phaser.
pub fn fire_phaser_message() -> ClientMessage {
    ClientMessage::FirePhaser
}

/// `ClientMessage` to set the phaser mode (Auto or Manual).
pub fn set_phaser_mode_message(mode: crate::messages::PhaserMode) -> ClientMessage {
    ClientMessage::SetPhaserMode { mode }
}

/// `ClientMessage` to toggle the phaser mode: Auto → Manual, Manual → Auto.
pub fn toggle_phaser_mode_message(current: PhaserMode) -> ClientMessage {
    let next = match current {
        PhaserMode::Auto => PhaserMode::Manual,
        PhaserMode::Manual => PhaserMode::Auto,
    };
    ClientMessage::SetPhaserMode { mode: next }
}

/// Returns `true` when the Fire Phaser button should be enabled.
///
/// The button is active only when the server has reported that the target is
/// in range/arc (`fire_ready`) AND the phaser bank is not on post-fire
/// cooldown (`on_cooldown`).
pub fn is_fire_button_enabled(state: &ClientSimState) -> bool {
    state.fire_ready && !state.on_cooldown
}

/// Human-readable label for the current phaser mode toggle button.
pub fn phaser_mode_label(mode: PhaserMode) -> &'static str {
    match mode {
        PhaserMode::Auto => "AUTO",
        PhaserMode::Manual => "MANUAL",
    }
}

/// `ClientMessage` to send when the Science officer taps an entity on their
/// long-range radar to suggest it as a target to the Weapons console.
pub fn set_science_target_message(uuid: String) -> ClientMessage {
    ClientMessage::SetScienceTarget { uuid }
}

/// Returns `true` when the Science Console's "Cancel Impulse" button should
/// be visible — i.e. when the impulse drive is charging or active.
pub fn cancel_impulse_button_visible(state: &crate::impulse::ImpulseState) -> bool {
    state.is_active() || state.phase == crate::impulse::ImpulsePhase::Charging
}

/// Called when the Science officer presses the "Cancel Impulse" button.
/// Returns `Some(CancelImpulse)` when the drive is charging or active,
/// `None` when idle (button should not be visible in that state).
pub fn press_cancel_impulse_button(state: &crate::impulse::ImpulseState) -> Option<ClientMessage> {
    if cancel_impulse_button_visible(state) {
        Some(ClientMessage::CancelImpulse)
    } else {
        None
    }
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
        SimSnapshot { red_alert, view_mode, ship_x: 0.0, ship_z: 0.0, ship_yaw: 0.0, hull_integrity: 100, power_levels: (2, 2, 2), flags: vec![] }
    }

    fn snap_pose(x: f32, z: f32, yaw: f32) -> SimSnapshot {
        SimSnapshot {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x: x,
            ship_z: z,
            ship_yaw: yaw,
            hull_integrity: 100,
            power_levels: (2, 2, 2),
            flags: vec![],
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
            fire_ready: false,
            on_cooldown: false,
            torpedo_count: 10,
            fore_port_loaded: true,
            fore_port_reload_secs: 0.0,
            fore_starboard_loaded: true,
            fore_starboard_reload_secs: 0.0,
            aft_loaded: true,
            aft_reload_secs: 0.0,
            torpedoes_in_flight: Vec::new(),
            modifiers: HashMap::new(),
            repair_icon: None,
            power_levels: (2, 2, 2),
            power_state_payload: None,
            repair_teams: [TeamSlot::Idle, TeamSlot::Idle, TeamSlot::Idle],
            current_breakdown: None,
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
            ship_stations: crate::stations::ShipStations::default(),
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
            fire_ready: false,
            on_cooldown: false,
            torpedo_count: 10,
            fore_port_loaded: true,
            fore_port_reload_secs: 0.0,
            fore_starboard_loaded: true,
            fore_starboard_reload_secs: 0.0,
            aft_loaded: true,
            aft_reload_secs: 0.0,
            torpedoes_in_flight: Vec::new(),
            modifiers: HashMap::new(),
            repair_icon: None,
            power_levels: (2, 2, 2),
            power_state_payload: None,
            repair_teams: [TeamSlot::Idle, TeamSlot::Idle, TeamSlot::Idle],
            current_breakdown: None,
        };
        s.apply(&ServerMessage::Welcome {
            state: GameState {
                phase: GamePhase::Lobby,
                players: vec![],
                world: None,
            },
            ship_stations: crate::stations::ShipStations::default(),
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
            fire_ready: false,
            on_cooldown: false,
            torpedo_count: 10,
            fore_port_loaded: true,
            fore_port_reload_secs: 0.0,
            fore_starboard_loaded: true,
            fore_starboard_reload_secs: 0.0,
            aft_loaded: true,
            aft_reload_secs: 0.0,
            torpedoes_in_flight: Vec::new(),
            modifiers: HashMap::new(),
            repair_icon: None,
            power_levels: (2, 2, 2),
            power_state_payload: None,
            repair_teams: [TeamSlot::Idle, TeamSlot::Idle, TeamSlot::Idle],
            current_breakdown: None,
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
            ship_stations: crate::stations::ShipStations::default(),
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
                },
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
            ship_stations: crate::stations::ShipStations::default(),
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

    // ── Science cancel-impulse button ────────────────────────────────────

    #[test]
    fn cancel_impulse_button_hidden_when_impulse_idle() {
        use crate::impulse::ImpulseState;
        let s = ImpulseState::new();
        assert!(!cancel_impulse_button_visible(&s));
    }

    #[test]
    fn cancel_impulse_button_visible_when_charging() {
        use crate::impulse::{ImpulseState, IMPULSE_CHARGE_DURATION};
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION / 2.0);
        assert!(cancel_impulse_button_visible(&s));
    }

    #[test]
    fn cancel_impulse_button_visible_when_active() {
        use crate::impulse::{ImpulseState, IMPULSE_CHARGE_DURATION};
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION);
        assert!(cancel_impulse_button_visible(&s));
    }

    #[test]
    fn press_cancel_impulse_sends_message_when_visible() {
        use crate::impulse::{ImpulseState, IMPULSE_CHARGE_DURATION};
        let mut s = ImpulseState::new();
        s.start_charge();
        s.tick(IMPULSE_CHARGE_DURATION / 2.0);
        assert_eq!(press_cancel_impulse_button(&s), Some(ClientMessage::CancelImpulse));
    }

    #[test]
    fn press_cancel_impulse_noop_when_idle() {
        use crate::impulse::ImpulseState;
        let s = ImpulseState::new();
        assert_eq!(press_cancel_impulse_button(&s), None);
    }

    // ── helm_radar_config ────────────────────────────────────────────────

    #[test]
    fn helm_radar_config_range_matches_constant() {
        let cfg = helm_radar_config();
        assert_eq!(cfg.range, HELM_RADAR_RANGE);
    }

    #[test]
    fn helm_radar_config_shows_asteroids() {
        use crate::entity_tags::EntityTag;
        let cfg = helm_radar_config();
        assert!(cfg.shows.contains(&EntityTag::Asteroid), "helm radar must show asteroids");
    }

    // ── weapons_radar_config ─────────────────────────────────────────────

    #[test]
    fn weapons_radar_config_range_matches_constant() {
        let cfg = weapons_radar_config();
        assert_eq!(cfg.range, WEAPONS_RADAR_RANGE);
    }

    #[test]
    fn weapons_radar_config_has_longer_range_than_helm() {
        assert!(
            WEAPONS_RADAR_RANGE > HELM_RADAR_RANGE,
            "weapons radar ({}) must have longer range than helm ({})",
            WEAPONS_RADAR_RANGE,
            HELM_RADAR_RANGE,
        );
    }

    #[test]
    fn weapons_radar_config_shows_asteroids() {
        use crate::entity_tags::EntityTag;
        let cfg = weapons_radar_config();
        assert!(cfg.shows.contains(&EntityTag::Asteroid), "weapons radar must show asteroids");
    }

    // ── compute_helm_radar_view ──────────────────────────────────────────

    #[test]
    fn helm_radar_view_empty_world_produces_empty_view() {
        let s = ClientSimState::default();
        let view = compute_helm_radar_view(&s);
        assert!(view.dots.is_empty());
        assert!(view.rings.is_empty());
    }

    #[test]
    fn helm_radar_view_includes_asteroid_within_helm_range() {
        use crate::messages::{AsteroidInfo, WorldData};
        let mut s = ClientSimState::default();
        s.world = WorldData {
            asteroids: vec![AsteroidInfo {
                uuid: "a1".into(),
                x: 0.0,
                z: -(HELM_RADAR_RANGE - 5.0),
                radius: 1.0,
                tags: vec!["asteroid".into()],
            }],
            asteroid_fields: vec![],
        };
        let view = compute_helm_radar_view(&s);
        assert_eq!(view.dots.len(), 1);
        assert_eq!(view.dots[0].uuid, "a1");
    }

    #[test]
    fn helm_radar_view_excludes_asteroid_beyond_helm_range() {
        use crate::messages::{AsteroidInfo, WorldData};
        let mut s = ClientSimState::default();
        // Place asteroid between helm range and weapons range.
        let beyond_helm = HELM_RADAR_RANGE + 5.0;
        s.world = WorldData {
            asteroids: vec![AsteroidInfo {
                uuid: "far".into(),
                x: 0.0,
                z: -beyond_helm,
                radius: 1.0,
                tags: vec!["asteroid".into()],
            }],
            asteroid_fields: vec![],
        };
        let view = compute_helm_radar_view(&s);
        assert!(view.dots.is_empty(), "asteroid beyond helm range must not appear");
    }

    #[test]
    fn helm_radar_dot_position_normalised_to_helm_range() {
        use crate::messages::{AsteroidInfo, WorldData};
        let mut s = ClientSimState::default();
        s.world = WorldData {
            asteroids: vec![AsteroidInfo {
                uuid: "at-edge".into(),
                x: 0.0,
                z: -HELM_RADAR_RANGE,
                radius: 1.0,
                tags: vec!["asteroid".into()],
            }],
            asteroid_fields: vec![],
        };
        let view = compute_helm_radar_view(&s);
        assert_eq!(view.dots.len(), 1);
        // Asteroid directly ahead at exactly helm range → radar_y = 1.0.
        let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
        close(view.dots[0].radar_y, 1.0);
    }

    // ── compute_weapons_radar_view ───────────────────────────────────────

    #[test]
    fn weapons_radar_view_empty_world_produces_empty_view() {
        let s = ClientSimState::default();
        let view = compute_weapons_radar_view(&s);
        assert!(view.dots.is_empty());
        assert!(view.rings.is_empty());
    }

    #[test]
    fn weapons_radar_view_includes_asteroid_within_weapons_range() {
        use crate::messages::{AsteroidInfo, WorldData};
        let mut s = ClientSimState::default();
        // Between helm range and weapons range — weapons only.
        let between = (HELM_RADAR_RANGE + WEAPONS_RADAR_RANGE) / 2.0;
        s.world = WorldData {
            asteroids: vec![AsteroidInfo {
                uuid: "mid".into(),
                x: 0.0,
                z: -between,
                radius: 1.0,
                tags: vec!["asteroid".into()],
            }],
            asteroid_fields: vec![],
        };
        let view = compute_weapons_radar_view(&s);
        assert_eq!(view.dots.len(), 1, "weapons radar should see asteroid between the two ranges");
        assert_eq!(view.dots[0].uuid, "mid");
    }

    #[test]
    fn weapons_radar_view_excludes_asteroid_beyond_weapons_range() {
        use crate::messages::{AsteroidInfo, WorldData};
        let mut s = ClientSimState::default();
        let beyond = WEAPONS_RADAR_RANGE + 5.0;
        s.world = WorldData {
            asteroids: vec![AsteroidInfo {
                uuid: "very-far".into(),
                x: 0.0,
                z: -beyond,
                radius: 1.0,
                tags: vec!["asteroid".into()],
            }],
            asteroid_fields: vec![],
        };
        let view = compute_weapons_radar_view(&s);
        assert!(view.dots.is_empty(), "asteroid beyond weapons range must not appear");
    }

    #[test]
    fn weapons_radar_dot_position_normalised_to_weapons_range() {
        use crate::messages::{AsteroidInfo, WorldData};
        let mut s = ClientSimState::default();
        s.world = WorldData {
            asteroids: vec![AsteroidInfo {
                uuid: "at-edge".into(),
                x: 0.0,
                z: -WEAPONS_RADAR_RANGE,
                radius: 1.0,
                tags: vec!["asteroid".into()],
            }],
            asteroid_fields: vec![],
        };
        let view = compute_weapons_radar_view(&s);
        assert_eq!(view.dots.len(), 1);
        let close = |a: f32, b: f32| assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
        close(view.dots[0].radar_y, 1.0);
    }

    #[test]
    fn helm_and_weapons_views_differ_for_asteroid_in_weapons_range_only() {
        use crate::messages::{AsteroidInfo, WorldData};
        let mut s = ClientSimState::default();
        // Asteroid between helm and weapons range.
        let between = (HELM_RADAR_RANGE + WEAPONS_RADAR_RANGE) / 2.0;
        s.world = WorldData {
            asteroids: vec![AsteroidInfo {
                uuid: "between".into(),
                x: 0.0,
                z: -between,
                radius: 1.0,
                tags: vec!["asteroid".into()],
            }],
            asteroid_fields: vec![],
        };
        let helm_view = compute_helm_radar_view(&s);
        let weapons_view = compute_weapons_radar_view(&s);
        assert!(helm_view.dots.is_empty(), "helm should NOT see asteroid beyond its range");
        assert_eq!(weapons_view.dots.len(), 1, "weapons SHOULD see asteroid within its range");
    }

    // ── science_radar_config ─────────────────────────────────────────────

    #[test]
    fn science_radar_config_range_matches_constant() {
        let cfg = science_radar_config();
        assert_eq!(cfg.range, SCIENCE_RADAR_RANGE);
    }

    #[test]
    fn science_radar_config_range_is_greater_than_weapons_range() {
        assert!(
            SCIENCE_RADAR_RANGE > WEAPONS_RADAR_RANGE,
            "science radar ({}) must have longer range than weapons ({})",
            SCIENCE_RADAR_RANGE,
            WEAPONS_RADAR_RANGE,
        );
    }

    #[test]
    fn science_radar_config_shows_asteroids_and_ships() {
        use crate::entity_tags::EntityTag;
        let cfg = science_radar_config();
        assert!(cfg.shows.contains(&EntityTag::Asteroid), "science radar must show asteroids");
        assert!(cfg.shows.contains(&EntityTag::Ship), "science radar must show ships");
    }

    // ── compute_science_long_range_radar_view ────────────────────────────

    #[test]
    fn science_long_range_radar_view_empty_world_produces_empty_view() {
        let s = ClientSimState::default();
        let view = compute_science_long_range_radar_view(&s);
        assert!(view.dots.is_empty());
        assert!(view.rings.is_empty());
    }

    #[test]
    fn science_long_range_radar_view_includes_asteroid_within_science_range() {
        use crate::messages::{AsteroidInfo, WorldData};
        let mut s = ClientSimState::default();
        s.world = WorldData {
            asteroids: vec![AsteroidInfo {
                uuid: "a1".into(),
                x: 0.0,
                z: -(SCIENCE_RADAR_RANGE * 0.5),
                radius: 1.0,
                tags: vec!["asteroid".into()],
            }],
            asteroid_fields: vec![],
        };
        let view = compute_science_long_range_radar_view(&s);
        assert_eq!(view.dots.len(), 1, "science radar must show asteroid within range");
    }

    // ── weapons_update ───────────────────────────────────────────────────

    #[test]
    fn weapons_update_sets_fire_ready_and_cooldown() {
        let mut s = ClientSimState::default();
        assert!(!s.fire_ready, "default: fire not ready");
        assert!(!s.on_cooldown, "default: not on cooldown");

        s.apply(&ServerMessage::WeaponsUpdate {
            target_uuid: None,
            fire_ready: true,
            on_cooldown: false,
            torpedo_count: 10,
            fore_port_loaded: true, fore_port_reload_secs: 0.0,
            fore_starboard_loaded: true, fore_starboard_reload_secs: 0.0,
            aft_loaded: true, aft_reload_secs: 0.0,
        });
        assert!(s.fire_ready);
        assert!(!s.on_cooldown);

        s.apply(&ServerMessage::WeaponsUpdate {
            target_uuid: None,
            fire_ready: false,
            on_cooldown: true,
            torpedo_count: 10,
            fore_port_loaded: true, fore_port_reload_secs: 0.0,
            fore_starboard_loaded: true, fore_starboard_reload_secs: 0.0,
            aft_loaded: true, aft_reload_secs: 0.0,
        });
        assert!(!s.fire_ready);
        assert!(s.on_cooldown);
    }

    #[test]
    fn fire_phaser_message_builder_produces_correct_message() {
        let msg = fire_phaser_message();
        assert_eq!(msg, ClientMessage::FirePhaser);
    }

    #[test]
    fn set_phaser_mode_message_builder_produces_correct_message() {
        assert_eq!(
            set_phaser_mode_message(PhaserMode::Manual),
            ClientMessage::SetPhaserMode { mode: PhaserMode::Manual },
        );
        assert_eq!(
            set_phaser_mode_message(PhaserMode::Auto),
            ClientMessage::SetPhaserMode { mode: PhaserMode::Auto },
        );
    }

    #[test]
    fn toggle_phaser_mode_auto_produces_set_manual() {
        let msg = toggle_phaser_mode_message(PhaserMode::Auto);
        assert_eq!(msg, ClientMessage::SetPhaserMode { mode: PhaserMode::Manual });
    }

    #[test]
    fn toggle_phaser_mode_manual_produces_set_auto() {
        let msg = toggle_phaser_mode_message(PhaserMode::Manual);
        assert_eq!(msg, ClientMessage::SetPhaserMode { mode: PhaserMode::Auto });
    }

    #[test]
    fn fire_button_disabled_when_not_fire_ready() {
        let s = ClientSimState::default();
        assert!(!is_fire_button_enabled(&s), "fire button should be disabled when fire_ready is false");
    }

    #[test]
    fn fire_button_enabled_when_fire_ready_and_not_on_cooldown() {
        let mut s = ClientSimState::default();
        s.fire_ready = true;
        s.on_cooldown = false;
        assert!(is_fire_button_enabled(&s), "fire button should be enabled when fire_ready and not on cooldown");
    }

    #[test]
    fn fire_button_disabled_when_on_cooldown_even_if_fire_ready() {
        let mut s = ClientSimState::default();
        s.fire_ready = true;
        s.on_cooldown = true;
        assert!(!is_fire_button_enabled(&s), "fire button should be disabled when on cooldown");
    }

    #[test]
    fn phaser_mode_label_auto() {
        assert_eq!(phaser_mode_label(PhaserMode::Auto), "AUTO");
    }

    #[test]
    fn phaser_mode_label_manual() {
        assert_eq!(phaser_mode_label(PhaserMode::Manual), "MANUAL");
    }

    #[test]
    fn weapons_update_torpedo_fields_update_tube_status() {
        let mut s = ClientSimState::default();
        // Default: all tubes loaded, full count.
        assert_eq!(s.torpedo_count, 10);
        assert!(s.fore_port_loaded, "fore port should start loaded");
        assert!(s.fore_starboard_loaded, "fore starboard should start loaded");
        assert!(s.aft_loaded, "aft should start loaded");
        assert_eq!(s.fore_port_reload_secs, 0.0);
        assert_eq!(s.fore_starboard_reload_secs, 0.0);
        assert_eq!(s.aft_reload_secs, 0.0);

        s.apply(&ServerMessage::WeaponsUpdate {
            target_uuid: None,
            fire_ready: false,
            on_cooldown: false,
            torpedo_count: 8,
            fore_port_loaded: false,
            fore_port_reload_secs: 7.5,
            fore_starboard_loaded: true,
            fore_starboard_reload_secs: 0.0,
            aft_loaded: false,
            aft_reload_secs: 3.2,
        });

        assert_eq!(s.torpedo_count, 8);
        assert!(!s.fore_port_loaded);
        assert_eq!(s.fore_port_reload_secs, 7.5);
        assert!(s.fore_starboard_loaded);
        assert_eq!(s.fore_starboard_reload_secs, 0.0);
        assert!(!s.aft_loaded);
        assert_eq!(s.aft_reload_secs, 3.2);
    }

    #[test]
    fn fire_torpedo_message_builder_produces_correct_message() {
        let msg = fire_torpedo_message(TorpedoTube::ForePort, Some("target-uuid".into()));
        assert_eq!(msg, ClientMessage::FireTorpedo {
            tube: TorpedoTube::ForePort,
            target_uuid: Some("target-uuid".into()),
        });
        let msg2 = fire_torpedo_message(TorpedoTube::Aft, None);
        assert_eq!(msg2, ClientMessage::FireTorpedo {
            tube: TorpedoTube::Aft,
            target_uuid: None,
        });
    }

    #[test]
    fn torpedo_launched_adds_to_in_flight() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::TorpedoLaunched {
            uuid: "t1".into(),
            tube: TorpedoTube::ForePort,
            x: 10.0,
            z: -5.0,
            heading: 0.0,
        });
        assert_eq!(s.torpedoes_in_flight.len(), 1);
        let (uuid, x, z, heading, tube) = &s.torpedoes_in_flight[0];
        assert_eq!(uuid, "t1");
        assert_eq!(*x, 10.0);
        assert_eq!(*z, -5.0);
        assert_eq!(*heading, 0.0);
        assert_eq!(*tube, TorpedoTube::ForePort);
    }

    #[test]
    fn torpedo_destroyed_removes_from_in_flight() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::TorpedoLaunched {
            uuid: "t1".into(),
            tube: TorpedoTube::Aft,
            x: 0.0,
            z: 0.0,
            heading: 0.0,
        });
        s.apply(&ServerMessage::TorpedoLaunched {
            uuid: "t2".into(),
            tube: TorpedoTube::ForeStarboard,
            x: 0.0,
            z: 0.0,
            heading: 0.0,
        });
        s.apply(&ServerMessage::TorpedoDestroyed { uuid: "t1".into() });
        assert_eq!(s.torpedoes_in_flight.len(), 1);
        assert_eq!(s.torpedoes_in_flight[0].0, "t2");
    }

    // ── modifier table in ClientSimState ────────────────────────────────

    #[test]
    fn modifier_added_is_stored_in_client_state() {
        use crate::messages::{ModifierSlot, ModifierSource};
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::ModifierAdded {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::MaxSpeed,
            bonus: 0.5,
        });
        let bonus = s.modifier_bonus(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
        assert!((bonus.unwrap() - 0.5).abs() < 1e-6, "expected bonus 0.5");
    }

    #[test]
    fn modifier_added_replaces_existing_same_source_slot() {
        use crate::messages::{ModifierSlot, ModifierSource};
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::ModifierAdded {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::MaxSpeed,
            bonus: 0.5,
        });
        s.apply(&ServerMessage::ModifierAdded {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::MaxSpeed,
            bonus: 0.9,
        });
        let bonus = s.modifier_bonus(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed);
        assert!((bonus.unwrap() - 0.9).abs() < 1e-6, "expected updated bonus 0.9");
        // Only one entry for this source+slot.
        assert_eq!(
            s.modifiers.values()
                .filter(|&&b| (b - 0.9_f32).abs() < 1e-6)
                .count(),
            1
        );
    }

    #[test]
    fn modifier_removed_clears_entry() {
        use crate::messages::{ModifierSlot, ModifierSource};
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::ModifierAdded {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::MaxSpeed,
            bonus: 0.5,
        });
        s.apply(&ServerMessage::ModifierRemoved {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::MaxSpeed,
        });
        assert!(s.modifier_bonus(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed).is_none());
    }

    #[test]
    fn modifier_removed_unknown_is_noop() {
        use crate::messages::{ModifierSlot, ModifierSource};
        let mut s = ClientSimState::default();
        // Should not panic or corrupt state.
        s.apply(&ServerMessage::ModifierRemoved {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::MaxSpeed,
        });
        assert!(s.modifier_bonus(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed).is_none());
    }

    #[test]
    fn welcome_clears_modifiers() {
        use crate::messages::{GamePhase, GameState, ModifierSlot, ModifierSource};
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::ModifierAdded {
            source: ModifierSource::ImpulseDrive,
            slot: ModifierSlot::MaxSpeed,
            bonus: 1.0,
        });
        s.apply(&ServerMessage::Welcome {
            state: GameState { phase: GamePhase::Lobby, players: vec![], world: None },
            ship_stations: crate::stations::ShipStations::default(),
        });
        assert!(
            s.modifier_bonus(&ModifierSource::ImpulseDrive, &ModifierSlot::MaxSpeed).is_none(),
            "Welcome must clear modifier table"
        );
    }

    // ── Power state in ClientSimState ────────────────────────────────

    #[test]
    fn sim_state_updates_power_levels_from_snapshot() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::SimState {
            snapshot: SimSnapshot {
                red_alert: false,
                view_mode: ViewMode::default(),
                ship_x: 0.0, ship_z: 0.0, ship_yaw: 0.0,
                hull_integrity: 100,
                power_levels: (4, 1, 3),
                flags: vec![],
            },
        });
        assert_eq!(s.power_levels, (4, 1, 3));
    }

    #[test]
    fn power_state_message_sets_power_state_payload() {
        let mut s = ClientSimState::default();
        assert!(s.power_state_payload.is_none(), "default: no power state payload");
        s.apply(&ServerMessage::PowerState {
            helm: 3,
            weapons: 2,
            science: 4,
            battery_charge: 75.0,
            locked: false,
        });
        assert_eq!(s.power_state_payload, Some((3, 2, 4, 75.0, false)));
    }

    #[test]
    fn second_power_state_message_overwrites_previous() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::PowerState {
            helm: 1, weapons: 1, science: 1,
            battery_charge: 10.0, locked: true,
        });
        s.apply(&ServerMessage::PowerState {
            helm: 4, weapons: 2, science: 2,
            battery_charge: 100.0, locked: false,
        });
        assert_eq!(s.power_state_payload, Some((4, 2, 2, 100.0, false)));
    }

    #[test]
    fn welcome_resets_power_state_fields() {
        let mut s = ClientSimState {
            red_alert: false,
            view_mode: ViewMode::default(),
            ship_x: 0.0, ship_z: 0.0, ship_yaw: 0.0,
            world: WorldData::default(),
            repair_cooldown_secs: 0.0,
            repair_in_progress: false,
            repair_penalty: false,
            phaser_mode: PhaserMode::Auto,
            last_phaser_target: None,
            science_target_suggestion: None,
            shield_facings: Vec::new(),
            fire_ready: false,
            on_cooldown: false,
            torpedo_count: 10,
            fore_port_loaded: true,
            fore_port_reload_secs: 0.0,
            fore_starboard_loaded: true,
            fore_starboard_reload_secs: 0.0,
            aft_loaded: true,
            aft_reload_secs: 0.0,
            torpedoes_in_flight: Vec::new(),
            modifiers: HashMap::new(),
            repair_icon: None,
            power_levels: (4, 1, 3),
            power_state_payload: Some((4, 2, 2, 100.0, false)),
            repair_teams: [TeamSlot::Idle, TeamSlot::Idle, TeamSlot::Idle],
            current_breakdown: None,
        };
        s.apply(&ServerMessage::Welcome {
            state: GameState { phase: GamePhase::Lobby, players: vec![], world: None },
            ship_stations: crate::stations::ShipStations::default(),
        });
        assert_eq!(s.power_levels, (2, 2, 2), "power_levels reset to default on Welcome");
        assert_eq!(s.power_state_payload, None, "power_state_payload cleared on Welcome");
    }

    // ── RepairState teams + current_breakdown ──────────────────────────

    #[test]
    fn repair_state_updates_teams_and_current_breakdown() {
        let mut s = ClientSimState::default();
        assert_eq!(s.repair_teams, [TeamSlot::Idle, TeamSlot::Idle, TeamSlot::Idle]);
        assert_eq!(s.current_breakdown, None);

        s.apply(&ServerMessage::RepairState {
            remaining_cooldown_secs: 0.0,
            in_progress: true,
            penalty: false,
            teams: [TeamSlot::Repairing { progress: 0.4 }, TeamSlot::Idle, TeamSlot::Cooldown { progress: 0.2 }],
            current_breakdown: Some((Console::Helm, Shape::Square)),
        });

        assert_eq!(s.repair_teams[0], TeamSlot::Repairing { progress: 0.4 });
        assert!(matches!(s.repair_teams[1], TeamSlot::Idle));
        assert_eq!(s.current_breakdown, Some((Console::Helm, Shape::Square)));
    }

    #[test]
    fn repair_state_clears_current_breakdown_when_queue_empty() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::RepairState {
            remaining_cooldown_secs: 0.0,
            in_progress: false,
            penalty: false,
            teams: [TeamSlot::Idle, TeamSlot::Idle, TeamSlot::Idle],
            current_breakdown: None,
        });
        assert_eq!(s.current_breakdown, None);
        assert!(s.repair_teams.iter().all(|t| matches!(t, TeamSlot::Idle)));
    }

    #[test]
    fn welcome_resets_repair_teams_and_current_breakdown() {
        let mut s = ClientSimState::default();
        s.apply(&ServerMessage::RepairState {
            remaining_cooldown_secs: 5.0,
            in_progress: true,
            penalty: false,
            teams: [TeamSlot::Repairing { progress: 0.5 }, TeamSlot::Cooldown { progress: 0.3 }, TeamSlot::Idle],
            current_breakdown: Some((Console::Tactical, Shape::Circle)),
        });
        s.apply(&ServerMessage::Welcome {
            state: GameState { phase: GamePhase::Lobby, players: vec![], world: None },
            ship_stations: crate::stations::ShipStations::default(),
        });
        assert_eq!(s.repair_teams, [TeamSlot::Idle, TeamSlot::Idle, TeamSlot::Idle]);
        assert_eq!(s.current_breakdown, None);
    }
}
