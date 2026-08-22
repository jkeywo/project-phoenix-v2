use super::*;
use crate::comms::content::OpenCommsRequest;
use crate::comms::server::CommsInboxRes;
use crate::console::comms::server::handle_comms_channel2;

/// The virtual path an inline `[script] setup = …` block compiles under.
/// Shared with the `console::comms::server` tests that seat a scripted
/// dialogue, so both name the same unit.
pub(crate) const PATH: &str = "fixture/scripted.toml#script.setup";

/// Build a `WorldScriptRuntime` from an inline-`[script]` fixture the SAME
/// way `compile_world_scripts` does in production, through the single fixture
/// compiler (`world::script::fixture`, issue #1215).
pub(crate) fn compile_fixture(body: &str) -> WorldScriptRuntime {
    crate::world::script::fixture::compile_world_runtime(
        "fixture/scripted.toml",
        &format!("[script]\nsetup = '''{body}'''\n"),
    )
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
    let script = &dialogue.script;
    assert_eq!(script.script_path, PATH);
    assert_eq!(script.node_fn, "hail_axiom");
    assert_eq!(
        script.on_pick,
        vec!["on_ack".to_string(), "on_decline".to_string()],
        "the on_pick names are parallel to the shown responses"
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
        crate::core::messages::ObjectiveStatus::Completed,
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
        .push(crate::core::messages::CommsContact {
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
        crate::core::messages::ObjectiveStatus::Completed,
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

/// The synthetic-sender escape, carried over from the deleted
/// `inject_comms_templates`: an
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

use crate::comms::server::tests::{comms_test_app, push_msg, setup_game_with_comms, tick};
use crate::core::messages::{ClientMessage, ObjectiveStatus};

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
                .after(crate::console::comms::server::handle_clear_comms)
                .before(crate::console::comms::server::handle_comms_channel2),
        );
    app
}

/// Seat the scripted thread: its trigger + handler table, and the objective
/// its picks resolve.
///
/// It used to seat a DECLARATIVE twin beside it — the same body, the same
/// two responses, the same two objective actions authored as `[[comms]]`
/// tables — and the tests below compared the two wire-for-wire and
/// effect-for-effect. Issue #985 deleted the front-end that authored the
/// twin, so what those tests can still pin is the concrete shape they always
/// asserted alongside the comparison, which is the half that pinned
/// behaviour rather than equality-to-itself.
fn seat_scripted_thread(app: &mut App) {
    setup_game_with_comms(app, STATION_UUID);
    app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
        "script_obj",
        "identify the ship",
        true,
        vec![],
    );
    let mut sr = compile_fixture(SCRIPTED_THREAD);
    {
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        crate::world::server::merge_script_triggers(&mut runtime.trigger_states, &mut sr);
    }
    app.world_mut().insert_resource(sr);
}

fn hail(app: &mut App) {
    push_msg(
        app,
        "comms",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::Hail {
                target_uuid: STATION_UUID.into(),
            },
        },
    );
    let _ = tick(app);
}

/// The one message the hail delivered.
fn only_message(app: &App) -> crate::core::messages::CommsMessage {
    let messages = app.world().resource::<CommsInboxRes>().0.messages();
    assert_eq!(messages.len(), 1, "the hail opens exactly one thread");
    messages[0].clone()
}

fn respond(app: &mut App, message_id: &str, response_index: usize) {
    push_msg(
        app,
        "comms",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::RespondToMessage {
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

/// What the Comms officer sees when a scripted thread opens: the authored
/// body and the authored response texts and `important` flags, in order.
#[test]
fn a_scripted_thread_delivers_its_authored_body_and_responses() {
    let mut app = live_comms_app();
    seat_scripted_thread(&mut app);
    hail(&mut app);

    let script = only_message(&app);
    assert_eq!(script.sender_uuid, STATION_UUID);
    assert_eq!(script.sender_name, "Starbase Alpha");
    assert!(!script.is_urgent);
    assert!(script.sender_in_range);
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

/// Each pick runs its own `on_pick` fn, and the objective moves the way that
/// fn says: response 0 completes it, response 1 fails it.
#[test]
fn each_scripted_response_applies_its_own_on_picks_effects() {
    for (index, expected) in [
        (0usize, ObjectiveStatus::Completed),
        (1usize, ObjectiveStatus::Failed),
    ] {
        let mut app = live_comms_app();
        seat_scripted_thread(&mut app);
        hail(&mut app);
        let script = only_message(&app);

        respond(&mut app, &script.id, index);

        assert_eq!(status(&app, "script_obj"), expected, "choice {index}");
    }
}

/// R1's proof, and the end of the warn-drop era for dialogue effects: a
/// scripted `on_pick` that spawns — a NAME-RESOLVING `BufferedEffect::Action`,
/// the kind the old effect-only dialogue entry point dropped with a warning —
/// resolves through `dispatch_action` and the entity EXISTS afterwards, with
/// its name registered for later triggers to resolve.
#[test]
fn a_scripted_on_pick_that_spawns_resolves_through_dispatch() {
    crate::entities::config_cache::insert_native_config(
        "fixture/comms_escort.toml".to_string(),
        crate::entities::config::EntityConfig::from_toml("").unwrap(),
    );
    let mut app = live_comms_app();
    setup_game_with_comms(&mut app, STATION_UUID);
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
    let script = only_message(&app);
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

// ── A promise gates a dialogue option, player-driven (issue #1029) ────────

/// One thread, four player picks, and the option list changing under them
/// because of a promise the player made three ticks earlier.
///
/// This is issue #1029's live consumer at full strength: the promise is
/// recorded by a real `RespondToMessage` travelling the real Input chain,
/// and the option it unlocks is offered by a node fn running in a **later
/// tick, against the live ledger** — not against the per-call snapshot the
/// recording handler mutated. Nothing here reads a script's own account of
/// what it did; every assertion is on what reached the inbox.
///
/// The chain, and what each link proves:
///
///   1. `root` offers "give your word" because the ledger has never heard of
///      `safe_passage` — the "unknown" state, doing real work.
///   2. Picking it records the promise. The LIVE ledger moves.
///   3. `status`, entered on a later tick, offers "your people are through"
///      because the promise reads `open` — the gate, opened by state the
///      previous call committed.
///   4. Picking THAT keeps the promise: the campaign flag is written, and
///      re-entering `root` no longer offers to give a word already given.
#[test]
fn a_player_pick_records_a_promise_that_unlocks_a_later_dialogue_option() {
    let mut app = live_comms_app();
    setup_game_with_comms(&mut app, STATION_UUID);
    let mut sr = compile_fixture(
        r#"
on_hailed("starbase_alpha", "hail_handler");
fn hail_handler(ctx) {
ctx.effects.open_comms(#{ from: "starbase_alpha", node_fn: "root" });
}

// The gate. Which options exist is ordinary control flow over the ledger —
// there is no `when:` field on a response, and #1029 does not add one.
fn root(ctx) {
let responses = [ #{ text: "Nothing to report.", on_pick: "on_stall" } ];
if ctx.commitments.state("safe_passage") == "unknown" {
    responses.push(#{ text: "You have my word.", on_pick: "on_promise" });
}
#{ message: "Committee to Phoenix. Where do we stand?", responses: responses }
}

fn on_stall(ctx) { }

// The beat that gives the word. Returns a follow-up whose own on_pick runs on a
// LATER tick — which is what makes the next node's read a read of the live
// ledger rather than of this call's snapshot.
fn on_promise(ctx) {
ctx.commitments.record(#{
    id: "safe_passage",
    made_to: "skyway_strike_committee",
    terms: "fixture.terms",
    resolves_when: "fixture.resolves",
});
#{ message: "Understood, Phoenix.", responses: [
    #{ text: "Stand by.", on_pick: "status" },
] }
}

fn status(ctx) {
let responses = [ #{ text: "Still working.", on_pick: "on_stall" } ];
if ctx.commitments.state("safe_passage") == "open" {
    responses.push(#{ text: "Your people are through.", on_pick: "on_honour" });
}
#{ message: "Committee standing by.", responses: responses }
}

fn on_honour(ctx) {
ctx.commitments.keep("safe_passage");
// Re-entering the FIRST node, now that the promise is spent: the offer to
// give a word already given must be gone.
root(ctx)
}
"#,
    );
    {
        let mut runtime = app.world_mut().resource_mut::<WorldContentRuntime>();
        crate::world::server::merge_script_triggers(&mut runtime.trigger_states, &mut sr);
    }
    app.world_mut().insert_resource(sr);

    /// The response texts on the most recently delivered message, plus its
    /// id — everything a player can see and act on, read off the inbox.
    fn latest(app: &App) -> (String, Vec<String>) {
        let messages = app.world().resource::<CommsInboxRes>().0.messages();
        let msg = messages.last().expect("a message was delivered").clone();
        (
            msg.id,
            msg.responses.iter().map(|r| r.text.clone()).collect(),
        )
    }

    // 1. The unknown state doing real work: the offer exists because no such
    //    promise has ever been made.
    hail(&mut app);
    let (root_id, offered) = latest(&app);
    assert_eq!(
        offered,
        vec![
            "Nothing to report.".to_string(),
            "You have my word.".to_string()
        ],
        "an unmade promise reads as unknown, and that is what offers the word"
    );

    // 2. The pick writes the LIVE ledger, through the real response path.
    respond(&mut app, &root_id, 1);
    assert_eq!(
        app.world()
            .resource::<WorldContentRuntime>()
            .commitments
            .state_of("safe_passage"),
        "open",
        "a player's choice put the promise on the books"
    );

    // 3. THE GATE: `status` runs on a later tick, reads the live ledger, and
    //    offers an option that did not exist before the promise was made.
    let (ack_id, _) = latest(&app);
    respond(&mut app, &ack_id, 0);
    let (status_id, offered) = latest(&app);
    assert_eq!(
        offered,
        vec![
            "Still working.".to_string(),
            "Your people are through.".to_string()
        ],
        "the option is offered because a promise made in an EARLIER call is \
         open in the live ledger"
    );

    // 4. Picking it settles the promise and writes the campaign flag; the
    //    first node no longer offers a word already given.
    respond(&mut app, &status_id, 1);
    let runtime = app.world().resource::<WorldContentRuntime>();
    assert_eq!(runtime.commitments.state_of("safe_passage"), "kept");
    assert_eq!(
        runtime.flags.counter("commitment.safe_passage.kept"),
        1,
        "and keeping it wrote the campaign flag an on_flag_set trigger watches"
    );
    assert_eq!(runtime.flags.counter("commitment.safe_passage.broken"), 0);

    let (_, offered) = latest(&app);
    assert_eq!(
        offered,
        vec!["Nothing to report.".to_string()],
        "the gate closes as well as it opens: the word has been given, so the \
         option to give it is gone"
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
    let runtime =
        crate::world::script::fixture::compile_world_runtime("assets/worlds/default.toml", text);
    let path = runtime
        .asts
        .keys()
        .next()
        .cloned()
        .expect("default.toml lifts one inline script unit");
    (runtime, path)
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
        &crate::world::deadlines::DeadlineTable::default(),
        &crate::world::commitments::CommitmentLedger::default(),
        &crate::dossier::evidence::EvidenceLog::default(),
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
/// delivers nothing at all, and the new system leaves no trace.
///
/// It used to assert the DECLARATIVE twin still arrived; issue #985 deleted
/// that front-end, so a hail into a script-free world is now exactly a
/// recorded hail and a `WorldEvent::Hailed` nobody is listening for.
#[test]
fn a_hail_on_a_script_free_world_delivers_nothing() {
    let mut app = live_comms_app();
    setup_game_with_comms(&mut app, STATION_UUID);
    hail(&mut app);

    assert!(
        app.world()
            .resource::<CommsInboxRes>()
            .0
            .messages()
            .is_empty(),
        "no front-end answered the hail, so no message is delivered"
    );
    let comms = app.world().resource::<CommsRuntime>();
    assert!(comms.active_dialogues.is_empty());
    assert!(
        comms.open_hails.contains(STATION_UUID),
        "the hail itself is still recorded — that is what re-arms the AI gate"
    );
}

// ── One-way announcement reports (combat_test.toml's shape) ───────────────
//
// `combat_test.toml` fires a series of one-way announcement reports
// (mission brief, wave clears, hull-threshold warnings) as `on_timer` /
// `on_hull_below` handlers that each `open_comms` a node with no
// responses. That queue-then-drain mechanism is what this test exercises,
// on a synthetic three-report script rather than the shipped world: the
// report ids are covered by `check-strings.mjs --strict`, and the
// shipped script's own trigger wiring is proven to compile and lint
// clean by `headless::app`'s hard-fail on `has_error` at load — pinning
// the shipped world's exact report list and order here would just be
// re-asserting its authored content in Rust.

/// Several queued one-way reports reach the inbox in authored order, each
/// keeping its own body and urgency — the mechanism every announcement
/// report in the shipped fleet's worlds relies on.
#[test]
fn queued_one_way_reports_reach_the_inbox_in_order_with_their_body_and_urgency() {
    let mut app = scripted_comms_app();
    let mut sr = compile_fixture(
        r#"
        fn report_a(ctx) { #{ message: "Report A." } }
        fn report_b(ctx) { #{ message: "Report B." } }
        fn report_c(ctx) { #{ message: "Report C." } }
        "#,
    );
    let reports = [("report_a", false), ("report_b", true), ("report_c", false)];
    sr.pending_comms_opens = reports
        .iter()
        .map(|(root_fn, urgent)| OpenCommsRequest {
            urgent: *urgent,
            ..request(root_fn, "starbase")
        })
        .collect();
    app.world_mut().insert_resource(sr);

    app.update();

    let messages = app.world().resource::<CommsInboxRes>().0.messages();
    assert_eq!(
        messages
            .iter()
            .map(|m| (m.body.as_str(), m.is_urgent))
            .collect::<Vec<_>>(),
        vec![
            ("Report A.", false),
            ("Report B.", true),
            ("Report C.", false),
        ],
        "each queued report reaches the inbox in order with its own body and urgency"
    );
    assert!(
        messages.iter().all(|m| m.responses.is_empty()),
        "one-way reports carry no responses"
    );
}
