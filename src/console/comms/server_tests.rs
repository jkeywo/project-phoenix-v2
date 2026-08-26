use super::*;
use crate::comms::server::CommsInboxRes;
use crate::core::messages::CommsMessage;
use crate::server_app::{LocalShip, Ship, ShipSystemBlackboards};

fn msg(id: &str) -> CommsMessage {
    CommsMessage {
        id: id.into(),
        sender_uuid: "sender-uuid".into(),
        sender_name: "Station Alpha".into(),
        subject: "Test".into(),
        body: "Body text".into(),
        body_params: Default::default(),
        responses: vec![crate::core::messages::CommsResponseView {
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
    crate::ai::host::register_ai_host_env(&mut app);
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
    let key = SystemId(crate::ship::system_registry::COMMS_SYSTEM_ID.to_string());
    let SystemBlackboard::Comms(bb) = bbs.0.get(&key).expect("comms blackboard missing").clone()
    else {
        panic!("wrong blackboard variant");
    };
    bb
}

// ── comms objective visibility (issue #752, objective-visibility-policy) ──

#[test]
fn comms_bb_hides_zero_score_doctrine_objective() {
    use crate::core::messages::{AiDirective, ObjectiveSource};
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

    let key = SystemId(crate::ship::system_registry::COMMS_SYSTEM_ID.to_string());
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
    ai_resolver.set(
        crate::ship::system_registry::comms_system_id(),
        ControlSource::Ai,
    );
    let ai_sources = ShipSystemControlSources(ai_resolver);
    let ai_policy = ai_sources
        .0
        .policy_for(&crate::ship::system_registry::comms_system_id());
    assert!(
        ai_policy.operate_ai,
        "AI Comms must gate through operate_ai"
    );

    let mut human_resolver = ControlSourceResolver::new();
    human_resolver.set(
        crate::ship::system_registry::comms_system_id(),
        ControlSource::Human,
    );
    let human_sources = ShipSystemControlSources(human_resolver);
    let human_policy = human_sources
        .0
        .policy_for(&crate::ship::system_registry::comms_system_id());
    assert!(!human_policy.operate_ai, "Human Comms must not operate AI");
}

// ── Backfill Comms AI hail execution (issue #753) ──────────────────────

use crate::core::messages::{AdmittedCommands, AiDirective, ObjectiveSource, SystemControlPayload};
use crate::objectives::{ObjectiveManager, UtilityConfig, ZeroGateCondition};
use crate::ship::control_source::{ControlSource, ControlSourceResolver};

/// Minimal app that runs ONLY `operate_comms_ai` (no `handle_hail`, no
/// AdmissionPlugin clear) so a test can inspect the `Hail` command the AI
/// leaves in the ship's own `AdmittedCommands`. Spawns one `LocalShip`
/// whose Comms system carries `comms_source`.
fn comms_ai_app(comms_source: ControlSource) -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
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
    resolver.set(
        crate::ship::system_registry::comms_system_id(),
        comms_source,
    );
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
        .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
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
        .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
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
    npc_resolver.set(
        crate::ship::system_registry::comms_system_id(),
        ControlSource::Ai,
    );
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
            .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
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
        .push(crate::core::messages::CommsContact {
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
    use crate::core::messages::AdmittedCommand;
    let mut q = app
        .world_mut()
        .query_filtered::<&mut AdmittedCommands, With<crate::server_app::LocalShip>>();
    q.single_mut(app.world_mut())
        .unwrap()
        .0
        .push(AdmittedCommand {
            target: crate::ship::system_registry::comms_system_id(),
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
fn destroyed_comms_hull() -> crate::entities::spawner::EntitySystemHull {
    let comms = crate::ship::system_registry::comms_system_id();
    let mut hull = crate::ship::damage::SystemHull::from_config(&[(comms.clone(), 100.0)]);
    hull.set_hp(&comms, 0.0);
    assert_eq!(
        hull.tier_for(&comms),
        crate::ship::damage::DamageTier::Destroyed,
        "fixture must actually destroy the Comms system"
    );
    crate::entities::spawner::EntitySystemHull(hull)
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
use crate::core::messages::{ClientMessage, ServerMessage};

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
            .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        q.single_mut(app.world_mut()).unwrap().0.set(
            crate::ship::system_registry::comms_system_id(),
            ControlSource::Ai,
        );
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
        .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
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
            .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
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
            .query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        q.single_mut(app.world_mut()).unwrap().0.set(
            crate::ship::system_registry::comms_system_id(),
            ControlSource::Ai,
        );
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
    crate::ai::host::register_ai_host_env(&mut app);
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ))
    .insert_resource(WorldContentRuntime::default())
    .insert_resource(CommsRuntime::default())
    .insert_resource(CommsInboxRes(crate::console::comms::CommsInbox::new()))
    .add_systems(Update, operate_comms_response_ai);

    let mut resolver = ControlSourceResolver::new();
    resolver.set(
        crate::ship::system_registry::comms_system_id(),
        comms_source,
    );
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
                    body_params: Default::default(),
                    responses: vec![CommsResponse {
                        text: "Acknowledge.".into(),
                        important: false,
                    }],
                },
                thread_id: id.clone(),
                script: crate::comms::content::ScriptedDialogue {
                    script_path: crate::comms::scripted::tests::PATH.to_string(),
                    origin_layer: None,
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
                .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
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
            .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
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
                .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
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
            .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
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
        .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
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
    let config = crate::entities::config::EntityConfig::from_toml(toml)
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
            body_params: Default::default(),
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
        .for_target(crate::ship::system_registry::COMMS_SYSTEM_ID)
        .filter_map(|cmd| match &cmd.payload {
            SystemControlPayload::RespondToMessage { response_index, .. } => Some(*response_index),
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
        .push(crate::core::messages::CommsContact {
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
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::RespondToMessage {
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
    use crate::server_app::Ship;

    let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456012";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, station_uuid);

    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
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
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::RespondToMessage {
                message_id: msg_id.clone(),
                response_index: 0,
            },
        },
    );
    let out = tick(&mut app);
    let (rejected_id, idx) = find_rejection(&out).expect("out-of-range response must be rejected");
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
                    body_params: Default::default(),
                    responses,
                },
                thread_id: "scripted-thread".to_string(),
                script: crate::comms::content::ScriptedDialogue {
                    script_path: crate::comms::scripted::tests::PATH.to_string(),
                    origin_layer: None,
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
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::RespondToMessage {
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
        crate::core::messages::ObjectiveStatus::Completed,
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
        crate::core::messages::ObjectiveStatus::Failed,
    );
    assert_eq!(
        app.world().resource::<CommsInboxRes>().0.messages().len(),
        1,
        "a terminal response injects nothing"
    );
}

// -- Issue #1049: `handle_clear_comms` also empties `active_dialogues` -----

/// Acceptance: seat a comms holder, open a (scripted) dialogue, issue
/// `ClearComms`, and `active_dialogues` is empty afterward — not just the
/// inbox and `open_hails`.
///
/// Also covers the "no dangling continuation" half of the acceptance
/// criteria: a `RespondToMessage` submitted against the now-cleared message
/// id must not silently resolve. It takes the shared router's ordinary
/// stale-dialogue arm — the SAME arm a late/duplicate submission against an
/// already-answered message already takes (see
/// `a_scripted_response_runs_its_on_pick_and_injects_the_follow_up` and
/// `stale_ai_response_is_rejected_by_the_shared_router`) — and is rejected
/// with `CommsResponseRejected` rather than re-running `on_pick` against
/// dropped state.
#[test]
fn clear_comms_empties_active_dialogues_and_a_cleared_dialogue_cannot_be_answered() {
    let station_uuid = "a1b2c3d4-e5f6-4789-abcd-ef0123456987";
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
        vec!["on_ack", "on_decline"],
        false,
    );
    assert!(
        app.world()
            .resource::<CommsRuntime>()
            .active_dialogues
            .contains_key(&id),
        "sanity: the dialogue is seated before the clear"
    );

    push_msg(
        &mut app,
        "comms",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::ClearComms,
        },
    );
    let _ = tick(&mut app);

    assert!(
        app.world()
            .resource::<CommsRuntime>()
            .active_dialogues
            .is_empty(),
        "ClearComms must retire every active dialogue, not just the inbox \
         and open_hails"
    );

    // No dangling continuation fires: the mid-dialogue mechanism this
    // codebase has (a follow-up node from `on_pick`) can only ever be
    // reached by answering the SAME message id again, so proving that path
    // is refused is proving there is nothing left to dangle.
    let out = respond(&mut app, &id, 0);
    let (rejected_id, idx) = find_rejection(&out).expect(
        "a response against a cleared dialogue must be rejected, not \
         silently accepted",
    );
    assert_eq!(rejected_id, id);
    assert_eq!(idx, 0);
    assert!(
        !app.world()
            .resource::<CommsInboxRes>()
            .0
            .messages()
            .iter()
            .any(|m| m.body == "Docking clamps released."),
        "the rejected on_pick's follow-up node must never be injected"
    );
    assert!(
        app.world()
            .resource::<CommsRuntime>()
            .active_dialogues
            .is_empty(),
        "the rejected submission must not resurrect an active_dialogues entry"
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
        crate::core::messages::ObjectiveStatus::Active,
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
        crate::core::messages::ObjectiveStatus::Completed,
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
        crate::core::messages::ObjectiveStatus::Completed,
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
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::Hail {
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
        let mut q = app.world_mut().query_filtered::<&mut crate::ship_plugin::ShipSystemControlSources, With<crate::server_app::Ship>>();
        for mut sources in q.iter_mut(app.world_mut()) {
            sources.0.set(
                crate::ship::system_registry::comms_system_id(),
                crate::ship::control_source::ControlSource::Ai,
            );
        }
    }

    push_msg(
        &mut app,
        "comms",
        ClientMessage::ControlSystem {
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::Hail {
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
        body_params: Default::default(),
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
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::ClearComms,
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
    use crate::server_app::Ship;

    let station_uuid = "station-out-of-range-hail";
    let mut app = comms_test_app();
    setup_game_with_comms(&mut app, station_uuid);

    app.world_mut().spawn((
        Ship,
        crate::server_app::LocalShip,
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
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::Hail {
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
    use crate::server_app::Ship;

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
        crate::server_app::LocalShip,
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
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::RespondToMessage {
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
        crate::core::messages::ObjectiveStatus::Active,
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
            target: crate::ship::system_registry::comms_system_id(),
            payload: crate::core::messages::SystemControlPayload::Hail {
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
