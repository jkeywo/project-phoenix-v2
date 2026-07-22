//! The admitted-command consumer registry — the single table (issue #833)
//! answering "which module consumes commands for this `SystemId`?".
//!
//! ## Why a registry and not a runtime dispatcher
//!
//! By #833 the routing this table *names* has already landed: admission
//! ([`super::admit_system_commands`]) drains the inbound `ControlSystem`
//! stream exactly once per tick into the owning ship's per-entity
//! [`AdmittedCommands`], and every consumer reads only its own slice via
//! [`AdmittedCommands::for_target`]. There is no central `SystemId → module`
//! dispatch table (`system_target_for_payload_type` was deleted in #822) and
//! deliberately so — a runtime dispatch point would re-introduce the central
//! chokepoint the per-entity design removed and would collapse the consumer
//! scheduling (appliers run in different `SimSet`s: Input / Physics /
//! Modifiers / Broadcast).
//!
//! So this module is a *load-time registration seam* plus an *end-of-frame
//! lint*, not a dispatcher:
//!
//! - Each console/ship plugin registers its consumer(s) at `build` time with
//!   one line ([`RegisterAdmittedConsumer::register_admitted_consumer`]).
//! - [`warn_unrouted_admitted_commands`] runs after every consumer set and
//!   warns (never drops, never mutates) if an admitted command's target
//!   matches no registered consumer. It is warning-only: it changes no
//!   simulation state, so the headless behavioural gate stays bit-identical.
//!
//! The `InterSystemQueue` (inter-system Channel-2/3) is a separate bus
//! and is not covered by this registry.

use bevy::prelude::*;

use crate::messages::AdmittedCommands;

/// A matcher over `SystemId` strings identifying one registered consumer's
/// target family. Keying by *kind* or *prefix* (never a concrete dynamic id)
/// lets dynamic families — shield arcs — register once at plugin build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumerMatcher {
    /// An exact system id, e.g. `"power-reactor"`, `"sensors"`, `"helm-thrust"`.
    Exact(String),
    /// A dynamic family by id prefix, e.g. `"shield-arc-"` for every shield arc.
    Prefix(String),
}

impl ConsumerMatcher {
    /// Match a single, statically-named system id.
    pub fn exact(id: impl Into<String>) -> Self {
        Self::Exact(id.into())
    }

    /// Match every system id sharing a prefix (a dynamic system family).
    pub fn prefix(prefix: impl Into<String>) -> Self {
        Self::Prefix(prefix.into())
    }

    fn matches(&self, target: &str) -> bool {
        match self {
            Self::Exact(id) => target == id,
            Self::Prefix(prefix) => target.starts_with(prefix.as_str()),
        }
    }
}

/// The one table answering "which module handles commands for this system?".
///
/// Populated at app build: each consumer plugin registers the kind/prefix that
/// covers the `SystemId`(s) its applier reads. A command's target is *routed*
/// iff it matches a registered matcher.
#[derive(Resource, Default, Debug)]
pub struct AdmittedConsumerRegistry {
    matchers: Vec<ConsumerMatcher>,
}

impl AdmittedConsumerRegistry {
    /// Record that a consumer exists for `matcher`. Idempotent — registering
    /// the same matcher twice is a no-op, so a plugin added twice (test
    /// harnesses do this) cannot inflate the table.
    pub fn register(&mut self, matcher: ConsumerMatcher) {
        if !self.matchers.contains(&matcher) {
            self.matchers.push(matcher);
        }
    }

    /// Does any registered consumer claim `target`?
    pub fn is_routed(&self, target: &str) -> bool {
        self.matchers.iter().any(|m| m.matches(target))
    }

    /// Number of distinct registered matchers (for coverage assertions).
    pub fn len(&self) -> usize {
        self.matchers.len()
    }

    /// Whether no consumer has registered yet.
    pub fn is_empty(&self) -> bool {
        self.matchers.is_empty()
    }
}

/// One-line registration API: `app.register_admitted_consumer(matcher)` in a
/// plugin's `build`. Initialises the registry resource on first use, so no
/// plugin needs to own the `init_resource` and ordering between plugin builds
/// does not matter.
pub trait RegisterAdmittedConsumer {
    /// Register a consumer matcher, returning `&mut Self` for chaining.
    fn register_admitted_consumer(&mut self, matcher: ConsumerMatcher) -> &mut Self;
}

impl RegisterAdmittedConsumer for App {
    fn register_admitted_consumer(&mut self, matcher: ConsumerMatcher) -> &mut Self {
        if !self.world().contains_resource::<AdmittedConsumerRegistry>() {
            self.init_resource::<AdmittedConsumerRegistry>();
        }
        self.world_mut()
            .resource_mut::<AdmittedConsumerRegistry>()
            .register(matcher);
        self
    }
}

/// Pure core of the unrouted-command lint: the *distinct* admitted targets that
/// no registered consumer matches, in first-seen order.
///
/// Keys on the registry (no registered consumer), NOT on whether the command
/// changed state — a consumer may legitimately no-op a command for an offline
/// system, and that must not warn.
pub fn unrouted_command_targets<'a>(
    admitted: &'a AdmittedCommands,
    registry: &AdmittedConsumerRegistry,
) -> Vec<&'a str> {
    let mut out: Vec<&'a str> = Vec::new();
    for cmd in admitted.0.iter() {
        let target = cmd.target.0.as_str();
        if !registry.is_routed(target) && !out.contains(&target) {
            out.push(target);
        }
    }
    out
}

/// End-of-frame lint: for every ship, warn about any admitted command whose
/// target matches no registered consumer.
///
/// Ordering (`.after(SimSet::Broadcast)`, set in [`super::AdmissionPlugin`] and
/// the production `server_app` wiring): the consumer appliers run in the
/// Input / Physics / Modifiers / Broadcast sets, and the *next* tick's
/// [`super::admit_system_commands`] clears `AdmittedCommands` before
/// `SimSet::Input`. Running after Broadcast therefore observes the full tick's
/// admitted set while it is still populated.
///
/// **Dedupe decision:** `unrouted_command_targets` collapses duplicates within
/// a tick, so an unrouted target that a persistent client keeps sending warns
/// at most once per ship per tick (commands are cleared each tick, so cross-tick
/// repetition is inherent and accepted — an unrouted target is a wiring bug, not
/// steady-state traffic).
///
/// **Warning-only:** it mutates nothing; the headless gate stays bit-identical.
pub fn warn_unrouted_admitted_commands(
    ship_query: Query<(Entity, &AdmittedCommands)>,
    registry: Option<Res<AdmittedConsumerRegistry>>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    use crate::logging::LogCat;
    let Some(registry) = registry else {
        return;
    };
    for (entity, admitted) in ship_query.iter() {
        for target in unrouted_command_targets(admitted, &registry) {
            crate::pwarn!(
                log,
                LogCat::Admit,
                entity = entity,
                "admitted command for unrouted system {} has no registered consumer",
                target,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{AdmittedCommand, SystemControlPayload, SystemId};

    fn admitted(targets: &[&str]) -> AdmittedCommands {
        AdmittedCommands(
            targets
                .iter()
                .map(|t| AdmittedCommand {
                    target: SystemId((*t).into()),
                    payload: SystemControlPayload::ToggleRedAlert,
                    response_token: None,
                })
                .collect(),
        )
    }

    #[test]
    fn exact_matcher_routes_only_its_id() {
        let mut reg = AdmittedConsumerRegistry::default();
        reg.register(ConsumerMatcher::exact("sensors"));
        assert!(reg.is_routed("sensors"));
        assert!(!reg.is_routed("sensor-radar"));
        assert!(!reg.is_routed("power-reactor"));
    }

    #[test]
    fn prefix_matcher_routes_a_dynamic_family() {
        let mut reg = AdmittedConsumerRegistry::default();
        reg.register(ConsumerMatcher::prefix("shield-arc-"));
        assert!(reg.is_routed("shield-arc-fore"));
        assert!(reg.is_routed("shield-arc-aft-port"));
        assert!(!reg.is_routed("shields"));
    }

    #[test]
    fn register_is_idempotent() {
        let mut reg = AdmittedConsumerRegistry::default();
        reg.register(ConsumerMatcher::exact("comms"));
        reg.register(ConsumerMatcher::exact("comms"));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn unrouted_targets_reports_only_unregistered_and_dedupes() {
        let mut reg = AdmittedConsumerRegistry::default();
        reg.register(ConsumerMatcher::exact("sensors"));
        // Two "sensors" (registered) + two "ghost" (not) → only one "ghost".
        let cmds = admitted(&["sensors", "ghost", "sensors", "ghost"]);
        assert_eq!(unrouted_command_targets(&cmds, &reg), vec!["ghost"]);
    }

    #[test]
    fn a_fully_registered_tick_has_no_unrouted_targets() {
        let mut reg = AdmittedConsumerRegistry::default();
        reg.register(ConsumerMatcher::exact("sensors"));
        reg.register(ConsumerMatcher::exact("power-reactor"));
        let cmds = admitted(&["sensors", "power-reactor"]);
        assert!(unrouted_command_targets(&cmds, &reg).is_empty());
    }

    /// Builds an app with every admitted-command consumer plugin and reads back
    /// the assembled registry (the same distributed registration production
    /// performs).
    fn full_consumer_registry_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::captain_plugin::CaptainPlugin)
            .add_plugins(crate::comms::CommsWorldPlugin)
            .add_plugins(crate::navigation_plugin::NavigationPlugin)
            .add_plugins(crate::shields_plugin::ShipShieldsPlugin)
            .add_plugins(crate::sensors_plugin::ShipSensorsPlugin)
            .add_plugins(crate::power_plugin::ShipPowerPlugin)
            .add_plugins(crate::ship_plugin::ShipPlugin)
            .add_plugins(crate::weapons_plugin::WeaponsPlugin)
            .add_plugins(crate::repair_plugin::RepairPlugin);
        app
    }

    /// The "one table" invariant, pinned as an executable coverage census:
    /// every admitted-command-capable system kind resolves to a registered
    /// consumer once the full app is built. This is the inverse of the §2
    /// consumer census — each `for_target` / direct-iter admitted consumer that
    /// landed in #824–#832 must have exactly one registration here.
    #[test]
    fn every_admitted_consumer_kind_is_registered() {
        use crate::system_registry as sr;
        let app = full_consumer_registry_app();
        let reg = app.world().resource::<AdmittedConsumerRegistry>();

        // One representative admitted target per consumer.
        let expected: &[&str] = &[
            sr::RED_ALERT_SYSTEM_ID,      // captain: handle_toggle_red_alert
            sr::CAPTAIN_SYSTEM_ID,        // captain: handle_set_objective_priority
            sr::VIEWSCREEN_SYSTEM_ID,     // captain: handle_set_view
            sr::COMMS_SYSTEM_ID,          // comms: handle_hail/respond/clear
            sr::NAVIGATION_SYSTEM_ID,     // navigation: handle_navigation_waypoint
            sr::SENSORS_SYSTEM_ID,        // sensors: handle_sensors_messages
            sr::POWER_REACTOR_SYSTEM_ID,  // power: handle_power_messages
            sr::HELM_THRUST_SYSTEM_ID,    // helm: process_helm_inputs
            sr::HELM_STEERING_SYSTEM_ID,  // helm: process_helm_inputs
            sr::HELM_IMPULSE_SYSTEM_ID,   // helm: process_helm_inputs
            sr::LATERAL_THRUST_SYSTEM_ID, // helm: process_helm_inputs
            sr::HELM_BOOST_SYSTEM_ID,     // helm: handle_boost_messages
            sr::TACTICAL_RADAR_SYSTEM_ID, // weapons: tactical radar selection
            sr::PHASER_CONTROL_SYSTEM_ID, // weapons: phaser control
            sr::REPAIR_SYSTEM_ID,         // repair: handle_dispatch_repair_team
            // weapons fire/load (issue #846): every phaser bank and torpedo tube
            // is a routed admitted consumer — no more legacy ClientMessage variants.
            sr::PHASER_FORE_SYSTEM_ID,
            sr::PHASER_AFT_SYSTEM_ID,
            sr::TORPEDO_TUBE_FORE_PORT_SYSTEM_ID,
            sr::TORPEDO_TUBE_FORE_STARBOARD_SYSTEM_ID,
            sr::TORPEDO_TUBE_AFT_SYSTEM_ID,
        ];
        for id in expected {
            assert!(
                reg.is_routed(id),
                "admitted-command system `{id}` has no registered consumer — \
                 add `app.register_admitted_consumer(...)` in its plugin's build"
            );
        }

        // The dynamic shield-arc family (shields: handle_shields_messages).
        let arc = sr::shield_arc_system_id("fore").unwrap();
        assert!(reg.is_routed(&arc.0), "shield-arc family not registered");
    }

    /// Every weapons fire/load system id is now a routed admitted consumer
    /// (issue #846): the legacy top-level `ClientMessage` variants have been
    /// deleted and all weapons commands travel as `ControlSystem` messages.
    #[test]
    fn fire_weapon_ids_are_registered_as_admitted_consumers() {
        let app = full_consumer_registry_app();
        let reg = app.world().resource::<AdmittedConsumerRegistry>();
        assert!(reg.is_routed(crate::system_registry::PHASER_FORE_SYSTEM_ID));
        assert!(reg.is_routed(crate::system_registry::PHASER_AFT_SYSTEM_ID));
        assert!(reg.is_routed(crate::system_registry::TORPEDO_TUBE_FORE_PORT_SYSTEM_ID));
        assert!(reg.is_routed(crate::system_registry::TORPEDO_TUBE_AFT_SYSTEM_ID));
        assert!(!reg.is_routed("no-such-system"));
    }

    /// `handle_set_torpedo_volley_target` reads per-ship `AdmittedCommands`, and
    /// `ai_torpedo_load` emits the order through `ai_emit` — so the tube ids must
    /// route, or the unrouted lint `pwarn!`s on every AI volley order.
    #[test]
    fn torpedo_tube_ids_are_registered_as_admitted_consumers() {
        let app = full_consumer_registry_app();
        let reg = app.world().resource::<AdmittedConsumerRegistry>();
        assert!(
            reg.is_routed(crate::system_registry::TORPEDO_TUBE_FORE_PORT_SYSTEM_ID),
            "torpedo tube ids must be routed — the volley order travels admitted"
        );
        // The prefix matcher must cover every authored per-hull tube id, not
        // just the one constant.
        assert!(reg.is_routed("torpedo-tube-aft"));
    }

    /// The unrouted-lint decision keys on the registry: an admitted command for
    /// a known-but-unregistered system id is flagged (would `pwarn!`), while a
    /// registered one is not — and the lint mutates nothing.
    ///
    /// (Note: `cargo test` installs no `tracing` subscriber, so the `pwarn!`
    /// text itself cannot be captured — see `logging::macros` tests. The
    /// decision that drives the warn is pinned here directly via the pure
    /// `unrouted_command_targets`, and the full Bevy system is exercised for
    /// no-panic / no-mutation below.)
    #[test]
    fn unrouted_lint_flags_unregistered_but_not_registered_ids() {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin);
        // Register just one consumer; "ghost-system" is deliberately absent.
        app.register_admitted_consumer(ConsumerMatcher::exact("sensors"));

        let before = admitted(&["sensors", "ghost-system"]);
        let ship = app
            .world_mut()
            .spawn(AdmittedCommands(before.0.clone()))
            .id();

        // Decision the lint acts on.
        let reg = app.world().resource::<AdmittedConsumerRegistry>();
        let stored = app.world().entity(ship).get::<AdmittedCommands>().unwrap();
        assert_eq!(unrouted_command_targets(stored, reg), vec!["ghost-system"]);

        // Run the real lint system: it must not panic and must not mutate the
        // admitted set (warning-only).
        app.world_mut()
            .run_system_cached(warn_unrouted_admitted_commands)
            .unwrap();
        let after = app.world().entity(ship).get::<AdmittedCommands>().unwrap();
        assert_eq!(after.0.len(), 2, "the lint must not drop admitted commands");
    }
}
