use crate::entities::config::EntityConfig;

/// Merge an authored `overrides` table on top of a resolved template.
///
/// Shared by every path that resolves an entity instance against its template:
/// [`resolve_entity_via`] below, the world validator's template-resolution
/// check (issue #973), and its doctrine read (issue #888), which has to see the
/// *effective* doctrine —
/// `assets/worlds/probe_artillery_standoff.toml` adds a doctrine entry by
/// override, and a validator reading the raw template would be judging content
/// no scenario ever runs.
///
/// Serialises the template to a `toml::Value` **losslessly** (issue #838):
/// `to_toml_value` re-emits the `[[station]]`/`[[system]]`/`[power_groups]`/
/// `[[shield_arc]]` blocks that a plain `toml::to_string` would drop (they live
/// in `#[serde(skip)]` fields), so the merged config keeps the template's whole
/// system suite instead of spawning a hull with no stations or weapons.
///
/// # Errors
///
/// Besides a serialise/parse failure, an override carrying the `_remove`
/// tombstone is rejected outright (issue #911): that marker is a
/// fragment-composition feature, and an instance override that writes one would
/// otherwise be a silent no-op — see
/// [`crate::entities::entity_override::reject_unhonoured_removals`].
pub fn apply_overrides(
    template: &EntityConfig,
    overrides: &toml::Value,
) -> Result<EntityConfig, String> {
    let template_value = template
        .to_toml_value()
        .map_err(|e| format!("template serialise error: {e}"))?;

    let merged =
        crate::entities::entity_override::merge_entity_config_toml(&template_value, overrides)
            .map_err(|e| format!("override rejected: {e}"))?;
    let merged_str =
        toml::to_string(&merged).map_err(|e| format!("merged serialise error: {e}"))?;
    EntityConfig::from_toml(&merged_str).map_err(|e| format!("merged parse error: {e:?}"))
}

/// Resolve an entity-template path to a concrete `EntityConfig`, abstracting
/// over *where* the template comes from: the filesystem on native
/// (`FsTemplateLoader`), the preloaded config cache on WASM
/// (`WasmTemplateLoader`).
///
/// Object-safe by design — `&self`, no generics on the method — so callers can
/// hold a `&dyn TemplateLoader` and tests can inject a fake without touching
/// the filesystem or the WASM thread-locals.
///
/// # Diagnostics are the caller's job
///
/// `load_template` returns `Option`, not `Result`: a missing template and a
/// malformed one both collapse to `None`, and any `toml::de::Error` is
/// intentionally dropped at this boundary. This module is deliberately free of
/// Bevy (see the file header), so it must not log. The caller — which knows
/// which entity, trigger, or spawn request asked for the template — is
/// responsible for emitting the warning.
pub trait TemplateLoader {
    /// Load and parse the template at `path`, or `None` if it cannot be found
    /// or parsed.
    fn load_template(&self, path: &str) -> Option<EntityConfig>;

    /// Whether a `None` from [`TemplateLoader::load_template`] is the FINAL
    /// answer — "this template does not exist" rather than "I cannot see it
    /// from here" (issue #973).
    ///
    /// This is the named authority condition
    /// [`crate::world::validate`] gates its `unresolvable-template` error on,
    /// and the direct twin of
    /// [`crate::entities::include_resolve::FragmentSource::absence_is_final`] one layer
    /// down (parsed configs rather than raw fragment text).
    ///
    /// # Which hosts are authoritative, and why
    ///
    /// * **Native** ([`FsTemplateLoader`], and [`WasmTemplateLoader`] via its
    ///   filesystem fallback): yes. The filesystem holds everything the run
    ///   will ever have, so absence is a fact about the content.
    /// * **Browser** ([`WasmTemplateLoader`] on `wasm32`): no. The preloaded
    ///   config cache fills one delivery at a time and the runtime layer load
    ///   spawns the moment a layer's TOML arrives, while that layer's entity
    ///   templates were only just queued. Reading that race as "the template
    ///   does not exist" would fail validation for the whole world and blank
    ///   it permanently, since the layer is marked loaded and never retried.
    ///
    /// # Why there is no default
    ///
    /// For the same reason `FragmentSource` has none: the dangerous answer is
    /// the *permissive* one, and the dangerous case — someone deletes
    /// [`WasmTemplateLoader`]'s override — is invisible to a native suite,
    /// because `true` IS the native answer. With no default, that deletion is
    /// a build error on both targets instead of a browser-only regression CI
    /// stays green through.
    fn absence_is_final(&self) -> bool;
}
