//! Entity schema: helm. Public paths remain in the parent module.
use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmConsoleConfig {
    #[serde(default)]
    pub max_speed: f32,
    #[serde(default)]
    pub max_reverse_speed: f32,
    #[serde(default)]
    pub acceleration: f32,
    #[serde(default)]
    pub deceleration: f32,
    #[serde(default)]
    pub max_yaw_rate: f32,
    /// Extra turn authority for flying slow, as a fraction added at a dead stop
    /// and lerped away to nothing at `max_speed`. `0.5` means a stationary hull
    /// turns 50% faster than it does at full throttle; `0.0` (the default) is
    /// the old speed-independent turn rate.
    ///
    /// This is the throttle-vs-turn trade that keeps evenly-matched hulls from
    /// deadlocking in a co-rotating circle. Authored per class: light hulls get
    /// the most, capital hulls none.
    #[serde(default)]
    pub low_speed_turn_boost: f32,
    /// Radar configuration for the Helm radar widget, from
    /// `[helm_console.radar]`.
    #[serde(default)]
    pub radar: Option<crate::radar_config::RadarConfig>,
    /// RGBA colour the helm radar uses for the red-alert hostile weapon-arc
    /// overlay (issue #874). Four floats in 0.0–1.0; the fourth is the fill
    /// opacity, and "faint" is the whole point of the overlay. When absent (or
    /// not exactly four entries) the `ShipClientConfig` default applies.
    #[serde(default)]
    pub hostile_arc_color: Vec<f32>,
    #[serde(default)]
    pub power_multipliers: Option<[f32; 4]>,
    /// Total time in seconds to fully charge the impulse drive.
    /// Defaults to `IMPULSE_CHARGE_DURATION` (3.0 s) when absent.
    #[serde(default = "default_impulse_charge_duration")]
    pub impulse_charge_duration: f32,
    /// Speed multiplier applied when impulse drive is active.
    /// Defaults to `IMPULSE_SPEED_MULTIPLIER` (10.0) when absent.
    #[serde(default = "default_impulse_speed_multiplier")]
    pub impulse_speed_multiplier: f32,
    /// Acceleration multiplier applied while impulse drive is active.
    /// Defaults to `IMPULSE_ACCELERATION_MULTIPLIER` (5.0) when absent.
    #[serde(default = "default_impulse_acceleration_multiplier")]
    pub impulse_acceleration_multiplier: f32,
    /// Minimum distance from target at which AI may engage impulse (world units).
    /// Defaults to 200.0 when absent.
    #[serde(default = "default_impulse_engage_distance")]
    pub impulse_engage_distance: f32,
    /// Distance from target at which AI cancels impulse (world units).
    /// Defaults to 40.0 when absent.
    #[serde(default = "default_impulse_cancel_distance")]
    pub impulse_cancel_distance: f32,
    /// Maximum visual banking (roll) angle in degrees when steering at full
    /// deflection. The ship leans into turns, lerped from 0 toward ±max_bank_deg
    /// based on steering input percentage. 0 = no banking.
    #[serde(default)]
    pub max_bank_deg: f32,
    /// How quickly the ship's visual roll lerps toward the target bank angle
    /// (units: per-second lerp rate). Defaults to
    /// [`crate::ship_plugin::BANK_LERP_RATE`] when absent.
    #[serde(default = "default_bank_lerp_rate")]
    pub bank_lerp_rate: f32,
    /// Optional boost drive config, from `[helm_console.boost]`. When absent the
    /// boost feature is disabled entirely (no button on the helm).
    #[serde(default)]
    pub boost: Option<BoostConfig>,
    /// Optional procedural engine PFX tuning, from `[helm_console.engine_pfx]`.
    /// Rendering code supplies defaults for omitted fields.
    #[serde(default)]
    pub engine_pfx: Option<EnginePfxConfig>,
    /// Optional lateral thrust tuning, from `[helm_console.lateral_thrust]`.
    /// When absent, ShipPhysicsConfig defaults are used.
    #[serde(default)]
    pub lateral_thrust: Option<LateralThrustConfig>,
    /// Inline stateless AI policy for the **Engines** (longitudinal thrust)
    /// fine system, from `[helm_console.engines_ai]` (issue #779). Absent ⇒ the
    /// canonical [`default_engines_ai_config`] (unconditional actuate) is
    /// synthesised at spawn. Drives the `longitudinal` channel with the
    /// `actuate_desired_travel` mode verb.
    #[serde(default)]
    pub engines_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Steering** (yaw) fine system, from
    /// `[helm_console.steering_ai]` (issue #779). Absent ⇒ the canonical
    /// [`default_steering_ai_config`] is synthesised at spawn. Drives the `yaw`
    /// channel with the `actuate_desired_facing` mode verb.
    #[serde(default)]
    pub steering_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Lateral Thrust** fine system, from
    /// `[helm_console.lateral_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_lateral_ai_config`] (unconditional actuate) is synthesised at
    /// spawn. Drives the `lateral` channel with the `actuate_lateral_thrust`
    /// mode verb.
    #[serde(default)]
    pub lateral_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Vertical Thrust** fine system, from
    /// `[helm_console.vertical_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_vertical_ai_config`] (unconditional actuate) is synthesised at
    /// spawn. Drives the `vertical` channel with the `actuate_vertical_thrust`
    /// mode verb.
    #[serde(default)]
    pub vertical_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Impulse** fine system, from
    /// `[helm_console.impulse_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_impulse_ai_config`] (unconditional permit) is synthesised at
    /// spawn. Drives the `impulse` channel with the `engage_impulse` mode verb.
    #[serde(default)]
    pub impulse_ai: Option<FineSystemAiConfigToml>,
    /// Inline stateless AI policy for the **Boost** fine system, from
    /// `[helm_console.boost_ai]` (issue #780). Absent ⇒ the canonical
    /// [`default_boost_ai_config`] (explicit idle — no AI boost) is synthesised
    /// at spawn. Drives the `boost` channel with the `engage_boost` mode verb.
    #[serde(default)]
    pub boost_ai: Option<FineSystemAiConfigToml>,
}

/// What vertical movement capability the ship has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VerticalMovementMode {
    /// No vertical movement — planar-only flight (current default).
    #[default]
    Planar,
    /// AI-only bounded vertical motion for collision avoidance.
    Bounded,
    /// Full 3D six-degree-of-freedom flight.
    Full3D,
}

/// Impulse capability tuning loaded from `[helm_capability.impulse]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImpulseCapabilityConfig {
    /// Steering multiplier applied while impulse is active.
    /// 0.0 = no steering, 0.1 = harsh but possible, 1.0 = full steering.
    #[serde(default = "default_impulse_steering_multiplier")]
    pub steering_multiplier: f32,
}

fn default_impulse_steering_multiplier() -> f32 {
    0.1
}

impl Default for ImpulseCapabilityConfig {
    fn default() -> Self {
        Self {
            steering_multiplier: default_impulse_steering_multiplier(),
        }
    }
}

/// Optional helm capability declaration for an entity (`[helm_capability]`).
///
/// When absent, the ship has no special helm capability and operates at the
/// default planar mode with full steering during impulse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelmCapabilityConfig {
    /// Vertical movement mode. Defaults to `Planar`.
    #[serde(default)]
    pub vertical_movement_mode: VerticalMovementMode,
    /// Maximum vertical offset (world units) a `Bounded` craft may climb away
    /// from its cruise plane while dodging moving hazards (issue #744). Ignored
    /// for `Planar` (no vertical motion) and `Full3D` (unbounded).
    /// Defaults to [`crate::ai::MAX_VERTICAL_OFFSET`] when absent.
    #[serde(default = "default_max_vertical_offset")]
    pub max_vertical_offset: f32,
    /// Gradual return-to-cruise gain for a `Bounded` craft once avoidance
    /// urgency falls (issue #744): the vertical actuator eases the ship back to
    /// its cruise plane at `-y * vertical_return_rate` rather than snapping.
    /// Defaults to [`crate::ai::VERTICAL_RETURN_RATE`] when absent.
    #[serde(default = "default_vertical_return_rate")]
    pub vertical_return_rate: f32,
    /// Impulse capability tuning.
    #[serde(default)]
    pub impulse: ImpulseCapabilityConfig,
}

fn default_max_vertical_offset() -> f32 {
    crate::ai::MAX_VERTICAL_OFFSET
}

fn default_vertical_return_rate() -> f32 {
    crate::ai::VERTICAL_RETURN_RATE
}

/// Hand-written so `HelmCapabilityConfig::default()` matches what serde produces
/// for a `[helm_capability]` block that omits every optional field — a derived
/// `Default` would zero the vertical tunables instead of reading their authored
/// constant defaults.
impl Default for HelmCapabilityConfig {
    fn default() -> Self {
        Self {
            vertical_movement_mode: VerticalMovementMode::default(),
            max_vertical_offset: default_max_vertical_offset(),
            vertical_return_rate: default_vertical_return_rate(),
            impulse: ImpulseCapabilityConfig::default(),
        }
    }
}

/// Procedural engine trail tuning, from `[helm_console.engine_pfx]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EnginePfxConfig {
    /// RGBA trail colour in 0.0-1.0. When omitted, renderer defaults are used.
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    /// Optional rig-marker names used as exhaust origins.
    #[serde(default)]
    pub markers: Vec<String>,
    /// Twist, in degrees, applied around the direction of marker-attached trail emitters.
    /// When omitted, attached trails retain their default orientation.
    #[serde(default)]
    pub roll_degrees: Option<f32>,
    /// Uniform width multiplier for marker-attached trail emitters.
    /// When omitted, attached trails retain their default size.
    #[serde(default)]
    pub scale: Option<f32>,
    /// Seconds each trail segment remains alive. When omitted, renderer defaults are used.
    #[serde(default)]
    pub trail_lifetime_secs: Option<f32>,
    /// Seconds between spawned trail segments. When omitted, renderer defaults are used.
    #[serde(default)]
    pub trail_spawn_interval_secs: Option<f32>,
}

/// Lateral thrust tuning, from `[helm_console.lateral_thrust]`.
/// When absent, the feature uses ShipPhysicsConfig defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LateralThrustConfig {
    /// Maximum lateral speed in world units per second.
    #[serde(default = "default_lateral_thrust_max_speed")]
    pub max_lateral_speed: f32,
    /// Lateral acceleration in world units per second squared.
    #[serde(default = "default_lateral_thrust_acceleration")]
    pub lateral_acceleration: f32,
}

fn default_lateral_thrust_max_speed() -> f32 {
    15.0
}

fn default_lateral_thrust_acceleration() -> f32 {
    15.0
}

impl Default for LateralThrustConfig {
    fn default() -> Self {
        Self {
            max_lateral_speed: default_lateral_thrust_max_speed(),
            lateral_acceleration: default_lateral_thrust_acceleration(),
        }
    }
}

/// Boost drive tuning, from `[helm_console.boost]`. Presence of this table is
/// what enables the boost feature on a ship.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoostConfig {
    /// Multiplier applied to both max speed and acceleration while engaged.
    pub multiplier: f32,
    /// Multiplier applied to max yaw rate while engaged.
    #[serde(default = "default_boost_steering_multiplier")]
    pub steering_multiplier: f32,
    /// Seconds a full battery lasts while boost is engaged.
    pub active_duration: f32,
    /// Seconds for an empty battery to recharge to full.
    pub recharge_duration: f32,
}

impl HelmConsoleConfig {
    /// Radar range from `[helm_console.radar] range`. Returns `0.0` when the
    /// `[helm_console.radar]` table is absent.
    pub fn effective_radar_range(&self) -> f32 {
        self.radar.as_ref().map_or(0.0, |r| r.range)
    }
}

fn default_bank_lerp_rate() -> f32 {
    crate::ship_plugin::BANK_LERP_RATE
}

fn default_impulse_charge_duration() -> f32 {
    crate::ship::impulse::IMPULSE_CHARGE_DURATION
}

fn default_impulse_speed_multiplier() -> f32 {
    crate::ship::impulse::IMPULSE_SPEED_MULTIPLIER
}

fn default_impulse_engage_distance() -> f32 {
    200.0
}

fn default_impulse_cancel_distance() -> f32 {
    40.0
}

fn default_impulse_acceleration_multiplier() -> f32 {
    crate::ship::impulse::IMPULSE_ACCELERATION_MULTIPLIER
}

fn default_boost_steering_multiplier() -> f32 {
    crate::ship::boost::BOOST_STEERING_MULTIPLIER
}
