use crate::entities::config::EntityConfig;
use crate::world::config::WorldEntity;
pub use phoenix_sim_gameplay::entities::loader::*;
pub struct SpawnTemplateLoader<'a> {
    /// The cache the spawn will read — the global one at `Startup`, a layer's
    /// own when a layer is spawning.
    pub cache: &'a std::collections::HashMap<String, EntityConfig>,
    /// What the host can serve behind that cache. Production passes
    /// [`FixtureTemplates`].
    pub host: &'a dyn TemplateLoader,
}

impl TemplateLoader for SpawnTemplateLoader<'_> {
    fn load_template(&self, path: &str) -> Option<EntityConfig> {
        match self.cache.get(path) {
            Some(config) => Some(config.clone()),
            None => self.host.load_template(path),
        }
    }

    /// The cache only ever ADDS to what the host can serve, so the host's
    /// answer is the binding one: a cache hit cannot make a blind host
    /// authoritative about everything it is still missing.
    fn absence_is_final(&self) -> bool {
        self.host.absence_is_final()
    }
}

/// Does the template at `path` declare an `[asteroid_field]`?
///
/// Asked through the *same* lookup [`resolve_entity_via`] performs — the
/// caller's cache, then the host loader — because this predicate decides
/// **which spawn path an entry takes**, and a predicate narrower than the spawn
/// mis-routes the entry rather than dropping it (issue #973 review, F1).
///
/// # Why the two must agree
///
/// `world::config::partition_immediate_entities_three_way` and
/// `world::config::is_owned_by_unified_pipeline` split the immediate `[[entity]]`
/// list between the two `Startup` halves: asteroid fields and named entries go
/// to `world::server::spawn_immediate_entities_internal`, the anonymous
/// remainder to `server_app::spawn_anonymous_entities_internal`. While the
/// predicate was cache-only and the spawn was cache-then-disk, a field template
/// that lived on disk but not in the cache answered `false` here, landed in the
/// anonymous bucket, and spawned down the wrong path: no
/// `[asteroid_field] anchor -> anchor_offset` resolution, and an
/// `upsert_world_entity` the field path deliberately omits. A belt at the world
/// origin instead of its anchor is exactly the quiet-wrong-world outcome #973
/// exists to turn loud.
///
/// Reachable rather than theoretical: `headless::app::build_headless_app`
/// derives its preload directory from `--ship`'s parent, so
/// `--ship assets/entities/test/rng_coverage_lancer.toml --world
/// assets/worlds/combat_test.toml` preloads only `assets/entities/test/` and
/// both `asteroid_field_main.toml` belts miss the cache.
///
/// A template that does not resolve at all is not an asteroid field here; the
/// activation gate has already refused that world on any host authoritative
/// enough to say so (see [`TemplateLoader::absence_is_final`]).
pub fn template_is_asteroid_field(
    path: &str,
    config_cache: &std::collections::HashMap<String, EntityConfig>,
    host: &dyn TemplateLoader,
) -> bool {
    SpawnTemplateLoader {
        cache: config_cache,
        host,
    }
    .load_template(path)
    .is_some_and(|c| c.asteroid_field.is_some())
}

/// Resolve a `WorldEntity` to a concrete `EntityConfig`:
/// 1. Look `entity_inst.template_path` up in `config_cache`, then — on a miss —
///    in the `host` loader behind it (issue #973).
/// 2. Optionally merge `entity_inst.overrides` on top of the template.
///
/// The **only** entity-instance resolver; all four `[[entity]]` spawn sites use
/// it, and it is the lookup [`crate::world::validate`]'s `unresolvable-template`
/// gate is handed, so "validation passed" and "the spawn will find it" are one
/// sentence.
///
/// # Errors
///
/// Two shapes, and both are silent drops at the call sites, which log and
/// `continue`: the template did not resolve at all, or `overrides` did not
/// merge (see [`apply_overrides`]). The activation gate refuses a world for
/// either before anything spawns.
pub fn resolve_entity_via(
    entity_inst: &WorldEntity,
    config_cache: &std::collections::HashMap<String, EntityConfig>,
    host: &dyn TemplateLoader,
) -> Result<EntityConfig, String> {
    let loader = SpawnTemplateLoader {
        cache: config_cache,
        host,
    };
    let template = loader
        .load_template(&entity_inst.template_path)
        .ok_or_else(|| {
            format!(
                "entity template not found in cache: '{}'",
                entity_inst.template_path
            )
        })?;

    match &entity_inst.overrides {
        None => Ok(template),
        Some(overrides) => apply_overrides(&template, overrides),
    }
}

#[cfg(test)]
pub struct FixtureTemplates;
#[cfg(test)]
impl TemplateLoader for FixtureTemplates {
    fn load_template(&self, path: &str) -> Option<EntityConfig> {
        use crate::entities::include_resolve::ParseEntityTemplate;
        crate::entities::include_resolve::resolve_template(
            path,
            &crate::entities::include_resolve::FixtureFragments,
        )
        .ok()?
        .parse()
        .ok()
    }
    fn absence_is_final(&self) -> bool {
        true
    }
}
