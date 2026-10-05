use bevy::prelude::*;
/// The currently selected science target on the Sensors console. `None` means
/// no target is selected. Broadcast to all clients via SensorsBlackboard so
/// every radar can render a blue science-target marker.
///
/// Per-entity `Component` on every ship (player + NPC). PR-7 (issue #597)
/// removed the dual `Resource` derive — every ship has its own sensors target.
#[derive(Component, Default, Clone, Debug)]
pub struct SensorRadarSelection(pub Option<String>);

/// Tracks the last frequency value sent for a given target so we avoid
/// re-emitting when nothing has changed.
///
/// Per-ship `Component` so NPC ships track their own Sensors→Tactical
/// frequency hints independently of the player's.
#[derive(Component, Default, Clone)]
pub struct SensorsFrequencyState {
    pub last_sent_target: Option<String>,
    pub last_sent_frequency: Option<f32>,
}

/// Tracks the last threat warning emitted per ship to debounce against
/// bus spam (issue #683). Sensors emits a `ThreatBearing` coordination
/// message to Shields only when a *new* threat appears or an existing
/// threat's bearing changes by more than the configured epsilon.
#[derive(Component, Default, Clone)]
pub struct SensorsThreatState {
    pub last_threat_uuid: Option<String>,
    pub last_bearing_rad: Option<f32>,
    pub last_label: Option<String>,
    pub last_distance: Option<f32>,
}

/// TOML-loaded configuration for the Sensors AI controller
/// (`console_ai::server::tick_frequency_hint_high_fidelity`, issue #692).
///
/// Loaded from `[sensors_console.ai]` in the ship entity TOML. Defaults are
/// used when the section is absent.
///
/// Dual `Resource + Component`, mirroring `ShieldsAiConfigResource` — but the
/// Resource half is **structural symmetry only, and has never been seeded**.
/// `ShipSensorsPlugin::build` registers it with `init_resource` and nothing
/// anywhere writes it (there is no sensors equivalent of the shields dual-write
/// in `server_app::spawn_game_start_entities`), so it has only ever held
/// `Self::default()`. Every read goes through the per-entity Component, which
/// the spawner and `spawn_game_start_entities` both attach; see
/// `console_ai::server::tick_frequency_hint_high_fidelity`. Do not reintroduce a `Res<_>` read
/// here: it applies one ship's tuning to every ship.
#[derive(Resource, Component, Clone, Debug)]
pub struct SensorsAiConfigResource {
    /// Delay (seconds) between a target lock and the AI-driven Sensors
    /// operator emitting a `FrequencyHint` coordination message to Tactical.
    pub frequency_hint_delay_secs: f32,
}

impl Default for SensorsAiConfigResource {
    fn default() -> Self {
        Self {
            frequency_hint_delay_secs: 3.0,
        }
    }
}
