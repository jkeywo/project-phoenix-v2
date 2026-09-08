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
        std::fs::read_to_string(path).ok()
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
                match ModelRig::from_toml(&toml_str) {
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
pub(crate) fn sync_authoritative_model_markers(world: &mut World) {
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
    let Some(path) = config.mesh.as_ref().and_then(primary_sidecar_path) else {
        return;
    };
    let body = std::fs::read_to_string(&path).unwrap_or_default();
    crate::content_ledger::record(&path, &body);
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
    let remaining = fixed.overstep();
    let timestep = fixed.timestep();
    let remainder_nanos = remaining.as_nanos() % timestep.as_nanos();
    let remainder = std::time::Duration::new(
        u64::try_from(remainder_nanos / 1_000_000_000).unwrap_or(u64::MAX),
        (remainder_nanos % 1_000_000_000) as u32,
    );
    fixed.discard_overstep(remaining - remainder);
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // UUIDs isolate temporary fixture directories, never simulation identities.
mod tests {
    use super::*;
    use crate::entities::config::{MeshConfig, MeshShape};

    fn primary_mesh() -> MeshSection {
        MeshSection(MeshConfig {
            model: Some("assets/models/dynasty_destroyer.glb".to_string()),
            variant: None,
            shape: MeshShape::Cuboid,
            colour: vec![0.5, 0.5, 0.5],
            radius: 1.0,
            size: Some([2.0, 1.0, 4.0]),
            minor_radius: 0.0,
            emissive: None,
            scale: 1.0,
            rotation: [0.0, 0.0, 0.0],
        })
    }

    #[derive(Resource, Default)]
    struct PresentationGeometry(Option<(Vec3, Vec3, Vec3)>);

    fn observe_presentation_geometry(
        subjects: Query<(&Transform, &ModelMarkers)>,
        mut observed: ResMut<PresentationGeometry>,
    ) {
        let (transform, markers) = subjects.single().expect("one visual subject");
        observed.0 = Some((
            markers
                .resolve_world_position(transform, "fore_emitter")
                .unwrap(),
            markers
                .resolve_world_direction(transform, "fore_emitter")
                .unwrap(),
            markers
                .resolve_target_point_world_position(transform, 2)
                .unwrap(),
        ));
    }

    fn profile(with_presentation_consumer: bool, mesh: MeshSection) -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<ModelRigReadiness>()
            .add_systems(PreUpdate, sync_authoritative_model_markers);
        if with_presentation_consumer {
            app.init_resource::<PresentationGeometry>()
                .add_systems(Update, observe_presentation_geometry);
        }
        let entity = app
            .world_mut()
            .spawn((
                mesh,
                Transform::from_translation(Vec3::new(12.0, 3.0, -8.0)),
            ))
            .id();
        app.update();
        (app, entity)
    }

    /// The authoritative seam has no GLB, Scene, AssetServer, camera, or render
    /// plugin dependency. A rendererless GM profile therefore gets the same
    /// weapon origin and target geometry a rendered host consumes.
    #[test]
    fn primary_rig_markers_load_without_a_renderer_or_glb_scene() {
        let (app, entity) = profile(false, primary_mesh());
        assert!(
            app.world().get_resource::<AssetServer>().is_none(),
            "precondition: this profile owns no renderer or asset server"
        );

        let transform = app.world().get::<Transform>(entity).unwrap();
        let markers = app
            .world()
            .get::<ModelMarkers>(entity)
            .expect("the simulation-side loader attaches the primary rig");
        let weapon_origin = markers
            .resolve_world_position(transform, "fore_emitter")
            .expect("the authored weapon marker resolves");
        let target = markers
            .resolve_target_point_world_position(transform, 1)
            .expect("the authored target point resolves");

        assert!(
            (weapon_origin - Vec3::new(11.999_293, 2.9, -7.455_968)).length() < 1e-4,
            "primary base rig must be composed into the weapon origin, got {weapon_origin:?}"
        );
        assert!(
            (target - Vec3::new(12.250_398, 2.8, -8.149_602)).length() < 1e-4,
            "primary base rig must be composed into the target point, got {target:?}"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    struct RigFixture {
        directory: std::path::PathBuf,
        model: String,
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl RigFixture {
        fn new() -> Self {
            let directory =
                std::env::temp_dir().join(format!("phoenix-marker-batch-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let model = directory.join("fixture.glb").to_string_lossy().into_owned();
            Self { directory, model }
        }

        fn write(&self, variant: Option<&str>, body: &str) {
            std::fs::write(
                crate::entities::model_rig::sidecar_path(&self.model, variant),
                body,
            )
            .unwrap();
        }

        fn mesh(&self, variant: Option<&str>, scale: f32) -> MeshSection {
            let mut mesh = primary_mesh();
            mesh.0.model = Some(self.model.clone());
            mesh.0.variant = variant.map(str::to_owned);
            mesh.0.scale = scale;
            mesh
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    impl Drop for RigFixture {
        fn drop(&mut self) {
            for entry in std::fs::read_dir(&self.directory).unwrap() {
                std::fs::remove_file(entry.unwrap().path()).unwrap();
            }
            std::fs::remove_dir(&self.directory).unwrap();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn rig_body(x: f32) -> String {
        format!(
            "[base]\noffset = [1.0, 0.0, 0.0]\nscale = [2.0, 2.0, 2.0]\n\
             [markers.fore_emitter]\nposition = [{x}, 0.0, 0.0]\n\
             direction = [0.0, 0.0, -1.0]\n\
             [[target_points]]\nposition = [{x}, 1.0, 0.0]\n"
        )
    }

    /// A shared rig is reusable geometry, not a shared entity transform or a
    /// model-only key: authored variants and parent scales stay independent.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn marker_batch_preserves_variants_and_each_entities_transform() {
        let fixture = RigFixture::new();
        fixture.write(None, &rig_body(3.0));
        fixture.write(Some("alternate"), &rig_body(5.0));
        let mut app = App::new();
        app.init_resource::<ModelRigReadiness>()
            .add_systems(PreUpdate, sync_authoritative_model_markers);
        let subjects: Vec<_> = [
            (None, 1.0, 7.0),
            (None, 3.0, 21.0),
            (Some("alternate"), 1.0, 11.0),
        ]
        .into_iter()
        .map(|(variant, scale, expected_x)| {
            let entity = app
                .world_mut()
                .spawn((
                    fixture.mesh(variant, scale),
                    Transform::from_xyz(10.0, 0.0, 0.0),
                ))
                .id();
            (entity, scale, expected_x + 10.0)
        })
        .collect();
        app.update();
        for (entity, scale, expected_x) in subjects {
            let transform = app.world().get::<Transform>(entity).unwrap();
            let markers = app.world().get::<ModelMarkers>(entity).unwrap();
            assert_eq!(transform.scale, Vec3::splat(scale));
            assert_eq!(
                markers.resolve_world_position(transform, "fore_emitter"),
                Some(Vec3::new(expected_x, 0.0, 0.0))
            );
            assert_eq!(
                markers.resolve_target_point_world_position(transform, 0),
                Some(Vec3::new(expected_x, 2.0 * scale, 0.0))
            );
        }
        assert!(!app
            .world()
            .resource::<ModelRigReadiness>()
            .blocks_simulation());
    }

    /// No lookup result survives the sync invocation, including the identity
    /// fallback for absence/malformed content. Future spawns resolve the latest
    /// body while already-attached entities retain their canonical geometry.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn marker_batch_observes_sidecar_delivery_and_replacement_on_later_passes() {
        let fixture = RigFixture::new();
        let mut app = App::new();
        app.init_resource::<ModelRigReadiness>()
            .add_systems(PreUpdate, sync_authoritative_model_markers);
        let mut prior = Vec::new();
        for (body, expected) in [
            (None, None),
            (Some("[malformed".to_string()), None),
            (Some(rig_body(3.0)), Some(7.0)),
            (Some(rig_body(5.0)), Some(11.0)),
        ] {
            if let Some(body) = body {
                fixture.write(None, &body);
            }
            // More than one entity exercises sharing inside each pass.
            for _ in 0..2 {
                let entity = app
                    .world_mut()
                    .spawn((fixture.mesh(None, 1.0), Transform::default()))
                    .id();
                prior.push((entity, expected));
            }
            app.update();
            for &(entity, expected) in &prior {
                let markers = app.world().get::<ModelMarkers>(entity).unwrap();
                assert_eq!(
                    markers.resolve_world_position(&Transform::IDENTITY, "fore_emitter"),
                    expected.map(|x| Vec3::new(x, 0.0, 0.0))
                );
            }
        }
    }

    /// Registering a presentation-side read must not change canonical geometry
    /// or the authoritative digest. This is the profile-axis parity guard: both
    /// worlds load their markers through the simulation system; render merely
    /// observes the result.
    #[test]
    fn rendered_and_rendererless_profiles_share_geometry_and_digest() {
        let mut authored_mesh = primary_mesh();
        authored_mesh.0.scale = 1.75;
        authored_mesh.0.rotation = [0.15, -0.35, 0.2];
        let (rendererless, rendererless_entity) = profile(false, authored_mesh.clone());
        let (rendered, rendered_entity) = profile(true, authored_mesh);

        let geometry = |app: &App, entity: Entity| {
            let transform = app.world().get::<Transform>(entity).unwrap();
            let markers = app.world().get::<ModelMarkers>(entity).unwrap();
            (
                markers
                    .resolve_world_position(transform, "fore_emitter")
                    .unwrap(),
                markers
                    .resolve_world_direction(transform, "fore_emitter")
                    .unwrap(),
                markers
                    .resolve_target_point_world_position(transform, 2)
                    .unwrap(),
            )
        };

        let rendered_geometry = geometry(&rendered, rendered_entity);
        assert_eq!(
            rendered.world().get::<Transform>(rendered_entity),
            rendererless.world().get::<Transform>(rendererless_entity),
            "authored mesh scale/rotation must be applied before the render axis splits"
        );
        assert_eq!(
            rendered_geometry,
            geometry(&rendererless, rendererless_entity),
            "the render axis may not select different weapon/target geometry"
        );
        assert_eq!(
            rendered
                .world()
                .resource::<PresentationGeometry>()
                .0
                .expect("the rendered profile consumed canonical markers"),
            rendered_geometry,
            "presentation must read the simulation-owned component, not resolve a second rig"
        );
        assert_eq!(
            crate::sim_digest::state_digest(&rendered),
            crate::sim_digest::state_digest(&rendererless),
            "the same authored parent transform must leave both profiles on one digest; the \
             geometry assertion above separately covers ModelMarkers, which are deferred state"
        );
    }

    /// Native filesystem ingestion and the browser's JS-delivery shape must
    /// freeze the identical sidecar key/body pair. The raw body is the identity
    /// input even when it is malformed (runtime falls back) or empty (404), so
    /// neither failure mode may alias the valid authored rig.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_and_wasm_shaped_sidecars_share_and_move_frozen_content_identity() {
        let mesh = primary_mesh();
        let config = crate::entities::config::EntityConfig {
            mesh: Some(mesh.0.clone()),
            ..Default::default()
        };
        let path = primary_sidecar_path(&mesh.0).expect("fixture has a model");
        let body = std::fs::read_to_string(&path).expect("shipped primary rig exists");

        crate::content_ledger::reset();
        record_primary_sidecar_from_fs(&config);
        crate::content_ledger::freeze();
        assert!(crate::content_ledger::frozen_covers(&path));
        let native_digest = crate::content_ledger::frozen_or_live().fold();

        crate::content_ledger::reset();
        let _ = crate::entities::config_cache::wasm_push_sidecar_toml(path.clone(), body);
        crate::content_ledger::freeze();
        let wasm_shaped_digest = crate::content_ledger::frozen_or_live().fold();
        assert_eq!(
            wasm_shaped_digest, native_digest,
            "the two boot profiles must bind the same canonical path and exact bytes"
        );

        crate::content_ledger::reset();
        let _ = crate::entities::config_cache::wasm_push_sidecar_toml(
            path.clone(),
            "[malformed-primary-rig".to_string(),
        );
        crate::content_ledger::freeze();
        let malformed_digest = crate::content_ledger::frozen_or_live().fold();
        assert_ne!(malformed_digest, native_digest);

        crate::content_ledger::reset();
        let _ = crate::entities::config_cache::wasm_push_sidecar_toml(path, String::new());
        crate::content_ledger::freeze();
        let absent_digest = crate::content_ledger::frozen_or_live().fold();
        assert_ne!(absent_digest, native_digest);
        assert_ne!(absent_digest, malformed_digest);
        crate::content_ledger::reset();
    }

    #[derive(Resource, Default)]
    struct FixedConsumerProbe {
        steps: u32,
        saw_missing_markers: bool,
        saw_canonical_markers: bool,
    }

    fn spawn_runtime_model(mut commands: Commands, mut spawned: Local<bool>) {
        if !*spawned {
            commands.spawn((primary_mesh(), Transform::default()));
            *spawned = true;
        }
    }

    fn consume_runtime_model(
        mut probe: ResMut<FixedConsumerProbe>,
        models: Query<Has<ModelMarkers>, With<MeshSection>>,
    ) {
        for has_markers in &models {
            probe.saw_missing_markers |= !has_markers;
            probe.saw_canonical_markers |= has_markers;
        }
        probe.steps += 1;
    }

    /// A model spawned by one fixed step is synchronised in `FixedLast`, before
    /// the following step can present it to an authoritative consumer. This is
    /// the runtime/script-spawn half of the seam; it does not rely on a render
    /// frame occurring between fixed steps.
    #[test]
    fn runtime_spawn_is_canonical_before_the_next_fixed_consumer() {
        let period = std::time::Duration::from_millis(10);
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<ModelRigReadiness>()
            .init_resource::<FixedConsumerProbe>()
            .add_systems(FixedUpdate, (spawn_runtime_model, consume_runtime_model))
            .add_systems(FixedLast, sync_authoritative_model_markers);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);

        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        app.update();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 2));
        app.update();

        let probe = app.world().resource::<FixedConsumerProbe>();
        assert_eq!(probe.steps, 2);
        assert!(!probe.saw_missing_markers);
        assert!(probe.saw_canonical_markers);
    }

    fn count_fixed_steps(mut probe: ResMut<FixedConsumerProbe>) {
        probe.steps += 1;
    }

    /// The runtime-spawn safeguard is dormant in an ordinary frame: all whole
    /// fixed steps and the fractional interpolation remainder survive when no
    /// live entity is waiting on a primary sidecar.
    #[test]
    fn normal_frame_keeps_fixed_overstep_when_no_rig_is_blocked() {
        let period = std::time::Duration::from_millis(10);
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<ModelRigReadiness>()
            .init_resource::<FixedConsumerProbe>()
            .add_systems(FixedUpdate, count_fixed_steps)
            .add_systems(FixedLast, discard_blocked_model_rig_overstep);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);

        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        app.update();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            period * 3 + period / 2,
        ));
        app.update();

        assert_eq!(app.world().resource::<FixedConsumerProbe>().steps, 3);
        assert_eq!(app.world().resource::<Time<Fixed>>().overstep(), period / 2);
    }

    #[test]
    fn blocked_runtime_rig_discards_only_unbegun_whole_steps() {
        let period = std::time::Duration::from_millis(10);
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<ModelRigReadiness>()
            .init_resource::<FixedConsumerProbe>()
            .add_systems(FixedUpdate, count_fixed_steps)
            .add_systems(FixedLast, discard_blocked_model_rig_overstep);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);
        app.world_mut()
            .resource_mut::<ModelRigReadiness>()
            .live_blocked = true;

        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        app.update();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            period * 3 + period / 2,
        ));
        app.update();

        assert_eq!(app.world().resource::<FixedConsumerProbe>().steps, 1);
        assert_eq!(app.world().resource::<Time<Fixed>>().overstep(), period / 2);
    }
}
