//! Spawning.
use super::*;

/// Startup system: spawn `[[entity]]` instances owned by the unified
/// `WorldConfig` pipeline.
///
/// The unified pipeline owns both asteroid-field templates AND any
/// `[[entity]]` carrying a `name` field. The complementary `setup_world`
/// in `server_app.rs` handles anonymous non-asteroid immediate entries
/// (stars, planets); the shared `is_owned_by_unified_pipeline` helper
/// guarantees no entry is spawned twice.
///
/// For named entries the UUID is read from `WorldConfig.name_to_uuid`
/// (populated by an earlier assign-uuid pass in this same system), so the
/// spawned `EntityUuid` component matches the UUID that trigger / comms
/// lookups resolve to. For asteroid-field entries a fresh UUID is allocated.
pub(crate) fn spawn_world_entities(
    mut commands: Commands,
    world_config: Option<ResMut<crate::world::config::WorldConfig>>,
    mut runtime: Option<ResMut<WorldContentRuntime>>,
    id_mint: crate::world_id::LiveMint<'_, { crate::world_id::IdNamespace::Entity as usize }>,
    script_gate: Option<Res<ScriptActivationGate>>,
) {
    if script_gate.is_some_and(|gate| gate.blocks_spawn("spawn_world_entities")) {
        return;
    }
    let Some(mut world_config) = world_config else {
        return; // No unified WorldConfig (native tests, hardcoded fallback).
    };

    // First pass (PRD #339 slice 2): assign UUIDs to every named [[entity]]
    // entry and register them in `WorldConfig.name_to_uuid` (and mirror
    // into `WorldContentRuntime.name_to_uuid` if present so trigger / comms
    // lookup paths see the same names). This pass runs independently of
    // template resolution so it works even when the config cache is empty
    // (e.g. in unit tests).
    let new_names = crate::world::config::assign_named_entity_uuids(&world_config.entities, || {
        crate::world_id::mint_live_id_with(id_mint.as_deref(), crate::world_id::IdNamespace::Entity)
    });
    for (name, uuid) in &new_names {
        world_config.name_to_uuid.insert(name.clone(), uuid.clone());
    }
    if let Some(runtime) = runtime.as_mut() {
        for (name, uuid) in &new_names {
            runtime.name_to_uuid.insert(name.clone(), uuid.clone());
        }
    }

    let config_cache = crate::entities::config_cache::get_config_cache();
    let world_snapshot = world_config.clone();
    // `ship_power` is seeded on `OnEnter(InProgress)` — not available at
    // Startup. Pass the runtime flags so any Immediate-path predicates that
    // don't depend on ship_power still evaluate correctly.
    let flags = runtime.as_ref().map(|r| &r.flags);
    let _spawned = spawn_immediate_entities_internal(
        &mut commands,
        &world_snapshot,
        &config_cache,
        flags,
        id_mint.as_deref(),
    );
}

/// Spawn the unified-pipeline-owned immediate `[[entity]]` instances.
///
/// Returns the list of spawned `Entity` handles in spawn order
/// (asteroid fields first, then named non-asteroid entries). Callers must
/// flush commands (e.g. via `app.update()`) before querying commands.
///
/// Extracted from `spawn_world_entities` so the spawn logic is testable
/// on native: tests pass a fixture `ConfigCache` (plain `HashMap`) directly
/// instead of relying on the WASM-only `CONFIG_CACHE` thread-local.
///
/// `flags` is the world flag/counter store used to evaluate optional `when`
/// predicates on entity entries. Pass `None` (or a store where `ship_power`
/// is unset) at Startup time — `ship_power` is seeded on
/// `OnEnter(GamePhase::InProgress)` before `spawn_game_start_entities`, so
/// `Immediate` entries evaluated here will see `ship_power = 0`.
pub fn spawn_immediate_entities_internal(
    commands: &mut Commands,
    world_config: &crate::world::config::WorldConfig,
    config_cache: &crate::entities::config_cache::ConfigCache,
    flags: Option<&crate::world::flags::FlagStore>,
    id_mint: Option<&crate::world_id::EntityMint>,
) -> Vec<Entity> {
    // Atomic-activation guard (issues #750/#752/#906/#969/#973): if this world's
    // composition is invalid, spawn NOTHING — a composition error must never
    // leave partial root-world content active. The headless build path aborts
    // earlier on the full composition; this seam is the last-resort gate for
    // the Bevy `Startup` spawn. `setup_world` in `server_app.rs` owns the OTHER
    // half of that spawn and consults the same gate, so a rejected world loses
    // both halves together.
    if world_activation_blocked(world_config, config_cache, "spawn_world_entities") {
        return Vec::new();
    }

    // The routing predicate asks the SAME lookup the spawn below performs
    // (issue #973 review): cache first, then the host loader. A cache-only
    // predicate answering `false` for a field template that is on disk but
    // uncached would push the entry into `setup_world`'s anonymous bucket,
    // where it spawns without its `[asteroid_field] anchor` resolved — a belt
    // silently sitting at the world origin. See
    // `entity_loader::template_is_asteroid_field`.
    let (fields, named, _anon) =
        crate::world::config::partition_immediate_entities_three_way(world_config, |path| {
            crate::entities::loader::template_is_asteroid_field(
                path,
                config_cache,
                &crate::entities::loader::WasmTemplateLoader,
            )
        });

    // Pre-resolve named-entity positions so `relative_to` references can be
    // looked up during spawn (PRD #337).
    let named_positions = crate::world::config::build_named_entity_positions(world_config);

    let mut spawned = Vec::with_capacity(fields.len() + named.len());

    // Helper: evaluate an optional `when` predicate against the flag store.
    let predicate_allows = |entity_inst: &crate::world::config::WorldEntity| -> bool {
        match &entity_inst.when_predicate {
            None => true,
            Some(pred) => {
                let empty = crate::world::flags::FlagStore::new();
                let store = flags.unwrap_or(&empty);
                pred.evaluate(&[store])
            }
        }
    };

    // Asteroid-field entries get a fresh UUID (they have no name to anchor to).
    for entity_inst in fields {
        if !predicate_allows(entity_inst) {
            continue;
        }
        let mut config = match crate::entities::loader::resolve_entity_via(
            entity_inst,
            config_cache,
            &crate::entities::loader::WasmTemplateLoader,
        ) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "spawn_world_entities: failed to resolve asteroid field '{}': {}",
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };
        // Resolve optional `anchor` reference into a concrete world-space offset
        // applied to the streaming spawner. Missing anchor → warn + fall back
        // to world origin so a typo never silently relocates the field.
        if let Some(field) = config.asteroid_field.as_mut() {
            if let Some(anchor_name) = field.anchor.as_ref() {
                match world_config.anchors.get(anchor_name) {
                    Some(pos) => field.anchor_offset = *pos,
                    None => {
                        bevy::log::warn!(
                            "spawn_world_entities: asteroid field '{}' references unknown anchor '{}' — falling back to world origin",
                            entity_inst.template_path, anchor_name
                        );
                        field.anchor_offset = [0.0, 0.0, 0.0];
                    }
                }
            }
        }
        let uuid =
            crate::world_id::mint_live_id_with(id_mint, crate::world_id::IdNamespace::Entity);
        let pos = match resolve_position(entity_inst, &world_config.anchors, &named_positions) {
            Ok(p) => p,
            Err(e) => {
                bevy::log::error!("spawn_world_entities: {e}");
                continue;
            }
        };
        if let Err(reason) = config.validate_field_at_origin(pos.to_array()) {
            bevy::log::error!("spawn_world_entities: invalid field at resolved position: {reason}");
            continue;
        }
        let entity = crate::entities::spawner::spawn_entity(
            commands,
            &config,
            pos,
            uuid,
            entity_inst.id.clone(),
        );
        commands
            .entity(entity)
            .insert(crate::entities::spawner::EntityTemplatePath::new(
                &entity_inst.template_path,
            ));
        spawned.push(entity);
    }

    // Named non-asteroid entries MUST use the UUID already registered in
    // `world_config.name_to_uuid` so triggers / comms resolve to a real
    // entity. A missing registration is a programmer error — log and skip
    // rather than allocate a fresh UUID (which would silently desync).
    for entity_inst in named {
        if !predicate_allows(entity_inst) {
            continue;
        }
        let name = entity_inst
            .name
            .as_ref()
            .expect("partition guarantees Some");
        let uuid = match world_config.name_to_uuid.get(name) {
            Some(u) => u.clone(),
            None => {
                bevy::log::error!(
                    "spawn_world_entities: named entity '{}' has no UUID in WorldConfig.name_to_uuid — skipping",
                    name
                );
                continue;
            }
        };
        let config = match crate::entities::loader::resolve_entity_via(
            entity_inst,
            config_cache,
            &crate::entities::loader::WasmTemplateLoader,
        ) {
            Ok(c) => c,
            Err(e) => {
                bevy::log::error!(
                    "spawn_world_entities: failed to resolve named entity '{}' ({}): {}",
                    name,
                    entity_inst.template_path,
                    e
                );
                continue;
            }
        };
        let pos = match resolve_position(entity_inst, &world_config.anchors, &named_positions) {
            Ok(p) => p,
            Err(e) => {
                bevy::log::error!("spawn_world_entities: named entity '{name}': {e}");
                continue;
            }
        };
        let mut config = config;
        if entity_inst.name.is_some() {
            config.name = entity_inst.name.clone();
        }
        let initial_custody = match crate::entities::craft::initial_location(
            world_config,
            entity_inst,
            &config,
            false,
        ) {
            Ok(location) => location,
            Err(error) => {
                error!("Immediate carried craft refused before materialization: {error}");
                continue;
            }
        };
        if let Err(reason) = config.validate_field_at_origin(pos.to_array()) {
            bevy::log::error!("spawn_world_entities: named entity '{name}': {reason}");
            continue;
        }
        let entity = crate::entities::spawner::spawn_entity(
            commands,
            &config,
            pos,
            uuid,
            entity_inst.id.clone(),
        );
        commands
            .entity(entity)
            .insert(crate::entities::spawner::EntityTemplatePath::new(
                &entity_inst.template_path,
            ));
        apply_authored_spawn_transform(commands, entity, pos, entity_inst.transform.as_ref());
        crate::entities::craft::stage_initial(commands, entity, initial_custody);
        // The authored narrative mark (issue #1338). Attached here rather than
        // in a `SpawnSection` because the payload is the WORLD's authored
        // `name` — the unique reference id triggers, comms and objectives all
        // address this instance by — and the entity template knows nothing
        // about it. Only this named branch can mark: an anonymous or
        // asteroid-field instance has no name to carry.
        if entity_inst.narrative {
            commands
                .entity(entity)
                .insert(crate::core::narrative::NarrativeMark(name.clone()));
        }
        spawned.push(entity);
    }

    spawned
}

/// Resolve an `[[entity]]` instance's spawn position via the pure
/// `world::config::resolve_entity_position` helper, then widen to a Bevy `Vec3`.
///
/// Centralises position resolution for the unified pipeline so anchor-named
/// entries (PRD #337 slice 3) share the same code path as inline-position
/// entries.
pub(super) fn resolve_position(
    entity_inst: &crate::world::config::WorldEntity,
    anchors: &HashMap<String, [f32; 3]>,
    entities_by_name: &HashMap<String, [f32; 3]>,
) -> Result<Vec3, String> {
    let pos =
        crate::world::config::resolve_entity_position_with(entity_inst, anchors, entities_by_name)?;
    Ok(Vec3::new(pos[0], pos[1], pos[2]))
}

/// Apply a non-player World's authored pose after the ordinary capability
/// inserts. Existing physical machinery owns its yaw; static scenery keeps
/// only its Transform. This is the same sign convention as scripted spawns.
pub(crate) fn apply_authored_spawn_transform(
    commands: &mut Commands,
    entity: Entity,
    position: Vec3,
    authored: Option<&crate::world::config::TransformConfig>,
) {
    let Some(authored) = authored else { return };
    if authored.rotation.is_some() || authored.scale.is_some() {
        commands.entity(entity).insert(Transform {
            translation: position,
            rotation: authored.quat(),
            scale: authored.scale_vec(),
        });
    }
    if let Some([_, transform_yaw, _]) = authored.rotation {
        commands
            .entity(entity)
            .queue(move |mut entity: bevy::ecs::world::EntityWorldMut| {
                if let Some(mut physics) = entity.get_mut::<crate::ship::state::ShipPhysics>() {
                    physics.yaw = -transform_yaw;
                }
            });
    }
}
