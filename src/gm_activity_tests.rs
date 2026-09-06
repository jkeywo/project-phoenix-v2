use super::*;
use bevy::ecs::message::Messages;

const SHIP_A: &str = "00000000-0000-4000-8000-000000000001";
const ORDINARY_ENTITY: &str = "00000000-0000-4000-8000-000000000002";
const SOURCE: &str = "00000000-0000-4000-8000-000000000003";

fn identities() -> BTreeMap<String, IdentityRecord> {
    [
        (
            SHIP_A.to_owned(),
            IdentityRecord {
                name: "Alliance cruiser".into(),
                is_ship: true,
            },
        ),
        (
            ORDINARY_ENTITY.to_owned(),
            IdentityRecord {
                name: "Beacon".into(),
                is_ship: false,
            },
        ),
        (
            SOURCE.to_owned(),
            IdentityRecord {
                name: "Raider".into(),
                is_ship: true,
            },
        ),
    ]
    .into_iter()
    .collect()
}

fn damage(victim: &str, attacker: Option<&str>, weapon: &str, amount: f32) -> BalanceEvent {
    BalanceEvent::DamageApplied {
        attacker: attacker.map(str::to_owned),
        victim: victim.to_owned(),
        victim_kind: VictimKind::Ship,
        weapon: weapon.to_owned(),
        amount,
        shield_absorbed: 1.0,
        hull_damage: amount - 1.0,
        system_hit: None,
    }
}

fn project(events: &[BalanceEvent]) -> Vec<GmActivityEntry> {
    project_balance_events(17, events, &identities(), &Default::default())
}

fn take(app: &mut App) -> Vec<GmActivityFeedPayload> {
    app.world_mut()
        .resource_mut::<Messages<GmActivityFeedChanged>>()
        .drain()
        .map(|event| event.payload)
        .collect()
}

fn app(depth: u32) -> App {
    let mut config = crate::world::config::WorldConfig::default();
    config.global.gm_activity_history_depth = depth;
    let mut app = App::new();
    app.add_plugins(bevy::state::app::StatesPlugin)
        .init_state::<GamePhase>()
        .insert_resource(BrowserGameMaster)
        .insert_resource(crate::sim_tick::SimTick(0))
        .insert_resource(config)
        .add_message::<BalanceEvent>()
        .add_plugins(GmActivityPlugin);
    app
}

fn install_production_gm_fleet(app: &mut App) -> crate::command_admission::HostSlot {
    use crate::command_admission::HostSlot;
    use crate::core::messages::StationId;
    use crate::lockstep::{FleetGm, FleetLockstep, FleetRoster, FleetShip, LockstepSession};

    let remote = HostSlot(2);
    let gm = HostSlot(3);
    let roster = FleetRoster::with_participants_and_gms(
        vec![
            FleetShip {
                host: HostSlot(1),
                ship_path: Some("assets/entities/alliance_cruiser.toml".into()),
                crew: vec![(StationId("helm".into()), "Std".into())],
            },
            FleetShip {
                host: remote,
                ship_path: Some("assets/entities/raider.toml".into()),
                crew: vec![(StationId("tactical".into()), "Std".into())],
            },
        ],
        vec![HostSlot(1), remote, gm],
        vec![FleetGm {
            host: gm,
            operator_id: "gm-alpha".into(),
        }],
        gm,
        HostSlot(1),
    )
    .expect("production GM topology is valid");
    app.insert_resource(FleetLockstep(LockstepSession::new(
        gm,
        roster.participants(),
        6,
    )));
    app.insert_resource(roster);
    app.insert_resource(
        crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator::new(
            "gm-alpha".into(),
            "Morgan".into(),
            true,
        )])
        .unwrap(),
    );
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::lockstep::FleetSlotOf(HostSlot(1)),
        EntityUuid(SHIP_A.into()),
        EntityName("Alliance cruiser".into()),
    ));
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::lockstep::FleetSlotOf(remote),
        EntityUuid(SOURCE.into()),
        EntityName("Raider".into()),
    ));
    remote
}

fn fixed_then_publish(app: &mut App) {
    app.world_mut().run_schedule(FixedLast);
    app.world_mut().run_schedule(PostUpdate);
}

#[test]
fn pure_projection_uses_canonical_category_order_and_preserves_simultaneous_repeats() {
    let events = vec![
        BalanceEvent::RedAlertChanged {
            ship: SHIP_A.into(),
            on: true,
        },
        BalanceEvent::TriggerFired {
            trigger_id: "arrival".into(),
            origin: "world.rhai".into(),
            entity: Some(ORDINARY_ENTITY.into()),
        },
        BalanceEvent::ObjectiveChanged {
            objective_id: "secure".into(),
            status: ObjectiveStatus::Completed,
            targets: vec!["beacon".into()],
        },
        BalanceEvent::EntityDestroyed {
            victim: SHIP_A.into(),
            killer: Some(SOURCE.into()),
        },
        damage(SHIP_A, Some(SOURCE), "bank", 3.0),
        damage(SHIP_A, Some(SOURCE), "bank", 3.0),
    ];
    let aliases = [("beacon".into(), ORDINARY_ENTITY.into())]
        .into_iter()
        .collect();
    let projected = project_balance_events(17, &events, &identities(), &aliases);
    assert_eq!(
        projected
            .iter()
            .map(|entry| entry.category)
            .collect::<Vec<_>>(),
        vec![
            GmActivityCategory::Damage,
            GmActivityCategory::Damage,
            GmActivityCategory::Destruction,
            GmActivityCategory::Objective,
            GmActivityCategory::Trigger,
            GmActivityCategory::RedAlert,
        ]
    );
    assert_eq!(projected[0], projected[1], "exact repeats remain rows");
    assert!(projected.iter().all(|entry| entry.tick == 17));
}

#[test]
fn semantic_ship_scope_comes_from_actual_ship_classification() {
    let projected = project(&[
        damage(ORDINARY_ENTITY, None, "region", 2.0),
        damage(SHIP_A, None, "region", 2.0),
    ]);
    let ordinary = projected
        .iter()
        .find(|entry| {
            entry
                .links
                .iter()
                .any(|link| link.entity.entity_id == ORDINARY_ENTITY)
        })
        .unwrap();
    let ship = projected
        .iter()
        .find(|entry| {
            entry
                .links
                .iter()
                .any(|link| link.entity.entity_id == SHIP_A)
        })
        .unwrap();
    assert!(ordinary.ships.is_empty());
    assert_eq!(ship.ships[0].entity_id, SHIP_A);
}

#[test]
fn damage_stable_keys_keep_victim_source_and_float_total_order() {
    let projected = project(&[
        damage(SHIP_A, Some(SOURCE), "bank", f32::INFINITY),
        damage(SHIP_A, Some(SOURCE), "bank", -0.0),
        damage(SHIP_A, Some(SOURCE), "bank", 0.0),
        damage(SHIP_A, Some(SOURCE), "bank", f32::NEG_INFINITY),
    ]);
    let amounts: Vec<_> = projected
        .iter()
        .map(|entry| match &entry.detail {
            GmActivityDetail::Damage(detail) => detail.amount,
            other => panic!("expected damage detail, got {other:?}"),
        })
        .collect();
    assert_eq!(amounts, vec![f32::NEG_INFINITY, -0.0, 0.0, f32::INFINITY]);
}

#[test]
fn objective_trigger_and_alert_details_retain_actual_transition_information() {
    let events = vec![
        BalanceEvent::ObjectiveChanged {
            objective_id: "o".into(),
            status: ObjectiveStatus::Active,
            targets: vec![],
        },
        BalanceEvent::ObjectiveChanged {
            objective_id: "o".into(),
            status: ObjectiveStatus::Completed,
            targets: vec![],
        },
        BalanceEvent::ObjectiveChanged {
            objective_id: "o".into(),
            status: ObjectiveStatus::Failed,
            targets: vec![],
        },
        BalanceEvent::TriggerFired {
            trigger_id: "named".into(),
            origin: "a.rhai".into(),
            entity: None,
        },
        BalanceEvent::TriggerFired {
            trigger_id: "a.rhai::anonymous_fn".into(),
            origin: "a.rhai".into(),
            entity: None,
        },
        BalanceEvent::TriggerFired {
            trigger_id: "named".into(),
            origin: "a.rhai".into(),
            entity: None,
        },
        BalanceEvent::RedAlertChanged {
            ship: SHIP_A.into(),
            on: false,
        },
        BalanceEvent::RedAlertChanged {
            ship: SHIP_A.into(),
            on: true,
        },
    ];
    let projected = project(&events);
    assert_eq!(
        projected
            .iter()
            .filter_map(|entry| match &entry.detail {
                GmActivityDetail::Objective(detail) => Some(detail.status),
                _ => None,
            })
            .collect::<Vec<_>>(),
        vec![
            GmActivityObjectiveStatus::Active,
            GmActivityObjectiveStatus::Completed,
            GmActivityObjectiveStatus::Failed,
        ]
    );
    let trigger_ids: Vec<_> = projected
        .iter()
        .filter_map(|entry| match &entry.detail {
            GmActivityDetail::Trigger(detail) => Some(detail.trigger_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(trigger_ids, vec!["a.rhai::anonymous_fn", "named", "named"]);
    let alert_edges: Vec<_> = projected
        .iter()
        .filter_map(|entry| match &entry.detail {
            GmActivityDetail::RedAlert(detail) => Some(detail.active),
            _ => None,
        })
        .collect();
    assert_eq!(alert_edges, vec![false, true]);
}

#[test]
fn pure_history_bounds_oldest_first_without_collapsing_repeats() {
    let repeated = project(&[
        damage(SHIP_A, None, "region", 2.0),
        damage(SHIP_A, None, "region", 2.0),
        damage(SHIP_A, None, "region", 2.0),
    ]);
    let mut history = GmActivityHistory::new(2);
    assert!(history.append(repeated));
    let payload = history.payload();
    assert_eq!(payload.capacity, 2);
    assert_eq!(payload.entries.len(), 2);
    assert_eq!(payload.entries[0], payload.entries[1]);
}

#[test]
fn history_reorders_late_same_tick_facts_before_operational_rows() {
    let connection = GmActivityEntry {
        tick: 17,
        category: GmActivityCategory::Connection,
        ships: vec![],
        links: vec![],
        detail: GmActivityDetail::Connection(GmActivityConnection {
            identity: GmActivityPublicIdentity {
                id: "crew-1".into(),
                name: "Ari".into(),
            },
            role: GmActivityConnectionRole::Crew,
            state: GmActivityConnectionState::Connected,
            ship: None,
        }),
    };
    let damage = project(&[damage(SHIP_A, None, "region", 2.0)])
        .pop()
        .unwrap();
    let mut history = GmActivityHistory::new(2);

    assert!(history.append([connection]));
    assert!(history.append([damage]));

    assert_eq!(
        history
            .payload()
            .entries
            .iter()
            .map(|entry| entry.category)
            .collect::<Vec<_>>(),
        vec![GmActivityCategory::Damage, GmActivityCategory::Connection]
    );
}

#[test]
fn gm_peer_projects_remote_fleet_crew_connect_and_disconnect_with_ship_scope() {
    let mut app = app(16);

    // A production BrowserGameMaster owns no LocalShip. Its private bootstrap
    // seeds no synthetic local crew before the digest-proven fleet commit.
    app.world_mut().run_schedule(PostUpdate);
    assert!(take(&mut app).pop().unwrap().entries.is_empty());

    // Commit installs the same remote topology every simulation peer holds.
    // That transition is the connected source fact for this first-time GM.
    let remote = install_production_gm_fleet(&mut app);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 8;
    fixed_then_publish(&mut app);
    let connected = take(&mut app).pop().unwrap();
    let connected = connected
        .entries
        .iter()
        .find(|entry| {
            matches!(
                &entry.detail,
                GmActivityDetail::Connection(GmActivityConnection {
                    state: GmActivityConnectionState::Connected,
                    ship: Some(ship),
                    ..
                }) if ship.entity_id == SOURCE
            )
        })
        .expect("the committed remote crew is visible to the GM");
    assert_eq!(connected.tick, 8);

    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 9;
    app.world_mut()
        .resource_mut::<crate::lockstep::FleetRoster>()
        .depart_slot(remote);
    fixed_then_publish(&mut app);
    let departed = take(&mut app).pop().unwrap();
    let row = departed
        .entries
        .iter()
        .find(|entry| {
            matches!(
                &entry.detail,
                GmActivityDetail::Connection(GmActivityConnection {
                    state: GmActivityConnectionState::Disconnected,
                    ship: Some(ship),
                    ..
                }) if ship.entity_id == SOURCE
            )
        })
        .expect("the agreed remote crew departure is visible to the GM");
    let GmActivityDetail::Connection(detail) = &row.detail else {
        panic!("remote departure must be a Connection row")
    };
    assert_eq!(row.tick, 9);
    assert_eq!(detail.identity.id, format!("crew:{SOURCE}"));
    assert_eq!(detail.identity.name, "Raider");
    assert_eq!(detail.role, GmActivityConnectionRole::Crew);
    assert_eq!(detail.state, GmActivityConnectionState::Disconnected);
    assert_eq!(row.ships, vec![detail.ship.clone().unwrap()]);
    assert_eq!(row.ships[0].entity_id, SOURCE);
    assert_eq!(row.links[0].role, GmActivityLinkRole::Ship);

    let wire = crate::core::codec::encode_gm_activity_feed(&departed).unwrap();
    assert!(!wire.contains("slot-"));
    assert!(!wire.contains("HostSlot"));
    assert!(!wire.contains("peer"));
    assert!(!wire.contains("credential"));
}

fn logged(
    operator_id: &str,
    correlation: &str,
    outcome: crate::gm_action::GmActionOutcome,
    reason: Option<crate::gm_action::GmActionRefusalReason>,
) -> crate::gm_action::LoggedGmAction {
    crate::gm_action::LoggedGmAction {
        operator_id: operator_id.into(),
        correlation: crate::gm_action::GmActionId::new(correlation).unwrap(),
        action_kind: crate::gm_action::GmActionKind::SessionPause,
        requested_active: true,
        outcome,
        tick: 12,
        reason,
        order: Some(crate::gm_action::GmActionOrder::new(
            crate::command_admission::log::HostSlot(1),
            7,
        )),
        target: None,
        effect: None,
    }
}

#[test]
fn terminal_gm_actions_attribute_exact_outcomes_dedup_and_rebase_on_restore() {
    let mut state = GmActivityState::default();
    assert!(terminal_action_entries(&mut state, None, None, None, None).is_empty());
    let mut results = crate::gm_action::LocalGmActionRefusals::default();
    results.push(logged(
        "gm-alpha",
        "pause-1",
        crate::gm_action::GmActionOutcome::Refused,
        Some(crate::gm_action::GmActionRefusalReason::WrongPhase),
    ));
    let roster = crate::gm_roster::GmRoster::try_new(vec![crate::gm_roster::GmOperator::new(
        "gm-alpha".into(),
        "Morgan".into(),
        true,
    )])
    .unwrap();
    let first = terminal_action_entries(&mut state, None, Some(&results), None, Some(&roster));
    let GmActivityDetail::GmAction(detail) = &first[0].detail else {
        panic!("expected GM action detail")
    };
    assert_eq!(detail.operator.name, "Morgan");
    assert_eq!(detail.outcome, GmActivityActionOutcome::Refused);
    assert_eq!(detail.reason.as_deref(), Some("wrong-phase"));
    assert_eq!(detail.order.unwrap().sequence, 7);
    assert!(
        terminal_action_entries(&mut state, None, Some(&results), None, Some(&roster)).is_empty()
    );

    let mut restored = crate::gm_action::LocalGmActionRefusals::default();
    restored.push(logged(
        "gm-beta",
        "old-restored-row",
        crate::gm_action::GmActionOutcome::Applied,
        None,
    ));
    let mut restored_starts = crate::lobby::StartGrantResults::default();
    restored_starts.push(crate::lobby::start_policy::StartGrantResult {
        tick: 44,
        status: crate::lobby::start_policy::StartGrantStatus::Applied,
        operator_id: Some("gm-beta".into()),
        reason: None,
        grant_id: Some("start-8".into()),
    });
    let mut world = World::new();
    world.insert_resource(state);
    world.insert_resource(restored_starts);
    rebase_after_restore(&mut world);
    let mut state = world
        .remove_resource::<GmActivityState>()
        .expect("activity state survives cursor rebase");
    let restored_starts = world.resource::<crate::lobby::StartGrantResults>();
    assert!(
        terminal_action_entries(
            &mut state,
            None,
            Some(&restored),
            Some(restored_starts),
            None
        )
        .is_empty(),
        "the restore hook rebases rather than replaying old pause or start rows"
    );
}

/// A fired GM event reaches the activity feed as the thing it is, naming the
/// event it fired (issue #1301). Crew see only the fictional consequence the
/// handler produced; this row is what tells the GMs who caused it.
#[test]
fn a_fired_gm_event_is_attributed_by_the_event_it_fired() {
    let mut state = GmActivityState::default();
    assert!(terminal_action_entries(&mut state, None, None, None, None).is_empty());
    let mut results = crate::gm_action::LocalGmActionRefusals::default();
    let mut fact = logged(
        "gm-alpha",
        "fire-1",
        crate::gm_action::GmActionOutcome::Applied,
        None,
    );
    fact.action_kind = crate::gm_action::GmActionKind::EventControl;
    fact.target = Some("base-world::breach_alarm".into());
    results.push(fact);

    let entries = terminal_action_entries(&mut state, None, Some(&results), None, None);
    let GmActivityDetail::GmAction(detail) = &entries[0].detail else {
        panic!("expected GM action detail")
    };
    assert_eq!(
        detail.action,
        GmActivityAction::FireGmEvent {
            event: "base-world::breach_alarm".into(),
        },
        "an event fire must not be reported as a session pause"
    );
    assert_eq!(detail.outcome, GmActivityActionOutcome::Applied);
    assert_eq!(entries[0].category, GmActivityCategory::GmAction);
}

/// A directed world effect is attributed by WHAT it hit and by what the hull
/// actually did with the amount — never reported as a session pause, and never
/// dropped for want of a resolved effect (issue #1310).
#[test]
fn a_direct_effect_is_attributed_by_its_target_and_its_resolved_amounts() {
    let mut state = GmActivityState::default();
    assert!(terminal_action_entries(&mut state, None, None, None, None).is_empty());
    let mut results = crate::gm_action::LocalGmActionRefusals::default();
    let mut fact = logged(
        "gm-alpha",
        "hit-1",
        crate::gm_action::GmActionOutcome::Applied,
        None,
    );
    fact.action_kind = crate::gm_action::GmActionKind::DirectEffect;
    fact.target = Some("npc-1".into());
    fact.effect = Some(crate::gm_effect::GmDirectEffectResult {
        kind: crate::gm_effect::GmDirectEffectKind::Damage,
        applied_milli_hp: 25_000,
        discarded_milli_hp: 5_000,
        destroyed: true,
    });
    results.push(fact);

    let entries = terminal_action_entries(&mut state, None, Some(&results), None, None);
    let GmActivityDetail::GmAction(detail) = &entries[0].detail else {
        panic!("expected GM action detail")
    };
    assert_eq!(
        detail.action,
        GmActivityAction::ApplyDirectEffect {
            entity: "npc-1".into(),
            heal: false,
            applied_milli_hp: 25_000,
            discarded_milli_hp: 5_000,
            destroyed: true,
        }
    );
    assert_eq!(detail.outcome, GmActivityActionOutcome::Applied);
    assert_eq!(entries[0].category, GmActivityCategory::GmAction);
}

/// A refusal settled before any hull was read carries no resolved effect. The
/// row still renders — with zeroes — rather than dropping the operator press.
#[test]
fn a_refused_direct_effect_still_names_the_target_it_was_aimed_at() {
    let mut state = GmActivityState::default();
    assert!(terminal_action_entries(&mut state, None, None, None, None).is_empty());
    let mut results = crate::gm_action::LocalGmActionRefusals::default();
    let mut fact = logged(
        "gm-alpha",
        "hit-2",
        crate::gm_action::GmActionOutcome::Refused,
        Some(crate::gm_action::GmActionRefusalReason::UnknownEntity),
    );
    fact.action_kind = crate::gm_action::GmActionKind::DirectEffect;
    fact.target = Some("npc-gone".into());
    results.push(fact);

    let entries = terminal_action_entries(&mut state, None, Some(&results), None, None);
    let GmActivityDetail::GmAction(detail) = &entries[0].detail else {
        panic!("expected GM action detail")
    };
    assert_eq!(
        detail.action,
        GmActivityAction::ApplyDirectEffect {
            entity: "npc-gone".into(),
            heal: false,
            applied_milli_hp: 0,
            discarded_milli_hp: 0,
            destroyed: false,
        }
    );
    assert_eq!(detail.reason.as_deref(), Some("unknown-entity"));
}

#[test]
fn force_start_uses_typed_result_once() {
    let mut state = GmActivityState::default();
    terminal_action_entries(&mut state, None, None, None, None);
    let mut starts = crate::lobby::StartGrantResults::default();
    starts.push(crate::lobby::start_policy::StartGrantResult {
        tick: 21,
        status: crate::lobby::start_policy::StartGrantStatus::NoOp,
        operator_id: Some("gm-alpha".into()),
        reason: Some(crate::lobby::start_policy::StartGrantReason::AlreadyStarted),
        grant_id: Some("start-1".into()),
    });
    let first = terminal_action_entries(&mut state, None, None, Some(&starts), None);
    let GmActivityDetail::GmAction(detail) = &first[0].detail else {
        panic!("expected GM action detail")
    };
    assert_eq!(detail.action, GmActivityAction::ForceStart);
    assert_eq!(detail.outcome, GmActivityActionOutcome::NoOp);
    assert_eq!(detail.reason.as_deref(), Some("already-started"));
    assert_eq!(first[0].tick, 21, "the result's source tick wins");
    assert!(terminal_action_entries(&mut state, None, None, Some(&starts), None).is_empty());
}

#[test]
fn multi_step_frame_preserves_each_operational_source_tick() {
    let mut app = app(16);
    let remote = install_production_gm_fleet(&mut app);
    let crewed_roster = app
        .world()
        .resource::<crate::lockstep::FleetRoster>()
        .clone();
    app.insert_resource(crate::lobby::StartGrantResults::default());
    app.add_systems(FixedLast, crate::sim_tick::advance_sim_tick);

    // Seed the frame-driven cursors before either source changes.
    app.world_mut().run_schedule(PostUpdate);
    take(&mut app);

    app.world_mut()
        .resource_mut::<crate::lockstep::FleetRoster>()
        .depart_slot(remote);
    app.world_mut()
        .resource_mut::<crate::lobby::StartGrantResults>()
        .push(crate::lobby::start_policy::StartGrantResult {
            tick: 0,
            status: crate::lobby::start_policy::StartGrantStatus::Applied,
            operator_id: Some("gm-alpha".into()),
            reason: None,
            grant_id: Some("start-1".into()),
        });
    app.world_mut().run_schedule(FixedLast);

    app.insert_resource(crewed_roster);
    app.world_mut()
        .resource_mut::<crate::lobby::StartGrantResults>()
        .push(crate::lobby::start_policy::StartGrantResult {
            tick: 1,
            status: crate::lobby::start_policy::StartGrantStatus::NoOp,
            operator_id: Some("gm-alpha".into()),
            reason: Some(crate::lobby::start_policy::StartGrantReason::AlreadyStarted),
            grant_id: Some("start-2".into()),
        });
    app.world_mut().run_schedule(FixedLast);

    assert_eq!(app.world().resource::<crate::sim_tick::SimTick>().0, 2);
    app.world_mut().run_schedule(PostUpdate);
    let payload = take(&mut app).pop().unwrap();
    let connection_ticks = payload
        .entries
        .iter()
        .filter(|entry| entry.category == GmActivityCategory::Connection)
        .map(|entry| entry.tick)
        .collect::<Vec<_>>();
    let force_start_ticks = payload
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                &entry.detail,
                GmActivityDetail::GmAction(GmActivityGmAction {
                    action: GmActivityAction::ForceStart,
                    ..
                })
            )
        })
        .map(|entry| entry.tick)
        .collect::<Vec<_>>();
    assert_eq!(connection_ticks, vec![0, 1]);
    assert_eq!(force_start_ticks, vec![0, 1]);
}

#[test]
fn plugin_publishes_fixed_facts_in_post_update_and_actions_without_a_fixed_tick() {
    let mut app = app(8);
    app.world_mut().run_schedule(PostUpdate);
    take(&mut app);
    app.world_mut().resource_mut::<crate::sim_tick::SimTick>().0 = 41;
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(damage(SHIP_A, None, "region", 2.0));
    app.world_mut().run_schedule(FixedLast);
    assert!(
        take(&mut app).is_empty(),
        "Host Channel publish is frame-driven"
    );
    app.world_mut().run_schedule(PostUpdate);
    let first = take(&mut app);
    assert_eq!(first[0].entries[0].tick, 41);

    let mut refusals = crate::gm_action::LocalGmActionRefusals::default();
    refusals.push(logged(
        "gm-alpha",
        "pause-while-paused",
        crate::gm_action::GmActionOutcome::NoOp,
        None,
    ));
    app.insert_resource(refusals);
    app.world_mut().run_schedule(PostUpdate);
    let paused_publish = take(&mut app);
    assert!(paused_publish[0]
        .entries
        .iter()
        .any(|entry| entry.category == GmActivityCategory::GmAction));
}

#[test]
fn identity_cache_survives_despawn_and_only_ship_markers_scope_rows() {
    let mut app = app(8);
    let ship = app
        .world_mut()
        .spawn((
            EntityUuid(SHIP_A.into()),
            EntityName("Named ship".into()),
            crate::server_app::Ship,
        ))
        .id();
    let ordinary = app
        .world_mut()
        .spawn((
            EntityUuid(ORDINARY_ENTITY.into()),
            EntityName("Named object".into()),
        ))
        .id();
    app.world_mut().run_schedule(FixedUpdate);
    app.world_mut().despawn(ship);
    app.world_mut().despawn(ordinary);
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(damage(SHIP_A, Some(ORDINARY_ENTITY), "impact", 2.0));
    fixed_then_publish(&mut app);
    let payload = take(&mut app).pop().unwrap();
    assert_eq!(payload.entries[0].ships[0].name, "Named ship");
    assert_eq!(payload.entries[0].ships.len(), 1);
    assert!(payload.entries[0]
        .links
        .iter()
        .any(|link| link.entity.name == "Named object"));
}

#[test]
fn lobby_reset_clears_history_but_game_over_does_not() {
    let mut app = app(8);
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(damage(SHIP_A, None, "region", 2.0));
    fixed_then_publish(&mut app);
    assert_eq!(take(&mut app)[0].entries.len(), 1);

    app.world_mut()
        .resource_mut::<NextState<GamePhase>>()
        .set(GamePhase::GameOver);
    app.world_mut().run_schedule(StateTransition);
    assert_eq!(
        app.world().resource::<State<GamePhase>>().get(),
        &GamePhase::GameOver
    );
    fixed_then_publish(&mut app);
    assert!(
        take(&mut app).is_empty(),
        "entering GameOver does not imply a reset"
    );
    assert_eq!(
        app.world()
            .resource::<GmActivityState>()
            .history
            .payload()
            .entries
            .len(),
        1
    );

    app.world_mut().run_system_cached(reset_on_lobby).unwrap();
    app.world_mut().run_schedule(PostUpdate);
    assert!(take(&mut app).pop().unwrap().entries.is_empty());
}

#[test]
fn absent_browser_gm_ignores_events() {
    let mut app = app(8);
    app.world_mut().remove_resource::<BrowserGameMaster>();
    app.world_mut()
        .resource_mut::<Messages<BalanceEvent>>()
        .write(damage(SHIP_A, None, "region", 2.0));
    fixed_then_publish(&mut app);
    assert!(take(&mut app).is_empty());
}
