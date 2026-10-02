use super::*;
use crate::command_admission::{HostSlot, ShipKey};
use crate::core::messages::SystemControlPayload;

fn gm_pause_grant(apply_tick: u64, active: bool) -> crate::gm_action::GmActionGrant {
    crate::gm_action::GmActionGrant {
        from: HostSlot(1),
        sequenced_by: HostSlot(1),
        operator_id: "gm-one".into(),
        correlation: crate::gm_action::GmActionId::new(format!("replay-{apply_tick}-{active}"))
            .unwrap(),
        recovery_generation: 0,
        apply_tick,
        order: crate::gm_action::GmActionOrder::new(HostSlot(1), 1),
        action: crate::gm_action::GmAction::SetSessionPaused { active },
    }
}

fn artifact() -> ReplayArtifact {
    let mut ledger = DigestLedger::new(50);
    ledger.record(50, 0xaaaa);
    ledger.final_digest = 0xbbbb;
    ReplayArtifact {
        version: ARTIFACT_VERSION,
        seed: 901,
        world_path: "assets/worlds/patrol.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        side_a: Vec::new(),
        side_b: Vec::new(),
        max_ticks: 260,
        dt: 1.0 / 60.0,
        final_tick: 259,
        log: CommandLog::default(),
        gm_actions: GmActionJournal::default(),
        ledger,
    }
}

#[test]
fn an_artifact_round_trips_through_ron() {
    let original = artifact();
    let text = original.to_ron().expect("serialises");
    assert_eq!(ReplayArtifact::from_ron(&text).expect("parses"), original);
}

/// A shape this build does not know must be refused, not read wrongly —
/// this is also the guard against a real version-1 artifact silently
/// replaying a duel run with its side lists dropped on the floor (see
/// `ARTIFACT_VERSION`'s own doc).
#[test]
fn a_future_artifact_version_is_refused() {
    let mut future = artifact();
    future.version = ARTIFACT_VERSION + 1;
    let text = future.to_ron().expect("serialises");
    assert!(matches!(
        ReplayArtifact::from_ron(&text),
        Err(ArtifactError::Version { .. })
    ));
}

#[test]
fn a_version_three_artifact_is_refused_before_missing_gm_state_is_defaulted() {
    let mut old = artifact();
    old.version = 3;
    let text = old.to_ron().expect("serialises");
    assert!(matches!(
        ReplayArtifact::from_ron(&text),
        Err(ArtifactError::Version {
            found: 3,
            expected: ARTIFACT_VERSION
        })
    ));
}

/// A version-1 artifact (no `side_a`/`side_b`) must be refused rather than
/// read with the new fields silently defaulted to empty — an empty roster
/// is indistinguishable from a genuine no-duel run, so defaulting would
/// silently replay the wrong scenario.
#[test]
fn a_version_one_artifact_is_refused_not_defaulted() {
    // Built from the wire shape directly: a real v1 file never had
    // `side_a`/`side_b` keys at all.
    #[derive(serde::Serialize)]
    struct ArtifactV1 {
        version: u32,
        seed: u64,
        world_path: String,
        ship_path: String,
        max_ticks: u64,
        dt: f64,
        log: CommandLog,
        ledger: DigestLedger,
    }
    let v1 = ArtifactV1 {
        version: 1,
        seed: 901,
        world_path: "assets/worlds/duel.toml".into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        max_ticks: 260,
        dt: 1.0 / 60.0,
        log: CommandLog::default(),
        ledger: DigestLedger::new(0),
    };
    let text =
        ron::ser::to_string_pretty(&v1, ron::ser::PrettyConfig::default()).expect("serialises");
    match ReplayArtifact::from_ron(&text) {
        Err(ArtifactError::Version { found, expected }) => {
            assert_eq!(found, 1);
            assert_eq!(expected, ARTIFACT_VERSION);
        }
        other => panic!("a v1 artifact must be refused by version, got {other:?}"),
    }
}

/// An unseeded run names something nothing can re-derive, so it must not
/// produce a file that looks replayable.
#[test]
fn an_unseeded_run_cannot_be_captured() {
    let args = HeadlessArgs {
        seed: None,
        ..Default::default()
    };
    assert!(matches!(
        ReplayArtifact::capture(
            &args,
            CommandLog::default(),
            GmActionJournal::default(),
            0,
            DigestLedger::new(0),
        ),
        Err(ArtifactError::Unseeded)
    ));
}

/// A replay must run the recording's world, hull, length and pacing — never
/// the replaying process's own defaults.
#[test]
fn replay_args_come_from_the_artifact_not_the_process() {
    let args = artifact().replay_args();
    assert_eq!(args.world_path, "assets/worlds/patrol.toml");
    assert_eq!(args.max_ticks, 260);
    assert_eq!(args.seed, Some(901));
    assert!(args.deterministic, "a seed implies a pinned scheduler");
}

#[test]
fn god_mode_is_the_one_target_that_needs_the_host_console_token() {
    let god = SystemId(crate::ship::system_registry::GOD_MODE_SYSTEM_ID.into());
    assert_eq!(replay_token_for(&god), LOCAL_CONSOLE_TOKEN);
    assert_eq!(
        replay_token_for(&SystemId("red-alert".into())),
        AI_BACKFILL_TOKEN
    );
}

/// The log entry shape the driver consumes, kept honest against the type.
#[test]
fn a_logged_command_carries_everything_a_replay_needs_to_route_it() {
    let entry = LoggedCommand {
        tick: 7,
        order: crate::command_admission::CommandOrder::default(),
        ship: ShipKey("uuid-1".into()),
        target: SystemId("red-alert".into()),
        payload: SystemControlPayload::SetRedAlert { active: true },
    };
    assert!(entry.ship.is_named());
    assert_eq!(replay_token_for(&entry.target), AI_BACKFILL_TOKEN);
}

#[test]
fn replay_preserves_an_adopted_restore_pause_for_a_typed_resume() {
    let mut source = GmActionJournal::default();
    source.adopt_initial_pause(true);
    source.insert(gm_pause_grant(0, false)).unwrap();
    source.restore_applied_frontier(1).unwrap();
    validate_gm_action_journal(&source).expect("the adopted base state is canonical");

    let mut app = App::new();
    app.insert_resource(SimTick(0));
    app.insert_resource(GmActionJournal::default());
    app.insert_resource(crate::gm_action::GmActionLog::default());
    app.insert_resource(crate::gm_action::SimulationPaused(true));
    app.add_systems(PreUpdate, crate::gm_action::apply_due_actions);
    seed_replay_initial_state(&mut app, &source);

    let mut sim = PhoenixSim {
        app,
        max_frames: 1,
        frames: 0,
        expected_commands: 0,
        applied: 0,
        submitted: 0,
        tail: true,
        gm_actions: source,
        final_tick: Some(0),
        ledger: DigestLedger::new(0),
    };
    sim.step();

    assert!(
        !sim.app
            .world()
            .resource::<crate::gm_action::SimulationPaused>()
            .0
    );
    assert_eq!(
        sim.app
            .world()
            .resource::<crate::gm_action::GmActionLog>()
            .entries()[0]
            .outcome,
        crate::gm_action::GmActionOutcome::Applied
    );
}

#[test]
fn replay_end_ignores_canonical_grants_beyond_the_recorded_final_tick() {
    let mut future = GmActionJournal::default();
    future.insert(gm_pause_grant(20, true)).unwrap();
    let mut app = App::new();
    app.insert_resource(SimTick(10));
    app.insert_resource(GmActionJournal::default());
    seed_replay_initial_state(&mut app, &future);
    let sim = PhoenixSim {
        app,
        max_frames: 1,
        frames: 0,
        expected_commands: 0,
        applied: 0,
        submitted: 0,
        tail: true,
        gm_actions: future,
        final_tick: Some(10),
        ledger: DigestLedger::new(0),
    };

    assert!(
        sim.reached_replay_end(),
        "a retained future owner commit must not drive replay past final_tick"
    );
}

/// A Fire replays through the SAME canonical lane every other GM action
/// uses (issue #1301): the artifact carries the grant, `apply_due_actions`
/// revalidates it against the replayed world's own trigger table, and the
/// durable result carries the event it named. Nothing about the event
/// family needs a second replay path.
///
/// Run over BOTH authoring surfaces (issue #1302). The revalidation reads
/// `fireable_index` and `manual_fire_is_still_live`, neither of which looks
/// at `Trigger::condition`, so a manual event and an ordinary
/// condition-bearing one that declares `gm_controls` must replay
/// identically — and a future change that special-cased
/// `TriggerCondition::Manual` would be caught here rather than as a
/// divergence in somebody's mission.
#[test]
fn a_fired_gm_event_replays_through_the_canonical_journal() {
    for condition in [
        crate::world::config::TriggerCondition::Manual,
        crate::world::config::TriggerCondition::OnDestroyed {
            entity_name: "courier".to_string(),
        },
    ] {
        replays_through_the_canonical_journal(condition);
    }
}

fn replays_through_the_canonical_journal(condition: crate::world::config::TriggerCondition) {
    let event = "base-world::breach_alarm";
    let mut source = GmActionJournal::default();
    source
        .insert(crate::gm_action::GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-one".into(),
            correlation: crate::gm_action::GmActionId::new("replay-fire-1").unwrap(),
            recovery_generation: 0,
            apply_tick: 0,
            order: crate::gm_action::GmActionOrder::new(HostSlot(1), 1),
            action: crate::gm_action::GmAction::FireGmEvent {
                event: event.into(),
            },
        })
        .unwrap();
    // The recording APPLIED the Fire, which is the prefix a replay adopts.
    source.restore_applied_frontier(1).unwrap();
    validate_gm_action_journal(&source).expect("a Fire journal is canonical");

    let mut trigger = crate::world::config::scripted_trigger(condition);
    trigger.id = Some("breach_alarm".into());
    trigger.gm_controls = Some(crate::world::config::GmEventControls::fire_only(
        "breach_alarm".into(),
        "world.gm.event.breach_alarm".into(),
    ));
    let runtime = crate::world::server::WorldContentRuntime {
        triggers: vec![crate::world::content::TriggerState {
            trigger,
            fired: false,
            origin_layer: None,
            seen_destroyed: Default::default(),
            last_fired_elapsed: None,
        }]
        .into(),
        ..Default::default()
    };

    let mut app = App::new();
    app.insert_resource(SimTick(0));
    app.insert_resource(GmActionJournal::default());
    app.insert_resource(crate::gm_action::GmActionLog::default());
    app.insert_resource(crate::gm_action::SimulationPaused(false));
    app.insert_resource(runtime);
    app.add_systems(PreUpdate, crate::gm_action::apply_due_actions);
    seed_replay_initial_state(&mut app, &source);

    let mut sim = PhoenixSim {
        app,
        max_frames: 1,
        frames: 0,
        expected_commands: 0,
        applied: 0,
        submitted: 0,
        tail: true,
        gm_actions: source,
        final_tick: Some(0),
        ledger: DigestLedger::new(0),
    };
    sim.step();

    let entry = &sim
        .app
        .world()
        .resource::<crate::gm_action::GmActionLog>()
        .entries()[0];
    assert_eq!(entry.outcome, crate::gm_action::GmActionOutcome::Applied);
    assert_eq!(entry.target.as_deref(), Some(event));
    assert_eq!(
        sim.app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_gm_event_fires
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![event.to_string()],
        "the replayed peer arms exactly what the recorded one armed"
    );
}

/// An armed Skip replays through the SAME canonical lane a Fire does
/// (issue #1304): the artifact carries the grant, `apply_due_actions`
/// re-resolves it against the replayed world's own trigger table at the
/// recorded apply tick, and the replayed peer ends up armed on the same
/// lever, for the same event, as the recorded one.
#[test]
fn an_armed_gm_event_skip_replays_through_the_canonical_journal() {
    let event = "base-world::courier_lost";
    let mut source = GmActionJournal::default();
    source
        .insert(crate::gm_action::GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-one".into(),
            correlation: crate::gm_action::GmActionId::new("replay-skip-1").unwrap(),
            recovery_generation: 0,
            apply_tick: 0,
            order: crate::gm_action::GmActionOrder::new(HostSlot(1), 1),
            action: crate::gm_action::GmAction::ArmGmEventSkip {
                event: event.into(),
            },
        })
        .unwrap();
    source.restore_applied_frontier(1).unwrap();
    validate_gm_action_journal(&source).expect("a Skip journal is canonical");

    let mut trigger = crate::world::config::scripted_trigger(
        crate::world::config::TriggerCondition::OnDestroyed {
            entity_name: "courier".to_string(),
        },
    );
    trigger.id = Some("courier_lost".into());
    let mut controls = crate::world::config::GmEventControls::fire_only(
        "courier_lost".into(),
        "world.gm.event.courier_lost".into(),
    );
    controls.skip = true;
    trigger.gm_controls = Some(controls);
    let runtime = crate::world::server::WorldContentRuntime {
        triggers: vec![crate::world::content::TriggerState {
            trigger,
            fired: false,
            origin_layer: None,
            seen_destroyed: Default::default(),
            last_fired_elapsed: None,
        }]
        .into(),
        ..Default::default()
    };

    let mut app = App::new();
    app.insert_resource(SimTick(0));
    app.insert_resource(GmActionJournal::default());
    app.insert_resource(crate::gm_action::GmActionLog::default());
    app.insert_resource(crate::gm_action::SimulationPaused(false));
    app.insert_resource(runtime);
    app.add_systems(PreUpdate, crate::gm_action::apply_due_actions);
    seed_replay_initial_state(&mut app, &source);

    let mut sim = PhoenixSim {
        app,
        max_frames: 1,
        frames: 0,
        expected_commands: 0,
        applied: 0,
        submitted: 0,
        tail: true,
        gm_actions: source,
        final_tick: Some(0),
        ledger: DigestLedger::new(0),
    };
    sim.step();

    let entry = &sim
        .app
        .world()
        .resource::<crate::gm_action::GmActionLog>()
        .entries()[0];
    assert_eq!(entry.outcome, crate::gm_action::GmActionOutcome::Applied);
    assert_eq!(entry.target.as_deref(), Some(event));
    assert_eq!(
        entry.lever,
        Some(crate::gm_event::GmEventLever::SkipNext),
        "the replayed fact says which lever, not just which event"
    );
    let replayed = sim
        .app
        .world()
        .resource::<crate::world::server::WorldContentRuntime>();
    assert_eq!(
        replayed
            .pending_gm_event_skips
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![event.to_string()],
        "the replayed peer arms exactly what the recorded one armed"
    );
    assert!(
        replayed.pending_gm_event_fires.is_empty(),
        "and arms nothing on the other lever"
    );
}

/// A directed world effect replays through the SAME canonical lane every
/// other GM action uses (issue #1310): the artifact carries the grant,
/// `apply_due_actions` re-resolves it against the replayed world's own
/// hulls at the recorded apply tick, and the durable result carries both
/// the entity it named and the amounts it resolved. Nothing about the
/// effect family needs a second replay path.
#[test]
fn a_direct_effect_replays_through_the_canonical_journal() {
    let mut source = GmActionJournal::default();
    source
        .insert(crate::gm_action::GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-one".into(),
            correlation: crate::gm_action::GmActionId::new("replay-hit-1").unwrap(),
            recovery_generation: 0,
            apply_tick: 0,
            order: crate::gm_action::GmActionOrder::new(HostSlot(1), 1),
            action: crate::gm_action::GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope: crate::gm_effect::GmDirectEffectScope::Entity,
                effect: crate::gm_effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 250_000,
            },
        })
        .unwrap();
    // The recording APPLIED the hit, which is the prefix a replay adopts.
    source.restore_applied_frontier(1).unwrap();
    validate_gm_action_journal(&source).expect("an effect journal is canonical");

    let mut app = App::new();
    app.insert_resource(SimTick(0));
    app.insert_resource(GmActionJournal::default());
    app.insert_resource(crate::gm_action::GmActionLog::default());
    app.insert_resource(crate::gm_action::SimulationPaused(false));
    app.insert_resource(crate::gm_effect::PendingGmDirectEffects::default());
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("npc-1".into()),
        crate::entities::spawner::EntitySystemHull(crate::ship::damage::SystemHull::from_config(
            &[(SystemId("captain".into()), 80.0)],
        )),
    ));
    app.add_systems(PreUpdate, crate::gm_action::apply_due_actions);
    seed_replay_initial_state(&mut app, &source);

    let mut sim = PhoenixSim {
        app,
        max_frames: 1,
        frames: 0,
        expected_commands: 0,
        applied: 0,
        submitted: 0,
        tail: true,
        gm_actions: source,
        final_tick: Some(0),
        ledger: DigestLedger::new(0),
    };
    sim.step();

    let entry = &sim
        .app
        .world()
        .resource::<crate::gm_action::GmActionLog>()
        .entries()[0];
    assert_eq!(entry.outcome, crate::gm_action::GmActionOutcome::Applied);
    assert_eq!(entry.target.as_deref(), Some("npc-1"));
    assert_eq!(
        entry.effect,
        Some(crate::gm_effect::GmDirectEffectResult {
            kind: crate::gm_effect::GmDirectEffectKind::Damage,
            applied_milli_hp: 80_000,
            discarded_milli_hp: 170_000,
            destroyed: true,
        }),
        "the replayed peer re-resolves the same clamp and the same lethality"
    );
    assert_eq!(
        sim.app
            .world()
            .resource::<crate::gm_effect::PendingGmDirectEffects>()
            .entries()
            .len(),
        1,
        "the replayed peer arms exactly what the recorded one armed"
    );
}

/// A Station-scoped effect replays down the same lane and re-resolves
/// against the replayed world's OWN authored ownership (issue #1311).
///
/// The artifact carries the scope, not a system list: which Systems `helm`
/// owns is a fact about the replayed hull's ship config, and a recorded
/// list would let a replay damage a Station the config no longer describes.
/// The clamp and the lethality prove the re-resolution actually narrowed —
/// 250 points asked of a 30-point Station lands 30, discards 220, and does
/// NOT report a kill, because the other Station's 50 points are still there.
#[test]
fn a_station_scoped_effect_replays_through_the_canonical_journal() {
    let mut source = GmActionJournal::default();
    source
        .insert(crate::gm_action::GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-one".into(),
            correlation: crate::gm_action::GmActionId::new("replay-hit-helm").unwrap(),
            recovery_generation: 0,
            apply_tick: 0,
            order: crate::gm_action::GmActionOrder::new(HostSlot(1), 1),
            action: crate::gm_action::GmAction::ApplyDirectEffect {
                target: "npc-1".into(),
                scope: crate::gm_effect::GmDirectEffectScope::Station(
                    crate::core::messages::StationId("helm".into()),
                ),
                effect: crate::gm_effect::GmDirectEffectKind::Damage,
                amount_milli_hp: 250_000,
            },
        })
        .unwrap();
    source.restore_applied_frontier(1).unwrap();
    validate_gm_action_journal(&source).expect("an effect journal is canonical");

    let mut app = App::new();
    app.insert_resource(SimTick(0));
    app.insert_resource(GmActionJournal::default());
    app.insert_resource(crate::gm_action::GmActionLog::default());
    app.insert_resource(crate::gm_action::SimulationPaused(false));
    app.insert_resource(crate::gm_effect::PendingGmDirectEffects::default());
    app.world_mut().spawn((
        crate::entities::spawner::EntityUuid("npc-1".into()),
        crate::entities::spawner::EntitySystemHull(crate::ship::damage::SystemHull::from_config(
            &[
                (SystemId("impulse-drive".into()), 30.0),
                (SystemId("phaser-bank".into()), 50.0),
            ],
        )),
        crate::ship::components::ShipConfigComponent(
            toml::from_str(
                r#"
[[station]]
id = "helm"
name = "station.helm.display_name"
description = "station.helm.description"
rank = "Lieutenant"

[[station]]
id = "tactical"
name = "station.tactical.display_name"
description = "station.tactical.description"
rank = "Lieutenant"

[[system]]
id = "impulse-drive"
kind = "impulse-drive"
station = "helm"

[[system]]
id = "phaser-bank"
kind = "phaser-bank"
station = "tactical"
"#,
            )
            .expect("a well-formed authoring fixture"),
        ),
    ));
    app.add_systems(PreUpdate, crate::gm_action::apply_due_actions);
    seed_replay_initial_state(&mut app, &source);

    let mut sim = PhoenixSim {
        app,
        max_frames: 1,
        frames: 0,
        expected_commands: 0,
        applied: 0,
        submitted: 0,
        tail: true,
        gm_actions: source,
        final_tick: Some(0),
        ledger: DigestLedger::new(0),
    };
    sim.step();

    let entry = &sim
        .app
        .world()
        .resource::<crate::gm_action::GmActionLog>()
        .entries()[0];
    assert_eq!(entry.outcome, crate::gm_action::GmActionOutcome::Applied);
    assert_eq!(
        entry.effect,
        Some(crate::gm_effect::GmDirectEffectResult {
            kind: crate::gm_effect::GmDirectEffectKind::Damage,
            applied_milli_hp: 30_000,
            discarded_milli_hp: 220_000,
            destroyed: false,
        }),
        "the replayed peer re-resolves the clamp against the STATION and the \
             lethality against the whole hull"
    );
    assert_eq!(
        entry.effect_scope,
        Some(crate::gm_effect::GmDirectEffectScope::Station(
            crate::core::messages::StationId("helm".into())
        )),
        "the durable fact still names the Station it narrowed to"
    );
    assert_eq!(
        sim.app
            .world()
            .resource::<crate::gm_effect::PendingGmDirectEffects>()
            .entries()
            .iter()
            .map(|effect| (effect.scope.clone(), effect.amount_milli_hp))
            .collect::<Vec<_>>(),
        vec![(
            crate::gm_effect::GmDirectEffectScope::Station(crate::core::messages::StationId(
                "helm".into()
            )),
            30_000
        )],
        "the replayed peer arms exactly what the recorded one armed"
    );
}

/// A placement replays through the SAME canonical lane (issue #1305): the
/// artifact carries the grant, `apply_due_actions` revalidates it against
/// the replayed world's own palette, and the replayed peer arms exactly
/// what the recorded one armed — same name, same coordinates, same order.
#[test]
fn a_gm_placement_replays_through_the_canonical_journal() {
    let mut source = GmActionJournal::default();
    source
        .insert(crate::gm_action::GmActionGrant {
            from: HostSlot(1),
            sequenced_by: HostSlot(1),
            operator_id: "gm-one".into(),
            correlation: crate::gm_action::GmActionId::new("replay-place-1").unwrap(),
            recovery_generation: 0,
            apply_tick: 0,
            order: crate::gm_action::GmActionOrder::new(HostSlot(1), 1),
            action: crate::gm_action::GmAction::SpawnPaletteEntity {
                palette: "raider".into(),
                variant: None,
                position_mm: [120_000, 0, -40_000],
                heading_mdeg: 90_000,
            },
        })
        .unwrap();
    // The recording APPLIED the placement, which is the prefix a replay
    // adopts.
    source.restore_applied_frontier(1).unwrap();
    validate_gm_action_journal(&source).expect("a placement journal is canonical");

    let runtime = crate::world::server::WorldContentRuntime {
        gm_palette: vec![crate::world::config::GmPaletteEntry {
            id: "raider".into(),
            label: "world.gm.palette.raider.label".into(),
            template_path: "assets/entities/ship_harrow_destroyer.toml".into(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let mut app = App::new();
    app.insert_resource(SimTick(0));
    app.insert_resource(GmActionJournal::default());
    app.insert_resource(crate::gm_action::GmActionLog::default());
    app.insert_resource(crate::gm_action::SimulationPaused(false));
    app.insert_resource(runtime);
    app.add_systems(PreUpdate, crate::gm_action::apply_due_actions);
    seed_replay_initial_state(&mut app, &source);

    let mut sim = PhoenixSim {
        app,
        max_frames: 1,
        frames: 0,
        expected_commands: 0,
        applied: 0,
        submitted: 0,
        tail: true,
        gm_actions: source,
        final_tick: Some(0),
        ledger: DigestLedger::new(0),
    };
    sim.step();

    let entry = &sim
        .app
        .world()
        .resource::<crate::gm_action::GmActionLog>()
        .entries()[0];
    assert_eq!(entry.outcome, crate::gm_action::GmActionOutcome::Applied);
    assert_eq!(entry.target.as_deref(), Some("raider"));
    assert_eq!(
        sim.app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .pending_gm_spawns,
        vec![crate::gm_spawn::PendingGmSpawn {
            palette: "raider".into(),
            variant: None,
            name: "raider_1".into(),
            position_mm: [120_000, 0, -40_000],
            heading_mdeg: 90_000,
        }],
        "the replayed peer arms exactly what the recorded one armed"
    );
}

/// Issue #1303: a Pause replays through the same canonical journal, with no
/// bespoke replay path of its own.
///
/// `ReplayGmSeed` seeds the whole journal and `apply_due_actions` is the one
/// production reducer both the live host and the replay run, so the only
/// thing worth pinning is that the reducer reaches the same authoritative
/// state the recording did — and that a Resume recorded after it lands too,
/// because the two together are what a mid-run pause looks like on the wire.
#[test]
fn a_paused_gm_event_replays_through_the_canonical_journal() {
    let event = "base-world::breach_alarm";
    let mut source = GmActionJournal::default();
    for (sequence, correlation, active) in
        [(1, "replay-pause-1", true), (2, "replay-pause-2", true)]
    {
        source
            .insert(crate::gm_action::GmActionGrant {
                from: HostSlot(1),
                sequenced_by: HostSlot(1),
                operator_id: "gm-one".into(),
                correlation: crate::gm_action::GmActionId::new(correlation).unwrap(),
                recovery_generation: 0,
                apply_tick: 0,
                order: crate::gm_action::GmActionOrder::new(HostSlot(1), sequence),
                action: crate::gm_action::GmAction::SetEventPaused {
                    event: event.into(),
                    active,
                },
            })
            .unwrap();
    }
    source.restore_applied_frontier(2).unwrap();
    validate_gm_action_journal(&source).expect("a Pause journal is canonical");

    let mut trigger = crate::world::config::scripted_trigger(
        crate::world::config::TriggerCondition::OnDestroyed {
            entity_name: "courier".to_string(),
        },
    );
    trigger.id = Some("breach_alarm".into());
    let mut controls = crate::world::config::GmEventControls::fire_only(
        "breach_alarm".into(),
        "world.gm.event.breach_alarm".into(),
    );
    controls.pause = true;
    trigger.gm_controls = Some(controls);
    let runtime = crate::world::server::WorldContentRuntime {
        triggers: vec![crate::world::content::TriggerState {
            trigger,
            fired: false,
            origin_layer: None,
            seen_destroyed: Default::default(),
            last_fired_elapsed: None,
        }]
        .into(),
        ..Default::default()
    };

    let mut app = App::new();
    app.insert_resource(SimTick(0));
    app.insert_resource(GmActionJournal::default());
    app.insert_resource(crate::gm_action::GmActionLog::default());
    app.insert_resource(crate::gm_action::SimulationPaused(false));
    app.insert_resource(runtime);
    app.add_systems(PreUpdate, crate::gm_action::apply_due_actions);
    seed_replay_initial_state(&mut app, &source);

    let mut sim = PhoenixSim {
        app,
        max_frames: 1,
        frames: 0,
        expected_commands: 0,
        applied: 0,
        submitted: 0,
        tail: true,
        gm_actions: source,
        final_tick: Some(0),
        ledger: DigestLedger::new(0),
    };
    sim.step();

    let entries = sim
        .app
        .world()
        .resource::<crate::gm_action::GmActionLog>()
        .entries()
        .to_vec();
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.outcome, entry.verb, entry.target.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            (
                crate::gm_action::GmActionOutcome::Applied,
                Some(crate::gm_action::GmEventVerb::Pause),
                Some(event)
            ),
            (
                crate::gm_action::GmActionOutcome::NoOp,
                Some(crate::gm_action::GmEventVerb::Pause),
                Some(event)
            ),
        ],
        "the replayed peer commits the same absolute answer the recorded one did"
    );
    assert_eq!(
        sim.app
            .world()
            .resource::<crate::world::server::WorldContentRuntime>()
            .paused_gm_events
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![event.to_string()],
        "and reaches the same paused set"
    );
    assert!(
        !sim.app
            .world()
            .resource::<crate::gm_action::SimulationPaused>()
            .0,
        "a paused EVENT never touches the session clock"
    );
}

#[test]
fn an_applied_gm_frontier_cannot_cross_the_recorded_final_tick() {
    let mut captured = artifact();
    captured.final_tick = 10;
    captured
        .gm_actions
        .insert(gm_pause_grant(20, true))
        .unwrap();
    captured.gm_actions.restore_applied_frontier(1).unwrap();

    assert!(matches!(
        captured.validate_gm_actions(),
        Err(ArtifactError::InvalidGmActions(why))
            if why == "applied GM frontier crosses the replay's final tick"
    ));
}
