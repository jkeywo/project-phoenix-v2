//! What a *runtime-spawned* entity was made from (issue #863).
//!
//! Pure module: no Bevy, no filesystem, no config cache. It holds one record
//! type and one resolver, and both exist to answer a single question a resume
//! could not answer before — **"this ship is in the save; what is it?"**
//!
//! # Why a record is needed at all
//!
//! A world's authored `[[entity]]` blocks are re-spawned by any fresh boot of
//! the same scenario, so a restore can find them already standing and simply
//! overwrite their state ([`crate::snapshot::restore`] is documented as an
//! overwrite, not a spawn). A **runtime** spawn is not like that. It happened
//! because a script said so, at a tick that depended on how the run went, and a
//! fresh app's bootstrap only reproduces it by re-running the same scenario to
//! the same point with the same inputs. A resumed browser session is exactly
//! the case where that does not hold: the fresh app boots with nobody at the
//! consoles, so a wave released by a player's action is a wave the bootstrap
//! never releases, and the raid the capture recorded is lost with no error.
//!
//! What the spawn needed was its template, its instance overrides and its
//! placement. None of those is recoverable from the spawned entity's
//! components: the template is merged into the config at spawn and the path is
//! discarded, and the overrides are gone the same way. So the spawn *records*
//! them, on the entity it produced.
//!
//! # Why it rides on the entity rather than in a ledger
//!
//! The sibling records on `WorldContentRuntime` (the deadline table, the
//! commitments ledger, the evidence log) are all state about the *world*, and
//! they live there so every site that borrows the runtime can mutate them. This
//! is state about **one entity**, and putting it on that entity buys the one
//! property a uuid-keyed ledger would have to implement by hand: a destroyed
//! ship's record dies with it. Nothing has to prune, nothing can go stale, and
//! the capture walk that already visits every `EntityUuid` reads it in place
//! rather than joining against a second table.
//!
//! # What it is not
//!
//! Not a spawn *queue* and not a replay log. Nothing reads a record during a
//! live run — no system, no tick, no predicate. It is written once at the spawn
//! and read exactly twice: by a capture, and by the restore that has to rebuild
//! the entity the capture named.

use serde::{Deserialize, Serialize};

use crate::entities::config::EntityConfig;
use crate::entities::loader::TemplateLoader;

/// The inputs a runtime spawn was resolved from, kept so it can be resolved
/// again.
///
/// Every field is one the spawn *consumed*; nothing derived from the resulting
/// entity is here, because everything derived is already in the snapshot
/// payload's own per-entity row. In particular the position is the position the
/// spawn was placed at, **not** where the ship has since flown to — that is
/// `EntityState::physics`, and a restore writes it over the top.
///
/// `anchor` is deliberately absent: an anchor is a name that resolves to a
/// position against the authoring layer's table, and the resolution has already
/// happened by the time a spawn exists. Storing the name would mean re-resolving
/// it against a layer that may not be loaded in the target world, which is a
/// second, quietly disagreeing answer to a question this record already has.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpawnOrigin {
    /// The entity template the spawn resolved, as authored.
    ///
    /// A path into content the *content digest* is bound to — so the file this
    /// names cannot have moved under a save that loads at all, which is why the
    /// template is re-read on restore rather than stored.
    pub template_path: String,
    /// The scenario's name for this entity, patched over the template's display
    /// name exactly as the spawn patched it. This is the name
    /// `WorldContentRuntime::name_to_uuid` keys on and an `AiDirective::Destroy`
    /// matches against.
    pub name: String,
    /// The resolved spawn position (anchor lookups already done).
    pub position: [f32; 3],
    /// XYZ Euler radians, as the `[[entity]]` schema means them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<[f32; 3]>,
    /// Per-axis scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<[f32; 3]>,
    /// The instance overrides merged onto the template, as the dynamic
    /// `toml::Value` the merge consumed.
    ///
    /// This is the field that makes the record worth having. A shipped raid's
    /// hostility, doctrine and weapon fit are *all* authored here rather than in
    /// the template — `combat_test` alone spawns nineteen override pairs — so a
    /// rebuild from the bare template would put back a hull that is neither
    /// hostile nor armed, which is a worse answer than reporting the gap.
    ///
    /// A `toml::Value` and not the resolved [`EntityConfig`]: the value is a
    /// dynamic document that pins no struct's shape as save format, whereas
    /// storing the config would freeze thirty authored sections' serde layout
    /// into the payload — and would still be wrong, because
    /// `EntityConfig`'s `#[serde(skip)]` system blocks do not survive a plain
    /// round-trip (issue #838).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<toml::Value>,
    /// The world layer whose script authored the spawn, `None` for the base
    /// world. A layer-owned entity is despawned when its layer unloads, so a
    /// rebuild has to re-declare the ownership or the entity outlives the layer
    /// that made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_path: Option<String>,
}

impl SpawnOrigin {
    /// Resolve this record back into the `EntityConfig` the spawn produced.
    ///
    /// The same two steps `world::dispatch`'s spawn arm takes, in the same
    /// order, through the same shared merge
    /// ([`crate::entities::loader::apply_overrides`]): load the template, merge
    /// the instance overrides onto it, then patch the scenario's name over the
    /// template's display name.
    ///
    /// # Failure is reported, never guessed
    ///
    /// `None` means the template did not resolve, and the caller turns that into
    /// a gap rather than a spawn: an entity built from a template this build
    /// cannot read is an entity with an invented collider, an invented hull
    /// maximum and no weapons, and a resumed world carrying one is worse than a
    /// resumed world that says out loud what it could not rebuild.
    ///
    /// A failed *override* merge is different and deliberately not fatal, for
    /// the reason the spawn path already gives it: the template alone is a
    /// partial answer, and a partial spawn beats none. The warning says so.
    pub fn resolve(
        &self,
        loader: &dyn TemplateLoader,
        warnings: &mut Vec<String>,
    ) -> Option<EntityConfig> {
        let name = &self.name;
        let Some(mut config) = loader.load_template(&self.template_path) else {
            warnings.push(format!(
                "spawn origin '{name}' template '{}' did not resolve",
                self.template_path
            ));
            return None;
        };

        if let Some(overrides) = &self.overrides {
            match crate::entities::loader::apply_overrides(&config, overrides) {
                Ok(merged) => config = merged,
                Err(e) => warnings.push(format!(
                    "spawn origin '{name}' overrides did not apply (kept template): {e}"
                )),
            }
        }

        if !name.is_empty() {
            config.name = Some(name.clone());
        }
        Some(config)
    }
}

#[cfg(test)]
#[path = "spawn_origin_tests.rs"]
mod tests;
