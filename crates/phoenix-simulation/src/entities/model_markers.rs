//! Authoritative per-entity model-marker loading (issue #1291).
//!
//! A model rig is authored beside the primary GLB named by an entity's
//! `[mesh]` section. Its markers and target points are simulation content:
//! weapons, cameras, docking, and effects resolve geometry through them whether
//! or not this process owns a renderer. The active visual is presentation only.
//! A generated LOD tier, procedural fallback, or billboard therefore never
//! replaces or removes the primary rig's [`ModelMarkers`].
//!
//! On native targets sidecars resolve synchronously from disk. On WASM they
//! arrive through the persistent sidecar inbox populated by the host page. The
//! [`sync_authoritative_model_markers`] retries entities whose fetch is still
//! in flight, immediately inserts resolved geometry, and is registered for
//! every simulation profile, including rendererless GM peers.

use std::collections::{BTreeSet, HashMap};

use bevy::prelude::*;

use crate::entities::model_rig::{ModelMarkers, ModelRig};
use crate::entities::spawner::MeshSection;

/// Local readiness for the authoritative primary-rig content.
///
/// This is deliberately separate from `AssetPreloadResource`: GLBs, textures,
/// and generated LOD tiers are presentation, while the primary sidecar carries
/// weapon origins and target geometry. Every authoritative browser profile,
/// including a rendererless GM, primes the same set before mission start.
#[derive(Resource, Debug, Default)]
pub struct ModelRigReadiness {
    primed: bool,
    required_sidecars: BTreeSet<String>,
    complete: bool,
    live_blocked: bool,
}

impl ModelRigReadiness {
    /// Whether the frozen template cache has been scanned and every primary
    /// sidecar has reached a terminal delivered state.
    pub fn is_ready(&self) -> bool {
        self.primed && self.complete
    }

    /// Whether a live model-bearing entity still lacks its canonical markers.
    /// This is the narrow fixed-clock hold; prewarming a future template alone
    /// never steals simulation time.
    pub fn blocks_simulation(&self) -> bool {
        self.live_blocked
    }

    #[cfg(test)]
    pub(crate) fn set_live_blocked_for_test(&mut self, blocked: bool) {
        self.live_blocked = blocked;
    }
}

/// Read a model-rig sidecar TOML for `path`.
///
/// - **Native**: `std::fs::read_to_string` (returns `None` when absent).
/// - **WASM**: checks the pending-sidecar cache populated by JS via
///   `wasm_push_sidecar_toml`; fires a deferred JS fetch on first miss and
///   returns `None` until the fetch resolves. An empty pushed string (404)
///   resolves to `Some(String::new())`, which parses to an identity rig.
///
/// The cache read is non-destructive so the authoritative loader, asset
/// preloader, renderer, and every entity sharing a model all see the same body.
fn load_sidecar_toml(path: &str, absence: Absence) -> Option<String> {
    let text = load_sidecar_toml_text(path, absence);
    // A rig sidecar is authored content too: record it into the ledger the same
    // way world/entity loaders do (issue #935).
    if let Some(text) = &text {
        crate::content_ledger::record(path, text);
    }
    text
}

/// Whether a sidecar that turns out not to exist is news.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Absence {
    /// The file is expected to exist; a 404 is a defect worth logging.
    Unexpected,
    /// The read is itself an existence test; a 404 is an answer, not a fault.
    Expected,
}

fn load_sidecar_toml_text(path: &str, absence: Absence) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = absence;
        crate::content_fs::read_to_string(path).ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::entities::config_cache::take_pending_sidecar_toml(path).or_else(|| {
            crate::entities::config_cache::request_sidecar_fetch(
                path.to_string(),
                absence == Absence::Expected,
            );
            None
        })
    }
}

/// Resolve a model's primary rig sidecar.
///
/// Returns `None` only while a WASM fetch is still in flight. A genuinely
/// absent sidecar or a malformed one degrades to an identity rig so an entity
/// remains usable, while a malformed body is logged loudly because it loses
/// both markers and its LOD declaration.
pub fn resolve_sidecar_rig(model_path: &str, variant: Option<&str>) -> Option<ModelRig> {
    resolve_sidecar_rig_where(model_path, variant, Absence::Unexpected)
}

/// [`resolve_sidecar_rig`] for a read that is itself an existence test.
///
/// Only legacy generated-tier convention probing should use this. Canonical
/// authoritative markers always resolve from the primary `[mesh]` rig through
/// [`resolve_sidecar_rig`].
pub fn resolve_sidecar_rig_optional(model_path: &str, variant: Option<&str>) -> Option<ModelRig> {
    resolve_sidecar_rig_where(model_path, variant, Absence::Expected)
}

fn resolve_sidecar_rig_where(
    model_path: &str,
    variant: Option<&str>,
    absence: Absence,
) -> Option<ModelRig> {
    let path = crate::entities::model_rig::sidecar_path(model_path, variant);
    match load_sidecar_toml(&path, absence) {
        Some(toml_str) => {
            if toml_str.trim().is_empty() {
                Some(ModelRig::default())
            } else {
                match parse_model_rig(&toml_str) {
                    Ok(rig) => Some(rig),
                    Err(e) => {
                        bevy::log::error!(
                            target: crate::logging::LogCat::Assets.target(),
                            "rig sidecar {path} failed to parse: {e}; falling back to an \
                             identity rig — this model loses its markers AND any [[lod]] \
                             chain, and will render only its flat [mesh] level"
                        );
                        Some(ModelRig::default())
                    }
                }
            }
        }
        None => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                Some(ModelRig::default())
            }
            #[cfg(target_arch = "wasm32")]
            {
                None
            }
        }
    }
}

/// Prime primary rigs and synchronise live entities with their canonical
/// marker geometry.
///
/// The browser config cache is frozen before the Bevy app is constructed, so
/// its first scan covers player hulls and templates that scripts may spawn
/// later. The live scan is the defensive runtime path for an entity supplied
/// outside that cache. This is an exclusive `World` system so a resolved rig is
/// inserted immediately: no fixed consumer can run between observing readiness
/// and seeing the component.
pub fn sync_authoritative_model_markers(world: &mut World) {
    let primed = world.resource::<ModelRigReadiness>().primed;
    let mut discovered = BTreeSet::new();

    if !primed {
        let cache = crate::entities::config_cache::get_config_cache();
        for config in cache.values() {
            if let Some(mesh) = &config.mesh {
                register_primary_sidecar(&mut discovered, mesh);
            }
        }
    }

    let live_meshes: Vec<crate::entities::config::MeshConfig> = {
        let mut meshes = world.query::<&MeshSection>();
        meshes.iter(world).map(|mesh| mesh.0.clone()).collect()
    };
    for mesh in &live_meshes {
        register_primary_sidecar(&mut discovered, mesh);
    }

    let required_sidecars = {
        let mut readiness = world.resource_mut::<ModelRigReadiness>();
        readiness.required_sidecars.extend(discovered);
        readiness.primed = true;
        readiness.required_sidecars.clone()
    };

    for path in &required_sidecars {
        crate::entities::config_cache::request_sidecar_fetch(path.clone(), false);
    }

    let candidates: Vec<(Entity, crate::entities::config::MeshConfig)> = {
        let mut entities = world.query_filtered::<(Entity, &MeshSection), Without<ModelMarkers>>();
        entities
            .iter(world)
            .filter(|(_, mesh)| mesh.0.model.is_some())
            .map(|(entity, mesh)| (entity, mesh.0.clone()))
            .collect()
    };

    // Resolve each exact sidecar only once in this synchronous batch. A cell
    // boundary can add many entities sharing one rig; repeated disk reads and
    // TOML parses do not give those entities different authored geometry.
    // This cache never survives the invocation: pending WASM fetches are
    // retried, and a later native spawn sees any intervening content change.
    // Entity order, per-entity transforms and immediate attachment stay intact.
    let mut batch_markers: HashMap<String, Option<ModelMarkers>> = HashMap::new();
    let mut live_blocked = false;
    for (entity, mesh) in candidates {
        let model_path = mesh.model.as_deref().expect("filtered above");
        let path = crate::entities::model_rig::sidecar_path(model_path, mesh.variant.as_deref());
        let markers = batch_markers.entry(path).or_insert_with(|| {
            resolve_sidecar_rig(model_path, mesh.variant.as_deref())
                .map(|rig| ModelMarkers::from_rig(&rig))
        });
        let Some(markers) = markers else {
            live_blocked = true;
            continue;
        };
        if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
            if let Some(mut transform) = entity_mut.get_mut::<Transform>() {
                apply_mesh_transform(&mesh, &mut transform);
            }
            entity_mut.insert(markers.clone());
        }
    }

    let delivered = required_sidecars
        .iter()
        .all(|path| crate::entities::config_cache::is_pending_sidecar_delivered(path));

    let mut readiness = world.resource_mut::<ModelRigReadiness>();
    readiness.live_blocked = live_blocked;
    // Native resolves sidecars synchronously from disk, including the explicit
    // identity-rig fallback for an absent file. Only WASM has a delivery wait.
    #[cfg(target_arch = "wasm32")]
    {
        readiness.complete = delivered;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = delivered;
        readiness.complete = true;
    }
}

fn register_primary_sidecar(
    required: &mut BTreeSet<String>,
    mesh: &crate::entities::config::MeshConfig,
) {
    if let Some(path) = primary_sidecar_path(mesh) {
        required.insert(path);
    }
}

/// Canonical repo-relative path of the primary authored rig for one `[mesh]`.
///
/// This is shared by boot-time content binding and runtime marker attachment so
/// those two authority seams cannot derive different keys for the same model.
pub(crate) fn primary_sidecar_path(mesh: &crate::entities::config::MeshConfig) -> Option<String> {
    let model_path = mesh.model.as_deref()?;
    let sidecar = crate::entities::model_rig::sidecar_path(model_path, mesh.variant.as_deref());
    Some(crate::entities::include_resolve::canonical_template_path(
        &sidecar,
    ))
}

/// Bind a native template's primary authored rig into the live content ledger.
///
/// Called by [`crate::entities::loader::FsTemplateLoader`] at the same moment it
/// records the composed entity template. Missing sidecars are recorded as empty
/// bytes, matching the browser preload's 404 delivery; malformed non-empty
/// bodies are recorded verbatim before runtime parsing falls back to an identity
/// rig. Either kind of content change therefore moves the frozen peer/save
/// identity.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn record_primary_sidecar_from_fs(config: &crate::entities::config::EntityConfig) {
    if let Some(record) = capture_primary_sidecar_from_fs(config) {
        record.apply();
    }
}

/// Capture the same optional sidecar record without changing the content ledger.
#[cfg(not(target_arch = "wasm32"))]
pub fn capture_primary_sidecar_from_fs(
    config: &crate::entities::config::EntityConfig,
) -> Option<crate::content_ledger::LedgerDigest> {
    let path = config.mesh.as_ref().and_then(primary_sidecar_path)?;
    let body = crate::content_fs::read_to_string(&path).unwrap_or_default();
    Some(crate::content_ledger::LedgerDigest {
        key: path,
        digest: vellum_digest::fnv1a(body.as_bytes()),
    })
}

/// Apply the authored `[mesh]` parent transform once, independently of the
/// renderer.
///
/// This used to live in `render_spawned_entities`, which meant the rendered
/// host changed authoritative marker world positions while rendererless peers
/// kept the spawn transform. Moving the exact operation here preserves the
/// shipped rendered geometry and gives every authoritative profile the same
/// rotation and scale before markers are consumed.
pub(crate) fn apply_authored_mesh_transform(
    mut entities: Query<
        (&MeshSection, &mut Transform),
        (Added<MeshSection>, Without<ModelMarkers>),
    >,
) {
    for (mesh, mut transform) in &mut entities {
        apply_mesh_transform(&mesh.0, &mut transform);
    }
}

fn apply_mesh_transform(mesh: &crate::entities::config::MeshConfig, transform: &mut Transform) {
    if mesh.scale == 1.0 && mesh.rotation == [0.0, 0.0, 0.0] {
        return;
    }
    transform.rotation = Quat::from_euler(
        EulerRot::XYZ,
        mesh.rotation[0],
        mesh.rotation[1],
        mesh.rotation[2],
    );
    transform.scale = Vec3::splat(mesh.scale);
}

/// Stop Bevy's fixed runner from beginning a second step in the same frame
/// after a runtime spawn reveals an unresolved primary rig.
///
/// The step that spawned the entity has already committed. Only whole unbegun
/// catch-up debt is discarded; the fractional interpolation remainder stays.
/// Normal frames are untouched when no live entity is blocked.
pub(crate) fn discard_blocked_model_rig_overstep(
    readiness: Res<ModelRigReadiness>,
    mut fixed: Option<ResMut<Time<Fixed>>>,
) {
    if !readiness.blocks_simulation() {
        return;
    }
    let Some(fixed) = fixed.as_deref_mut() else {
        return;
    };
    crate::lockstep::discard_whole_fixed_overstep(fixed);
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // UUIDs isolate temporary fixture directories, never simulation identities.
#[path = "model_markers_tests.rs"]
mod tests;

use crate::entities::model_rig::parse_model_rig;
