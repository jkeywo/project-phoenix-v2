//! System-kind registry and stable `SystemId` helpers.
//!
//! ## The three id namespaces (issue #801)
//!
//! One id, one meaning. Every identifier in the station/system architecture
//! belongs to exactly one of three namespaces:
//!
//! | Namespace | What it names | Type | Examples |
//! |-----------|---------------|------|----------|
//! | **System id** | A declared `[[system]]` instance: gets a `ControlSource`, gates admission, can be damaged/repaired | `SystemId` | `"helm-thrust"`, `"phaser-fore"`, `"sensors"` |
//! | **Station id** | A crew station (console). Keys console-level blackboards and channel-3 coordination routing | `StationId` (carried as `SystemId` in blackboard maps and coordination envelopes — see below) | `"helm"`, `"tactical"`, `"science"` |
//! | **Wire target** | The `target` string of a `ClientMessage::ControlSystem` envelope. Always a system id | JSON string | `"helm-steering"`, `"tactical-radar"`, `"phaser-control"` |
//!
//! The coarse `helm` and `tactical` *systems* were deleted by #801: `"helm"`
//! and `"tactical"` are now station ids only. Console-level blackboards (the
//! Helm console blackboard, the Weapons console blackboard) are keyed by the
//! station id; per-system blackboards (`"phaser-bank-*"`, `"power-reactor"`,
//! `"helm-lateral-thrust"`) keep system-id keys. The blackboard map and the
//! `BlackboardUpdate` wire message are typed `SystemId`, so station-id keys
//! are carried inside `SystemId` values — use the `*_station_key()` helpers
//! below, never `helm_thrust_system_id()`-style helpers, for those entries.
//!
//! ## SystemId naming convention (pinned by issue #525)
//!
//! Every `SystemId` string follows one of three patterns:
//!
//! | Pattern | Rule | Examples |
//! |---------|------|---------|
//! | **Coarse system** | Lowercase kebab matching the system kind id | `"sensors"`, `"captain"`, `"red-alert"` |
//! | **Fine system** | Kind id + `-` + instance suffix | `"phaser-fore"`, `"torpedo-tube-fore-port"` |
//! | **Ownerless capability** | Bare capability id (lowercase kebab) | `"red-alert"`, `"viewscreen"` |
//!
//! Multi-word ids always use hyphens (`-`), never underscores.
//!
//! ### `red_alert` vs `red-alert` quirk
//!
//! The registry key (`*_KIND` constants) uses snake_case for `red_alert` because
//! Rust identifiers and some legacy map keys historically used underscores, while
//! the wire `*_SYSTEM_ID` value uses kebab (`"red-alert"`). All other systems have
//! identical `*_KIND` and `*_SYSTEM_ID` values. New systems must use the same
//! lowercase-kebab string for both constants to avoid this split.

use crate::core::messages::{ConsoleFamily, SystemId};
use crate::ship::config::SystemInstanceConfig;
use std::collections::HashMap;

// ── Ownerless capability systems ─────────────────────────────────────────────

/// Wire `SystemId` for the Red Alert coarse system.
///
/// Ownerless capability — multi-word kebab id. Registry kind key is `"red_alert"`
/// (snake_case legacy quirk; see module-level doc for details).
pub const RED_ALERT_SYSTEM_ID: &str = "red-alert";
/// Registry kind key for Red Alert (snake_case for legacy reasons; see module doc).
pub const RED_ALERT_KIND: &str = "red_alert";

/// Wire `SystemId` for the Viewscreen coarse system.
///
/// Ownerless capability — single-word lowercase id.
pub const VIEWSCREEN_SYSTEM_ID: &str = "viewscreen";
pub const VIEWSCREEN_KIND: &str = "viewscreen";

/// Wire `SystemId` for the God Mode debug toggle (issue #900).
///
/// Ownerless capability, but unlike `RED_ALERT_SYSTEM_ID`/`VIEWSCREEN_SYSTEM_ID`
/// deliberately NOT declared by any `[[system]]` block in ship TOML: no station
/// owns it, and `command_admission::policy::station_for_system` returning
/// `None` for it is what denies a remote human token (the "unknown system"
/// fallback). A local-console token is still admitted because
/// `ControlSourceResolver::policy_for` defaults an unregistered `SystemId` to
/// `ControlSource::Human` (`accept_human_input: true`), and
/// `is_command_authorized`'s `LOCAL_CONSOLE_TOKEN` branch checks only that
/// policy — never station tenure. The same default policy has `operate_ai:
/// false`, so an `ai:`-prefixed token is denied without any special-casing:
/// this system id has no registered `[[system]]` kind, so it never appears
/// in the kind registry either.
pub const GOD_MODE_SYSTEM_ID: &str = "god-mode";

// ── Station ids (console namespace, issue #801) ──────────────────────────────
//
// These are NOT system ids. `"helm"` and `"tactical"` name crew stations
// (consoles). They key console-level blackboard entries and channel-3
// coordination routing, both of which are typed `SystemId` on the wire and in
// the per-ship blackboard map — so the station-id string is carried inside a
// `SystemId` value via the `*_station_key()` helpers. No `[[system]]` block
// declares these ids, no `ControlSource` is registered for them, and no
// `ControlSystem` wire message may target them.

/// Station id for the Helm console. Keys the Helm console blackboard and
/// helm-directed coordination messages.
pub const HELM_STATION_ID: &str = "helm";

/// Station id for the Tactical (weapons) console on the crewed hulls. Keys the
/// Weapons console blackboard and tactical-directed coordination messages.
/// Note the *station* owning a hull's weapons is resolved from the ship config
/// (`ShipConfig::weapons_station`) — a single-station hull may own its guns on
/// `"pilot"` — but the blackboard/coordination key is always this string.
pub const TACTICAL_STATION_ID: &str = "tactical";

// ── Station-owned coarse systems ─────────────────────────────────────────────

/// Wire `SystemId` for the Power coarse system.
pub const POWER_SYSTEM_ID: &str = "power";
pub const POWER_KIND: &str = "power";

/// Wire `SystemId` for the Sensors coarse system.
pub const SENSORS_SYSTEM_ID: &str = "sensors";
pub const SENSORS_KIND: &str = "sensors";

/// Wire `SystemId` for the Navigation coarse system.
pub const NAVIGATION_SYSTEM_ID: &str = "navigation";
pub const NAVIGATION_KIND: &str = "navigation";

/// Wire `SystemId` for the Shields coarse system.
pub const SHIELDS_SYSTEM_ID: &str = "shields";
pub const SHIELDS_KIND: &str = "shields";

/// Wire `SystemId` for the Comms coarse system.
pub const COMMS_SYSTEM_ID: &str = "comms";
pub const COMMS_KIND: &str = "comms";

/// Wire `SystemId` for the Captain coarse system.
pub const CAPTAIN_SYSTEM_ID: &str = "captain";
pub const CAPTAIN_KIND: &str = "captain";

/// Wire `SystemId` for the Repair coarse system.
pub const REPAIR_SYSTEM_ID: &str = "repair";
pub const REPAIR_KIND: &str = "repair";

/// Wire `SystemId` for the Command coarse system (issue #1107).
///
/// The admitted-command target for an auxiliary Command station's stance
/// selection (`SystemControlPayload::SetStationStance`). Like the other
/// capability systems it owns no fine actuator — it is the seat a
/// `human_seeking` + `auxiliary` Command station carries so its stance orders
/// have a station to be authorised against (the seek host, normally Captain).
pub const COMMAND_SYSTEM_ID: &str = "command";
pub const COMMAND_KIND: &str = "command";

/// Wire `SystemId` for the tractor-beam system (issue #1156).
///
/// The linchpin of PRD #1143's coupling family: a first-class,
/// engineering-owned `[[system]]` that declares its own power group, carries a
/// damage entry, is admission-gated (`EngageTractor` / `ReleaseTractor`), and
/// publishes its own blackboard. Engaging it couples the ship to whatever
/// Tactical currently has locked; the pure sibling `crate::tractor::coupling`
/// owns the geometry. Its coupling terms — range, offset, minimum power level —
/// are authored in a hull's `[tractor]` table, never hardcoded, so a hull that
/// declares neither the system nor the table is unchanged in every way. The
/// umbilical (#1160), dock (#1159) and external repair-dispatch (#1161) copy
/// this shape.
pub const TRACTOR_SYSTEM_ID: &str = "tractor";
pub const TRACTOR_KIND: &str = "tractor";

/// Wire `SystemId` for the docking system (issue #1159).
///
/// The second slice of PRD #1143's coupling family: a helm-owned `[[system]]`
/// that declares its own power group, carries a damage entry, is admission-gated
/// (`Dock` / `Undock`), and publishes its own blackboard. Running it flies an
/// automatic manoeuvre that mates the two hulls' nearest viable dock-marker pair
/// (the pure `crate::dock::mating` owns the geometry). Its terms — range, engage
/// distance, approach speed, mate tolerance, undock clearance, minimum power
/// level — are authored in a hull's `[dock]` table, and its dock markers in the
/// rig sidecar, so a hull that declares neither is unchanged in every way. The
/// docked relationship this forms is what the umbilical (#1160) gates on.
pub const DOCK_SYSTEM_ID: &str = "dock";
pub const DOCK_KIND: &str = "dock";

/// Wire `SystemId` for the transfer umbilical (issue #1160).
///
/// The third slice of PRD #1143's coupling family: an engineering-owned
/// `[[system]]` that declares its own power group, carries a damage entry, is
/// admission-gated (`StartTransfer` / `StopTransfer`), and publishes its own
/// blackboard. Running it moves an authored capacity per second between the two
/// DOCKED hulls' capacity ledgers (the pure `crate::umbilical::flow` owns the
/// arithmetic). Its terms — capacity id, rate, direction, minimum power level —
/// are authored in a hull's `[umbilical]` table, so a hull that declares neither
/// the system nor the table is unchanged in every way. It gates on the dock
/// (#1159): a flow runs only while the umbilical's own hull is docked.
pub const UMBILICAL_SYSTEM_ID: &str = "umbilical";
pub const UMBILICAL_KIND: &str = "umbilical";

// ── Fine-grained Helm systems (issue #511) ────────────────────────────────────

/// Wire `SystemId` for the Helm Joystick fine system.
pub const HELM_JOYSTICK_KIND: &str = "helm_joystick";
pub const HELM_JOYSTICK_SYSTEM_ID: &str = "helm-joystick";

/// Wire `SystemId` for the Helm Engine fine systems (port + starboard instances).
pub const HELM_ENGINE_KIND: &str = "helm_engine";
pub const HELM_ENGINE_PORT_SYSTEM_ID: &str = "helm-engine-port";
pub const HELM_ENGINE_STARBOARD_SYSTEM_ID: &str = "helm-engine-starboard";

/// Wire `SystemId` for the Helm Radar fine system.
pub const HELM_RADAR_KIND: &str = "helm_radar";
pub const HELM_RADAR_SYSTEM_ID: &str = "helm-radar";

/// Wire `SystemId` for the Helm Impulse fine system.
pub const HELM_IMPULSE_KIND: &str = "helm_impulse";
pub const HELM_IMPULSE_SYSTEM_ID: &str = "helm-impulse";

/// Wire `SystemId` for the Helm Lateral Thrust fine system.
pub const LATERAL_THRUST_KIND: &str = "lateral_thrust";
pub const LATERAL_THRUST_SYSTEM_ID: &str = "helm-lateral-thrust";

/// Wire `SystemId` for the Helm Vertical Thrust fine system (issue #744).
///
/// Owns the vertical (up/down) axis: the `VerticalThrustInput` intent component.
/// AI-only — no player-facing control — driven by `ai_helm_vertical_thrust` for
/// bounded / full-3D craft avoiding moving hazards.
pub const VERTICAL_THRUST_KIND: &str = "vertical_thrust";
pub const VERTICAL_THRUST_SYSTEM_ID: &str = "helm-vertical-thrust";

/// Wire `SystemId` for the Helm Thrust fine system (issue #701).
///
/// Owns the throttle axis: the `ThrustInput` intent component. Split out of
/// the coarse `helm` kind so a station rating can automate the throttle while
/// a human keeps the stick (and vice versa), and so the axis can be damaged
/// and repaired independently.
pub const HELM_THRUST_KIND: &str = "helm_thrust";
pub const HELM_THRUST_SYSTEM_ID: &str = "helm-thrust";

/// Wire `SystemId` for the Helm Steering fine system (issue #701).
///
/// Owns the yaw axis: the `SteeringInput` intent component. Counterpart to
/// [`HELM_THRUST_KIND`] — see that constant for the rationale behind the
/// per-axis split.
pub const HELM_STEERING_KIND: &str = "helm_steering";
pub const HELM_STEERING_SYSTEM_ID: &str = "helm-steering";

/// Wire `SystemId` for the Helm Boost fine system (issue #801).
///
/// Owns the boost drive commands (`ToggleBoost` / `SetBoost`). Split out of
/// the deleted coarse `helm` system so boost admission gates on its own
/// declared system, like every other helm axis.
pub const HELM_BOOST_KIND: &str = "helm_boost";
pub const HELM_BOOST_SYSTEM_ID: &str = "helm-boost";

// ── Fine-grained Tactical systems (issue #512) ────────────────────────────────
//
// The coarse `tactical` kind is gone entirely (#512 removed the `[[system]]`
// block; #801 removed the id from the system namespace). `"tactical"` survives
// only as [`TACTICAL_STATION_ID`] — the station-id key for the Weapons console
// blackboard and coordination routing. Ship-level operations moved to real
// declared systems: `SetTarget` targets `tactical-radar`; `SetPhaserMode` /
// `SetPhaserFrequency` target `phaser-control`.

/// Wire `SystemId` for the Phaser Bank fine systems.
///
/// Registered per-instance in TOML (e.g. `"phaser-fore"`, `"phaser-aft"`).
pub const PHASER_BANK_KIND: &str = "phaser_bank";
pub const PHASER_FORE_SYSTEM_ID: &str = "phaser-fore";
pub const PHASER_AFT_SYSTEM_ID: &str = "phaser-aft";

/// Wire `SystemId` for the Torpedo Tube fine systems.
///
/// Registered per-instance in TOML (e.g. `"torpedo-tube-fore-port"`).
pub const TORPEDO_TUBE_KIND: &str = "torpedo_tube";
pub const TORPEDO_TUBE_FORE_PORT_SYSTEM_ID: &str = "torpedo-tube-fore-port";
pub const TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID: &str = "torpedo-tube-fore-starboard";
pub const TORPEDO_TUBE_AFT_SYSTEM_ID: &str = "torpedo-tube-aft";

/// Wire `SystemId` for the Blaster Bank fine systems (issue #631).
///
/// Registered per-instance in TOML (e.g. `"blaster-fore"`, `"blaster-aft"`).
/// A blaster bank fires straight-flying projectiles in data-driven volleys.
pub const BLASTER_BANK_KIND: &str = "blaster_bank";

/// Wire `SystemId` for the Phaser Control fine system (issue #801).
///
/// A single declared system owning the ship-wide phaser settings: the firing
/// mode (`CurrentPhaserMode`) and the emitter frequency
/// (`ShipPhaserFrequency`). These are ship-wide values, NOT per-bank — the
/// data model is unchanged; this system exists so `SetPhaserMode` /
/// `SetPhaserFrequency` admission gates on a real declared system instead of
/// the deleted coarse `tactical` id.
pub const PHASER_CONTROL_KIND: &str = "phaser_control";
pub const PHASER_CONTROL_SYSTEM_ID: &str = "phaser-control";

/// Wire `SystemId` for the Tactical Radar fine system.
///
/// Mirrors `HELM_RADAR_KIND`/`HELM_RADAR_SYSTEM_ID` — the tactical station's
/// short-range weapons radar, made damageable/repairable like every other
/// fine system.
pub const TACTICAL_RADAR_KIND: &str = "tactical_radar";
pub const TACTICAL_RADAR_SYSTEM_ID: &str = "tactical-radar";

/// Wire `SystemId` for the Sensor Radar fine system.
///
/// Mirrors `HELM_RADAR_KIND`/`HELM_RADAR_SYSTEM_ID` — the sensors/science
/// station's long-range radar, made damageable/repairable like every other
/// fine system.
pub const SENSOR_RADAR_KIND: &str = "sensor_radar";
pub const SENSOR_RADAR_SYSTEM_ID: &str = "sensor-radar";

/// Wire `SystemId` for the Torpedo Magazine fine system (single instance).
///
/// The magazine owns the shared torpedo `count`; tubes claim a round via
/// the channel-2 [`crate::core::messages::InterSystemPayload::ClaimTorpedoRound`]
/// message. A Disabled/Destroyed magazine refuses claims (no tubes can load
/// even if a round would otherwise be available), and also blocks the fire
/// path so loaded tubes cannot launch.
pub const TORPEDO_MAGAZINE_KIND: &str = "torpedo_magazine";
pub const TORPEDO_MAGAZINE_SYSTEM_ID: &str = "torpedo-magazine";

// ── Fine-grained Power systems (issue #513) ──────────────────────────────────
//
// The coarse `power` kind is DELETED from the player ship TOML, but
// `POWER_SYSTEM_ID = "power"` remains as a stable string constant so tests
// and legacy readers (e.g. the JS panel's aggregate `blackboards['power']`
// entry) can still address the aggregate surface. All admission /
// allocation logic now targets the fine `power_reactor` kind. Both fine
// Power systems live on the `power` station and are held by the single
// power-station holder — the split is invisible to the human but grants
// per-instance damage semantics (reactor disabled → no allocation input;
// battery disabled → no emergency reserves).

/// Wire `SystemId` for the Power Reactor fine system.
///
/// The reactor OWNS the allocation surface: `SetPowerGroupAllocation`
/// payloads are gated on `policy_for(&power_reactor_system_id())`.
/// A Disabled/Destroyed reactor refuses allocation input via the standard
/// `accept_human_input` gate.
pub const POWER_REACTOR_KIND: &str = "power_reactor";
pub const POWER_REACTOR_SYSTEM_ID: &str = "power-reactor";

/// Wire `SystemId` for the Power Battery fine system.
///
/// The battery owns the emergency-reserve state published to the Power console.
/// Its charge is integrated from the reactor's authored allocation-rate curve;
/// weapon activity does not mutate it directly.
pub const POWER_BATTERY_KIND: &str = "power_battery";
pub const POWER_BATTERY_SYSTEM_ID: &str = "power-battery";

// ── Fine-grained Shields systems (issue #514) ────────────────────────────────
//
// The coarse `shields` kind is DELETED from the player ship TOML, but
// `SHIELDS_SYSTEM_ID = "shields"` remains as a stable string constant so
// tests and legacy readers (e.g. the JS panel's aggregate
// `blackboards['shields']` entry) can still address the aggregate surface.
// All per-arc admission and per-arc damage now target `shield_arc` fine
// systems registered per-instance in TOML (e.g. `"shield-arc-fore"`).
//
// Ships may declare any number of `[[shield_arc]]` blocks; each block
// auto-generates a corresponding `[[system]]` entry with
// `kind = "shield_arc"` at TOML-parse time.

/// Wire `SystemId` for the Shield Arc fine systems.
///
/// Registered per-instance in TOML (e.g. `"shield-arc-fore"`,
/// `"shield-arc-aft"`). Arc count is variable — a ship declares one
/// `[[shield_arc]]` block per arc, from which the parser synthesises a
/// matching `[[system]]` entry.
pub const SHIELD_ARC_KIND: &str = "shield_arc";

/// Authoritative metadata owned by one `[[system]]` kind.
///
/// Console Family is optional during the #1251 tracer: Command and Dock carry
/// it now, while issue #1252 migrates every remaining kind and removes the
/// client's temporary inference fallback. Command capability joins this same
/// descriptor in issue #1253 rather than creating a second registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemKindDescriptor {
    kind: String,
    console_family: Option<ConsoleFamily>,
}

impl SystemKindDescriptor {
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            console_family: None,
        }
    }

    pub fn with_console_family(mut self, family: ConsoleFamily) -> Self {
        self.console_family = Some(family);
        self
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn console_family(&self) -> Option<ConsoleFamily> {
        self.console_family
    }
}

/// The descriptor registry for valid `[[system]]` kind strings.
///
/// Consumers (`ship_plugin::load_ship_config_from_disk`,
/// `entities::config`) build the registry via [`Self::with_core_systems`]
/// and validate TOML against [`Self::kinds`] / [`Self::contains`]. The
/// pre-#520 per-kind named AI-controller registration layer that used to
/// live alongside the kinds was dead weight and has been deleted — AI
/// behaviour is attached per kind by dedicated Bevy systems gated on
/// `ControlSourceResolver::policy_for`, not by registry lookup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemKindRegistry {
    descriptors: HashMap<String, SystemKindDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SystemRegistryError {
    EmptyKind,
    DuplicateKind { kind: String },
}

impl std::fmt::Display for SystemRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for SystemRegistryError {}

impl SystemKindRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_red_alert() -> Result<Self, SystemRegistryError> {
        let mut registry = Self::new();
        registry.register(RED_ALERT_KIND)?;
        Ok(registry)
    }

    pub fn with_core_systems() -> Result<Self, SystemRegistryError> {
        let mut registry = Self::with_red_alert()?;
        registry.register(POWER_KIND)?;
        registry.register(SENSORS_KIND)?;
        registry.register(NAVIGATION_KIND)?;
        registry.register(SHIELDS_KIND)?;
        registry.register(COMMS_KIND)?;
        registry.register(CAPTAIN_KIND)?;
        registry.register(VIEWSCREEN_KIND)?;
        registry.register(REPAIR_KIND)?;
        // Command is the #1251 Console-Family metadata tracer.
        registry.register_descriptor(
            SystemKindDescriptor::new(COMMAND_KIND).with_console_family(ConsoleFamily::Command),
        )?;
        // Tractor-beam system (issue #1156).
        registry.register(TRACTOR_KIND)?;
        // Dock is rendered by Helm regardless of either instance/station id:
        // the second half of the #1251 Console-Family metadata tracer.
        registry.register_descriptor(
            SystemKindDescriptor::new(DOCK_KIND).with_console_family(ConsoleFamily::Helm),
        )?;
        // Transfer umbilical (issue #1160).
        registry.register(UMBILICAL_KIND)?;
        // Fine-grained Helm systems (issue #511)
        registry.register(HELM_JOYSTICK_KIND)?;
        registry.register(HELM_ENGINE_KIND)?;
        registry.register(HELM_RADAR_KIND)?;
        registry.register(HELM_IMPULSE_KIND)?;
        registry.register(LATERAL_THRUST_KIND)?;
        registry.register(VERTICAL_THRUST_KIND)?;
        // Per-axis Helm systems (issue #701)
        registry.register(HELM_THRUST_KIND)?;
        registry.register(HELM_STEERING_KIND)?;
        // Helm boost fine system (issue #801)
        registry.register(HELM_BOOST_KIND)?;
        // Fine-grained Tactical systems (issue #512)
        registry.register(PHASER_BANK_KIND)?;
        registry.register(TORPEDO_TUBE_KIND)?;
        registry.register(TORPEDO_MAGAZINE_KIND)?;
        // Blaster bank fine system (issue #631)
        registry.register(BLASTER_BANK_KIND)?;
        // Phaser control fine system (issue #801)
        registry.register(PHASER_CONTROL_KIND)?;
        // Tactical / sensor radar fine systems
        registry.register(TACTICAL_RADAR_KIND)?;
        registry.register(SENSOR_RADAR_KIND)?;
        // Fine-grained Power systems (issue #513)
        registry.register(POWER_REACTOR_KIND)?;
        registry.register(POWER_BATTERY_KIND)?;
        // Fine-grained Shields systems (issue #514)
        registry.register(SHIELD_ARC_KIND)?;
        Ok(registry)
    }

    pub fn register(&mut self, kind: impl Into<String>) -> Result<(), SystemRegistryError> {
        self.register_descriptor(SystemKindDescriptor::new(kind))
    }

    pub fn register_descriptor(
        &mut self,
        descriptor: SystemKindDescriptor,
    ) -> Result<(), SystemRegistryError> {
        if descriptor.kind.trim().is_empty() {
            return Err(SystemRegistryError::EmptyKind);
        }
        if self.descriptors.contains_key(&descriptor.kind) {
            return Err(SystemRegistryError::DuplicateKind {
                kind: descriptor.kind,
            });
        }
        self.descriptors.insert(descriptor.kind.clone(), descriptor);
        Ok(())
    }

    pub fn contains(&self, kind: &str) -> bool {
        self.descriptors.contains_key(kind)
    }

    pub fn kinds(&self) -> impl Iterator<Item = &str> {
        self.descriptors.keys().map(|kind| kind.as_str())
    }

    pub fn descriptor(&self, kind: &str) -> Option<&SystemKindDescriptor> {
        self.descriptors.get(kind)
    }

    /// Resolve authored System instances to the Console Family declared by
    /// their kinds. Unmigrated kinds are intentionally omitted: this tracer's
    /// payload tells the client exactly which ids are authoritative and leaves
    /// only those missing ids eligible for the temporary #1252 fallback.
    pub fn project_console_families(
        &self,
        systems: &[SystemInstanceConfig],
    ) -> HashMap<String, ConsoleFamily> {
        systems
            .iter()
            .filter_map(|system| {
                self.descriptor(&system.kind)
                    .and_then(SystemKindDescriptor::console_family)
                    .map(|family| (system.id.0.clone(), family))
            })
            .collect()
    }
}

// ── SystemId helpers ──────────────────────────────────────────────────────────
//
// Each helper returns a `SystemId` backed by the corresponding `*_SYSTEM_ID`
// constant. Always prefer these helpers over inline `SystemId("helm".into())`
// literals — the helpers are the pinned authoritative source.

pub fn red_alert_system_id() -> SystemId {
    SystemId(RED_ALERT_SYSTEM_ID.to_string())
}

// ── Station-key helpers (console namespace, issue #801) ──────────────────────
//
// These return the station-id string wrapped in a `SystemId` because the
// blackboard map (`ShipSystemBlackboards`), the `BlackboardUpdate` wire
// message and the coordination envelope are all typed `SystemId`. They are
// NOT system ids: nothing registers a `ControlSource` for them and no
// `ControlSystem` message may target them.

/// Station-id key for the Helm console blackboard / helm-directed coordination.
pub fn helm_station_key() -> SystemId {
    SystemId(HELM_STATION_ID.to_string())
}

/// Station-id key for the Weapons console blackboard / tactical-directed
/// coordination.
pub fn tactical_station_key() -> SystemId {
    SystemId(TACTICAL_STATION_ID.to_string())
}

// NOTE: There is no `power_system_id()` helper. The coarse `POWER_SYSTEM_ID`
// string constant is retained only for the aggregate blackboard key (published
// alongside the fine `power-reactor` / `power-battery` blackboards for legacy
// JS readers). All control-input routing must use
// `power_reactor_system_id()` (allocation surface) or
// `power_battery_system_id()` (channel-2 drain target).

pub fn sensors_system_id() -> SystemId {
    SystemId(SENSORS_SYSTEM_ID.to_string())
}

pub fn navigation_system_id() -> SystemId {
    SystemId(NAVIGATION_SYSTEM_ID.to_string())
}

pub fn shields_system_id() -> SystemId {
    SystemId(SHIELDS_SYSTEM_ID.to_string())
}

pub fn comms_system_id() -> SystemId {
    SystemId(COMMS_SYSTEM_ID.to_string())
}

pub fn captain_system_id() -> SystemId {
    SystemId(CAPTAIN_SYSTEM_ID.to_string())
}

pub fn viewscreen_system_id() -> SystemId {
    SystemId(VIEWSCREEN_SYSTEM_ID.to_string())
}

pub fn repair_system_id() -> SystemId {
    SystemId(REPAIR_SYSTEM_ID.to_string())
}

pub fn command_system_id() -> SystemId {
    SystemId(COMMAND_SYSTEM_ID.to_string())
}

/// The tractor-beam system's wire `SystemId` (issue #1156). The admitted target
/// for `EngageTractor` / `ReleaseTractor` and the key its blackboard publishes
/// under.
pub fn tractor_system_id() -> SystemId {
    SystemId(TRACTOR_SYSTEM_ID.to_string())
}

/// The conventional shipped docking `SystemId` (issue #1159).
///
/// Runtime Dock controls resolve their authored instance id by `kind = "dock"`;
/// this helper is for canonical topology and fixtures that deliberately use the
/// conventional `id = "dock"` spelling.
pub fn dock_system_id() -> SystemId {
    SystemId(DOCK_SYSTEM_ID.to_string())
}

/// The transfer umbilical's wire `SystemId` (issue #1160). The admitted target
/// for `StartTransfer` / `StopTransfer` and the key its blackboard publishes
/// under.
pub fn umbilical_system_id() -> SystemId {
    SystemId(UMBILICAL_SYSTEM_ID.to_string())
}

// ── Fine Helm system id helpers (issue #511) ──────────────────────────────────

pub fn helm_joystick_system_id() -> SystemId {
    SystemId(HELM_JOYSTICK_SYSTEM_ID.to_string())
}

pub fn helm_engine_port_system_id() -> SystemId {
    SystemId(HELM_ENGINE_PORT_SYSTEM_ID.to_string())
}

pub fn helm_engine_starboard_system_id() -> SystemId {
    SystemId(HELM_ENGINE_STARBOARD_SYSTEM_ID.to_string())
}

pub fn helm_radar_system_id() -> SystemId {
    SystemId(HELM_RADAR_SYSTEM_ID.to_string())
}

pub fn tactical_radar_system_id() -> SystemId {
    SystemId(TACTICAL_RADAR_SYSTEM_ID.to_string())
}

pub fn sensor_radar_system_id() -> SystemId {
    SystemId(SENSOR_RADAR_SYSTEM_ID.to_string())
}

pub fn helm_impulse_system_id() -> SystemId {
    SystemId(HELM_IMPULSE_SYSTEM_ID.to_string())
}

pub fn lateral_thrust_system_id() -> SystemId {
    SystemId(LATERAL_THRUST_SYSTEM_ID.to_string())
}

pub fn vertical_thrust_system_id() -> SystemId {
    SystemId(VERTICAL_THRUST_SYSTEM_ID.to_string())
}

// ── Per-axis Helm system id helpers (issue #701) ──────────────────────────────

pub fn helm_thrust_system_id() -> SystemId {
    SystemId(HELM_THRUST_SYSTEM_ID.to_string())
}

pub fn helm_steering_system_id() -> SystemId {
    SystemId(HELM_STEERING_SYSTEM_ID.to_string())
}

pub fn helm_boost_system_id() -> SystemId {
    SystemId(HELM_BOOST_SYSTEM_ID.to_string())
}

// ── Fine Tactical system id helpers (issue #512) ──────────────────────────────

pub fn phaser_fore_system_id() -> SystemId {
    SystemId(PHASER_FORE_SYSTEM_ID.to_string())
}

pub fn phaser_aft_system_id() -> SystemId {
    SystemId(PHASER_AFT_SYSTEM_ID.to_string())
}

pub fn torpedo_tube_fore_port_system_id() -> SystemId {
    SystemId(TORPEDO_TUBE_FORE_PORT_SYSTEM_ID.to_string())
}

pub fn torpedo_tube_fore_starboard_system_id() -> SystemId {
    SystemId(TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID.to_string())
}

pub fn torpedo_tube_aft_system_id() -> SystemId {
    SystemId(TORPEDO_TUBE_AFT_SYSTEM_ID.to_string())
}

pub fn torpedo_magazine_system_id() -> SystemId {
    SystemId(TORPEDO_MAGAZINE_SYSTEM_ID.to_string())
}

pub fn phaser_control_system_id() -> SystemId {
    SystemId(PHASER_CONTROL_SYSTEM_ID.to_string())
}

// ── Fine Power system id helpers (issue #513) ─────────────────────────────────

pub fn power_reactor_system_id() -> SystemId {
    SystemId(POWER_REACTOR_SYSTEM_ID.to_string())
}

pub fn power_battery_system_id() -> SystemId {
    SystemId(POWER_BATTERY_SYSTEM_ID.to_string())
}

/// Resolve the `SystemId` for a phaser bank by its TOML `id`.
///
/// The convention is `"phaser-<bank_id>"`, so `"fore"` → `"phaser-fore"`,
/// `"aft"` → `"phaser-aft"`, `"port"` → `"phaser-port"`, etc. Returns
/// `Some` for every non-empty bank id — callers should combine with
/// `system_is_registered` on the ship's `ControlSourceResolver` to
/// distinguish "the fine system is offline" from "the ship never declared
/// a fine system for this bank" (NPC path).
pub fn phaser_bank_system_id(bank_id: &str) -> Option<SystemId> {
    if bank_id.is_empty() {
        return None;
    }
    Some(SystemId(format!("phaser-{bank_id}")))
}

/// Resolve the `SystemId` for a blaster bank by its TOML `id` (issue #631).
///
/// The convention is `"blaster-<bank_id>"`, so `"fore"` → `"blaster-fore"`,
/// `"aft"` → `"blaster-aft"`, etc. Returns `Some` for every non-empty bank id.
/// Underscore-to-hyphen conversion follows the project convention.
pub fn blaster_bank_system_id(bank_id: &str) -> Option<SystemId> {
    if bank_id.is_empty() {
        return None;
    }
    Some(SystemId(format!("blaster-{}", bank_id.replace('_', "-"))))
}

/// Resolve the `SystemId` for a torpedo tube by its TOML `id`.
///
/// The convention is `"torpedo-tube-<tube_id_with_underscores_to_hyphens>"`,
/// so `"fore_port"` → `"torpedo-tube-fore-port"`. Returns `Some` for every
/// non-empty tube id.
pub fn torpedo_tube_system_id(tube_id: &str) -> Option<SystemId> {
    if tube_id.is_empty() {
        return None;
    }
    Some(SystemId(format!(
        "torpedo-tube-{}",
        tube_id.replace('_', "-")
    )))
}

/// Resolve the `SystemId` for a shield arc by its TOML `id`.
///
/// The convention is `"shield-arc-<arc_id_with_underscores_to_hyphens>"`,
/// so `"fore"` → `"shield-arc-fore"`, `"all"` → `"shield-arc-all"`.
/// Returns `Some` for every non-empty arc id. NPCs that declare a single
/// omni arc (id = "all") get their own fine SystemId without needing a
/// match-arm update.
pub fn shield_arc_system_id(arc_id: &str) -> Option<SystemId> {
    if arc_id.is_empty() {
        return None;
    }
    Some(SystemId(format!("shield-arc-{}", arc_id.replace('_', "-"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Stable id string values ───────────────────────────────────────────────
    // These tests pin the naming convention so a rename of a constant breaks CI
    // rather than silently drifting the wire format.

    #[test]
    fn coarse_system_ids_are_lowercase_kebab() {
        let ids = [
            RED_ALERT_SYSTEM_ID,
            POWER_SYSTEM_ID,
            SENSORS_SYSTEM_ID,
            NAVIGATION_SYSTEM_ID,
            SHIELDS_SYSTEM_ID,
            COMMS_SYSTEM_ID,
            CAPTAIN_SYSTEM_ID,
            VIEWSCREEN_SYSTEM_ID,
            REPAIR_SYSTEM_ID,
            COMMAND_SYSTEM_ID,
        ];
        for id in ids {
            assert_eq!(
                id,
                id.to_lowercase(),
                "SystemId constant {id:?} is not lowercase"
            );
            assert!(
                !id.contains('_'),
                "SystemId constant {id:?} contains underscore (use hyphen)"
            );
            assert!(!id.is_empty(), "SystemId constant must not be empty");
        }
    }

    #[test]
    fn coarse_system_id_values_are_stable() {
        assert_eq!(RED_ALERT_SYSTEM_ID, "red-alert");
        assert_eq!(POWER_SYSTEM_ID, "power");
        assert_eq!(SENSORS_SYSTEM_ID, "sensors");
        assert_eq!(NAVIGATION_SYSTEM_ID, "navigation");
        assert_eq!(SHIELDS_SYSTEM_ID, "shields");
        assert_eq!(COMMS_SYSTEM_ID, "comms");
        assert_eq!(CAPTAIN_SYSTEM_ID, "captain");
        assert_eq!(VIEWSCREEN_SYSTEM_ID, "viewscreen");
        assert_eq!(REPAIR_SYSTEM_ID, "repair");
        assert_eq!(COMMAND_SYSTEM_ID, "command");
    }

    #[test]
    fn system_id_helpers_return_expected_values() {
        assert_eq!(red_alert_system_id().0, RED_ALERT_SYSTEM_ID);
        // No `power_system_id()` helper — see note above the station-key helpers.
        // The coarse constant is still pinned by `coarse_system_id_values_are_stable`.
        assert_eq!(sensors_system_id().0, SENSORS_SYSTEM_ID);
        assert_eq!(navigation_system_id().0, NAVIGATION_SYSTEM_ID);
        assert_eq!(shields_system_id().0, SHIELDS_SYSTEM_ID);
        assert_eq!(comms_system_id().0, COMMS_SYSTEM_ID);
        assert_eq!(captain_system_id().0, CAPTAIN_SYSTEM_ID);
        assert_eq!(viewscreen_system_id().0, VIEWSCREEN_SYSTEM_ID);
        assert_eq!(repair_system_id().0, REPAIR_SYSTEM_ID);
    }

    // ── Registry API ─────────────────────────────────────────────────────────

    #[test]
    fn register_adds_kind() {
        let mut registry = SystemKindRegistry::new();

        registry.register("red_alert").unwrap();

        assert!(registry.contains("red_alert"));
    }

    #[test]
    fn command_and_dock_descriptors_own_the_tracer_console_families() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert_eq!(
            registry
                .descriptor(COMMAND_KIND)
                .and_then(SystemKindDescriptor::console_family),
            Some(ConsoleFamily::Command)
        );
        assert_eq!(
            registry
                .descriptor(DOCK_KIND)
                .and_then(SystemKindDescriptor::console_family),
            Some(ConsoleFamily::Helm)
        );
        assert_eq!(
            registry
                .descriptor(HELM_THRUST_KIND)
                .and_then(SystemKindDescriptor::console_family),
            None,
            "unmigrated kinds stay absent until #1252 rather than gaining guessed metadata"
        );
    }

    #[test]
    fn console_family_projection_resolves_instance_ids_by_kind() {
        use crate::ship::config::SystemInstanceConfig;

        let systems = vec![
            SystemInstanceConfig {
                id: SystemId("bridge-orders".into()),
                kind: COMMAND_KIND.into(),
                station: None,
                ai_only: false,
                human_seeking: false,
                seek_order: Vec::new(),
                power_group: None,
                marker: None,
                config: None,
            },
            SystemInstanceConfig {
                id: SystemId("berthing-clamps".into()),
                kind: DOCK_KIND.into(),
                station: None,
                ai_only: false,
                human_seeking: false,
                seek_order: Vec::new(),
                power_group: None,
                marker: None,
                config: None,
            },
            SystemInstanceConfig {
                id: SystemId("main-drive".into()),
                kind: HELM_THRUST_KIND.into(),
                station: None,
                ai_only: false,
                human_seeking: false,
                seek_order: Vec::new(),
                power_group: None,
                marker: None,
                config: None,
            },
        ];

        let projected = SystemKindRegistry::with_core_systems()
            .unwrap()
            .project_console_families(&systems);
        assert_eq!(
            projected.get("bridge-orders"),
            Some(&ConsoleFamily::Command)
        );
        assert_eq!(projected.get("berthing-clamps"), Some(&ConsoleFamily::Helm));
        assert!(
            !projected.contains_key("main-drive"),
            "the tracer payload must not fabricate metadata for unmigrated families"
        );
    }

    #[test]
    fn every_shipped_command_and_dock_instance_projects_its_console_family() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/entities");
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("assets/entities must be readable")
            .map(|entry| entry.expect("readable entity entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .collect();
        entries.sort();

        let registry = SystemKindRegistry::with_core_systems().unwrap();
        let mut command_instances = 0usize;
        let mut dock_instances = 0usize;

        for path in entries {
            let source = path.to_string_lossy().replace('\\', "/");
            let entity = crate::entities::include_resolve::load_entity_config(&source)
                .unwrap_or_else(|error| panic!("parse {source}: {error}"));
            let Some(ship) = entity.ship_config.as_ref() else {
                continue;
            };
            let projected = registry.project_console_families(&ship.systems);
            for system in &ship.systems {
                let expected = match system.kind.as_str() {
                    COMMAND_KIND => {
                        command_instances += 1;
                        ConsoleFamily::Command
                    }
                    DOCK_KIND => {
                        dock_instances += 1;
                        ConsoleFamily::Helm
                    }
                    _ => continue,
                };
                assert_eq!(
                    projected.get(&system.id.0),
                    Some(&expected),
                    "{source}: {:?} must reach the public topology projection by kind",
                    system.id
                );
            }
        }

        assert_eq!(
            command_instances, 1,
            "the shipped Command tracer disappeared"
        );
        assert!(
            dock_instances >= 4,
            "expected every shipped Dock tracer topology, got {dock_instances}"
        );
    }

    #[test]
    fn rejects_empty_kind() {
        let mut registry = SystemKindRegistry::new();

        assert_eq!(registry.register("  "), Err(SystemRegistryError::EmptyKind));
    }

    #[test]
    fn rejects_duplicate_kind() {
        let mut registry = SystemKindRegistry::new();
        registry.register("red_alert").unwrap();

        assert_eq!(
            registry.register("red_alert"),
            Err(SystemRegistryError::DuplicateKind {
                kind: "red_alert".into()
            })
        );
    }

    #[test]
    fn red_alert_registry_contains_kind() {
        let registry = SystemKindRegistry::with_red_alert().unwrap();

        assert!(registry.contains(RED_ALERT_KIND));
    }

    /// The coarse `helm` / `tactical` kinds were deleted by #801: `"helm"`
    /// and `"tactical"` are station ids only. A ship TOML declaring either
    /// as a `[[system]]` kind must fail validation.
    #[test]
    fn coarse_helm_and_tactical_kinds_are_not_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(
            !registry.contains("helm"),
            "coarse `helm` must not be a registered system kind"
        );
        assert!(
            !registry.contains("tactical"),
            "coarse `tactical` must not be a registered system kind"
        );
    }

    /// Station-id keys are stable wire/blackboard strings — the client reads
    /// `blackboards['helm']` / `blackboards['tactical']`, so these must never
    /// drift.
    #[test]
    fn station_key_values_are_stable() {
        assert_eq!(HELM_STATION_ID, "helm");
        assert_eq!(TACTICAL_STATION_ID, "tactical");
        assert_eq!(helm_station_key().0, HELM_STATION_ID);
        assert_eq!(tactical_station_key().0, TACTICAL_STATION_ID);
    }

    #[test]
    fn core_registry_contains_all_coarse_kinds() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        for kind in [
            RED_ALERT_KIND,
            POWER_KIND,
            SENSORS_KIND,
            NAVIGATION_KIND,
            SHIELDS_KIND,
            COMMS_KIND,
            CAPTAIN_KIND,
            VIEWSCREEN_KIND,
            REPAIR_KIND,
        ] {
            assert!(
                registry.contains(kind),
                "coarse kind {kind:?} not registered"
            );
        }
    }

    // ── Fine Helm system tests (issue #511) ───────────────────────────────────

    #[test]
    fn fine_helm_kinds_are_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(
            registry.contains(HELM_JOYSTICK_KIND),
            "helm_joystick not registered"
        );
        assert!(
            registry.contains(HELM_ENGINE_KIND),
            "helm_engine not registered"
        );
        assert!(
            registry.contains(HELM_RADAR_KIND),
            "helm_radar not registered"
        );
        assert!(
            registry.contains(HELM_IMPULSE_KIND),
            "helm_impulse not registered"
        );
        assert!(
            registry.contains(LATERAL_THRUST_KIND),
            "lateral_thrust not registered"
        );
    }

    /// The per-axis Helm kinds (issue #701) must be registered like every
    /// other fine kind, or a ship TOML naming them fails `ShipConfig` parse.
    #[test]
    fn per_axis_helm_kinds_are_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(
            registry.contains(HELM_THRUST_KIND),
            "helm_thrust not registered"
        );
        assert!(
            registry.contains(HELM_STEERING_KIND),
            "helm_steering not registered"
        );
        assert!(
            registry.contains(HELM_BOOST_KIND),
            "helm_boost not registered"
        );
    }

    #[test]
    fn fine_helm_system_ids_are_lowercase_kebab() {
        let ids = [
            HELM_JOYSTICK_SYSTEM_ID,
            HELM_ENGINE_PORT_SYSTEM_ID,
            HELM_ENGINE_STARBOARD_SYSTEM_ID,
            HELM_RADAR_SYSTEM_ID,
            HELM_IMPULSE_SYSTEM_ID,
            LATERAL_THRUST_SYSTEM_ID,
            HELM_THRUST_SYSTEM_ID,
            HELM_STEERING_SYSTEM_ID,
            HELM_BOOST_SYSTEM_ID,
        ];
        for id in ids {
            assert_eq!(
                id,
                id.to_lowercase(),
                "Fine helm SystemId {id:?} is not lowercase"
            );
            assert!(
                !id.contains('_'),
                "Fine helm SystemId {id:?} contains underscore (use hyphen)"
            );
            assert!(!id.is_empty(), "Fine helm SystemId must not be empty");
        }
        assert_eq!(HELM_JOYSTICK_SYSTEM_ID, "helm-joystick");
        assert_eq!(HELM_ENGINE_PORT_SYSTEM_ID, "helm-engine-port");
        assert_eq!(HELM_ENGINE_STARBOARD_SYSTEM_ID, "helm-engine-starboard");
        assert_eq!(HELM_RADAR_SYSTEM_ID, "helm-radar");
        assert_eq!(HELM_IMPULSE_SYSTEM_ID, "helm-impulse");
        assert_eq!(LATERAL_THRUST_SYSTEM_ID, "helm-lateral-thrust");
        assert_eq!(HELM_THRUST_SYSTEM_ID, "helm-thrust");
        assert_eq!(HELM_STEERING_SYSTEM_ID, "helm-steering");
        assert_eq!(HELM_BOOST_SYSTEM_ID, "helm-boost");
    }

    #[test]
    fn fine_helm_system_id_helpers_return_expected_values() {
        assert_eq!(helm_joystick_system_id().0, HELM_JOYSTICK_SYSTEM_ID);
        assert_eq!(helm_engine_port_system_id().0, HELM_ENGINE_PORT_SYSTEM_ID);
        assert_eq!(
            helm_engine_starboard_system_id().0,
            HELM_ENGINE_STARBOARD_SYSTEM_ID
        );
        assert_eq!(helm_radar_system_id().0, HELM_RADAR_SYSTEM_ID);
        assert_eq!(helm_impulse_system_id().0, HELM_IMPULSE_SYSTEM_ID);
        assert_eq!(lateral_thrust_system_id().0, LATERAL_THRUST_SYSTEM_ID);
        assert_eq!(helm_thrust_system_id().0, HELM_THRUST_SYSTEM_ID);
        assert_eq!(helm_steering_system_id().0, HELM_STEERING_SYSTEM_ID);
        assert_eq!(helm_boost_system_id().0, HELM_BOOST_SYSTEM_ID);
    }

    // ── Fine Tactical system tests (issue #512) ───────────────────────────────

    #[test]
    fn fine_tactical_kinds_are_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(
            registry.contains(PHASER_BANK_KIND),
            "phaser_bank not registered"
        );
        assert!(
            registry.contains(TORPEDO_TUBE_KIND),
            "torpedo_tube not registered"
        );
        assert!(
            registry.contains(TORPEDO_MAGAZINE_KIND),
            "torpedo_magazine not registered"
        );
        assert!(
            registry.contains(PHASER_CONTROL_KIND),
            "phaser_control not registered"
        );
    }

    #[test]
    fn fine_tactical_system_ids_are_lowercase_kebab() {
        let ids = [
            PHASER_FORE_SYSTEM_ID,
            PHASER_AFT_SYSTEM_ID,
            TORPEDO_TUBE_FORE_PORT_SYSTEM_ID,
            TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID,
            TORPEDO_TUBE_AFT_SYSTEM_ID,
            TORPEDO_MAGAZINE_SYSTEM_ID,
            PHASER_CONTROL_SYSTEM_ID,
        ];
        for id in ids {
            assert_eq!(
                id,
                id.to_lowercase(),
                "Fine tactical SystemId {id:?} is not lowercase"
            );
            assert!(
                !id.contains('_'),
                "Fine tactical SystemId {id:?} contains underscore (use hyphen)"
            );
            assert!(!id.is_empty(), "Fine tactical SystemId must not be empty");
        }
        assert_eq!(PHASER_FORE_SYSTEM_ID, "phaser-fore");
        assert_eq!(PHASER_AFT_SYSTEM_ID, "phaser-aft");
        assert_eq!(TORPEDO_TUBE_FORE_PORT_SYSTEM_ID, "torpedo-tube-fore-port");
        assert_eq!(
            TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID,
            "torpedo-tube-fore-starboard"
        );
        assert_eq!(TORPEDO_TUBE_AFT_SYSTEM_ID, "torpedo-tube-aft");
        assert_eq!(TORPEDO_MAGAZINE_SYSTEM_ID, "torpedo-magazine");
        assert_eq!(PHASER_CONTROL_SYSTEM_ID, "phaser-control");
    }

    #[test]
    fn fine_tactical_system_id_helpers_return_expected_values() {
        assert_eq!(phaser_fore_system_id().0, PHASER_FORE_SYSTEM_ID);
        assert_eq!(phaser_aft_system_id().0, PHASER_AFT_SYSTEM_ID);
        assert_eq!(
            torpedo_tube_fore_port_system_id().0,
            TORPEDO_TUBE_FORE_PORT_SYSTEM_ID
        );
        assert_eq!(
            torpedo_tube_fore_starboard_system_id().0,
            TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID
        );
        assert_eq!(torpedo_tube_aft_system_id().0, TORPEDO_TUBE_AFT_SYSTEM_ID);
        assert_eq!(torpedo_magazine_system_id().0, TORPEDO_MAGAZINE_SYSTEM_ID);
        assert_eq!(phaser_control_system_id().0, PHASER_CONTROL_SYSTEM_ID);
    }

    #[test]
    fn phaser_bank_system_id_resolves_known_ids() {
        assert_eq!(phaser_bank_system_id("fore"), Some(phaser_fore_system_id()));
        assert_eq!(phaser_bank_system_id("aft"), Some(phaser_aft_system_id()));
        // Derives arbitrary bank ids via the `phaser-<id>` naming convention;
        // NPC ships that declare e.g. "port"/"starboard" get their own fine
        // SystemIds without needing a match-arm update.
        assert_eq!(
            phaser_bank_system_id("port"),
            Some(SystemId("phaser-port".into()))
        );
        assert_eq!(
            phaser_bank_system_id("starboard"),
            Some(SystemId("phaser-starboard".into()))
        );
        assert_eq!(phaser_bank_system_id(""), None);
    }

    #[test]
    fn torpedo_tube_system_id_resolves_known_ids() {
        assert_eq!(
            torpedo_tube_system_id("fore_port"),
            Some(torpedo_tube_fore_port_system_id())
        );
        assert_eq!(
            torpedo_tube_system_id("fore_starboard"),
            Some(torpedo_tube_fore_starboard_system_id())
        );
        assert_eq!(
            torpedo_tube_system_id("aft"),
            Some(torpedo_tube_aft_system_id())
        );
        // Underscore-to-hyphen conversion is the convention.
        assert_eq!(
            torpedo_tube_system_id("dorsal_upper"),
            Some(SystemId("torpedo-tube-dorsal-upper".into()))
        );
        assert_eq!(torpedo_tube_system_id(""), None);
    }

    // ── Fine Blaster system tests (issue #631) ────────────────────────────────

    #[test]
    fn blaster_bank_kind_is_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();
        assert!(
            registry.contains(BLASTER_BANK_KIND),
            "blaster_bank not registered"
        );
    }

    #[test]
    fn blaster_bank_kind_constant_is_correct() {
        assert_eq!(BLASTER_BANK_KIND, "blaster_bank");
    }

    #[test]
    fn blaster_bank_system_id_resolves_known_ids() {
        assert_eq!(
            blaster_bank_system_id("fore"),
            Some(SystemId("blaster-fore".into()))
        );
        assert_eq!(
            blaster_bank_system_id("aft"),
            Some(SystemId("blaster-aft".into()))
        );
        assert_eq!(
            blaster_bank_system_id("fore_port"),
            Some(SystemId("blaster-fore-port".into()))
        );
        assert_eq!(blaster_bank_system_id(""), None);
    }

    #[test]
    fn blaster_bank_system_ids_are_lowercase_kebab() {
        for bank_id in &["fore", "aft", "port", "starboard", "fore_port"] {
            let sid =
                blaster_bank_system_id(bank_id).expect("non-empty id should produce a SystemId");
            assert_eq!(
                sid.0,
                sid.0.to_lowercase(),
                "SystemId {sid:?} for blaster bank {bank_id:?} is not lowercase"
            );
            assert!(
                !sid.0.contains('_'),
                "SystemId {sid:?} for blaster bank {bank_id:?} contains underscore"
            );
            assert!(
                sid.0.starts_with("blaster-"),
                "SystemId {sid:?} must start with blaster-"
            );
        }
    }

    // ── Fine Tactical/Sensor Radar system tests ───────────────────────────────

    #[test]
    fn radar_kinds_are_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(
            registry.contains(TACTICAL_RADAR_KIND),
            "tactical_radar not registered"
        );
        assert!(
            registry.contains(SENSOR_RADAR_KIND),
            "sensor_radar not registered"
        );
    }

    #[test]
    fn radar_system_ids_are_lowercase_kebab() {
        let ids = [TACTICAL_RADAR_SYSTEM_ID, SENSOR_RADAR_SYSTEM_ID];
        for id in ids {
            assert_eq!(
                id,
                id.to_lowercase(),
                "Radar SystemId {id:?} is not lowercase"
            );
            assert!(
                !id.contains('_'),
                "Radar SystemId {id:?} contains underscore (use hyphen)"
            );
            assert!(!id.is_empty(), "Radar SystemId must not be empty");
        }
        assert_eq!(TACTICAL_RADAR_SYSTEM_ID, "tactical-radar");
        assert_eq!(SENSOR_RADAR_SYSTEM_ID, "sensor-radar");
    }

    #[test]
    fn radar_system_id_helpers_return_expected_values() {
        assert_eq!(tactical_radar_system_id().0, TACTICAL_RADAR_SYSTEM_ID);
        assert_eq!(sensor_radar_system_id().0, SENSOR_RADAR_SYSTEM_ID);
    }

    // ── Fine Power system tests (issue #513) ──────────────────────────────────

    #[test]
    fn fine_power_kinds_are_registered() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();

        assert!(
            registry.contains(POWER_REACTOR_KIND),
            "power_reactor not registered"
        );
        assert!(
            registry.contains(POWER_BATTERY_KIND),
            "power_battery not registered"
        );
    }

    #[test]
    fn fine_power_system_ids_are_lowercase_kebab() {
        let ids = [POWER_REACTOR_SYSTEM_ID, POWER_BATTERY_SYSTEM_ID];
        for id in ids {
            assert_eq!(
                id,
                id.to_lowercase(),
                "Fine power SystemId {id:?} is not lowercase"
            );
            assert!(
                !id.contains('_'),
                "Fine power SystemId {id:?} contains underscore (use hyphen)"
            );
            assert!(!id.is_empty(), "Fine power SystemId must not be empty");
        }
        assert_eq!(POWER_REACTOR_SYSTEM_ID, "power-reactor");
        assert_eq!(POWER_BATTERY_SYSTEM_ID, "power-battery");
    }

    #[test]
    fn fine_power_system_id_helpers_return_expected_values() {
        assert_eq!(power_reactor_system_id().0, POWER_REACTOR_SYSTEM_ID);
        assert_eq!(power_battery_system_id().0, POWER_BATTERY_SYSTEM_ID);
    }

    // ── Fine Shields system tests (issue #514) ────────────────────────────────

    #[test]
    fn shield_arcs_registered_in_system_kind_registry() {
        let registry = SystemKindRegistry::with_core_systems().unwrap();
        assert!(
            registry.contains(SHIELD_ARC_KIND),
            "shield_arc not registered"
        );
    }

    #[test]
    fn shield_arc_kind_string_is_lowercase_snake() {
        // Kind key uses snake_case per registry convention (matches
        // `phaser_bank`, `power_reactor` etc.).
        assert_eq!(SHIELD_ARC_KIND, "shield_arc");
    }

    #[test]
    fn shield_arc_system_id_helper_returns_expected_shape() {
        assert_eq!(
            shield_arc_system_id("fore"),
            Some(SystemId("shield-arc-fore".into()))
        );
        assert_eq!(
            shield_arc_system_id("port"),
            Some(SystemId("shield-arc-port".into()))
        );
        assert_eq!(
            shield_arc_system_id("aft"),
            Some(SystemId("shield-arc-aft".into()))
        );
        assert_eq!(
            shield_arc_system_id("starboard"),
            Some(SystemId("shield-arc-starboard".into()))
        );
        // Single-omni NPC arc id
        assert_eq!(
            shield_arc_system_id("all"),
            Some(SystemId("shield-arc-all".into()))
        );
        // Underscore-to-hyphen conversion is the convention.
        assert_eq!(
            shield_arc_system_id("dorsal_upper"),
            Some(SystemId("shield-arc-dorsal-upper".into()))
        );
        assert_eq!(shield_arc_system_id(""), None);
    }

    #[test]
    fn shield_arc_kebab_case_conformance() {
        // Every SystemId synthesised for shield arcs must be lowercase kebab.
        for arc_id in &["fore", "port", "aft", "starboard", "all", "dorsal_upper"] {
            let sid = shield_arc_system_id(arc_id).expect("non-empty id should synthesise");
            assert_eq!(
                sid.0,
                sid.0.to_lowercase(),
                "SystemId {sid:?} for arc {arc_id:?} is not lowercase"
            );
            assert!(
                !sid.0.contains('_'),
                "SystemId {sid:?} for arc {arc_id:?} contains underscore (use hyphen)"
            );
            assert!(
                sid.0.starts_with("shield-arc-"),
                "SystemId {sid:?} must start with shield-arc-"
            );
        }
    }
}
