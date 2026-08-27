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
//! - Each console/ship plugin registers its consumer address domain at `build`
//!   time with one line
//!   ([`RegisterAdmittedConsumer::register_admitted_consumer`]).
//! - [`warn_unrouted_admitted_commands`] runs after every consumer set and
//!   warns (never drops, never mutates) if an admitted command's target
//!   matches no registered consumer. It is warning-only: it changes no
//!   simulation state, so the headless behavioural gate stays bit-identical.
//!
//! The `InterSystemQueue` (inter-system Channel-2/3) is a separate bus
//! and is not covered by this registry.

use bevy::prelude::*;

use crate::core::messages::AdmittedCommands;

/// A matcher identifying one registered admitted-command consumer.
///
/// Every declared-System matcher names the authoritative System kind *and* the
/// address domain the real consumer reads: one canonical id, one generated-id
/// prefix, or any authored instance of that kind. Keeping both dimensions stops
/// registration metadata from claiming an arbitrary id that a fixed-id handler
/// would silently ignore. Undeclared host-only capabilities (`god-mode`) retain
/// a separate exact-target form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumerMatcher {
    /// Every authored instance of a System kind, e.g. Dock, whose runtime
    /// component carries and reads its authored System id.
    Kind(String),
    /// One canonical declared System id.
    Exact { kind: String, id: String },
    /// A generated declared-System id family.
    Prefix { kind: String, prefix: String },
    /// One exact undeclared host capability id, e.g. `"god-mode"`.
    UndeclaredExact(String),
}

impl ConsumerMatcher {
    /// Match every authored instance carrying `kind` in ship topology.
    pub fn kind(kind: impl Into<String>) -> Self {
        Self::Kind(kind.into())
    }

    /// Match one canonical id belonging to `kind`.
    pub fn exact(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self::Exact {
            kind: kind.into(),
            id: id.into(),
        }
    }

    /// Match the generated ids beginning with `prefix` that belong to `kind`.
    pub fn prefix(kind: impl Into<String>, prefix: impl Into<String>) -> Self {
        Self::Prefix {
            kind: kind.into(),
            prefix: prefix.into(),
        }
    }

    /// Match one target that is deliberately absent from ship topology.
    pub fn undeclared_exact(id: impl Into<String>) -> Self {
        Self::UndeclaredExact(id.into())
    }

    fn matches_target(&self, target: &str) -> bool {
        match self {
            Self::UndeclaredExact(id) => target == id,
            Self::Kind(_) | Self::Exact { .. } | Self::Prefix { .. } => false,
        }
    }

    fn matches_system(&self, system: &crate::ship::config::SystemInstanceConfig) -> bool {
        match self {
            Self::Kind(kind) => system.kind == *kind,
            Self::Exact { kind, id } => system.kind == *kind && system.id.0 == *id,
            Self::Prefix { kind, prefix } => {
                system.kind == *kind && system.id.0.starts_with(prefix)
            }
            Self::UndeclaredExact(_) => false,
        }
    }

    fn claims_kind(&self, candidate: &str) -> bool {
        match self {
            Self::Kind(kind) | Self::Exact { kind, .. } | Self::Prefix { kind, .. } => {
                kind == candidate
            }
            Self::UndeclaredExact(_) => false,
        }
    }
}

/// The one table answering "which module handles commands for this system?".
///
/// Populated at app build: each consumer plugin registers the System kind and
/// address domain its applier reads. A declared command target is
/// *routed* iff its resolved [`crate::ship::config::SystemInstanceConfig`]
/// matches a registered matcher. Undeclared host capabilities use exact target
/// matching instead.
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
    ///
    /// This raw-target form intentionally cannot resolve any declared-System
    /// matcher without ship topology. Runtime linting and descriptor coverage
    /// use [`Self::is_system_routed`] for declared Systems.
    pub fn is_routed(&self, target: &str) -> bool {
        self.matchers.iter().any(|m| m.matches_target(target))
    }

    /// Does any registered consumer claim this authored System instance?
    pub fn is_system_routed(&self, system: &crate::ship::config::SystemInstanceConfig) -> bool {
        self.matchers.iter().any(|m| m.matches_system(system))
    }

    /// Does production declare any consumer address domain for this System kind?
    pub fn claims_kind(&self, kind: &str) -> bool {
        self.matchers
            .iter()
            .any(|matcher| matcher.claims_kind(kind))
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
    ship_config: Option<&crate::ship::config::ShipConfig>,
    registry: &AdmittedConsumerRegistry,
) -> Vec<&'a str> {
    let mut out: Vec<&'a str> = Vec::new();
    for cmd in admitted.0.iter() {
        let target = cmd.target.0.as_str();
        let routed = ship_config
            .and_then(|config| config.systems.iter().find(|system| system.id.0 == target))
            .map_or_else(
                || registry.is_routed(target),
                |system| registry.is_system_routed(system),
            );
        if !routed && !out.contains(&target) {
            out.push(target);
        }
    }
    out
}

/// Every commandable authored System instance for which production registered
/// no consumer.
///
/// Commandability comes only from [`crate::ship::system_registry::SystemKindDescriptor`].
/// Passive capabilities are ignored even when no consumer matcher claims them,
/// so the coverage guard cannot turn read-only topology into false failures.
pub fn unrouted_commandable_systems<'a>(
    systems: &'a [crate::ship::config::SystemInstanceConfig],
    descriptors: &crate::ship::system_registry::SystemKindRegistry,
    consumers: &AdmittedConsumerRegistry,
) -> Vec<&'a crate::ship::config::SystemInstanceConfig> {
    systems
        .iter()
        .filter(|system| {
            descriptors
                .descriptor(&system.kind)
                .is_some_and(|descriptor| descriptor.accepts_admitted_commands())
                && !consumers.is_system_routed(system)
        })
        .collect()
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
    ship_query: Query<(
        Entity,
        &AdmittedCommands,
        Option<&crate::ship_plugin::ShipConfigComponent>,
    )>,
    registry: Option<Res<AdmittedConsumerRegistry>>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
) {
    use crate::logging::LogCat;
    let Some(registry) = registry else {
        return;
    };
    for (entity, admitted, ship_config) in ship_query.iter() {
        for target in
            unrouted_command_targets(admitted, ship_config.map(|config| &config.0), &registry)
        {
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
    use crate::core::messages::{AdmittedCommand, SystemControlPayload, SystemId};

    fn system(id: &str, kind: &str) -> crate::ship::config::SystemInstanceConfig {
        crate::ship::config::SystemInstanceConfig {
            id: SystemId(id.into()),
            kind: kind.into(),
            station: None,
            ai_only: true,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        }
    }

    fn ship_config(
        systems: Vec<crate::ship::config::SystemInstanceConfig>,
    ) -> crate::ship::config::ShipConfig {
        crate::ship::config::ShipConfig {
            stations: Vec::new(),
            systems,
            power_groups: std::collections::HashMap::new(),
            coordination_lag_secs: 0.0,
        }
    }

    fn admitted(targets: &[&str]) -> AdmittedCommands {
        AdmittedCommands(
            targets
                .iter()
                .map(|t| AdmittedCommand {
                    target: SystemId((*t).into()),
                    payload: SystemControlPayload::SetRedAlert { active: true },
                    response_token: None,
                })
                .collect(),
        )
    }

    #[test]
    fn undeclared_exact_matcher_routes_only_its_id() {
        let mut reg = AdmittedConsumerRegistry::default();
        reg.register(ConsumerMatcher::undeclared_exact("god-mode"));
        assert!(reg.is_routed("god-mode"));
        assert!(!reg.is_routed("god-mode-extra"));
        assert!(!reg.is_system_routed(&system("god-mode", "debug")));
    }

    #[test]
    fn kind_matcher_routes_arbitrary_instance_ids() {
        let mut reg = AdmittedConsumerRegistry::default();
        reg.register(ConsumerMatcher::kind(
            crate::ship::system_registry::DOCK_KIND,
        ));

        assert!(reg.is_system_routed(&system(
            "berthing-clamps",
            crate::ship::system_registry::DOCK_KIND,
        )));
        assert!(
            !reg.is_routed("berthing-clamps"),
            "raw target matching must not infer a System kind without topology"
        );
    }

    #[test]
    fn register_is_idempotent() {
        let mut reg = AdmittedConsumerRegistry::default();
        reg.register(ConsumerMatcher::exact("comms", "comms"));
        reg.register(ConsumerMatcher::exact("comms", "comms"));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn unrouted_targets_reports_only_unregistered_and_dedupes() {
        let mut reg = AdmittedConsumerRegistry::default();
        reg.register(ConsumerMatcher::undeclared_exact("sensors"));
        // Two "sensors" (registered) + two "ghost" (not) → only one "ghost".
        let cmds = admitted(&["sensors", "ghost", "sensors", "ghost"]);
        assert_eq!(unrouted_command_targets(&cmds, None, &reg), vec!["ghost"]);
    }

    #[test]
    fn a_fully_registered_tick_has_no_unrouted_targets() {
        let mut reg = AdmittedConsumerRegistry::default();
        reg.register(ConsumerMatcher::undeclared_exact("sensors"));
        reg.register(ConsumerMatcher::undeclared_exact("power-reactor"));
        let cmds = admitted(&["sensors", "power-reactor"]);
        assert!(unrouted_command_targets(&cmds, None, &reg).is_empty());
    }

    /// Build through the same two registration entry points production uses:
    /// canonical simulation composition, then the World plugin that owns the
    /// Tractor/Dock/Umbilical family.
    fn production_consumer_registry_app() -> App {
        let mut app = App::new();
        crate::server_app::add_simulation_plugins_with(
            &mut app,
            crate::server_app::SimPluginOptions {
                render: false,
                ..Default::default()
            },
        );
        app.add_plugins(crate::world::server::WorldPlugin);
        app
    }

    /// Every top-level `assets/entities/*.toml` ship, resolved through the
    /// production include loader. Subdirectories are fixtures/fragments rather
    /// than shipped hulls (the same fleet boundary as the other authored-content
    /// censuses).
    fn shipped_ship_configs() -> Vec<(String, crate::ship::config::ShipConfig)> {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/entities");
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .expect("assets/entities must be readable")
            .map(|entry| entry.expect("readable directory entry").path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "toml")
            })
            .collect();
        entries.sort();

        entries
            .into_iter()
            .filter_map(|path| {
                let stem = path
                    .file_stem()
                    .expect("TOML path has a stem")
                    .to_string_lossy()
                    .to_string();
                let key = path.to_string_lossy().replace('\\', "/");
                let config = crate::entities::include_resolve::load_entity_config(&key)
                    .unwrap_or_else(|error| panic!("{stem} must load: {error}"));
                config.ship_config.map(|ship| (stem, ship))
            })
            .collect()
    }

    /// The descriptor-derived completeness invariant. There is no expected-id
    /// fixture here: every shipped System instance whose authoritative kind
    /// descriptor says it accepts admitted commands must be claimed by the
    /// registry assembled through production composition.
    #[test]
    fn every_commandable_shipped_system_has_a_production_consumer() {
        use crate::ship::system_registry as sr;

        let app = production_consumer_registry_app();
        let consumers = app.world().resource::<AdmittedConsumerRegistry>();
        let descriptors = sr::SystemKindRegistry::with_core_systems().unwrap();
        let mut covered_kinds = std::collections::BTreeSet::new();
        let mut missing = Vec::new();

        // Descriptor completeness does not depend on a kind already having a
        // shipped instance. This checks only that production names an address
        // domain for every commandable descriptor; the shipped-hull census
        // below separately proves each real instance falls inside that domain.
        let missing_descriptor_kinds: Vec<_> = descriptors
            .kinds()
            .filter(|kind| {
                descriptors
                    .descriptor(kind)
                    .is_some_and(|descriptor| descriptor.accepts_admitted_commands())
                    && !consumers.claims_kind(kind)
            })
            .collect();
        assert!(
            missing_descriptor_kinds.is_empty(),
            "commandable descriptors without a production consumer: {}",
            missing_descriptor_kinds.join(", ")
        );

        for (hull, config) in shipped_ship_configs() {
            for system in &config.systems {
                let descriptor = descriptors
                    .descriptor(&system.kind)
                    .unwrap_or_else(|| panic!("{hull}: unknown System kind `{}`", system.kind));
                if descriptor.accepts_admitted_commands() {
                    covered_kinds.insert(system.kind.clone());
                }
            }
            for system in unrouted_commandable_systems(&config.systems, &descriptors, consumers) {
                missing.push(format!("{hull}: {} (kind {})", system.id.0, system.kind));
            }
        }

        assert!(
            missing.is_empty(),
            "commandable Systems without a production consumer:\n{}",
            missing.join("\n")
        );

        // Acceptance coverage guard: these are kinds, not representative ids.
        // A content edit that removed the last real instance would otherwise
        // make this issue's named dynamic/auxiliary cases vacuous.
        for required in [
            sr::COMMAND_KIND,
            sr::TRACTOR_KIND,
            sr::DOCK_KIND,
            sr::UMBILICAL_KIND,
            sr::PHASER_BANK_KIND,
            sr::BLASTER_BANK_KIND,
            sr::TORPEDO_TUBE_KIND,
            sr::SHIELD_ARC_KIND,
        ] {
            assert!(
                covered_kinds.contains(required),
                "the shipped-hull census did not exercise commandable kind `{required}`"
            );
        }
    }

    #[test]
    fn missing_commandable_consumer_is_detected_but_passive_kind_is_ignored() {
        let mut descriptors = crate::ship::system_registry::SystemKindRegistry::new();
        descriptors
            .register_commandable(
                "test_commandable",
                crate::core::messages::ConsoleFamily::Command,
            )
            .unwrap();
        descriptors
            .register(
                "test_passive",
                crate::core::messages::ConsoleFamily::Command,
            )
            .unwrap();
        let systems = vec![
            system("orders", "test_commandable"),
            system("telemetry", "test_passive"),
        ];
        let mut consumers = AdmittedConsumerRegistry::default();

        let missing = unrouted_commandable_systems(&systems, &descriptors, &consumers);
        assert_eq!(
            missing
                .iter()
                .map(|system| system.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["orders"],
            "a passive capability must not become a false coverage failure"
        );

        consumers.register(ConsumerMatcher::kind("test_commandable"));
        assert!(unrouted_commandable_systems(&systems, &descriptors, &consumers).is_empty());
    }

    #[test]
    fn fixed_target_domain_rejects_noncanonical_instance_but_dock_accepts_arbitrary_id() {
        let app = production_consumer_registry_app();
        let consumers = app.world().resource::<AdmittedConsumerRegistry>();
        let descriptors = crate::ship::system_registry::SystemKindRegistry::with_core_systems()
            .expect("core descriptors");
        let config = ship_config(vec![
            system("tow-a", crate::ship::system_registry::TRACTOR_KIND),
            system("berthing-clamps", crate::ship::system_registry::DOCK_KIND),
        ]);

        let commands = admitted(&["tow-a", "berthing-clamps"]);
        assert_eq!(
            unrouted_command_targets(&commands, Some(&config), consumers),
            vec!["tow-a"],
            "Tractor reads its canonical id, while Dock reads its authored component id"
        );
        assert_eq!(
            unrouted_commandable_systems(&config.systems, &descriptors, consumers)
                .into_iter()
                .map(|system| system.id.0.as_str())
                .collect::<Vec<_>>(),
            vec!["tow-a"]
        );
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
        // Register Dock, whose consumer really reads the arbitrary authored id
        // stored in its runtime component. "ghost-system" is absent from both.
        app.register_admitted_consumer(ConsumerMatcher::kind(
            crate::ship::system_registry::DOCK_KIND,
        ));

        let config = ship_config(vec![system(
            "berthing-clamps",
            crate::ship::system_registry::DOCK_KIND,
        )]);
        let before = admitted(&["berthing-clamps", "ghost-system"]);
        let ship = app
            .world_mut()
            .spawn((
                AdmittedCommands(before.0.clone()),
                crate::ship_plugin::ShipConfigComponent(config.clone()),
            ))
            .id();

        // Decision the lint acts on.
        let reg = app.world().resource::<AdmittedConsumerRegistry>();
        let stored = app.world().entity(ship).get::<AdmittedCommands>().unwrap();
        assert_eq!(
            unrouted_command_targets(stored, Some(&config), reg),
            vec!["ghost-system"]
        );

        // Run the real lint system: it must not panic and must not mutate the
        // admitted set (warning-only).
        app.world_mut()
            .run_system_cached(warn_unrouted_admitted_commands)
            .unwrap();
        let after = app.world().entity(ship).get::<AdmittedCommands>().unwrap();
        assert_eq!(after.0.len(), 2, "the lint must not drop admitted commands");
    }
}
