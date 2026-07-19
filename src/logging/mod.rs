//! Categorised, entity-filterable logging.
//!
//! Two dimensions of filtering, configured identically on both targets:
//!
//! * **Category** — a [`LogCat`] drawn from the module tree (`ai`, `helm`,
//!   `admit`, …), each with its own level. Spelled `ai=debug,admit=trace` in a
//!   log spec, which reads like `RUST_LOG` on purpose.
//! * **Entity** — an optional allow-list of entity display names. When set,
//!   only events tagged with a matching entity are emitted.
//!
//! # Why this isn't pure `tracing`
//!
//! Category alone would be a one-liner: put it in the event's `target:` and let
//! `EnvFilter` do the work. The entity filter is what forces a resource.
//! `tracing`'s `Layer::enabled()` is handed only `Metadata` — target, level,
//! file, line — and never the event's *fields*, so "only entities named
//! Ironveil" is not expressible as a `Filter`. Implementing it in `tracing`
//! means intercepting in `on_event` and therefore replacing the formatting
//! layer, separately for `tracing_subscriber::fmt` on native and `tracing-wasm`
//! in the browser.
//!
//! So instead: the category still travels as a `tracing` target (free, and
//! `RUST_LOG` keeps working), but both filters are checked against
//! [`LogFilterConfig`] *before* the `tracing` macro fires. `LogPlugin` is left
//! completely alone, which is what keeps browser console output working for
//! free.
//!
//! # Using it
//!
//! ```ignore
//! fn my_system(log: Option<Res<LogFilterConfig>>, q: Query<(Entity, &Hull)>) {
//!     for (e, hull) in &q {
//!         pdebug!(log, LogCat::Damage, entity = e, "hull now {}", hull.current);
//!     }
//! }
//! ```
//!
//! **Take `Option<Res<LogFilterConfig>>`, not `Res<LogFilterConfig>`.** A bare
//! `Res` fails Bevy's parameter validation in any app that never inserted the
//! resource, which is every bare-`App` unit test in this crate — so adding one
//! log line to a system would break every test that runs it. `None` falls back
//! to warn-level with no entity filtering. See [`AsLogFilter`].
//!
//! The macros are for *systems*. Plain helper functions with no config in scope
//! should keep using a bare `warn!(target: LogCat::Config.target(), ...)`
//! rather than growing a parameter for it.

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

mod filter;
mod macros;
mod spec;

pub use filter::{refresh_log_entity_filter, AsLogFilter, EntityFilter, LogFilterConfig};
pub use spec::{parse_log_entities, parse_log_spec, LogSpecError};

// `#[macro_export]` hoists these to the crate root. Re-exporting them here lets
// a call site pull the macro and the types it needs from one place:
// `use crate::logging::{pdebug, LogCat, LogFilterConfig};`
pub use crate::{pdebug, perror, pinfo, ptrace, pwarn};

/// Log category. The `strum` derives give us `"ai" <-> LogCat::Ai` in both
/// directions for free — parsing log specs and producing the `&'static str`
/// that becomes the `tracing` target.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Debug,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::EnumIter,
)]
#[strum(serialize_all = "lowercase")]
pub enum LogCat {
    /// `src/ai/*`, `src/console_ai/*` — objectives, doctrines, LOD, AI helm.
    Ai,
    /// `src/console/helm/`, `src/ship/{helm,impulse,lateral_thrust,boost}.rs`.
    Helm,
    /// `src/weapons/*`, `src/console/weapons/` — phasers, torpedoes, blasters.
    Weapons,
    /// `src/ship/shields.rs`, `src/weapons/shield.rs` — arcs, focus, frequency.
    Shields,
    /// `src/ship/damage.rs` — hull/system damage and tier transitions. Kept
    /// separate from `Weapons` because damage drives repair, and a
    /// damage-only trace is a thing you want often.
    Damage,
    /// `src/ship/power.rs`, `src/modifiers/power_system.rs`.
    Power,
    /// `src/ship/sensors.rs`.
    Sensors,
    /// `src/comms/`, `src/console/comms/`.
    Comms,
    /// `src/console/repair/`, `src/modifiers/repair_teams.rs`.
    Repair,
    /// `src/console/navigation/` — waypoints, routes.
    Nav,
    /// `src/console/captain/` — red alert, priority boosts.
    Captain,
    /// `src/lobby/*` — sessions, station claims, ratings, readiness.
    Lobby,
    /// `admit_system_commands` — the command-authority chokepoint. Its own
    /// category rather than part of `Lobby`: it is the single place that
    /// decides whether any command (human or AI) takes effect, and it already
    /// had an ad-hoc `[admit]` prefix convention worth formalising.
    Admit,
    /// `src/world/*` — triggers, scenario dispatch, entity spawning.
    World,
    /// `src/regions/*` — containment and region effects.
    Regions,
    /// Rapier, `integrate_ship_physics`, collisions.
    Physics,
    /// `SimOutbox`, `dispatch_sim_broadcasts`, codec encoding.
    Broadcast,
    /// `src/server/asset_preload.rs`.
    Assets,
    /// Entity/ship/world TOML parsing and validation.
    Config,
}

impl LogCat {
    /// The `tracing` target string for this category.
    ///
    /// Deliberately a hand-written `const fn` rather than the `strum`
    /// `Into<&'static str>` impl: `tracing` builds a `static __CALLSITE` from
    /// its `target:` argument, so the expression must be const-evaluable. That
    /// is also why the `plog!` macros interpolate `$cat.target()` directly
    /// instead of going through a `let` binding — and why `$cat` at a call site
    /// must be a literal variant such as `LogCat::Ai`, never a runtime
    /// variable.
    ///
    /// Kept in step with the `strum` derives by
    /// `target_matches_strum_serialisation` below.
    pub const fn target(self) -> &'static str {
        match self {
            Self::Ai => "ai",
            Self::Helm => "helm",
            Self::Weapons => "weapons",
            Self::Shields => "shields",
            Self::Damage => "damage",
            Self::Power => "power",
            Self::Sensors => "sensors",
            Self::Comms => "comms",
            Self::Repair => "repair",
            Self::Nav => "nav",
            Self::Captain => "captain",
            Self::Lobby => "lobby",
            Self::Admit => "admit",
            Self::World => "world",
            Self::Regions => "regions",
            Self::Physics => "physics",
            Self::Broadcast => "broadcast",
            Self::Assets => "assets",
            Self::Config => "config",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::IntoEnumIterator;

    /// `target()` is hand-written for const-ness; this is what stops it
    /// drifting from the `strum` serialisation the spec parser accepts.
    #[test]
    fn target_matches_strum_serialisation() {
        for cat in LogCat::iter() {
            let via_strum: &'static str = cat.into();
            assert_eq!(cat.target(), via_strum, "target() drifted for {cat:?}");
        }
    }
}

/// Per-category verbosity. Ordered so `Off < Error < ... < Trace`, which makes
/// "is this level enabled" a single comparison.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum LevelFilter {
    Off,
    Error,
    #[default]
    Warn,
    Info,
    Debug,
    Trace,
}

impl LevelFilter {
    /// Whether an event at `level` passes this filter.
    pub fn allows(self, level: LevelFilter) -> bool {
        level != LevelFilter::Off && level <= self
    }
}

/// Registers the logging resource and the entity-filter maintenance system.
///
/// Insert a configured [`LogFilterConfig`] *before* adding this plugin to
/// override the default (warn everywhere, no entity filter).
pub struct LoggingPlugin;

impl Plugin for LoggingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LogFilterConfig>().add_systems(
            PreUpdate,
            refresh_log_entity_filter.run_if(|cfg: Res<LogFilterConfig>| cfg.has_entity_filter()),
        );
    }
}

/// Convenience for constructing a config in tests and in the two front ends.
pub(crate) fn empty_per_cat() -> HashMap<LogCat, LevelFilter> {
    HashMap::new()
}

pub(crate) fn empty_entities() -> HashSet<Entity> {
    HashSet::new()
}
