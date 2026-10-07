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
    for row in &identity.game_start_entity_uuids {
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
    /// A captured `EntityUuid` no entity in the target world carries.
    MissingEntity(String),
    /// Canonical assignments cannot resume against the authored crew/workflow.
    RepairJobsInvalid { uuid: String, reason: String },
    /// The payload carries unread Coordination enqueues but the target app did
    /// not register the message buffers/cursor that own that staging seam.
    CoordinationStagingUnavailable { pending: usize },
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
    ScenarioTriggersMoved { saved: usize, found: usize },
    /// A stored index is out of range or duplicated; no latches were written.
    ScenarioTriggerIndexInvalid { index: u32 },
    /// The capture was waiting on scripted `after(n, |ctx| …)` callbacks and the
    /// bootstrapped world has no `WorldScriptRuntime` to queue them on — a save
    /// from a scripted world being restored into one whose scripts did not
    /// compile. The content dimension is what should have refused this; the gap
    /// is here so it is never silent if it does not.
    ScriptRuntimeAbsent { pending_callbacks: usize },
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
    let Some(mut query) = world.try_query::<&EntityUuid>() else {
        return false;
    };
    let present: Vec<&str> = query.iter(world).map(|uuid| uuid.0.as_str()).collect();
    let roster_rebuildable = snapshot
        .entities
        .iter()
        .all(|row| present.contains(&row.uuid.as_str()) || row.spawn.is_some());
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
