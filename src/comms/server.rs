//! Server-side Comms runtime: the thin Bevy applier over the pure
//! `comms::content` evaluators (issue #816).
//!
//! Owns the comms runtime state (`CommsRuntime`), the shared inbox resource
//! (`CommsInboxRes`), the viewscreen message (`OnScreenMessage`), the
//! channel-2 delivery message (`CommsChannel2Event`), and the Bevy systems
//! that drive range gating, the hail roster and the `CommsState` broadcast. `CommsWorldPlugin` registers everything in the
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

use crate::comms::content::ActiveDialogue;
use crate::console::comms::inbox::CommsInbox;
use crate::console::comms::server::{
    handle_clear_comms, handle_comms_channel2, handle_hail, handle_respond_to_message,
    handle_show_on_screen,
};
use crate::core::messages::{CommsContact, CommsMessage, ServerMessage, StationId, ViewMode};
use crate::lobby::{Sessions, Target};
use crate::server_app::SimOutbox;
use crate::world::server::ObjectiveManagerRes;

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
    /// Active in-flight dialogues keyed by CommsMessage id.
    pub active_dialogues: HashMap<String, ActiveDialogue>,
    /// Hailable contacts, derived from the live entities that opted in with
    /// `[comms] hailable = true` (issue #985's roster slice, `comms::roster`).
    /// Rebuilt every tick by [`update_comms_range_flags`].
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
    /// The session token `broadcast_comms_state` last addressed a `CommsState`
    /// to, so a change of Comms host forces a fresh broadcast even when nothing
    /// in the inbox, contacts or objectives is dirty. Without this a host that
    /// becomes the resolved Comms seat AFTER the last content change — a visiting
    /// seat relocating on a disconnect, or the original holder reconnecting and
    /// reclaiming it — would never receive the current comms snapshot, because
    /// the targeted `CommsState` only goes to the resolved host and only on a
    /// dirty tick. `None` until the first broadcast (or after a tick with no
    /// resolvable host).
    pub last_broadcast_host: Option<String>,
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

/// Delivery contract for one [`CommsChannel2Event`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommsChannel2Delivery {
    /// Generic injection has no dialogue-runtime prerequisite.
    Generic,
    /// A scripted dialogue message is deliverable only while its authoritative
    /// `CommsRuntime::active_dialogues` entry still exists. This closes the
    /// Physics-to-Broadcast race where a layer unload or `ClearComms` retires
    /// the dialogue after the writer queued its presentation event.
    ScriptedDialogue,
}

/// Channel-2 (immediate sim-level) delivery of scenario content into the Comms system.
/// Fired by the world engine instead of mutating `CommsInboxRes` directly; consumed by
/// `handle_comms_channel2` in the Broadcast set.
#[derive(Message, Clone, Debug)]
pub struct CommsChannel2Event {
    pub message: CommsMessage,
    pub delivery: CommsChannel2Delivery,
}

impl CommsChannel2Event {
    /// Generic, unconditional channel-2 injection.
    pub fn generic(message: CommsMessage) -> Self {
        Self {
            message,
            delivery: CommsChannel2Delivery::Generic,
        }
    }

    /// Presentation half of an authoritative scripted dialogue.
    pub fn scripted_dialogue(message: CommsMessage) -> Self {
        Self {
            message,
            delivery: CommsChannel2Delivery::ScriptedDialogue,
        }
    }
}

/// Retire every active dialogue owned by `origin_layer`, plus the visible inbox
/// and viewscreen rows belonging to those dialogue threads.
///
/// Idempotent by construction: after the first call there are no matching
/// `active_dialogues`, so a later call from the actual unload is a no-op. The
/// early Input call closes the response race; the unload call is the lifecycle
/// backstop for fixtures and for changes queued later in the tick.
pub(crate) fn retire_layer_owned_dialogues(
    origin_layer: &str,
    comms: &mut CommsRuntime,
    inbox: Option<&mut CommsInboxRes>,
    on_screen: Option<&mut OnScreenMessage>,
) -> usize {
    let mut retired_message_ids = HashSet::new();
    comms.active_dialogues.retain(|message_id, dialogue| {
        let retire = dialogue.script.origin_layer.as_deref() == Some(origin_layer);
        if retire {
            retired_message_ids.insert(message_id.clone());
        }
        !retire
    });
    if retired_message_ids.is_empty() {
        return 0;
    }

    comms.needs_broadcast = true;
    if let Some(inbox) = inbox {
        let stale_ids: Vec<String> = inbox
            .0
            .messages()
            .into_iter()
            // Thread ids are authored/local and can be shared by another
            // owner. Only the active message id is authoritative ownership.
            .filter(|message| retired_message_ids.contains(&message.id))
            .map(|message| message.id)
            .collect();
        for message_id in stale_ids {
            inbox.0.remove(&message_id);
        }
    }
    if let Some(on_screen) = on_screen {
        let is_stale = on_screen
            .0
            .as_ref()
            .is_some_and(|message| retired_message_ids.contains(&message.id));
        if is_stale {
            on_screen.0 = None;
        }
    }
    retired_message_ids.len()
}

/// Close layer-owned response surfaces before either Backfill or a human can
/// answer them in `SimSet::Input`.
pub(crate) fn retire_dialogues_for_pending_layer_unloads(
    pending: Option<Res<crate::world::server::PendingWorldLayerChanges>>,
    mut comms: Option<ResMut<CommsRuntime>>,
    mut inbox: Option<ResMut<CommsInboxRes>>,
    mut on_screen: Option<ResMut<OnScreenMessage>>,
) {
    let (Some(pending), Some(comms)) = (pending.as_deref(), comms.as_deref_mut()) else {
        return;
    };
    let mut seen = HashSet::new();
    for change in &pending.0 {
        let crate::world::server::WorldLayerChange::Unload(path) = change else {
            continue;
        };
        if seen.insert(path.as_str()) {
            retire_layer_owned_dialogues(
                path,
                comms,
                inbox.as_deref_mut(),
                on_screen.as_deref_mut(),
            );
        }
    }
}

// -- Plugin ------------------------------------------------------------------

/// Registers the comms resources, messages, and systems in the SAME sets and
/// relative order the pre-#816 `WorldPlugin` used:
///
/// * Input: pending-layer dialogue retirement, then the four console command
///   handlers (from `console::comms`).
/// * Broadcast chain: `handle_comms_channel2` → `auto_clear_on_screen_message`
///   → `update_comms_range_flags` → `broadcast_comms_state` (the world's
///   `broadcast_objective_summary` orders itself `.after(broadcast_comms_state)`).
/// * Physics: `open_scripted_comms_threads`, between the script callback drain
///   and the delayed-action queue.
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
            crate::ship::system_registry::COMMS_KIND,
            crate::ship::system_registry::COMMS_SYSTEM_ID,
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
                OnEnter(crate::core::messages::GamePhase::InProgress),
                mark_comms_dirty_on_game_start,
            )
            .add_systems(
                FixedUpdate,
                (
                    // Unload wins a response tie. This precedes `handle_hail`;
                    // the console plugin orders response AI after hail and
                    // before the router, placing both AI and human response
                    // paths after cleanup.
                    retire_dialogues_for_pending_layer_unloads
                        .in_set(crate::sim_sets::SimSet::Input),
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
            // The scripted-thread drain (issue #984): after BOTH script call
            // sites have queued their opens, before the delayed queue a dialogue
            // fn's own `in_seconds` effect joins.
            //
            // It is the only comms system left in `Physics`. `tick_pending_follow_ups`
            // and `inject_comms_templates` sat here with it, ordered around
            // `collect_world_events` / `tick_trigger_pipeline` (#718/#719) because
            // they read world EVENTS to decide when a declarative template fired
            // and when a queued follow-up was due. Issue #985 deleted both with the
            // `[[comms]]` front-end: a scripted thread opens from a script handler
            // (an `on_hailed` trigger fn calling `open_comms`), so the event->comms
            // edge those orderings protected now runs through the trigger pipeline
            // itself and needs no comms-side ordering of its own.
            .add_systems(
                FixedUpdate,
                crate::comms::scripted::open_scripted_comms_threads
                    .after(crate::world::server::tick_script_callbacks)
                    .before(crate::world::server::tick_delayed_actions)
                    .in_set(crate::sim_sets::SimSet::Physics),
            );
    }
}

// -- Startup -----------------------------------------------------------------

/// Startup system: mark the comms runtime dirty when a world is loaded, so the
/// first `broadcast_comms_state` fires.
///
/// When no `WorldConfig` resource is present (native unit tests) this is a
/// no-op and comms systems remain quiet.
pub(crate) fn init_comms_runtime(
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut comms: ResMut<CommsRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    if world_config.is_none() {
        return;
    }
    // Nothing is derived from the world any more: issue #985 deleted the
    // `[[comms]]` front-end, and with it the template table this used to build
    // and the contact roster it used to seed from each template's `from`. The
    // roster's one source is now the live ECS — every entity carrying
    // `[comms] hailable = true` — which `update_comms_range_flags` unions in
    // every tick (`crate::comms::roster`). What survives here is the reason the
    // system existed at Startup at all: telling the first broadcast there is
    // something to send.
    comms.needs_broadcast = true;
    inbox.0.mark_dirty();
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
    if comms.contacts.is_empty() {
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
    view_mode_q: Query<&crate::ship::state::ShipViewMode, With<crate::server_app::LocalShip>>,
) {
    if on_screen.0.is_none() {
        return;
    }
    let current_view = view_mode_q
        .single()
        .map(|vm| vm.view_mode.clone())
        .unwrap_or(crate::core::messages::ViewMode::Camera(
            crate::core::messages::CameraView::default(),
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

/// Recompute per-entity comms-range flags from ship + entity transforms, and
/// maintain the entity-derived half of the hail roster.
///
/// Runs before `broadcast_comms_state`. Finds the player ship (entity with
/// `Ship` marker + `Transform` + optional `CommsRange`) and computes
/// `crate::comms::in_range(distance, ship_range, entity_range)` for every
/// entity carrying `EntityUuid` + `Transform` + `CommsRange`. Updates the
/// `comms.range_flags` map and stamps `comms.contacts[i].in_range`. Sets
/// `comms.needs_broadcast = true` if any flag flipped vs. the prior snapshot.
///
/// This is also the roster's LIFECYCLE system (issue #985). It already pruned
/// contacts whose entity has left the ECS; it now also ADDS a contact for every
/// live entity that opted in with `[comms] hailable = true`
/// (`crate::comms::CommsHailable`). The same live query drives both halves, so a
/// hailable entity appears on the roster the tick after it spawns and drops off
/// the tick after it is destroyed. Since issue #985 deleted the `[[comms]]`
/// front-end this is the roster's ONLY source; the union rule and the
/// deterministic append order still live in `crate::comms::roster`, where they
/// now keep the per-tick re-derivation idempotent.
pub(crate) fn update_comms_range_flags(
    mut comms: ResMut<CommsRuntime>,
    ship_q: Query<
        (&Transform, Option<&crate::comms::CommsRange>),
        With<crate::server_app::LocalShip>,
    >,
    entity_q: Query<(
        &crate::entities::spawner::EntityUuid,
        &Transform,
        &crate::comms::CommsRange,
        Option<&crate::comms::CommsHailable>,
        Option<&crate::entities::spawner::EntityName>,
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

    // Build the live set of comms-range-bearing UUIDs and refresh flags. The
    // same pass collects the entity-derived hail candidates (issue #985) —
    // unsorted here, because ECS iteration order is archetype order;
    // `merge_entity_contacts` is what imposes the deterministic append order.
    let mut live: HashSet<String> = HashSet::new();
    let mut derived_contacts: Vec<crate::comms::EntityContact> = Vec::new();
    for (uuid, tf, range, hailable, entity_name) in entity_q.iter() {
        let dist = ship_pos.distance(tf.translation);
        let in_range = crate::comms::in_range(dist, ship_range, range.0);
        let prior = comms.range_flags.insert(uuid.0.clone(), in_range);
        if prior != Some(in_range) {
            any_changed = true;
        }
        live.insert(uuid.0.clone());
        if let Some(hailable) = hailable {
            derived_contacts.push(crate::comms::EntityContact {
                name: crate::comms::entity_contact_label(
                    hailable.display_name.as_deref(),
                    entity_name.map(|n| n.0.as_str()),
                    &uuid.0,
                ),
                uuid: uuid.0.clone(),
            });
        }
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

    // Union in the entity-derived contacts (issue #985), AFTER the prune so a
    // candidate is never added and dropped in the same pass, and BEFORE the
    // range stamp below so a freshly added contact gets its real `in_range` on
    // the tick it appears rather than a default-true first frame. Declarative
    // entries win a UUID collision, which is what keeps every shipped world's
    // roster byte-identical while `[[comms]]` still exists.
    if crate::comms::merge_entity_contacts(&mut comms.contacts, &mut derived_contacts) {
        any_changed = true;
    }

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

/// Broadcast `CommsState` to whoever is currently hosting the Comms system
/// when the inbox is dirty or `CommsRuntime::needs_broadcast` is set.
///
/// The address is resolved through `command_admission::station_for_system`
/// (issue #984), which is the human-seeking host when one has been sought and
/// the `[[system]] id = "comms"` block's authored station otherwise. It used to
/// be a literal `StationId("comms")`, and that was a BUG on two shipped hulls:
/// the destroyer homes the comms SYSTEM on its `tactical` station and the
/// courier on `captain`, and neither hull declares a `comms` STATION at all —
/// so the lookup found no holder, took the early return, and those two hulls
/// never received a single `CommsState`. Their consoles ran purely off the
/// `Target::All` blackboard from `publish_comms_blackboard`. `SystemId` and
/// `StationId` coincide here on the cruiser and battleship by accident, which
/// is exactly what hid it — never cast one to the other.
pub(crate) fn broadcast_comms_state(
    sessions: Res<Sessions>,
    ship_query: Query<
        (
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&crate::ship_plugin::HumanSeekingHosts>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut comms: ResMut<CommsRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    objectives: Res<ObjectiveManagerRes>,
    mut outbox: ResMut<SimOutbox>,
) {
    // Resolve the current Comms host BEFORE the dirty gate: a change of host is
    // itself a reason to broadcast. A seat that becomes the resolved Comms host
    // on an otherwise-clean tick — a visiting host relocating after a disconnect,
    // or the original holder reconnecting and reclaiming the seat — must still
    // receive the current snapshot, and the targeted `CommsState` only ever goes
    // to the resolved host and only on a dirty tick.
    let Some((ship_config, seeking_hosts)) = ship_query.iter().next() else {
        return;
    };
    // A fixture whose LocalShip carries no `ShipConfigComponent`, or a hull
    // that declares no comms system, keeps the historical literal.
    let comms_station = ship_config
        .and_then(|c| {
            crate::command_admission::station_for_system(
                &c.0,
                seeking_hosts,
                &crate::ship::system_registry::comms_system_id(),
            )
        })
        .unwrap_or_else(|| StationId(crate::ship::system_registry::COMMS_SYSTEM_ID.into()));
    let comms_token = sessions
        .0
        .holder_for_station(&comms_station)
        .map(|t| t.to_string());

    let host_changed = comms.last_broadcast_host.as_deref() != comms_token.as_deref();
    let dirty =
        inbox.0.is_dirty() || comms.needs_broadcast || objectives.0.is_dirty() || host_changed;
    if !dirty {
        return;
    }

    let Some(comms_token) = comms_token else {
        inbox.0.mark_clean();
        comms.needs_broadcast = false;
        comms.last_broadcast_host = None;
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
        contact.is_urgent = messages.iter().any(|m| {
            m.sender_uuid == contact.uuid && m.effective_priority().is_urgent() && !m.is_read
        });
    }

    outbox.push_reliable((
        Target::Token(comms_token.to_string()),
        ServerMessage::CommsState {
            messages,
            objectives: objectives_snap,
            contacts,
        },
    ));

    inbox.0.mark_clean();
    comms.needs_broadcast = false;
    comms.last_broadcast_host = Some(comms_token);
}

#[cfg(test)]
#[path = "server_tests.rs"]
pub(crate) mod tests;
