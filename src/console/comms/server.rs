//! Server-side Comms console plugin (issue #427, migrated to blackboard #565).
//!
//! Issue #608: the comms conversation engine (hail / respond / clear /
//! show-on-screen / channel-2 handlers) lives here, alongside the
//! blackboard-publish system, so changing comms behaviour no longer
//! requires reaching into the world module.

use crate::ship_plugin::ShipSystemControlSources;
use bevy::prelude::*;

use crate::messages::{CommsBlackboard, ObjectiveSnapshot, SystemBlackboard, SystemId};
use crate::world::server::{ObjectiveManagerRes, WorldContentRuntime};

use crate::comms::content::ActiveDialogue;
use crate::comms::server::{
    current_sender_in_range, CommsChannel2Event, CommsInboxRes, CommsRuntime, OnScreenMessage,
};
use crate::entity_spawner::EntityUuid;
use crate::messages::{CommsMessage, GamePhase};
use crate::world::content::WorldEvent;
use crate::world::server::{ShipModifiersParams, WorldLayerParams};

pub struct CommsConsolePlugin;

impl Plugin for CommsConsolePlugin {
    fn build(&self, app: &mut App) {
        // The shared AI decision cadence (issue #889): `operate_comms_ai` and
        // `operate_comms_response_ai` were two of four hosts #895's FixedUpdate
        // migration left ungated — see the identical note on
        // `NavigationPlugin::build`. `register_ai_cadence` is idempotent, so
        // `CommsConsolePlugin` used standalone still gets `AiTickReady`
        // inserted rather than panicking on a missing `Res`.
        crate::ai::cadence::register_ai_cadence(app);
        app.add_systems(
            FixedUpdate,
            (
                publish_comms_blackboard.in_set(crate::sim_sets::SimSet::Publish),
                // `Input`, not `Physics`, and explicitly before `handle_hail`:
                // the `Hail` command the Comms AI emits must be consumed in the
                // SAME tick, exactly as `ai_torpedo_load` is ordered before
                // `handle_set_torpedo_volley_target` (issue #753).
                operate_comms_ai
                    .in_set(crate::sim_sets::SimSet::Input)
                    .before(handle_hail)
                    .run_if(crate::ai::cadence::ai_tick_ready),
                // Same shape for the dialogue-response policy (issue #786): the
                // `RespondToMessage` the Comms AI emits must be drained by the
                // SAME `handle_respond_to_message` router a human's response
                // goes through, in the same tick — so trigger actions fire and
                // follow-ups advance identically for both (AGENTS.md #6).
                // `.after(operate_comms_ai)` because that host now takes
                // `ResMut<CommsRuntime>` (the AC5 candidacy retirement) while
                // this one reads it: an explicit order, not an executor
                // coin-flip.
                // `.after(handle_hail)` closes the last unordered pair: without
                // it these two conflict on `CommsRuntime`, `WorldContentRuntime`
                // and the LocalShip's `AdmittedCommands` with no edge between
                // them, so the documented order was not actually total. No
                // cycle — `handle_hail` already precedes
                // `handle_respond_to_message`.
                operate_comms_response_ai
                    .in_set(crate::sim_sets::SimSet::Input)
                    .after(operate_comms_ai)
                    .after(handle_hail)
                    .before(handle_respond_to_message)
                    .run_if(crate::ai::cadence::ai_tick_ready),
            ),
        );
    }
}

// ── Blackboard publish ────────────────────────────────────────────────────────

fn publish_comms_blackboard(
    inbox: Option<Res<CommsInboxRes>>,
    runtime: Option<Res<CommsRuntime>>,
    objectives: Option<Res<ObjectiveManagerRes>>,
    // Local-ship world conditions for scoring the objective pool (issue #752):
    // comms hides zero-score doctrine objectives exactly as the captain panel
    // does, so it needs red-alert + hull to evaluate zero-gates / modifiers.
    local_conditions_q: Query<
        (
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::entity_spawner::EntitySystemHull>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut ship_q: Query<
        (
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
            &mut crate::server_app::ShipSystemBlackboards,
        ),
        With<crate::server_app::Ship>,
    >,
) {
    // Comms is fundamentally a player channel: the inbox, runtime, and
    // objective managers are singleton, player-session-scoped resources. The
    // shared content is therefore built ONCE from those singletons and
    // published into the LocalShip's blackboard. Every NPC ship still receives
    // a comms blackboard entry (AC #831: "NPC ships have comms blackboards")
    // for architectural consistency with the other per-entity systems, but it
    // is empty — an NPC carries no player messages, objectives, or contacts.
    // (Mirrors #830 navigation's `is_local`-gated per-Ship publish.)
    //
    // The three `Option<Res>` fallbacks are retained deliberately (issue #831
    // conservative prune): `ObjectiveManagerRes` is world-load-gated (only
    // inserted on world load, never `init_resource`d), and the console test
    // harness exercises the inbox/runtime paths without the full comms server
    // plugin — matching the #830 precedent that kept `ObjectiveManagerRes`.
    let mut messages = inbox.as_ref().map(|r| r.0.messages()).unwrap_or_default();

    if let Some(rt) = runtime.as_ref() {
        for m in messages.iter_mut() {
            if let Some(flag) = rt.range_flags.get(&m.sender_uuid).copied() {
                m.sender_in_range = flag;
            } else if rt.range_active && uuid::Uuid::parse_str(&m.sender_uuid).is_ok() {
                m.sender_in_range = false;
            }
            // Per-response availability tracks sender range (issue #761), the
            // same authoritative reachability that stamps `sender_in_range`.
            for r in m.responses.iter_mut() {
                r.available = m.sender_in_range;
            }
        }
    }

    // Score the active objective pool against the local ship's conditions and
    // apply the shared player-facing visibility filter (issue #752): mission
    // objectives are always shown; doctrine objectives are hidden while their
    // utility score is zero. Comms does not apply the captain boost — that is a
    // captain-scoped mechanism — so a zero-gated doctrine objective stays hidden
    // in comms until its own conditions lift it.
    let (red_alert, hull_fraction) = local_conditions_q
        .single()
        .ok()
        .map(|(ra, hull)| {
            let red_alert = ra.map(|r| r.0).unwrap_or(false);
            let hull_fraction = hull
                .map(|h| {
                    let max = h.0.total_max();
                    if max > 0.0 {
                        (h.0.total_current() / max).clamp(0.0, 1.0)
                    } else {
                        1.0
                    }
                })
                .unwrap_or(1.0);
            (red_alert, hull_fraction)
        })
        .unwrap_or((false, 1.0));
    let conditions = crate::objectives::WorldConditions {
        red_alert,
        hull_fraction,
        attacked: false,
    };
    let objectives_snap: Vec<ObjectiveSnapshot> = objectives
        .as_ref()
        .map(|o| {
            o.0.scored_pool(&conditions)
                .into_iter()
                .filter(crate::objectives::is_visible_objective)
                .map(|s| s.snapshot)
                .collect()
        })
        .unwrap_or_default();

    let mut contacts = runtime
        .as_ref()
        .map(|rt| rt.contacts.clone())
        .unwrap_or_default();
    for contact in contacts.iter_mut() {
        contact.is_urgent = messages
            .iter()
            .any(|m| m.sender_uuid == contact.uuid && m.is_urgent && !m.is_read);
    }

    let local_bb = CommsBlackboard {
        messages,
        objectives: objectives_snap,
        contacts,
    };

    let comms_key = SystemId(crate::system_registry::COMMS_SYSTEM_ID.to_string());
    for (is_local, mut bbs) in ship_q.iter_mut() {
        let bb = if is_local {
            local_bb.clone()
        } else {
            CommsBlackboard::default()
        };
        bbs.0.insert(comms_key.clone(), SystemBlackboard::Comms(bb));
    }
}

// ── Comms conversation handlers (issue #608, moved from world/server.rs) ──────

/// Handle `Hail { target_uuid }` messages from Comms console holders.
///
/// Range-gates the hail, records it on `CommsRuntime::open_hails`, and emits
/// `WorldEvent::Hailed` so a scripted `on_hailed` handler can answer it.
pub(crate) fn handle_hail(
    ship_query: Query<&crate::messages::AdmittedCommands, With<crate::simulation::LocalShip>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut comms: ResMut<CommsRuntime>,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::system_registry::COMMS_SYSTEM_ID) {
        let target_uuid = match &cmd.payload {
            crate::messages::SystemControlPayload::Hail { target_uuid } => target_uuid,
            _ => continue,
        };

        // Server-side range gate: when range tracking is active, the target
        // must be a known, in-range entity. Out-of-range hails are silently
        // dropped (clients enforce the same gate UX-side; this defends
        // against stale or malicious clients).
        if comms.range_active {
            match comms.range_flags.get(target_uuid).copied() {
                Some(true) => {}
                _ => continue,
            }
        }

        // Record the hail as authoritative comms state (issue #786). This is
        // the ONLY place a hail is known to have actually been issued: it is
        // past the range gate and applies identically to a human officer's hail
        // and a Backfill AI's. The Comms AI's anti-respam gate reads it back as
        // `candidate_fact(has_open_hail_thread)`; `handle_clear_comms` clears
        // it. Deliberately recorded before the event is routed, because a hail
        // that no handler answers still happened.
        comms.open_hails.insert(target_uuid.clone());

        // Route the Hailed event into the trigger system. This is now the ONLY
        // thing a hail does beyond recording itself: issue #985 deleted the
        // `[[comms]]` template table this used to evaluate, so a hail no longer
        // injects anything of its own. A scripted `on_hailed(entity, "handler")`
        // registration rides this same event and calls
        // `ctx.effects.open_comms(#{...})` — the same tick, because
        // `tick_trigger_pipeline` drains `pending_world_events` in
        // `SimSet::Physics`, which runs after `SimSet::Input`.
        runtime.pending_world_events.push(WorldEvent::Hailed {
            target_uuid: target_uuid.clone(),
        });
    }
}

/// Auxiliary params for [`handle_respond_to_message`], bundled into one
/// `SystemParam` so the system stays within Bevy's 16-argument limit
/// (issue #761 added the rejection-feedback seam). Carries the tick-scoped id
/// mint and balance-event ledger the shared dispatch pass needs, plus `Sessions` +
/// `SimOutbox` for routing `CommsResponseRejected` to the submitting holder, and
/// the script runtime + clock the scripted arm answers a dialogue with.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct CommsRespondAux<'w> {
    sessions: Res<'w, crate::lobby::Sessions>,
    outbox: ResMut<'w, crate::simulation::SimOutbox>,
    /// The tick-scoped id mint (issue #907): spawned-entity ids for the shared
    /// dispatch pass, and the follow-up message ids minted below it. Replaces
    /// the seeded `SimRng` this bundle used to carry — nothing in this system
    /// draws a random number any more, it only mints identities.
    id_mint: Option<Res<'w, crate::world_id::WorldIdMint>>,
    balance_events: Option<ResMut<'w, bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
    /// The Rhai runtime a scripted thread's `on_pick` fn runs on, plus the tick
    /// its deferred work is stamped against (issue #984). Bundled here rather
    /// than added as system params because the system is already at Bevy's
    /// argument limit — which is what this bundle exists for. Both halves are
    /// `Option`, so a script-free world (every shipped one) never reaches the
    /// scripted arm at all.
    script: crate::world::server::ScriptRuntimeParams<'w>,
    /// Mission-clock source for the same stamping. `Option` for the bare-`App`
    /// fixtures that run this handler without a `TimePlugin`.
    time: Option<Res<'w, bevy::time::Time>>,
}

/// Handle `RespondToMessage { message_id, response_index }` from Comms holders.
///
/// Calls the picked response's `on_pick` fn, applies the effects it buffered,
/// records the choice on the inbox message, and advances the dialogue to the
/// follow-up node the fn returned (if any).
pub(crate) fn handle_respond_to_message(
    ship_query: Query<
        (
            &crate::messages::AdmittedCommands,
            // The comms rejection channel is addressed to whoever is HOSTING
            // the comms system (issue #984), which on the destroyer is the
            // Tactical seat and on the courier the Captain's — resolved, never
            // string-cast from the system id.
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&crate::ship_plugin::HumanSeekingHosts>,
        ),
        With<crate::simulation::LocalShip>,
    >,
    mut runtime: ResMut<WorldContentRuntime>,
    mut comms: ResMut<CommsRuntime>,
    mut inbox: ResMut<CommsInboxRes>,
    mut channel2_writer: MessageWriter<CommsChannel2Event>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut commands: Commands,
    mut ai_query: Query<
        (
            &EntityUuid,
            Option<&mut crate::weapons_plugin::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<crate::entity_spawner::BehaviourSection>,
    >,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
    mut world_layers: WorldLayerParams,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
    mut faction_dispatch: crate::world::server::FactionDispatchParams,
    // Bundled to stay within Bevy's 16-argument system limit: the seeded RNG
    // and balance-event ledger the dispatch pass needs, plus the issue-#761
    // rejection-feedback seam (`Sessions` + `SimOutbox`) addressed to the
    // submitting Comms holder.
    mut aux: CommsRespondAux,
) {
    let Some((admitted, ship_config, seeking_hosts)) = ship_query.iter().next() else {
        return;
    };
    // Resolve the submitting comms token once per tick: the rejection channel
    // (issue #761) targets whoever currently holds the Comms console — the
    // station `station_for_system` resolves for the comms SYSTEM, which is the
    // sought human-seeking host when there is one and the hull's authored
    // station otherwise.
    let comms_station = ship_config
        .and_then(|c| {
            crate::command_admission::station_for_system(
                &c.0,
                seeking_hosts,
                &crate::system_registry::comms_system_id(),
            )
        })
        .unwrap_or_else(|| {
            crate::messages::StationId(crate::system_registry::COMMS_SYSTEM_ID.into())
        });
    let comms_token = aux
        .sessions
        .0
        .holder_for_station(&comms_station)
        .map(|t| t.to_string());
    // Helper: push a `CommsResponseRejected` for the attempted control so the
    // client can flash it red. A no-op when no comms holder is seated.
    let reject = |outbox: &mut crate::simulation::SimOutbox, message_id: &str, idx: usize| {
        if let Some(token) = comms_token.as_deref() {
            outbox.0.push((
                crate::lobby::Target::Token(token.to_string()),
                crate::messages::ServerMessage::CommsResponseRejected {
                    message_id: message_id.to_string(),
                    response_index: idx,
                },
            ));
        }
    };
    for cmd in admitted.for_target(crate::system_registry::COMMS_SYSTEM_ID) {
        let (message_id, response_index) = match &cmd.payload {
            crate::messages::SystemControlPayload::RespondToMessage {
                message_id,
                response_index,
            } => (message_id, response_index),
            _ => continue,
        };

        // Look up active dialogue for this message.
        let dialogue = match comms.active_dialogues.get(message_id) {
            Some(d) => d.clone(),
            None => {
                // Stale submission: the message has no active dialogue (already
                // responded to, cleared, or never existed). Reject so the
                // client flashes the attempted control red (issue #761 AC3).
                reject(&mut aux.outbox, message_id, *response_index);
                continue;
            }
        };

        // Server-side range gate: if range tracking is active, the sender
        // of this message must currently be in range. Out-of-range responses
        // are rejected (issue #761): forced/stale submissions on a greyed
        // response are refused and the attempted control flashes red.
        if comms.range_active {
            let sender_uuid = inbox.0.sender_uuid_for(message_id).unwrap_or_default();
            match comms.range_flags.get(&sender_uuid).copied() {
                Some(true) => {}
                _ => {
                    reject(&mut aux.outbox, message_id, *response_index);
                    continue;
                }
            }
        }

        let responses = &dialogue.current_node.responses;
        if *response_index >= responses.len() {
            // Out-of-bounds index (forced/stale client): reject.
            reject(&mut aux.outbox, message_id, *response_index);
            continue;
        }

        // The bindings the response's effects are applied through.
        //
        // Issue #722: an `on_pick`'s effects reach the world through the shared
        // `world::dispatch` table (issue #708/#710), the same table
        // `tick_trigger_pipeline` and `tick_delayed_actions` use — so a scripted
        // `spawn_entity` mints its uuid from the real `WorldIdMint` in the same
        // order its trigger-authored twin would. A comms thread carries no
        // originating sub-world layer, so `origin_layer` is always `None` here
        // and every layer-scoped action resolves against the base world. This is
        // a single-shot dispatch, not a chaining pass: `new_events` are routed
        // onto `runtime.pending_world_events` so `tick_trigger_pipeline` (later
        // in the same tick via `SimSet::Physics`, since this handler runs in
        // `SimSet::Input`) picks them up and fires any chained
        // `on_flag_set` / `on_flag_cleared` / `on_destroyed` triggers.
        let origin_layer: Option<String> = None;
        let name_to_uuid_snapshot = runtime.name_to_uuid.clone();
        // Build reverse map (UUID → entity name) so `AddObjective`'s
        // empty-`targets` fallback can resolve to the comms sender's name.
        // Comms has no `TriggerState.entity_name` to fall back to instead —
        // this pre-resolved name is what gets passed as
        // `DispatchContext::entity_name` below.
        let uuid_to_name: std::collections::HashMap<&str, &str> = name_to_uuid_snapshot
            .iter()
            .map(|(name, uuid)| (uuid.as_str(), name.as_str()))
            .collect();
        // Build UUID → ECS Entity map once per response so the applier's
        // per-entity modifier/flag commands can resolve their `entity`
        // target in O(1) instead of scanning `entity_uuid_query` each time.
        let uuid_to_entity: std::collections::HashMap<String, Entity> = entity_uuid_query
            .iter()
            .map(|(ent, uuid_comp)| (uuid_comp.0.clone(), ent))
            .collect();
        let sender_uuid = inbox.0.sender_uuid_for(message_id);
        let sender_entity_name: Option<String> = sender_uuid
            .as_deref()
            .and_then(|suid| uuid_to_name.get(suid).copied())
            .map(String::from);

        let empty_anchors: std::collections::HashMap<String, [f32; 3]> =
            std::collections::HashMap::new();
        // Same template source `tick_trigger_pipeline` / `tick_delayed_actions`
        // use (issue #715): built once per system run, config cache first,
        // filesystem fallback on native. Collapses comms's old three-message
        // cfg-split loader (cache / native-fs / wasm) into this one path.
        let template_loader = crate::entity_loader::WasmTemplateLoader;
        // Seeded, for the same reason the trigger pipeline is: a spawned
        // entity's UUID keys the balance ledgers in the headless report.
        let uuid_source = || {
            crate::world_id::mint_id_with(
                aux.id_mint.as_deref(),
                crate::world_id::IdNamespace::Entity,
            )
        };

        // ── The dialogue's own effects (issue #984) ──────────────────────────
        //
        // Entering a node and picking a response are the SAME operation: call
        // the `on_pick` fn, take the effects it buffered, read the follow-up
        // node it returned. Issue #985 deleted the declarative arm that used to
        // sit beside this one and dispatch a `[[comms.response.action]]` array;
        // the bindings above were always built once and shared by both, and are
        // now simply this arm's.
        let sd = &dialogue.script;
        // Read the clock BEFORE borrowing the runtime, so the two reads of
        // `aux` stay sequential.
        let now_tick = aux.script.sim_tick.as_ref().map(|t| t.0).unwrap_or(0);
        let elapsed_secs = aux.time.as_ref().and_then(|t| {
            runtime
                .mission_clock_anchor_secs
                .map(|loaded| (t.elapsed_secs() - loaded).max(0.0))
        });
        let script_clock = crate::world::script::schedule::SchedClock {
            tick: now_tick,
            elapsed_secs: elapsed_secs.unwrap_or(0.0),
            tick_hz: world_layers
                .base_world_config
                .as_ref()
                .map(|wc| wc.global.sim_tick_hz)
                .unwrap_or(crate::world::script::schedule::SchedClock::ZERO.tick_hz),
        };

        let Some(sr) = aux.script.runtime.as_deref_mut() else {
            // A scripted dialogue with no script runtime behind it is an
            // inconsistent state (a reload dropped the runtime under a live
            // thread). Refuse rather than silently doing nothing.
            reject(&mut aux.outbox, message_id, *response_index);
            continue;
        };
        // Reset the shared budget once per tick, `SimTick`-keyed and
        // idempotent — the same block `tick_trigger_pipeline`,
        // `tick_script_callbacks` and `open_scripted_comms_threads` open
        // with. This handler runs in `SimSet::Input`, BEFORE any of them, so
        // without it the arm would read (and charge) the PREVIOUS tick's
        // budget: a stale trip would spuriously refuse this tick's pick, and
        // the charges it did make would land on a budget wiped moments later
        // in `Physics` — leaving live dialogue calls effectively unbudgeted.
        if sr.budget_tick != now_tick {
            sr.budget = crate::world::script::schedule::TickBudget::new();
            sr.budget_tick = now_tick;
        }
        // Issue #1050 / R5: a call the budget refuses produces nothing, and
        // "nothing" is what a terminal response that buffered nothing also
        // produces — so the refusal is caught BEFORE the call rather than
        // inferred from its result. `can_admit()`, not `tripped()`: the call
        // that REACHES the call cap is refused and trips the budget in one
        // step, so a `tripped()` pre-flight passes on a pick that is about to
        // be dropped. A player's pick the tick cannot afford therefore
        // flashes the attempted control red through the SAME `reject` closure
        // the stale, out-of-range and out-of-bounds refusals use, instead of
        // appearing to do nothing.
        if !sr.budget.can_admit() {
            reject(&mut aux.outbox, message_id, *response_index);
            continue;
        }
        // Parallel to the shown responses by construction (`project_node`),
        // so the bounds check above already covers this index; refused
        // rather than indexed, so a future drift cannot panic mid-mission.
        let Some(on_pick_fn) = sd.on_pick.get(*response_index).cloned() else {
            reject(&mut aux.outbox, message_id, *response_index);
            continue;
        };

        // Entering a node and picking a response are the SAME operation:
        // call the fn, take the effects it buffered, read the follow-up node
        // it returned. Disjoint field borrows so the one `&self` call takes
        // `&mut budget` and `&ast` at once while `&runtime.flags` (a
        // DISJOINT resource) is the flag overlay base.
        let entered = {
            let crate::world::server::WorldScriptRuntime {
                host, asts, budget, ..
            } = &mut *sr;
            match asts.get(&sd.script_path) {
                Some(ast) => Some(crate::world::script::comms::enter_node(
                    host,
                    budget,
                    &script_clock,
                    ast,
                    &sd.script_path,
                    &on_pick_fn,
                    &runtime.flags,
                    &runtime.deadlines,
                )),
                None => {
                    bevy::log::warn!(
                        "handle_respond_to_message: on_pick '{on_pick_fn}' names a missing \
                         unit '{}'",
                        sd.script_path
                    );
                    None
                }
            }
        };
        // A malformed return is refused like any other pick that produced no
        // node — but only AFTER the effects the call really did buffer have
        // been applied below, which is why it is carried as a flag rather
        // than `continue`d here. An unresolvable name and a refused call
        // produced nothing at all, so both refuse immediately.
        let mut malformed: Option<String> = None;
        let (effects, follow_up) = match entered {
            Some(Ok(pair)) => pair,
            Some(Err(crate::world::script::comms::EnterError::Shape { effects, message })) => {
                malformed = Some(message);
                (*effects, None)
            }
            Some(Err(err)) => {
                bevy::log::warn!("handle_respond_to_message: on_pick '{on_pick_fn}' {err}");
                reject(&mut aux.outbox, message_id, *response_index);
                continue;
            }
            None => {
                reject(&mut aux.outbox, message_id, *response_index);
                continue;
            }
        };

        let mut new_events: Vec<WorldEvent> = Vec::new();
        crate::world::server::apply_script_commands(
            effects.commands,
            "handle_respond_to_message (script)",
            &mut new_events,
            &uuid_to_entity,
            &mut runtime,
            &mut objectives,
            &mut commands,
            &mut ship_modifiers,
            world_layers.pending_layers.as_deref_mut(),
            world_layers.layer_map.as_deref_mut(),
            next_state.as_deref_mut(),
            game_over_reason.as_deref_mut(),
            &mut faction_dispatch,
            &mut ai_query,
            aux.balance_events.as_deref_mut(),
            &uuid_source,
            &template_loader,
            world_layers
                .base_world_config
                .as_ref()
                .map(|wc| &wc.anchors)
                .unwrap_or(&empty_anchors),
            origin_layer.clone(),
            sender_entity_name.clone(),
        );
        // Single-shot dispatch, not a chaining pass — `new_events` go onto
        // `pending_world_events` for `tick_trigger_pipeline` to observe, the
        // same routing the declarative arm below uses.
        runtime.pending_world_events.extend(new_events);
        // An `on_pick`'s own deferred work: `in_seconds` effects join the
        // delayed queue (dropped when the mission clock is unanchored, the
        // trigger path's rule), `after` callbacks join the callback queue,
        // and an `open_comms` from a response queues for the next drain —
        // which is how a DELAYED scripted reply is authored.
        if elapsed_secs.is_some() {
            runtime.pending_delayed_actions.extend(effects.delayed);
        }
        sr.pending_callbacks.extend(effects.callbacks);
        sr.pending_comms_opens.extend(effects.comms_opens);
        // And an `on_pick` that slipped or cancelled a named deadline (issue
        // #1024): the player's answer moves the mission's clock.
        crate::world::server::apply_deadline_changes(
            &effects.deadline_changes,
            &mut runtime.deadlines,
            &mut sr.pending_callbacks,
            script_clock.tick,
            script_clock.tick_hz,
        );

        // The malformed-return refusal, taken here so the effects the call
        // genuinely produced are applied first (see `EnterError::Shape`).
        // The pick itself is refused: nothing recorded, no node injected.
        if let Some(message) = malformed {
            bevy::log::warn!("handle_respond_to_message: on_pick '{on_pick_fn}': {message}");
            reject(&mut aux.outbox, message_id, *response_index);
            continue;
        }

        // Record the chosen response on the inbox message (the tail both
        // arms share).
        inbox.0.record_response(message_id, *response_index);
        // Issue #984 finding 9: retire the answered node's dialogue entry so a
        // duplicate submission on the same message id cannot re-run `on_pick` —
        // which for a spawning or objective-mutating response would apply its
        // effects twice. The second submission takes the stale-submission arm
        // above instead and flashes red. (`handle_clear_comms` still leaks
        // `active_dialogues` for messages that are never answered — issue
        // #1049.)
        comms.active_dialogues.remove(message_id);

        // Advance to the follow-up node the `on_pick` fn returned. `None` is
        // a terminal response and ends the thread.
        if let Some(node) = follow_up {
            let thread_id = dialogue.thread_id.clone();
            let sender_uuid = inbox.0.sender_uuid_for(message_id).unwrap_or_default();
            let sender_name = inbox.0.sender_name_for(message_id).unwrap_or_default();
            // R6: a follow-up INHERITS the urgency the thread was opened
            // with, so an urgent thread stays urgent as it advances. (The
            // declarative arm deleted in issue #985 hardcoded `urgent: false`
            // here, because follow-up urgency was not a TOML-level concept.)
            let urgent = inbox.0.is_urgent_for(message_id).unwrap_or(false);
            let (wire_node, on_pick) = crate::world::script::comms::project_node(&node);
            let new_msg_id = crate::world_id::mint_id_with(
                aux.id_mint.as_deref(),
                crate::world_id::IdNamespace::Message,
            );
            let available = current_sender_in_range(&comms, &sender_uuid);
            let new_responses =
                crate::comms::content::response_views(&wire_node.responses, available);
            let new_msg = CommsMessage::injected(
                new_msg_id.clone(),
                sender_uuid,
                sender_name,
                wire_node.body.clone(),
                new_responses,
                thread_id.clone(),
                available,
                urgent,
            );
            channel2_writer.write(CommsChannel2Event { message: new_msg });
            comms.active_dialogues.insert(
                new_msg_id,
                ActiveDialogue {
                    current_node: wire_node,
                    thread_id,
                    script: crate::comms::content::ScriptedDialogue {
                        script_path: sd.script_path.clone(),
                        node_fn: on_pick_fn,
                        on_pick,
                    },
                },
            );
        }
    }
}

/// Handle `ClearComms` from Comms console holders.
///
/// Clearing the inbox also retires the authoritative `CommsRuntime.open_hails`
/// record (issue #786): the officer has declared the comms slate empty, so a
/// standing `Hail` directive becomes eligible to hail its target ONE more time.
/// That re-arm cannot loop on its own — it needs another `ClearComms`.
///
/// This is a HUMAN path only. `ClearComms` is emitted by the Comms console
/// (`gui/comms-state.js`, `gui/action-map.js`) and by nothing else —
/// `TriggerAction` has no `ClearComms` variant, so no scenario can script one.
/// An unmanned (Backfill) ship therefore never reaches this handler at all, and
/// its latch re-arms through [`operate_comms_ai`]'s candidacy retirement
/// instead. See [`has_open_hail_thread_with`].
///
/// # Known leak: `active_dialogues` (issue #1049)
///
/// Clearing empties the inbox and `open_hails` but NOT
/// `CommsRuntime::active_dialogues`, so every cleared message's dialogue state
/// stays resident for the rest of the mission — unbounded in a long scenario, and
/// a submission against a cleared message id still resolves. Scripted threads
/// (issue #984) inherit the leak unchanged: their `ActiveDialogue` carries a
/// [`ScriptedDialogue`](crate::comms::content::ScriptedDialogue) and is retained
/// the same way. Fixing it is #1049's job, deliberately not this handler's — the
/// retention is load-bearing for the declarative follow-up path and unpicking it
/// is a behaviour change, not a cleanup.
pub(crate) fn handle_clear_comms(
    ship_query: Query<&crate::messages::AdmittedCommands, With<crate::simulation::LocalShip>>,
    mut inbox: ResMut<CommsInboxRes>,
    mut comms: ResMut<CommsRuntime>,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::system_registry::COMMS_SYSTEM_ID) {
        if matches!(
            cmd.payload,
            crate::messages::SystemControlPayload::ClearComms
        ) {
            inbox.0.clear();
            comms.open_hails.clear();
        }
    }
}

/// Handle `ShowOnScreen { message_id }` from Comms console holders.
///
/// Looks up the message in the inbox, stores it in `OnScreenMessage`, and
/// pushes `ViewMode::Comms` so the viewscreen switches to the comms overlay.
pub(crate) fn handle_show_on_screen(
    ship_query: Query<&crate::messages::AdmittedCommands, With<crate::simulation::LocalShip>>,
    inbox: Res<CommsInboxRes>,
    mut on_screen: ResMut<OnScreenMessage>,
    mut view_mode_q: Query<
        &mut crate::ship_state::ShipViewMode,
        With<crate::simulation::LocalShip>,
    >,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    let Some(mut vm) = view_mode_q.iter_mut().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::system_registry::COMMS_SYSTEM_ID) {
        let show_message_id: Option<&String> = match &cmd.payload {
            crate::messages::SystemControlPayload::ShowOnScreen { message_id } => Some(message_id),
            _ => None,
        };
        if let Some(message_id) = show_message_id {
            if let Some(msg) = inbox.0.messages().into_iter().find(|m| &m.id == message_id) {
                let already_on_screen = matches!(vm.view_mode, crate::messages::ViewMode::Comms)
                    && on_screen
                        .0
                        .as_ref()
                        .is_some_and(|displayed| displayed.id == msg.id);
                if already_on_screen {
                    on_screen.0 = None;
                    vm.restore_captain_view();
                } else {
                    on_screen.0 = Some(msg.clone());
                    // Only push the Comms overlay when it is not already the
                    // resolved view. `show_view_mode` now routes through the
                    // unified latest-wins arbiter method (issue #769), whose
                    // toggle-off dismisses a re-request of the active view —
                    // so re-issuing Comms while it is already showing (e.g.
                    // switching to a different on-screen message) must NOT call
                    // it, otherwise the overlay would be dismissed instead of
                    // updated.
                    if !matches!(vm.view_mode, crate::messages::ViewMode::Comms) {
                        vm.show_view_mode(crate::messages::ViewMode::Comms);
                    }
                }
            }
        }
    }
}

/// Consume channel-2 deliveries addressed to the Comms system.
///
/// Injects each message into `CommsInboxRes`. Inject-only.
///
/// # The retired AI auto-response stub (issue #786, AC3)
///
/// This system used to carry a three-line AI branch:
///
/// ```text
///   if policy.operate_ai && !ev.message.responses.is_empty() {
///       inbox.0.record_response(&ev.message.id, 0);
///   }
/// ```
///
/// which was a live AGENTS.md §6 violation. It wrote the authoritative inbox
/// DIRECTLY, so an AI answer bypassed admission entirely AND bypassed
/// [`handle_respond_to_message`] — meaning the response's `TriggerAction`s never
/// dispatched and its `follow_up` never advanced. AI and human "responding" were
/// two different operations wearing the same name.
///
/// It is retired outright. The decision now lives in
/// [`operate_comms_response_ai`], which emits an ordinary admitted
/// `RespondToMessage` for the SAME router a human's response traverses, so
/// actions fire and follow-ups advance identically for both. This system no
/// longer reads `ShipSystemControlSources` at all.
pub(crate) fn handle_comms_channel2(
    mut reader: MessageReader<CommsChannel2Event>,
    mut inbox: ResMut<CommsInboxRes>,
) {
    for ev in reader.read() {
        inbox.0.inject(ev.message.clone());
    }
}

// ── Comms AI: authored hail ranking + authored dialogue responses (#786) ─────
//
// Comms is the LAST Group A per-system Backfill policy to move onto the shared
// data-driven spine, and the FIRST system to author BOTH machines at once:
//
//   * WHO to hail → the #776 [`crate::ai::selector::TargetSelector`]. Hail
//     targets are contacts with REAL entity UUIDs, so `SelectorCandidate.uuid`
//     is genuine identity and the winning uuid drops straight into
//     `SystemControlPayload::Hail { target_uuid }` — no side-table at all
//     (contrast #778 Navigation's `uuid → WaypointMode` map and #785 Repair's
//     station-id keys). Comms is the FIFTH selector host.
//   * HOW to answer an open dialogue → the #775 channel/verb
//     [`crate::ai::policy::AiPolicy`]. Responses are a fixed, small,
//     INDEX-addressed set (`ActiveDialogue.current_node.responses`, addressed by
//     `usize`), which is the same argument that kept Shields (#783) on
//     channel/verb: there is no entity set for a candidate-source selector to
//     union.
//
// Both retired their predecessors outright — the hardcoded filter+argmax AND the
// `record_response(id, 0)` stub — leaving no residual Rust kernel (the #784/#785
// shape, not #783's retained kernel). No AI-owned state component survives.

/// Per-ship resolved Comms hail selector (issue #786).
///
/// Holds the ship's data-driven [`crate::ai::selector::TargetSelector`], decoded
/// from the authored `[comms_console.selector]` block, plus the authored ship
/// `power_rating`, which [`operate_comms_ai`] exposes to the selector's
/// expressions as `self_fact(power_rating)`. Attached at spawn beside the
/// Sensors/Tactical/Navigation/Repair selectors.
///
/// Since #885b stage 5d there is no Rust-side synthesised default behind it: a
/// ship without the component has no hail ranking and [`operate_comms_ai`] skips
/// it, rather than being handed automation nobody authored (PRD #774 US7).
/// Mirrors [`crate::console::navigation::NavigationTargetSelector`].
#[derive(Component, Clone, Debug)]
pub struct CommsTargetSelector {
    /// The resolved ranking policy.
    pub selector: crate::ai::selector::TargetSelector,
    /// Authored ship power rating, seeded from `EntityConfig.power_rating`.
    pub power_rating: Option<f32>,
}

/// Per-ship resolved Comms dialogue-response policy (issue #786).
///
/// Holds the ship's inline stateless [`crate::ai::policy::AiPolicy`], decoded
/// from the authored `[comms_console.ai]` block and resolved once per open
/// dialogue by [`operate_comms_response_ai`] on the single `comms_respond`
/// channel. Mirrors [`crate::ship::shields::ShieldsFocusAiPolicy`] /
/// `PowerAiPolicy`.
///
/// Since #885b stage 5d there is no synthesised default behind it: a ship
/// without the component answers nothing.
#[derive(Component, Clone, Debug)]
pub struct CommsResponseAiPolicy(pub crate::ai::policy::AiPolicy);

/// Per-ship multiplier on the shared AI base cadence for `[comms_console.ai]`
/// (issue #889's PASM-tracked runtime gap: `evaluate_every_ticks` was parsed
/// and validated but no host read it). `operate_comms_response_ai` decides on
/// every Nth arm of the shared `ai_tick_ready` latch rather than every arm.
///
/// A sibling component to [`CommsResponseAiPolicy`] rather than a field on it
/// (or on the shared [`crate::ai::policy::AiPolicy`] type), for the same
/// reason `PowerAiCadence` sits beside `PowerAiPolicy`: dozens of call sites
/// across the crate build an `AiPolicy` by literal, and a sibling component
/// keeps wiring one host's cadence from touching all of them.
///
/// `1` — the parse default, and what every shipped hull authors today — means
/// "every arm", identical to behaviour before this component existed.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct CommsResponseAiCadence(pub u32);

/// Resolve a ship's two Comms-console AI components from its `EntityConfig`
/// (issue #786).
///
/// ONE source of truth for BOTH spawn paths, because there are two and they do
/// not share code:
///
///   * [`crate::entities::spawner::spawn_entity`] — every world/NPC entity
///     carrying a `[behaviour]` block;
///   * `server_app::spawn_game_start_entities` — the PLAYER ship, which never
///     goes through the spawner at all.
///
/// Both `operate_comms_ai` and `operate_comms_response_ai` are filtered
/// `With<LocalShip>`, i.e. they run ONLY on the player ship, so the second call
/// site is the one that actually matters: without it `[comms_console]` parses,
/// validates, and is then silently ignored because the host's tick-local
/// canonical default always wins — and `self_fact/fact(power_rating)` is
/// permanently absent (the #779 empty-facts failure mode).
///
/// Each half is `None` when its block is unauthored, and the caller attaches
/// nothing for it — since #885b stage 5d there is no synthesised fallback, and
/// strict AI-declaration mode rejects an AI-capable hull that omits either block
/// at load. `to_selector`/`to_policy` cannot fail for an authored block: both
/// were validated in `EntityConfig::from_toml`.
pub fn comms_console_ai_components(
    config: &crate::entity_config::EntityConfig,
) -> (
    Option<CommsTargetSelector>,
    Option<CommsResponseAiPolicy>,
    Option<CommsResponseAiCadence>,
) {
    let selector = config
        .comms_console
        .as_ref()
        .and_then(|cc| cc.selector.as_ref())
        .map(|s| CommsTargetSelector {
            selector: s.to_selector().unwrap_or_default(),
            power_rating: config.power_rating.map(|r| r as f32),
        });
    let ai_cfg = config.comms_console.as_ref().and_then(|cc| cc.ai.as_ref());
    let policy = ai_cfg.map(|ai| CommsResponseAiPolicy(ai.to_policy().unwrap_or_default()));
    // Carried from the SAME authored block `policy` decodes from (issue #889's
    // evaluate_every_ticks): a resolved `AiPolicy` alone forgets this field, so
    // it rides alongside as a sibling component rather than being lost.
    let cadence = ai_cfg.map(|ai| CommsResponseAiCadence(ai.evaluate_every_ticks));
    (selector, policy, cadence)
}

/// One hail candidate's observable readings, resolved host-side before the pure
/// fact seed (issue #786). Every field is authoritative observable state — the
/// scored objective pool, the comms contact roster, the range flags, the inbox —
/// and nothing here is private AI memory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommsHailCandidateReading {
    /// A positive, Comms-relevant `Hail` directive currently names this target.
    pub source_hail_objective: bool,
    /// This target is on the authoritative comms contact roster
    /// (`CommsRuntime.contacts`) — the same list a human Comms officer hails
    /// from. Enriches a coincident directive; not independently eligible under
    /// the canonical policy.
    pub source_comms_contact: bool,
    /// Utility score of the originating objective (0.0 for a contact-only
    /// candidate). The banded score ladder ranks on this.
    pub objective_score: f32,
    /// Whether the target is currently within comms range. Honours
    /// `CommsRuntime.range_active`: while range tracking is inactive everything
    /// is in range, matching `current_sender_in_range`.
    pub in_range: bool,
    /// Whether the contact is flagged urgent on the roster.
    pub is_urgent: bool,
    /// Whether THIS ship has an outstanding hail to this target: a `Hail` that
    /// reached [`handle_hail`] and has not been retired by a `ClearComms`. The
    /// AC5 anti-respam reading — read off the authoritative
    /// `CommsRuntime.open_hails` record rather than AI memory, and deliberately
    /// NOT inferred from the inbox (see [`has_open_hail_thread_with`]).
    pub has_open_hail_thread: bool,
    /// Whether this contact has any un-orphaned message sitting in the inbox,
    /// or an open dialogue node awaiting an answer. Distinct from
    /// `has_open_hail_thread`: a scenario-pushed message (`on_world_loaded`, a
    /// follow-up, a distress call) sets THIS and not that, because we never
    /// hailed them. Seeded for authors; deliberately absent from the canonical
    /// eligibility, since "they messaged us" is no reason not to hail back.
    pub has_unread_from_sender: bool,
    /// Whether the originating objective is mandatory.
    pub mandatory: bool,
}

/// Seed one hail candidate's CANDIDATE-context facts (issue #786).
///
/// Pure and Bevy-free (AGENTS.md rule #10): the host resolves the live comms
/// readings before calling this. The #779 empty-facts lesson applies — EVERY
/// fact an authored guard could name is seeded here, so a `candidate_fact(...)`
/// guard actually fires instead of silently reading "absent → false".
pub fn seed_comms_hail_facts(reading: &CommsHailCandidateReading) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    facts.set("source_hail_objective", b(reading.source_hail_objective));
    facts.set("source_comms_contact", b(reading.source_comms_contact));
    facts.set("objective_score", reading.objective_score as f64);
    facts.set("in_range", b(reading.in_range));
    facts.set("is_urgent", b(reading.is_urgent));
    facts.set("has_open_hail_thread", b(reading.has_open_hail_thread));
    facts.set("has_unread_from_sender", b(reading.has_unread_from_sender));
    facts.set("mandatory", b(reading.mandatory));
    facts
}

/// Seed the operating ship's SELF-context facts for the hail selection
/// (issue #786). Pure and Bevy-free, same contract as
/// [`seed_comms_hail_facts`].
///
///   - `power_rating` — the authored ship power rating, ABSENT (not zero) when
///     the ship declares none, so `self_fact(power_rating)` guards do not fire
///     on an unrated ship.
///   - `comms_available` — 1.0 while the ship's own Comms fine system is not
///     Disabled/Destroyed (the AC2 system-availability reading, read off
///     `EntitySystemHull` exactly as the shields/power hosts do).
///   - `red_alert` — 1.0 while the ship is at red alert.
///   - `contact_count` — how many hailable contacts the roster carries.
pub fn seed_comms_self_facts(
    power_rating: Option<f32>,
    comms_available: bool,
    red_alert: bool,
    contact_count: usize,
) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set("comms_available", if comms_available { 1.0 } else { 0.0 });
    facts.set("red_alert", if red_alert { 1.0 } else { 0.0 });
    facts.set("contact_count", contact_count as f64);
    if let Some(pr) = power_rating {
        facts.set("power_rating", pr as f64);
    }
    facts
}

/// One open dialogue's observable readings, resolved host-side before the pure
/// fact seed (issue #786). As with the hail readings, every field is
/// authoritative: the inbox message, its active dialogue node, the range flags,
/// and the operating ship's own state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommsResponseReading {
    /// How many responses the open dialogue node offers.
    pub response_count: usize,
    /// How many of them are currently selectable (the sender is in range).
    pub available_response_count: usize,
    /// How many of them the author marked `important`.
    pub important_response_count: usize,
    /// Whether the message is flagged urgent.
    pub is_urgent: bool,
    /// Whether the message has already been read.
    pub is_read: bool,
    /// Whether the message's sender has gone away (`is_orphaned`).
    pub is_orphaned: bool,
    /// Whether the message's sender is currently in comms range.
    pub sender_in_range: bool,
    /// Ship state, folded into the same flat fact set because
    /// [`crate::ai::policy::AiPolicy::resolve_channel`] evaluates over ONE
    /// `AiFacts` (unlike the selector's three contexts).
    pub red_alert: bool,
    /// Whether the ship's own Comms fine system is available (AC2).
    pub comms_available: bool,
    /// Authored ship power rating; absent when the ship declares none.
    pub power_rating: Option<f32>,
}

/// Seed the facts for one Comms dialogue-response decision (issue #786).
///
/// Pure and Bevy-free (AGENTS.md rule #10). Same #779 empty-facts discipline as
/// [`seed_comms_hail_facts`]: every fact an authored `when` guard could name is
/// seeded, so `fact(is_urgent)`, `fact(response_count)`, … all read real values.
pub fn seed_comms_response_facts(reading: &CommsResponseReading) -> crate::world::flags::AiFacts {
    let mut facts = crate::world::flags::AiFacts::new();
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    facts.set("response_count", reading.response_count as f64);
    facts.set(
        "available_response_count",
        reading.available_response_count as f64,
    );
    facts.set(
        "important_response_count",
        reading.important_response_count as f64,
    );
    facts.set("is_urgent", b(reading.is_urgent));
    facts.set("is_read", b(reading.is_read));
    facts.set("is_orphaned", b(reading.is_orphaned));
    facts.set("sender_in_range", b(reading.sender_in_range));
    facts.set("red_alert", b(reading.red_alert));
    facts.set("comms_available", b(reading.comms_available));
    if let Some(pr) = reading.power_rating {
        facts.set("power_rating", pr as f64);
    }
    facts
}

/// Whether the ship's own Comms fine system is usable this tick (AC2).
///
/// Reads the authoritative per-system hull: Disabled and Destroyed count as
/// unavailable, Operational and Damaged as available. Ships with no hull
/// tracker (bare-`App` fixtures, entities that never took damage modelling) are
/// treated as available, matching the other AI hosts' hull fallbacks.
fn comms_system_available(hull: Option<&crate::entity_spawner::EntitySystemHull>) -> bool {
    let Some(hull) = hull else {
        return true;
    };
    !matches!(
        hull.0.tier_for(&crate::system_registry::comms_system_id()),
        crate::ship::damage::DamageTier::Disabled | crate::ship::damage::DamageTier::Destroyed
    )
}

/// Whether THIS ship has an outstanding hail to `target_uuid` (issue #786, AC5
/// anti-respam). The canonical eligibility gate.
///
/// This REPLACES the retired `CommsAiHailState.last_hailed` AI memory with an
/// authoritative reading: [`handle_hail`] records every hail that passes the
/// server-side range gate into `CommsRuntime.open_hails`, for human officer and
/// Backfill AI alike. Nothing here is AI-owned or PER-AI: it is one shared set
/// on `CommsRuntime`, written by the same handler for both actors, so a human's
/// hail suppresses the AI's and a control flip needs no reset. (Its retirement
/// has an AI-operated arm — see below — but the record itself is shared state,
/// and that arm is inert while a human holds the console.)
///
/// # Why this is NOT derived from the inbox
///
/// The obvious derivation — "an un-orphaned inbox message or open dialogue from
/// that sender exists" — does not TERMINATE, and it false-positives:
///
///   * A hail to a target with no matching `on_hailed` template, or whose
///     template has already `fired` (`evaluate_comms_templates` latches
///     `state.fired` permanently, so a template fires AT MOST ONCE EVER), seats
///     no message and no dialogue. The inbox-derived guard therefore never
///     arms, and a standing `Hail` directive re-emits EVERY TICK for the life of
///     the objective — re-pushing `WorldEvent::Hailed`, which a `repeat = true`
///     `on_hailed` trigger carrying `increment_world_flag` / `add_objective`
///     would act on without bound. Post-`ClearComms` this is unconditional: the
///     template is already spent, so the guard can never re-arm.
///   * ANY inbound message from that contact satisfied it, regardless of
///     provenance. `assets/worlds/before_the_fire.toml`'s `on_world_loaded`
///     message from Axiom Station would have permanently suppressed a
///     legitimate `Hail` directive naming Axiom Station.
///
/// # How it re-arms
///
/// `open_hails` is a latch, so while the directive stands and the target is in
/// range the contact is hailed exactly once — a target whose dialogue was fully
/// played out but whose message still sits in the inbox is not re-hailed. Three
/// things retire an entry, and none of them is inbox-shaped:
///
///   * [`handle_clear_comms`] — the human Comms officer emptying the slate.
///     There is no scripted path to this: `ClearComms` is a console action only
///     (`gui/comms-state.js`, `gui/action-map.js`), and `TriggerAction` has no
///     `ClearComms` variant. An unmanned ship therefore never sees one.
///   * [`operate_comms_ai`]'s per-tick candidacy retirement — the entry is
///     dropped once the target stops being a live hail candidate (its directive
///     completed or was removed, or it left comms range). This is the re-arm
///     the retired `last_hailed` had, and it is what an unmanned ship actually
///     relies on: without it a second, later `Hail` directive naming an
///     already-hailed contact would be dropped forever. It still terminates —
///     a standing directive's target stays a candidate every tick, so the latch
///     holds for exactly the looping case.
///   * [`crate::comms::server::update_comms_range_flags`] — the target's entity
///     despawned, so the record is pruned with its contact and range flag.
///     Without this the set grows monotonically and a
///     LoadWorld → UnloadWorld → LoadWorld cycle re-registering the same
///     authored UUID would leave that contact permanently un-hailable.
///
/// The alternative (re-arming on inbox emptiness) reintroduces the unterminating
/// loop above, which is strictly worse.
fn has_open_hail_thread_with(target_uuid: &str, comms: Option<&CommsRuntime>) -> bool {
    comms.is_some_and(|rt| rt.open_hails.contains(target_uuid))
}

/// Whether `sender_uuid` has any live inbound traffic in the officer's inbox —
/// an un-orphaned message, or an open `ActiveDialogue` awaiting an answer.
///
/// Seeded as `candidate_fact(has_unread_from_sender)` for authors who genuinely
/// want "don't hail someone who is already talking to us". It says nothing
/// about who opened the channel, so it is NOT the anti-respam gate; see
/// [`has_open_hail_thread_with`].
fn has_unread_from_sender_with(
    sender_uuid: &str,
    comms: Option<&CommsRuntime>,
    inbox: Option<&CommsInboxRes>,
) -> bool {
    let Some(inbox) = inbox else {
        return false;
    };
    if inbox
        .0
        .messages()
        .iter()
        .any(|m| m.sender_uuid == sender_uuid && !m.is_orphaned)
    {
        return true;
    }
    comms.is_some_and(|rt| {
        rt.active_dialogues
            .keys()
            .filter_map(|id| inbox.0.sender_uuid_for(id))
            .any(|s| s == sender_uuid)
    })
}

/// Backfill Comms AI: rank and issue hails through the AUTHORED selector
/// (issue #753, converted to a data-driven policy by issue #786).
///
/// Comms is a player channel — the inbox, contacts, range flags, and objective
/// pool are player-session-scoped singletons published onto the `LocalShip`
/// (`publish_comms_blackboard`). This operator therefore runs only for the
/// `LocalShip` and only when its Comms system is `ControlSource::Ai` (the
/// unmanned-player-ship Backfill case).
///
/// # Authored ranking (AC1)
///
/// The retired hardcoded `filter(...).find_map(...)` argmax is GONE. The scored
/// objective pool's positive Comms-relevant `Hail` directives become
/// `hail-objectives` candidates keyed on the REAL contact UUID each directive's
/// authored name resolves to, the comms roster becomes `comms-contacts`
/// candidates that ENRICH them, and the authored
/// [`crate::ai::selector::TargetSelector`] filters and ranks the union. The
/// winning uuid goes straight into `SystemControlPayload::Hail { target_uuid }`
/// — no side-table, because selector identity and hail identity are the same
/// thing here.
///
/// `current` is `None` and the canonical `switch_margin` is 0: a hail is a
/// ONE-SHOT event, not a retained target, so there is nothing for hysteresis to
/// hold on to.
///
/// # AC2 — three eligibility gates, all authored, all from authoritative state
///
///   1. AUTHORITATIVE CONTACTS — a directive's target name must resolve through
///      [`resolve_hail_target`] (world `name_to_uuid`, then the comms contact
///      roster, then a literal UUID). An unresolvable name produces no candidate
///      at all, exactly as before.
///   2. RANGE — seeded as `candidate_fact(in_range)` from
///      `CommsRuntime.range_flags`, honouring `range_active` the same way
///      `current_sender_in_range` does (while range tracking is inactive
///      everything is in range). `handle_hail` KEEPS its own hard server-side
///      range gate: this is defence in depth, not a relocation.
///   3. SYSTEM AVAILABILITY — seeded as `self_fact(comms_available)` from
///      `EntitySystemHull` and NAMED in the canonical eligibility, alongside the
///      `operate_ai` control gate: a Disabled or Destroyed Comms system produces
///      no eligible candidate at all, so the ship stops hailing. The response
///      policy carries the mirror term (`fact(comms_available) > 0`), so a
///      destroyed Comms system silences both halves.
///
/// A fourth gate, `candidate_fact(has_open_hail_thread) < 1`, is the AC5
/// anti-respam latch — see [`has_open_hail_thread_with`] for why it reads the
/// authoritative `CommsRuntime.open_hails` record rather than the inbox, and
/// what the latch costs.
///
/// # AC5 — the latch RE-ARMS when the target stops being a candidate
///
/// Before candidates are seeded, this host retires every `open_hails` entry
/// whose UUID is not a live hail candidate THIS tick — no positive Comms `Hail`
/// directive names it any more, or it has left comms range. That is what the
/// retired `CommsAiHailState.last_hailed` did (it was reset whenever no target
/// was selectable), and dropping it would have been a regression: a second,
/// later `Hail` directive naming an already-hailed contact would be dropped
/// forever on an unmanned ship, which has no `ClearComms` path at all
/// (`ClearComms` is a human console action only — no `TriggerAction` emits it).
///
/// It still TERMINATES, and that is the whole reason the retirement is keyed on
/// candidacy rather than on the inbox: a STANDING directive's target is a
/// candidate on every tick of its life, so the latch is never retired for
/// exactly the case that loops. Retirement needs an externally-driven change —
/// the objective completing or being removed, or the target crossing the comms
/// range boundary — neither of which the hail itself can cause.
///
/// The retirement runs only on the AI-operated path (below the `operate_ai`
/// gate), so a human officer's open channels are never quietly closed under
/// them; their record is retired by their own `ClearComms`, as before.
/// [`crate::comms::server::update_comms_range_flags`] separately prunes entries
/// whose entity has despawned.
///
/// # AC4 — the scenario flag chain is READ-ONLY, structurally
///
/// `WorldContentRuntime` is taken as a shared `Option<Res<_>>` (never `ResMut`)
/// and only its `flags` store is handed to `select` as `&[&FlagStore]`. Every
/// `FlagStore` mutator takes `&mut self`, so no authored predicate can reach one
/// through a shared reference — the read-only guarantee is a type-level fact,
/// not a convention. Authored `eligibility`/`score` guards may therefore gate on
/// scenario flags and counters freely. Legitimate mutation of scenario state
/// stays where it belongs: in the consequence router
/// ([`handle_respond_to_message`]'s `dispatch_action` pass).
///
/// # AC5 — human exclusivity and lifecycle reset
///
/// The `operate_ai` gate below stands the AI down the moment a human takes the
/// Comms console, and admission independently refuses an `ai:` emission on a
/// human-held system. Deleting `CommsAiHailState` removed the last piece of
/// AI-OWNED comms memory: every decision is a pure function of this tick's
/// authoritative snapshot, and there is nothing per-AI to reset on a control
/// flip. The anti-respam latch that replaced it (`CommsRuntime.open_hails`) is
/// shared comms state written by `handle_hail` for HUMAN and AI hails alike, so
/// it does not need resetting either — a human officer inherits, and adds to,
/// the same record.
///
/// `CommsRuntime` is taken as `ResMut` ONLY for the candidacy retirement above.
/// Ordering is explicit and total: this system is `.before(handle_hail)`, which
/// the comms plugin chains before `handle_respond_to_message` and
/// `handle_clear_comms`, and [`operate_comms_response_ai`] is `.after` this one.
#[allow(clippy::too_many_arguments)]
pub fn operate_comms_ai(
    objectives: Option<Res<ObjectiveManagerRes>>,
    // Read-only scenario flag/counter store (AC4). `Option<Res<_>>` so bare-`App`
    // fixtures that never insert it still pass parameter validation; absent, the
    // flag chain is empty and flag guards read false.
    runtime: Option<Res<WorldContentRuntime>>,
    // Loaded sub-world layers (issue #891 stage 2): the chain is anchored at
    // the layer that spawned each ship, `parent:`-walkable to the base store.
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`.
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    // `ResMut` for ONE reason: retiring the `open_hails` latch for targets that
    // stopped being candidates (the AC5 re-arm). Every other comms read here is
    // a plain read.
    mut comms: Option<ResMut<CommsRuntime>>,
    inbox: Option<Res<CommsInboxRes>>,
    sessions: Res<crate::lobby::Sessions>,
    // `Option<Res<_>>`, never bare — bare-`App` fixtures never insert it.
    log: Option<Res<crate::logging::LogFilterConfig>>,
    mut ships: Query<
        (
            Entity,
            Option<&EntityUuid>,
            &ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&CommsTargetSelector>,
        ),
        With<crate::server_app::LocalShip>,
    >,
) {
    /// One resolved `hail-objectives` candidacy for this tick: the contact UUID
    /// an active `Hail` directive names, plus the readings the latch retirement
    /// and the fact seed both need.
    struct DirectiveHit {
        uuid: String,
        score: f32,
        mandatory: bool,
        in_range: bool,
    }

    for (entity, entity_uuid, sources, ship_config, mut admitted, red_alert, hull, selector_comp) in
        ships.iter_mut()
    {
        let policy = sources
            .0
            .policy_for(&crate::system_registry::comms_system_id());
        if !policy.operate_ai {
            continue;
        }
        // No authored `[comms_console.selector]` ⇒ no component ⇒ no hail
        // ranking. Since #885b stage 5d there is no synthesised stand-in: a
        // system nobody declared is not handed automation (PRD #774 US7).
        let Some(selector_comp) = selector_comp else {
            continue;
        };

        // The read-only scenario flag chain (AC4), anchored at the layer that
        // spawned THIS ship (issue #891 stage 2) — correctly layered, so
        // `parent:` prefixes climb toward the base store.
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_q.get(entity).ok(),
            runtime.as_deref(),
            layers.as_deref(),
        );

        // Score the objective pool against the same conditions
        // `publish_comms_blackboard` uses (red alert + hull fraction).
        let red_alert = red_alert.map(|r| r.0).unwrap_or(false);
        let hull_fraction = hull
            .map(|h| {
                let max = h.0.total_max();
                if max > 0.0 {
                    (h.0.total_current() / max).clamp(0.0, 1.0)
                } else {
                    1.0
                }
            })
            .unwrap_or(1.0);
        let conditions = crate::objectives::WorldConditions {
            red_alert,
            hull_fraction,
            attacked: false,
        };

        // ── Pass 1: this tick's DIRECTIVE candidacy ──────────────────────────
        // Source `hail-objectives`: every positive, Comms-relevant Hail directive
        // whose authored target name resolves to a real contact UUID. Built in
        // `scored_pool` order (descending score, a total order via `total_cmp`),
        // so candidate order is deterministic; the selector's smallest-UUID rule
        // breaks any residual score-band tie.
        //
        // Resolved BEFORE the latch retirement below, because candidacy is what
        // the retirement is keyed on — and then reused verbatim to seed the
        // candidates, so the two cannot drift apart.
        let mut hits: Vec<DirectiveHit> = Vec::new();
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        if let Some(mgr) = objectives.as_ref() {
            for scored in mgr.0.scored_pool(&conditions) {
                if !scored
                    .relevance
                    .contains(&crate::messages::SystemAffinity::Comms)
                {
                    continue;
                }
                let crate::messages::AiDirective::Hail { target } = &scored.directive else {
                    continue;
                };
                if scored.score <= 0.0 {
                    // `scored_pool` deliberately returns zero-gated objectives
                    // with `score = 0.0`, and the eligibility term
                    // `candidate_fact(objective_score) > 0` refuses to hail on
                    // them. Dropping them here keeps candidacy — and therefore
                    // the latch retirement below — keyed on POSITIVE directives
                    // only, so a directive that gates off and later goes
                    // positive again re-arms (as the retired `last_hailed`
                    // did). Termination is unaffected: a standing positive
                    // directive's target is still a candidate every tick.
                    continue;
                }
                let Some(uuid) = resolve_hail_target(target, runtime.as_deref(), comms.as_deref())
                else {
                    // Unresolvable name: no candidate, so no hail (unchanged).
                    continue;
                };
                if !seen.insert(uuid.clone()) {
                    // Two directives naming the same contact: the first (highest
                    // scoring) wins, matching the retired `find_map` fall-through.
                    continue;
                }
                // Range reading, honouring `range_active` exactly as
                // `current_sender_in_range` does: while range tracking is
                // inactive (lobby, pure-handler fixtures) every target counts as
                // in range.
                let in_range = match comms.as_deref() {
                    Some(rt) if rt.range_active => {
                        rt.range_flags.get(&uuid).copied().unwrap_or(false)
                    }
                    _ => true,
                };
                hits.push(DirectiveHit {
                    uuid,
                    score: scored.score,
                    mandatory: scored.snapshot.mandatory,
                    in_range,
                });
            }
        }

        // ── AC5 re-arm: retire the latch for non-candidates ──────────────────
        // An `open_hails` entry survives only while its target is still a LIVE
        // hail candidate — named by a positive Comms `Hail` directive and in
        // comms range. This restores the re-arm the retired
        // `CommsAiHailState.last_hailed` had (it reset whenever no target was
        // selectable) without reintroducing the loop, because a STANDING
        // directive's target is a candidate on every tick of its life: the latch
        // is never retired for exactly the case that would loop. Retirement
        // needs an externally-driven change — the objective completing or being
        // removed, or the target crossing the range boundary.
        //
        // Below the `operate_ai` gate on purpose: a human officer's open
        // channels are theirs to close with `ClearComms`.
        if let Some(rt) = comms.as_mut() {
            rt.open_hails
                .retain(|uuid| hits.iter().any(|h| &h.uuid == uuid && h.in_range));
        }

        // ── Pass 2: candidate seeding ────────────────────────────────────────
        // Reads the POST-retirement `open_hails`, so `has_open_hail_thread`
        // reflects this tick's latch, not last tick's.
        let in_range = |uuid: &str| match comms.as_deref() {
            Some(rt) if rt.range_active => rt.range_flags.get(uuid).copied().unwrap_or(false),
            _ => true,
        };
        let contact_for = |uuid: &str| {
            comms
                .as_deref()
                .and_then(|rt| rt.contacts.iter().find(|c| c.uuid == uuid))
        };
        let mut candidates: Vec<crate::ai::selector::SelectorCandidate> = Vec::new();
        for hit in &hits {
            let contact = contact_for(&hit.uuid);
            let reading = CommsHailCandidateReading {
                source_hail_objective: true,
                source_comms_contact: contact.is_some(),
                objective_score: hit.score,
                in_range: hit.in_range,
                is_urgent: contact.is_some_and(|c| c.is_urgent),
                has_open_hail_thread: has_open_hail_thread_with(&hit.uuid, comms.as_deref()),
                has_unread_from_sender: has_unread_from_sender_with(
                    &hit.uuid,
                    comms.as_deref(),
                    inbox.as_deref(),
                ),
                mandatory: hit.mandatory,
            };
            candidates.push(crate::ai::selector::SelectorCandidate {
                uuid: hit.uuid.clone(),
                position: [0.0, 0.0, 0.0],
                facts: seed_comms_hail_facts(&reading),
            });
        }
        // Source `comms-contacts`: the authoritative hailable roster. These carry
        // NO `source_hail_objective` marker, so under the canonical eligibility
        // they never independently hail — they merge their live readings into a
        // coincident directive candidate (the selector dedups by UUID, folding
        // facts) and stand ready for an author to widen eligibility onto them.
        // (The #778 `chart-contacts` pattern.)
        if let Some(rt) = comms.as_deref() {
            for contact in &rt.contacts {
                if seen.contains(&contact.uuid) {
                    continue;
                }
                let reading = CommsHailCandidateReading {
                    source_hail_objective: false,
                    source_comms_contact: true,
                    objective_score: 0.0,
                    in_range: in_range(&contact.uuid),
                    is_urgent: contact.is_urgent,
                    has_open_hail_thread: has_open_hail_thread_with(
                        &contact.uuid,
                        comms.as_deref(),
                    ),
                    has_unread_from_sender: has_unread_from_sender_with(
                        &contact.uuid,
                        comms.as_deref(),
                        inbox.as_deref(),
                    ),
                    mandatory: false,
                };
                candidates.push(crate::ai::selector::SelectorCandidate {
                    uuid: contact.uuid.clone(),
                    position: [0.0, 0.0, 0.0],
                    facts: seed_comms_hail_facts(&reading),
                });
            }
        }

        // SELF context. Hail candidates are not spatial destinations — comms
        // reach is the authored `[comms].range` radius, already resolved into the
        // `in_range` fact — so every candidate sits at the ship's own origin and
        // the planar horizon never double-gates what range already decided.
        let self_ctx = crate::ai::selector::SelfContext {
            position: [0.0, 0.0, 0.0],
            facts: seed_comms_self_facts(
                selector_comp.power_rating,
                comms_system_available(hull),
                red_alert,
                comms.as_deref().map(|rt| rt.contacts.len()).unwrap_or(0),
            ),
        };

        // `current: None` — a hail is a one-shot event, not a retained target.
        let Some(target_uuid) =
            selector_comp
                .selector
                .select(&self_ctx, &candidates, None, &flag_chain)
        else {
            continue;
        };

        let admitted_ok = crate::command_admission::ai_emit::emit_ai_command(
            entity_uuid,
            crate::system_registry::comms_system_id(),
            crate::messages::SystemControlPayload::Hail {
                target_uuid: target_uuid.clone(),
            },
            sources,
            &sessions,
            ship_config,
            &mut admitted,
        );

        if admitted_ok {
            crate::pdebug!(
                log,
                crate::logging::LogCat::Comms,
                entity = entity,
                "backfill comms AI hailed {target_uuid}"
            );
        } else {
            // Refused at admission: the comms system's `ai:` token was not
            // admitted despite `operate_ai` — the coarse gate above and the
            // admission gate are reading different control sources. Warn.
            crate::pwarn!(
                log,
                crate::logging::LogCat::Comms,
                entity = entity,
                "backfill comms AI hail for {target_uuid} refused at admission"
            );
        }
    }
}

/// Backfill Comms AI: answer open dialogues through the AUTHORED response
/// policy (issue #786, AC3).
///
/// Replaces the `record_response(&id, 0)` stub that used to sit inside
/// [`handle_comms_channel2`] — a direct authoritative-inbox write that bypassed
/// both admission and the consequence router (see that system's doc comment).
/// For each inbox message with an OPEN dialogue awaiting an answer, this
/// resolves the ship's `comms_respond` channel and, when a rule fires, emits an
/// ordinary admitted `SystemControlPayload::RespondToMessage { message_id,
/// response_index }` through the shared `emit_ai_command` seam.
///
/// Ordered in `SimSet::Input` `.before(handle_respond_to_message)`, so the
/// EXISTING router drains it the same tick and runs unchanged: the response's
/// `TriggerAction`s dispatch through `dispatch_action`/`apply_dispatch_result`,
/// `selected_response` is recorded by the ROUTER, and any `follow_up` advances —
/// exactly as for a human Comms officer. No branch on actor exists downstream of
/// admission (AGENTS.md rule #6).
///
/// # AC5 — no AI-side staleness check is needed
///
/// The shared router already rejects a stale dialogue (no `active_dialogues`
/// entry), an out-of-range sender, and an out-of-bounds index, emitting
/// `CommsResponseRejected` in each case. Those gates now apply to AI emissions
/// IDENTICALLY, because an AI response is just another admitted command — which
/// is precisely the point of routing it through admission. The only AI-side
/// filtering here is "is there an open dialogue awaiting an answer at all",
/// which selects what to decide about rather than re-implementing authority.
/// The policy itself is stateless, so there is nothing to reset when control
/// flips between human and AI.
///
/// # AC4 — read-only scenario state
///
/// Same structural guarantee as [`operate_comms_ai`]: `WorldContentRuntime` is a
/// shared `Option<Res<_>>` and its flag store reaches `resolve_channel` only as
/// `&[&FlagStore]`, whose mutators all require `&mut self`.
#[allow(clippy::too_many_arguments)]
pub fn operate_comms_response_ai(
    comms: Option<Res<CommsRuntime>>,
    inbox: Option<Res<CommsInboxRes>>,
    // Read-only scenario flag/counter store (AC4).
    runtime: Option<Res<WorldContentRuntime>>,
    // Loaded sub-world layers (issue #891 stage 2).
    layers: Option<Res<crate::world::server::WorldLayerMap>>,
    // The per-ship origin-layer stamp (issue #891 review finding 1): an O(1)
    // read replacing the old `WorldLayerMap` scan inside `entity_flag_chain`.
    origin_q: Query<&crate::world::server::EntityOriginLayer>,
    sessions: Res<crate::lobby::Sessions>,
    log: Option<Res<crate::logging::LogFilterConfig>>,
    // The shared AI base cadence's raw tick + interval (issue #889's
    // evaluate_every_ticks, wired at runtime). `Option<Res<_>>` for the usual
    // bare-`App` reason: several fixtures below register this system directly
    // without `register_ai_cadence`, so these read the same (0, 1) fallback
    // `evaluate_every_ticks_ready` already treats as "always due" — identical
    // to this system's pre-existing (ungated w.r.t. per-host cadence)
    // behaviour in every such fixture.
    tick: Option<Res<crate::sim_tick::SimTick>>,
    base_interval: Option<Res<crate::ai::cadence::AiBaseInterval>>,
    mut ships: Query<
        (
            Entity,
            Option<&EntityUuid>,
            &ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &mut crate::messages::AdmittedCommands,
            Option<&crate::ship_state::ShipRedAlert>,
            Option<&crate::entity_spawner::EntitySystemHull>,
            Option<&CommsResponseAiPolicy>,
            Option<&CommsResponseAiCadence>,
            // The authored ship `power_rating` lives on the CO-LOCATED selector
            // component (both are inserted on the same entity at spawn), so it
            // is read from there rather than left permanently absent — the #779
            // empty-facts lesson: a fact an authored guard can name must carry a
            // real value in production, or `fact(power_rating) > 3` silently
            // never fires.
            Option<&CommsTargetSelector>,
        ),
        With<crate::server_app::LocalShip>,
    >,
) {
    let (Some(comms), Some(inbox)) = (comms.as_deref(), inbox.as_deref()) else {
        return;
    };
    let tick = tick.map(|t| t.0).unwrap_or(0);
    let base_interval = base_interval.map(|b| b.0).unwrap_or(1);

    for (
        entity,
        entity_uuid,
        sources,
        ship_config,
        mut admitted,
        red_alert,
        hull,
        policy_comp,
        cadence_comp,
        selector_comp,
    ) in ships.iter_mut()
    {
        let control = sources
            .0
            .policy_for(&crate::system_registry::comms_system_id());
        if !control.operate_ai {
            // AC5 human exclusivity: a human Comms officer answers their own
            // dialogues. Stateless, so nothing to reset.
            continue;
        }
        // No authored `[comms_console.ai]` ⇒ no component ⇒ the ship answers
        // nothing. There is no synthesised stand-in since #885b stage 5d.
        let Some(policy_comp) = policy_comp else {
            continue;
        };
        let policy = &policy_comp.0;
        // Per-host multiplier on the shared base cadence (issue #889's
        // evaluate_every_ticks, wired at runtime): a ship whose
        // `[comms_console.ai]` authors `evaluate_every_ticks = n` decides on
        // every Nth arm of `ai_tick_ready`, not every arm. `1` (every shipped
        // hull today) reduces this to a no-op.
        let evaluate_every_ticks = cadence_comp.map(|c| c.0).unwrap_or(1);
        if !crate::ai::cadence::evaluate_every_ticks_ready(
            tick,
            base_interval,
            evaluate_every_ticks,
        ) {
            continue;
        }
        // The read-only scenario flag chain (AC4), anchored at the layer that
        // spawned THIS ship (issue #891 stage 2).
        let flag_chain = crate::world::server::entity_flag_chain(
            origin_q.get(entity).ok(),
            runtime.as_deref(),
            layers.as_deref(),
        );
        let red_alert = red_alert.map(|r| r.0).unwrap_or(false);
        let comms_available = comms_system_available(hull);
        let power_rating = selector_comp.and_then(|s| s.power_rating);

        // Inbox order is insertion order, so the decision sequence is
        // deterministic on the fixed tick.
        for message in inbox.0.messages() {
            // Only messages with an OPEN dialogue awaiting an answer are
            // decided about. `selected_response.is_some()` means it was already
            // answered; no `active_dialogues` entry means the thread is stale
            // (the router would reject it anyway).
            if message.selected_response.is_some() {
                continue;
            }
            let Some(dialogue) = comms.active_dialogues.get(&message.id) else {
                continue;
            };
            let responses = &dialogue.current_node.responses;
            if responses.is_empty() {
                continue;
            }

            let sender_in_range = current_sender_in_range(comms, &message.sender_uuid);
            let reading = CommsResponseReading {
                response_count: responses.len(),
                available_response_count: if sender_in_range { responses.len() } else { 0 },
                important_response_count: responses.iter().filter(|r| r.important).count(),
                is_urgent: message.is_urgent,
                is_read: message.is_read,
                is_orphaned: message.is_orphaned,
                sender_in_range,
                red_alert,
                comms_available,
                power_rating,
            };
            let facts = seed_comms_response_facts(&reading);

            // Only the response verb ever resolves on this channel (the policy is
            // validated to carry no other), so this `if let` is exhaustive in
            // practice; anything else holds (no emit).
            let Some(crate::ai::policy::AiPolicyVerb::RespondToMessage(index)) = policy
                .resolve_channel(
                    crate::entities::config::COMMS_RESPOND_CHANNEL,
                    &facts,
                    &flag_chain,
                )
            else {
                // No rule fired: hold — leave the dialogue open this tick.
                continue;
            };
            let index = *index as usize;
            if index >= responses.len() {
                // An authored index the current node cannot honour. The router
                // would reject it and flash the Comms officer's panel red, so
                // skip rather than manufacture a rejection storm; the router's
                // bounds check remains the authority for anything that does get
                // through.
                crate::pwarn!(
                    log,
                    crate::logging::LogCat::Comms,
                    entity = entity,
                    "comms AI response index {index} out of bounds for message {} ({} responses)",
                    message.id,
                    responses.len()
                );
                continue;
            }

            let admitted_ok = crate::command_admission::ai_emit::emit_ai_command(
                entity_uuid,
                crate::system_registry::comms_system_id(),
                crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: message.id.clone(),
                    response_index: index,
                },
                sources,
                &sessions,
                ship_config,
                &mut admitted,
            );
            if admitted_ok {
                crate::pdebug!(
                    log,
                    crate::logging::LogCat::Comms,
                    entity = entity,
                    "backfill comms AI answered {} with response {index}",
                    message.id
                );
            } else {
                crate::pwarn!(
                    log,
                    crate::logging::LogCat::Comms,
                    entity = entity,
                    "backfill comms AI response to {} refused at admission",
                    message.id
                );
            }
        }
    }
}

/// Resolve a Hail directive's target NAME to an entity UUID.
///
/// `AiDirective::Hail` carries an authored entity NAME while
/// `SystemControlPayload::Hail` needs a UUID. Resolution order: the world's
/// `name_to_uuid` map (authoritative, matches `ship::sensors`' Destroy-target
/// resolution), then a comms contact whose display name matches, then the
/// target string itself if it is already a valid UUID (authors may target by
/// UUID directly). Returns `None` when the name cannot be resolved — the
/// caller then issues no hail (AC: "no action for unresolvable directives").
fn resolve_hail_target(
    target: &str,
    runtime: Option<&WorldContentRuntime>,
    comms: Option<&CommsRuntime>,
) -> Option<String> {
    if let Some(uuid) = runtime.and_then(|rt| rt.name_to_uuid.get(target).cloned()) {
        return Some(uuid);
    }
    if let Some(uuid) = comms.and_then(|c| {
        c.contacts
            .iter()
            .find(|contact| contact.name == target)
            .map(|contact| contact.uuid.clone())
    }) {
        return Some(uuid);
    }
    if uuid::Uuid::parse_str(target).is_ok() {
        return Some(target.to_string());
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::comms::server::CommsInboxRes;
    use crate::messages::CommsMessage;
    use crate::server_app::{LocalShip, Ship, ShipSystemBlackboards};

    fn msg(id: &str) -> CommsMessage {
        CommsMessage {
            id: id.into(),
            sender_uuid: "sender-uuid".into(),
            sender_name: "Station Alpha".into(),
            subject: "Test".into(),
            body: "Body text".into(),
            responses: vec![crate::messages::CommsResponseView {
                text: "OK".into(),
                important: false,
                available: true,
            }],
            selected_response: None,
            is_read: false,
            is_orphaned: false,
            sender_in_range: true,
            thread_id: id.into(),
            is_urgent: false,
        }
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.insert_resource(CommsInboxRes(crate::console::comms::CommsInbox::new()))
            .insert_resource(CommsRuntime::default())
            .add_systems(Update, publish_comms_blackboard);
        // Spawn a LocalShip entity so the query in publish_comms_blackboard
        // resolves. The `Ship` marker is required now that the publish iterates
        // `With<Ship>` per-entity (issue #831).
        app.world_mut()
            .spawn((Ship, LocalShip, ShipSystemBlackboards::default()));
        app
    }

    fn comms_bb(app: &mut App) -> CommsBlackboard {
        let mut q = app
            .world_mut()
            .query_filtered::<&ShipSystemBlackboards, With<LocalShip>>();
        let bbs = q
            .single(app.world())
            .expect("no LocalShip with ShipSystemBlackboards");
        let key = SystemId(crate::system_registry::COMMS_SYSTEM_ID.to_string());
        let SystemBlackboard::Comms(bb) =
            bbs.0.get(&key).expect("comms blackboard missing").clone()
        else {
            panic!("wrong blackboard variant");
        };
        bb
    }

    // ── comms objective visibility (issue #752, objective-visibility-policy) ──

    #[test]
    fn comms_bb_hides_zero_score_doctrine_objective() {
        use crate::messages::{AiDirective, ObjectiveSource};
        use crate::objectives::{ObjectiveManager, UtilityConfig, ZeroGateCondition};
        let mut app = test_app();
        let mut mgr = ObjectiveManager::new();
        // Doctrine objective gated on red_alert — score 0 while not at red alert
        // (the test LocalShip has no ShipRedAlert, so conditions.red_alert=false).
        mgr.add_full(
            "doctrine-hidden",
            "Hidden doctrine",
            false,
            vec![],
            AiDirective::None,
            UtilityConfig {
                base_priority: 30.0,
                zero_gates: vec![ZeroGateCondition {
                    condition: "red_alert".into(),
                    threshold: None,
                }],
                ..Default::default()
            },
            ObjectiveSource::Doctrine,
        );
        app.world_mut().insert_resource(ObjectiveManagerRes(mgr));
        app.update();

        assert!(
            comms_bb(&mut app).objectives.is_empty(),
            "a zero-score doctrine objective must be hidden from the comms panel"
        );
    }

    #[test]
    fn comms_bb_shows_mission_objective_regardless_of_score() {
        use crate::objectives::ObjectiveManager;
        let mut app = test_app();
        let mut mgr = ObjectiveManager::new();
        mgr.add("mission-1", "Reach the station", true, vec![]);
        app.world_mut().insert_resource(ObjectiveManagerRes(mgr));
        app.update();

        let bb = comms_bb(&mut app);
        assert_eq!(bb.objectives.len(), 1);
        assert_eq!(bb.objectives[0].id, "mission-1");
    }

    #[test]
    fn blackboard_reflects_inbox_messages() {
        let mut app = test_app();
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(msg("m1"));
        app.update();

        let bb = comms_bb(&mut app);
        assert_eq!(bb.messages.len(), 1);
        assert_eq!(bb.messages[0].id, "m1");
    }

    #[test]
    fn npc_ship_gets_empty_comms_blackboard() {
        // AC #831: NPC ships have comms blackboards. Comms is a player channel,
        // so the local ship carries the shared inbox content while an NPC ship
        // gets an entry that is present but empty.
        let mut app = test_app();
        let npc = app
            .world_mut()
            .spawn((Ship, ShipSystemBlackboards::default()))
            .id();
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(msg("m1"));
        app.update();

        let key = SystemId(crate::system_registry::COMMS_SYSTEM_ID.to_string());
        let npc_bbs = app
            .world()
            .entity(npc)
            .get::<ShipSystemBlackboards>()
            .unwrap();
        let SystemBlackboard::Comms(npc_bb) = npc_bbs
            .0
            .get(&key)
            .expect("NPC ship must have a comms blackboard entry")
            .clone()
        else {
            panic!("wrong blackboard variant");
        };
        assert!(
            npc_bb.messages.is_empty(),
            "an NPC ship's comms blackboard must carry no player messages"
        );

        // The local ship still gets the shared player-channel content.
        let local_bb = comms_bb(&mut app);
        assert_eq!(local_bb.messages.len(), 1);
    }

    /// Verifies operate_comms_ai runs per-entity for AI-controlled ships (issue #593 AC).
    #[test]
    fn operate_comms_ai_per_entity_ai_gate() {
        use crate::ship::control_source::{ControlSource, ControlSourceResolver};
        use crate::ship_plugin::ShipSystemControlSources;

        let mut ai_resolver = ControlSourceResolver::new();
        ai_resolver.set(crate::system_registry::comms_system_id(), ControlSource::Ai);
        let ai_sources = ShipSystemControlSources(ai_resolver);
        let ai_policy = ai_sources
            .0
            .policy_for(&crate::system_registry::comms_system_id());
        assert!(
            ai_policy.operate_ai,
            "AI Comms must gate through operate_ai"
        );

        let mut human_resolver = ControlSourceResolver::new();
        human_resolver.set(
            crate::system_registry::comms_system_id(),
            ControlSource::Human,
        );
        let human_sources = ShipSystemControlSources(human_resolver);
        let human_policy = human_sources
            .0
            .policy_for(&crate::system_registry::comms_system_id());
        assert!(!human_policy.operate_ai, "Human Comms must not operate AI");
    }

    // ── Backfill Comms AI hail execution (issue #753) ──────────────────────

    use crate::messages::{AdmittedCommands, AiDirective, ObjectiveSource, SystemControlPayload};
    use crate::objectives::{ObjectiveManager, UtilityConfig, ZeroGateCondition};
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};

    /// Minimal app that runs ONLY `operate_comms_ai` (no `handle_hail`, no
    /// AdmissionPlugin clear) so a test can inspect the `Hail` command the AI
    /// leaves in the ship's own `AdmittedCommands`. Spawns one `LocalShip`
    /// whose Comms system carries `comms_source`.
    fn comms_ai_app(comms_source: ControlSource) -> App {
        let mut app = App::new();
        app.insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .insert_resource(WorldContentRuntime::default())
        .insert_resource(CommsRuntime::default())
        // Issue #786: the anti-respam guard reads AUTHORITATIVE comms state
        // (`CommsRuntime.open_hails`, plus the inbox for the seeded-but-ungated
        // `has_unread_from_sender`), so the fixture carries a real inbox and a
        // real comms runtime instead of an AI memory component.
        .insert_resource(CommsInboxRes(crate::console::comms::CommsInbox::new()))
        .add_systems(Update, operate_comms_ai);

        let mut resolver = ControlSourceResolver::new();
        resolver.set(crate::system_registry::comms_system_id(), comms_source);
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            ShipSystemControlSources(resolver),
            crate::ship_plugin::ShipConfigComponent::default(),
            AdmittedCommands::default(),
            // The AUTHORED `[comms_console.selector]` block every shipped hull
            // carries. Since #885b stage 5d `operate_comms_ai` has no
            // synthesised fallback — a ship with no selector hails nobody.
            CommsTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail")
                    .to_selector()
                    .expect("the shipped Comms hail selector decodes"),
                power_rating: None,
            },
        ));
        app
    }

    /// Register `name → uuid` in the world runtime so a Hail directive naming
    /// `name` resolves to `uuid`.
    fn register_name(app: &mut App, name: &str, uuid: &str) {
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert(name.into(), uuid.into());
    }

    /// Insert an `ObjectiveManagerRes` carrying a single objective.
    fn set_objective(
        app: &mut App,
        id: &str,
        directive: AiDirective,
        utility: UtilityConfig,
        source: ObjectiveSource,
    ) {
        let mut mgr = ObjectiveManager::new();
        mgr.add_full(id, "text", false, vec![], directive, utility, source);
        app.world_mut().insert_resource(ObjectiveManagerRes(mgr));
    }

    /// Collect the `target_uuid`s of every `Hail` admitted to the Comms system
    /// on the (sole) `LocalShip`.
    fn admitted_hail_targets(app: &mut App) -> Vec<String> {
        let mut q = app
            .world_mut()
            .query_filtered::<&AdmittedCommands, With<crate::server_app::LocalShip>>();
        let admitted = q.single(app.world()).expect("LocalShip admitted commands");
        admitted
            .for_target(crate::system_registry::COMMS_SYSTEM_ID)
            .filter_map(|cmd| match &cmd.payload {
                SystemControlPayload::Hail { target_uuid } => Some(target_uuid.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn comms_ai_hails_from_relevant_hail_directive() {
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();

        assert_eq!(
            admitted_hail_targets(&mut app),
            vec!["station-alpha-uuid".to_string()],
            "a relevant, in-range Hail directive must produce a Hail attempt to the resolved UUID"
        );
    }

    #[test]
    fn comms_ai_emits_the_same_hail_payload_a_human_sends() {
        // AI/human symmetry (AGENTS.md #6): the AI emits the SAME typed
        // `SystemControlPayload::Hail` a human Comms officer's
        // `ControlSystem { target: comms, payload: Hail { .. } }` carries — no
        // bespoke AI payload. Assert the admitted command is byte-identical to
        // the human payload.
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();

        let human_payload = SystemControlPayload::Hail {
            target_uuid: "station-alpha-uuid".into(),
        };
        let mut q = app
            .world_mut()
            .query_filtered::<&AdmittedCommands, With<crate::server_app::LocalShip>>();
        let admitted = q.single(app.world()).unwrap();
        let ai_payloads: Vec<_> = admitted
            .for_target(crate::system_registry::COMMS_SYSTEM_ID)
            .map(|cmd| cmd.payload.clone())
            .collect();
        assert_eq!(
            ai_payloads,
            vec![human_payload],
            "AI-emitted comms payload must equal the payload a human ControlSystem sends"
        );
    }

    #[test]
    fn comms_ai_does_not_hail_when_human_operated() {
        // Gate: a human-held Comms console must refuse AI emission entirely.
        let mut app = comms_ai_app(ControlSource::Human);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();

        assert!(
            admitted_hail_targets(&mut app).is_empty(),
            "a human-operated Comms console must not emit an AI hail"
        );
    }

    #[test]
    fn comms_ai_does_not_hail_zero_score_directive() {
        // A doctrine Hail gated on red_alert scores 0 while not at red alert
        // (the test ship has no ShipRedAlert). No hail must occur.
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                zero_gates: vec![ZeroGateCondition {
                    condition: "red_alert".into(),
                    threshold: None,
                }],
                ..Default::default()
            },
            ObjectiveSource::Doctrine,
        );
        app.update();

        assert!(
            admitted_hail_targets(&mut app).is_empty(),
            "a zero-score Hail directive must produce no hail"
        );
    }

    #[test]
    fn comms_ai_does_not_hail_irrelevant_directive() {
        // A Destroy directive is Helm/Weapons-relevant, not Comms-relevant.
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        set_objective(
            &mut app,
            "destroy-alpha",
            AiDirective::Destroy {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();

        assert!(
            admitted_hail_targets(&mut app).is_empty(),
            "a non-Hail (Comms-irrelevant) directive must produce no hail"
        );
    }

    #[test]
    fn comms_ai_does_not_hail_out_of_range_target() {
        // Range tracking active and the target flagged out of range: mirror
        // handle_hail's server-side gate so no hail attempt is emitted.
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            comms.range_active = true;
            comms.range_flags.insert("station-alpha-uuid".into(), false);
        }
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();

        assert!(
            admitted_hail_targets(&mut app).is_empty(),
            "an out-of-range Hail target must produce no hail"
        );
    }

    #[test]
    fn comms_ai_does_not_hail_unresolvable_name() {
        // No name_to_uuid entry and the target is not itself a UUID.
        let mut app = comms_ai_app(ControlSource::Ai);
        set_objective(
            &mut app,
            "hail-ghost",
            AiDirective::Hail {
                target: "Ghost Station".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();

        assert!(
            admitted_hail_targets(&mut app).is_empty(),
            "an unresolvable Hail target name must produce no hail"
        );
    }

    #[test]
    fn comms_ai_hail_is_isolated_to_the_local_ship() {
        // Per-ship isolation: a second, non-local AI-comms ship must never gain
        // the hail — comms is a player channel scoped to the LocalShip.
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");

        let mut npc_resolver = ControlSourceResolver::new();
        npc_resolver.set(crate::system_registry::comms_system_id(), ControlSource::Ai);
        let npc = app
            .world_mut()
            .spawn((
                crate::server_app::Ship,
                ShipSystemControlSources(npc_resolver),
                crate::ship_plugin::ShipConfigComponent::default(),
                AdmittedCommands::default(),
            ))
            .id();

        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();

        assert_eq!(
            admitted_hail_targets(&mut app),
            vec!["station-alpha-uuid".to_string()],
            "the local ship must gain the AI hail"
        );
        let npc_admitted = app.world().entity(npc).get::<AdmittedCommands>().unwrap();
        assert_eq!(
            npc_admitted
                .for_target(crate::system_registry::COMMS_SYSTEM_ID)
                .count(),
            0,
            "a non-local ship must not be contaminated by the local ship's comms hail"
        );
    }

    // ── Authored hail ranking (issue #786) ──────────────────────────────────

    /// Two competing Hail directives, both eligible: the AUTHORED banded score
    /// ladder must pick the higher-scoring one (AC1 — the POLICY ranks, not a
    /// hardcoded argmax).
    ///
    /// The scores straddle the canonical bands (20 → 0 bands, 50 → 2 bands), so
    /// the ladder genuinely discriminates rather than tying and falling through
    /// to the selector's smallest-UUID rule.
    #[test]
    fn comms_ai_hails_the_higher_scored_of_two_eligible_directives() {
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Station Alpha", "zzz-station-alpha-uuid");
        register_name(&mut app, "Station Beta", "aaa-station-beta-uuid");
        {
            let mut mgr = ObjectiveManager::new();
            // Deliberately give the LOW-scoring objective the alphabetically
            // smaller UUID, so a tie would resolve to Beta and this assertion
            // only passes if the score ladder actually ranked.
            mgr.add_full(
                "hail-beta",
                "text",
                false,
                vec![],
                AiDirective::Hail {
                    target: "Station Beta".into(),
                },
                UtilityConfig {
                    base_priority: 20.0,
                    ..Default::default()
                },
                ObjectiveSource::Mission,
            );
            mgr.add_full(
                "hail-alpha",
                "text",
                false,
                vec![],
                AiDirective::Hail {
                    target: "Station Alpha".into(),
                },
                UtilityConfig {
                    base_priority: 50.0,
                    ..Default::default()
                },
                ObjectiveSource::Mission,
            );
            app.world_mut().insert_resource(ObjectiveManagerRes(mgr));
        }
        app.update();

        assert_eq!(
            admitted_hail_targets(&mut app),
            vec!["zzz-station-alpha-uuid".to_string()],
            "the authored score ladder must rank the higher-scored Hail directive first"
        );
    }

    /// A `comms-contacts` candidate carries no `source_hail_objective` marker,
    /// so under the canonical eligibility it ENRICHES rather than independently
    /// hails — the #778 `chart-contacts` shape. Baseline preservation: the
    /// retired code only ever hailed from the objective pool.
    #[test]
    fn comms_ai_does_not_hail_a_contact_with_no_directive() {
        let mut app = comms_ai_app(ControlSource::Ai);
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .contacts
            .push(crate::messages::CommsContact {
                uuid: "lonely-contact-uuid".into(),
                name: "Lonely Outpost".into(),
                in_range: true,
                is_urgent: true,
            });
        // No objective at all — nothing has ordered a hail.
        app.world_mut()
            .insert_resource(ObjectiveManagerRes(ObjectiveManager::new()));
        app.update();

        assert!(
            admitted_hail_targets(&mut app).is_empty(),
            "a comms contact with no Hail directive must not independently hail"
        );
    }

    // ── AC4: read-only scenario gating ──────────────────────────────────────

    /// An authored `eligibility` may READ scenario flags, and the tick must
    /// leave the flag store byte-identical (AC4 — read-only is structural:
    /// `evaluate_selector` takes `&[&FlagStore]` and every mutator needs
    /// `&mut self`).
    #[test]
    fn comms_ai_reads_but_never_mutates_scenario_flags() {
        use crate::entities::config::{FineSystemAiSelectorToml, ScoreTermToml};

        let flag_gated_selector = |app: &mut App| {
            let cfg = FineSystemAiSelectorToml {
                param: std::collections::HashMap::new(),
                sources: crate::entities::config::COMMS_SELECTOR_SOURCES
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                horizon: 1.0e9,
                switch_margin: 0.0,
                // Same canonical gates, plus a scenario-flag read.
                eligibility: "candidate_fact(source_hail_objective) > 0 \
                              and candidate_fact(in_range) > 0 \
                              and candidate_fact(objective_score) > 0 \
                              and flag(diplomatic_clearance)"
                    .to_string(),
                score: vec![ScoreTermToml {
                    when: "candidate_fact(objective_score) > 0".to_string(),
                    weight: 1.0,
                }],
            };
            assert!(
                crate::entities::config::validate_fine_system_ai_selector(
                    &cfg,
                    crate::entities::config::COMMS_SELECTOR_SOURCES
                )
                .is_ok(),
                "the flag-gated test selector must be valid authored content"
            );
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            let ship = q.single(app.world()).unwrap();
            app.world_mut()
                .entity_mut(ship)
                .insert(CommsTargetSelector {
                    selector: cfg.to_selector().unwrap(),
                    power_rating: None,
                });
        };

        // ── Flag CLEAR: the authored gate refuses the hail. ──────────────────
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        flag_gated_selector(&mut app);
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        let before = app.world().resource::<WorldContentRuntime>().flags.clone();
        app.update();
        assert!(
            admitted_hail_targets(&mut app).is_empty(),
            "an authored flag gate that reads false must refuse the hail"
        );
        assert_eq!(
            app.world().resource::<WorldContentRuntime>().flags,
            before,
            "AC4: evaluating the policy must leave the scenario flag store untouched"
        );

        // ── Flag SET: the same authored gate admits the hail. ────────────────
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .flags
            .set_flag("diplomatic_clearance");
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut AdmittedCommands, With<crate::server_app::LocalShip>>();
            q.single_mut(app.world_mut()).unwrap().0.clear();
        }
        let before = app.world().resource::<WorldContentRuntime>().flags.clone();
        app.update();
        assert_eq!(
            admitted_hail_targets(&mut app),
            vec!["station-alpha-uuid".to_string()],
            "a set scenario flag must let the authored gate admit the hail"
        );
        assert_eq!(
            app.world().resource::<WorldContentRuntime>().flags,
            before,
            "AC4: a FIRING policy must still leave the scenario flag store untouched"
        );
    }

    // ── AC5: anti-respam re-derived from authoritative state ────────────────

    /// Emulate `AdmissionPlugin`'s per-tick clear of the ship's admitted buffer.
    fn clear_admitted(app: &mut App) {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut AdmittedCommands, With<crate::server_app::LocalShip>>();
        q.single_mut(app.world_mut()).unwrap().0.clear();
    }

    /// Run one tick with a fresh admitted buffer and report how many hails the
    /// AI emitted on it. (`AdmissionPlugin` clears the buffer per tick; these
    /// bare-`App` fixtures do not carry it, so the helper does that job.)
    fn tick_and_count_hails(app: &mut App) -> usize {
        clear_admitted(app);
        app.update();
        admitted_hail_targets(app).len()
    }

    /// Push a `ClearComms` into the ship's admitted buffer, the way the seated
    /// Comms officer's `ControlSystem { payload: ClearComms }` arrives after
    /// admission. Used to prove that the anti-respam latch re-arms ONCE on an
    /// explicit, externally-driven clear — and not on its own.
    fn admit_clear_comms(app: &mut App) {
        use crate::messages::AdmittedCommand;
        let mut q = app
            .world_mut()
            .query_filtered::<&mut AdmittedCommands, With<crate::server_app::LocalShip>>();
        q.single_mut(app.world_mut())
            .unwrap()
            .0
            .push(AdmittedCommand {
                target: crate::system_registry::comms_system_id(),
                payload: SystemControlPayload::ClearComms,
                response_token: None,
            });
    }

    /// A standing Hail directive stays in the scored pool every tick, so without
    /// a TERMINATING guard the AI re-emits the same `Hail` forever — re-pushing
    /// `WorldEvent::Hailed`, which a `repeat = true` `on_hailed` trigger would
    /// act on without bound.
    ///
    /// Issue #786 replaced the retired `CommsAiHailState.last_hailed` AI memory
    /// with `candidate_fact(has_open_hail_thread)`, read off the authoritative
    /// `CommsRuntime.open_hails` record that `handle_hail` writes for human and
    /// AI hails alike. This test runs the real `handle_hail` +
    /// `handle_comms_channel2` so a genuine dialogue forms, then ticks MANY
    /// times to pin that the hail count stops growing — termination, not merely
    /// "quiet on tick 2".
    #[test]
    fn comms_ai_does_not_respam_a_standing_hail_while_its_thread_is_live() {
        let mut app = comms_ai_app(ControlSource::Ai);
        app.add_message::<CommsChannel2Event>().add_systems(
            Update,
            (handle_hail, handle_comms_channel2)
                .chain()
                .after(operate_comms_ai),
        );
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );

        app.update();
        assert_eq!(admitted_hail_targets(&mut app).len(), 1, "first tick hails");
        assert!(
            app.world()
                .resource::<CommsRuntime>()
                .open_hails
                .contains("station-alpha-uuid"),
            "the hail must be recorded — that record IS the guard"
        );

        for tick in 0..25 {
            assert_eq!(
                tick_and_count_hails(&mut app),
                0,
                "tick {tick}: a standing Hail whose thread is still open must \
                 never be re-emitted — the guard TERMINATES"
            );
        }
    }

    /// The latch re-arms EXACTLY ONCE on an explicit `ClearComms`, then
    /// terminates again.
    ///
    /// This is the deliberate improvement over the retired `last_hailed` cache
    /// (which stayed latched forever) AND the correction of the unterminating
    /// inbox-derived guard it briefly replaced: a hail seats no message of its
    /// own (issue #985 left `handle_hail` recording the hail and emitting
    /// `WorldEvent::Hailed`, nothing more), so no inbox-derived condition could
    /// ever re-arm — only `open_hails`, written by `handle_hail` itself, closes
    /// the loop.
    #[test]
    fn comms_ai_rehails_once_after_clear_comms_and_then_terminates() {
        let mut app = comms_ai_app(ControlSource::Ai);
        app.add_message::<CommsChannel2Event>().add_systems(
            Update,
            (handle_hail, handle_comms_channel2, handle_clear_comms)
                .chain()
                .after(operate_comms_ai),
        );
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();
        assert_eq!(admitted_hail_targets(&mut app).len(), 1);
        assert!(app
            .world()
            .resource::<CommsRuntime>()
            .open_hails
            .contains("station-alpha-uuid"));

        // The officer clears the slate. Same tick: the AI has already decided
        // (it is ordered before the handlers), so the re-hail lands on the NEXT
        // tick.
        clear_admitted(&mut app);
        admit_clear_comms(&mut app);
        app.update();
        assert!(
            app.world().resource::<CommsRuntime>().open_hails.is_empty(),
            "ClearComms must retire the open-hail record alongside the inbox"
        );

        assert_eq!(
            tick_and_count_hails(&mut app),
            1,
            "after an explicit ClearComms the standing directive hails afresh"
        );
        assert!(
            app.world()
                .resource::<CommsInboxRes>()
                .0
                .messages()
                .is_empty(),
            "a hail seats NO message on its own — an inbox-derived guard could \
             never re-arm here"
        );
        for tick in 0..25 {
            assert_eq!(
                tick_and_count_hails(&mut app),
                0,
                "tick {tick}: the re-hail must latch again, not loop"
            );
        }
    }

    /// FINDING 2 regression — a SECOND, later `Hail` directive naming a contact
    /// the ship has already hailed must still be honoured.
    ///
    /// An unmanned (Backfill) ship has NO `ClearComms` path — the command is a
    /// human console action only, and `TriggerAction` has no variant for it — so
    /// if `open_hails` only ever retired on `ClearComms`, every contact would be
    /// hailable exactly once per session and a mission's later "hail them again"
    /// beat would be silently dropped forever. `operate_comms_ai` therefore
    /// retires the latch once the target stops being a live hail candidate,
    /// which is what the retired `last_hailed` did (it reset whenever no target
    /// was selectable).
    #[test]
    fn a_second_hail_directive_after_the_first_completes_hails_again() {
        let mut app = comms_ai_app(ControlSource::Ai);
        app.add_message::<CommsChannel2Event>()
            .add_systems(Update, handle_hail.after(operate_comms_ai));
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        let directive = |id: &str| {
            (
                id.to_string(),
                AiDirective::Hail {
                    target: "Station Alpha".into(),
                },
            )
        };

        // Beat one: the briefing objective orders a hail. It lands, then latches.
        let (id, dir) = directive("hail-alpha-briefing");
        set_objective(
            &mut app,
            &id,
            dir,
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();
        assert_eq!(admitted_hail_targets(&mut app).len(), 1, "beat one hails");
        assert_eq!(tick_and_count_hails(&mut app), 0, "and then latches");

        // The objective completes and leaves the pool. Nothing else names the
        // station, so the latch retires: it is no longer a hail candidate.
        app.world_mut()
            .insert_resource(ObjectiveManagerRes(ObjectiveManager::new()));
        assert_eq!(
            tick_and_count_hails(&mut app),
            0,
            "with no directive there is nothing to hail"
        );
        assert!(
            app.world().resource::<CommsRuntime>().open_hails.is_empty(),
            "the latch must retire once the target stops being a candidate — \
             otherwise an unmanned ship can never hail this contact again"
        );

        // Beat two: a later objective orders the same contact hailed again.
        let (id, dir) = directive("hail-alpha-followup");
        set_objective(
            &mut app,
            &id,
            dir,
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        assert_eq!(
            tick_and_count_hails(&mut app),
            1,
            "a second Hail directive naming an already-hailed contact must be \
             honoured, not permanently dropped"
        );
        // ...and it terminates again, exactly as beat one did.
        for tick in 0..25 {
            assert_eq!(
                tick_and_count_hails(&mut app),
                0,
                "tick {tick}: beat two must latch too — the re-arm is keyed on \
                 candidacy, and a STANDING directive's target is a candidate \
                 every tick"
            );
        }
    }

    /// FINDING 2 regression — an out-of-range round trip re-arms the latch, the
    /// other half of what the retired `last_hailed` reset did.
    ///
    /// Termination is unaffected: leaving and re-entering comms range is driven
    /// by physical movement, not by the hail, so the hail cannot cause its own
    /// re-arm.
    #[test]
    fn leaving_and_re_entering_comms_range_re_arms_the_hail_latch() {
        let mut app = comms_ai_app(ControlSource::Ai);
        app.add_message::<CommsChannel2Event>()
            .add_systems(Update, handle_hail.after(operate_comms_ai));
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        {
            let mut rt = app.world_mut().resource_mut::<CommsRuntime>();
            rt.range_active = true;
            rt.range_flags
                .insert("station-alpha-uuid".to_string(), true);
        }
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();
        assert_eq!(admitted_hail_targets(&mut app).len(), 1, "in range: hails");
        assert_eq!(tick_and_count_hails(&mut app), 0, "and then latches");

        // The station falls out of comms range.
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .range_flags
            .insert("station-alpha-uuid".to_string(), false);
        assert_eq!(
            tick_and_count_hails(&mut app),
            0,
            "out of range there is no candidate, so no hail"
        );
        assert!(
            app.world().resource::<CommsRuntime>().open_hails.is_empty(),
            "an out-of-range target is no longer a candidate: the latch retires"
        );

        // ...and comes back.
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .range_flags
            .insert("station-alpha-uuid".to_string(), true);
        assert_eq!(
            tick_and_count_hails(&mut app),
            1,
            "back in range, the standing directive hails afresh"
        );
        for tick in 0..25 {
            assert_eq!(
                tick_and_count_hails(&mut app),
                0,
                "tick {tick}: and latches again — no loop"
            );
        }
    }

    /// FINDING 2 — the latch retirement is scoped to the AI-operated path, so a
    /// HUMAN Comms officer's open channels are never quietly closed under them.
    /// Theirs are retired by their own `ClearComms`.
    #[test]
    fn the_candidacy_re_arm_never_touches_a_human_officers_open_hails() {
        let mut app = comms_ai_app(ControlSource::Human);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        app.world_mut()
            .insert_resource(ObjectiveManagerRes(ObjectiveManager::new()));
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .open_hails
            .insert("station-alpha-uuid".to_string());
        app.update();
        assert!(
            app.world()
                .resource::<CommsRuntime>()
                .open_hails
                .contains("station-alpha-uuid"),
            "with a human at Comms the AI must not retire their open-hail record"
        );
    }

    /// The termination guarantee holds for the case that has NO comms content at
    /// all: a hail to a target with no `on_hailed` template seats no message and
    /// no dialogue, so nothing inbox-shaped exists to suppress the next hail.
    /// The authoritative `open_hails` record still arms, because the hail
    /// genuinely happened.
    ///
    /// (Previously this test asserted the OPPOSITE — that a no-template target
    /// keeps being hailed every tick — on the grounds that "no memory" implies
    /// "no suppression". That blessed an unbounded `WorldEvent::Hailed` loop.)
    #[test]
    fn comms_ai_terminates_a_standing_hail_to_a_target_with_no_comms_template() {
        let mut app = comms_ai_app(ControlSource::Ai);
        app.add_message::<CommsChannel2Event>()
            .add_systems(Update, handle_hail.after(operate_comms_ai));
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        // Deliberately NO `install_hail_template`.
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();
        assert_eq!(admitted_hail_targets(&mut app).len(), 1, "first tick hails");
        assert!(
            app.world()
                .resource::<CommsInboxRes>()
                .0
                .messages()
                .is_empty(),
            "no template fired, so no dialogue was ever seated"
        );

        for tick in 0..25 {
            assert_eq!(
                tick_and_count_hails(&mut app),
                0,
                "tick {tick}: a no-template target must NOT be re-hailed every \
                 tick — `WorldEvent::Hailed` would fire repeat-able `on_hailed` \
                 triggers without bound"
            );
        }
    }

    /// No AI-OWNED comms state survives issue #786: `CommsAiHailState` is
    /// deleted, and the decision is a pure function of this tick's authoritative
    /// snapshot. The anti-respam record that replaced it lives on `CommsRuntime`
    /// and is written by `handle_hail` for HUMAN hails too — so a human officer's
    /// hail suppresses the AI's identically, which no AI-private memory could do.
    #[test]
    fn comms_ai_keeps_no_private_memory_between_ticks() {
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        // A HUMAN officer hailed Station Alpha last tick. The AI never ran, so
        // no AI-side cache exists — yet the shared authoritative record must
        // still suppress the AI's hail.
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .open_hails
            .insert("station-alpha-uuid".to_string());
        app.update();
        assert!(
            admitted_hail_targets(&mut app).is_empty(),
            "the anti-respam guard reads SHARED authoritative comms state, not \
             AI-private memory — a human's hail suppresses the AI's"
        );

        // Retiring the record (a `ClearComms`) makes the same standing directive
        // eligible again, proving nothing AI-side was latched.
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .open_hails
            .clear();
        assert_eq!(
            tick_and_count_hails(&mut app),
            1,
            "with the shared record cleared the directive is eligible again"
        );
    }

    /// FINDING 2 regression, shaped after `assets/worlds/before_the_fire.toml`:
    /// an `on_world_loaded` template pushes an urgent message FROM Axiom Station
    /// into the inbox before the ship has hailed anyone. Giving the briefing
    /// objective a `Hail` directive naming Axiom Station must still hail.
    ///
    /// The old inbox-derived guard returned true for ANY un-orphaned message
    /// from that sender UUID regardless of provenance, so the opening message
    /// permanently satisfied it and the AI would NEVER have hailed.
    #[test]
    fn an_unrelated_inbound_message_does_not_suppress_a_legitimate_hail() {
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Axiom Station", "axiom-station-uuid");
        // Scenario-pushed comms resolve `sender_uuid` from the template's `from`
        // name, so an `on_world_loaded` message arrives already attributed to
        // the station's real UUID.
        {
            let mut message = msg("axiom-briefing");
            message.sender_uuid = "axiom-station-uuid".to_string();
            message.is_urgent = true;
            app.world_mut()
                .resource_mut::<CommsInboxRes>()
                .0
                .inject(message);
        }
        set_objective(
            &mut app,
            "obj-hail-briefing",
            AiDirective::Hail {
                target: "Axiom Station".into(),
            },
            UtilityConfig {
                base_priority: 40.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        app.update();
        assert_eq!(
            admitted_hail_targets(&mut app),
            vec!["axiom-station-uuid".to_string()],
            "a message the station SENT us is not a hail WE opened — it must not \
             suppress the directive"
        );
    }

    /// AC2 — a Destroyed Comms fine system stops the ship hailing. The gate is
    /// the canonical eligibility's `self_fact(comms_available) > 0` term, seeded
    /// from `EntitySystemHull`; the sibling response-half assertion lives in
    /// `comms_response_ai_holds_while_the_comms_system_is_destroyed`.
    #[test]
    fn comms_ai_does_not_hail_while_the_comms_system_is_destroyed() {
        let mut app = comms_ai_app(ControlSource::Ai);
        register_name(&mut app, "Station Alpha", "station-alpha-uuid");
        set_objective(
            &mut app,
            "hail-alpha",
            AiDirective::Hail {
                target: "Station Alpha".into(),
            },
            UtilityConfig {
                base_priority: 20.0,
                ..Default::default()
            },
            ObjectiveSource::Mission,
        );
        // Sanity: healthy first.
        app.update();
        assert_eq!(admitted_hail_targets(&mut app).len(), 1);

        destroy_comms_system(&mut app);
        assert_eq!(
            tick_and_count_hails(&mut app),
            0,
            "AC2: a Destroyed Comms system must stop the ship hailing"
        );
    }

    /// Attach an `EntitySystemHull` to the fixture's `LocalShip` whose Comms
    /// fine system is Destroyed.
    fn destroy_comms_system(app: &mut App) {
        let hull = destroyed_comms_hull();
        let entity = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            q.single(app.world()).unwrap()
        };
        app.world_mut().entity_mut(entity).insert(hull);
    }

    /// An `EntitySystemHull` whose Comms fine system reads
    /// `DamageTier::Destroyed` through the real tier derivation (`current == 0`).
    fn destroyed_comms_hull() -> crate::entity_spawner::EntitySystemHull {
        let comms = crate::system_registry::comms_system_id();
        let mut hull = crate::ship::damage::SystemHull::from_config(&[(comms.clone(), 100.0)]);
        hull.set_hp(&comms, 0.0);
        assert_eq!(
            hull.tier_for(&comms),
            crate::ship::damage::DamageTier::Destroyed,
            "fixture must actually destroy the Comms system"
        );
        crate::entity_spawner::EntitySystemHull(hull)
    }

    // -- handle_respond_to_message: comms-response action dispatch parity ---
    //
    // Moved from world::server::tests (issue #608). These tests share the
    // low-level test harness (`comms_test_app`, `push_msg`, `tick`,
    // `write_spawn_template_fixture`) with the rest of the world-module test
    // suite, so that harness stays in `world::server::tests` (now
    // `pub(crate)`) and is imported here rather than duplicated.
    use crate::comms::content::{CommsDialogueNode, CommsResponse};
    use crate::comms::server::tests::{comms_test_app, push_msg, setup_game_with_comms, tick};
    use crate::messages::{ClientMessage, ServerMessage};

    // -- PRD #397 fix 2: comms-response action dispatch parity ----------------
    //
    // These tests assert that `handle_respond_to_message` dispatches every
    // `TriggerAction` variant that `tick_trigger_pipeline` dispatches. The
    // "enumeration" test at the end matches on every variant of `TriggerAction`
    // so adding a new variant is a compile error until the new variant is
    // wired into both dispatch sites and a per-variant assertion is added.

    // -- Issue #786: AI responses traverse the REAL consequence router --------
    //
    // The retired `handle_comms_channel2` stub wrote `inbox.record_response(&id,
    // 0)` directly, so an AI "response" fired NO trigger actions and advanced NO
    // follow-up. These tests pin the replacement: the Comms AI's answer is an
    // ordinary admitted `RespondToMessage` drained by the SAME
    // `handle_respond_to_message` a human's answer is, so `dispatch_action` runs
    // and the ROUTER (not a stub) records `selected_response`.

    /// AC3/AC6 — an AI response runs its `on_pick` fn through the existing
    /// consequence router, and `selected_response` is recorded BY THE ROUTER
    /// rather than by a stub. This is the test the retired
    /// `record_response(&id, 0)` stub could never have passed: it never reached
    /// the dispatch path at all.
    ///
    /// The dialogue is seated directly rather than produced by an AI hail: the
    /// hail used to fire a `[[comms]] on_hailed` template, and issue #985
    /// deleted that front-end. What the test is for — the AI's answer traversing
    /// the SAME `handle_respond_to_message` a human's does — is unchanged.
    #[test]
    fn comms_ai_response_fires_its_on_pick_through_the_router() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456786";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        app.world_mut()
            .insert_resource(crate::comms::scripted::tests::compile_fixture(
                r#"
                fn on_ack(ctx) { ctx.flags.ai_comms_answered = 1; }
                "#,
            ));
        // Comms under AI control — the Backfill case — with the response
        // decider ordered exactly as the real plugin orders it: inside
        // `SimSet::Input`, before the handler that drains it.
        app.add_systems(
            FixedUpdate,
            operate_comms_response_ai
                .before(handle_respond_to_message)
                .after(crate::server_app::AdmissionSet),
        );
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::simulation::LocalShip>>();
            q.single_mut(app.world_mut())
                .unwrap()
                .0
                .set(crate::system_registry::comms_system_id(), ControlSource::Ai);
        }

        let id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Go ahead, Phoenix.",
            vec!["on_ack"],
            false,
        );
        let _ = tick(&mut app);

        let runtime = app.world().resource::<WorldContentRuntime>();
        assert_eq!(
            runtime.flags.counter("ai_comms_answered"),
            1,
            "an AI comms response must apply its on_pick's effects through the \
             shared router — the retired stub applied none"
        );
        assert!(
            runtime.pending_world_events.iter().any(|e| matches!(
                e, WorldEvent::FlagSet { name, .. } if name == "ai_comms_answered"
            )),
            "the AI response's flag write must enqueue a FlagSet event for \
             tick_trigger_pipeline to chain on, exactly as a human response does"
        );

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(
            messages
                .iter()
                .find(|m| m.id == id)
                .expect("the seated message is still in the inbox")
                .selected_response,
            Some(0),
            "the ROUTER records selected_response for an AI answer, as it does \
             for a human one"
        );
    }

    /// AI/human symmetry (AGENTS.md #6): the AI's answer is byte-identical to
    /// the `RespondToMessage` payload a human Comms officer submits, and it is
    /// admitted onto the same `AdmittedCommands` buffer.
    #[test]
    fn comms_ai_emits_the_same_response_payload_a_human_sends() {
        let mut app = comms_ai_response_app();
        let (msg_id, _) = seat_ai_dialogue(&mut app, "sender-uuid");
        app.update();

        let mut q = app
            .world_mut()
            .query_filtered::<&AdmittedCommands, With<crate::server_app::LocalShip>>();
        let admitted = q.single(app.world()).unwrap();
        let payloads: Vec<_> = admitted
            .for_target(crate::system_registry::COMMS_SYSTEM_ID)
            .map(|cmd| cmd.payload.clone())
            .collect();
        assert_eq!(
            payloads,
            vec![SystemControlPayload::RespondToMessage {
                message_id: msg_id,
                response_index: 0,
            }],
            "AI-emitted comms response must equal the payload a human \
             ControlSystem sends"
        );
    }

    /// AC5 human exclusivity: a human-held Comms console answers its own
    /// dialogues — the AI must emit nothing.
    #[test]
    fn comms_ai_does_not_respond_when_human_operated() {
        let mut app = comms_ai_response_app_with(ControlSource::Human);
        let _ = seat_ai_dialogue(&mut app, "sender-uuid");
        app.update();

        let mut q = app
            .world_mut()
            .query_filtered::<&AdmittedCommands, With<crate::server_app::LocalShip>>();
        let admitted = q.single(app.world()).unwrap();
        assert_eq!(
            admitted
                .for_target(crate::system_registry::COMMS_SYSTEM_ID)
                .count(),
            0,
            "a human-operated Comms console must not emit an AI response"
        );
    }

    /// AC5 — the AI needs NO staleness check of its own. When the dialogue is
    /// invalidated between the decision and the router (the stale case), the
    /// SHARED router's existing gate rejects the AI's response exactly as it
    /// rejects a forced human one: `CommsResponseRejected` goes out and
    /// `selected_response` is never recorded.
    ///
    /// The sibling of `stale_response_is_rejected`, which pins the same gate for
    /// a human submission.
    #[test]
    fn stale_ai_response_is_rejected_by_the_shared_router() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456013";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        // A test-only saboteur that retires the dialogue AFTER the AI decides
        // but BEFORE the router runs — the stale window. Armed by the test only
        // for the final tick, so the dialogue survives the tick that seats it.
        #[derive(Resource, Default)]
        struct RetireDialoguesNow(bool);
        fn retire_dialogues(armed: Res<RetireDialoguesNow>, mut comms: ResMut<CommsRuntime>) {
            if armed.0 {
                comms.active_dialogues.clear();
            }
        }
        // `FixedUpdate` (issue #895): same schedule as the router, so the
        // decide → sabotage → route order is real.
        app.init_resource::<RetireDialoguesNow>().add_systems(
            FixedUpdate,
            (operate_comms_response_ai, retire_dialogues)
                .chain()
                .after(crate::server_app::AdmissionSet)
                .before(handle_respond_to_message),
        );
        let _ = tick(&mut app);

        // Seat a dialogue. (It used to arrive from a hail firing a `[[comms]]`
        // template; issue #985 deleted that front-end. What is under test is the
        // ROUTER's stale gate applying to an AI-origin response, which does not
        // care how the thread was opened.)
        let msg_id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Go ahead, Phoenix.",
            vec!["on_ack"],
            false,
        );

        // Now hand Comms to the AI.
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::simulation::LocalShip>>();
            q.single_mut(app.world_mut())
                .unwrap()
                .0
                .set(crate::system_registry::comms_system_id(), ControlSource::Ai);
        }

        // Next tick the AI decides, the saboteur retires the dialogue, and the
        // router refuses the now-stale response.
        app.world_mut().resource_mut::<RetireDialoguesNow>().0 = true;
        let out = tick(&mut app);
        let (rejected_id, idx) =
            find_rejection(&out).expect("a stale AI response must be rejected by the router");
        assert_eq!(rejected_id, msg_id);
        assert_eq!(idx, 0);
        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        let msg = messages
            .iter()
            .find(|m| m.id == msg_id)
            .expect("the message is still in the inbox");
        assert_eq!(
            msg.selected_response, None,
            "a rejected AI response must not record a selection — the router, \
             not the AI, is the authority"
        );
    }

    /// Minimal app running ONLY `operate_comms_response_ai` (no router, no
    /// AdmissionPlugin clear) so a test can inspect the `RespondToMessage` the
    /// AI leaves in the ship's own `AdmittedCommands`.
    fn comms_ai_response_app() -> App {
        comms_ai_response_app_with(ControlSource::Ai)
    }

    fn comms_ai_response_app_with(comms_source: ControlSource) -> App {
        let mut app = App::new();
        app.insert_resource(crate::lobby::Sessions(
            crate::lobby::session::SessionManager::new(),
        ))
        .insert_resource(WorldContentRuntime::default())
        .insert_resource(CommsRuntime::default())
        .insert_resource(CommsInboxRes(crate::console::comms::CommsInbox::new()))
        .add_systems(Update, operate_comms_response_ai);

        let mut resolver = ControlSourceResolver::new();
        resolver.set(crate::system_registry::comms_system_id(), comms_source);
        app.world_mut().spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            ShipSystemControlSources(resolver),
            crate::ship_plugin::ShipConfigComponent::default(),
            AdmittedCommands::default(),
            // The AUTHORED `[comms_console.ai]` block every shipped hull
            // carries. Since #885b stage 5d `operate_comms_response_ai` has no
            // synthesised fallback — a ship with no policy answers nothing.
            CommsResponseAiPolicy(
                crate::entities::authored_ai_pins::shipped_policy_toml("comms_response")
                    .to_policy()
                    .expect("the shipped Comms response policy decodes"),
            ),
            // …and the co-located hail selector, which is where the response
            // host reads the ship's authored `power_rating` from.
            CommsTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail")
                    .to_selector()
                    .expect("the shipped Comms hail selector decodes"),
                power_rating: None,
            },
        ));
        app
    }

    /// Seat one open dialogue: an un-answered inbox message from `sender_uuid`
    /// plus the matching `ActiveDialogue` node with a single response. Returns
    /// `(message_id, sender_uuid)`.
    fn seat_ai_dialogue(app: &mut App, sender_uuid: &str) -> (String, String) {
        use crate::comms::content::{ActiveDialogue, CommsDialogueNode, CommsResponse};
        let mut message = msg("ai-dialogue-1");
        message.sender_uuid = sender_uuid.to_string();
        let id = message.id.clone();
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(message);
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .active_dialogues
            .insert(
                id.clone(),
                ActiveDialogue {
                    current_node: CommsDialogueNode {
                        body: "Go ahead.".into(),
                        responses: vec![CommsResponse {
                            text: "Acknowledge.".into(),
                            important: false,
                        }],
                    },
                    thread_id: id.clone(),
                    script: crate::comms::content::ScriptedDialogue {
                        script_path: crate::comms::scripted::tests::PATH.to_string(),
                        node_fn: "root".to_string(),
                        on_pick: vec!["on_ack".to_string()],
                    },
                },
            );
        (id, sender_uuid.to_string())
    }

    /// AC4 for the RESPONSE half: an authored `when` guard may read scenario
    /// flags, and resolving it must leave the flag store byte-identical.
    #[test]
    fn comms_response_ai_reads_but_never_mutates_scenario_flags() {
        use crate::entities::config::{
            FineSystemAiConfigToml, FineSystemAiRuleToml, COMMS_RESPOND_CHANNEL, COMMS_RESPOND_VERB,
        };
        let mut app = comms_ai_response_app();
        let (msg_id, _) = seat_ai_dialogue(&mut app, "sender-uuid");
        let cfg = FineSystemAiConfigToml {
            evaluate_every_ticks: crate::entities::config::default_evaluate_every_ticks(),
            idle: false,
            param: std::collections::HashMap::new(),
            rule: vec![FineSystemAiRuleToml {
                priority: 0,
                channel: COMMS_RESPOND_CHANNEL.to_string(),
                when: "flag(cleared_to_answer)".to_string(),
                verb: COMMS_RESPOND_VERB.to_string(),
                value: false,
                level: 0,
                response_index: 0,
            }],
            initial_state: None,
            state: Vec::new(),
            memory: std::collections::HashMap::new(),
        };
        assert!(
            crate::entities::config::validate_fine_system_ai_policy(
                &cfg,
                crate::entities::config::COMMS_RESPOND_CHANNELS,
                crate::entities::config::COMMS_RESPOND_VERBS,
            )
            .is_ok(),
            "the flag-gated test policy must be valid authored content"
        );
        {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            let ship = q.single(app.world()).unwrap();
            app.world_mut()
                .entity_mut(ship)
                .insert(CommsResponseAiPolicy(cfg.to_policy().unwrap()));
        }

        let before = app.world().resource::<WorldContentRuntime>().flags.clone();
        app.update();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&AdmittedCommands, With<crate::server_app::LocalShip>>();
            let admitted = q.single(app.world()).unwrap();
            assert_eq!(
                admitted
                    .for_target(crate::system_registry::COMMS_SYSTEM_ID)
                    .count(),
                0,
                "an authored flag gate that reads false must hold the response"
            );
        }
        assert_eq!(
            app.world().resource::<WorldContentRuntime>().flags,
            before,
            "AC4: resolving the response policy must not mutate scenario flags"
        );

        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .flags
            .set_flag("cleared_to_answer");
        let before = app.world().resource::<WorldContentRuntime>().flags.clone();
        app.update();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&AdmittedCommands, With<crate::server_app::LocalShip>>();
            let admitted = q.single(app.world()).unwrap();
            let payloads: Vec<_> = admitted
                .for_target(crate::system_registry::COMMS_SYSTEM_ID)
                .map(|cmd| cmd.payload.clone())
                .collect();
            assert_eq!(
                payloads,
                vec![SystemControlPayload::RespondToMessage {
                    message_id: msg_id,
                    response_index: 0,
                }],
                "a set scenario flag must let the authored guard answer"
            );
        }
        assert_eq!(
            app.world().resource::<WorldContentRuntime>().flags,
            before,
            "AC4: a FIRING response policy must still leave scenario flags untouched"
        );
    }

    /// An already-answered message is not re-answered: the host only decides
    /// about dialogues genuinely awaiting an answer, so a routed response is
    /// never re-emitted on the next tick.
    #[test]
    fn comms_ai_does_not_re_answer_a_resolved_message() {
        let mut app = comms_ai_response_app();
        let (msg_id, _) = seat_ai_dialogue(&mut app, "sender-uuid");
        app.update();
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&AdmittedCommands, With<crate::server_app::LocalShip>>();
            assert_eq!(
                q.single(app.world())
                    .unwrap()
                    .for_target(crate::system_registry::COMMS_SYSTEM_ID)
                    .count(),
                1
            );
        }
        // The router would have recorded the selection; emulate that plus the
        // AdmissionPlugin per-tick clear.
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .record_response(&msg_id, 0);
        {
            let mut q = app
                .world_mut()
                .query_filtered::<&mut AdmittedCommands, With<crate::server_app::LocalShip>>();
            q.single_mut(app.world_mut()).unwrap().0.clear();
        }
        app.update();
        let mut q = app
            .world_mut()
            .query_filtered::<&AdmittedCommands, With<crate::server_app::LocalShip>>();
        assert_eq!(
            q.single(app.world())
                .unwrap()
                .for_target(crate::system_registry::COMMS_SYSTEM_ID)
                .count(),
            0,
            "an answered message must not be answered again"
        );
    }

    /// Count the `RespondToMessage`s the AI left in the ship's admitted buffer.
    fn admitted_response_count(app: &mut App) -> usize {
        let mut q = app
            .world_mut()
            .query_filtered::<&AdmittedCommands, With<crate::server_app::LocalShip>>();
        q.single(app.world())
            .unwrap()
            .for_target(crate::system_registry::COMMS_SYSTEM_ID)
            .filter(|cmd| matches!(cmd.payload, SystemControlPayload::RespondToMessage { .. }))
            .count()
    }

    /// AC2, response half — a Destroyed Comms fine system stops the ship
    /// ANSWERING, not just hailing. The gate is the canonical policy's
    /// `fact(comms_available) > 0` term; before this was added the default
    /// `when = "true"` rule let a ship with no Comms system at all keep talking.
    #[test]
    fn comms_response_ai_holds_while_the_comms_system_is_destroyed() {
        let mut app = comms_ai_response_app();
        seat_ai_dialogue(&mut app, "sender-uuid");
        // Sanity: healthy first.
        app.update();
        assert_eq!(admitted_response_count(&mut app), 1);

        clear_admitted(&mut app);
        let hull = destroyed_comms_hull();
        let entity = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            q.single(app.world()).unwrap()
        };
        app.world_mut().entity_mut(entity).insert(hull);
        app.update();
        assert_eq!(
            admitted_response_count(&mut app),
            0,
            "AC2: a Destroyed Comms system must stop the ship answering too"
        );
    }

    /// FINDING 5 regression — a sender that leaves comms range mid-dialogue must
    /// not make the AI re-emit a response the router is guaranteed to reject.
    ///
    /// `handle_respond_to_message` refuses an out-of-range response, so under the
    /// old `when = "true"` rule the AI re-submitted the same doomed
    /// `RespondToMessage` every tick forever (and the officer's panel re-flashed
    /// its rejection). The canonical rule now names `fact(sender_in_range) > 0`.
    #[test]
    fn comms_response_ai_holds_while_the_sender_is_out_of_range() {
        let sender = "a1b2c3d4-e5f6-4789-abcd-ef0123456099";
        let mut app = comms_ai_response_app();
        seat_ai_dialogue(&mut app, sender);
        // Range tracking live, sender out of range.
        {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            comms.range_active = true;
            comms.range_flags.insert(sender.to_string(), false);
        }
        for tick in 0..10 {
            clear_admitted(&mut app);
            app.update();
            assert_eq!(
                admitted_response_count(&mut app),
                0,
                "tick {tick}: a doomed out-of-range response must not be re-emitted"
            );
        }

        // The sender comes back: the same standing dialogue is answered.
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .range_flags
            .insert(sender.to_string(), true);
        clear_admitted(&mut app);
        app.update();
        assert_eq!(
            admitted_response_count(&mut app),
            1,
            "once the sender is back in range the AI answers normally"
        );
    }

    /// Attach the Comms console AI pair to the fixture's `LocalShip` exactly the
    /// way production does — through
    /// [`comms_console_ai_components`], the single helper BOTH
    /// `entities::spawner::spawn_entity` and `server_app::spawn_game_start_entities`
    /// call. Tests that go through this are testing the wiring, not a hand-built
    /// component: if the helper stopped reading `[comms_console]`, or stopped
    /// carrying `power_rating`, they fail.
    fn attach_comms_console_ai_from_toml(app: &mut App, toml: &str) {
        let config = crate::entity_config::EntityConfig::from_toml(toml)
            .expect("the fixture template must parse and validate");
        let (selector, policy, cadence) = comms_console_ai_components(&config);
        let entity = {
            let mut q = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            q.single(app.world()).unwrap()
        };
        // Each half is `None` when the fixture does not author it — since #885b
        // stage 5d the helper no longer invents one — and the fixture app
        // already carries the SHIPPED declaration for both, so an unauthored
        // half simply keeps the baseline it started with.
        assert!(
            selector.is_some() || policy.is_some(),
            "the fixture must author at least one half of `[comms_console]`, or it              tests nothing"
        );
        match selector {
            Some(selector) => {
                app.world_mut().entity_mut(entity).insert(selector);
            }
            // No authored selector block, but the fixture may still declare a
            // ship `power_rating` — which rides the SELECTOR component in
            // production, and which the response host reads off it. Carry it
            // onto the baseline component rather than dropping it.
            None => {
                if let Some(rating) = config.power_rating {
                    let mut e = app.world_mut().entity_mut(entity);
                    let mut comp = e
                        .get_mut::<CommsTargetSelector>()
                        .expect("the fixture app attaches the shipped hail selector");
                    comp.power_rating = Some(rating as f32);
                }
            }
        }
        if let Some(policy) = policy {
            app.world_mut().entity_mut(entity).insert(policy);
        }
        if let Some(cadence) = cadence {
            app.world_mut().entity_mut(entity).insert(cadence);
        }
    }

    /// FINDING 1/4 regression — `fact(power_rating)` must carry the ship's REAL
    /// authored rating in the response policy, not be permanently absent.
    ///
    /// `CommsResponseAiPolicy` carries no rating of its own, so the host reads it
    /// off the co-located `CommsTargetSelector`. While that component was never
    /// attached (or carried `None`), an authored `fact(power_rating) > 3` guard
    /// silently never fired — exactly the #779 empty-facts failure mode.
    ///
    /// Driven end to end from TOML through the PRODUCTION helper both spawn
    /// paths call, so it proves the wiring rather than a hand-built fixture:
    /// `power_rating = 5` in the template must reach the running host. Both
    /// directions are asserted: a satisfied guard answers, an unsatisfied one
    /// holds.
    #[test]
    fn comms_response_ai_reads_the_ships_real_power_rating() {
        // A rating of 5 satisfies `> 3` and fails `> 8`.
        for (threshold, expected, why) in [
            (
                "3",
                1,
                "a power_rating guard the ship SATISFIES must fire — the fact must \
                 carry the real authored rating, read off the co-located selector \
                 component the spawn paths attach",
            ),
            (
                "8",
                0,
                "a power_rating guard the ship fails must hold — proving the fact \
                 is a real reading, not a constant",
            ),
        ] {
            let mut app = comms_ai_response_app();
            seat_ai_dialogue(&mut app, "sender-uuid");
            attach_comms_console_ai_from_toml(
                &mut app,
                &format!(
                    r##"
power_rating = 5

[[comms_console.ai.rule]]
priority = 0
channel = "comms_respond"
when = "fact(power_rating) > {threshold}"
verb = "respond_to_message"
response_index = 0
"##
                ),
            );
            app.update();
            assert_eq!(admitted_response_count(&mut app), expected, "{why}");
        }
    }

    /// FINDING 1 regression — an authored `[comms_console.ai]` block must
    /// actually REACH the running host and BEAT the canonical default.
    ///
    /// Before the `server_app` attach, `[comms_console]` parsed and validated and
    /// was then silently ignored on the only ship either Comms AI host runs on:
    /// with no component attached, `operate_comms_response_ai`'s tick-local
    /// `default_policy` always won. The default answers with index 0; this
    /// template authors index 1, so a passing assertion can only come from the
    /// authored policy.
    #[test]
    fn an_authored_comms_response_policy_beats_the_canonical_default() {
        use crate::comms::content::{ActiveDialogue, CommsDialogueNode, CommsResponse};
        let mut app = comms_ai_response_app();
        let (msg_id, _) = seat_ai_dialogue(&mut app, "sender-uuid");
        // Widen the seated node to two responses so index 1 is in bounds.
        {
            let mut rt = app.world_mut().resource_mut::<CommsRuntime>();
            let dialogue = rt.active_dialogues.get_mut(&msg_id).unwrap();
            dialogue.current_node = CommsDialogueNode {
                body: "Go ahead.".into(),
                responses: vec![
                    CommsResponse {
                        text: "Acknowledge.".into(),
                        important: false,
                    },
                    CommsResponse {
                        text: "Stand by.".into(),
                        important: false,
                    },
                ],
            };
            let _: &ActiveDialogue = dialogue;
        }
        attach_comms_console_ai_from_toml(
            &mut app,
            r##"
[[comms_console.ai.rule]]
priority = 0
channel = "comms_respond"
when = "fact(response_count) > 1"
verb = "respond_to_message"
response_index = 1
"##,
        );
        app.update();

        let mut q = app
            .world_mut()
            .query_filtered::<&AdmittedCommands, With<crate::server_app::LocalShip>>();
        let indices: Vec<usize> = q
            .single(app.world())
            .unwrap()
            .for_target(crate::system_registry::COMMS_SYSTEM_ID)
            .filter_map(|cmd| match &cmd.payload {
                SystemControlPayload::RespondToMessage { response_index, .. } => {
                    Some(*response_index)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            indices,
            vec![1],
            "the AUTHORED `[comms_console.ai]` rule must decide the answer — the \
             canonical default answers with index 0, so index 1 can only come \
             from the authored policy actually reaching the host"
        );
    }

    /// FINDING 1 regression, hail half — an authored `[comms_console.selector]`
    /// must reach `operate_comms_ai` and BEAT the canonical default.
    ///
    /// The canonical eligibility requires a positive
    /// `candidate_fact(source_hail_objective)`, so a roster contact with no
    /// `Hail` directive naming it is NEVER hailed (the `comms-contacts` source
    /// only enriches). This template widens eligibility onto the roster, so a
    /// hail here can only come from the authored selector.
    #[test]
    fn an_authored_comms_hail_selector_beats_the_canonical_default() {
        let mut app = comms_ai_app(ControlSource::Ai);
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .contacts
            .push(crate::messages::CommsContact {
                uuid: "lonely-contact-uuid".into(),
                name: "Lonely Outpost".into(),
                in_range: true,
                is_urgent: true,
            });
        // No objective at all — nothing has ordered a hail, so the canonical
        // default selector produces no eligible candidate.
        app.world_mut()
            .insert_resource(ObjectiveManagerRes(ObjectiveManager::new()));
        app.update();
        assert!(
            admitted_hail_targets(&mut app).is_empty(),
            "baseline: the canonical default never hails a directive-less contact"
        );

        clear_admitted(&mut app);
        attach_comms_console_ai_from_toml(
            &mut app,
            r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["hail-objectives", "comms-contacts"]
eligibility = "candidate_fact(source_comms_contact) > 0 and candidate_fact(in_range) > 0 and candidate_fact(has_open_hail_thread) < 1"

[[comms_console.selector.score]]
when = "candidate_fact(is_urgent) > 0"
weight = 100.0
"##,
        );
        app.update();
        assert_eq!(
            admitted_hail_targets(&mut app),
            vec!["lonely-contact-uuid".to_string()],
            "the AUTHORED `[comms_console.selector]` must decide eligibility — \
             the canonical default forbids this hail, so it can only come from \
             the authored selector actually reaching the host"
        );
    }

    // -- Issue #761: authoritative rejection feedback (AC3) --------------------

    fn find_rejection(out: &[crate::lobby::OutboundMessage]) -> Option<(String, usize)> {
        out.iter().find_map(|m| match &m.msg {
            ServerMessage::CommsResponseRejected {
                message_id,
                response_index,
            } => Some((message_id.clone(), *response_index)),
            _ => None,
        })
    }

    /// A `RespondToMessage` for a message with no active dialogue (stale — the
    /// message was cleared or never existed) is rejected, and the rejection is
    /// addressed to the submitting comms holder.
    #[test]
    fn stale_response_is_rejected() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456011";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: "no-such-message".into(),
                    response_index: 3,
                },
            },
        );
        let out = tick(&mut app);
        let (message_id, response_index) =
            find_rejection(&out).expect("stale response must be rejected");
        assert_eq!(message_id, "no-such-message");
        assert_eq!(response_index, 3);
    }

    /// A `RespondToMessage` whose sender has left comms range is rejected
    /// (forced/stale submission on a greyed response). Hail in range to seat an
    /// active dialogue, move the station away, then respond.
    #[test]
    fn out_of_range_response_is_rejected() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456012";
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

        // Seat a dialogue while the sender is in range.
        let msg_id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Go ahead, Phoenix.",
            vec!["on_ack"],
            false,
        );

        // Move the station out of range.
        if let Ok(mut e) = app.world_mut().get_entity_mut(station_entity) {
            e.insert(Transform::from_xyz(5000.0, 0.0, 0.0));
        }
        let _ = tick(&mut app);

        // Respond now that the sender is out of range.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: msg_id.clone(),
                    response_index: 0,
                },
            },
        );
        let out = tick(&mut app);
        let (rejected_id, idx) =
            find_rejection(&out).expect("out-of-range response must be rejected");
        assert_eq!(rejected_id, msg_id);
        assert_eq!(idx, 0);
    }

    // -- Issue #984: the scripted arm of handle_respond_to_message -------------
    //
    // The arm under test lives in this file; its fixture helper is shared with
    // `comms::scripted`, which owns the open half of the same thread lifecycle.

    /// Seat a scripted thread the way `open_scripted_comms_threads` would: the
    /// projected node in the inbox and in `active_dialogues`, with the
    /// `ScriptedDialogue` naming the unit and the `on_pick` fn per response.
    /// Returns the message id.
    fn seat_scripted_dialogue(
        app: &mut App,
        sender_uuid: &str,
        body: &str,
        on_pick: Vec<&str>,
        urgent: bool,
    ) -> String {
        let id = format!("scripted-msg-{}", on_pick.len());
        let responses: Vec<CommsResponse> = on_pick
            .iter()
            .enumerate()
            .map(|(i, _)| CommsResponse {
                text: format!("Response {i}"),
                important: false,
            })
            .collect();
        let mut message = msg(&id);
        message.sender_uuid = sender_uuid.to_string();
        message.body = body.to_string();
        message.is_urgent = urgent;
        message.responses = crate::comms::content::response_views(&responses, true);
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(message);
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .active_dialogues
            .insert(
                id.clone(),
                ActiveDialogue {
                    current_node: CommsDialogueNode {
                        body: body.to_string(),
                        responses,
                    },
                    thread_id: "scripted-thread".to_string(),
                    script: crate::comms::content::ScriptedDialogue {
                        script_path: crate::comms::scripted::tests::PATH.to_string(),
                        node_fn: "root".to_string(),
                        on_pick: on_pick.iter().map(|s| s.to_string()).collect(),
                    },
                },
            );
        id
    }

    fn respond(
        app: &mut App,
        message_id: &str,
        response_index: usize,
    ) -> Vec<crate::lobby::OutboundMessage> {
        push_msg(
            app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: message_id.to_string(),
                    response_index,
                },
            },
        );
        tick(app)
    }

    const DIALOGUE_TREE: &str = r#"
        fn on_ack(ctx) {
            ctx.effects.complete_objective("reach_axiom");
            #{ message: "Docking clamps released.", responses: [
                #{ text: "Confirm", on_pick: "on_confirm" },
            ] }
        }
        fn on_decline(ctx) { ctx.effects.fail_objective("reach_axiom"); }
        fn on_confirm(ctx) { }
    "#;

    /// Picking a scripted response runs its `on_pick` fn through the shared
    /// dispatch path and injects the follow-up node the fn returned — the whole
    /// scripted arm, end to end through the live handler.
    #[test]
    fn a_scripted_response_runs_its_on_pick_and_injects_the_follow_up() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456984";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        app.world_mut()
            .insert_resource(crate::comms::scripted::tests::compile_fixture(
                DIALOGUE_TREE,
            ));
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "reach Axiom",
            true,
            vec![],
        );
        let id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Axiom Station, go ahead.",
            vec!["on_ack", "on_decline"],
            false,
        );

        let _ = respond(&mut app, &id, 0);

        assert_eq!(
            app.world()
                .resource::<ObjectiveManagerRes>()
                .0
                .sorted_snapshots()
                .into_iter()
                .find(|o| o.id == "reach_axiom")
                .expect("the objective exists")
                .status,
            crate::messages::ObjectiveStatus::Completed,
            "the on_pick fn's effects must reach the shared apply path"
        );

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        let follow = messages
            .iter()
            .find(|m| m.body == "Docking clamps released.")
            .expect("the follow-up node the on_pick returned must be injected");
        assert_eq!(
            follow.thread_id, "scripted-thread",
            "a follow-up stays in its thread"
        );
        assert_eq!(
            follow.responses.iter().map(|r| &r.text).collect::<Vec<_>>(),
            vec!["Confirm"]
        );
        assert_eq!(
            messages
                .iter()
                .find(|m| m.id == id)
                .expect("the answered message is still in the inbox")
                .selected_response,
            Some(0),
            "the pick is recorded on the message the player answered"
        );

        let comms = app.world().resource::<CommsRuntime>();
        let script = comms
            .active_dialogues
            .get(&follow.id)
            .expect("the follow-up seats its own dialogue")
            .script
            .clone();
        assert_eq!(
            script.node_fn, "on_ack",
            "the fn that produced the shown node is the one recorded"
        );
        assert_eq!(script.on_pick, vec!["on_confirm".to_string()]);
    }

    /// A scripted `on_pick` that returns `()` is a terminal response: its
    /// effects apply and the thread ends with no further message.
    #[test]
    fn a_terminal_scripted_response_ends_the_thread() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456985";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        app.world_mut()
            .insert_resource(crate::comms::scripted::tests::compile_fixture(
                DIALOGUE_TREE,
            ));
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "reach Axiom",
            true,
            vec![],
        );
        let id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Axiom Station, go ahead.",
            vec!["on_ack", "on_decline"],
            false,
        );

        let _ = respond(&mut app, &id, 1);

        assert_eq!(
            app.world()
                .resource::<ObjectiveManagerRes>()
                .0
                .sorted_snapshots()
                .into_iter()
                .find(|o| o.id == "reach_axiom")
                .expect("the objective exists")
                .status,
            crate::messages::ObjectiveStatus::Failed,
        );
        assert_eq!(
            app.world().resource::<CommsInboxRes>().0.messages().len(),
            1,
            "a terminal response injects nothing"
        );
    }

    /// The tick the responding `handle_respond_to_message` will read — what a
    /// budget must be stamped with to belong to THIS tick rather than a stale
    /// one. `advance_sim_tick` runs in `FixedLast`, so the value the handler
    /// sees during a step is the one readable before that step runs; a fixture
    /// with no `SimTick` at all reads 0, exactly as the handler's
    /// `unwrap_or(0)` does.
    fn responding_tick(app: &App) -> u64 {
        app.world()
            .get_resource::<crate::sim_tick::SimTick>()
            .map(|t| t.0)
            .unwrap_or(0)
    }

    /// Issue #1050 / R5: a dialogue call refused by the tick's script budget
    /// must flash the attempted control red, not vanish. The refusal is detected
    /// BEFORE the call, because a refused call produces exactly what a terminal
    /// response with no effects produces.
    ///
    /// The budget is stamped with the RESPONDING tick, so this proves the
    /// refusal on a budget that genuinely belongs to this tick — not on a stale
    /// one the handler's per-tick reset would (and now does) wipe. Before that
    /// reset existed, this arm read the previous tick's budget and a spent one
    /// leaked forward into a spurious rejection; setting `budget_tick` here is
    /// what keeps the test honest about which defect it is asserting.
    #[test]
    fn a_budget_refused_scripted_response_is_rejected() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456986";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let mut sr = crate::comms::scripted::tests::compile_fixture(DIALOGUE_TREE);
        // Spend the tick's whole operation budget: `charge_ops` trips it, and a
        // tripped budget refuses every remaining call — exactly the state a busy
        // tick leaves behind.
        sr.budget.charge_ops(crate::world::script::MAX_OPS_PER_TICK);
        assert!(sr.budget.tripped());
        sr.budget_tick = responding_tick(&app);
        app.world_mut().insert_resource(sr);
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "reach Axiom",
            true,
            vec![],
        );
        let id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Axiom Station, go ahead.",
            vec!["on_ack", "on_decline"],
            false,
        );

        let out = respond(&mut app, &id, 0);

        let (rejected_id, idx) =
            find_rejection(&out).expect("a budget-refused response must be rejected");
        assert_eq!(rejected_id, id);
        assert_eq!(idx, 0);
        assert_eq!(
            app.world()
                .resource::<ObjectiveManagerRes>()
                .0
                .sorted_snapshots()
                .into_iter()
                .find(|o| o.id == "reach_axiom")
                .expect("the objective exists")
                .status,
            crate::messages::ObjectiveStatus::Active,
            "and the refused pick must have applied nothing"
        );
        assert_eq!(
            app.world().resource::<CommsInboxRes>().0.messages().len(),
            1,
            "and injected nothing"
        );
    }

    /// The other half of the budget contract, and the defect the per-tick reset
    /// closes: a budget left tripped by a PREVIOUS tick must not refuse this
    /// tick's pick. This arm runs in `SimSet::Input`, ahead of every `Physics`
    /// script system, so it is the one call site that would otherwise read a
    /// stale budget — and its charges would land on a budget wiped later in the
    /// same tick, leaving live dialogue calls effectively unbudgeted.
    #[test]
    fn a_stale_tripped_budget_does_not_refuse_this_ticks_scripted_response() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456996";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let mut sr = crate::comms::scripted::tests::compile_fixture(DIALOGUE_TREE);
        sr.budget.charge_ops(crate::world::script::MAX_OPS_PER_TICK);
        assert!(sr.budget.tripped());
        // Stamped with a tick that is NOT the responding one: a spent budget
        // belonging to the past.
        sr.budget_tick = responding_tick(&app).wrapping_sub(1);
        app.world_mut().insert_resource(sr);
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "reach Axiom",
            true,
            vec![],
        );
        let id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Axiom Station, go ahead.",
            vec!["on_ack", "on_decline"],
            false,
        );

        let out = respond(&mut app, &id, 0);

        assert!(
            find_rejection(&out).is_none(),
            "last tick's spent budget must not refuse this tick's pick"
        );
        assert_eq!(
            app.world()
                .resource::<ObjectiveManagerRes>()
                .0
                .sorted_snapshots()
                .into_iter()
                .find(|o| o.id == "reach_axiom")
                .expect("the objective exists")
                .status,
            crate::messages::ObjectiveStatus::Completed,
            "the pick runs on a fresh budget"
        );
        // And the charge landed on THIS tick's budget, adopted by the reset.
        let sr = app
            .world()
            .resource::<crate::world::server::WorldScriptRuntime>();
        assert_eq!(sr.budget_tick, responding_tick(&app));
        assert_eq!(sr.budget.calls_used(), 1, "the dialogue call was charged");
    }

    /// Finding 4's immediate half: an `on_pick` naming a fn that does not exist
    /// must be DISTINGUISHABLE from a terminal response. It refuses the pick —
    /// the control flashes red and nothing is recorded — instead of silently
    /// killing the thread (or panicking mid-mission on the `CallError`).
    #[test]
    fn a_scripted_response_whose_on_pick_is_missing_is_rejected() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456997";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        app.world_mut()
            .insert_resource(crate::comms::scripted::tests::compile_fixture(
                DIALOGUE_TREE,
            ));
        let id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Axiom Station, go ahead.",
            vec!["on_typo_never_defined"],
            false,
        );

        let out = respond(&mut app, &id, 0);

        let (rejected_id, idx) =
            find_rejection(&out).expect("an unresolvable on_pick must be rejected");
        assert_eq!(rejected_id, id);
        assert_eq!(idx, 0);
        assert_eq!(
            app.world().resource::<CommsInboxRes>().0.messages()[0].selected_response,
            None,
            "and the response must NOT be recorded as answered"
        );
    }

    /// Finding 3 through the live handler: a malformed return does not un-apply
    /// the work the fn really did. The call succeeded and its buffers drained,
    /// so the objective it completed stays completed — while the pick itself is
    /// still refused, because there is no node to advance to.
    #[test]
    fn a_malformed_on_pick_return_still_applies_the_effects_it_produced() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456998";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        app.world_mut()
            .insert_resource(crate::comms::scripted::tests::compile_fixture(
                r#"
                fn on_ack(ctx) {
                    ctx.effects.complete_objective("reach_axiom");
                    "not a node map"
                }
                "#,
            ));
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "reach Axiom",
            true,
            vec![],
        );
        let id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Axiom Station, go ahead.",
            vec!["on_ack"],
            false,
        );

        let out = respond(&mut app, &id, 0);

        assert_eq!(
            app.world()
                .resource::<ObjectiveManagerRes>()
                .0
                .sorted_snapshots()
                .into_iter()
                .find(|o| o.id == "reach_axiom")
                .expect("the objective exists")
                .status,
            crate::messages::ObjectiveStatus::Completed,
            "the completed objective must survive the malformed return"
        );
        assert!(
            find_rejection(&out).is_some(),
            "and the pick is still refused — there is no node to advance to"
        );
        assert_eq!(
            app.world().resource::<CommsInboxRes>().0.messages()[0].selected_response,
            None,
            "so the response is not recorded either"
        );
    }

    /// Finding 9: answering the same scripted message twice must not re-run its
    /// `on_pick`. The answered node's dialogue entry is retired, so the second
    /// submission takes the stale-submission arm and flashes red instead of
    /// applying the response's effects a second time.
    #[test]
    fn answering_a_scripted_message_twice_does_not_re_run_its_on_pick() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456999";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        app.world_mut()
            .insert_resource(crate::comms::scripted::tests::compile_fixture(
                r#"fn on_ack(ctx) { ctx.flags.increment("acks", 1); }"#,
            ));
        let id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Axiom Station, go ahead.",
            vec!["on_ack"],
            false,
        );

        let _ = respond(&mut app, &id, 0);
        let after_first = app
            .world()
            .resource::<WorldContentRuntime>()
            .flags
            .counter("acks");
        let out = respond(&mut app, &id, 0);

        assert_eq!(after_first, 1, "the first pick ran its on_pick once");
        assert_eq!(
            app.world()
                .resource::<WorldContentRuntime>()
                .flags
                .counter("acks"),
            1,
            "the second submission must not re-run it"
        );
        assert!(
            find_rejection(&out).is_some(),
            "the answered message has no active dialogue any more, so it is refused"
        );
    }

    /// R6, the deliberate divergence: a scripted follow-up INHERITS the urgency
    /// the thread was opened with, where a declarative one hardcodes `false`.
    #[test]
    fn a_scripted_follow_up_inherits_the_threads_urgency() {
        let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456987";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        app.world_mut()
            .insert_resource(crate::comms::scripted::tests::compile_fixture(
                DIALOGUE_TREE,
            ));
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "reach Axiom",
            true,
            vec![],
        );
        let id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Axiom Station, go ahead.",
            vec!["on_ack", "on_decline"],
            true,
        );

        let _ = respond(&mut app, &id, 0);

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        let follow = messages
            .iter()
            .find(|m| m.body == "Docking clamps released.")
            .expect("the follow-up is injected");
        assert!(
            follow.is_urgent,
            "an urgent scripted thread stays urgent as it advances"
        );
    }

    // -- The retired duplicate-arm contract (issues #397 fix 2, #722, #985) ----
    //
    // A battery of `comms_response_dispatches_<variant>` tests used to live
    // here, one per `TriggerAction`, plus an enumeration test that matched on
    // every variant so adding one was a compile error until it was wired into
    // BOTH dispatch sites. They existed because `handle_respond_to_message` had
    // its own dispatch arm beside `tick_trigger_pipeline`'s, and two arms can
    // drift.
    //
    // Issue #985 deleted that arm with the `[[comms]]` front-end that fed it. A
    // response's effects now arrive as `BufferedEffect`s from its `on_pick` fn
    // and go through `apply_script_commands`, which routes a name-resolving
    // effect through the same `dispatch_action` and hands EVERY result to the
    // same `apply_dispatch_result` the trigger pipeline calls. There is one
    // applier and one call into it, so per-variant equivalence is structural
    // rather than something a test can protect — and a new `TriggerAction`
    // variant is wired into one place, which `world::dispatch`'s own
    // enumeration still covers.

    // -- Comms conversation-cycle tests (issue #608, moved from
    // world::server::tests). Cover handle_hail / handle_respond_to_message /
    // handle_clear_comms / handle_comms_channel2 end-to-end via the shared
    // comms_test_app()/setup_game_with_comms() harness (still in
    // world::server::tests, imported above).
    // -- Cycle 1: hail delivers CommsState to comms holder --------------------

    // Cycle 1 used to assert that a hail delivered a `[[comms]] on_hailed`
    // template's message to the Comms holder. Issue #985 deleted that
    // front-end: a hail now records itself and emits `WorldEvent::Hailed`, and
    // what answers it is a scripted `on_hailed` handler
    // (`comms::scripted::tests::a_hail_on_a_script_free_world_delivers_nothing`
    // pins the empty case; the `default_worlds_hail_*` tests pin the answered
    // one). The two cycles below are unchanged in intent — they assert a hail
    // that must NOT be admitted leaves no trace — but they read that off
    // `open_hails`, which `handle_hail` writes, rather than off an inbox
    // nothing fills any more.

    // -- Cycle 2: hail from non-Comms player is ignored -----------------------

    #[test]
    fn hail_from_non_comms_player_is_ignored() {
        let station_uuid = "station-uuid-002";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "captain",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.into(),
                },
            },
        );
        let _ = tick(&mut app);

        assert!(
            app.world().resource::<CommsRuntime>().open_hails.is_empty(),
            "a non-Comms player's hail must not be admitted, so nothing is recorded"
        );
    }

    #[test]
    fn hail_blocked_when_comms_system_ai_controlled() {
        let station_uuid = "station-uuid-ai-block";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let _ = tick(&mut app);

        // Set comms system to AI control (blocks human input).
        {
            let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::simulation::Ship>>();
            for mut sources in q.iter_mut(app.world_mut()) {
                sources.0.set(
                    crate::system_registry::comms_system_id(),
                    crate::control_source::ControlSource::Ai,
                );
            }
        }

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
            app.world().resource::<CommsRuntime>().open_hails.is_empty(),
            "a human hail must be blocked when the comms system is AI-controlled"
        );
    }

    // -- Cycle 4: clear comms removes read/orphaned messages ------------------

    #[test]
    fn clear_comms_removes_orphaned_messages_and_broadcasts_update() {
        let station_uuid = "station-uuid-004";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        let _ = tick(&mut app);

        // Inject an orphaned message directly.
        let orphaned = CommsMessage {
            id: "orphaned-001".into(),
            sender_uuid: station_uuid.into(),
            sender_name: "Starbase Alpha".into(),
            subject: "Old message".into(),
            body: "Old message body".into(),
            responses: vec![],
            selected_response: None,
            is_read: false,
            is_orphaned: true,
            sender_in_range: true,
            thread_id: "orphaned-001".into(),
            is_urgent: false,
        };
        // Orphan it before injection so clear() will remove it.
        app.world_mut()
            .resource_mut::<CommsInboxRes>()
            .0
            .inject(orphaned);
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::ClearComms,
            },
        );
        let out = tick(&mut app);

        let comms_state = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { messages, .. } = &m.msg {
                Some(messages.clone())
            } else {
                None
            }
        });
        assert!(
            comms_state.is_some(),
            "CommsState expected after ClearComms"
        );
        let messages = comms_state.unwrap();
        assert!(
            messages.iter().all(|m| !m.is_orphaned),
            "all orphaned messages must be cleared"
        );
    }

    // -- Cycle 5: initial CommsState with contacts sent on game start ---------

    #[test]
    fn initial_comms_state_includes_contacts_from_scenario() {
        let station_uuid = "station-uuid-005";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);

        let out = tick(&mut app);

        let contacts = out.iter().find_map(|m| {
            if let ServerMessage::CommsState { contacts, .. } = &m.msg {
                Some(contacts.clone())
            } else {
                None
            }
        });
        assert!(
            contacts.is_some(),
            "initial CommsState with contacts expected"
        );
        let contacts = contacts.unwrap();
        assert!(
            contacts.iter().any(|c| c.uuid == station_uuid),
            "station must appear as a contact"
        );
    }

    // Cycles 6-9 covered the declarative front-end's own shapes: the thread id
    // a hail minted, a `[[comms.response]] follow_up` inheriting its parent's
    // thread, a follow-up's `speaker` override, and the `...` placeholder a
    // TRIGGERED follow-up seated while it waited. Issue #985 deleted all four
    // with the parser that authored them. The scripted analogues that replace
    // them are above: `a_scripted_response_runs_its_on_pick_and_injects_the_follow_up`
    // (the follow-up node, in its parent's thread),
    // `a_terminal_scripted_response_ends_the_thread`, and
    // `a_scripted_follow_up_inherits_the_threads_urgency`. A DELAYED reply is
    // no longer a queued node with a placeholder at all — it is
    // `ctx.schedule.after(n, |ctx| ctx.effects.open_comms(#{thread_id: ..}))`,
    // covered in `comms::scripted`.

    /// A Hail targeting an out-of-range entity must NOT be recorded as a hail
    /// (server-side enforcement; stale clients can't bypass the client gate).
    #[test]
    fn server_rejects_hail_when_target_out_of_range() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-out-of-range-hail";
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
            Transform::from_xyz(5000.0, 0.0, 0.0),
            CommsRange(100.0),
        ));

        // Flush initial broadcast so range_flags is populated.
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
            app.world().resource::<CommsRuntime>().open_hails.is_empty(),
            "an out-of-range Hail must not pass the range gate, so nothing is \
             recorded and no `WorldEvent::Hailed` reaches a handler"
        );
    }

    /// A `RespondToMessage` whose dialogue sender is out of range must NOT run
    /// the picked response's `on_pick` fn.
    #[test]
    fn server_rejects_respond_when_sender_out_of_range() {
        use crate::comms::CommsRange;
        use crate::entities::spawner::EntityUuid;
        use crate::simulation::Ship;

        let station_uuid = "station-respond-oor";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        app.world_mut()
            .insert_resource(crate::comms::scripted::tests::compile_fixture(
                DIALOGUE_TREE,
            ));
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "reach Axiom",
            true,
            vec![],
        );

        // Start in range, seat the thread, then move the station far away and
        // respond.
        app.world_mut().spawn((
            Ship,
            crate::simulation::LocalShip,
            Transform::from_xyz(0.0, 0.0, 0.0),
            CommsRange(500.0),
        ));
        let station_entity = app
            .world_mut()
            .spawn((
                EntityUuid(station_uuid.into()),
                Transform::from_xyz(50.0, 0.0, 0.0),
                CommsRange(500.0),
            ))
            .id();
        let _ = tick(&mut app);

        let msg_id = seat_scripted_dialogue(
            &mut app,
            station_uuid,
            "Axiom Station, go ahead.",
            vec!["on_ack", "on_decline"],
            false,
        );

        // Move the station far away.
        if let Ok(mut e) = app.world_mut().get_entity_mut(station_entity) {
            e.insert(Transform::from_xyz(5000.0, 0.0, 0.0));
        }
        // Tick to refresh range_flags.
        let _ = tick(&mut app);

        // Try to respond.
        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::RespondToMessage {
                    message_id: msg_id.clone(),
                    response_index: 0,
                },
            },
        );
        let _ = tick(&mut app);

        // `on_ack` completes `reach_axiom`; refused, it must still be Active.
        assert_eq!(
            app.world()
                .resource::<ObjectiveManagerRes>()
                .0
                .sorted_snapshots()
                .into_iter()
                .find(|o| o.id == "reach_axiom")
                .expect("the objective exists")
                .status,
            crate::messages::ObjectiveStatus::Active,
            "out-of-range Respond must not run the response's on_pick fn"
        );
    }

    #[test]
    fn control_system_hail_dispatches_same_as_client_message_hail() {
        let station_uuid = "station-uuid-control-sys";
        let mut app = comms_test_app();
        setup_game_with_comms(&mut app, station_uuid);
        // Flush the initial broadcast.
        let _ = tick(&mut app);

        push_msg(
            &mut app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: station_uuid.to_string(),
                },
            },
        );
        let _ = tick(&mut app);

        // The observable effect of a hail reaching `handle_hail` is the
        // authoritative record it writes. (It used to be the `[[comms]]`
        // template message the hail injected; issue #985 deleted that
        // front-end — what answers a hail now is a scripted `on_hailed`
        // handler, and this test is about the COMMAND arriving.)
        assert!(
            app.world()
                .resource::<CommsRuntime>()
                .open_hails
                .contains(station_uuid),
            "ControlSystem::Hail must reach handle_hail and be recorded"
        );
    }
}
