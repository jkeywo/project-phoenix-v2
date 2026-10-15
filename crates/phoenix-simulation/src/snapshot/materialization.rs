//! Materialization.
use super::*;

/// Exercise the same spawn recipe in a disposable World before touching the
/// live roster, layer membership or run clocks. No gameplay schedules run here.
pub(super) fn preflight_missing_entities(
    world: &World,
    snapshot: &PhoenixSnapshot,
    report: &mut RestoreReport,
) {
    let present: Vec<_> = world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>()
        .map(|mut query| {
            query
                .iter(world)
                .map(|(entity, uuid)| {
                    (
                        uuid.0.clone(),
                        entity,
                        world
                            .get::<crate::ship::compartment_damage::ShipDamage>(entity)
                            .cloned(),
                        world.get::<ShipRepairTeams>(entity).cloned(),
                        world
                            .get::<crate::resources::server::ResourceServices>(entity)
                            .cloned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let missing: Vec<_> = snapshot
        .entities
        .iter()
        .filter(|row| !present.iter().any(|(uuid, ..)| uuid == &row.uuid) && row.spawn.is_some())
        .collect();
    let mut staging = World::new();
    if let Some(fixed) = world.get_resource::<Time<Fixed>>() {
        staging.insert_resource(fixed.clone());
    }
    // Only immutable authored shape/ownership is seeded. Saved stock and job
    // payloads remain the candidate state, never mutable staging simulation.
    for (uuid, live_entity, damage, teams, services) in &present {
        let mut entity = staging.spawn(EntityUuid(uuid.clone()));
        super::ports::seed_shape(world, *live_entity, &mut entity);
        if let Some(damage) = damage {
            entity.insert(damage.clone());
        }
        if let Some(teams) = teams {
            entity.insert(teams.clone());
        }
        if let Some(services) = services {
            entity.insert(services.clone());
        }
    }
    let mut staged = Vec::new();
    for row in missing {
        match spawn_from_origin(
            &mut staging,
            row,
            row.spawn.as_ref().expect("filtered origin"),
        ) {
            Some(entity) => staged.push((entity, row)),
            None => report
                .gaps
                .push(RestoreGap::MissingEntity(row.uuid.clone())),
        }
    }
    for row in snapshot.entities.iter().filter(|row| {
        row.weapon_flight_carrier && !present.iter().any(|(uuid, ..)| uuid == &row.uuid)
    }) {
        let entity =
            crate::console::weapons::flight_carrier::spawn_empty(&mut staging, row.uuid.clone());
        crate::console::weapons::flight_carrier::seed_banks(
            &mut staging,
            entity,
            &row.carrier_bolt_banks,
        );
        staged.push((entity, row));
    }
    // Existing attackers can retain samples of targets that are materialized by
    // this restore. Validate all attacks only after every target shape exists.
    for row in &snapshot.entities {
        if row
            .recovery
            .as_ref()
            .is_some_and(|state| state.last_tick > snapshot.tick)
        {
            report.gaps.push(RestoreGap::ResourceStateInvalid {
                uuid: row.uuid.clone(),
                reason: "saved recovery work is newer than the captured simulation tick".into(),
            });
        }
        if row.production.as_ref().is_some_and(|state| {
            state.last_tick > snapshot.tick
                || state
                    .active
                    .as_ref()
                    .is_some_and(|batch| batch.born_tick > snapshot.tick)
        }) {
            report.gaps.push(RestoreGap::ResourceStateInvalid {
                uuid: row.uuid.clone(),
                reason: "saved production work is newer than the captured simulation tick".into(),
            });
        }
        if row.logistics.as_ref().is_some_and(|state| {
            state.last_tick > snapshot.tick
                || state
                    .batches
                    .values()
                    .any(|batch| batch.born_tick > snapshot.tick)
        }) {
            report.gaps.push(RestoreGap::ResourceStateInvalid {
                uuid: row.uuid.clone(),
                reason: "saved logistics work is newer than the captured simulation tick".into(),
            });
        }
        if let Err(reason) = super::preflight::validate_physical_attacks(&staging, row) {
            report.gaps.push(RestoreGap::ResourceStateInvalid {
                uuid: row.uuid.clone(),
                reason,
            });
        }
    }
    if let Err((uuid, reason)) = super::preflight::validate_cross_entity_repairs(&staging, snapshot)
    {
        report
            .gaps
            .push(RestoreGap::RepairJobsInvalid { uuid, reason });
    }
    if let Err((uuid, reason)) = super::ports::validate_candidate(&staging, snapshot) {
        report
            .gaps
            .push(RestoreGap::ResourceStateInvalid { uuid, reason });
    }
    for (entity, row) in staged {
        if let Err(reason) = super::application::validate_entity_repair_jobs(&staging, entity, row)
        {
            report.gaps.push(RestoreGap::RepairJobsInvalid {
                uuid: row.uuid.clone(),
                reason,
            });
        }
        if let Err(reason) = super::preflight::validate_entity_resources(&staging, entity, row) {
            report.gaps.push(RestoreGap::ResourceStateInvalid {
                uuid: row.uuid.clone(),
                reason,
            });
        }
    }
}

/// Overwrite each captured entity's state, despawning anything the capture did
/// not have.
pub(super) fn restore_entities(
    world: &mut World,
    snapshot: &PhoenixSnapshot,
    report: &mut RestoreReport,
) {
    let restored_at = fixed_elapsed_secs(world).unwrap_or_default();
    // Alert, attacker and stored stances are replaced together, not changed by
    // a Captain command. Consume any bootstrap alert edge, including one no
    // reader has seen yet. Bevy clamps both ages to MAX_CHANGE_AGE and compares
    // them strictly, so this rollback baseline is old for every reader, even
    // across tick wrap. A subsequent real write still stamps the current tick.
    let restored_alert_tick = bevy::ecs::change_detection::Tick::new(
        world
            .read_change_tick()
            .get()
            .wrapping_sub(bevy::ecs::change_detection::MAX_CHANGE_AGE),
    );
    let Some(mut query) = world
        .try_query_filtered::<(Entity, &EntityUuid), Allow<bevy::ecs::entity_disabling::Disabled>>(
        )
    else {
        return;
    };
    let present: Vec<(Entity, String)> = query
        .iter(world)
        .map(|(entity, uuid)| (entity, uuid.0.clone()))
        .collect();

    let mut surplus = Vec::new();
    let mut writes: Vec<(Entity, EntityState)> = Vec::new();
    for (entity, uuid) in &present {
        match snapshot.entities.iter().find(|row| &row.uuid == uuid) {
            Some(row) => writes.push((*entity, row.clone())),
            None => surplus.push(*entity),
        }
    }

    // Issue #863. A row with nothing to write into is either a ship the
    // bootstrap will never make — a mid-run spawn — or a genuine gap, and the
    // difference is whether the capture recorded what the spawn was made from.
    // Built here rather than waited for: waiting is what a fresh app booted with
    // nobody at the consoles does forever.
    for row in &snapshot.entities {
        if present.iter().any(|(_, uuid)| uuid == &row.uuid) {
            continue;
        }
        match if row.weapon_flight_carrier {
            let entity =
                crate::console::weapons::flight_carrier::spawn_empty(world, row.uuid.clone());
            crate::console::weapons::flight_carrier::seed_banks(
                world,
                entity,
                &row.carrier_bolt_banks,
            );
            Some(entity)
        } else {
            row.spawn
                .as_ref()
                .and_then(|origin| spawn_from_origin(world, row, origin))
        } {
            Some(entity) => {
                report.entities_spawned += 1;
                writes.push((entity, row.clone()));
            }
            None => report
                .gaps
                .push(RestoreGap::MissingEntity(row.uuid.clone())),
        }
    }

    // Both kinds together: `entities_restored` is "captured rows that found a
    // home", and a row that found a home this restore *built* is as restored as
    // one that found a bootstrapped ship waiting.
    report.entities_restored = writes.len();

    for (entity, row) in &writes {
        if let Err(reason) = super::preflight::validate_entity_resources(world, *entity, row) {
            report.gaps.push(RestoreGap::ResourceStateInvalid {
                uuid: row.uuid.clone(),
                reason,
            });
        }
        if let Err(reason) = validate_entity_repair_jobs(world, *entity, row) {
            report.gaps.push(RestoreGap::RepairJobsInvalid {
                uuid: row.uuid.clone(),
                reason,
            });
        }
    }
    if !report.gaps.is_empty() {
        report.entities_restored = 0;
        return;
    }
    for (entity, row) in writes {
        apply_entity_state_with(
            world,
            entity,
            &row,
            snapshot.tick,
            restored_at,
            restored_alert_tick,
        );
    }

    super::craft::restore_presence(world, snapshot);
    report.despawned += surplus.len();
    for entity in surplus {
        if let Ok(entity_mut) = world.get_entity_mut(entity) {
            entity_mut.despawn();
        }
    }
}

/// Rebuild one mid-run spawn the bootstrapped world does not have (issue #863).
///
/// The same three steps the live spawn took, through the same two functions, so
/// a rebuilt ship and a streamed-in one cannot come out different: resolve the
/// template and overrides ([`SpawnOrigin::resolve`]), build the entity
/// ([`crate::entities::spawner::spawn_entity`]), then re-declare the layer
/// ownership and the placement the applier's own arm declares.
///
/// `None` — and therefore a [`RestoreGap::MissingEntity`] — when the template
/// does not resolve. That is the honest answer for the same reason
/// [`restore_asteroids`] gives it for a rock with no config path: an entity
/// built without its template is one with an invented collider and no weapons,
/// and a resumed world carrying one is worse than a resumed world that says what
/// it could not rebuild.
///
/// # Why the fidelity tier is read off the row
///
/// The helm axes are not spawn components — they arrive with
/// `ai_high_fidelity_components()` when the LOD system promotes a ship inside the
/// player's bubble — so a freshly built ship has none of them and every
/// [`ControlState`] write below would land nowhere. The capture answers that
/// explicitly through [`EntityState::ai_fidelity`], rather than inferring a tier
/// from one bundle member. So the tier is *read*, not guessed, and the canonical
/// bundle is present before the ordinary entity restore applies its continuations.
pub(crate) fn spawn_from_origin(
    world: &mut World,
    row: &EntityState,
    origin: &crate::world::spawn_origin::SpawnOrigin,
) -> Option<Entity> {
    let mut warnings = Vec::new();
    let config = origin.resolve(&crate::entities::loader::WasmTemplateLoader, &mut warnings);
    for warning in &warnings {
        bevy::log::warn!(
            target: crate::logging::LogCat::World.target(),
            "snapshot restore: {warning}"
        );
    }
    let config = config?;

    let position = Vec3::new(origin.position[0], origin.position[1], origin.position[2]);
    let entity = {
        let mut commands = world.commands();
        crate::entities::spawner::spawn_entity(
            &mut commands,
            &config,
            position,
            row.uuid.clone(),
            None,
        )
    };
    // `spawn_entity` builds through `Commands`, so the components are queued
    // rather than present. Everything below — and every state write in
    // `restore_entities` afterwards — reads the entity directly, so the queue
    // has to be applied before any of it runs.
    world.flush();

    let mut entity_mut = world.entity_mut(entity);
    // The applier's own placement step, on its terms: `spawn_entity` set the
    // translation, and a rotation or scale replaces the whole `Transform`
    // through the canonical `TransformConfig` conversions rather than a second
    // Euler expression.
    if origin.rotation.is_some() || origin.scale.is_some() {
        let transform_config = crate::world::config::TransformConfig {
            rotation: origin.rotation,
            scale: origin.scale,
            ..Default::default()
        };
        entity_mut.insert(Transform {
            translation: position,
            rotation: transform_config.quat(),
            scale: transform_config.scale_vec(),
        });
    }
    // Put the record back on the ship it describes, so a save taken *after* this
    // resume can rebuild it again. A resumed run that could only be resumed once
    // is a continuation with an expiry date.
    entity_mut.insert((
        crate::entities::spawner::EntityTemplatePath::new(&origin.template_path),
        crate::entities::spawner::EntitySpawnOrigin(origin.clone()),
    ));
    if row
        .ai_fidelity
        .as_ref()
        .is_some_and(|fidelity| fidelity.high_fidelity)
    {
        entity_mut.insert(crate::ai::server::ai_high_fidelity_components());
    }

    if let Some(path) = &origin.layer_path {
        // Layer ownership is what makes `UnloadWorld` despawn a layer's ad-hoc
        // spawns. Declared only when the target world actually has that layer
        // loaded — the applier's own guard — because pushing a handle into a
        // layer the resumed world never loaded would record an ownership nothing
        // will ever act on.
        let owned = world
            .get_resource_mut::<crate::world::server::WorldLayerMap>()
            .and_then(|mut layers| {
                layers.0.get_mut(path).map(|layer| {
                    layer.spawned_entities.push(entity);
                })
            })
            .is_some();
        if owned {
            world
                .entity_mut(entity)
                .insert(crate::world::server::EntityOriginLayer(path.clone()));
        }
    }

    Some(entity)
}

/// Overwrite the streamed belt: keep the rocks the capture knows, despawn the
/// ones it does not, and **spawn** the ones the target world never streamed.
///
/// The spawn half is what makes a restore authoritative over the belt rather
/// than merely corrective, and it is what Combat Test needs. A capture taken
/// after the player has flown somewhere names rocks whose cells the fresh app —
/// bootstrapped at the spawn point and stepped only far enough to raise its
/// roster — has never had in window. Reporting those as
/// [`RestoreGap::MissingAsteroid`]s and carrying on would leave the resumed
/// world short of exactly the rocks the capture's digest counted, so the digest
/// would not match and nothing in the save would say why.
///
/// A rock is spawned through `asteroid_lifecycle::rock_bundle` — the same
/// component set the streamer itself builds — so a restored rock and a streamed
/// one are the same entity, down to the `ColliderSection` that collision
/// avoidance reads an obstacle's size from.
pub(super) fn restore_asteroids(
    world: &mut World,
    snapshot: &PhoenixSnapshot,
    report: &mut RestoreReport,
) {
    let Some(mut query) = world.try_query::<(Entity, &AsteroidUuid)>() else {
        return;
    };
    let present: Vec<(Entity, String)> = query
        .iter(world)
        .map(|(entity, uuid)| (entity, uuid.0.clone()))
        .collect();

    let mut surplus = Vec::new();
    let mut writes: Vec<(Entity, AsteroidState)> = Vec::new();
    for (entity, uuid) in &present {
        match snapshot.asteroids.iter().find(|row| &row.uuid == uuid) {
            Some(row) => writes.push((*entity, row.clone())),
            None => surplus.push((*entity, uuid.clone())),
        }
    }
    let missing: Vec<AsteroidState> = snapshot
        .asteroids
        .iter()
        .filter(|row| !present.iter().any(|(_, uuid)| uuid == &row.uuid))
        .cloned()
        .collect();

    report.asteroids_restored = writes.len();

    for (entity, row) in writes {
        let mut entity_mut = world.entity_mut(entity);
        if let Some(mut transform) = entity_mut.get_mut::<Transform>() {
            transform.translation =
                Vec3::new(row.translation[0], row.translation[1], row.translation[2]);
            // Not re-normalised. The stored quaternion came off a live
            // `Transform` and is already unit; normalising it again is a
            // divide that moves the low bits, and the capture this restore is
            // checked against folds bit patterns.
            transform.rotation = Quat::from_xyzw(
                row.rotation[0],
                row.rotation[1],
                row.rotation[2],
                row.rotation[3],
            );
        }
        if let Some(rows) = &row.hull {
            if let Some(mut hull) = entity_mut.get_mut::<EntitySystemHull>() {
                apply_hull(&mut hull.0, rows);
            }
        }
    }

    for (entity, uuid) in &surplus {
        if let Ok(entity_mut) = world.get_entity_mut(*entity) {
            entity_mut.despawn();
        }
        if let Some(mut map) = world.get_resource_mut::<AsteroidEntityMap>() {
            map.0.remove(uuid);
        }
    }
    report.despawned += surplus.len();

    for row in &missing {
        // Without a config path there is nothing to build the rock *from* — no
        // collider, no hull maximum, no mesh — so this stays the honest gap it
        // always was rather than becoming a rock with invented dimensions. In
        // practice it means a hand-placed rock the target world does not have,
        // which is a scenario difference the content digest is the answer to.
        let Some(config_path) = row.config_path.as_deref() else {
            report
                .gaps
                .push(RestoreGap::MissingAsteroid(row.uuid.clone()));
            continue;
        };
        let config = crate::asteroids::lifecycle::rock_config(config_path);
        let current_hp = row
            .hull
            .as_ref()
            .and_then(|rows| rows.first().map(|(_, current, _)| *current))
            .unwrap_or(config.max_hp);
        let mut spawned = world.spawn(crate::asteroids::lifecycle::rock_bundle(
            &row.uuid,
            &config,
            Vec3::new(row.translation[0], row.translation[1], row.translation[2]),
            Quat::from_xyzw(
                row.rotation[0],
                row.rotation[1],
                row.rotation[2],
                row.rotation[3],
            ),
            row.shield_pierce,
            current_hp,
        ));
        if let Some(mesh) = &config.mesh {
            spawned.insert(crate::entities::spawner::MeshSection(mesh.clone()));
        }
        let entity = spawned.id();
        if let Some(mut map) = world.get_resource_mut::<AsteroidEntityMap>() {
            map.0.insert(row.uuid.clone(), entity);
        }
        report.asteroids_restored += 1;
    }

    restore_asteroid_window(world, snapshot);
}

/// Put the streamer's own progress back, so its next tick resumes rather than
/// rebuilding. See [`AsteroidWindowState`].
pub(super) fn restore_asteroid_window(world: &mut World, snapshot: &PhoenixSnapshot) {
    let Some(stored) = snapshot.asteroid_window.as_ref() else {
        return;
    };
    // Cosmetic handles belong to the app that spawned them, and the arena the
    // restore is about to install may not be the one their slots were indexed
    // against. They carry no uuid, no hull and no collider, so despawning them
    // costs a frame of set dressing and buys a window whose every remaining
    // handle is one this restore put there.
    let cosmetics: Vec<Entity> = world
        .get_resource::<AsteroidWindow>()
        .map(|window| {
            window
                .cosmetic_upper_slots
                .iter()
                .chain(window.cosmetic_lower_slots.iter())
                .flatten()
                .flatten()
                .copied()
                .collect()
        })
        .unwrap_or_default();
    for entity in cosmetics {
        if let Ok(entity_mut) = world.get_entity_mut(entity) {
            entity_mut.despawn();
        }
    }

    let Some(mut window) = world.get_resource_mut::<AsteroidWindow>() else {
        return;
    };
    let size = (2 * stored.despawn_cells + 1) as usize;
    window.slots = vec![vec![None; size]; size];
    window.cosmetic_upper_slots = vec![vec![None; size]; size];
    window.cosmetic_lower_slots = vec![vec![None; size]; size];
    for slot in &stored.slots {
        let (x, z) = (slot.x as usize, slot.z as usize);
        let Some(cell) = window.slots.get_mut(z).and_then(|row| row.get_mut(x)) else {
            continue;
        };
        *cell = Some(AsteroidData {
            uuid: slot.uuid.clone(),
            config_path: slot.config_path.clone(),
            hp: slot.hp,
            max_hp: slot.max_hp,
            y: slot.y,
        });
    }
    window.arena_gx = stored.arena_gx;
    window.arena_gz = stored.arena_gz;
    window.despawn_cells = stored.despawn_cells;
    window.spawn_cells = stored.spawn_cells;
    window.resolution = stored.resolution;
    window.player_grid = stored.player_grid;
    window.composition_key = stored.composition_key;
    window.needs_init = stored.needs_init;
}

/// Terminal state of [`reconcile_world_layers`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerReconcileStatus {
    /// Ordered active topology and declared layer-entity identities are exact.
    Ready,
    /// A deterministic load/unload is queued or already in flight. Run another
    /// simulation update and call the reconciler again.
    Waiting,
    /// `path` is a desired layer whose fresh load reached a failed sentinel (or
    /// whose captured ownership/identity cannot be reconstructed safely).
    Failed(String),
}

/// Reconcile the fresh bootstrap's derived layer runtime to the capture.
///
/// Call this between simulation updates, before [`ready_to_restore`] or
/// [`ready_to_rebuild`]. It queues deterministic layer changes through the same
/// `PendingWorldLayerChanges` boundary gameplay uses: the first divergent live
/// suffix unloads newest-first, then missing layers load one at a time in the
/// capture's activation order with their captured loader ownership.
///
/// Successful activations mint declared layer entities again. Once topology is
/// exact this function also exchanges those fresh UUIDs for the captured UUIDs,
/// before the roster readiness predicate runs. Script-spawned entities are not
/// remapped here: they already carry `EntitySpawnOrigin` recipes and use the
/// ordinary rebuild path.
pub fn reconcile_world_layers(
    world: &mut World,
    snapshot: &PhoenixSnapshot,
) -> LayerReconcileStatus {
    let saved = saved_layer_topology(snapshot);
    let live = live_layer_topology(world);

    if pending_layer_changes(world) != 0 {
        return LayerReconcileStatus::Waiting;
    }

    // Sentinels are loader bookkeeping, not captured composition. Remove every
    // non-target sentinel so it cannot survive a resume as an `AlreadyLoaded`
    // dedup blocker. A sentinel at a DESIRED path is handled below as terminal:
    // retrying identical content would only reproduce the same failure forever.
    let desired_paths: std::collections::HashSet<&str> =
        saved.iter().map(|layer| layer.path.as_str()).collect();
    let non_target_sentinels: Vec<String> = inactive_layer_paths(world)
        .into_iter()
        .filter(|path| !desired_paths.contains(path.as_str()))
        .collect();
    if !non_target_sentinels.is_empty() {
        queue_layer_changes(
            world,
            non_target_sentinels
                .into_iter()
                .map(crate::world::server::WorldLayerChange::Unload),
        );
        return LayerReconcileStatus::Waiting;
    }

    if saved == live {
        return if reconcile_layer_entity_uuids(world, snapshot) {
            LayerReconcileStatus::Ready
        } else {
            LayerReconcileStatus::Failed("declared layer entity identity".to_string())
        };
    }

    let common_prefix = saved
        .iter()
        .zip(&live)
        .take_while(|(saved, live)| saved == live)
        .count();

    if common_prefix < live.len() {
        let unloads: Vec<String> = live[common_prefix..]
            .iter()
            .rev()
            .map(|layer| layer.path.clone())
            .collect();
        queue_layer_changes(
            world,
            unloads
                .into_iter()
                .map(crate::world::server::WorldLayerChange::Unload),
        );
        return LayerReconcileStatus::Waiting;
    }

    let Some(next) = saved.get(common_prefix) else {
        return LayerReconcileStatus::Waiting;
    };

    // A refused desired load is terminal. The sentinel proves this bootstrap
    // already attempted the exact path; deleting it and retrying cannot repair
    // parse/compile/fetch failure and would turn a named refusal into a loop.
    let has_failed_sentinel = world
        .get_resource::<crate::world::server::WorldLayerMap>()
        .and_then(|layers| layers.0.get(&next.path))
        .is_some_and(|runtime| !runtime.is_active);
    if has_failed_sentinel {
        return LayerReconcileStatus::Failed(next.path.clone());
    }

    // A captured child can only have activated after its loader. Refusing an
    // impossible/corrupt ordering is safer than silently loading it at base
    // scope and realigning trigger indices against another flag chain.
    if let Some(loader) = &next.loader_path {
        if !live[..common_prefix]
            .iter()
            .any(|layer| &layer.path == loader)
        {
            return LayerReconcileStatus::Failed(next.path.clone());
        }
    }

    queue_layer_changes(
        world,
        std::iter::once(crate::world::server::WorldLayerChange::Load {
            path: next.path.clone(),
            loader_path: next.loader_path.clone(),
        }),
    );
    LayerReconcileStatus::Waiting
}

pub(super) fn queue_layer_changes(
    world: &mut World,
    changes: impl IntoIterator<Item = crate::world::server::WorldLayerChange>,
) {
    if world
        .get_resource::<crate::world::server::PendingWorldLayerChanges>()
        .is_none()
    {
        world.insert_resource(crate::world::server::PendingWorldLayerChanges::default());
    }
    world
        .resource_mut::<crate::world::server::PendingWorldLayerChanges>()
        .0
        .extend(changes);
}

pub(super) fn reconcile_layer_entity_uuids(world: &mut World, snapshot: &PhoenixSnapshot) -> bool {
    let current_entities: Vec<(Entity, String)> = {
        let mut query = world.query::<(Entity, &EntityUuid)>();
        query
            .iter(world)
            .map(|(entity, uuid)| (entity, uuid.0.clone()))
            .collect()
    };
    let current_by_uuid: std::collections::HashMap<String, Entity> = current_entities
        .iter()
        .map(|(entity, uuid)| (uuid.clone(), *entity))
        .collect();

    let mut remaps: Vec<(Entity, String)> = Vec::new();
    let mut add_remap = |entity: Entity, desired: &str| -> bool {
        if let Some((_, existing)) = remaps.iter().find(|(found, _)| *found == entity) {
            return existing == desired;
        }
        if remaps
            .iter()
            .any(|(found, uuid)| *found != entity && uuid == desired)
        {
            return false;
        }
        remaps.push((entity, desired.to_string()));
        true
    };

    {
        let Some(layers) = world.get_resource::<crate::world::server::WorldLayerMap>() else {
            return snapshot.layer_flags.is_empty();
        };
        for saved in &snapshot.layer_flags {
            let Some(live) = layers.0.get(&saved.path) else {
                return false;
            };
            for (slot, desired) in saved.declared_entity_uuids.iter().enumerate() {
                let Some(desired) = desired else {
                    continue;
                };
                let Some(entity) = live.spawned_entities.get(slot).copied() else {
                    return false;
                };
                if !add_remap(entity, desired) {
                    return false;
                }
            }
        }
    }

    // Named entities give the same identity exchange a second, independent key
    // and cover any minted base entity whose UUID collides with a layer's saved
    // one after the fresh load happened at another point in the run.
    if let Some(saved_scenario) = &snapshot.scenario {
        let live_names = world
            .get_resource::<WorldContentRuntime>()
            .map(|runtime| runtime.name_to_uuid.clone())
            .unwrap_or_default();
        for (name, desired) in &saved_scenario.name_to_uuid {
            let Some(current) = live_names.get(name) else {
                continue;
            };
            let Some(entity) = current_by_uuid.get(current).copied() else {
                continue;
            };
            if !add_remap(entity, desired) {
                return false;
            }
        }
    }

    remaps.sort_by_key(|(entity, _)| entity.index());
    let remapped_entities: std::collections::HashSet<Entity> =
        remaps.iter().map(|(entity, _)| *entity).collect();
    let mut displaced: Vec<Entity> = remaps
        .iter()
        .filter_map(|(entity, desired)| {
            current_by_uuid
                .get(desired)
                .copied()
                .filter(|holder| holder != entity && !remapped_entities.contains(holder))
        })
        .collect();
    displaced.sort_by_key(|entity| entity.index());
    displaced.dedup();

    let current_uuid_by_entity: std::collections::HashMap<Entity, String> = current_entities
        .iter()
        .map(|(entity, uuid)| (*entity, uuid.clone()))
        .collect();
    remaps.retain(|(entity, desired)| current_uuid_by_entity.get(entity) != Some(desired));
    if remaps.is_empty() && displaced.is_empty() {
        return true;
    }

    // Move every participant off the UUID namespace first. This makes swaps and
    // cycles safe and prevents the transient duplicate that a one-by-one rename
    // would create when mint order shifted several named entities at once.
    let mut used: std::collections::HashSet<String> = current_entities
        .iter()
        .map(|(_, uuid)| uuid.clone())
        .collect();
    used.extend(remaps.iter().map(|(_, desired)| desired.clone()));
    let mut temporary_index = 0u64;
    for entity in remaps
        .iter()
        .map(|(entity, _)| *entity)
        .chain(displaced.iter().copied())
    {
        let temporary = loop {
            let candidate = format!("__snapshot_layer_reconcile_{temporary_index}");
            temporary_index += 1;
            if used.insert(candidate.clone()) {
                break candidate;
            }
        };
        world.entity_mut(entity).insert(EntityUuid(temporary));
    }
    for (entity, desired) in &remaps {
        world
            .entity_mut(*entity)
            .insert(EntityUuid(desired.clone()));
    }

    if let (Some(saved_scenario), Some(mut runtime)) = (
        snapshot.scenario.as_ref(),
        world.get_resource_mut::<WorldContentRuntime>(),
    ) {
        runtime.name_to_uuid = saved_scenario.name_to_uuid.iter().cloned().collect();
    }
    true
}
