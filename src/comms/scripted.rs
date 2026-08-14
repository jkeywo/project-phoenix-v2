//! Scripted comms threads reaching the console (issue #984).
//!
//! The materialising half of `ctx.effects.open_comms(#{…})`: a scripted handler
//! (or a deferred callback) buffers an
//! [`OpenCommsRequest`](crate::comms::content::OpenCommsRequest) onto
//! [`WorldScriptRuntime::pending_comms_opens`], and
//! [`open_scripted_comms_threads`] drains that queue, enters the thread's root
//! node, and injects the resulting message into the inbox as a channel-2
//! delivery — the SAME delivery a fired `[[comms]]` template makes.
//!
//! It lives in its own file, not in `comms::server`, for two reasons: the M7
//! collapse deletes the declarative front-end and this module is what survives,
//! so the boundary is already drawn; and `comms::server` is the applier for the
//! *declarative* evaluators, which this path does not touch at all.
//!
//! # Disjoint from the declarative path
//!
//! A scripted thread and a declarative one share the inbox, the thread ids, the
//! range gate, and [`ActiveDialogue`] — but nothing else. A declarative
//! dialogue's `script` field is `None` and this system never writes one; a
//! scripted dialogue's node carries `actions: []` / `follow_up: None` by
//! construction (see
//! [`project_node`](crate::world::script::comms::project_node)), so every
//! declarative reader sees the shape it always saw. A world with no
//! `WorldScriptRuntime` — every shipped world today — returns from this system
//! before touching anything, so its digest is unchanged by construction.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::comms::content::{response_views, ActiveDialogue, ScriptedDialogue};
use crate::comms::server::{current_sender_in_range, CommsChannel2Event, CommsRuntime};
use crate::entity_spawner::EntityUuid;
use crate::messages::{CommsMessage, GamePhase};
use crate::world::content::WorldEvent;
use crate::world::script::comms::{enter_node, project_node, EnterError};
use crate::world::script::schedule::{SchedClock, TickBudget};
use crate::world::server::{
    apply_script_commands, ObjectiveManagerRes, ScriptRuntimeParams, ShipModifiersParams,
    WorldContentRuntime, WorldLayerParams, WorldScriptRuntime,
};

/// The three tick-scoped reads [`open_scripted_comms_threads`] needs that are
/// not already bundled, grouped so the system stays comfortably under Bevy's
/// 16-parameter cap.
///
/// `id_mint` is command-addressing surface (issue #907 AC2) — a recorded
/// `RespondToMessage { message_id, .. }` resolves against `active_dialogues`, so
/// a peer that minted the id differently could not replay the command — and is
/// also the entity mint a scripted `spawn_entity` draws from. `time` anchors the
/// mission clock a dialogue fn's `in_seconds`/`after` work is stamped against,
/// and `balance_events` is the ledger the shared apply path writes.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct ScriptedCommsAux<'w> {
    id_mint: Option<Res<'w, crate::world_id::WorldIdMint>>,
    balance_events: Option<ResMut<'w, bevy::ecs::message::Messages<crate::balance::BalanceEvent>>>,
    time: Option<Res<'w, bevy::time::Time>>,
}

/// Materialise every queued `open_comms` request into a live comms thread
/// (issue #984).
///
/// Registered by `CommsWorldPlugin` in `SimSet::Physics`, ordered
/// `.after(tick_script_callbacks)` (so a request queued by a trigger handler OR
/// by a deferred callback is materialised on the tick it was made) and
/// `.before(tick_delayed_actions)` (so a dialogue fn's own `in_seconds` effect
/// reaches the delayed queue before that queue is drained, exactly as a
/// trigger's or a callback's does — without that edge the two systems would be
/// unordered on `WorldContentRuntime` and a zero-delay effect would fire this
/// tick or next depending on the executor).
///
/// Per request, in queue order:
///
/// 1. resolve the sender UUID from `name_to_uuid` by the SAME rule
///    `inject_comms_templates` uses, including the synthetic-sender escape (an
///    unresolvable `from` falls through to itself, which
///    [`current_sender_in_range`] treats as always-readable);
/// 2. enter the root node under the tick's SHARED [`TickBudget`], gated on a
///    pre-flight [`can_admit`](TickBudget::can_admit) check;
/// 3. route the call's `commands` through [`apply_script_commands`] with the
///    same `uuid_source` / `template_loader` / anchors bindings the trigger and
///    callback paths bind — so a root fn's `spawn_entity` / `add_objective`
///    resolves through `dispatch_action` identically to its declarative twin;
/// 4. re-queue the call's own deferred work: `delayed` onto
///    `pending_delayed_actions` (dropped when the mission clock is unanchored,
///    the trigger path's rule), `callbacks` onto `pending_callbacks`, and any
///    NESTED `comms_opens` back onto `pending_comms_opens` — drained on the NEXT
///    tick, never re-entrantly, the same rule the callback queue follows;
/// 5. mint the message id, project the node onto the wire shape, write the
///    channel-2 delivery, and record the [`ActiveDialogue`] carrying the
///    [`ScriptedDialogue`] the response handler answers from.
///
/// # Determinism
/// A no-op for every script-free world: no `WorldScriptRuntime` means an early
/// return before any `DerefMut`, so no change-detection tick flips and no
/// resource is written. `state_digest` folds no comms state at all, so a
/// script-free digest is byte-identical by construction. For a scripted world
/// the queue is an ordered `Vec` drained front-to-back, every peer runs the same
/// requests through the same shared budget in the same order, and both minted
/// ids come from the tick-scoped `WorldIdMint`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn open_scripted_comms_threads(
    mut script: ScriptRuntimeParams,
    mut runtime: ResMut<WorldContentRuntime>,
    mut comms: ResMut<CommsRuntime>,
    mut channel2_writer: MessageWriter<CommsChannel2Event>,
    mut objectives: ResMut<ObjectiveManagerRes>,
    mut commands: Commands,
    mut ship_modifiers: ShipModifiersParams,
    mut next_state: Option<ResMut<NextState<GamePhase>>>,
    mut game_over_reason: Option<ResMut<crate::simulation::GameOverReason>>,
    mut world_layers: WorldLayerParams,
    entity_uuid_query: Query<(Entity, &EntityUuid)>,
    mut faction_dispatch: crate::world::server::FactionDispatchParams,
    mut ai_query: Query<
        (
            &EntityUuid,
            Option<&mut crate::weapons_plugin::TacticalRadarSelection>,
            Option<&crate::entities::spawner::FactionComponent>,
        ),
        With<crate::entity_spawner::BehaviourSection>,
    >,
    mut aux: ScriptedCommsAux,
) {
    // `now_tick` before the `WorldScriptRuntime` borrow (disjoint `script` field).
    let now_tick = script.sim_tick.as_ref().map(|t| t.0).unwrap_or(0);
    // Script-free world (or a bare-`App` fixture): nothing to do, and nothing
    // written — the `ResMut` params are fetched but never `DerefMut`'d on this
    // arm, so no change-detection tick flips.
    let Some(sr) = script.runtime.as_deref_mut() else {
        return;
    };
    if sr.pending_comms_opens.is_empty() {
        return;
    }
    // Normally already done this tick by `tick_trigger_pipeline` /
    // `tick_script_callbacks`, both of which precede this system; keyed on the
    // tick so it is idempotent, and repeated here so a fixture that registers
    // only this system still shares ONE budget per tick rather than carrying a
    // stale trip forward.
    if sr.budget_tick != now_tick {
        sr.budget = TickBudget::new();
        sr.budget_tick = now_tick;
    }
    // Taken whole up front: a request this pass produces (a root fn that itself
    // calls `open_comms`) lands on the now-empty queue and is drained NEXT tick,
    // never re-entrantly within this loop.
    let requests = std::mem::take(&mut sr.pending_comms_opens);

    // The clock a dialogue fn's OWN deferred work is stamped against — the same
    // shape `tick_script_callbacks` builds.
    let elapsed_secs = aux.time.as_ref().and_then(|t| {
        runtime
            .mission_clock_anchor_secs
            .map(|loaded| (t.elapsed_secs() - loaded).max(0.0))
    });
    let script_clock = SchedClock {
        tick: now_tick,
        elapsed_secs: elapsed_secs.unwrap_or(0.0),
        tick_hz: world_layers
            .base_world_config
            .as_ref()
            .map(|wc| wc.global.sim_tick_hz)
            .unwrap_or(SchedClock::ZERO.tick_hz),
    };

    let uuid_to_entity: HashMap<String, Entity> = entity_uuid_query
        .iter()
        .map(|(ent, uuid_comp)| (uuid_comp.0.clone(), ent))
        .collect();

    // The name-resolving-effect dispatch context (issue #984 R1), built EXACTLY
    // as `tick_trigger_pipeline` / `tick_script_callbacks` build theirs: the same
    // `mint_id_with(id_mint, Entity)` closure, so a root fn's `spawn_entity` mints
    // inside `dispatch_spawn_entity` from the real `WorldIdMint` in the same order
    // as the declarative twin (never at the effects.rs boundary, never a fallback
    // mint), and the same `WasmTemplateLoader`.
    let empty_anchors: HashMap<String, [f32; 3]> = HashMap::new();
    let template_loader = crate::entity_loader::WasmTemplateLoader;
    let uuid_source = || {
        crate::world_id::mint_id_with(aux.id_mint.as_deref(), crate::world_id::IdNamespace::Entity)
    };

    // Reborrow as a plain `&mut` so `runtime.flags` (the flag overlay base) and
    // `&mut runtime` (the apply path) can be borrowed in sequence — the disjoint
    // field split every script call site uses.
    let runtime = &mut *runtime;

    for req in requests {
        // A spent budget refuses every remaining call this tick by contract, so
        // stop here rather than logging once per request. Deterministic: the trip
        // is a pure function of the tick's call/op sequence, so every peer drops
        // the same tail. The requests are dropped, not re-queued — re-queueing a
        // refused open would let a busy tick push work forward indefinitely.
        //
        // `can_admit()`, not `tripped()`: the call that REACHES the call cap is
        // refused and trips the budget in one step, so a `tripped()` pre-flight
        // passes on a call that is about to be dropped — and the drop would then
        // surface below as the misleading "root fn returned no node".
        if !sr.budget.can_admit() {
            bevy::log::warn!(
                target: crate::logging::LogCat::World.target(),
                "open_scripted_comms_threads: the tick's script budget is spent; \
                 dropping the remaining comms opens"
            );
            break;
        }

        // Sender identity, by `inject_comms_templates`' rule: `_self` is the
        // reserved synthetic internal sender and renders as "Internal Report";
        // the player-facing display name resolves independently of the reference
        // id; and the UUID is keyed on the RAW `from`, so a synthetic sender
        // deliberately falls through to the name itself.
        let channel_name = if req.from == "_self" {
            "Internal Report".to_string()
        } else {
            req.from.clone()
        };
        let sender_uuid = runtime
            .name_to_uuid
            .get(&req.from)
            .cloned()
            .unwrap_or_else(|| req.from.clone());
        // The same three-step fallback `handle_hail` resolves a channel label
        // with: the open's own `display_name` first, then the CONTACT's authored
        // name (so a scripted thread from a known station is labelled the way
        // every other message from that station is), and only then the raw
        // reference id. Without the middle step a scripted open that omitted
        // `display_name` showed the player an internal id where the declarative
        // path showed a name.
        let sender_name = req
            .display_name
            .clone()
            .or_else(|| {
                comms
                    .contacts
                    .iter()
                    .find(|c| c.uuid == sender_uuid)
                    .map(|c| c.name.clone())
            })
            .unwrap_or(channel_name);

        // Enter the root node under the tick's SHARED budget. Split
        // `WorldScriptRuntime` into disjoint field borrows so the one `&self` call
        // takes `&mut budget` and `&ast` at once while `&runtime.flags` (a
        // DISJOINT resource) is the overlay base.
        let entered = {
            let WorldScriptRuntime {
                host, asts, budget, ..
            } = &mut *sr;
            match asts.get(&req.script_path) {
                Some(ast) => Some(enter_node(
                    host,
                    budget,
                    &script_clock,
                    ast,
                    &req.script_path,
                    &req.root_fn,
                    &runtime.flags,
                )),
                None => {
                    bevy::log::warn!(
                        "open_scripted_comms_threads: root fn '{}' names a missing unit '{}'",
                        req.root_fn,
                        req.script_path
                    );
                    None
                }
            }
        };
        let Some(entered) = entered else {
            continue;
        };
        // Three outcomes, three log lines — a malformed return, an unresolvable
        // name and a refused call are different authoring problems, and none of
        // them is the "returned no node" case below.
        let (effects, node, malformed) = match entered {
            Ok((effects, node)) => (effects, node, false),
            Err(EnterError::Shape { effects, message }) => {
                // The call SUCCEEDED and drained its buffers; only the returned
                // value is malformed. Its effects are applied below before the
                // request is abandoned — see `EnterError::Shape`.
                bevy::log::warn!(
                    "open_scripted_comms_threads: root fn '{}' in '{}': {message}",
                    req.root_fn,
                    req.script_path
                );
                (effects, None, true)
            }
            Err(err @ (EnterError::Unresolved | EnterError::Refused)) => {
                bevy::log::warn!(
                    "open_scripted_comms_threads: root fn '{}' in '{}' {err}; \
                     no message injected",
                    req.root_fn,
                    req.script_path
                );
                continue;
            }
        };

        // Effects first, message second: a root fn that both sets a flag and
        // returns a node has its flag applied before the message is delivered,
        // matching the declarative order (a template's trigger actions fire in
        // `tick_trigger_pipeline` before `inject_comms_templates`' delivery is
        // consumed in `SimSet::Broadcast`).
        let mut out_events: Vec<WorldEvent> = Vec::new();
        apply_script_commands(
            effects.commands,
            "open_scripted_comms_threads",
            &mut out_events,
            &uuid_to_entity,
            runtime,
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
            // A comms thread carries no originating sub-world layer (the
            // declarative `CommsTemplate` has no `origin_layer` field either), so
            // every layer-scoped action resolves against the base world. The
            // entity name is the thread's sender, matching the declarative
            // response path's pre-resolved `sender_entity_name`.
            None,
            Some(req.from.clone()),
        );
        runtime.pending_world_events.extend(out_events);
        if elapsed_secs.is_some() {
            runtime.pending_delayed_actions.extend(effects.delayed);
        }
        sr.pending_callbacks.extend(effects.callbacks);
        sr.pending_comms_opens.extend(effects.comms_opens);

        // A malformed return was already logged above; its effects have now been
        // applied, and there is no node to show.
        if malformed {
            continue;
        }

        // A root fn that returned `()` opened no thread — its effects still
        // applied above. Authoring error rather than a supported shape (an open
        // with nothing to show), so it is worth a line in the log. Genuinely a
        // no-node return: a budget refusal cannot reach here (the `can_admit`
        // pre-flight above breaks the loop first).
        let Some(node) = node else {
            bevy::log::warn!(
                "open_scripted_comms_threads: root fn '{}' returned no node; \
                 no message injected",
                req.root_fn
            );
            continue;
        };

        let thread_id = req.thread_id.clone().unwrap_or_else(|| {
            crate::world_id::mint_id_with(
                aux.id_mint.as_deref(),
                crate::world_id::IdNamespace::Message,
            )
        });
        let (wire_node, on_pick) = project_node(&node);
        let msg_id = crate::world_id::mint_id_with(
            aux.id_mint.as_deref(),
            crate::world_id::IdNamespace::Message,
        );
        let available = current_sender_in_range(&comms, &sender_uuid);
        let responses = response_views(&wire_node.responses, available);
        let msg = CommsMessage::injected(
            msg_id.clone(),
            sender_uuid,
            sender_name,
            wire_node.body.clone(),
            responses,
            thread_id.clone(),
            available,
            req.urgent,
        );
        channel2_writer.write(CommsChannel2Event { message: msg });
        comms.active_dialogues.insert(
            msg_id,
            ActiveDialogue {
                current_node: wire_node,
                thread_id,
                script: Some(ScriptedDialogue {
                    script_path: req.script_path.clone(),
                    node_fn: req.root_fn.clone(),
                    on_pick,
                }),
            },
        );
    }
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::comms::content::OpenCommsRequest;
    use crate::comms::server::CommsInboxRes;
    use crate::console::comms::server::handle_comms_channel2;
    use crate::world::script::engine::RuntimeHost;
    use crate::world::script::schedule::PendingCallbacks;

    /// The virtual path an inline `[script] setup = …` block compiles under.
    /// Shared with the `console::comms::server` tests that seat a scripted
    /// dialogue, so both name the same unit.
    pub(crate) const PATH: &str = "fixture/scripted.toml#script.setup";

    /// A test resolver that reads no sibling files — inline `[script]` blocks are
    /// lifted from the TOML directly and never consult a resolver.
    struct NoScriptResolver;
    impl crate::world::script::load::ScriptResolver for NoScriptResolver {
        fn read(&self, _path: &str) -> Option<String> {
            None
        }
    }

    /// Build a `WorldScriptRuntime` from an inline-`[script]` fixture the SAME
    /// way `compile_world_scripts` does in production.
    pub(crate) fn compile_fixture(body: &str) -> WorldScriptRuntime {
        let world_toml = format!("[script]\nsetup = '''{body}'''\n");
        let value: toml::Value = toml::from_str(&world_toml).expect("valid fixture toml");
        let compiled = crate::world::script::load::load_world_scripts(
            "fixture/scripted.toml",
            &value,
            &NoScriptResolver,
        );
        assert!(
            !crate::world::validate::has_error(&compiled.findings),
            "fixture scripts must compile clean: {:?}",
            compiled.findings
        );
        WorldScriptRuntime {
            host: RuntimeHost::new(),
            asts: compiled.asts,
            triggers: compiled.script_triggers,
            handlers: Vec::new(),
            budget: TickBudget::new(),
            budget_tick: 0,
            content_hash: compiled.content_hash,
            pending_callbacks: PendingCallbacks::new(),
            pending_comms_opens: Vec::new(),
        }
    }

    /// The smallest app that runs the live drain and delivers what it writes:
    /// `open_scripted_comms_threads` → `handle_comms_channel2` → inbox, the same
    /// Physics-then-Broadcast order `CommsWorldPlugin` registers.
    fn scripted_comms_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin)
            .init_resource::<WorldContentRuntime>()
            .init_resource::<CommsRuntime>()
            .init_resource::<CommsInboxRes>()
            .init_resource::<ObjectiveManagerRes>()
            .add_message::<CommsChannel2Event>()
            .add_systems(
                Update,
                (open_scripted_comms_threads, handle_comms_channel2).chain(),
            );
        app
    }

    fn request(root_fn: &str, from: &str) -> OpenCommsRequest {
        OpenCommsRequest {
            from: from.to_string(),
            root_fn: root_fn.to_string(),
            display_name: None,
            thread_id: None,
            urgent: false,
            script_path: PATH.to_string(),
        }
    }

    const AXIOM_TREE: &str = r#"
        fn hail_axiom(ctx) {
            #{ message: "Axiom Station, go ahead.", responses: [
                #{ text: "Acknowledge", on_pick: "on_ack" },
                #{ text: "Decline",     on_pick: "on_decline", important: true },
            ] }
        }
        fn on_ack(ctx)     { ctx.effects.complete_objective("reach_axiom"); }
        fn on_decline(ctx) { ctx.effects.fail_objective("reach_axiom"); }
    "#;

    /// The slice's whole point: a queued request becomes a real inbox message
    /// with a real dialogue behind it, through the LIVE system.
    #[test]
    fn a_queued_open_injects_the_root_node_and_records_a_scripted_dialogue() {
        let mut app = scripted_comms_app();
        let mut sr = compile_fixture(AXIOM_TREE);
        sr.pending_comms_opens.push(OpenCommsRequest {
            display_name: Some("Axiom Control".into()),
            urgent: true,
            ..request("hail_axiom", "axiom")
        });
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("axiom".into(), "axiom-uuid".into());
        app.world_mut().insert_resource(sr);

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(
            messages.len(),
            1,
            "the open must inject exactly one message"
        );
        let msg = &messages[0];
        assert_eq!(msg.body, "Axiom Station, go ahead.");
        assert_eq!(
            msg.sender_uuid, "axiom-uuid",
            "the sender reference id resolves through name_to_uuid"
        );
        assert_eq!(
            msg.sender_name, "Axiom Control",
            "display_name overrides the reference id, as a template's does"
        );
        assert!(msg.is_urgent, "urgency rides on the OPEN");
        assert!(!msg.thread_id.is_empty(), "an absent thread_id is minted");
        assert_eq!(
            msg.responses
                .iter()
                .map(|r| (r.text.as_str(), r.important))
                .collect::<Vec<_>>(),
            vec![("Acknowledge", false), ("Decline", true)]
        );

        let comms = app.world().resource::<CommsRuntime>();
        let dialogue = comms
            .active_dialogues
            .get(&msg.id)
            .expect("the injected message has an active dialogue");
        assert_eq!(dialogue.thread_id, msg.thread_id);
        let script = dialogue.script.as_ref().expect("a scripted dialogue");
        assert_eq!(script.script_path, PATH);
        assert_eq!(script.node_fn, "hail_axiom");
        assert_eq!(
            script.on_pick,
            vec!["on_ack".to_string(), "on_decline".to_string()],
            "the on_pick names are parallel to the shown responses"
        );
        assert!(
            dialogue
                .current_node
                .responses
                .iter()
                .all(|r| r.actions.is_empty() && r.follow_up.is_none()),
            "a projected node carries no declarative actions or follow-up"
        );
        assert!(
            app.world()
                .resource::<WorldScriptRuntime>()
                .pending_comms_opens
                .is_empty(),
            "the queue is drained"
        );
    }

    /// A root fn's effects route through the SAME apply path a trigger handler's
    /// do, and the node it returned is still delivered.
    #[test]
    fn a_root_fns_effects_apply_through_the_shared_dispatch_path() {
        let mut app = scripted_comms_app();
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "reach Axiom",
            true,
            vec![],
        );
        let mut sr = compile_fixture(
            r#"
            fn hail_axiom(ctx) {
                ctx.effects.complete_objective("reach_axiom");
                ctx.flags.hailed = 1;
                #{ message: "Docking clamps released.", responses: [] }
            }
            "#,
        );
        sr.pending_comms_opens.push(request("hail_axiom", "axiom"));
        app.world_mut().insert_resource(sr);

        app.update();

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
            "the root fn's complete_objective must reach the objective manager"
        );
        assert!(
            app.world()
                .resource::<WorldContentRuntime>()
                .flags
                .flag("hailed"),
            "the root fn's flag write must land on the live store"
        );
        assert_eq!(
            app.world().resource::<CommsInboxRes>().0.messages().len(),
            1,
            "and the node it returned is still delivered"
        );
    }

    /// The no-re-entrancy rule (the `pending_callbacks` rule, applied to opens): a
    /// thread that opens another thread does NOT materialise it inside this pass.
    #[test]
    fn a_nested_open_is_drained_on_the_next_tick_not_re_entrantly() {
        let mut app = scripted_comms_app();
        let mut sr = compile_fixture(
            r#"
            fn first(ctx) {
                ctx.effects.open_comms(#{ from: "axiom", node_fn: "second" });
                #{ message: "One.", responses: [] }
            }
            fn second(ctx) { #{ message: "Two.", responses: [] } }
            "#,
        );
        sr.pending_comms_opens.push(request("first", "axiom"));
        app.world_mut().insert_resource(sr);

        app.update();
        let bodies: Vec<String> = app
            .world()
            .resource::<CommsInboxRes>()
            .0
            .messages()
            .into_iter()
            .map(|m| m.body)
            .collect();
        assert_eq!(
            bodies,
            vec!["One.".to_string()],
            "the nested open must not be materialised in the same pass"
        );
        assert_eq!(
            app.world()
                .resource::<WorldScriptRuntime>()
                .pending_comms_opens
                .len(),
            1,
            "it is queued for the next drain instead"
        );

        app.update();
        let bodies: Vec<String> = app
            .world()
            .resource::<CommsInboxRes>()
            .0
            .messages()
            .into_iter()
            .map(|m| m.body)
            .collect();
        assert_eq!(bodies, vec!["One.".to_string(), "Two.".to_string()]);
    }

    /// Display-name parity with `handle_hail`: an open that omits
    /// `display_name` falls back to the CONTACT's authored name, not to the raw
    /// reference id. Every fixture above hardcodes `display_name`, which is what
    /// hid this — a scripted thread from a known station used to label itself
    /// with an internal id where the declarative path showed a name.
    #[test]
    fn an_open_without_a_display_name_falls_back_to_the_contact_name() {
        let mut app = scripted_comms_app();
        let mut sr = compile_fixture(AXIOM_TREE);
        sr.pending_comms_opens.push(request("hail_axiom", "axiom"));
        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert("axiom".into(), "axiom-uuid".into());
        app.world_mut()
            .resource_mut::<CommsRuntime>()
            .contacts
            .push(crate::messages::CommsContact {
                uuid: "axiom-uuid".into(),
                name: "Axiom Control".into(),
                in_range: true,
                is_urgent: false,
            });
        app.world_mut().insert_resource(sr);

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(
            messages[0].sender_name, "Axiom Control",
            "the contact's name is the fallback, exactly as handle_hail resolves it"
        );
    }

    /// And with no contact either, the reference id is still the last resort —
    /// the third step of the same fallback.
    #[test]
    fn an_open_with_neither_display_name_nor_contact_uses_the_reference_id() {
        let mut app = scripted_comms_app();
        let mut sr = compile_fixture(AXIOM_TREE);
        sr.pending_comms_opens.push(request("hail_axiom", "axiom"));
        app.world_mut().insert_resource(sr);

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages[0].sender_name, "axiom");
    }

    /// Finding 3 on the open path: a root fn that completed an objective and
    /// then returned a malformed map keeps the objective completed. The call
    /// SUCCEEDED and its buffers drained — only the return value is wrong, which
    /// is a different thing from the script ERROR settled decision 10 discards
    /// whole.
    #[test]
    fn a_malformed_root_return_still_applies_the_effects_it_produced() {
        let mut app = scripted_comms_app();
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "reach_axiom",
            "reach Axiom",
            true,
            vec![],
        );
        let mut sr = compile_fixture(
            r#"
            fn hail_axiom(ctx) {
                ctx.effects.complete_objective("reach_axiom");
                "not a node map"
            }
            "#,
        );
        sr.pending_comms_opens.push(request("hail_axiom", "axiom"));
        app.world_mut().insert_resource(sr);

        app.update();

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
            app.world()
                .resource::<CommsInboxRes>()
                .0
                .messages()
                .is_empty(),
            "and no message is injected — there was no node to show"
        );
    }

    /// An unresolvable root fn opens nothing and does not panic: the name is
    /// resolved against the unit BEFORE the call, so an authoring typo is a
    /// refusal rather than a mid-mission `CallError`.
    #[test]
    fn an_unresolvable_root_fn_opens_no_thread() {
        let mut app = scripted_comms_app();
        let mut sr = compile_fixture(AXIOM_TREE);
        sr.pending_comms_opens
            .push(request("hail_axium_typo", "axiom"));
        app.world_mut().insert_resource(sr);

        app.update();

        assert!(app
            .world()
            .resource::<CommsInboxRes>()
            .0
            .messages()
            .is_empty());
        assert!(app
            .world()
            .resource::<CommsRuntime>()
            .active_dialogues
            .is_empty());
    }

    /// The synthetic-sender escape, mirroring `inject_comms_templates`: an
    /// unresolvable `from` falls through to itself and stays readable, and the
    /// reserved `_self` renders as the internal-report channel.
    #[test]
    fn a_synthetic_sender_falls_through_to_its_own_name_and_stays_readable() {
        let mut app = scripted_comms_app();
        let mut sr = compile_fixture(r#"fn report(ctx) { #{ message: "Scan complete." } }"#);
        sr.pending_comms_opens.push(request("report", "_self"));
        app.world_mut().resource_mut::<CommsRuntime>().range_active = true;
        app.world_mut().insert_resource(sr);

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].sender_uuid, "_self");
        assert_eq!(messages[0].sender_name, "Internal Report");
        assert!(
            messages[0].sender_in_range,
            "a synthetic sender has no entity to range-check against"
        );
    }

    /// The digest-neutrality guard: with no `WorldScriptRuntime` the system
    /// returns before touching anything, so a script-free world (every shipped
    /// one) takes a byte-identical path to before this slice.
    #[test]
    fn a_script_free_world_is_a_no_op() {
        let mut app = scripted_comms_app();
        app.update();
        assert!(app
            .world()
            .resource::<CommsInboxRes>()
            .0
            .messages()
            .is_empty());
        assert!(app
            .world()
            .resource::<CommsRuntime>()
            .active_dialogues
            .is_empty());
    }

    // -- The live parity suite (issue #984) -----------------------------------
    //
    // The M4 unit parity test compared a scripted `on_pick`'s buffered
    // `ActionCmd`s against its TOML twin's dispatched ones by calling the host
    // directly. These promote that claim to the LIVE app: one hail, two threads
    // — a scripted one whose trigger handler calls `open_comms`, and its
    // declarative `[[comms]]` twin — travelling the real
    // Input → Physics → Broadcast chain, compared on what actually reaches the
    // wire and what their picks actually do to the world.

    use crate::comms::content::{
        CommsDialogueNode, CommsResponse, CommsTemplate, CommsTemplateState,
    };
    use crate::comms::server::tests::{comms_test_app, push_msg, setup_game_with_comms, tick};
    use crate::messages::{ClientMessage, CommsMessage, ObjectiveStatus};
    use crate::world::content::{TriggerAction, TriggerCondition};

    const STATION_UUID: &str = "a1b2c3d4-e5f6-4789-abcd-ef0123456990";

    /// The scripted thread: an `on_hailed` handler opens it, the root node
    /// offers two responses, and each `on_pick` resolves the objective.
    const SCRIPTED_THREAD: &str = r#"
on_hailed("starbase_alpha", "hail_handler");
fn hail_handler(ctx) {
    ctx.effects.open_comms(#{
        from: "starbase_alpha",
        node_fn: "root",
        display_name: "Starbase Alpha",
    });
}
fn root(ctx) {
    #{ message: "USS Phoenix, please identify yourself.", responses: [
        #{ text: "We are on a survey mission.", on_pick: "on_ack" },
        #{ text: "No comment.", on_pick: "on_decline", important: true },
    ] }
}
fn on_ack(ctx)     { ctx.effects.complete_objective("script_obj"); }
fn on_decline(ctx) { ctx.effects.fail_objective("script_obj"); }
"#;

    /// The declarative twin of [`SCRIPTED_THREAD`]: the same body, the same two
    /// responses, the same two objective actions — authored as TOML tables.
    fn declarative_twin() -> CommsTemplateState {
        let action = |a: TriggerAction| vec![a];
        CommsTemplateState {
            template: CommsTemplate {
                from: "starbase_alpha".into(),
                trigger: TriggerCondition::OnHailed {
                    entity_name: "starbase_alpha".into(),
                },
                node: CommsDialogueNode {
                    body: "USS Phoenix, please identify yourself.".into(),
                    responses: vec![
                        CommsResponse {
                            text: "We are on a survey mission.".into(),
                            important: false,
                            actions: action(TriggerAction::CompleteObjective {
                                id: "decl_obj".into(),
                            }),
                            follow_up: None,
                        },
                        CommsResponse {
                            text: "No comment.".into(),
                            important: true,
                            actions: action(TriggerAction::FailObjective {
                                id: "decl_obj".into(),
                            }),
                            follow_up: None,
                        },
                    ],
                    speaker: None,
                    trigger: None,
                },
                thread_id: None,
                urgent: false,
                root_follow_up: None,
                display_name: None,
            },
            fired: false,
        }
    }

    /// The full live path: `comms_test_app`'s Input → Broadcast chain with the
    /// world-script Physics systems spliced in exactly where `CommsWorldPlugin`
    /// puts them (`collect_world_events` → `tick_trigger_pipeline` →
    /// `tick_script_callbacks` → `open_scripted_comms_threads`, between the
    /// follow-up tick and the channel-2 delivery). The order is total, not
    /// merely implied: every added system is pinned against the chain it joins.
    ///
    /// `tick_script_callbacks` is in the chain because production's
    /// `open_scripted_comms_threads` is registered `.after(` it — an open queued
    /// by a deferred `ctx.schedule.after(n, …)` callback is materialised on the
    /// tick it was made, and that edge is only exercised if both systems are
    /// present. Leaving it out let the fixture pass on an ordering the shipped
    /// schedule does not have.
    fn live_comms_app() -> App {
        let mut app = comms_test_app();
        app.init_resource::<crate::world::server::WorldEventBuffer>()
            .add_message::<crate::ai::server::AiEntityAttacked>()
            .add_message::<crate::ai::server::AiEntityDestroyed>()
            .add_message::<crate::ai::server::AiWaypointReached>()
            .add_systems(
                FixedUpdate,
                (
                    crate::world::server::collect_world_events,
                    crate::world::server::tick_trigger_pipeline,
                    crate::world::server::tick_script_callbacks,
                    open_scripted_comms_threads,
                )
                    .chain()
                    .after(crate::comms::server::tick_pending_follow_ups)
                    .before(crate::console::comms::server::handle_comms_channel2),
            );
        app
    }

    /// Seat the twins: the declarative template, the scripted trigger + its
    /// handler table, and the two objectives their picks resolve.
    fn seat_twin_threads(app: &mut App, scripted: bool) {
        setup_game_with_comms(app, STATION_UUID);
        {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            // Replace the harness's own template with the declarative twin.
            comms.comms_template_states = vec![declarative_twin()];
        }
        for id in ["decl_obj", "script_obj"] {
            app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
                id,
                "identify the ship",
                true,
                vec![],
            );
        }
        if scripted {
            let mut sr = compile_fixture(SCRIPTED_THREAD);
            {
                let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
                crate::world::server::merge_script_triggers(&mut runtime.trigger_states, &mut sr);
            }
            app.world_mut().insert_resource(sr);
        }
    }

    fn hail(app: &mut App) {
        push_msg(
            app,
            "comms",
            ClientMessage::ControlSystem {
                target: crate::system_registry::comms_system_id(),
                payload: crate::messages::SystemControlPayload::Hail {
                    target_uuid: STATION_UUID.into(),
                },
            },
        );
        let _ = tick(app);
    }

    /// Split the inbox into `(declarative, scripted)` by which dialogue carries a
    /// `ScriptedDialogue` — the only thing that distinguishes the twins.
    fn split_twins(app: &App) -> (CommsMessage, CommsMessage) {
        let comms = app.world().resource::<CommsRuntime>();
        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        let scripted = |m: &CommsMessage| {
            comms
                .active_dialogues
                .get(&m.id)
                .is_some_and(|d| d.script.is_some())
        };
        let decl = messages
            .iter()
            .find(|m| !scripted(m))
            .expect("the declarative twin is delivered")
            .clone();
        let script = messages
            .iter()
            .find(|m| scripted(m))
            .expect("the scripted twin is delivered")
            .clone();
        (decl, script)
    }

    fn respond(app: &mut App, message_id: &str, response_index: usize) {
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
        let _ = tick(app);
    }

    fn status(app: &App, id: &str) -> ObjectiveStatus {
        app.world()
            .resource::<ObjectiveManagerRes>()
            .0
            .sorted_snapshots()
            .into_iter()
            .find(|o| o.id == id)
            .unwrap_or_else(|| panic!("objective '{id}' exists"))
            .status
    }

    /// The wire half of the parity claim: one hail delivers both threads, and
    /// what the Comms officer sees is indistinguishable between them.
    #[test]
    fn scripted_and_declarative_threads_deliver_identical_messages_on_the_wire() {
        let mut app = live_comms_app();
        seat_twin_threads(&mut app, true);
        hail(&mut app);

        let (decl, script) = split_twins(&app);
        assert_eq!(decl.body, script.body);
        assert_eq!(decl.subject, script.subject);
        assert_eq!(decl.sender_uuid, script.sender_uuid);
        assert_eq!(
            decl.sender_name, script.sender_name,
            "the open's display_name is the scripted analogue of the contact label"
        );
        assert_eq!(decl.is_urgent, script.is_urgent);
        assert_eq!(decl.sender_in_range, script.sender_in_range);
        assert_eq!(
            decl.responses, script.responses,
            "text, `important` and availability must match response for response"
        );
        // And the concrete shape, so this pins behaviour rather than
        // equality-to-itself.
        assert_eq!(script.body, "USS Phoenix, please identify yourself.");
        assert_eq!(
            script
                .responses
                .iter()
                .map(|r| (r.text.as_str(), r.important))
                .collect::<Vec<_>>(),
            vec![
                ("We are on a survey mission.", false),
                ("No comment.", true)
            ]
        );
    }

    /// The effect half: for the SAME pick, both threads resolve their objective
    /// the same way — the live-app form of "identical `ActionCmd`s per choice".
    #[test]
    fn scripted_and_declarative_responses_have_identical_effects_per_choice() {
        for (index, expected) in [
            (0usize, ObjectiveStatus::Completed),
            (1usize, ObjectiveStatus::Failed),
        ] {
            let mut app = live_comms_app();
            seat_twin_threads(&mut app, true);
            hail(&mut app);
            let (decl, script) = split_twins(&app);

            respond(&mut app, &decl.id, index);
            respond(&mut app, &script.id, index);

            assert_eq!(
                status(&app, "decl_obj"),
                status(&app, "script_obj"),
                "choice {index}: the scripted thread and its TOML twin must have \
                 identical effects"
            );
            assert_eq!(status(&app, "script_obj"), expected);
        }
    }

    /// R1's proof, and the end of the warn-drop era for dialogue effects: a
    /// scripted `on_pick` that spawns — a NAME-RESOLVING `BufferedEffect::Action`,
    /// the kind the old effect-only dialogue entry point dropped with a warning —
    /// resolves through `dispatch_action` and the entity EXISTS afterwards, with
    /// its name registered for later triggers to resolve.
    #[test]
    fn a_scripted_on_pick_that_spawns_resolves_through_dispatch() {
        crate::config_cache::insert_native_config(
            "fixture/comms_escort.toml".to_string(),
            crate::entity_config::EntityConfig::from_toml("").unwrap(),
        );
        let mut app = live_comms_app();
        seat_twin_threads(&mut app, false);
        let mut sr = compile_fixture(
            r#"
on_hailed("starbase_alpha", "hail_handler");
fn hail_handler(ctx) {
    ctx.effects.open_comms(#{ from: "starbase_alpha", node_fn: "root" });
}
fn root(ctx) {
    #{ message: "Escort inbound?", responses: [
        #{ text: "Send it", on_pick: "on_send" },
    ] }
}
fn on_send(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "fixture/comms_escort.toml",
        name: "escort",
        position: [100, 0, 0],
    });
}
"#,
        );
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            crate::world::server::merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);
        app.world_mut()
            .insert_resource(crate::world_id::WorldIdMint::default());

        hail(&mut app);
        let (_decl, script) = split_twins(&app);
        respond(&mut app, &script.id, 0);

        let escort_uuid = app
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid
            .get("escort")
            .cloned()
            .expect(
                "a scripted on_pick's spawn must resolve through dispatch_action and \
                 register its name — not be warn-dropped",
            );
        let mut q = app.world_mut().query::<&EntityUuid>();
        assert!(
            q.iter(app.world()).any(|u| u.0 == escort_uuid),
            "and the entity must actually exist in the ECS after the response"
        );
        // R1's OTHER half: the id came from the REAL `WorldIdMint` the
        // declarative arm draws from, not from a fallback mint inside the
        // effects boundary. A fallback would leave this counter at zero while
        // still producing a plausible-looking uuid — which is exactly the
        // divergence that breaks structural byte-identity between peers.
        assert_eq!(
            app.world()
                .resource::<crate::world_id::WorldIdMint>()
                .minted_so_far(crate::world_id::IdNamespace::Entity),
            1,
            "exactly one entity id, minted from the shared tick-scoped mint"
        );
    }

    // ── The shipped converted world (issue #984) ──────────────────────────────
    //
    // `default.toml` is the first world whose COMMS moved to `[script]`, and
    // the digest A/B that gates a conversion cannot speak for it: nothing in
    // that world attacks or hails anything during a headless run, and
    // `state_digest` folds no comms state in any case. These three tests are the
    // behavioural half of the evidence — the real shipped script, compiled the
    // way production compiles it, driven through the live path.

    /// The reference id `default.toml` gives Starbase Alpha. The `[[comms]]`
    /// blocks the conversion deleted named it in `from` and `entity`; the
    /// `[script]` block names it in `on_hailed(…)` and `open_comms(#{from})`.
    const DEFAULT_STARBASE: &str = "world.entity.starbase_alpha.name";
    const DEFAULT_RAIDER: &str = "world.entity.raider_alpha.name";

    /// Compile the SHIPPED `default.toml`'s `[script]` block exactly as
    /// `compile_world_scripts` does, and return it alongside the virtual path
    /// its inline block was lifted to.
    fn compile_default_world() -> (WorldScriptRuntime, String) {
        let text = include_str!("../../assets/worlds/default.toml");
        let value: toml::Value = toml::from_str(text).expect("default.toml is valid TOML");
        let compiled = crate::world::script::load::load_world_scripts(
            "assets/worlds/default.toml",
            &value,
            &NoScriptResolver,
        );
        assert!(
            !crate::world::validate::has_error(&compiled.findings),
            "the shipped default.toml script must compile and lint clean: {:?}",
            compiled.findings
        );
        let path = compiled
            .asts
            .keys()
            .next()
            .cloned()
            .expect("default.toml lifts one inline script unit");
        (
            WorldScriptRuntime {
                host: RuntimeHost::new(),
                asts: compiled.asts,
                triggers: compiled.script_triggers,
                handlers: Vec::new(),
                budget: TickBudget::new(),
                budget_tick: 0,
                content_hash: compiled.content_hash,
                pending_callbacks: PendingCallbacks::new(),
                pending_comms_opens: Vec::new(),
            },
            path,
        )
    }

    /// Every registration the deleted `[[trigger]]` blocks and `[[comms]]`
    /// templates carried is present, in the authored order — the order intra-tick
    /// dispatch (and therefore the digest) depends on.
    ///
    /// The pairing is the conversion map: two of these were `[[trigger]]`
    /// blocks and three were a `[[comms]]` template's `trigger =` / `entity =`
    /// pair, and the raider's `on_attacked` carries BOTH of that event's old
    /// reactions in one handler.
    #[test]
    fn default_worlds_script_registers_every_trigger_its_declarative_blocks_carried() {
        use crate::world::config::TriggerCondition;
        let (sr, _path) = compile_default_world();
        let registered: Vec<(TriggerCondition, &str)> = sr
            .triggers
            .iter()
            .map(|t| (t.trigger.condition.clone(), t.handler.as_str()))
            .collect();
        assert_eq!(
            registered,
            vec![
                (
                    TriggerCondition::OnDestroyed {
                        entity_name: DEFAULT_RAIDER.into()
                    },
                    "on_raider_destroyed"
                ),
                (
                    TriggerCondition::OnAttacked {
                        entity_name: DEFAULT_RAIDER.into()
                    },
                    "on_raider_attacked"
                ),
                (
                    TriggerCondition::OnAttacked {
                        entity_name: DEFAULT_STARBASE.into()
                    },
                    "on_starbase_attacked"
                ),
                (
                    TriggerCondition::OnHailed {
                        entity_name: DEFAULT_STARBASE.into()
                    },
                    "on_starbase_hailed"
                ),
            ]
        );
    }

    /// The raider's `on_attacked` handler emits BOTH of what that event used to
    /// do: the `[[trigger]]`'s `load_world` — byte-identical to what
    /// `dispatch_action` produces for the declarative action, so the
    /// reinforcements LAYER still loads — and the `[[comms]]` template's
    /// broadcast, now an `open_comms` naming the announcement node.
    #[test]
    fn default_worlds_raider_attack_still_loads_the_reinforcements_layer_and_broadcasts() {
        use crate::world::script::effects::BufferedEffect;
        let (sr, path) = compile_default_world();
        let mut budget = TickBudget::new();
        let (effects, node) = crate::world::script::comms::enter_node(
            &sr.host,
            &mut budget,
            &SchedClock::ZERO,
            sr.asts.get(&path).expect("compiled unit"),
            &path,
            "on_raider_attacked",
            &crate::world::flags::FlagStore::new(),
        )
        .expect("the handler runs");
        assert!(node.is_none(), "a trigger handler returns no dialogue node");

        assert_eq!(
            effects.commands,
            vec![BufferedEffect::Cmd(
                crate::world::dispatch::ActionCmd::LoadWorld {
                    path: "assets/worlds/reinforcements.toml".into(),
                    loader_path: None,
                }
            )],
            "the layer load survived the conversion unchanged"
        );
        assert_eq!(effects.comms_opens.len(), 1);
        assert_eq!(effects.comms_opens[0].from, DEFAULT_RAIDER);
        assert_eq!(effects.comms_opens[0].root_fn, "raider_mayday");
        assert!(
            !effects.comms_opens[0].urgent,
            "neither distress template authored `urgent`"
        );
    }

    /// Seat the shipped `default.toml` script over the live comms harness, with
    /// Starbase Alpha under the reference id the world actually uses.
    fn seat_default_world(app: &mut App) {
        setup_game_with_comms(app, STATION_UUID);
        {
            let mut comms = app.world_mut().resource_mut::<CommsRuntime>();
            // The converted world authors NO declarative templates; drop the
            // harness's own so the inbox holds only what the script delivers.
            comms.comms_template_states.clear();
        }
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            runtime
                .name_to_uuid
                .insert(DEFAULT_STARBASE.into(), STATION_UUID.into());
        }
        let (mut sr, _path) = compile_default_world();
        {
            let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
            crate::world::server::merge_script_triggers(&mut runtime.trigger_states, &mut sr);
        }
        app.world_mut().insert_resource(sr);
        app.world_mut()
            .insert_resource(crate::world_id::WorldIdMint::default());
    }

    /// Hailing Starbase Alpha delivers the SAME message the `[[comms]]`
    /// `on_hailed` template delivered: the same body id, the same two response
    /// text ids, in the same order.
    ///
    /// The ids are the point. A dialogue node's `message` and a response's
    /// `text` reach the wire as message bodies exactly as the declarative
    /// `message` / `text` fields did, so every `strings.csv` row the world used
    /// still resolves and none had to be renumbered.
    #[test]
    fn default_worlds_hail_delivers_the_same_body_and_responses_as_its_template() {
        let mut app = live_comms_app();
        seat_default_world(&mut app);
        hail(&mut app);

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1, "the hail opens exactly one thread");
        let msg = &messages[0];
        assert_eq!(msg.body, "world.default.comms.2.message");
        assert_eq!(
            msg.sender_uuid, STATION_UUID,
            "the open's `from` resolves through name_to_uuid, as the template's did"
        );
        assert_eq!(
            msg.sender_name, "Starbase Alpha",
            "with no `display_name` the label falls back to the CONTACT's name — \
             which after #985 is the entity's own reference id"
        );
        assert!(!msg.is_urgent);
        assert_eq!(
            msg.responses
                .iter()
                .map(|r| (r.text.as_str(), r.important))
                .collect::<Vec<_>>(),
            vec![
                ("world.default.comms.response.0.text", false),
                ("world.default.comms.response.1.text", false),
            ]
        );
    }

    /// Each response adds the objective its `[[comms.response.action]]` added,
    /// with the same id, the same text id and the same `mandatory` flag — and
    /// the thread ends there, as both terminal responses always did.
    #[test]
    fn default_worlds_hail_responses_add_the_objectives_their_actions_did() {
        for (index, id, text, mandatory) in [
            (
                0usize,
                "obj-survey",
                "world.default.comms.response.action.obj_survey.text",
                false,
            ),
            (
                1usize,
                "obj-dock",
                "world.default.comms.response.action.obj_dock.text",
                true,
            ),
        ] {
            let mut app = live_comms_app();
            seat_default_world(&mut app);
            hail(&mut app);
            let message_id = app.world().resource::<CommsInboxRes>().0.messages()[0]
                .id
                .clone();

            respond(&mut app, &message_id, index);

            let snapshot = app
                .world()
                .resource::<ObjectiveManagerRes>()
                .0
                .sorted_snapshots()
                .into_iter()
                .find(|o| o.id == id)
                .unwrap_or_else(|| panic!("response {index} must add objective '{id}'"));
            assert_eq!(snapshot.text, text, "the strings.csv id is unchanged");
            assert_eq!(snapshot.mandatory, mandatory);

            assert_eq!(
                app.world().resource::<CommsInboxRes>().0.messages().len(),
                1,
                "both responses are terminal — no follow-up node is delivered"
            );
        }
    }

    /// The no-op guard at full scale: the SAME hail on a world with no scripts
    /// delivers the declarative twin and nothing else, and the new system leaves
    /// no trace — every shipped world takes this path.
    #[test]
    fn a_hail_on_a_script_free_world_delivers_only_the_declarative_thread() {
        let mut app = live_comms_app();
        seat_twin_threads(&mut app, false);
        hail(&mut app);

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(messages.len(), 1, "only the declarative twin arrives");
        let comms = app.world().resource::<CommsRuntime>();
        assert_eq!(comms.active_dialogues.len(), 1);
        assert!(
            comms.active_dialogues.values().all(|d| d.script.is_none()),
            "a declarative dialogue never carries scripted state"
        );

        // And the declarative pick still behaves exactly as it always did.
        let id = messages[0].id.clone();
        respond(&mut app, &id, 0);
        assert_eq!(status(&app, "decl_obj"), ObjectiveStatus::Completed);
        assert_eq!(status(&app, "script_obj"), ObjectiveStatus::Active);
    }

    // ── The shipped demo scenario (issue #984) ────────────────────────────────
    //
    // `combat_test.toml` is the LAST world to convert, and the only one whose
    // comms a headless run can neither exercise nor speak for: its twelve
    // reports are one-way announcements over Channel 2, which no `SimOutbox`
    // message and no `state_digest` field records. The digest A/B that gates the
    // conversion therefore proves the world's SIMULATION is unmoved — it matches
    // to the byte over three run lengths — and says nothing at all about the
    // reports. These tests are that half of the evidence, on the real shipped
    // script compiled the way production compiles it.

    /// Compile the SHIPPED `combat_test.toml`'s `[script]` block exactly as
    /// `compile_world_scripts` does.
    fn compile_combat_test() -> (WorldScriptRuntime, String) {
        let text = include_str!("../../assets/worlds/combat_test.toml");
        let value: toml::Value = toml::from_str(text).expect("combat_test.toml is valid TOML");
        let compiled = crate::world::script::load::load_world_scripts(
            "assets/worlds/combat_test.toml",
            &value,
            &NoScriptResolver,
        );
        assert!(
            !crate::world::validate::has_error(&compiled.findings),
            "the shipped combat_test.toml script must compile and lint clean: {:?}",
            compiled.findings
        );
        let path = compiled
            .asts
            .keys()
            .next()
            .cloned()
            .expect("combat_test.toml lifts one inline script unit");
        (
            WorldScriptRuntime {
                host: RuntimeHost::new(),
                asts: compiled.asts,
                triggers: compiled.script_triggers,
                handlers: Vec::new(),
                budget: TickBudget::new(),
                budget_tick: 0,
                content_hash: compiled.content_hash,
                pending_callbacks: PendingCallbacks::new(),
                pending_comms_opens: Vec::new(),
            },
            path,
        )
    }

    /// The twelve reports, in the order the `[[comms]]` templates were authored:
    /// `(handler, node fn, strings.csv body id, urgent)`.
    const COMBAT_TEST_REPORTS: &[(&str, &str, &str, bool)] = &[
        (
            "send_brief",
            "brief",
            "world.combat_test.comms.0.message",
            true,
        ),
        (
            "report_wave_1",
            "wave_1_report",
            "world.combat_test.comms.1.message",
            false,
        ),
        (
            "report_wave_2",
            "wave_2_report",
            "world.combat_test.comms.2.message",
            false,
        ),
        (
            "report_wave_3",
            "wave_3_report",
            "world.combat_test.comms.3.message",
            false,
        ),
        (
            "report_wave_4",
            "wave_4_report",
            "world.combat_test.comms.4.message",
            false,
        ),
        (
            "report_wave_5",
            "wave_5_report",
            "world.combat_test.comms.5.message",
            true,
        ),
        (
            "report_wave_6",
            "wave_6_report",
            "world.combat_test.comms.6.message",
            true,
        ),
        (
            "report_wave_7",
            "wave_7_report",
            "world.combat_test.comms.7.message",
            true,
        ),
        (
            "report_wave_8",
            "wave_8_report",
            "world.combat_test.comms.8.message",
            true,
        ),
        (
            "report_hull_75",
            "hull_75",
            "world.combat_test.comms.9.message",
            true,
        ),
        (
            "report_hull_50",
            "hull_50",
            "world.combat_test.comms.10.message",
            true,
        ),
        (
            "report_hull_10",
            "hull_10",
            "world.combat_test.comms.11.message",
            true,
        ),
    ];

    /// Every registration the deleted twenty `[[trigger]]` blocks and twelve
    /// `[[comms]]` templates carried is present, with the same condition, in the
    /// authored order — the order intra-tick dispatch (and therefore the digest)
    /// depends on.
    ///
    /// The comms registrations sit LAST and unmerged, even though eight of them
    /// share a clock with a wave handler that could have carried them: the
    /// declarative comms evaluator drained its templates in authored order after
    /// the triggers had run, and keeping them last is what reproduces the order
    /// the twelve opens are queued in.
    #[test]
    fn combat_tests_script_registers_every_trigger_and_comms_block_it_replaced() {
        use crate::world::config::TriggerCondition;
        const STARBASE: &str = "world.entity.starbase_alpha.name";
        let (sr, _path) = compile_combat_test();

        let mut expected: Vec<(TriggerCondition, String)> =
            vec![(TriggerCondition::OnWorldLoaded, "arm_the_scenario".into())];
        let cadence = [0.0f32, 45.0, 90.0, 135.0, 180.0, 225.0, 270.0, 315.0];
        for (wave, after_secs) in cadence.iter().enumerate() {
            expected.push((
                TriggerCondition::OnTimer {
                    after_secs: *after_secs,
                },
                format!("release_wave_{}", wave + 1),
            ));
        }
        for wave in 1..=8 {
            expected.push((
                TriggerCondition::OnAllDestroyed {
                    group: format!("wave_{wave}"),
                    after_secs: 0.0,
                },
                format!("wave_{wave}_cleared"),
            ));
        }
        expected.push((
            TriggerCondition::OnAllDestroyed {
                group: "hostiles".into(),
                after_secs: 0.0,
            },
            "on_raid_broken".into(),
        ));
        expected.push((
            TriggerCondition::OnDestroyed {
                entity_name: STARBASE.into(),
            },
            "on_starbase_lost".into(),
        ));
        expected.push((
            TriggerCondition::OnWorldLoaded,
            "add_standing_mission".into(),
        ));
        // The twelve comms, in the order the templates were authored.
        expected.push((TriggerCondition::OnWorldLoaded, "send_brief".into()));
        for (wave, after_secs) in cadence.iter().enumerate() {
            expected.push((
                TriggerCondition::OnTimer {
                    after_secs: *after_secs,
                },
                format!("report_wave_{}", wave + 1),
            ));
        }
        for (threshold, handler) in [
            (0.75f32, "report_hull_75"),
            (0.5, "report_hull_50"),
            (0.1, "report_hull_10"),
        ] {
            expected.push((
                TriggerCondition::OnHullBelow {
                    entity_name: STARBASE.into(),
                    threshold,
                },
                handler.into(),
            ));
        }

        let registered: Vec<(TriggerCondition, String)> = sr
            .triggers
            .iter()
            .map(|t| (t.trigger.condition.clone(), t.handler.clone()))
            .collect();
        assert_eq!(registered, expected);

        // The victory guard is a trigger-LEVEL predicate, not an `if` in the
        // handler. That distinction is the scenario: a `when` that reads false
        // leaves the trigger armed, an in-handler guard spends it (issue #892).
        let guarded: Vec<&str> = sr
            .triggers
            .iter()
            .filter(|t| t.trigger.when.is_some())
            .map(|t| t.handler.as_str())
            .collect();
        assert_eq!(
            guarded,
            vec!["on_raid_broken"],
            "victory is the one guarded registration in this world"
        );
    }

    /// Each of the twelve handlers opens exactly one thread, from Starbase
    /// Alpha, with the urgency its template authored — and the node it names
    /// returns the same `strings.csv` body id the template's `message` carried,
    /// with no responses, because all twelve always were one-way reports.
    #[test]
    fn combat_tests_twelve_reports_keep_their_senders_bodies_and_urgency() {
        const STARBASE: &str = "world.entity.starbase_alpha.name";
        let (sr, path) = compile_combat_test();
        let ast = sr.asts.get(&path).expect("compiled unit");
        let flags = crate::world::flags::FlagStore::new();

        for (handler, node_fn, body, urgent) in COMBAT_TEST_REPORTS {
            let mut budget = TickBudget::new();
            let (effects, node) = crate::world::script::comms::enter_node(
                &sr.host,
                &mut budget,
                &SchedClock::ZERO,
                ast,
                &path,
                handler,
                &flags,
            )
            .unwrap_or_else(|e| panic!("{handler} must run: {e}"));
            assert!(node.is_none(), "{handler} is a trigger handler, not a node");
            assert!(
                effects.commands.is_empty() && effects.delayed.is_empty(),
                "{handler} opens a thread and does nothing else"
            );
            assert_eq!(effects.comms_opens.len(), 1, "{handler} opens one thread");
            let open = &effects.comms_opens[0];
            assert_eq!(open.from, STARBASE);
            assert_eq!(open.root_fn, *node_fn);
            assert_eq!(open.urgent, *urgent, "{handler}'s urgency is authored");
            assert!(open.display_name.is_none() && open.thread_id.is_none());

            let mut budget = TickBudget::new();
            let (_, node) = crate::world::script::comms::enter_node(
                &sr.host,
                &mut budget,
                &SchedClock::ZERO,
                ast,
                &path,
                node_fn,
                &flags,
            )
            .unwrap_or_else(|e| panic!("{node_fn} must run: {e}"));
            let node = node.unwrap_or_else(|| panic!("{node_fn} must return a dialogue node"));
            assert_eq!(node.message, *body, "{node_fn} keeps its strings.csv id");
            assert!(
                node.responses.is_empty(),
                "{node_fn} is an announcement — the player cannot reply to it"
            );
        }
    }

    /// End to end through the LIVE drain: the twelve requests the shipped
    /// handlers actually produce become twelve inbox messages, with the twelve
    /// authored body ids in the authored order, each attributed to the station's
    /// resolved UUID and carrying its authored urgency.
    ///
    /// This is the assertion the digest cannot make. A converted world's
    /// simulation parity is proven by `--seed 42` matching to the byte; the
    /// reports never touch that surface, so they are proven here instead.
    #[test]
    fn combat_tests_reports_reach_the_inbox_with_their_authored_bodies() {
        const STARBASE: &str = "world.entity.starbase_alpha.name";
        const STARBASE_UUID: &str = "starbase-uuid";
        let mut app = scripted_comms_app();
        let (mut sr, path) = compile_combat_test();
        let flags = crate::world::flags::FlagStore::new();

        // Queue what the world's own handlers queue, in registration order.
        let ast = sr.asts.get(&path).expect("compiled unit").clone();
        let mut queued = Vec::new();
        for (handler, _, _, _) in COMBAT_TEST_REPORTS {
            let mut budget = TickBudget::new();
            let (effects, _) = crate::world::script::comms::enter_node(
                &sr.host,
                &mut budget,
                &SchedClock::ZERO,
                &ast,
                &path,
                handler,
                &flags,
            )
            .expect("the handler runs");
            queued.extend(effects.comms_opens);
        }
        assert_eq!(queued.len(), COMBAT_TEST_REPORTS.len());
        sr.pending_comms_opens = queued;

        app.world_mut()
            .resource_mut::<WorldContentRuntime>()
            .name_to_uuid
            .insert(STARBASE.into(), STARBASE_UUID.into());
        app.world_mut().insert_resource(sr);
        app.world_mut()
            .insert_resource(crate::world_id::WorldIdMint::default());

        app.update();

        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        assert_eq!(
            messages
                .iter()
                .map(|m| (m.body.as_str(), m.is_urgent))
                .collect::<Vec<_>>(),
            COMBAT_TEST_REPORTS
                .iter()
                .map(|(_, _, body, urgent)| (*body, *urgent))
                .collect::<Vec<_>>(),
            "every authored report reaches the wire, in order, with its urgency"
        );
        assert!(
            messages.iter().all(|m| m.sender_uuid == STARBASE_UUID),
            "each report resolves `from` through name_to_uuid, as its template's \
             `from` did"
        );
        assert!(
            messages.iter().all(|m| m.responses.is_empty()),
            "all twelve are one-way"
        );
    }
}
