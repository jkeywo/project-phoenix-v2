//! Server-side Comms console plugin (issue #427, migrated to blackboard #565).
//!
//! Issue #608: the comms conversation engine (hail / respond / clear /
//! show-on-screen / channel-2 handlers) lives here, alongside the
//! blackboard-publish system, so changing comms behaviour no longer
//! requires reaching into the world module.

use crate::ship_plugin::ShipSystemControlSources;
use bevy::prelude::*;

use crate::core::messages::{CommsBlackboard, ObjectiveSnapshot, SystemBlackboard, SystemId};
use crate::world::server::{ObjectiveManagerRes, WorldContentRuntime};

use crate::comms::content::ActiveDialogue;
use crate::comms::server::{
    current_sender_in_range, CommsChannel2Event, CommsInboxRes, CommsRuntime, OnScreenMessage,
};
use crate::core::messages::{CommsMessage, GamePhase};
use crate::entities::spawner::EntityUuid;
use crate::world::content::WorldEvent;
use crate::world::server::{EffectQueues, ShipModifiersParams, WorldLayerParams};

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
            Option<&crate::ship::state::ShipRedAlert>,
            Option<&crate::entities::spawner::EntitySystemHull>,
        ),
        With<crate::server_app::LocalShip>,
    >,
    mut ship_q: Query<
        (
            bevy::ecs::query::Has<crate::server_app::LocalShip>,
            &mut crate::server_app::ShipSystemBlackboards,
            // Where the human seek landed comms, if anywhere (issue #984).
            // `Option` because only `LocalShip` carries the component, and
            // optional access filters no archetype — the matched set, and so
            // the iteration order, is exactly what it was.
            Option<&crate::ship::components::HumanSeekingHosts>,
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
        contact.is_urgent = messages.iter().any(|m| {
            m.sender_uuid == contact.uuid && m.effective_priority().is_urgent() && !m.is_read
        });
    }

    let local_bb = CommsBlackboard {
        messages,
        objectives: objectives_snap,
        contacts,
        // Filled per ship below: the seek is a property of the hull, not of the
        // shared player-channel content this local blackboard is built from.
        host_station: None,
    };

    let comms_key = SystemId(crate::ship::system_registry::COMMS_SYSTEM_ID.to_string());
    for (is_local, mut bbs, hosts) in ship_q.iter_mut() {
        let mut bb = if is_local {
            local_bb.clone()
        } else {
            CommsBlackboard::default()
        };
        bb.host_station = hosts.and_then(|h| h.0.get(&comms_key).cloned());
        bbs.0.insert(comms_key.clone(), SystemBlackboard::Comms(bb));
    }
}

// ── Comms conversation handlers (issue #608, moved from world/server.rs) ──────

/// Handle `Hail { target_uuid }` messages from Comms console holders.
///
/// Range-gates the hail, records it on `CommsRuntime::open_hails`, and emits
/// `WorldEvent::Hailed` so a scripted `on_hailed` handler can answer it.
pub(crate) fn handle_hail(
    ship_query: Query<&crate::core::messages::AdmittedCommands, With<crate::server_app::LocalShip>>,
    mut runtime: ResMut<WorldContentRuntime>,
    mut comms: ResMut<CommsRuntime>,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::ship::system_registry::COMMS_SYSTEM_ID) {
        let target_uuid = match &cmd.payload {
            crate::core::messages::SystemControlPayload::Hail { target_uuid } => target_uuid,
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
    outbox: ResMut<'w, crate::server_app::SimOutbox>,
    /// The tick-scoped id mint (issue #907): spawned-entity ids for the shared
    /// dispatch pass, and the follow-up message ids minted below it. Replaces
    /// the seeded `SimRng` this bundle used to carry — nothing in this system
    /// draws a random number any more, it only mints identities.
    id_mint: Option<Res<'w, crate::world_id::WorldIdMint>>,
    balance_events:
        Option<ResMut<'w, bevy::ecs::message::Messages<crate::core::balance::BalanceEvent>>>,
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
            &crate::core::messages::AdmittedCommands,
            // The comms rejection channel is addressed to whoever is HOSTING
            // the comms system (issue #984), which on the destroyer is the
            // Tactical seat and on the courier the Captain's — resolved, never
            // string-cast from the system id.
            Option<&crate::ship_plugin::ShipConfigComponent>,
            Option<&crate::ship_plugin::HumanSeekingHosts>,
        ),
        With<crate::server_app::LocalShip>,
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
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<crate::entities::spawner::BehaviourSection>,
    >,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::server_app::GameOverReason>>,
    mut world_layers: WorldLayerParams,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
    mut faction_dispatch: crate::world::server::FactionDispatchParams,
    // Bundled to stay within Bevy's 16-argument system limit: the seeded RNG
    // and balance-event ledger the dispatch pass needs, plus the issue-#761
    // rejection-feedback seam (`Sessions` + `SimOutbox`) addressed to the
    // submitting Comms holder.
    mut aux: CommsRespondAux,
    // The per-owner effect queues an `on_pick` script pushes onto (issue #1223),
    // the same sinks the trigger/callback paths use.
    mut effect_queues: EffectQueues,
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
                &crate::ship::system_registry::comms_system_id(),
            )
        })
        .unwrap_or_else(|| {
            crate::core::messages::StationId(crate::ship::system_registry::COMMS_SYSTEM_ID.into())
        });
    let comms_token = aux
        .sessions
        .0
        .holder_for_station(&comms_station)
        .map(|t| t.to_string());
    // Helper: push a `CommsResponseRejected` for the attempted control so the
    // client can flash it red. A no-op when no comms holder is seated.
    let reject = |outbox: &mut crate::server_app::SimOutbox, message_id: &str, idx: usize| {
        if let Some(token) = comms_token.as_deref() {
            outbox.push_reliable((
                crate::lobby::Target::Token(token.to_string()),
                crate::core::messages::ServerMessage::CommsResponseRejected {
                    message_id: message_id.to_string(),
                    response_index: idx,
                },
            ));
        }
    };
    for cmd in admitted.for_target(crate::ship::system_registry::COMMS_SYSTEM_ID) {
        let (message_id, response_index) = match &cmd.payload {
            crate::core::messages::SystemControlPayload::RespondToMessage {
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
        // order its trigger-authored twin would. A scripted comms thread carries
        // its originating sub-world layer, so effects and flag writes remain in
        // the scope of the handler which opened it. This is
        // a single-shot dispatch, not a chaining pass: `new_events` are routed
        // onto `runtime.pending_world_events` so `tick_trigger_pipeline` (later
        // in the same tick via `SimSet::Physics`, since this handler runs in
        // `SimSet::Input`) picks them up and fires any chained
        // `on_flag_set` / `on_flag_cleared` / `on_destroyed` triggers.
        let origin_layer = dialogue.script.origin_layer.clone();
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
        let template_loader = crate::entities::loader::WasmTemplateLoader;
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
        let dialogue_flag_chain: Vec<crate::world::flags::FlagStore> =
            crate::world::server::layered_flag_chain(
                origin_layer.as_deref(),
                &runtime.flags,
                world_layers.layer_map.as_deref(),
            )
            .into_iter()
            .cloned()
            .collect();
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
                Some(ast) => Some(crate::world::script::comms::enter_node_scoped(
                    host,
                    budget,
                    &script_clock,
                    ast,
                    &sd.script_path,
                    &on_pick_fn,
                    &dialogue_flag_chain,
                    &runtime.deadlines,
                    &runtime.commitments,
                    &runtime.evidence,
                    origin_layer.as_deref(),
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
            &mut effect_queues.out(),
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
        // And a beat that gave the captain's word, or settled it (issue #1029).
        crate::world::server::apply_commitment_changes(
            &effects.commitment_changes,
            &mut runtime.commitments,
            script_clock.tick,
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
        // above instead and flashes red. (`handle_clear_comms` also retires
        // every UNANSWERED entry on an explicit clear — issue #1049.)
        comms.active_dialogues.remove(message_id);

        // Advance to the follow-up node the `on_pick` fn returned. `None` is
        // a terminal response and ends the thread.
        if let Some(node) = follow_up {
            let thread_id = dialogue.thread_id.clone();
            let sender_uuid = inbox.0.sender_uuid_for(message_id).unwrap_or_default();
            let sender_name = inbox.0.sender_name_for(message_id).unwrap_or_default();
            // R6: a follow-up inherits legacy urgency, while a response
            // acknowledges generic Critical state. A later authored OPEN may
            // raise the thread again. (The
            // declarative arm deleted in issue #985 hardcoded `urgent: false`
            // here, because follow-up urgency was not a TOML-level concept.)
            let priority = inbox
                .0
                .priority_for(message_id)
                .unwrap_or_default()
                .after_response();
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
                wire_node.body_params.clone(),
                new_responses,
                thread_id.clone(),
                available,
                priority,
            );
            channel2_writer.write(CommsChannel2Event::scripted_dialogue(new_msg));
            comms.active_dialogues.insert(
                new_msg_id,
                ActiveDialogue {
                    current_node: wire_node,
                    thread_id,
                    script: crate::comms::content::ScriptedDialogue {
                        script_path: sd.script_path.clone(),
                        origin_layer: sd.origin_layer.clone(),
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
/// # `active_dialogues` is HARD-cleared (issue #1049)
///
/// Clearing empties `CommsRuntime::active_dialogues` alongside the inbox and
/// `open_hails` — a full clear, not a selective prune. That is safe because
/// nothing consumes an `active_dialogues` entry except by looking it up
/// through `message_id` and treating a miss as an ordinary, already-handled
/// case, never as an invariant violation:
///
///   * [`handle_respond_to_message`] is the only path that can advance a
///     dialogue, and it already has a "stale submission" arm for a missing
///     entry (already answered, never existed, or now cleared) — it rejects
///     with `CommsResponseRejected` so the client flashes the attempted
///     control red. A clear simply routes every open dialogue through the
///     SAME arm a late/duplicate submission already takes.
///   * [`operate_comms_response_ai`] skips any inbox message with no
///     `active_dialogues` entry — it never decides about a thread that isn't
///     there, so a clear just leaves it nothing to decide about.
///
/// There is also no PENDING server-side continuation that a hard clear could
/// orphan. A scripted dialogue's only deferred work — `in_seconds` effects
/// (`pending_delayed_actions`) and `after` callbacks (`pending_callbacks`) —
/// never re-enters an existing `active_dialogues` entry by id; when one of
/// them eventually queues a follow-up `open_comms` request,
/// `open_scripted_comms_threads` mints a BRAND NEW message id and inserts a
/// fresh entry (`comms/scripted.rs`), exactly as a live `on_pick`'s own
/// follow-up node does (`handle_respond_to_message` below). Neither writer
/// ever mutates or resurrects a cleared entry, so there is nothing to cancel
/// and nothing selective to preserve — a full clear and a "prune only the
/// entries with no pending effect" clear are the same operation here.
pub(crate) fn handle_clear_comms(
    ship_query: Query<&crate::core::messages::AdmittedCommands, With<crate::server_app::LocalShip>>,
    mut inbox: ResMut<CommsInboxRes>,
    mut comms: ResMut<CommsRuntime>,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::ship::system_registry::COMMS_SYSTEM_ID) {
        if matches!(
            cmd.payload,
            crate::core::messages::SystemControlPayload::ClearComms
        ) {
            // Clearing invalidates every live dialogue. Preserve its historical
            // row when it is still unread, but acknowledge any continuing
            // Critical interruption before the dialogue authority disappears.
            let dialogue_ids: Vec<String> = comms.active_dialogues.keys().cloned().collect();
            for message_id in dialogue_ids {
                inbox.0.acknowledge_priority(&message_id);
            }
            inbox.0.clear();
            comms.open_hails.clear();
            comms.active_dialogues.clear();
        }
    }
}

/// Handle `ShowOnScreen { message_id }` from Comms console holders.
///
/// Looks up the message in the inbox, stores it in `OnScreenMessage`, and
/// pushes `ViewMode::Comms` so the viewscreen switches to the comms overlay.
pub(crate) fn handle_show_on_screen(
    ship_query: Query<&crate::core::messages::AdmittedCommands, With<crate::server_app::LocalShip>>,
    inbox: Res<CommsInboxRes>,
    mut on_screen: ResMut<OnScreenMessage>,
    mut view_mode_q: Query<
        &mut crate::ship::state::ShipViewMode,
        With<crate::server_app::LocalShip>,
    >,
) {
    let Some(admitted) = ship_query.iter().next() else {
        return;
    };
    let Some(mut vm) = view_mode_q.iter_mut().next() else {
        return;
    };
    for cmd in admitted.for_target(crate::ship::system_registry::COMMS_SYSTEM_ID) {
        let show_message_id: Option<&String> = match &cmd.payload {
            crate::core::messages::SystemControlPayload::ShowOnScreen { message_id } => {
                Some(message_id)
            }
            _ => None,
        };
        if let Some(message_id) = show_message_id {
            if let Some(msg) = inbox.0.messages().into_iter().find(|m| &m.id == message_id) {
                let already_on_screen =
                    matches!(vm.view_mode, crate::core::messages::ViewMode::Comms)
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
                    if !matches!(vm.view_mode, crate::core::messages::ViewMode::Comms) {
                        vm.show_view_mode(crate::core::messages::ViewMode::Comms);
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
    comms: Res<CommsRuntime>,
) {
    for ev in reader.read() {
        if ev.delivery == crate::comms::server::CommsChannel2Delivery::ScriptedDialogue
            && !comms.active_dialogues.contains_key(&ev.message.id)
        {
            // The authoritative dialogue was retired after its presentation
            // event was queued (layer unload, ClearComms, or response race).
            // Skip this one event in place; never drain/rewrite the shared
            // Messages buffer, which would mint new IDs and replay survivors to
            // readers that already consumed them.
            continue;
        }
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
    config: &crate::entities::config::EntityConfig,
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
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    facts.set_fact(fid::SOURCE_HAIL_OBJECTIVE, b(reading.source_hail_objective));
    facts.set_fact(fid::SOURCE_COMMS_CONTACT, b(reading.source_comms_contact));
    facts.set_fact(fid::OBJECTIVE_SCORE, reading.objective_score as f64);
    facts.set_fact(fid::IN_RANGE, b(reading.in_range));
    facts.set_fact(fid::IS_URGENT, b(reading.is_urgent));
    facts.set_fact(fid::HAS_OPEN_HAIL_THREAD, b(reading.has_open_hail_thread));
    facts.set_fact(
        fid::HAS_UNREAD_FROM_SENDER,
        b(reading.has_unread_from_sender),
    );
    facts.set_fact(fid::MANDATORY, b(reading.mandatory));
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
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    facts.set_fact(
        fid::COMMS_AVAILABLE,
        if comms_available { 1.0 } else { 0.0 },
    );
    facts.set_fact(fid::RED_ALERT, if red_alert { 1.0 } else { 0.0 });
    facts.set_fact(fid::CONTACT_COUNT, contact_count as f64);
    if let Some(pr) = power_rating {
        facts.set_fact(fid::POWER_RATING, pr as f64);
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
    use crate::entities::ai_flag_hosts as fid;
    let mut facts = crate::world::flags::AiFacts::new();
    let b = |v: bool| if v { 1.0 } else { 0.0 };
    facts.set_fact(fid::RESPONSE_COUNT, reading.response_count as f64);
    facts.set_fact(
        fid::AVAILABLE_RESPONSE_COUNT,
        reading.available_response_count as f64,
    );
    facts.set_fact(
        fid::IMPORTANT_RESPONSE_COUNT,
        reading.important_response_count as f64,
    );
    facts.set_fact(fid::IS_URGENT, b(reading.is_urgent));
    facts.set_fact(fid::IS_READ, b(reading.is_read));
    facts.set_fact(fid::IS_ORPHANED, b(reading.is_orphaned));
    facts.set_fact(fid::SENDER_IN_RANGE, b(reading.sender_in_range));
    facts.set_fact(fid::RED_ALERT, b(reading.red_alert));
    facts.set_fact(fid::COMMS_AVAILABLE, b(reading.comms_available));
    if let Some(pr) = reading.power_rating {
        facts.set_fact(fid::POWER_RATING, pr as f64);
    }
    facts
}

/// Whether the ship's own Comms fine system is usable this tick (AC2).
///
/// Reads the authoritative per-system hull: Disabled and Destroyed count as
/// unavailable, Operational and Damaged as available. Ships with no hull
/// tracker (bare-`App` fixtures, entities that never took damage modelling) are
/// treated as available, matching the other AI hosts' hull fallbacks.
fn comms_system_available(hull: Option<&crate::entities::spawner::EntitySystemHull>) -> bool {
    let Some(hull) = hull else {
        return true;
    };
    !matches!(
        hull.0
            .tier_for(&crate::ship::system_registry::comms_system_id()),
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

/// The read-only comms context `operate_comms_ai` reads besides the
/// [`crate::ai::host::AiHostEnv`], bundled as one `SystemParam` (issue #1185):
/// the objective pool, the comms runtime, the inbox, the session table, and the
/// log filter.
///
/// A signature grouping only — every field keeps its type and optionality;
/// `comms` stays `ResMut` for the one write this system makes (retiring the
/// `open_hails` candidacy latch), so the access set is byte-for-byte unchanged.
/// The system destructures it back to its original locals at entry.
#[derive(bevy::ecs::system::SystemParam)]
pub struct CommsAiContext<'w> {
    objectives: Option<Res<'w, ObjectiveManagerRes>>,
    comms: Option<ResMut<'w, CommsRuntime>>,
    inbox: Option<Res<'w, CommsInboxRes>>,
    sessions: Res<'w, crate::lobby::Sessions>,
    log: Option<Res<'w, crate::logging::LogFilterConfig>>,
}

/// The read-only comms context `operate_comms_response_ai` reads besides the
/// [`crate::ai::host::AiHostEnv`], bundled as one `SystemParam` (issue #1185):
/// the comms runtime, the inbox, the session table, the log filter, and the
/// shared AI base cadence (raw tick + interval).
///
/// A signature grouping only — every field keeps its type and `Option` fallback
/// (`comms` here is a plain `Res`, unlike [`CommsAiContext`]'s `ResMut`), so the
/// access set is byte-for-byte unchanged; the system destructures it back to its
/// original locals at entry.
#[derive(bevy::ecs::system::SystemParam)]
pub struct CommsResponseContext<'w> {
    comms: Option<Res<'w, CommsRuntime>>,
    inbox: Option<Res<'w, CommsInboxRes>>,
    sessions: Res<'w, crate::lobby::Sessions>,
    log: Option<Res<'w, crate::logging::LogFilterConfig>>,
    tick: Option<Res<'w, crate::sim_tick::SimTick>>,
    base_interval: Option<Res<'w, crate::ai::cadence::AiBaseInterval>>,
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
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    // The objective pool, comms runtime + inbox, session table, and log filter
    // bundled as one `SystemParam` (issue #1185); `comms` stays `ResMut` for the
    // AC5 `open_hails` retirement. See [`CommsAiContext`].
    context: CommsAiContext,
    mut ships: Query<
        (
            Entity,
            Option<&EntityUuid>,
            &ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &mut crate::core::messages::AdmittedCommands,
            Option<&crate::ship::state::ShipRedAlert>,
            Option<&crate::entities::spawner::EntitySystemHull>,
            Option<&CommsTargetSelector>,
        ),
        With<crate::server_app::LocalShip>,
    >,
) {
    // Restore the pre-#1185 locals so the body below is byte-for-byte unchanged.
    let CommsAiContext {
        objectives,
        mut comms,
        inbox,
        sessions,
        log,
    } = context;

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
        // Control-Source gate through the shared AI host spine (issue #1208): a
        // human holder (or an offline system) stands the hail selector down.
        // Comms hail selection resolves a data-driven SELECTOR the spine does not
        // model, so only its gate — the one step it shares with the policy hosts —
        // routes here.
        if !crate::ai::host::ai_operates(
            &sources.0,
            crate::ship::system_registry::comms_system_id(),
        ) {
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
        let flag_chain = ai_env.flag_chain(entity);

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
                    .contains(&crate::core::messages::SystemAffinity::Comms)
                {
                    continue;
                }
                let crate::core::messages::AiDirective::Hail { target } = &scored.directive else {
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
                let Some(uuid) =
                    resolve_hail_target(target, Some(ai_env.content_runtime()), comms.as_deref())
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
            crate::ship::system_registry::comms_system_id(),
            crate::core::messages::SystemControlPayload::Hail {
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
    // The read-only AI-host world context — flag chain, sessions, and origin
    // stamps — behind one bare-`Res` system param (issue #1207). A fixture that
    // runs this host must register it (`register_ai_host_env`) or fail loudly at
    // schedule build, so a bare `App` cannot silently diverge from production.
    ai_env: crate::ai::host::AiHostEnv,
    // The comms runtime + inbox, session table, log filter, and shared AI base
    // cadence (tick + interval) bundled as one `SystemParam` (issue #1185). See
    // [`CommsResponseContext`].
    context: CommsResponseContext,
    mut ships: Query<
        (
            Entity,
            Option<&EntityUuid>,
            &ShipSystemControlSources,
            Option<&crate::ship_plugin::ShipConfigComponent>,
            &mut crate::core::messages::AdmittedCommands,
            Option<&crate::ship::state::ShipRedAlert>,
            Option<&crate::entities::spawner::EntitySystemHull>,
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
    // Restore the pre-#1185 locals so the body below is byte-for-byte unchanged.
    let CommsResponseContext {
        comms,
        inbox,
        sessions,
        log,
        tick,
        base_interval,
    } = context;

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
        // The Control-Source gate (AC5 human exclusivity — a human Comms officer
        // answers their own dialogues) and the strict AI-declaration check (no
        // `[comms_console.ai]` ⇒ the ship answers nothing) now live in the shared
        // `decide` spine, evaluated per message below (issue #1208). The policy is
        // borrowed there straight from the optional component.
        let policy = policy_comp.map(|p| &p.0);
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
        let flag_chain = ai_env.flag_chain(entity);
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
                is_urgent: message.effective_priority().is_urgent(),
                is_read: message.is_read,
                is_orphaned: message.is_orphaned,
                sender_in_range,
                red_alert,
                comms_available,
                power_rating,
            };
            let facts = seed_comms_response_facts(&reading);

            // Gate → declare → resolve the `respond` channel through the shared AI
            // host spine (issue #1208). Only the response verb ever resolves on
            // this channel (the policy is validated to carry no other), so this
            // `let else` is exhaustive in practice; a human holder
            // (`NotAiOperated`), an undeclared ship (`Undeclared`) or a no-rule
            // tick (`Held`) all leave the dialogue open this tick.
            let tick = crate::ai::host::HostTick {
                system: crate::ship::system_registry::comms_system_id(),
                channel: crate::entities::config::COMMS_RESPOND_CHANNEL,
                facts: &facts,
                flags: &flag_chain,
                state: None,
            };
            let crate::ai::host::HostOutcome::Act(
                crate::ai::policy::AiPolicyVerb::RespondToMessage(index),
            ) = crate::ai::host::decide(&sources.0, policy, &tick)
            else {
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
                crate::ship::system_registry::comms_system_id(),
                crate::core::messages::SystemControlPayload::RespondToMessage {
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
#[path = "server_tests.rs"]
mod tests;
