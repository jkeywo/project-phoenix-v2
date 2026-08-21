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
use crate::comms_inbox::CommsInbox;
use crate::console::comms::server::{
    handle_clear_comms, handle_comms_channel2, handle_hail, handle_respond_to_message,
    handle_show_on_screen,
};
use crate::lobby::{Sessions, Target};
use crate::messages::{CommsContact, CommsMessage, ServerMessage, StationId, ViewMode};
use crate::simulation::SimOutbox;
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
                FixedUpdate,
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
        With<crate::simulation::LocalShip>,
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
        With<crate::simulation::LocalShip>,
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
                &crate::system_registry::comms_system_id(),
            )
        })
        .unwrap_or_else(|| StationId(crate::system_registry::COMMS_SYSTEM_ID.into()));
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
    comms.last_broadcast_host = Some(comms_token);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
    use crate::messages::*;
    use crate::world::server::tests::ai_trigger_test_app;
    use crate::world::server::{broadcast_objective_summary, WorldContentRuntime};

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
                FixedUpdate,
                (
                    handle_hail,
                    handle_respond_to_message,
                    handle_clear_comms,
                    handle_comms_channel2,
                    update_comms_range_flags,
                    broadcast_comms_state,
                    broadcast_objective_summary,
                )
                    .chain()
                    .after(crate::server_app::AdmissionSet),
            )
            .add_systems(PostUpdate, collect);
        // One fixed step per update (issue #895): the chain above joins the
        // admission seam in `FixedUpdate`, and each harness tick steps it once.
        crate::ship::test_support::drive_one_fixed_step_per_update(
            &mut app,
            std::time::Duration::from_millis(1),
        );
        app.world_mut().spawn((
            crate::simulation::Ship,
            crate::simulation::LocalShip,
            crate::ship_plugin::ShipConfigComponent::default(),
            crate::ship_plugin::ShipSystemControlSources::default(),
            crate::ship_plugin::ActiveStationRatings::default(),
            crate::ship_plugin::CoordinationQueue::default(),
            crate::messages::AdmittedCommands::default(),
            // The AUTHORED Comms console AI pair every shipped hull carries.
            // Since #885b stage 5d neither host has a synthesised fallback, so a
            // fixture whose subject is the AI answering (or being refused by)
            // the router has to attach the declarations a real hull writes.
            crate::console::comms::server::CommsResponseAiPolicy(
                crate::entities::authored_ai_pins::shipped_policy_toml("comms_response")
                    .to_policy()
                    .expect("the shipped Comms response policy decodes"),
            ),
            crate::console::comms::server::CommsTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail")
                    .to_selector()
                    .expect("the shipped Comms hail selector decodes"),
                power_rating: None,
            },
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

        // Seat the station as a hail contact directly, so these tests are
        // independent of both TOML loading and the entity-derived roster pass.
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
        comms.needs_broadcast = true;
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

        // Seat a message from the station while it is in range. (It used to
        // arrive from a `[[comms]] on_hailed` template; issue #985 deleted that
        // front-end, and the stamp under test reads the inbox, not the source.)
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(CommsMessage::injected(
                "msg-range-far".into(),
                station_uuid.into(),
                "Starbase Alpha".into(),
                "Go ahead, Phoenix.".into(),
                Default::default(),
                vec![],
                "thread-range-far".into(),
                true,
                false,
            ));
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
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(CommsMessage::injected(
                "msg-range-near".into(),
                station_uuid.into(),
                "Starbase Alpha".into(),
                "Go ahead, Phoenix.".into(),
                Default::default(),
                vec![],
                "thread-range-near".into(),
                true,
                false,
            ));
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
    ///
    /// The message is injected straight into the inbox rather than produced by a
    /// hail. It used to come from a `[[comms]] on_hailed` template, and issue
    /// #985 deleted that front-end; what is under test here is the BROADCAST
    /// stamp, which reads the inbox and the range map and does not care which
    /// front-end seated the row.
    #[test]
    fn comms_state_projects_important_and_available_onto_responses() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456761";
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
        {
            let mut inbox = app.world_mut().resource_mut::<CommsInboxRes>();
            inbox.0.inject(CommsMessage::injected(
                "msg-important".into(),
                station_uuid.into(),
                "Starbase Alpha".into(),
                "USS Phoenix, please identify yourself.".into(),
                Default::default(),
                vec![CommsResponseView {
                    text: "We are on a survey mission.".into(),
                    important: true,
                    available: true,
                }],
                "thread-important".into(),
                true,
                false,
            ));
        }
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

    // -- Entity-derived hail contacts (#985) ----------------------------------

    /// Broadcast contacts, in the order the Comms console receives them.
    fn broadcast_contacts(out: &[OutboundMessage]) -> Vec<CommsContact> {
        out.iter()
            .find_map(|m| {
                if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                    Some(contacts.clone())
                } else {
                    None
                }
            })
            .expect("CommsState must be broadcast")
    }

    /// A `CommsRange` entity that did NOT opt in stays off the roster. This is
    /// the whole reason `hailable` exists: every shipped warship and station
    /// declares a range, so range-only derivation would put the entire
    /// `combat_test` wave order on the Comms officer's contact list.
    #[test]
    fn a_comms_range_entity_without_the_opt_in_is_not_a_contact() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::{EntityName, EntityUuid};
        use crate::simulation::Ship;

        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, "declared-station");
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        // The seated contact's own entity, so it survives the prune.
        app.world_mut().spawn((
            EntityUuid("declared-station".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        // A range-bearing enemy that never opted in.
        app.world_mut().spawn((
            EntityUuid("harrow-wave-1".into()),
            EntityName("wave_1".into()),
            Transform::from_xyz(20.0, 0.0, 0.0),
            CommsRange(600.0),
        ));

        let contacts = broadcast_contacts(&tick(&mut app));
        assert_eq!(
            contacts.iter().map(|c| c.uuid.as_str()).collect::<Vec<_>>(),
            vec!["declared-station"],
            "only the seated contact belongs on the roster, got {contacts:?}"
        );
    }

    /// The opt-in marker puts a live entity on the roster, labelled from its
    /// `EntityName` (the world's `name` reference id — the same string a
    /// `[[comms]] from` would have carried).
    #[test]
    fn a_hailable_entity_joins_the_roster_labelled_from_its_entity_name() {
        use crate::comms::{CommsHailable, CommsRange};
        use crate::entities::spawner::{EntityName, EntityUuid};
        use crate::simulation::Ship;

        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, "declared-station");
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid("declared-station".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid("courier-uuid".into()),
            EntityName("world.entity.courier.name".into()),
            Transform::from_xyz(30.0, 0.0, 0.0),
            CommsRange(500.0),
            CommsHailable::default(),
        ));

        let contacts = broadcast_contacts(&tick(&mut app));
        let derived = contacts
            .iter()
            .find(|c| c.uuid == "courier-uuid")
            .expect("the hailable entity must join the roster");
        assert_eq!(derived.name, "world.entity.courier.name");
        assert!(
            derived.in_range,
            "the new contact must carry its real range stamp on the tick it appears"
        );
        // The seated entry is still first — entity-derived contacts are
        // APPENDED, never interleaved.
        assert_eq!(contacts[0].uuid, "declared-station");
    }

    /// An authored `[comms] display_name` beats the reference id, and the
    /// out-of-range stamp reaches an entity-derived contact.
    #[test]
    fn an_entity_derived_contact_uses_its_authored_display_name_and_range_stamp() {
        use crate::comms::{CommsHailable, CommsRange};
        use crate::entities::spawner::{EntityName, EntityUuid};
        use crate::simulation::Ship;

        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, "declared-station");
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(100.0),
        ));
        app.world_mut().spawn((
            EntityUuid("outpost-uuid".into()),
            EntityName("world.entity.outpost.name".into()),
            Transform::from_xyz(5_000.0, 0.0, 0.0),
            CommsRange(100.0),
            CommsHailable {
                display_name: Some("Relay Outpost".into()),
            },
        ));

        let contacts = broadcast_contacts(&tick(&mut app));
        let derived = contacts
            .iter()
            .find(|c| c.uuid == "outpost-uuid")
            .expect("the hailable entity must join the roster");
        assert_eq!(derived.name, "Relay Outpost");
        assert!(
            !derived.in_range,
            "a distant entity-derived contact must be stamped out of range"
        );
    }

    /// Lifecycle: a hailable entity that spawns mid-mission joins the roster,
    /// and leaves it when it is destroyed. Same live query that drives the
    /// range flags, so the two can never disagree.
    #[test]
    fn an_entity_derived_contact_appears_on_spawn_and_drops_on_despawn() {
        use crate::comms::{CommsHailable, CommsRange};
        use crate::entities::spawner::{EntityName, EntityUuid};
        use crate::simulation::Ship;

        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, "declared-station");
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid("declared-station".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        let before = broadcast_contacts(&tick(&mut app));
        assert!(!before.iter().any(|c| c.uuid == "reinforcement-uuid"));

        // Spawn.
        let reinforcement = app
            .world_mut()
            .spawn((
                EntityUuid("reinforcement-uuid".into()),
                EntityName("world.entity.reinforcement.name".into()),
                Transform::from_xyz(40.0, 0.0, 0.0),
                CommsRange(500.0),
                CommsHailable::default(),
            ))
            .id();
        let after_spawn = broadcast_contacts(&tick(&mut app));
        assert!(
            after_spawn.iter().any(|c| c.uuid == "reinforcement-uuid"),
            "contact must appear when the entity spawns, got {after_spawn:?}"
        );

        // Despawn.
        app.world_mut().entity_mut(reinforcement).despawn();
        let after_despawn = broadcast_contacts(&tick(&mut app));
        assert!(
            !after_despawn.iter().any(|c| c.uuid == "reinforcement-uuid"),
            "contact must drop when the entity is destroyed, got {after_despawn:?}"
        );
        assert!(
            after_despawn.iter().any(|c| c.uuid == "declared-station"),
            "the seated contact must survive the despawn of an unrelated entity"
        );
    }

    /// A roster row already seated for a UUID and an entity-derived candidate
    /// naming the SAME entity collapse to a single row, and the seated row's
    /// display metadata is the one that survives.
    ///
    /// The seated row used to be a declarative `[[comms]]` contact, which is
    /// what made the rule "declarative wins"; issue #985 deleted that source, so
    /// what the rule now protects is idempotency across ticks — the roster is
    /// re-merged every tick, and a contact must not lose its label (or its
    /// `in_range` / `is_urgent` stamps) to its own re-derivation.
    #[test]
    fn a_seated_contact_wins_the_uuid_collision_with_its_entity() {
        use crate::comms::{CommsHailable, CommsRange};
        use crate::entities::spawner::{EntityName, EntityUuid};
        use crate::simulation::Ship;

        let mut app = comms_test_app();
        // `setup_game_with_comms` seats the contact named "Starbase Alpha" for
        // this UUID.
        setup_game_with_comms(&mut app, "starbase-uuid");
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid("starbase-uuid".into()),
            EntityName("world.entity.starbase_alpha.name".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            CommsRange(500.0),
            CommsHailable {
                display_name: Some("Ignored By The Declarative Entry".into()),
            },
        ));

        let contacts = broadcast_contacts(&tick(&mut app));
        assert_eq!(
            contacts.len(),
            1,
            "the two sources must collapse to one row, got {contacts:?}"
        );
        assert_eq!(contacts[0].name, "Starbase Alpha");
    }

    /// Roster order must not depend on ECS archetype iteration order: several
    /// hailable entities land in `(name, uuid)` order no matter what.
    #[test]
    fn entity_derived_contacts_are_appended_in_deterministic_order() {
        use crate::comms::{CommsHailable, CommsRange};
        use crate::entities::spawner::{EntityName, EntityUuid};
        use crate::simulation::Ship;

        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, "declared-station");
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        app.world_mut().spawn((
            EntityUuid("declared-station".into()),
            Transform::from_xyz(10.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        // Spawned in a deliberately unsorted order.
        for (uuid, name) in [
            ("u-delta", "Delta"),
            ("u-alpha", "Alpha"),
            ("u-charlie", "Charlie"),
            ("u-bravo", "Bravo"),
        ] {
            app.world_mut().spawn((
                EntityUuid(uuid.into()),
                EntityName(name.into()),
                Transform::from_xyz(25.0, 0.0, 0.0),
                CommsRange(500.0),
                CommsHailable::default(),
            ));
        }

        let contacts = broadcast_contacts(&tick(&mut app));
        assert_eq!(
            contacts.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["Starbase Alpha", "Alpha", "Bravo", "Charlie", "Delta"],
            "seated first, then entity-derived in (name, uuid) order"
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

    // -- Issue #506: channel-2 routing tests ----------------------------------

    /// Scenario content arrives in `CommsInboxRes` via channel-2: a producer
    /// writes `CommsChannel2Event`, `handle_comms_channel2` injects it.
    ///
    /// The producer used to be `inject_comms_templates` firing a `[[comms]]`
    /// template on `WorldLoaded`; issue #985 deleted it, and the producer is now
    /// `open_scripted_comms_threads` (covered in `comms::scripted`). The event is
    /// written directly here because the DELIVERY leg is what this test is for.
    #[test]
    fn scenario_hail_arrives_in_inbox_via_channel2() {
        let mut app = ai_trigger_test_app();

        app.world_mut().write_message(CommsChannel2Event {
            message: CommsMessage::injected(
                "msg-ch2".into(),
                "outpost-uuid-ch2".into(),
                "Outpost Alpha".into(),
                "Channel-2 test message.".into(),
                Default::default(),
                vec![],
                "thread-ch2".into(),
                true,
                false,
            ),
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

        app.world_mut().write_message(CommsChannel2Event {
            message: CommsMessage::injected(
                "msg-ai-ch2".into(),
                "sector-hq-uuid".into(),
                "sector_hq".into(),
                "AI auto-respond test.".into(),
                Default::default(),
                vec![CommsResponseView {
                    text: "Acknowledged.".into(),
                    important: false,
                    available: true,
                }],
                "thread-ai-ch2".into(),
                true,
                false,
            ),
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
