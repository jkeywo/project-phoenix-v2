//! The runtime filter resource and the system that keeps its entity set fresh.

use super::{empty_entities, empty_per_cat, LevelFilter, LogCat};
use crate::entities::spawner::EntityName;
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
            allowed: empty_entities(),
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
            per_cat: empty_per_cat(),
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

/// Keeps [`EntityFilter::allowed`] in sync with the world.
///
/// Driven by `Added<EntityName>` and `RemovedComponents<EntityName>`, so it is
/// O(changes) rather than O(entities) and costs nothing once the world settles.
/// The whole system is `run_if`-gated on a filter being configured, so in the
/// default case it never runs at all.
pub fn refresh_log_entity_filter(
    mut cfg: ResMut<LogFilterConfig>,
    added: Query<(Entity, &EntityName), Added<EntityName>>,
    mut removed: RemovedComponents<EntityName>,
) {
    // Collect first so we are not holding a borrow of `cfg` across the checks.
    let newly_named: Vec<(Entity, String)> =
        added.iter().map(|(e, name)| (e, name.0.clone())).collect();
    let gone: Vec<Entity> = removed.read().collect();

    if newly_named.is_empty() && gone.is_empty() {
        return;
    }

    let Some(filter) = cfg.entity_filter.as_mut() else {
        return;
    };
    for (entity, name) in newly_named {
        if filter.matches_name(&name) {
            filter.allowed.insert(entity);
        }
    }
    for entity in gone {
        filter.allowed.remove(&entity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_level_applies_to_unlisted_categories() {
        let cfg = LogFilterConfig {
            default_level: LevelFilter::Info,
            ..Default::default()
        };
        assert!(cfg.cat_enabled(LogCat::Ai, LevelFilter::Warn));
        assert!(cfg.cat_enabled(LogCat::Ai, LevelFilter::Info));
        assert!(!cfg.cat_enabled(LogCat::Ai, LevelFilter::Debug));
    }

    #[test]
    fn per_category_override_beats_default() {
        let mut per_cat = empty_per_cat();
        per_cat.insert(LogCat::Ai, LevelFilter::Trace);
        per_cat.insert(LogCat::Physics, LevelFilter::Off);
        let cfg = LogFilterConfig {
            default_level: LevelFilter::Warn,
            per_cat,
            entity_filter: None,
        };
        assert!(cfg.cat_enabled(LogCat::Ai, LevelFilter::Trace));
        assert!(!cfg.cat_enabled(LogCat::Physics, LevelFilter::Error));
        // Unlisted category still gets the default.
        assert!(cfg.cat_enabled(LogCat::Helm, LevelFilter::Warn));
        assert!(!cfg.cat_enabled(LogCat::Helm, LevelFilter::Info));
    }

    #[test]
    fn off_suppresses_every_level() {
        assert!(!LevelFilter::Off.allows(LevelFilter::Error));
        assert!(!LevelFilter::Trace.allows(LevelFilter::Off));
    }

    #[test]
    fn no_entity_filter_allows_everything() {
        let cfg = LogFilterConfig::default();
        assert!(cfg.entity_allowed(Entity::from_raw_u32(7).unwrap()));
    }

    #[test]
    fn entity_filter_matches_exactly_then_case_insensitive_substring() {
        let f = EntityFilter::new(vec!["Ironveil".into()]);
        assert!(f.matches_name("Ironveil"));
        assert!(f.matches_name("ironveil"));
        assert!(f.matches_name("USS Ironveil Mk II"));
        assert!(!f.matches_name("Ashrender"));
    }

    #[test]
    fn entity_filter_denies_unresolved_entities() {
        let cfg = LogFilterConfig {
            entity_filter: Some(EntityFilter::new(vec!["Ironveil".into()])),
            ..Default::default()
        };
        // Nothing resolved yet, so nothing passes.
        assert!(!cfg.entity_allowed(Entity::from_raw_u32(7).unwrap()));
    }

    #[test]
    fn refresh_adds_matching_and_drops_despawned() {
        let mut app = App::new();
        app.insert_resource(LogFilterConfig {
            entity_filter: Some(EntityFilter::new(vec!["Ironveil".into()])),
            ..Default::default()
        });
        app.add_systems(Update, refresh_log_entity_filter);

        let hit = app.world_mut().spawn(EntityName("Ironveil".into())).id();
        let miss = app.world_mut().spawn(EntityName("Ashrender".into())).id();
        app.update();

        let cfg = app.world().resource::<LogFilterConfig>();
        assert!(cfg.entity_allowed(hit));
        assert!(!cfg.entity_allowed(miss));

        app.world_mut().despawn(hit);
        app.update();
        let cfg = app.world().resource::<LogFilterConfig>();
        assert!(!cfg.entity_allowed(hit));
    }
}
