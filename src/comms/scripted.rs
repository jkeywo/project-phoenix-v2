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
use crate::world::script::comms::{enter_node, project_node};
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
///    pre-flight `tripped()` check;
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
        // A tripped budget refuses every remaining call this tick by contract, so
        // stop here rather than logging once per request. Deterministic: the trip
        // is a pure function of the tick's call/op sequence, so every peer drops
        // the same tail. The requests are dropped, not re-queued — re-queueing a
        // refused open would let a busy tick push work forward indefinitely.
        if sr.budget.tripped() {
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
        let sender_name = req.display_name.clone().unwrap_or(channel_name);
        let sender_uuid = runtime
            .name_to_uuid
            .get(&req.from)
            .cloned()
            .unwrap_or_else(|| req.from.clone());

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
        let (effects, node) = match entered {
            Ok(pair) => pair,
            Err(err) => {
                // A shape error (the fn compiled but returned the wrong thing) —
                // authoring-facing, distinct from a script error, which
                // `call_dialogue` already handled under the failure policy.
                bevy::log::warn!(
                    "open_scripted_comms_threads: root fn '{}' in '{}': {err}",
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

        // A root fn that returned `()` opened no thread — its effects still
        // applied above. Authoring error rather than a supported shape (an open
        // with nothing to show), so it is worth a line in the log.
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
mod tests {
    use super::*;
    use crate::comms::content::OpenCommsRequest;
    use crate::comms::server::CommsInboxRes;
    use crate::console::comms::server::handle_comms_channel2;
    use crate::world::script::engine::RuntimeHost;
    use crate::world::script::schedule::PendingCallbacks;

    const PATH: &str = "fixture/scripted.toml#script.setup";

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
    fn compile_fixture(body: &str) -> WorldScriptRuntime {
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
}
