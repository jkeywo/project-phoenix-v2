//! Server-side Comms runtime: the thin Bevy applier over the pure
//! `comms::content` evaluators (issue #816).
//!
//! Owns the comms runtime state (`CommsRuntime`), the shared inbox resource
//! (`CommsInboxRes`), the viewscreen message (`OnScreenMessage`), the
//! channel-2 delivery message (`CommsChannel2Event`), and the Bevy systems
//! that drive template injection, follow-up delivery, range gating, and the
//! `CommsState` broadcast. `CommsWorldPlugin` registers everything in the
//! same sets and relative order the `WorldPlugin` used before the
//! consolidation.
//!
//! The four console Input handlers (`handle_hail`, `handle_respond_to_message`,
//! `handle_clear_comms`, `handle_show_on_screen`) and the channel-2 consumer
//! (`handle_comms_channel2`) remain in `console::comms::server` — they are the
//! Comms console System — and are registered here so the Input → Broadcast
//! chain is owned by one plugin.

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::comms::content::{
    comms_template_states_from_world, evaluate_comms_templates, follow_up_trigger_holds,
    ActiveDialogue, CommsTemplateState, PendingFollowUp,
};
use crate::comms_inbox::CommsInbox;
use crate::console::comms::server::{
    handle_clear_comms, handle_comms_channel2, handle_hail, handle_respond_to_message,
    handle_show_on_screen,
};
use crate::entity_spawner::EntityUuid;
use crate::lobby::{Sessions, Target};
use crate::messages::{CommsContact, CommsMessage, ServerMessage, StationId, ViewMode};
use crate::simulation::SimOutbox;
use crate::world::content::WorldEvent;
use crate::world::server::{ObjectiveManagerRes, WorldContentRuntime, WorldEventBuffer};

// -- Resources ---------------------------------------------------------------

/// Server-side runtime state for the comms half of the active world content.
///
/// Split out of `WorldContentRuntime` (issue #816) so the Comms concept has
/// one home: trigger/flag/entity-group state stays in the world runtime,
/// comms template/dialogue/contact/range state lives here. Populated at
/// `Startup` by `init_comms_runtime` from the unified `WorldConfig` resource.
/// When no world is loaded all vecs/maps are empty and comms systems are
/// no-ops.
#[derive(Resource, Default)]
pub struct CommsRuntime {
    /// Mutable per-template runtime state (fired flag).
    pub comms_template_states: Vec<CommsTemplateState>,
    /// Active in-flight dialogues keyed by CommsMessage id.
    pub active_dialogues: HashMap<String, ActiveDialogue>,
    /// Hailable contacts derived from world comms templates.
    pub contacts: Vec<CommsContact>,
    /// Set to `true` whenever contacts or other world-level data changes so
    /// `broadcast_comms_state` knows to push a fresh snapshot even if the
    /// inbox itself hasn't changed.
    pub needs_broadcast: bool,
    /// Per-entity-UUID snapshot of comms-range flags. Populated by
    /// `update_comms_range_flags` each tick from ship + entity transforms +
    /// `CommsRange` components. UUIDs absent from the map default to true at
    /// stamp time *only when `range_active == false`* (backward compat for
    /// pure-handler tests and lobby phase). When `range_active == true`,
    /// missing UUIDs are treated as `sender_in_range = false`.
    pub range_flags: HashMap<String, bool>,
    /// `true` once `update_comms_range_flags` has located a player `Ship`
    /// and is maintaining `range_flags`. While `false`, range gating is
    /// fully bypassed (preserves lobby + pure-handler tests).
    pub range_active: bool,
    /// Comms follow-ups awaiting their trigger condition before injection.
    /// Response follow-ups carry a `placeholder_id` so the inbox shows a
    /// `...` row while the trigger is pending; chained roots stay silent.
    pub pending_follow_ups: Vec<PendingFollowUp>,
    /// Entity UUIDs this ship has HAILED and not yet cleared (issue #786).
    ///
    /// Authoritative comms state, not AI memory: `handle_hail` records EVERY
    /// hail that passes the server-side range gate — human officer or Backfill
    /// AI alike — and `handle_clear_comms` empties it alongside the inbox it
    /// mirrors. It is the record of a command that actually happened, in the
    /// same category as `range_flags`, and both actors read the same set.
    ///
    /// It exists because "did we hail them?" is NOT derivable from the inbox: a
    /// hail to a target with no matching (or already-`fired`) `on_hailed`
    /// template seats no message and no dialogue at all. Without this record a
    /// standing `Hail` directive re-emits every tick forever. See
    /// [`crate::console::comms::server::has_open_hail_thread_with`].
    ///
    /// Three things retire an entry: a human officer's `ClearComms`
    /// (`handle_clear_comms`), the target ceasing to be a live hail candidate
    /// (`operate_comms_ai`'s per-tick retirement — the unmanned ship's only
    /// re-arm), and the target's entity despawning
    /// ([`update_comms_range_flags`], which keeps the set from growing
    /// monotonically across world-layer cycles).
    pub open_hails: std::collections::BTreeSet<String>,
}

/// Bevy resource wrapping the server-side comms inbox.
///
/// Wrapping `CommsInbox` in a newtype lets us insert it as a Bevy `Resource`
/// without adding Bevy dependency to the pure `comms_inbox` module.
#[derive(Resource, Default)]
pub struct CommsInboxRes(pub CommsInbox);

/// The comms message currently being displayed on the viewscreen.
///
/// Set when a Comms officer sends `ShowOnScreen { message_id }`.
/// Cleared automatically when:
/// - The message is responded to.
/// - The message becomes orphaned or the sender goes out of range.
/// - The captain overrides the view mode away from `ViewMode::Comms`.
#[derive(Resource, Default)]
pub struct OnScreenMessage(pub Option<CommsMessage>);

/// Channel-2 (immediate sim-level) delivery of scenario content into the Comms system.
/// Fired by the world engine instead of mutating `CommsInboxRes` directly; consumed by
/// `handle_comms_channel2` in the Broadcast set.
#[derive(Message, Clone, Debug)]
pub struct CommsChannel2Event {
    pub message: CommsMessage,
}

// -- Plugin ------------------------------------------------------------------

/// Registers the comms resources, messages, and systems in the SAME sets and
/// relative order the pre-#816 `WorldPlugin` used:
///
/// * Input: the four console command handlers (from `console::comms`).
/// * Broadcast chain: `handle_comms_channel2` → `auto_clear_on_screen_message`
///   → `update_comms_range_flags` → `broadcast_comms_state` (the world's
///   `broadcast_objective_summary` orders itself `.after(broadcast_comms_state)`).
/// * Physics chain: `tick_pending_follow_ups` before the world's
///   `collect_world_events`; `inject_comms_templates` between
///   `collect_world_events` and `tick_trigger_pipeline` — preserving the
///   documented `tick_pending_follow_ups → collect_world_events →
///   inject_comms_templates → tick_trigger_pipeline` pipeline (#718/#719).
/// * Startup: `init_comms_runtime` after the world's `init_world_runtime`
///   (it reads the merged `name_to_uuid`).
/// * OnEnter(InProgress): `mark_comms_dirty_on_game_start`.
///
/// Added by `WorldPlugin`, whose systems the ordering constraints reference.
pub struct CommsWorldPlugin;

impl Plugin for CommsWorldPlugin {
    fn build(&self, app: &mut App) {
        use crate::command_admission::{ConsumerMatcher, RegisterAdmittedConsumer};
        // Admitted-command consumer (issue #833): the comms input handlers
        // (`handle_hail` / `handle_respond_to_message` / `handle_clear_comms`)
        // all read the `comms` system's admitted commands.
        app.register_admitted_consumer(ConsumerMatcher::exact(
            crate::system_registry::COMMS_SYSTEM_ID,
        ));
        app.init_resource::<CommsRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<OnScreenMessage>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Startup,
                init_comms_runtime.after(crate::world::server::init_world_runtime),
            )
            .add_systems(
                OnEnter(crate::messages::GamePhase::InProgress),
                mark_comms_dirty_on_game_start,
            )
            .add_systems(
                Update,
                (
                    handle_hail.in_set(crate::sim_sets::SimSet::Input),
                    handle_respond_to_message.in_set(crate::sim_sets::SimSet::Input),
                    // CLEAR WINS on a tie (issue #786). `handle_hail` and
                    // `handle_clear_comms` both take `ResMut<CommsRuntime>` and
                    // both write `open_hails`; if a Hail and a ClearComms land
                    // in the same tick, whether the hail survives the clear must
                    // not depend on executor ordering. The enclosing `.chain()`
                    // already orders these, but the constraint is spelled out
                    // explicitly so it survives the chain being unpicked — this
                    // is the same class of bug #785 fixed for repair. Clear-wins
                    // matches the inbox semantics the two share.
                    handle_clear_comms
                        .in_set(crate::sim_sets::SimSet::Input)
                        .after(handle_hail),
                    // Deterministic same-tick viewscreen ordering (issue #769):
                    // apply comms `ShowOnScreen` AFTER captain `SetView` so the
                    // latest-valid-command-wins `sequence` is a total order when
                    // both land in one tick (comms show is the later, winning
                    // request on a tie).
                    handle_show_on_screen
                        .in_set(crate::sim_sets::SimSet::Input)
                        .after(crate::console::captain::server::handle_set_view),
                    handle_comms_channel2.in_set(crate::sim_sets::SimSet::Broadcast),
                    auto_clear_on_screen_message.in_set(crate::sim_sets::SimSet::Broadcast),
                    update_comms_range_flags.in_set(crate::sim_sets::SimSet::Broadcast),
                    broadcast_comms_state.in_set(crate::sim_sets::SimSet::Broadcast),
                )
                    .chain(),
            )
            // Explicit trigger-pipeline ordering (#718/#719):
            // `tick_pending_follow_ups` must observe `pending_world_events`
            // BEFORE `collect_world_events` drains them into
            // `WorldEventBuffer` (same-tick follow-up reaction);
            // `inject_comms_templates` reads the buffer and must run before
            // `tick_trigger_pipeline`'s dispatch can mutate
            // `runtime.name_to_uuid` (`SpawnEntity`); the pipeline consumes
            // the buffer last.
            .add_systems(
                Update,
                (
                    tick_pending_follow_ups.before(crate::world::server::collect_world_events),
                    inject_comms_templates
                        .after(crate::world::server::collect_world_events)
                        .before(crate::world::server::tick_trigger_pipeline),
                )
                    .in_set(crate::sim_sets::SimSet::Physics),
            );
    }
}

// -- Startup -----------------------------------------------------------------

/// Startup system: initialise `CommsRuntime` and `CommsInboxRes` from the
/// loaded `WorldConfig` (if any).
///
/// Runs after `init_world_runtime` in the Startup schedule so the merged
/// `WorldContentRuntime.name_to_uuid` (spawn pass + config names) is
/// available for contact resolution. When no `WorldConfig` resource is
/// present (native unit tests) this is a no-op and comms systems remain
/// quiet.
pub(crate) fn init_comms_runtime(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    world: Res<WorldContentRuntime>,
    mut comms: ResMut<CommsRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    let Some(world_config) = world_config else {
        return;
    };

    // Derive comms runtime states straight from the parsed world.
    comms.comms_template_states = comms_template_states_from_world(&world_config);

    // Build the contact list from comms templates using the merged
    // `world.name_to_uuid` so unified-pipeline UUIDs are picked up.
    let mut contacts: Vec<CommsContact> = Vec::new();
    for tmpl in &world_config.comms {
        let uuid = match world.name_to_uuid.get(&tmpl.from) {
            Some(u) => u.clone(),
            None => continue,
        };
        if !contacts.iter().any(|c: &CommsContact| c.uuid == uuid) {
            contacts.push(CommsContact {
                uuid,
                // Player-facing contact label uses the sender display text
                // when authored, falling back to the `from` reference id
                // (issue #751).
                name: tmpl
                    .display_name
                    .clone()
                    .unwrap_or_else(|| tmpl.from.clone()),
                in_range: true,
                is_urgent: false,
            });
        }
    }
    comms.contacts = contacts;
    comms.needs_broadcast = true;

    // Mark inbox dirty so the first InProgress broadcast fires even though
    // no messages have arrived yet.
    inbox.0.mark_dirty();
}

/// Merge a parsed world's comms templates and contacts into the live
/// `CommsRuntime` and mark it for re-broadcast.
///
/// Shared by the world-layer lifecycle systems (`apply_pending_scenario_loads`
/// and `apply_world_layer_changes`' Load branch) so world/server.rs no longer
/// defines any comms merge behaviour. Contacts are de-duplicated by UUID;
/// names that don't resolve through `name_to_uuid` are skipped. Returns the
/// freshly derived template states so layer loads can snapshot them for
/// `UnloadWorld` reversal.
pub(crate) fn merge_world_comms(
    comms: &mut CommsRuntime,
    world_config: &crate::world::config::WorldConfig,
    name_to_uuid: &HashMap<String, String>,
) -> Vec<CommsTemplateState> {
    let states = comms_template_states_from_world(world_config);
    comms.comms_template_states.extend(states.iter().cloned());

    // Merge contacts (skip duplicates by uuid).
    for tmpl in &world_config.comms {
        let uuid = match name_to_uuid.get(&tmpl.from) {
            Some(u) => u.clone(),
            None => continue,
        };
        if !comms.contacts.iter().any(|c: &CommsContact| c.uuid == uuid) {
            comms.contacts.push(CommsContact {
                uuid,
                // Display text over reference id (issue #751), matching
                // `init_comms_runtime`.
                name: tmpl
                    .display_name
                    .clone()
                    .unwrap_or_else(|| tmpl.from.clone()),
                in_range: true,
                is_urgent: false,
            });
        }
    }

    comms.needs_broadcast = true;
    states
}

/// Remove the comms template states belonging to an unloaded world layer.
///
/// Counterpart to `merge_world_comms`, called by `apply_world_layer_changes`'
/// Unload branch: each template in the layer snapshot removes at most one
/// matching live state (matched by template equality).
pub(crate) fn remove_layer_comms(comms: &mut CommsRuntime, layer_states: &[CommsTemplateState]) {
    let removed_comms: HashSet<usize> = layer_states
        .iter()
        .filter_map(|ls| {
            comms
                .comms_template_states
                .iter()
                .position(|rs| rs.template == ls.template)
        })
        .collect();
    let mut ci = 0usize;
    comms.comms_template_states.retain(|_| {
        let keep = !removed_comms.contains(&ci);
        ci += 1;
        keep
    });

    comms.needs_broadcast = true;
}

/// Re-mark the comms runtime dirty when the game enters InProgress.
///
/// `init_comms_runtime` marks the runtime dirty during Startup so the first
/// `broadcast_comms_state` fires. However, if no player holds the Comms console
/// during Lobby, that broadcast clears the dirty flag without sending anything.
/// This system ensures the flag is restored when InProgress begins, so the Comms
/// console holder receives the initial contact list on the first InProgress tick.
fn mark_comms_dirty_on_game_start(
    mut comms: ResMut<CommsRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    if comms.contacts.is_empty() && comms.comms_template_states.is_empty() {
        return;
    }
    comms.needs_broadcast = true;
    inbox.0.mark_dirty();
}

// -- Helpers -----------------------------------------------------------------

/// Resolve the current `sender_in_range` flag for an injection-time message,
/// matching the stamp logic in `broadcast_comms_state`. Used by every site
/// that inserts a new `CommsMessage` so the field is correct from the moment
/// the message lands in the inbox (belt-and-braces against future refactors
/// that bypass the broadcast stamp pass).
pub(crate) fn current_sender_in_range(comms: &CommsRuntime, sender_uuid: &str) -> bool {
    // Synthetic senders (not a real UUID4 — e.g. "_self", "Starcorp Command") are
    // always readable: they have no physical entity to range-check against.
    if uuid::Uuid::parse_str(sender_uuid).is_err() {
        return true;
    }
    match comms.range_flags.get(sender_uuid).copied() {
        Some(flag) => flag,
        None => !comms.range_active,
    }
}

// -- Update systems ----------------------------------------------------------

/// Tick pending comms follow-ups: advance queue-relative timers, evaluate
/// trigger conditions against current world state plus this tick's pending
/// events, and inject any follow-ups whose conditions are now met.
///
/// Ordering: registered BEFORE `collect_world_events` (see
/// `CommsWorldPlugin::build`) so this system observes `pending_world_events`
/// BEFORE they are drained into `WorldEventBuffer`. This lets follow-ups
/// react to events on the same tick they fire.
///
/// "Fire immediately if already true" semantics applies to state-based
/// triggers: `OnEnteredRegion` fires if the ship is currently inside the
/// region; `OnFlagSet` fires if the flag is currently set; `OnDestroyed`
/// fires if the named entity is no longer in the ECS; `OnWorldLoaded`
/// always fires. Event-only triggers (`OnAttacked`, `OnHailed`) require
/// the matching event to be observed in `pending_world_events`.
pub(crate) fn tick_pending_follow_ups(
    time: Res<bevy::time::Time>,
    world: Res<WorldContentRuntime>,
    mut comms: ResMut<CommsRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    mut channel2_writer: MessageWriter<CommsChannel2Event>,
    region_membership: Option<Res<crate::regions::server::RegionMembership>>,
    ship_query: Query<Entity, With<crate::simulation::LocalShip>>,
    entity_uuid_q: Query<&EntityUuid>,
) {
    if comms.pending_follow_ups.is_empty() {
        return;
    }

    let dt = time.delta_secs();

    // Snapshot of the events + flags + name lookup every pending follow-up
    // evaluated this tick sees. The world runtime is read-only here, so the
    // borrows stay stable for the whole pass.
    let events_snapshot: &[WorldEvent] = &world.pending_world_events;
    let name_to_uuid_snapshot = &world.name_to_uuid;
    let flags_snapshot = &world.flags;
    let entity_groups = &world.entity_groups;

    // Build the set of region UUIDs the player ship is currently inside.
    let inside_region_uuids: HashSet<String> = if let (Some(membership), Some(ship_entity)) =
        (region_membership.as_ref(), ship_query.iter().next())
    {
        membership
            .inside
            .get(&ship_entity)
            .map(|set| {
                set.iter()
                    .filter_map(|e| membership.region_uuids.get(e).cloned())
                    .collect()
            })
            .unwrap_or_default()
    } else {
        HashSet::new()
    };

    // Build the set of all live entity UUIDs (for OnDestroyed checks).
    let live_uuids: HashSet<String> = entity_uuid_q.iter().map(|u| u.0.clone()).collect();

    let mut ready: Vec<PendingFollowUp> = Vec::new();
    let mut keep: Vec<PendingFollowUp> = Vec::with_capacity(comms.pending_follow_ups.len());

    for mut pfu in comms.pending_follow_ups.drain(..) {
        pfu.elapsed_secs += dt;
        let fires = follow_up_trigger_holds(
            pfu.node.trigger.as_ref(),
            pfu.elapsed_secs,
            events_snapshot,
            name_to_uuid_snapshot,
            flags_snapshot,
            &inside_region_uuids,
            &live_uuids,
            entity_groups,
        );
        if fires {
            ready.push(pfu);
        } else {
            keep.push(pfu);
        }
    }
    comms.pending_follow_ups = keep;

    for pfu in ready {
        if let Some(placeholder_id) = &pfu.placeholder_id {
            inbox.0.remove(placeholder_id);
        }

        // Inject the real message.
        let new_msg_id = uuid::Uuid::new_v4().to_string();
        let available = current_sender_in_range(&comms, &pfu.sender_uuid);
        let responses = crate::comms::content::response_views(&pfu.node.responses, available);
        let new_msg = CommsMessage {
            id: new_msg_id.clone(),
            sender_uuid: pfu.sender_uuid.clone(),
            sender_name: pfu.sender_name.clone(),
            subject: pfu.node.body.chars().take(40).collect(),
            body: pfu.node.body.clone(),
            responses,
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: available,
            thread_id: pfu.thread_id.clone(),
            is_urgent: pfu.urgent,
        };
        channel2_writer.write(CommsChannel2Event { message: new_msg });
        comms.active_dialogues.insert(
            new_msg_id,
            ActiveDialogue {
                current_node: pfu.node.clone(),
                thread_id: pfu.thread_id.clone(),
            },
        );
    }
}

/// Auto-clear `OnScreenMessage` when the displayed message is no longer valid.
///
/// Clears when:
/// - The message has been responded to (`selected_response` is `Some`).
/// - The message is orphaned (sender entity destroyed/despawned).
/// - The sender is out of comms range.
/// - The ship view mode is no longer `ViewMode::Comms` (captain overrode it).
fn auto_clear_on_screen_message(
    mut on_screen: ResMut<OnScreenMessage>,
    inbox: Res<CommsInboxRes>,
    view_mode_q: Query<&crate::ship_state::ShipViewMode, With<crate::simulation::LocalShip>>,
) {
    if on_screen.0.is_none() {
        return;
    }
    let current_view = view_mode_q
        .single()
        .map(|vm| vm.view_mode.clone())
        .unwrap_or(crate::messages::ViewMode::Camera(
            crate::messages::CameraView::default(),
        ));
    // If the captain (or anyone) has switched away from Comms view, clear.
    if !matches!(current_view, ViewMode::Comms) {
        on_screen.0 = None;
        return;
    }
    // Check the live inbox record for the displayed message.
    let should_clear = if let Some(ref displayed) = on_screen.0 {
        match inbox
            .0
            .messages()
            .into_iter()
            .find(|m| m.id == displayed.id)
        {
            None => true, // message purged from inbox
            Some(live) => {
                live.selected_response.is_some()   // responded to
                || live.is_orphaned                // sender gone
                || !live.sender_in_range // out of range
            }
        }
    } else {
        false
    };
    if should_clear {
        on_screen.0 = None;
    }
}

/// Recompute per-entity comms-range flags from ship + entity transforms.
///
/// Runs before `broadcast_comms_state`. Finds the player ship (entity with
/// `Ship` marker + `Transform` + optional `CommsRange`) and computes
/// `crate::comms::in_range(distance, ship_range, entity_range)` for every
/// entity carrying `EntityUuid` + `Transform` + `CommsRange`. Updates the
/// `comms.range_flags` map and stamps `comms.contacts[i].in_range`. Sets
/// `comms.needs_broadcast = true` if any flag flipped vs. the prior snapshot.
pub(crate) fn update_comms_range_flags(
    mut comms: ResMut<CommsRuntime>,
    ship_q: Query<
        (&Transform, Option<&crate::comms::CommsRange>),
        With<crate::simulation::LocalShip>,
    >,
    entity_q: Query<(
        &crate::entities::spawner::EntityUuid,
        &Transform,
        &crate::comms::CommsRange,
    )>,
) {
    let Some((ship_tf, ship_range_opt)) = ship_q.iter().next() else {
        // No ship: either lobby/pure-handler tests (range tracking never
        // activated — preserve default-true semantics) or the ship was
        // destroyed mid-game. In the latter case, do NOT reset
        // `range_active` to false — that would silently re-enable all
        // comms (a back-door past the Hail/Respond gates). Instead, force
        // every tracked flag to false so the gates stay closed.
        if comms.range_active {
            let mut any_changed = false;
            for v in comms.range_flags.values_mut() {
                if *v {
                    *v = false;
                    any_changed = true;
                }
            }
            let before = comms.contacts.len();
            for c in comms.contacts.iter_mut() {
                if c.in_range {
                    c.in_range = false;
                    any_changed = true;
                }
            }
            let _ = before;
            if any_changed {
                comms.needs_broadcast = true;
            }
        }
        return;
    };
    let ship_range = ship_range_opt.map(|r| r.0).unwrap_or(0.0);
    let ship_pos = ship_tf.translation;

    let mut any_changed = !comms.range_active;
    comms.range_active = true;

    // Build the live set of comms-range-bearing UUIDs and refresh flags.
    let mut live: HashSet<String> = HashSet::new();
    for (uuid, tf, range) in entity_q.iter() {
        let dist = ship_pos.distance(tf.translation);
        let in_range = crate::comms::in_range(dist, ship_range, range.0);
        let prior = comms.range_flags.insert(uuid.0.clone(), in_range);
        if prior != Some(in_range) {
            any_changed = true;
        }
        live.insert(uuid.0.clone());
    }

    // Remove stale flags for despawned entities.
    let stale: Vec<String> = comms
        .range_flags
        .keys()
        .filter(|k| !live.contains(*k))
        .cloned()
        .collect();
    if !stale.is_empty() {
        any_changed = true;
        for k in stale {
            comms.range_flags.remove(&k);
        }
    }

    // Prune contacts whose entity has no [comms] block (no CommsRange).
    let before = comms.contacts.len();
    let live_ref = &live;
    comms.contacts.retain(|c| live_ref.contains(&c.uuid));
    if comms.contacts.len() != before {
        any_changed = true;
    }

    // Prune the open-hail record the same way (issue #786). Without this it
    // grows monotonically and retains the UUIDs of despawned entities: a
    // LoadWorld → UnloadWorld → LoadWorld cycle that re-registers the same
    // authored UUID would leave that contact permanently un-hailable, because
    // `candidate_fact(has_open_hail_thread)` would still read 1 for a hail
    // issued in a previous life of the world. Layer unload despawns the layer's
    // entities (`apply_world_layer_changes`), so this covers `remove_layer_comms`
    // too. Deliberately does NOT touch `any_changed`: `open_hails` is not part
    // of the broadcast `CommsState`, so dropping a stale entry is not a reason
    // to re-broadcast.
    comms.open_hails.retain(|uuid| live_ref.contains(uuid));

    // Stamp the surviving contacts in place from the flag map.
    let CommsRuntime {
        range_flags,
        contacts,
        ..
    } = &mut *comms;
    for c in contacts.iter_mut() {
        if let Some(flag) = range_flags.get(&c.uuid).copied() {
            c.in_range = flag;
        }
    }

    if any_changed {
        comms.needs_broadcast = true;
    }
}

/// Broadcast `CommsState` to the Comms console holder when the inbox is dirty
/// or `CommsRuntime::needs_broadcast` is set.
pub(crate) fn broadcast_comms_state(
    sessions: Res<Sessions>,
    ship_query: Query<(), With<crate::simulation::LocalShip>>,
    mut comms: ResMut<CommsRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    objectives: Res<ObjectiveManagerRes>,
    mut outbox: ResMut<SimOutbox>,
) {
    let dirty = inbox.0.is_dirty() || comms.needs_broadcast || objectives.0.is_dirty();
    if !dirty {
        return;
    }

    let Some(()) = ship_query.iter().next() else {
        return;
    };
    let Some(comms_token) = sessions.0.holder_for_station(&StationId("comms".into())) else {
        inbox.0.mark_clean();
        comms.needs_broadcast = false;
        return;
    };

    let mut messages = inbox.0.messages();
    for m in messages.iter_mut() {
        if let Some(flag) = comms.range_flags.get(&m.sender_uuid).copied() {
            m.sender_in_range = flag;
        } else if comms.range_active {
            // Synthetic senders (non-UUID ids like "_self", "Starcorp Command")
            // are always readable — they have no physical entity to range-check.
            if uuid::Uuid::parse_str(&m.sender_uuid).is_ok() {
                m.sender_in_range = false;
            }
            // else: leave sender_in_range = true for synthetic senders
        }
        // Availability of every response tracks the message's sender range
        // (issue #761): a response is submittable exactly when its sender is
        // reachable. Stamped here so the authoritative range pass is the one
        // source of truth for both `sender_in_range` and per-response
        // `available`.
        for r in m.responses.iter_mut() {
            r.available = m.sender_in_range;
        }
    }
    let objectives_snap = objectives.0.sorted_snapshots();
    let mut contacts = comms.contacts.clone();
    // Auto-derive is_urgent: a contact is urgent when it has at least one
    // unread urgent message in the current inbox.
    for contact in contacts.iter_mut() {
        contact.is_urgent = messages
            .iter()
            .any(|m| m.sender_uuid == contact.uuid && m.is_urgent && !m.is_read);
    }

    outbox.0.push((
        Target::Token(comms_token.to_string()),
        ServerMessage::CommsState {
            messages,
            objectives: objectives_snap,
            contacts,
        },
    ));

    inbox.0.mark_clean();
    comms.needs_broadcast = false;
}

/// Auto-fire comms templates that match this tick's external `WorldEvent`s
/// (e.g. `on_attacked` distress calls) and inject the resulting messages onto
/// channel-2. These are broadcast messages — no player hailing involved.
///
/// Reads `WorldEventBuffer` directly: only EXTERNAL events reach comms
/// templates — `tick_trigger_pipeline`'s internally-produced chaining events
/// (`FlagSet`, `FlagCleared`, `Destroyed` from a `DestroyEntity` action)
/// never do.
///
/// Ordering (#719): registered after `collect_world_events` (which fills the
/// buffer) and before `tick_trigger_pipeline`. Running before the pipeline
/// means `world.name_to_uuid` read here is identical to the tick-level
/// clone the pipeline takes — no `SpawnEntity` dispatch has mutated the map
/// yet this tick.
///
/// Change detection (#716/#718 discipline): early-out on an empty buffer
/// WITHOUT mutably dereferencing `comms`. This is behaviour-preserving
/// even on the tick where the buffer is empty but the pipeline still runs
/// for pending delayed actions: `evaluate_comms_templates` over an empty
/// events slice is a guaranteed no-op (`events.iter().any(..)` is `false`
/// for every template, so no `fired` flag flips and nothing is returned).
pub(crate) fn inject_comms_templates(
    world: Res<WorldContentRuntime>,
    mut comms: ResMut<CommsRuntime>,
    buffer: Res<WorldEventBuffer>,
    mut channel2_writer: MessageWriter<CommsChannel2Event>,
) {
    if buffer.0.is_empty() {
        return;
    }

    // Reborrow the `ResMut` as a plain `&mut` so disjoint field borrows can
    // be split (`&mut comms.comms_template_states` while other comms fields
    // are read). Placed after the early return so an event-free tick never
    // marks the resource changed; a tick with events marks it exactly as the
    // pre-split system did.
    let comms = &mut *comms;

    let fired_comms = evaluate_comms_templates(
        &mut comms.comms_template_states,
        &buffer.0,
        &world.name_to_uuid,
    );
    for fc in fired_comms {
        let thread_id = fc
            .thread_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        // `_self` is the reserved synthetic internal-sender name; render it as
        // "Internal Report" in the comms UI so the crew sees a ship-generated
        // intelligence summary rather than a literal "_self" sender label.
        let channel_name = if fc.from == "_self" {
            "Internal Report".to_string()
        } else {
            fc.from.clone()
        };
        // Sender identity (issue #751): the UUID resolves from the `from`
        // reference id (below), while the player-facing display name resolves
        // independently. Precedence: per-node `speaker` override (most
        // specific) → template `display_name` → the `from`-derived channel
        // name. Backward compatible: a template with neither renders `from`
        // exactly as before.
        let base_sender_name = fc.display_name.clone().unwrap_or(channel_name);
        let sender_name = fc.node.speaker.clone().unwrap_or(base_sender_name);
        // Keyed on the RAW `fc.from` (not the mapped display name): a
        // synthetic sender like `_self` deliberately falls through to the
        // name itself, which `current_sender_in_range` treats as
        // always-in-range via its non-UUID escape hatch.
        let sender_uuid = world
            .name_to_uuid
            .get(&fc.from)
            .cloned()
            .unwrap_or_else(|| fc.from.clone());

        // Root templates inject immediately when their template-level
        // `trigger` fires. Per-node triggers are reserved for follow-ups.
        let msg_id = uuid::Uuid::new_v4().to_string();
        let available = current_sender_in_range(comms, &sender_uuid);
        let responses = crate::comms::content::response_views(&fc.node.responses, available);
        let msg = crate::messages::CommsMessage {
            id: msg_id.clone(),
            sender_uuid: sender_uuid.clone(),
            sender_name: sender_name.clone(),
            subject: fc.node.body.chars().take(40).collect(),
            body: fc.node.body.clone(),
            responses,
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: available,
            thread_id: thread_id.clone(),
            is_urgent: fc.urgent,
        };
        channel2_writer.write(CommsChannel2Event { message: msg });
        comms.active_dialogues.insert(
            msg_id,
            ActiveDialogue {
                current_node: fc.node.clone(),
                thread_id: thread_id.clone(),
            },
        );

        // Schedule the chained root follow_up, if any. See the
        // matching block in `handle_hail` for the rationale.
        if let Some(ref fu) = fc.root_follow_up {
            let fu_sender_name = fu.speaker.clone().unwrap_or(sender_name.clone());
            comms.pending_follow_ups.push(PendingFollowUp {
                node: fu.clone(),
                sender_uuid: sender_uuid.clone(),
                sender_name: fu_sender_name,
                thread_id: thread_id.clone(),
                elapsed_secs: 0.0,
                placeholder_id: None,
                urgent: fc.urgent,
            });
        }
    }
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ai_plugin::AiEntityAttacked;
    use crate::comms::content::{CommsDialogueNode, CommsResponse, CommsTemplate};
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::world::content::{TriggerAction, TriggerCondition};
    use crate::world::server::broadcast_objective_summary;
    use crate::world::server::tests::ai_trigger_test_app;

    // -- Test app -------------------------------------------------------------

    #[derive(Resource, Default)]
    pub(crate) struct Outbox(pub(crate) Vec<OutboundMessage>);

    fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
        for m in reader.read() {
            box_.0.push(m.clone());
        }
    }

    pub(crate) fn comms_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(LobbyPlugin)
            .add_plugins(bevy::time::TimePlugin)
            .add_plugins(crate::server_app::AdmissionPlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .init_resource::<SimOutbox>()
            .init_resource::<Outbox>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Update,
                (
                    handle_hail,
                    handle_respond_to_message,
                    handle_clear_comms,
                    tick_pending_follow_ups,
                    handle_comms_channel2,
                    update_comms_range_flags,
                    broadcast_comms_state,
                    broadcast_objective_summary,
                )
                    .chain()
                    .after(crate::server_app::AdmissionSet),
            )
            .add_systems(PostUpdate, collect);
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::messages::AdmittedCommands::default(),
        ));
        app
    }

    pub(crate) fn push_msg(app: &mut App, token: &str, msg: ClientMessage) {
        app.world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.into(),
                msg,
            });
    }

    pub(crate) fn tick(app: &mut App) -> Vec<OutboundMessage> {
        app.update();
        let sim_entries = std::mem::take(&mut app.world_mut().resource_mut::<SimOutbox>().0);
        let mut msgs = app.world().resource::<Outbox>().0.clone();
        for (target, msg) in sim_entries {
            msgs.push(OutboundMessage {
                target,
                msg,
                delivery: crate::messages::DeliveryClass::Reliable,
            });
        }
        app.world_mut().resource_mut::<Outbox>().0.clear();
        msgs
    }

    /// Set up a game in InProgress phase with a comms player and captain.
    pub(crate) fn setup_game_with_comms(app: &mut App, station_uuid: &str) {
        // Register captain
        push_msg(
            app,
            "captain",
            ClientMessage::Identify {
                token: "captain".into(),
                name: "Alice".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "captain",
            ClientMessage::SelectStation {
                station: "Captain".into(),
            },
        );
        tick(app);
        // Register comms
        push_msg(
            app,
            "comms",
            ClientMessage::Identify {
                token: "comms".into(),
                name: "Uhura".into(),
            },
        );
        tick(app);
        push_msg(
            app,
            "comms",
            ClientMessage::SelectStation {
                station: "Comms".into(),
            },
        );
        tick(app);
        // Start game
        push_msg(app, "captain", ClientMessage::SetReady { ready: true });
        push_msg(app, "comms", ClientMessage::SetReady { ready: true });
        tick(app);

        // Manually install a comms template into the runtime so tests are
        // independent of TOML loading.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("starbase_alpha".into(), station_uuid.into());
        let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
        comms.contacts.push(CommsContact {
            uuid: station_uuid.into(),
            name: "Starbase Alpha".into(),
            in_range: true,
            is_urgent: false,
        });
        comms.comms_template_states.push(CommsTemplateState {
            template: CommsTemplate {
                from: "starbase_alpha".into(),
                trigger: TriggerCondition::OnHailed {
                    entity_name: "starbase_alpha".into(),
                },
                node: CommsDialogueNode {
                    body: "USS Phoenix, please identify yourself.".into(),
                    responses: vec![CommsResponse {
                        text: "We are on a survey mission.".into(),
                        important: false,
                        actions: vec![TriggerAction::AddObjective {
                            id: "obj-survey".into(),
                            text: "Complete the survey".into(),
                            mandatory: true,
                            targets: vec![],
                            directive: crate::messages::AiDirective::None,
                            utility: crate::objectives::UtilityConfig::default(),
                            source: crate::messages::ObjectiveSource::default(),
                        }],
                        follow_up: None,
                    }],
                    speaker: None,
                    trigger: None,
                },
                thread_id: None,
                urgent: false,
                root_follow_up: None,
                display_name: None,
            },
            fired: false,
        });
        comms.needs_broadcast = true;
    }

    // -- Root comms templates (moved from world::server::tests, #816) ---------

    #[test]
    fn root_comms_template_with_on_timer_trigger_waits_silently() {
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert(
                "Research Outpost".to_string(),
                "research-outpost-uuid".to_string(),
            );
            // Simulate that the world has been alive for less than the
            // template's `after_secs` — no TimerElapsed event yet.
            runtime
                .pending_world_events
                .push(WorldEvent::TimerElapsed { elapsed_secs: 1.0 });
        }
        {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            comms.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    from: "Research Outpost".to_string(),
                    trigger: TriggerCondition::OnTimer { after_secs: 3.0 },
                    node: CommsDialogueNode {
                        body: "Ardent, this is Dr. Myst.".to_string(),
                        responses: vec![],
                        speaker: Some("Dr. Myst".to_string()),
                        trigger: None,
                    },
                    thread_id: Some("research-scholar".to_string()),
                    urgent: true,
                    root_follow_up: None,
                    display_name: None,
                },
                fired: false,
            }];
        }

        app.update();

        {
            let messages = app.world().resource::<CommsInboxRes>().0.messages();
            assert!(
                messages.is_empty(),
                "on_timer root comms must stay silent until the timer fires"
            );
            let comms = app.world().resource::<CommsRuntime>();
            assert!(
                comms.pending_follow_ups.is_empty(),
                "root templates do not queue onto pending_follow_ups"
            );
        }

        // Push a TimerElapsed event past the threshold; template fires now.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .pending_world_events
                .push(WorldEvent::TimerElapsed { elapsed_secs: 3.5 });
        }
        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender_name, "Dr. Myst");
        assert_eq!(messages[0].body, "Ardent, this is Dr. Myst.");
        assert_eq!(messages[0].thread_id, "research-scholar");
        assert!(messages[0].is_urgent);
    }

    /// `inject_comms_templates` (auto-fire path: `on_world_loaded`,
    /// `on_attacked`, `on_destroyed`, `on_flag_set`) also schedules the
    /// chained `root_follow_up`. Verified by emitting `WorldLoaded` on a
    /// template with a chained node.
    #[test]
    fn root_follow_up_fires_for_auto_triggered_template() {
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime.name_to_uuid.insert(
                "Research Outpost".to_string(),
                "research-outpost-uuid".to_string(),
            );
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }
        {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            comms.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    from: "Research Outpost".to_string(),
                    trigger: TriggerCondition::OnWorldLoaded,
                    node: CommsDialogueNode {
                        body: "Stand by.".to_string(),
                        responses: vec![],
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: Some("research-scholar".to_string()),
                    urgent: false,
                    root_follow_up: Some(CommsDialogueNode {
                        body: "Captain. Dr. Myst speaking.".to_string(),
                        responses: vec![],
                        speaker: Some("Dr. Myst".to_string()),
                        trigger: Some(TriggerCondition::OnTimer { after_secs: 2.0 }),
                    }),
                    display_name: None,
                },
                fired: false,
            }];
        }

        app.update();

        // Root injected; chained follow-up queued.
        {
            let messages = app.world().resource::<CommsInboxRes>().0.messages();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].body, "Stand by.");
            let comms = app.world().resource::<CommsRuntime>();
            assert_eq!(comms.pending_follow_ups.len(), 1);
        }

        // Trip the queue-relative timer and tick.
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .pending_follow_ups[0]
            .elapsed_secs = 5.0;
        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 2);
        let chained = &messages[1];
        assert_eq!(chained.sender_name, "Dr. Myst");
        assert_eq!(chained.body, "Captain. Dr. Myst speaking.");
        assert_eq!(chained.thread_id, "research-scholar");
    }

    // -- on_attacked comms template auto-injection tests ----------------------

    /// When an entity is attacked, comms templates with `on_attacked` condition
    /// must fire automatically (no player hailing required) and inject a message
    /// into the CommsInbox.
    #[test]
    fn on_attacked_comms_template_auto_injects_into_inbox() {
        let mut app = ai_trigger_test_app();

        let raider_uuid = "raider-uuid-auto-001";
        let attacker_uuid = uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000001").unwrap();
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("raider".to_string(), raider_uuid.to_string());
        {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            comms.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    from: "raider".to_string(),
                    trigger: TriggerCondition::OnAttacked {
                        entity_name: "raider".to_string(),
                    },
                    node: CommsDialogueNode {
                        body: "Mayday! We are under attack!".to_string(),
                        responses: vec![],
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: false,
                    root_follow_up: None,
                    display_name: None,
                },
                fired: false,
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: raider_uuid.to_string(),
                attacker_uuid,
            });

        app.update();

        let inbox = &app.world().resource::<CommsInboxRes>().0;
        let messages = inbox.messages();
        assert_eq!(
            messages.len(),
            1,
            "on_attacked comms template must auto-inject one message"
        );
        assert_eq!(messages[0].body, "Mayday! We are under attack!");
        assert_eq!(
            messages[0].responses.len(),
            0,
            "broadcast message should have no responses"
        );
    }

    /// Sender identity (issue #751): the `from` reference id resolves the
    /// message's `sender_uuid` (used for range/contact lookup), while the
    /// authored `display_name` resolves the player-facing `sender_name`
    /// independently. Delivery still targets the ship Comms system
    /// (`CommsInboxRes` via `CommsChannel2Event`).
    #[test]
    fn sender_identity_resolves_from_reference_while_display_is_independent() {
        let mut app = ai_trigger_test_app();

        let raider_uuid = "raider-uuid-identity-751";
        let attacker_uuid = uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000751").unwrap();
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("raider".to_string(), raider_uuid.to_string());
        {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            comms.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    // Reference id used to resolve the sender UUID …
                    from: "raider".to_string(),
                    // … but the crew sees this label instead.
                    display_name: Some("Unknown Contact".to_string()),
                    trigger: TriggerCondition::OnAttacked {
                        entity_name: "raider".to_string(),
                    },
                    node: CommsDialogueNode {
                        body: "Back off!".to_string(),
                        responses: vec![],
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: false,
                    root_follow_up: None,
                },
                fired: false,
            }];
        }

        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: raider_uuid.to_string(),
                attacker_uuid,
            });

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1, "delivery must still reach ship Comms");
        assert_eq!(
            messages[0].sender_uuid, raider_uuid,
            "sender_uuid resolves from the `from` reference id"
        );
        assert_eq!(
            messages[0].sender_name, "Unknown Contact",
            "sender_name resolves from display_name, independent of `from`"
        );
    }

    /// A comms template with `on_attacked` must fire only once (single-shot).
    #[test]
    fn on_attacked_comms_template_fires_only_once() {
        let mut app = ai_trigger_test_app();

        let raider_uuid = "raider-uuid-once-002";
        let attacker_uuid = uuid::Uuid::parse_str("cccccccc-0000-0000-0000-000000000002").unwrap();
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("raider".to_string(), raider_uuid.to_string());
        {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            comms.comms_template_states = vec![CommsTemplateState {
                template: CommsTemplate {
                    from: "raider".to_string(),
                    trigger: TriggerCondition::OnAttacked {
                        entity_name: "raider".to_string(),
                    },
                    node: CommsDialogueNode {
                        body: "Distress signal transmitted.".to_string(),
                        responses: vec![],
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: false,
                    root_follow_up: None,
                    display_name: None,
                },
                fired: false,
            }];
        }

        // First attack
        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: raider_uuid.to_string(),
                attacker_uuid,
            });
        app.update();

        // Second attack
        app.world_mut()
            .resource_mut::<Messages<AiEntityAttacked>>()
            .write(AiEntityAttacked {
                entity_uuid: raider_uuid.to_string(),
                attacker_uuid,
            });
        app.update();

        let inbox = &app.world().resource::<CommsInboxRes>().0;
        assert_eq!(
            inbox.messages().len(),
            1,
            "on_attacked comms template must fire only once"
        );
    }

    // -- Slice 7: range-aware comms broadcast ---------------------------------

    #[test]
    fn comms_state_marks_contact_out_of_range_when_ship_too_far() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-uuid-range-far";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        // Spawn the ship close to the station so the initial hail succeeds,
        // then move the station far away to verify the flag flips.
        let ship_entity = app
            .world_mut()
            .spawn((
                Ship,
                crate::simulation::LocalShip,
                Transform::from_xyz(0.0, 0.0, 0.0),
                CommsRange(100.0),
            ))
            .id();
        let station_entity = app
            .world_mut()
            .spawn((
                EntityUuid(station_uuid.into()),
                Transform::from_xyz(50.0, 0.0, 0.0),
                CommsRange(100.0),
            ))
            .id();

        // Flush initial broadcast.
        let _ = tick(&mut app);

        // Hail in range so a message is injected.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let _ = tick(&mut app);

        // Now move the station far away (combined range 200, distance 1000).
        let _ = ship_entity;
        if let Ok(mut e) = app.world_mut().get_entity_mut(station_entity) {
            e.insert(Transform::from_xyz(1000.0, 0.0, 0.0));
        }
        let out = tick(&mut app);

        let (messages, contacts) = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState {
                    messages, contacts, ..
                } = &m.msg
                {
                    Some((messages.clone(), contacts.clone()))
                } else {
                    None
                }
            })
            .expect("CommsState must be broadcast after range flip");

        let contact = contacts
            .iter()
            .find(|c| c.uuid == station_uuid)
            .expect("contact present");
        assert!(!contact.in_range, "contact should be out of range");
        assert_eq!(messages.len(), 1, "one hail message expected");
        assert!(
            !messages[0].sender_in_range,
            "sender_in_range must be false when station is far"
        );
    }

    /// Issue #786: `open_hails` must be pruned alongside `contacts` when the
    /// target entity stops existing, or it grows monotonically and retains the
    /// UUIDs of despawned entities. A LoadWorld → UnloadWorld → LoadWorld cycle
    /// that re-registers the same authored UUID would otherwise leave that
    /// contact permanently un-hailable: `candidate_fact(has_open_hail_thread)`
    /// would still read 1 for a hail issued in a previous life of the world.
    #[test]
    fn despawning_a_hailed_entity_prunes_the_open_hail_record() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-uuid-open-hail-prune";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(100.0),
        ));
        let station_entity = app
            .world_mut()
            .spawn((
                EntityUuid(station_uuid.into()),
                Transform::from_xyz(50.0, 0.0, 0.0),
                CommsRange(100.0),
            ))
            .id();
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let _ = tick(&mut app);
        assert!(
            app.world()
                .resource::<CommsRuntime>()
                .open_hails
                .contains(station_uuid),
            "the hail must be recorded while the target is live"
        );

        // The layer unloads (or the station is destroyed): its entity despawns.
        app.world_mut().despawn(station_entity);
        let _ = tick(&mut app);
        assert!(
            !app.world()
                .resource::<CommsRuntime>()
                .open_hails
                .contains(station_uuid),
            "a despawned entity's open-hail record must be pruned alongside its \
             contact and range flag"
        );
    }

    /// Issue #786 determinism: a `Hail` and a `ClearComms` landing in the SAME
    /// tick must resolve the same way every run — clear wins, matching the inbox
    /// semantics the two handlers share. `handle_clear_comms` is explicitly
    /// `.after(handle_hail)`, so the outcome cannot depend on executor ordering.
    #[test]
    fn a_same_tick_clear_wins_over_a_hail() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-uuid-same-tick-clear";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(100.0),
        ));
        app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(50.0, 0.0, 0.0),
            CommsRange(100.0),
        ));
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::ClearComms,
            },
        );
        let _ = tick(&mut app);

        assert!(
            app.world().resource::<CommsRuntime>().open_hails.is_empty(),
            "clear must win over a same-tick hail: `handle_clear_comms` runs \
             after `handle_hail`, deterministically"
        );
    }

    #[test]
    fn comms_state_marks_contact_in_range_when_ship_close() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-uuid-range-near";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            CommsRange(500.0),
        ));

        let _ = tick(&mut app);
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let out = tick(&mut app);

        let (messages, contacts) = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState {
                    messages, contacts, ..
                } = &m.msg
                {
                    Some((messages.clone(), contacts.clone()))
                } else {
                    None
                }
            })
            .expect("CommsState must be broadcast");

        let contact = contacts
            .iter()
            .find(|c| c.uuid == station_uuid)
            .expect("contact present");
        assert!(contact.in_range, "contact should be in range");
        assert!(
            messages[0].sender_in_range,
            "sender_in_range true when station within range"
        );
    }

    /// Issue #761 projection: the authored `important` flag and the
    /// authoritative `available` (in-range) flag ride onto each wire response.
    /// `important` is preserved regardless of range; `available` tracks the
    /// sender's reachability and flips false once the station leaves range.
    #[test]
    fn comms_state_projects_important_and_available_onto_responses() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456761";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        // Mark the sole authored response important so we can assert it rides
        // the wire independent of range.
        {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            comms.comms_template_states[0].template.node.responses[0].important = true;
        }

        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(100.0),
        ));
        let station_entity = app
            .world_mut()
            .spawn((
                EntityUuid(station_uuid.into()),
                Transform::from_xyz(50.0, 0.0, 0.0),
                CommsRange(100.0),
            ))
            .id();

        let _ = tick(&mut app);
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let out = tick(&mut app);
        let messages = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::CommsState { messages, .. } => Some(messages.clone()),
                _ => None,
            })
            .expect("CommsState must be broadcast");
        assert_eq!(messages[0].responses.len(), 1);
        assert!(
            messages[0].responses[0].important,
            "authored important flag must ride the wire"
        );
        assert!(
            messages[0].responses[0].available,
            "response available while sender in range"
        );

        // Move the station far away: the response becomes unavailable while its
        // important flag is unchanged.
        if let Ok(mut e) = app.world_mut().get_entity_mut(station_entity) {
            e.insert(Transform::from_xyz(5000.0, 0.0, 0.0));
        }
        let out = tick(&mut app);
        let messages = out
            .iter()
            .find_map(|m| match &m.msg {
                ServerMessage::CommsState { messages, .. } => Some(messages.clone()),
                _ => None,
            })
            .expect("CommsState re-broadcast after range flip");
        assert!(
            !messages[0].responses[0].available,
            "response unavailable once sender leaves range"
        );
        assert!(
            messages[0].responses[0].important,
            "important flag is range-independent"
        );
    }

    // -- Review fixes: pruning, server enforcement, despawn handling ----------

    /// Contacts whose UUID has no matching `CommsRange`-bearing entity in
    /// the world (e.g. the world TOML names a `[[comms]]` template but the
    /// referenced entity doesn't declare a `[comms]` block) MUST be pruned
    /// before broadcast so they never appear as permanently in-range.
    #[test]
    fn contact_without_comms_range_entity_is_pruned_from_broadcast() {
        use crate::comms::CommsRange;
        use crate::simulation::Ship;

        let bogus_uuid = "no-such-entity";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, bogus_uuid);

        // Spawn the ship so range tracking activates, but DO NOT spawn an
        // entity with `bogus_uuid` + CommsRange.
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));

        let out = tick(&mut app);
        let contacts = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                    Some(contacts.clone())
                } else {
                    None
                }
            })
            .expect("CommsState must be broadcast");

        assert!(
            !contacts.iter().any(|c| c.uuid == bogus_uuid),
            "contact for entity without [comms] block must be pruned, got {contacts:?}"
        );
    }

    /// When a comms-bearing entity is despawned, its `range_flags` entry
    /// must be removed and any inbox message from that sender must be
    /// stamped `sender_in_range = false` on the next broadcast.
    #[test]
    fn entity_despawn_flips_sender_in_range_to_false() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        // Use a real UUID4 so the non-UUID synthetic-sender exception introduced
        // for `_self` / "Starcorp Command" does not suppress the range flip.
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456789";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(1000.0),
        ));
        let station_entity = app
            .world_mut()
            .spawn((
                EntityUuid(station_uuid.into()),
                Transform::from_xyz(50.0, 0.0, 0.0),
                CommsRange(1000.0),
            ))
            .id();
        let _ = tick(&mut app);

        // Hail to populate the inbox while in range.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let _ = tick(&mut app);

        // Now despawn the station entity.
        app.world_mut().despawn(station_entity);
        let out = tick(&mut app);

        let messages = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { messages, .. } = &m.msg {
                    Some(messages.clone())
                } else {
                    None
                }
            })
            .expect("a broadcast must fire after despawn (range flip)");

        assert!(
            messages.iter().all(|m| !m.sender_in_range),
            "after despawn, all messages from that sender must have sender_in_range=false: {messages:?}"
        );
    }

    /// Two entities at different distances each get their own flag; flipping
    /// only the closer one's range must not affect the farther one's flag.
    #[test]
    fn multiple_entities_have_independent_range_flags() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let near_uuid = "near-1";
        let far_uuid = "far-1";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, near_uuid);
        // Manually add a second contact.
        {
            let comms = &mut app.world_mut().resource_mut::<CommsRuntime>();
            comms.contacts.push(CommsContact {
                uuid: far_uuid.into(),
                name: "Far".into(),
                in_range: true,
                is_urgent: false,
            });
        }

        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid(near_uuid.into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid(far_uuid.into()),
            Transform::from_xyz(5000.0, 0.0, 0.0),
            CommsRange(500.0),
        ));

        let out = tick(&mut app);
        let contacts = out
            .iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                    Some(contacts.clone())
                } else {
                    None
                }
            })
            .expect("CommsState must be broadcast");

        let near = contacts
            .iter()
            .find(|c| c.uuid == near_uuid)
            .expect("near contact");
        let far = contacts
            .iter()
            .find(|c| c.uuid == far_uuid)
            .expect("far contact");
        assert!(near.in_range, "near contact must be in range");
        assert!(!far.in_range, "far contact must be out of range");
    }

    /// When a contact flips in_range, a CommsState broadcast must fire even
    /// if the inbox itself is clean.
    #[test]
    fn range_flip_triggers_fresh_broadcast() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-flip";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        let ship_entity = app
            .world_mut()
            .spawn((
                Ship,
                crate::simulation::LocalShip,
                Transform::from_xyz(0.0, 0.0, 0.0),
                CommsRange(500.0),
            ))
            .id();
        app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            CommsRange(500.0),
        ));

        // Drain initial broadcasts.
        let _ = tick(&mut app);
        let _ = tick(&mut app);

        // Move ship far away — this must trigger a fresh broadcast even
        // though the inbox didn't change.
        if let Ok(mut e) = app.world_mut().get_entity_mut(ship_entity) {
            e.insert(Transform::from_xyz(5000.0, 0.0, 0.0));
        }
        let out = tick(&mut app);

        let has_broadcast = out
            .iter()
            .any(|m| matches!(&m.msg, ServerMessage::CommsState { .. }));
        assert!(
            has_broadcast,
            "range flip from in→out must trigger a fresh CommsState broadcast"
        );
    }

    /// If the player ship is despawned mid-game (hypothetical hull-zero edge
    /// case), the server must NOT silently re-enable comms by flipping
    /// `range_active` back to false. All tracked flags must be forced to
    /// false so the Hail / Respond gates stay closed.
    #[test]
    fn ship_despawn_mid_game_keeps_gates_closed() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-ship-despawn";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        let ship_entity = app
            .world_mut()
            .spawn((
                Ship,
                crate::simulation::LocalShip,
                Transform::from_xyz(0.0, 0.0, 0.0),
                CommsRange(1000.0),
            ))
            .id();
        app.world_mut().spawn((
            EntityUuid(station_uuid.into()),
            Transform::from_xyz(100.0, 0.0, 0.0),
            CommsRange(1000.0),
        ));
        let _ = tick(&mut app);

        // Sanity: contact is in range.
        {
            let comms = app.world().resource::<CommsRuntime>();
            assert!(comms.range_active, "range_active must be true with ship");
            assert_eq!(comms.range_flags.get(station_uuid).copied(), Some(true));
        }

        // Despawn the ship.
        app.world_mut().despawn(ship_entity);
        let _ = tick(&mut app);

        let comms = app.world().resource::<CommsRuntime>();
        assert!(
            comms.range_active,
            "range_active must REMAIN true after ship despawn (no back-door)"
        );
        assert_eq!(
            comms.range_flags.get(station_uuid).copied(),
            Some(false),
            "tracked flag must be forced false on ship despawn"
        );
        assert!(
            comms.contacts.iter().all(|c| !c.in_range),
            "all contacts must be out of range after ship despawn"
        );
    }

    // -- Same-tick follow-up ordering (#718) ----------------------------------

    /// `tick_pending_follow_ups` must snapshot `pending_world_events` BEFORE
    /// `collect_world_events` drains them into `WorldEventBuffer`. A
    /// registration that ran collection first would leave the snapshot empty
    /// and the event-only `OnAttacked` follow-up trigger used here would
    /// never observe its event (it is consumed this tick, not requeued).
    #[test]
    fn pending_follow_up_reacts_to_event_queued_same_tick() {
        let mut app = ai_trigger_test_app();

        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("freighter".to_string(), "freighter-uuid".to_string());
            // Queued before this tick's update, e.g. by `tick_delayed_actions`
            // on the previous tick or by a region observer.
            runtime.pending_world_events.push(WorldEvent::Attacked {
                uuid: "freighter-uuid".to_string(),
                attacker_uuid: "raider-uuid".to_string(),
            });
        }
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .pending_follow_ups
            .push(PendingFollowUp {
                node: CommsDialogueNode {
                    body: "Mayday! We are under attack!".to_string(),
                    responses: vec![],
                    speaker: Some("Freighter".to_string()),
                    trigger: Some(TriggerCondition::OnAttacked {
                        entity_name: "freighter".to_string(),
                    }),
                },
                sender_uuid: "freighter-uuid".to_string(),
                sender_name: "Freighter".to_string(),
                thread_id: "convoy-thread".to_string(),
                elapsed_secs: 0.0,
                placeholder_id: None,
                urgent: true,
            });

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(
            messages.len(),
            1,
            "an OnAttacked follow-up must fire on the SAME tick its event sits \
             in pending_world_events — tick_pending_follow_ups must run before \
             collect_world_events drains the queue"
        );
        assert_eq!(messages[0].body, "Mayday! We are under attack!");
    }

    // -- tick_pending_follow_ups: integration of triggered follow-ups ---------

    /// Build a minimal app for testing `tick_pending_follow_ups` directly.
    /// Mirrors the existing `delayed_follow_up_replacement_preserves_display_speaker`
    /// shape but exercises the new trigger evaluator.
    fn pending_follow_up_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<CommsInboxRes>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Update,
                (tick_pending_follow_ups, handle_comms_channel2).chain(),
            );
        app
    }

    /// Queue a triggered follow-up with a `...` placeholder onto the runtime.
    fn queue_triggered_follow_up(
        app: &mut App,
        body: &str,
        sender_uuid: &str,
        thread_id: &str,
        placeholder_id: &str,
        trigger: TriggerCondition,
    ) {
        let placeholder = CommsMessage {
            id: placeholder_id.into(),
            sender_uuid: sender_uuid.into(),
            sender_name: "Axiom Station".into(),
            subject: "...".into(),
            body: "...".into(),
            responses: vec![],
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: true,
            thread_id: thread_id.into(),
            is_urgent: false,
        };
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(placeholder);
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .pending_follow_ups
            .push(PendingFollowUp {
                node: CommsDialogueNode {
                    body: body.into(),
                    responses: vec![],
                    speaker: None,
                    trigger: Some(trigger),
                },
                sender_uuid: sender_uuid.into(),
                sender_name: "Axiom Station".into(),
                thread_id: thread_id.into(),
                elapsed_secs: 0.0,
                placeholder_id: Some(placeholder_id.into()),
                urgent: false,
            });
    }

    #[test]
    fn pending_follow_up_with_on_flag_set_trigger_stays_queued_until_flag_is_set() {
        let mut app = pending_follow_up_test_app();
        queue_triggered_follow_up(
            &mut app,
            "Aphelion armed — we're committed now.",
            "axiom-uuid",
            "thread-aphelion",
            "placeholder-aphelion",
            TriggerCondition::OnFlagSet {
                name: "aphelion_armed".into(),
            },
        );

        // Tick once with flag unset — placeholder stays, follow-up still queued.
        app.update();
        {
            let messages = app.world().resource::<CommsInboxRes>().0.messages();
            assert_eq!(messages.len(), 1);
            assert_eq!(
                messages[0].body, "...",
                "placeholder must remain while the trigger is unsatisfied"
            );
            let comms = app.world().resource::<CommsRuntime>();
            assert_eq!(comms.pending_follow_ups.len(), 1);
        }

        // Set the flag; next tick must inject the real message.
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .flags
            .set_flag("aphelion_armed");
        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(
            messages.len(),
            1,
            "placeholder must be replaced by the real message"
        );
        assert_eq!(messages[0].body, "Aphelion armed — we're committed now.");
        assert_eq!(messages[0].thread_id, "thread-aphelion");
        let comms = app.world().resource::<CommsRuntime>();
        assert!(comms.pending_follow_ups.is_empty());
    }

    #[test]
    fn pending_follow_up_with_on_flag_set_fires_immediately_if_flag_already_set() {
        // Critical case for the user request: "or immediately if it's
        // already in range". Set the flag BEFORE queueing the follow-up;
        // the very first tick must inject the real message.
        let mut app = pending_follow_up_test_app();
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .flags
            .set_flag("aphelion_armed");

        queue_triggered_follow_up(
            &mut app,
            "Already-armed acknowledgement.",
            "axiom-uuid",
            "thread-aphelion",
            "placeholder-aphelion",
            TriggerCondition::OnFlagSet {
                name: "aphelion_armed".into(),
            },
        );

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body, "Already-armed acknowledgement.");
    }

    #[test]
    fn pending_follow_up_with_on_timer_uses_queue_relative_elapsed_secs() {
        let mut app = pending_follow_up_test_app();
        queue_triggered_follow_up(
            &mut app,
            "Three seconds elapsed.",
            "axiom-uuid",
            "thread-timer",
            "placeholder-timer",
            TriggerCondition::OnTimer { after_secs: 3.0 },
        );

        // Force the queue-relative elapsed_secs past the threshold.
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .pending_follow_ups[0]
            .elapsed_secs = 4.0;
        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].body, "Three seconds elapsed.");
    }

    // -- Issue #506: channel-2 routing tests ----------------------------------

    /// Scenario hail arrives in CommsInboxRes via channel-2 (inject_comms_templates
    /// writes to CommsChannel2Event; handle_comms_channel2 injects into inbox).
    #[test]
    fn scenario_hail_arrives_in_inbox_via_channel2() {
        let mut app = ai_trigger_test_app();

        // Install a comms template that fires on WorldLoaded.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("outpost_alpha".to_string(), "outpost-uuid-ch2".to_string());
            // Queue a WorldLoaded event so inject_comms_templates fires the template.
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .comms_template_states
            .push(CommsTemplateState {
                template: CommsTemplate {
                    from: "outpost_alpha".to_string(),
                    trigger: TriggerCondition::OnWorldLoaded,
                    node: CommsDialogueNode {
                        body: "Channel-2 test message.".to_string(),
                        responses: vec![],
                        speaker: Some("Outpost Alpha".to_string()),
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: false,
                    root_follow_up: None,
                    display_name: None,
                },
                fired: false,
            });

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(
            messages.len(),
            1,
            "scenario hail must arrive in inbox after routing through channel-2"
        );
        assert_eq!(messages[0].body, "Channel-2 test message.");
        assert_eq!(messages[0].sender_name, "Outpost Alpha");
    }

    /// Issue #786: `handle_comms_channel2` is INJECT-ONLY. It used to carry an
    /// AI branch that called `inbox.record_response(&id, 0)` directly whenever
    /// the comms system was AI-operated — a direct authoritative write that
    /// bypassed admission AND `handle_respond_to_message`, so no trigger action
    /// ever fired and no follow-up ever advanced. That stub is retired: the AI's
    /// answer is now decided by `operate_comms_response_ai` and emitted as an
    /// ordinary admitted `RespondToMessage` for the real router (proved by
    /// `console::comms::server::tests::comms_ai_response_fires_trigger_actions_through_the_router`).
    ///
    /// This test pins the retirement: with an AI-operated comms system and a
    /// message carrying a response, channel-2 delivery injects the message and
    /// leaves `selected_response` untouched.
    #[test]
    fn channel2_injection_never_auto_responds_for_ai_comms() {
        let mut app = ai_trigger_test_app();

        // Spawn a Ship entity with comms system set to AI control.
        {
            let mut sources = crate::ship_plugin::ShipSystemControlSources::default();
            sources.0.set(
                crate::system_registry::comms_system_id(),
                crate::control_source::ControlSource::Ai,
            );
            app.world_mut().spawn((
                crate::simulation::Ship,
                crate::simulation::LocalShip,
                sources,
            ));
        }

        // Install a template with a response, fired on WorldLoaded.
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert("sector_hq".to_string(), "sector-hq-uuid".to_string());
            runtime.pending_world_events.push(WorldEvent::WorldLoaded);
        }
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .comms_template_states
            .push(CommsTemplateState {
                template: CommsTemplate {
                    from: "sector_hq".to_string(),
                    trigger: TriggerCondition::OnWorldLoaded,
                    node: CommsDialogueNode {
                        body: "AI auto-respond test.".to_string(),
                        responses: vec![CommsResponse {
                            text: "Acknowledged.".to_string(),
                            important: false,
                            actions: vec![],
                            follow_up: None,
                        }],
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: false,
                    root_follow_up: None,
                    display_name: None,
                },
                fired: false,
            });

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1, "message must be injected into inbox");
        assert_eq!(
            messages[0].selected_response, None,
            "channel-2 delivery must never record a response: the retired stub \
             bypassed admission and the consequence router (issue #786)"
        );
        assert!(
            !messages[0].is_read,
            "channel-2 delivery must not mark an AI-operated ship's message read"
        );
    }
}
