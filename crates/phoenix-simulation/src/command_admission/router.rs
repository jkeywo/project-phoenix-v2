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
//!   ([`RegisterAdmittedConsumer::register_admitted_consumer`]), including the
//!   payloads and legacy spellings for which it supplies terminal feedback.
//! - Explicit correlations must have exactly one installed feedback owner;
//!   Admission refuses missing or ambiguous ownership before queue/log writes.
//! - [`warn_unrouted_admitted_commands`] runs after every consumer set and
//!   warns (never drops, never mutates) if an admitted command's target
//!   matches no registered consumer. It is warning-only: it changes no
//!   simulation state, so the headless behavioural gate stays bit-identical.
//!
//! The `InterSystemQueue` (inter-system Channel-2/3) is a separate bus
//! and is not covered by this registry.

use bevy::prelude::*;

use crate::core::messages::{
    AdmittedCommands, SystemControlPayload, SystemControlPayloadDiscriminants, SystemId,
};

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

    /// Declare the terminal-feedback promise beside its actual consumer.
    pub fn with_feedback(
        self,
        address: FeedbackAddress,
        payloads: &'static [SystemControlPayloadDiscriminants],
    ) -> ConsumerRegistration {
        assert!(
            matches!(
                (&self, address),
                (
                    Self::Exact { .. } | Self::Prefix { .. } | Self::UndeclaredExact(_),
                    FeedbackAddress::MatcherSpelling
                ) | (Self::Kind(_), FeedbackAddress::DeclaredKindOrCanonical(_))
                    | (
                        Self::Prefix { .. },
                        FeedbackAddress::LowercasePrefixSpelling
                    )
            ),
            "feedback address mode must match the consumer address domain"
        );
        ConsumerRegistration {
            matcher: self,
            feedback: Some(CorrelatedFeedback { address, payloads }),
        }
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

/// Compatibility spellings accepted by an installed terminal-feedback owner.
/// These are intentionally independent of command authority. In particular,
/// legacy canonical/generated spellings need not have an authored topology row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackAddress {
    MatcherSpelling,
    DeclaredKindOrCanonical(&'static str),
    LowercasePrefixSpelling,
}

#[derive(Clone, Debug)]
struct CorrelatedFeedback {
    address: FeedbackAddress,
    payloads: &'static [SystemControlPayloadDiscriminants],
}

impl CorrelatedFeedback {
    fn equivalent(&self, other: &Self) -> bool {
        self.address == other.address
            && self
                .payloads
                .iter()
                .all(|payload| other.payloads.contains(payload))
            && other
                .payloads
                .iter()
                .all(|payload| self.payloads.contains(payload))
    }
}

/// Registration metadata only: no callback or schedule is dispatched here.
#[derive(Clone, Debug)]
pub struct ConsumerRegistration {
    matcher: ConsumerMatcher,
    feedback: Option<CorrelatedFeedback>,
}

impl From<ConsumerMatcher> for ConsumerRegistration {
    fn from(matcher: ConsumerMatcher) -> Self {
        Self {
            matcher,
            feedback: None,
        }
    }
}

impl ConsumerRegistration {
    fn supports_feedback(
        &self,
        target: &SystemId,
        payload: &SystemControlPayload,
        config: Option<&crate::ship::config::ShipConfig>,
    ) -> bool {
        let Some(feedback) = &self.feedback else {
            return false;
        };
        if !feedback
            .payloads
            .contains(&SystemControlPayloadDiscriminants::from(payload))
        {
            return false;
        }
        match (&self.matcher, feedback.address) {
            (
                ConsumerMatcher::Exact { id, .. } | ConsumerMatcher::UndeclaredExact(id),
                FeedbackAddress::MatcherSpelling,
            ) => target.0 == *id,
            (ConsumerMatcher::Prefix { prefix, .. }, FeedbackAddress::MatcherSpelling) => {
                target.0.starts_with(prefix)
            }
            (ConsumerMatcher::Kind(kind), FeedbackAddress::DeclaredKindOrCanonical(canonical)) => {
                target.0 == canonical
                    || config
                        .and_then(|config| config.system(target))
                        .is_some_and(|system| system.kind == *kind)
            }
            (ConsumerMatcher::Prefix { prefix, .. }, FeedbackAddress::LowercasePrefixSpelling) => {
                target.0.strip_prefix(prefix).is_some_and(|suffix| {
                    !suffix.is_empty()
                        && suffix.bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                        })
                })
            }
            _ => unreachable!("feedback mode validated at registration"),
        }
    }
}

/// A correlation must have exactly one installed terminal owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackSupport {
    Unsupported,
    Supported,
    Ambiguous,
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
    registrations: Vec<ConsumerRegistration>,
}

impl AdmittedConsumerRegistry {
    /// Record that a consumer exists for `matcher`. Idempotent — registering
    /// the same matcher twice is a no-op, so a plugin added twice (test
    /// harnesses do this) cannot inflate the table.
    pub fn register(&mut self, registration: impl Into<ConsumerRegistration>) {
        let registration = registration.into();
        if let Some(existing) = self
            .registrations
            .iter()
            .find(|entry| entry.matcher == registration.matcher)
        {
            let identical = match (&existing.feedback, &registration.feedback) {
                (None, None) => true,
                (Some(left), Some(right)) => left.equivalent(right),
                _ => false,
            };
            assert!(
                identical,
                "contradictory feedback metadata for consumer {:?}",
                registration.matcher
            );
            return;
        }
        self.registrations.push(registration);
    }

    /// Read the promise without running a consumer or changing its schedule.
    pub fn feedback_support(
        &self,
        target: &SystemId,
        payload: &SystemControlPayload,
        config: &crate::ship::config::ShipConfig,
    ) -> FeedbackSupport {
        self.lookup_feedback(target, payload, Some(config))
    }

    /// Legacy spelling-only lookup for advisory activity, where authored
    /// uncorrelated controls historically counted as worked controls.
    pub(crate) fn legacy_feedback_support(
        &self,
        target: &SystemId,
        payload: &SystemControlPayload,
    ) -> FeedbackSupport {
        self.lookup_feedback(target, payload, None)
    }

    fn lookup_feedback(
        &self,
        target: &SystemId,
        payload: &SystemControlPayload,
        config: Option<&crate::ship::config::ShipConfig>,
    ) -> FeedbackSupport {
        let mut matching = self
            .registrations
            .iter()
            .filter(|entry| entry.supports_feedback(target, payload, config));
        match (matching.next(), matching.next()) {
            (None, _) => FeedbackSupport::Unsupported,
            (Some(_), None) => FeedbackSupport::Supported,
            (Some(_), Some(_)) => FeedbackSupport::Ambiguous,
        }
    }

    /// Does any registered consumer claim `target`?
    ///
    /// This raw-target form intentionally cannot resolve any declared-System
    /// matcher without ship topology. Runtime linting and descriptor coverage
    /// use [`Self::is_system_routed`] for declared Systems.
    pub fn is_routed(&self, target: &str) -> bool {
        self.registrations
            .iter()
            .any(|entry| entry.matcher.matches_target(target))
    }

    /// Does any registered consumer claim this authored System instance?
    pub fn is_system_routed(&self, system: &crate::ship::config::SystemInstanceConfig) -> bool {
        self.registrations
            .iter()
            .any(|entry| entry.matcher.matches_system(system))
    }

    /// Does production declare any consumer address domain for this System kind?
    pub fn claims_kind(&self, kind: &str) -> bool {
        self.registrations
            .iter()
            .any(|entry| entry.matcher.claims_kind(kind))
    }

    /// Number of distinct registered matchers (for coverage assertions).
    pub fn len(&self) -> usize {
        self.registrations.len()
    }

    /// Whether no consumer has registered yet.
    pub fn is_empty(&self) -> bool {
        self.registrations.is_empty()
    }
}

/// One-line registration API: `app.register_admitted_consumer(matcher)` in a
/// plugin's `build`. Initialises the registry resource on first use, so no
/// plugin needs to own the `init_resource` and ordering between plugin builds
/// does not matter.
pub trait RegisterAdmittedConsumer {
    /// Register a consumer matcher, returning `&mut Self` for chaining.
    fn register_admitted_consumer(
        &mut self,
        registration: impl Into<ConsumerRegistration>,
    ) -> &mut Self;
}

impl RegisterAdmittedConsumer for App {
    fn register_admitted_consumer(
        &mut self,
        registration: impl Into<ConsumerRegistration>,
    ) -> &mut Self {
        if !self.world().contains_resource::<AdmittedConsumerRegistry>() {
            self.init_resource::<AdmittedConsumerRegistry>();
        }
        self.world_mut()
            .resource_mut::<AdmittedConsumerRegistry>()
            .register(registration);
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
#[path = "router_tests.rs"]
mod tests;
