//! The runtime filter resource and the system that keeps its entity set fresh.

use super::{LevelFilter, LogCat};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

/// The resolved entity allow-list.
///
/// `names` is what the operator authored (`--log-entity Ironveil`); `allowed`
/// is the set of entities those names currently resolve to. Resolution runs
/// *backwards* — names to entities, once, on spawn — so that the hot path (a
/// log call site holding an `Entity`) is a single hash lookup rather than a
/// component fetch plus a string compare.
#[derive(Clone, Debug)]
pub struct EntityFilter {
    /// Names as authored. Matched exactly first, then case-insensitively as a
    /// substring, so `--log-entity ironveil` finds `"Ironveil"`.
    pub names: Vec<String>,
    /// Entities currently matching any of `names`.
    pub allowed: HashSet<Entity>,
}

impl EntityFilter {
    pub fn new(names: Vec<String>) -> Self {
        Self {
            names,
            allowed: HashSet::new(),
        }
    }

    /// Whether `name` matches any configured pattern.
    pub fn matches_name(&self, name: &str) -> bool {
        self.names
            .iter()
            .any(|pat| pat == name || name.to_lowercase().contains(&pat.to_lowercase()))
    }
}

/// Runtime log filtering state. Read by the `plog!` family before any
/// formatting happens.
#[derive(Resource, Clone, Debug)]
pub struct LogFilterConfig {
    /// Level applied to categories with no explicit entry.
    pub default_level: LevelFilter,
    /// Per-category overrides.
    pub per_cat: HashMap<LogCat, LevelFilter>,
    /// `None` means no entity filtering at all — every event passes. This is
    /// the default and the only case that matters for production perf.
    pub entity_filter: Option<EntityFilter>,
}

impl Default for LogFilterConfig {
    fn default() -> Self {
        Self {
            default_level: LevelFilter::Warn,
            per_cat: HashMap::new(),
            entity_filter: None,
        }
    }
}

impl LogFilterConfig {
    /// Whether `cat` emits at `level`.
    #[inline]
    pub fn cat_enabled(&self, cat: LogCat, level: LevelFilter) -> bool {
        self.per_cat
            .get(&cat)
            .copied()
            .unwrap_or(self.default_level)
            .allows(level)
    }

    /// Whether events tagged with `entity` pass the entity filter. Always true
    /// when no filter is configured.
    #[inline]
    pub fn entity_allowed(&self, entity: Entity) -> bool {
        match &self.entity_filter {
            None => true,
            Some(f) => f.allowed.contains(&entity),
        }
    }

    pub fn has_entity_filter(&self) -> bool {
        self.entity_filter.is_some()
    }
}

/// The config used when a system has no `LogFilterConfig` available.
///
/// Plain `LogFilterConfig::default()` — warn level, no entity filter — but as a
/// `'static` so [`AsLogFilter`] can hand out a reference.
fn fallback() -> &'static LogFilterConfig {
    static FALLBACK: std::sync::OnceLock<LogFilterConfig> = std::sync::OnceLock::new();
    FALLBACK.get_or_init(LogFilterConfig::default)
}

/// Lets the `plog!` macros accept whatever shape a call site has the config in.
///
/// The `Option<Res<_>>` impl is the important one. Systems that take a bare
/// `Res<LogFilterConfig>` fail parameter validation in any app that never
/// inserted the resource — which is every bare-`App` unit test in this crate,
/// and there are hundreds of them. Adding one log line to a system would
/// otherwise break every test that runs it, so the documented call-site
/// signature is `Option<Res<LogFilterConfig>>` and a `None` falls back to
/// warn-level with no entity filtering.
pub trait AsLogFilter {
    fn log_filter(&self) -> &LogFilterConfig;
}

impl AsLogFilter for LogFilterConfig {
    fn log_filter(&self) -> &LogFilterConfig {
        self
    }
}

impl<T: AsLogFilter + ?Sized> AsLogFilter for &T {
    fn log_filter(&self) -> &LogFilterConfig {
        (**self).log_filter()
    }
}

impl AsLogFilter for Res<'_, LogFilterConfig> {
    fn log_filter(&self) -> &LogFilterConfig {
        self
    }
}

impl AsLogFilter for Option<Res<'_, LogFilterConfig>> {
    fn log_filter(&self) -> &LogFilterConfig {
        // Written as a match rather than `as_deref().unwrap_or_else(fallback)`:
        // the latter unifies the borrow with the `'static` fallback and the
        // compiler then demands `&self` outlive `'static`.
        match self {
            Some(res) => res,
            None => fallback(),
        }
    }
}

/// The shape an **exclusive** system has the config in.
///
/// A `&mut World` system cannot hold a `Res` borrow across the mutations that
/// are the reason it is exclusive, so it does
/// `world.get_resource::<LogFilterConfig>().cloned()` once at the top and logs
/// through that. Same `None` semantics as the `Option<Res<_>>` impl above —
/// warn-level, no entity filtering — so the two call-site shapes cannot drift.
impl AsLogFilter for Option<LogFilterConfig> {
    fn log_filter(&self) -> &LogFilterConfig {
        match self {
            Some(cfg) => cfg,
            None => fallback(),
        }
    }
}

/// The shape a HELPER called out of a system has the config in.
///
/// A system holding `Option<Res<LogFilterConfig>>` hands a helper
/// `log.as_deref()` rather than the `Res` itself, because the helper has no
/// business naming a Bevy type to log a line. Same `None` semantics as the two
/// impls above — warn-level, no entity filtering — so a helper's logging cannot
/// drift from its caller's.
impl AsLogFilter for Option<&LogFilterConfig> {
    fn log_filter(&self) -> &LogFilterConfig {
        match self {
            Some(cfg) => cfg,
            None => fallback(),
        }
    }
}
