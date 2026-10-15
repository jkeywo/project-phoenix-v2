//! Preflight.
use super::*;

/// Validate the saved GameStart identity map against the world that will boot.
///
/// Format validation above can prove order, namespace and UUID uniqueness without
/// loading content. Once the selected scenario is available, this second gate
/// proves every authored index still exists and still names a `GameStart` row.
/// It deliberately does not require the UUID to occur in the captured entity
/// roster: a GameStart entity destroyed before the save is absent there, but its
/// original identity is still required so the fresh bootstrap can spawn and the
/// restore can recognise that entity as surplus and despawn it again.
pub fn validate_boot_identity_for_world(
    identity: &BootIdentity,
    world_config: &crate::world::config::WorldConfig,
) -> Result<(), LoadRefusal> {
    let mut slots = std::collections::BTreeSet::new();
    let mut uuids = std::collections::BTreeSet::new();
    for row in &identity.game_start_entity_uuids {
        if !uuids.insert(&row.entity_uuid) {
            return Err(LoadRefusal::Unparsable(
                "duplicate GameStart actor UUID".into(),
            ));
        }
        if let Some(slot) = &row.authored_slot_id {
            if !slots.insert(slot) {
                return Err(LoadRefusal::Unparsable(
                    "duplicate authored launch slot in GameStart identity".into(),
                ));
            }
        }
        let authored_index = row.authored_index as usize;
        let Some(authored) = world_config.entities.get(authored_index) else {
            return Err(LoadRefusal::Unparsable(format!(
                "the current snapshot boot identity names missing authored entity row {} in a world with {} row(s)",
                row.authored_index,
                world_config.entities.len()
            )));
        };
        if authored.spawn_on != crate::world::config::WorldEntitySpawnOn::GameStart {
            return Err(LoadRefusal::Unparsable(format!(
                "the current snapshot boot identity names authored entity row {}, which is not a GameStart row",
                row.authored_index
            )));
        }
    }
    // Optional for legacy/reduced boot records, required by any situation that
    // uses a slot. Validate provided links from accepted authored rows and the
    // saved inclusion frontier, never from arbitrary current actors.
    if !slots.is_empty() {
        let cache = crate::entities::config_cache::get_config_cache();
        let catalog = world_config.effective_ship_slots();
        let mut authored_ship_index = 0;
        let mut included_ship_index = 0;
        for (index, entity) in world_config.entities.iter().enumerate() {
            if entity.spawn_on != crate::world::config::WorldEntitySpawnOn::GameStart {
                continue;
            }
            let config = crate::entities::loader::resolve_entity_via(
                entity,
                &cache,
                &crate::entities::loader::WasmTemplateLoader,
            )
            .map_err(|error| {
                LoadRefusal::Unparsable(format!(
                    "cannot resolve authored launch row {index}: {error}"
                ))
            })?;
            let ship = config.tags.iter().any(|tag| tag == "ship");
            let authored_slot = ship
                .then(|| world_config.ship_slots.get(authored_ship_index))
                .flatten();
            if ship {
                authored_ship_index += 1;
            }
            let Some(saved) = identity
                .game_start_entity_uuids
                .iter()
                .find(|row| row.authored_index as usize == index)
            else {
                continue;
            };
            let expected = authored_slot.or_else(|| {
                (ship && world_config.ship_slots.is_empty())
                    .then(|| catalog.get(included_ship_index))
                    .flatten()
            });
            if ship {
                included_ship_index += 1;
            }
            if saved
                .authored_slot_id
                .as_deref()
                .is_some_and(|slot| expected.is_none_or(|expected| expected.id != slot))
            {
                return Err(LoadRefusal::Unparsable(format!(
                    "GameStart row {index} has an incompatible authored launch slot"
                )));
            }
        }
    }
    for (authored_index, authored) in world_config.entities.iter().enumerate() {
        if authored.spawn_on != crate::world::config::WorldEntitySpawnOn::GameStart
            || authored.when_predicate.is_some()
        {
            continue;
        }
        let authored_index = u32::try_from(authored_index).map_err(|_| {
            LoadRefusal::Unparsable(
                "the loaded world contains more authored entity rows than a boot identity can address"
                    .to_string(),
            )
        })?;
        if identity
            .game_start_entity_uuids
            .binary_search_by_key(&authored_index, |row| row.authored_index)
            .is_err()
        {
            return Err(LoadRefusal::Unparsable(format!(
                "the current snapshot boot identity omits unconditional GameStart entity row {authored_index}"
            )));
        }
    }
    Ok(())
}

// ── Restore ──────────────────────────────────────────────────────────────────

/// Something the capture named that the bootstrapped world did not have.
///
/// Reported rather than skipped. A restore that quietly drops a ship produces a
/// world that looks right and diverges for a reason nothing in the save points
/// at, which is strictly worse than a restore that says what it could not do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestoreGap {
    /// Exact saved mission age requires both the world and fixed-time owners.
    MissionClockUnavailable,
    /// A captured `EntityUuid` no entity in the target world carries.
    MissingEntity(String),
    /// Canonical assignments cannot resume against the authored crew/workflow.
    RepairJobsInvalid {
        uuid: String,
        reason: String,
    },
    ResourceStateInvalid {
        uuid: String,
        reason: String,
    },
    /// The payload carries unread Coordination enqueues but the target app did
    /// not register the message buffers/cursor that own that staging seam.
    CoordinationStagingUnavailable {
        pending: usize,
    },
    /// A captured `AsteroidUuid` no asteroid in the target world carries.
    MissingAsteroid(String),
    /// A supporting world present at capture has not finished loading in the
    /// fresh bootstrap. Its ASTs/handlers are derived and cannot be restored by
    /// payload, so ownership state must never be written onto a missing layer.
    MissingWorldLayer(String),
    /// The fresh world's ordered active layer composition differs from the
    /// capture, or a layer change is still in flight. Script trigger state is
    /// restored by index, so even equal path sets in another activation order
    /// are unsafe to write into.
    WorldLayerTopologyMoved {
        saved: Vec<String>,
        found: Vec<String>,
        pending: usize,
        inactive: Vec<String>,
    },
    /// The captured `SimRngState` has a different number of streams than this
    /// build declares — a save from before a stream was added. Refused rather
    /// than mapped by position, which would hand one call site another's
    /// sequence.
    RngStreamsMoved,
    /// The bootstrapped world's trigger table is a different length than the
    /// capture's, so the capture's per-trigger rows cannot be trusted to name
    /// the same triggers.
    ///
    /// Reported rather than written in by position, and the distinction is the
    /// whole reason an index is usable as a key here at all: the table is
    /// rebuilt by a deterministic replay of the same load — the declarative
    /// states in `the order `init_world_runtime` builds them in
    /// appended by `merge_script_triggers` in compile order — so two runs of the
    /// same content produce the same table. A table of a *different* length is
    /// therefore not a table whose rows shifted; it is a world that loaded a
    /// different layer set, and writing fired-state into it by position would
    /// arm and disarm triggers at random.
    ScenarioTriggersMoved {
        saved: usize,
        found: usize,
    },
    /// A stored index is out of range or duplicated; no latches were written.
    ScenarioTriggerIndexInvalid {
        index: u32,
    },
    /// The capture was waiting on scripted `after(n, |ctx| …)` callbacks and the
    /// bootstrapped world has no `WorldScriptRuntime` to queue them on — a save
    /// from a scripted world being restored into one whose scripts did not
    /// compile. The content dimension is what should have refused this; the gap
    /// is here so it is never silent if it does not.
    ScriptRuntimeAbsent {
        pending_callbacks: usize,
    },
    /// The capture was holding scripted comms state — queued `open_comms`
    /// requests, or live scripted dialogues — and the bootstrapped world has no
    /// `WorldScriptRuntime` to run them against.
    ///
    /// [`Self::ScriptRuntimeAbsent`]'s comms twin and reported separately
    /// because the loss is different: a dropped callback is deferred work that
    /// never happens, while this is a conversation the player is *in* that
    /// cannot be answered. The content dimension is what should have refused the
    /// save; this is here so it is never silent if it does not.
    CommsScriptRuntimeAbsent {
        pending_opens: usize,
        scripted_dialogues: usize,
    },
}

impl std::fmt::Display for RestoreGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreGap::MissionClockUnavailable => f.write_str("saved mission age requires WorldContentRuntime and Time<Fixed> before restore"),
            RestoreGap::ResourceStateInvalid { uuid, reason } => write!(f, "resource state for {uuid} cannot be restored: {reason}"),
            RestoreGap::RepairJobsInvalid { uuid, reason } => write!(f, "repair jobs for {uuid} cannot be restored: {reason}"),
            RestoreGap::MissingEntity(uuid) => {
                write!(f, "the world has no entity `{uuid}` to restore into")
            }
            RestoreGap::CoordinationStagingUnavailable { pending } => write!(
                f,
                "this save has {pending} unread coordination enqueue(s), but the target world \
                 did not register their staging cursor"
            ),
            RestoreGap::MissingAsteroid(uuid) => {
                write!(f, "the world has no asteroid `{uuid}` to restore into")
            }
            RestoreGap::MissingWorldLayer(path) => {
                write!(f, "the world layer `{path}` has not loaded for restore")
            }
            RestoreGap::WorldLayerTopologyMoved {
                saved,
                found,
                pending,
                inactive,
            } => write!(
                f,
                "this save's ordered world-layer topology is {saved:?}, but the fresh \
                 world has {found:?} with {pending} change(s) still pending and \
                 inactive load sentinel(s) {inactive:?}"
            ),
            RestoreGap::RngStreamsMoved => f.write_str(
                "this save's generator streams do not match this build's; \
                 mapping them by position would misroute a call site's sequence",
            ),
            RestoreGap::ScenarioTriggerIndexInvalid { index } => write!(
                f, "this save repeats or cannot resolve scenario trigger index {index}; no trigger continuation was restored"
            ),
            RestoreGap::ScenarioTriggersMoved { saved, found } => write!(
                f,
                "this save records {saved} scenario trigger(s) and the world has \
                 {found}; writing fired state in by position would arm and \
                 disarm the wrong triggers"
            ),
            RestoreGap::ScriptRuntimeAbsent { pending_callbacks } => write!(
                f,
                "this save is waiting on {pending_callbacks} scripted callback(s) \
                 and the world compiled no scripts to run them"
            ),
            RestoreGap::CommsScriptRuntimeAbsent {
                pending_opens,
                scripted_dialogues,
            } => write!(
                f,
                "this save holds {pending_opens} queued comms open(s) and \
                 {scripted_dialogues} scripted dialogue(s) and the world \
                 compiled no scripts to answer them"
            ),
        }
    }
}

/// Whether a bootstrapped world is far enough along to be restored into.
///
/// A fresh app does not have the scenario's ships at tick 0 — the lobby's
/// collective auto-start has to run first, and the world spawns on the phase
/// transition. So both callers that restore (the browser boot path and the
/// integration test) step the fresh app until this is true and only then
/// overwrite. It is the same question [`restore`] would otherwise answer too
/// late, as a list of [`RestoreGap::MissingEntity`]s.
///
/// # It waits for the whole roster; [`ready_to_rebuild`] is the other half
///
/// This predicate is unchanged by issue #863 and deliberately so: a bootstrapped
/// ship is a *better* ship than one [`restore`] builds, because the bootstrap ran
/// the world's own systems over it — faction registration, AI token and the
/// power seed among them — and the payload does not cover every one of those.
/// AI fidelity is no longer one of the reasons to wait: format 14 explicitly
/// replaces its canonical bundle and dwell boundary. The duel's 120-frame
/// continuation claim is measured
/// against a fully bootstrapped roster and is the standing proof: restoring
/// earlier, onto hulls this module had built instead, parts the two worlds one
/// frame after a matching digest.
///
/// So the order of preference is: wait for the bootstrap, and build only what
/// never arrives. [`ready_to_rebuild`] is the second half of that sentence, and
/// the caller's own deadline is what decides when to ask it.
pub fn ready_to_restore(world: &World, snapshot: &PhoenixSnapshot) -> bool {
    let Some(mut query) = world.try_query::<&EntityUuid>() else {
        return false;
    };
    // Before format 14 this also waited for `ThrustInput`: a High row restored
    // onto a not-yet-promoted bootstrap had nowhere to apply its controls. The
    // explicit fidelity restore now inserts the canonical bundle FIRST, so
    // waiting for one of that bundle's members would deadlock the inverse case
    // (captured High, bootstrap Low) instead of protecting the write.
    let present: Vec<&str> = query.iter(world).map(|uuid| uuid.0.as_str()).collect();
    let roster_ready = snapshot
        .entities
        .iter()
        .all(|row| present.contains(&row.uuid.as_str()));
    roster_ready && belt_ready(world, snapshot) && layers_ready(world, snapshot)
}

/// Whether a world that has stopped becoming [`ready_to_restore`] can be
/// restored into *anyway*, by building what the bootstrap never produced (issue
/// #863).
///
/// # Why this is a second predicate and not a looser first one
///
/// [`ready_to_restore`] answers "is the bootstrap done?" and a caller loops on it
/// until a deadline. This answers the question that deadline used to end the
/// session on: **is what is still missing something this build can make?**
///
/// The two have to stay apart because a mid-run spawn is genuinely ambiguous
/// while the wait is still running. `duel` spawns its whole NPC roster through
/// script effects at t=0, and a fresh app produces every one of those ships
/// within a frame or two — so a predicate that treated "spawned" as "do not
/// wait" would build ten hulls it was about to be handed, which is measurably
/// worse (see [`ready_to_restore`]). `probe_reinforce` spawns two at t=3 s, and a
/// fresh app driven by nobody produces them never. Nothing in the payload
/// distinguishes those two futures, so the only honest discriminator is *time*:
/// wait, and if the ships have still not arrived, build them.
///
/// True when every captured row is either standing — the ordinary case and the
/// same roster rule as above — or carries a
/// [`SpawnOrigin`](crate::world::spawn_origin::SpawnOrigin) that [`restore`] can
/// build it from. Fidelity-owned controls are deliberately not a second gate:
/// restore inserts their canonical bundle from the captured tier before writing
/// them.
pub fn ready_to_rebuild(world: &World, snapshot: &PhoenixSnapshot) -> bool {
    // Missing physical flight carriers have a closed runtime recipe rather
    // than an authored SpawnOrigin. Validate that whole candidate namespace
    // before advertising the same materialisation restore already supports.
    if super::flight_carriers::validate(world, snapshot).is_err() {
        return false;
    }
    let Some(mut query) = world.try_query::<&EntityUuid>() else {
        return false;
    };
    let present: Vec<&str> = query.iter(world).map(|uuid| uuid.0.as_str()).collect();
    let roster_rebuildable = snapshot.entities.iter().all(|row| {
        present.contains(&row.uuid.as_str()) || row.spawn.is_some() || row.weapon_flight_carrier
    });
    roster_rebuildable && belt_ready(world, snapshot) && layers_ready(world, snapshot)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LayerTopologyEntry {
    pub(super) path: String,
    pub(super) loader_path: Option<String>,
}

pub(super) fn saved_layer_topology(snapshot: &PhoenixSnapshot) -> Vec<LayerTopologyEntry> {
    snapshot
        .layer_flags
        .iter()
        .map(|layer| LayerTopologyEntry {
            path: layer.path.clone(),
            loader_path: layer.loader_path.clone(),
        })
        .collect()
}

pub(super) fn live_layer_topology(world: &World) -> Vec<LayerTopologyEntry> {
    let Some(layers) = world.get_resource::<crate::world::server::WorldLayerMap>() else {
        return Vec::new();
    };
    let mut active: Vec<_> = layers
        .0
        .iter()
        .filter(|(_, runtime)| runtime.is_active)
        .collect();
    active.sort_by(|(path_a, runtime_a), (path_b, runtime_b)| {
        runtime_a
            .activation_order
            .cmp(&runtime_b.activation_order)
            .then_with(|| path_a.cmp(path_b))
    });
    active
        .into_iter()
        .map(|(path, runtime)| LayerTopologyEntry {
            path: path.clone(),
            loader_path: runtime.loader_path.clone(),
        })
        .collect()
}

pub(super) fn layer_topology_labels(rows: &[LayerTopologyEntry]) -> Vec<String> {
    rows.iter()
        .map(|row| match &row.loader_path {
            Some(loader) => format!("{} <- {}", row.path, loader),
            None => row.path.clone(),
        })
        .collect()
}

pub(super) fn pending_layer_changes(world: &World) -> usize {
    world
        .get_resource::<crate::world::server::PendingWorldLayerChanges>()
        .map_or(0, |pending| pending.0.len())
}

pub(super) fn inactive_layer_paths(world: &World) -> Vec<String> {
    let mut paths: Vec<String> = world
        .get_resource::<crate::world::server::WorldLayerMap>()
        .into_iter()
        .flat_map(|layers| layers.0.iter())
        .filter(|(_, runtime)| !runtime.is_active)
        .map(|(path, _)| path.clone())
        .collect();
    paths.sort();
    paths
}

pub(super) fn layer_topology_gap(world: &World, snapshot: &PhoenixSnapshot) -> Option<RestoreGap> {
    let saved = saved_layer_topology(snapshot);
    let found = live_layer_topology(world);
    let pending = pending_layer_changes(world);
    let inactive = inactive_layer_paths(world);
    (saved != found || pending != 0 || !inactive.is_empty()).then(|| {
        RestoreGap::WorldLayerTopologyMoved {
            saved: layer_topology_labels(&saved),
            found: layer_topology_labels(&found),
            pending,
            inactive,
        }
    })
}

/// A layer's executable state (ASTs and handler alignment) is rebuilt by the
/// fresh bootstrap, not serialized. The ordered active composition, loader
/// ownership and an empty pending-change queue must all match before scoped
/// callbacks/deadlines or index-keyed trigger state can be restored.
pub(super) fn layers_ready(world: &World, snapshot: &PhoenixSnapshot) -> bool {
    layer_topology_gap(world, snapshot).is_none()
}

/// Whether the target world's asteroid streamer has settled onto the same
/// composition the capture was taken against.
///
/// [`restore_asteroids`] is authoritative over the *rocks* — it spawns the ones
/// the capture names and despawns the ones it does not — but it cannot be
/// authoritative over a field entity that has not loaded yet.
/// `update_asteroid_window` recomputes its composition key from the live
/// `AsteroidFieldSection`s every tick, and a key that disagrees with the
/// window's is its signal that a world layer loaded or unloaded a field, which
/// it answers by clearing the window wholesale. Restoring into a world whose
/// fields were still arriving would therefore be undone by the very next tick:
/// the belt wiped, the digest silently short of every rock the capture counted.
///
/// So a capture whose streamer had settled waits for one whose streamer has
/// too. A capture with no streamed field at all — the duel arena — waits for
/// nothing, because there is nothing to disagree about.
pub(super) fn belt_ready(world: &World, snapshot: &PhoenixSnapshot) -> bool {
    let Some(stored) = snapshot.asteroid_window.as_ref() else {
        return true;
    };
    if stored.needs_init {
        return true;
    }
    let Some(live) = world.get_resource::<AsteroidWindow>() else {
        return false;
    };
    if !live.needs_init {
        return live.composition_key == stored.composition_key;
    }

    // A paused GM-reconnect candidate is deliberately forbidden from taking a
    // private fixed tick before the owner's canonical record commits. That
    // means its fresh AsteroidWindow can still carry `needs_init = true` even
    // though every authored field entity has finished loading. Waiting for
    // `update_asteroid_window` here would deadlock: that system is itself in
    // FixedUpdate, behind the pause this transaction must preserve.
    //
    // The initialized window was only a proxy for the fact we actually need:
    // the live field composition must be the one the stored window was built
    // from, so the first post-Resume fixed tick cannot rebuild the restored
    // belt. Compute that same order-sensitive key directly from the standing
    // field components. `restore_asteroid_window` then installs the canonical
    // initialized window (including this key) without spending a simulation
    // tick. Ordinary save resume still takes its existing initialized-window
    // branch above.
    let Some(mut fields) =
        world.try_query::<(Entity, &crate::entities::spawner::AsteroidFieldSection)>()
    else {
        return false;
    };
    let mut sections: Vec<(Entity, &crate::entities::spawner::AsteroidFieldSection)> =
        fields.iter(world).collect();
    sections.sort_by_key(|(entity, _)| *entity);
    let contributions: Vec<crate::asteroids::spawner::FieldContribution> = sections
        .into_iter()
        .filter_map(|(_, section)| {
            crate::asteroids::spawner::FieldContribution::from_config(&section.0)
        })
        .collect();
    crate::asteroids::lifecycle::composition_key(&contributions) == stored.composition_key
}

/// Validate the canonical inventory and its actual committed activities before writes.
pub(super) fn validate_entity_resources(
    world: &World,
    entity: Entity,
    row: &EntityState,
) -> Result<(), String> {
    crate::dock::server::validate_saved_entity(
        world,
        entity,
        row.dock.as_ref(),
        row.physics.as_ref(),
    )?;
    super::fitting::validate_entity(world, entity, row)?;
    super::guided_source::validate_entity(world, Some(entity), row, u64::MAX)?;
    super::helm_movement::validate_live(world, entity, row, u64::MAX)?;
    super::transit::validate_live(world, entity, row, u64::MAX)?;
    super::illumination::validate_entity(world, entity, row)?;
    super::jamming::validate_entity(world, entity, row)?;
    match (
        world.get::<crate::console::navigation::sharing::NavigationSharingProfile>(entity),
        &row.navigation_sharing,
    ) {
        (None, None) => {}
        (Some(accepted), Some(saved)) => {
            saved.validate_owner(&row.uuid, &accepted.owner, u64::MAX)?
        }
        _ => return Err("saved Navigation sharing disagrees with accepted equipment".into()),
    }
    match (
        world.get::<crate::science::broad::BroadProfile>(entity),
        &row.broad,
    ) {
        (None, None) => {}
        (Some(profile), Some(saved)) => {
            let dt = world
                .get_resource::<Time<Fixed>>()
                .ok_or("broad continuation requires fixed logical time")?
                .timestep()
                .as_secs_f32();
            if profile.0.keys().ne(saved.keys()) {
                return Err("broad observation owners disagree with accepted instruments".into());
            }
            for (system, state) in saved {
                state.validate(&profile.0[system].profile, u64::MAX, dt)?;
            }
        }
        _ => return Err("saved broad observations disagree with accepted equipment".into()),
    }
    match (
        world.get::<crate::science::passive::PassiveProfile>(entity),
        &row.passive,
    ) {
        (None, None) => {}
        (Some(profile), Some(saved)) => saved.validate(profile, &row.uuid, u64::MAX)?,
        _ => return Err("saved passive receiver disagrees with accepted equipment".into()),
    }
    match (
        world.get::<crate::science::ShipScanRecord>(entity),
        &row.scan,
    ) {
        (Some(record), Some(saved)) => {
            record.validate_state(saved, u64::MAX)?;
            let forecasts = saved
                .instruments
                .values()
                .flat_map(|owner| &owner.history)
                .filter_map(|reading| reading.forecast.as_ref());
            for forecast in forecasts {
                let dt = f64::from(
                    world
                        .get_resource::<Time<Fixed>>()
                        .ok_or("forecast continuation requires fixed logical time")?
                        .timestep()
                        .as_secs_f32(),
                );
                let elapsed = (forecast.sample_ticks[1] - forecast.sample_ticks[0]) as f64 * dt;
                if forecast.sample_interval_secs != elapsed {
                    return Err("forecast interval disagrees with fixed logical time".into());
                }
            }
        }
        (None, None) => {}
        _ => return Err("saved observations disagree with the accepted scanner".into()),
    }
    let saved_shields = row
        .weapons
        .as_ref()
        .map(|state| state.shield_operations.as_slice())
        .unwrap_or(&[]);
    match world.get::<crate::ship::shields::ShipShields>(entity) {
        Some(live) => {
            live.0.validate_operations(saved_shields)?;
            live.0.validate_facings(
                row.weapons
                    .as_ref()
                    .map(|state| state.shield_charge.as_slice())
                    .unwrap_or(&[]),
            )?;
        }
        None if !saved_shields.is_empty() => {
            return Err("shield operations have no installed arcs".into())
        }
        None => {}
    }
    match (
        world.get::<crate::ship::power::ShipPowerSystem>(entity),
        &row.power,
    ) {
        (Some(live), Some(saved)) => {
            let fitted;
            let live = if let (Some(profile), Some(state)) = (
                world.get::<crate::resources::fitting_server::CompiledShipFitting>(entity),
                &row.fitting,
            ) {
                fitted = super::fitting::fitted_power(&profile.0, state, row)?;
                &fitted
            } else {
                live
            };
            let ids = row
                .infrastructure
                .as_ref()
                .map(|state| {
                    state
                        .capacities()
                        .iter()
                        .map(|capacity| capacity.id.clone())
                        .collect()
                })
                .unwrap_or_default();
            live.0.validate_continuation(saved, &ids)?;
        }
        (None, None) => {}
        _ => return Err("energy network continuation disagrees with authored machinery".into()),
    }
    if row.infrastructure.is_none()
        && world
            .get::<crate::infrastructure::InfrastructureCondition>(entity)
            .is_some()
    {
        return Err("infrastructure provider is missing its canonical inventory".into());
    }
    if row.weapons.is_none()
        && world
            .get::<crate::console::weapons::torpedo::TorpedoSystemResource>(entity)
            .is_some()
    {
        return Err("torpedo provider is missing its canonical inventory".into());
    }
    if let Some(infrastructure) = &row.infrastructure {
        infrastructure.validate_stocks()?;
        let live = world
            .get::<crate::infrastructure::InfrastructureCondition>(entity)
            .ok_or("inventory has no authored infrastructure provider")?;
        if live.0.capacities().len() != infrastructure.capacities().len()
            || infrastructure
                .capacities()
                .iter()
                .zip(live.0.capacities())
                .any(|(saved, authored)| {
                    saved.id != authored.id
                        || saved.ceiling != authored.ceiling
                        || saved.label != authored.label
                        || saved.stock.reserved.keys().any(|claim| {
                            !claim.starts_with("repair/")
                                && !claim.starts_with("external-repair/")
                                && !claim.starts_with("refit/")
                        })
                })
        {
            return Err(
                "infrastructure inventory disagrees with authored stores or has unsupported claims"
                    .into(),
            );
        }
        if infrastructure
            .capacities()
            .iter()
            .any(|capacity| !capacity.stock.reserved.is_empty())
        {
            let jobs = row
                .repair
                .as_ref()
                .and_then(|repair| repair.jobs.as_ref())
                .ok_or("repair parts claims have no team owner")?;
            super::application::validate_physical_repair_claims(world, entity, row, jobs)?;
        }
    }
    if let Some(flow) = &row.umbilical {
        if !world
            .get::<crate::umbilical::TransferUmbilical>(entity)
            .is_some_and(|live| live.config.permits_resource(&flow.selected_resource))
        {
            return Err("umbilical continuation selects an unauthored resource".into());
        }
        if !flow.carry.is_finite()
            || !(0.0..1.0).contains(&flow.carry)
            || (!flow.running && flow.carry != 0.0)
        {
            return Err("invalid fractional resource-flow continuation".into());
        }
    }
    if let Some(stored) = &row.weapons {
        validate_physical_attacks(world, row)?;
        if let Some(live) =
            world.get::<crate::console::weapons::torpedo::TorpedoSystemResource>(entity)
        {
            if stored.magazine.is_none() {
                return Err("torpedo-equipped ship is missing its canonical magazine".into());
            }
            let mut candidate = live.0.clone();
            let mut ids = std::collections::BTreeSet::new();
            if stored.tubes.len() != candidate.tubes.len()
                || stored.tubes.iter().any(|tube| {
                    !ids.insert(&tube.id)
                        || !candidate.tubes.iter().any(|live| {
                            live.id == tube.id
                                && tube.loaded_count <= live.volley_max
                                && tube.target_count <= live.volley_max
                        })
                        || tube.load_phase > 3
                        || tube.load_timer.iter().any(|t| !t.is_finite() || *t < 0.0)
                        || tube.load_timer[0] > tube.load_timer[1]
                })
            {
                return Err(
                    "torpedo inventory has missing, duplicate or invalid tube activities".into(),
                );
            }
            super::application::apply_torpedo_inventory(&mut candidate, stored);
            candidate.validate_magazine()?;
        } else if stored.magazine.is_some() {
            return Err("magazine has no authored torpedo provider".into());
        }
    }
    Ok(())
}

/// Validate launched effects against the complete immutable target roster.
pub(super) fn validate_physical_attacks(world: &World, row: &EntityState) -> Result<(), String> {
    let Some(stored) = &row.weapons else {
        return Ok(());
    };
    super::guided_rounds::validate_weapon_state(stored)?;
    for profile in stored
        .torpedoes_in_flight
        .iter()
        .map(|round| &round.profile.impact)
        .chain(stored.bursts.iter().map(|burst| &burst.profile.impact))
        .chain(
            stored
                .blasters
                .iter()
                .flat_map(|bank| bank.in_flight.iter().map(|round| &round.impact)),
        )
    {
        profile.validate()?;
    }
    if stored
        .torpedoes_in_flight
        .iter()
        .any(|round| !round.strike_damage_bonus.is_finite() || round.strike_damage_bonus < -1.0)
    {
        return Err("invalid launched torpedo damage modifier".into());
    }
    for bank in &stored.blasters {
        for bolt in &bank.in_flight {
            if ![
                bolt.x,
                bolt.y,
                bolt.z,
                bolt.heading,
                bolt.pitch,
                bolt.speed,
                bolt.lifespan_remaining,
                bolt.collision_radius,
                bolt.visual_scale,
                bolt.shield_pierce,
            ]
            .into_iter()
            .all(f32::is_finite)
                || bolt.pitch.abs() > std::f32::consts::FRAC_PI_2
                || bolt.speed <= 0.0
                || bolt.lifespan_remaining <= 0.0
                || bolt.collision_radius < 0.0
                || bolt.damage < 0
                || !(0.0..=1.0).contains(&bolt.shield_pierce)
            {
                return Err("invalid emitted blaster flight frontier".into());
            }
        }
    }
    let mut beam_banks = std::collections::BTreeSet::new();
    for (bank, target, remaining, accumulator, cooldown, operation) in &stored.beams {
        if bank.is_empty()
            || target.is_empty()
            || !beam_banks.insert(bank)
            || !remaining.is_finite()
            || *remaining < 0.0
            || (*operation == phoenix_model::weapon_operation::BeamOperation::Fixed
                && *remaining == 0.0)
            || !accumulator.is_finite()
            || !(0.0..1.0).contains(accumulator)
            || !cooldown.is_finite()
            || *cooldown < 0.0
        {
            return Err("invalid active beam operation/duration/accumulator frontier".into());
        }
    }
    for (bank, (target, sample)) in &stored.beam_impact_samples {
        if !stored.beams.iter().any(|(id, ..)| id == bank)
            || target.is_empty()
            || sample.deck.is_empty()
            || !sample.profile_fraction.is_finite()
            || !(0.0..1.0).contains(&sample.profile_fraction)
        {
            return Err("invalid retained beam impact sample".into());
        }
        if let Some(mut targets) = world.try_query::<(
            &crate::entities::spawner::EntityUuid,
            &crate::ship::compartment_damage::ShipDamage,
        )>() {
            if let Some((_, damage)) = targets.iter(world).find(|(uuid, _)| &uuid.0 == target) {
                if !damage
                    .definition
                    .interior
                    .decks
                    .iter()
                    .any(|deck| deck.id == sample.deck)
                {
                    return Err("retained beam sample refers to an absent target deck".into());
                }
            }
        }
    }
    Ok(())
}

/// Cross-actor references and reservations must be checked on the complete
/// candidate roster before any live entity, layer or run-state writes.
pub(super) fn validate_cross_entity_repairs(
    world: &World,
    snapshot: &PhoenixSnapshot,
) -> Result<(), (String, String)> {
    use crate::resources::service::{resolve_endpoint, ResourceEndpoint, ResourceLink};
    use phoenix_model::damage::RepairWorkTarget;
    use std::collections::{BTreeMap, BTreeSet};
    let roster: BTreeSet<_> = snapshot
        .entities
        .iter()
        .map(|row| row.uuid.clone())
        .collect();
    let entities: BTreeMap<_, _> = world
        .try_query::<(Entity, &EntityUuid)>()
        .map(|mut query| {
            query
                .iter(world)
                .map(|(entity, uuid)| (uuid.0.clone(), entity))
                .collect()
        })
        .unwrap_or_default();
    let definitions: BTreeMap<_, _> = entities
        .iter()
        .filter(|(uuid, _)| roster.contains(*uuid))
        .filter_map(|(uuid, entity)| {
            world
                .get::<crate::ship::compartment_damage::ShipDamage>(*entity)
                .map(|damage| (uuid.clone(), damage.definition.clone()))
        })
        .collect();
    let mut claimed: BTreeMap<(String, RepairWorkTarget), (String, u64)> = BTreeMap::new();
    for row in &snapshot.entities {
        let Some(jobs) = row.repair.as_ref().and_then(|repair| repair.jobs.as_ref()) else {
            continue;
        };
        let validate = || -> Result<(), String> {
            let entity = *entities.get(&row.uuid).ok_or("repair source is missing")?;
            let teams = world
                .get::<ShipRepairTeams>(entity)
                .ok_or("repair source has no authored team owner")?;
            teams
                .0
                .validate_external_physical_targets(jobs, &definitions, &roster)?;
            let mut expected = BTreeMap::new();
            for (id, work) in &jobs.external_physical_work {
                if !work.reserved {
                    continue;
                }
                for (index, part) in work.requirements.iter().enumerate() {
                    let services = world
                        .get::<crate::resources::server::ResourceServices>(entity)
                        .ok_or("external repair parts have no source resource services")?;
                    let ResourceEndpoint::Capacity { id: capacity } = resolve_endpoint(
                        &part.resource,
                        &services.0,
                        &ResourceLink {
                            service: Some(part.service.clone()),
                            provider: None,
                        },
                    )?
                    else {
                        return Err("external parts do not resolve to source physical stock".into());
                    };
                    if expected
                        .insert(
                            (capacity, format!("{}/{index}", work.claim(*id))),
                            part.amount,
                        )
                        .is_some()
                    {
                        return Err("duplicate external repair parts claim".into());
                    }
                }
            }
            let actual: BTreeMap<_, _> = row
                .infrastructure
                .as_ref()
                .into_iter()
                .flat_map(|inventory| inventory.capacities())
                .flat_map(|capacity| {
                    capacity
                        .stock
                        .reserved
                        .iter()
                        .filter(|(claim, _)| claim.starts_with("external-repair/"))
                        .map(|(claim, amount)| ((capacity.id.clone(), claim.clone()), *amount))
                })
                .collect();
            if actual != expected {
                return Err("external repair progress and source inventory claims disagree".into());
            }
            Ok(())
        };
        validate().map_err(|reason| (row.uuid.clone(), reason))?;
        for job in jobs
            .teams
            .iter()
            .flatten()
            .filter(|job| job.outcome.is_none())
        {
            let work = match &job.activity {
                crate::jobs::repair::RepairActivity::Internal { .. } => jobs
                    .physical_work
                    .get(&job.id.0)
                    .and_then(|work| work.target.clone())
                    .map(|target| (row.uuid.clone(), target)),
                crate::jobs::repair::RepairActivity::External { target } => jobs
                    .external_physical_work
                    .get(&job.id.0)
                    .filter(|work| &work.target_uuid == target && roster.contains(target))
                    .and_then(|work| work.target.clone())
                    .map(|work| (target.clone(), work)),
                crate::jobs::repair::RepairActivity::Refit { .. } => None,
            };
            if let Some(key) = work {
                if claimed.insert(key, (row.uuid.clone(), job.id.0)).is_some() {
                    return Err((
                        row.uuid.clone(),
                        "multiple active repair teams claim the same physical task".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Clock shape is required before any entity or global continuation is written.
pub(super) fn mission_clock_gap(world: &World, snapshot: &PhoenixSnapshot) -> Option<RestoreGap> {
    (snapshot
        .scenario
        .as_ref()
        .is_some_and(|scenario| scenario.mission_elapsed.is_some())
        && (!world.contains_resource::<WorldContentRuntime>()
            || !world.contains_resource::<Time<Fixed>>()))
    .then_some(RestoreGap::MissionClockUnavailable)
}
