use super::*;
use crate::core::messages::*;
use crate::lobby::{InboundMessage, LobbyPlugin, OutboundMessage};
use crate::server_app::SimOutbox;
use crate::server_app::{ShipImpulse, ShipShields};
use crate::ship::damage::SystemHull;
use crate::ship_plugin::ShipSystemControlSources;
use crate::weapons::shield::ShieldSystem;

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

fn test_app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    // `FixedUpdate`, where `RepairPlugin` and `AdmissionPlugin` register
    // since issue #895 — configured on `Update` this chain would order
    // nothing, leaving admission unordered against the repair handlers.
    app.configure_sets(
        FixedUpdate,
        (
            crate::sim_sets::SimSet::Input,
            crate::sim_sets::SimSet::Physics,
            crate::sim_sets::SimSet::Damage,
            crate::sim_sets::SimSet::Modifiers,
            crate::sim_sets::SimSet::Publish,
            crate::sim_sets::SimSet::PublishAggregate,
            crate::sim_sets::SimSet::Broadcast,
        )
            .chain(),
    )
    .add_plugins(LobbyPlugin)
    .add_plugins(bevy::time::TimePlugin)
    .add_plugins(crate::server_app::AdmissionPlugin)
    .init_resource::<crate::lobby::WorldResource>()
    .init_resource::<SimOutbox>()
    .init_resource::<Outbox>()
    .add_plugins(RepairPlugin)
    .add_plugins(repair_state_broadcaster())
    .add_systems(PostUpdate, collect);
    // One fixed step per update, 200 ms of sim time each (issue #895), so
    // the Hz-based repair broadcast timer fires within a single harness
    // tick.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(200),
    );
    // Spawn the player ship entity so handle_dispatch_repair_team can query it.
    let hull_config = &[
        (SystemId("helm".into()), 25.0_f32),
        (SystemId("helm-engine-port".into()), 25.0),
        (SystemId("tactical".into()), 25.0),
        (SystemId("power".into()), 25.0),
        (SystemId("shields".into()), 25.0),
        (SystemId("core".into()), 50.0),
    ];
    app.world_mut().spawn((
        crate::server_app::Ship,
        crate::server_app::LocalShip,
        crate::ship_plugin::ShipConfigComponent::default(),
        crate::ship_plugin::ShipSystemControlSources::default(),
        crate::core::messages::AdmittedCommands::default(),
        crate::ship_plugin::ActiveStationRatings::default(),
        crate::ship_plugin::CoordinationQueue::default(),
        crate::entities::spawner::EntitySystemHull(SystemHull::from_config(hull_config)),
        crate::server_app::ShipSystemBlackboards::default(),
        ShipShields(ShieldSystem::default(), 0.5),
        ShipImpulse(crate::ship::impulse::ImpulseState::new()),
        crate::modifiers::ShipModifiers::new(),
        RepairRequestQueue::default(),
        // Nested tuple to keep the outer bundle within Bevy's 15-arity limit.
        // Issue #830: the global `ShipRepairTeams` Resource is gone; every
        // ship (including this test's LocalShip) carries its own component.
        (
            crate::ship_plugin::RepairHumanAlerted::default(),
            crate::ship_plugin::LastSystemTiers::default(),
            ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(2)),
            // The AUTHORED `[repair.selector]` block every shipped hull
            // carries. Since #885b stage 5d `operate_repair_ai` has no
            // synthesised fallback — a ship with no selector dispatches
            // nothing — so a fixture that wants dispatch must attach the
            // declaration a real hull writes.
            RepairTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("repair")
                    .to_selector()
                    .expect("the shipped Repair selector decodes"),
                power_rating: None,
            },
        ),
    ));
    app
}

/// Read the LocalShip's own `ShipRepairTeams` component (issue #830 — no
/// global Resource). Returns an owned clone for assertion convenience.
fn local_teams(app: &mut App) -> ShipRepairTeams {
    let mut q = app
        .world_mut()
        .query_filtered::<&ShipRepairTeams, With<crate::server_app::LocalShip>>();
    q.single(app.world())
        .expect("LocalShip must carry ShipRepairTeams")
        .clone()
}

/// Dispatch a team on the LocalShip's own `ShipRepairTeams` component.
fn dispatch_local(app: &mut App, idx: usize, sid: SystemId, name: &str) {
    let mut q = app
        .world_mut()
        .query_filtered::<&mut ShipRepairTeams, With<crate::server_app::LocalShip>>();
    q.single_mut(app.world_mut())
        .expect("LocalShip must carry ShipRepairTeams")
        .0
        .dispatch(idx, sid, name.to_string());
}

/// Damage the named systems on the LocalShip's hull, adding a row for any
/// the fixture hull does not already carry.
///
/// The ids passed here are always systems the shipped battleship config
/// (`ShipConfigComponent::default()`) OWNS from the station under test, so a
/// `RepairTarget::Station` dispatch resolves through `systems_for_station`
/// rather than through the station-name fallback.
///
/// That matters since the issue #1013 review: `resolve_repair_target` no
/// longer falls back to `SystemId(station_id)` when the station's own name
/// is also a hull row, because such a row is OWNERLESS (bucketed under
/// `core`) and sweeping from it would walk the team out of the station it
/// was sent to. This fixture's coarse `helm`/`tactical`/`power`/`shields`
/// rows are exactly that shape, so the dispatch tests below name the fine
/// systems a production console would — the console's target list is built
/// from the ship's fine hull rows.
///
/// HP is set to 80% of max: below max, so the system is a resolvable repair
/// target, but still `Operational`, so no tier crossing fires and no
/// unrelated console is taken offline.
fn damage_owned_fine_systems(app: &mut App, systems: &[&str]) {
    let at_80: Vec<(&str, f32)> = systems.iter().map(|id| (*id, 0.8)).collect();
    damage_owned_fine_systems_to(app, &at_80);
}

/// [`damage_owned_fine_systems`] with an explicit HP fraction per system, so
/// a test can put a station's systems into DIFFERENT damage tiers and
/// observe which one a sweep or a console tap picks out of them.
fn damage_owned_fine_systems_to(app: &mut App, systems: &[(&str, f32)]) {
    let local_ship = {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        q.single(app.world()).expect("one LocalShip")
    };
    let mut rows: Vec<(SystemId, f32)> = app
        .world()
        .get::<crate::entities::spawner::EntitySystemHull>(local_ship)
        .expect("LocalShip must carry EntitySystemHull")
        .0
        .iter()
        .map(|(sid, entry)| (sid.clone(), entry.max))
        .collect();
    for (id, _) in systems {
        let sid = SystemId((*id).into());
        if !rows.iter().any(|(existing, _)| *existing == sid) {
            rows.push((sid, 25.0));
        }
    }
    let mut hull = SystemHull::from_config(&rows);
    for (id, fraction) in systems {
        let sid = SystemId((*id).into());
        let max = hull.get(&sid).expect("just built this row").max;
        hull.set_hp(&sid, max * fraction);
    }
    app.world_mut()
        .entity_mut(local_ship)
        .insert(crate::entities::spawner::EntitySystemHull(hull));
}

fn repair_bb(app: &mut App) -> RepairBlackboard {
    let mut q = app
        .world_mut()
        .query_filtered::<&crate::server_app::ShipSystemBlackboards, With<crate::server_app::LocalShip>>();
    let bbs = q
        .single(app.world())
        .expect("LocalShip must have ShipSystemBlackboards");
    let key = SystemId(REPAIR_SYSTEM_ID.to_string());
    let SystemBlackboard::Repair(bb) = bbs.0.get(&key).unwrap() else {
        panic!("expected Repair blackboard");
    };
    bb.clone()
}

fn push(app: &mut App, token: &str, msg: ClientMessage) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg,
        });
}

fn tick(app: &mut App) -> Vec<OutboundMessage> {
    app.update();
    let sim_entries = app.world_mut().resource_mut::<SimOutbox>().drain();
    let mut out = app.world().resource::<Outbox>().0.clone();
    for entry in sim_entries {
        out.push(OutboundMessage {
            target: entry.target,
            msg: entry.message,
            delivery: entry.delivery,
        });
    }
    app.world_mut().resource_mut::<Outbox>().0.clear();
    out
}

fn start_game(app: &mut App) {
    push(
        app,
        "captain",
        ClientMessage::Identify {
            token: "captain".into(),
            name: "Alice".into(),
        },
    );
    tick(app);
    push(
        app,
        "captain",
        ClientMessage::SelectStation {
            station: "Captain".into(),
        },
    );
    tick(app);
    push(
        app,
        "eng",
        ClientMessage::Identify {
            token: "eng".into(),
            name: "Bob".into(),
        },
    );
    tick(app);
    push(
        app,
        "eng",
        ClientMessage::SelectStation {
            station: "Repair".into(),
        },
    );
    tick(app);
    push(app, "captain", ClientMessage::SetReady { ready: true });
    push(app, "eng", ClientMessage::SetReady { ready: true });
    tick(app);
}

fn team_is_travelling(teams: &ShipRepairTeams, idx: usize) -> bool {
    matches!(
        teams.0.slots()[idx],
        crate::core::messages::TeamSlot::Travelling { .. }
    )
}

fn team_is_idle(teams: &ShipRepairTeams, idx: usize) -> bool {
    matches!(teams.0.slots()[idx], crate::core::messages::TeamSlot::Idle)
}

// ── Dispatch tests ──────────────────────────────────────────────────────

/// Non-Repair console holder sending `DispatchRepairTeam` is ignored.
#[test]
fn non_repair_sender_is_ignored() {
    let mut app = test_app();
    start_game(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        team_is_idle(&teams, 0),
        "team 0 should remain idle after non-Repair dispatch"
    );
}

/// Repair holder dispatches team to a console → team enters Travelling.
#[test]
fn dispatch_sends_team_to_travelling() {
    let mut app = test_app();
    start_game(&mut app);
    damage_owned_fine_systems(&mut app, &["helm-engine-port"]);

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        team_is_travelling(&teams, 0),
        "team 0 should be travelling after dispatch"
    );
}

/// A station dispatch must resolve to an owned fine hull system so a team
/// can finish travelling and restore HP instead of immediately returning.
#[test]
fn station_dispatch_repairs_damaged_owned_fine_system() {
    let mut app = test_app();
    start_game(&mut app);

    let local_ship = {
        let mut query = app
            .world_mut()
            .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
        query
            .single(app.world())
            .expect("test fixture must contain one LocalShip")
    };
    app.world_mut()
        .entity_mut(local_ship)
        .insert(ShipRepairTeams(
            crate::modifiers::repair_teams::RepairTeams::default(),
        ));

    let damaged_system = SystemId("helm-engine-port".into());
    let hp_before = 10.0;
    {
        let mut query = app.world_mut().query_filtered::<
            &mut crate::entities::spawner::EntitySystemHull,
            With<crate::server_app::LocalShip>,
        >();
        let mut hull = query
            .single_mut(app.world_mut())
            .expect("test fixture must contain one LocalShip hull");
        hull.0.set_hp(&damaged_system, hp_before);
    }

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: SystemId(REPAIR_SYSTEM_ID.into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);

    {
        let teams = local_teams(&mut app);
        let TeamSlot::Travelling { system_id, .. } = &teams.0.slots()[0] else {
            panic!("team 0 should be travelling to the damaged fine system");
        };
        assert_eq!(system_id.as_ref(), Some(&damaged_system));
    }

    // Default travel time is five seconds and the test clock advances 0.2s
    // per update. Run long enough to arrive and perform at least one repair.
    for _ in 0..30 {
        tick(&mut app);
    }

    let mut query = app.world_mut().query_filtered::<
        &crate::entities::spawner::EntitySystemHull,
        With<crate::server_app::LocalShip>,
    >();
    let hull = query
        .single(app.world())
        .expect("test fixture must contain one LocalShip hull");
    assert!(
        hull.0.current_for(&damaged_system).unwrap() > hp_before,
        "the arrived team should restore the station-owned fine system"
    );
}

/// When team is busy, dispatching to a different console redirects it.
#[test]
fn all_busy_teams_ignore_further_dispatches() {
    let mut app = test_app();
    start_game(&mut app);
    // One owned fine system per station this test addresses, so each
    // dispatch resolves without the (now refused) station-name fallback.
    damage_owned_fine_systems(
        &mut app,
        &["helm-engine-port", "tactical-radar", "power-reactor"],
    );

    // Dispatch both teams (default is 2).
    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);
    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 1,
                target: RepairTarget::Station(StationId("tactical".into())),
            },
        },
    );
    tick(&mut app);

    // Redirect team 0 to Power (different console) — now team 0 is Returning with queue
    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("power".into())),
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    // team 0 should be Returning (redirected), team 1 still Travelling
    assert!(matches!(
        &teams.0.slots()[0],
        crate::core::messages::TeamSlot::Returning { .. }
    ));
    assert!(team_is_travelling(&teams, 1));
}

/// RepairState broadcast includes the team slot states.
#[test]
fn repair_state_broadcast_includes_team_slots() {
    let mut app = test_app();
    start_game(&mut app);
    damage_owned_fine_systems(&mut app, &["helm-engine-port"]);

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    let out1 = tick(&mut app);
    let out2 = tick(&mut app);

    let has_repair_state = out1.iter().chain(out2.iter()).any(|m| {
        matches!(&m.msg, ServerMessage::RepairState { teams } if
            teams.iter().any(|t| matches!(t, crate::core::messages::TeamSlot::Travelling { .. })))
    });
    assert!(
        has_repair_state,
        "RepairState should include a Travelling team after dispatch"
    );
}

// ── ControlSystem dispatch tests ─────────────────────────────────────────

/// Repair holder dispatches via `ControlSystem` → team enters Travelling.
#[test]
fn control_system_dispatch_authorized_sends_team_to_travelling() {
    let mut app = test_app();
    start_game(&mut app);
    damage_owned_fine_systems(&mut app, &["helm-engine-port"]);

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        team_is_travelling(&teams, 0),
        "team 0 should be travelling after ControlSystem dispatch"
    );
}

/// Non-Repair console holder sending `ControlSystem` dispatch is rejected.
#[test]
fn control_system_dispatch_unauthorized_sender_is_rejected() {
    let mut app = test_app();
    start_game(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        team_is_idle(&teams, 0),
        "team 0 should remain idle when non-Repair sender uses ControlSystem"
    );
}

/// `ControlSystem` dispatch is blocked when the repair system is AI-controlled.
#[test]
fn control_system_dispatch_rejected_when_ai_controlled() {
    let mut app = test_app();
    start_game(&mut app);

    // Set repair system to AI control.
    {
        let mut q = app
            .world_mut()
            .query_filtered::<&mut ShipSystemControlSources, With<crate::server_app::LocalShip>>();
        for mut cs in q.iter_mut(app.world_mut()) {
            cs.0.set(
                crate::ship::system_registry::repair_system_id(),
                crate::ship::control_source::ControlSource::Ai,
            );
        }
    }

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        team_is_idle(&teams, 0),
        "team 0 should remain idle when repair system is AI-controlled"
    );
}

#[test]
fn control_system_dispatch_repair_target_core_dispatches_team() {
    let mut app = test_app();
    start_game(&mut app);

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: crate::core::messages::SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Core,
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        team_is_travelling(&teams, 0),
        "team 0 should be travelling to Core after RepairTarget::Core dispatch"
    );
}

/// End-to-end TOML-driven wiring check: build the runtime `RepairTeams`
/// the same way `spawn_game_start_entities` does (parse alliance_battleship.toml
/// → RepairConfig::to_runtime → RepairTeams::new_with_timings) and
/// assert the timings match the TOML. Changing
/// `travel_duration_secs = 5.0` to e.g. `99.0` in alliance_battleship.toml
/// would fail this test.
#[test]
fn repair_teams_resource_reflects_battleship_toml_repair_block() {
    // Through the resolver (issue #876): this hull is COMPOSED, so its baked
    // bytes are no longer the document `spawn_game_start_entities` reads.
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_battleship.toml",
    )
    .expect("alliance_battleship.toml must compose and parse");
    let rc = config
        .repair
        .expect("alliance_battleship must declare [repair]");
    let timings = rc.to_runtime();
    let teams = crate::modifiers::repair_teams::RepairTeams::new_with_timings(2, timings);
    assert_eq!(teams.timings().travel_duration, rc.travel_duration_secs);
    assert_eq!(
        teams.timings().repair_rate_hp_per_sec,
        rc.repair_rate_hp_per_sec
    );
    // And the runtime defaults still match (until someone intentionally
    // diverges them).
    let baseline = crate::modifiers::repair_teams::RepairTimings::default();
    assert_eq!(teams.timings().travel_duration, baseline.travel_duration);
    assert_eq!(
        teams.timings().repair_rate_hp_per_sec,
        baseline.repair_rate_hp_per_sec
    );
}

// ── Blackboard publish tests ─────────────────────────────────────────────

#[test]
fn publish_repair_blackboard_contains_teams_and_hull() {
    let mut app = test_app();
    start_game(&mut app);
    tick(&mut app);

    let bb = repair_bb(&mut app);
    assert!(!bb.teams.is_empty(), "expected at least one team slot");
    assert!(!bb.system_hull.is_empty(), "expected system_hull entries");
    assert!(
        bb.travel_duration_secs > 0.0,
        "expected positive travel duration"
    );
}

#[test]
fn publish_repair_blackboard_reflects_dispatch() {
    let mut app = test_app();
    start_game(&mut app);
    tick(&mut app);

    dispatch_local(&mut app, 0, SystemId("helm".into()), "Helm");
    tick(&mut app);

    let bb = repair_bb(&mut app);
    assert!(
        bb.teams
            .iter()
            .any(|t| matches!(t, TeamSlot::Travelling { .. })),
        "expected a Travelling team slot after dispatch"
    );
}

#[test]
fn publish_repair_blackboard_contains_damageable_systems() {
    let mut app = test_app();
    start_game(&mut app);
    tick(&mut app);

    let bb = repair_bb(&mut app);
    assert!(
        !bb.damageable_systems.is_empty(),
        "expected damageable_systems"
    );
    assert!(
        bb.damageable_systems.contains(&SystemId("helm".into())),
        "Helm should appear in damageable_systems"
    );
    assert!(
        bb.damageable_systems.contains(&SystemId("core".into())),
        "Core should appear in damageable_systems"
    );
}

/// A queue entry whose station's only system transitions Disabled→Destroyed
/// must be RETAINED by the retain predicate (issue #1013 — the direct
/// inverse of the pre-#1013 eviction).
///
/// A destroyed system used to be treated as a lost cause, so its station's
/// request was dropped and no team was ever sent again. Now the on-site
/// sweep repairs destroyed systems, so dropping the request is precisely
/// what would strand them: nothing else in the game clears a Destroyed
/// latch.
///
/// The predicate below is a copy of `operate_repair_ai`'s `rq.entries.retain`
/// body. Change one and change the other — see
/// `prune_retains_an_all_destroyed_station_through_the_ai_loop` for the test
/// that runs the production copy.
#[test]
fn queue_entry_retained_when_all_systems_destroyed() {
    use crate::ship::config::{ShipConfig, SystemInstanceConfig};
    use crate::ship::damage::SystemHull;

    let station_id = "helm";
    let system_id = SystemId("helm".into());

    let config = ShipConfig {
        stations: vec![],
        systems: vec![SystemInstanceConfig {
            id: system_id.clone(),
            kind: "helm".into(),
            station: Some(StationId(station_id.into())),
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        }],
        power_groups: Default::default(),
        coordination_lag_secs: 2.0,
    };

    let mut hull = SystemHull::from_config(&[(system_id.clone(), 25.0_f32)]);

    let mut rq = RepairRequestQueue { entries: vec![] };
    rq.entries.push(RepairQueueEntry {
        station_id: station_id.into(),
        station_label: "Helm".into(),
        tier: crate::ship::damage::DamageTier::Disabled,
        deficit: 25.0,
    });
    assert_eq!(rq.entries.len(), 1, "entry must be present before retain");

    hull.set_hp(&system_id, 0.0);
    assert_eq!(
        hull.tier_for(&system_id),
        crate::ship::damage::DamageTier::Destroyed,
        "system must be Destroyed after set_hp(0)"
    );

    rq.entries.retain(|entry| {
        config
            .systems
            .iter()
            .filter(|s| {
                s.station.as_ref().map(|st| st.0.as_str()) == Some(entry.station_id.as_str())
            })
            .any(|s| hull.tier_for(&s.id) != crate::ship::damage::DamageTier::Operational)
    });

    assert_eq!(
        rq.entries.len(),
        1,
        "queue entry must be retained when all station systems are Destroyed \
         — the sweep can repair them"
    );

    // …and it IS dropped once the station is genuinely fixed.
    hull.set_hp(&system_id, 25.0);
    rq.entries.retain(|entry| {
        config
            .systems
            .iter()
            .filter(|s| {
                s.station.as_ref().map(|st| st.0.as_str()) == Some(entry.station_id.as_str())
            })
            .any(|s| hull.tier_for(&s.id) != crate::ship::damage::DamageTier::Operational)
    });
    assert!(
        rq.entries.is_empty(),
        "a fully repaired station's entry must still be evicted"
    );
}

/// Verifies that operate_repair_ai loops over all entities with
/// ShipSystemControlSources, gating on operate_ai (issue #590 AC).
#[test]
fn operate_repair_ai_runs_per_entity_for_ai_controlled_ships() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};

    let mut ai_resolver = ControlSourceResolver::new();
    ai_resolver.set(
        crate::ship::system_registry::repair_system_id(),
        ControlSource::Ai,
    );
    let ai_sources = ShipSystemControlSources(ai_resolver);
    let policy = ai_sources
        .0
        .policy_for(&crate::ship::system_registry::repair_system_id());
    assert!(policy.operate_ai, "AI Repair must gate through operate_ai");

    let mut human_resolver = ControlSourceResolver::new();
    human_resolver.set(
        crate::ship::system_registry::repair_system_id(),
        ControlSource::Human,
    );
    let human_sources = ShipSystemControlSources(human_resolver);
    let human_policy = human_sources
        .0
        .policy_for(&crate::ship::system_registry::repair_system_id());
    assert!(!human_policy.operate_ai, "human Repair must not operate AI");
}

// ── NPC AI repair through admission (issue #830) ─────────────────────────

/// A minimal ship config whose `helm` station owns a single `helm` fine
/// system, so `resolve_repair_target(Station("helm"))` resolves to it.
fn npc_repair_config() -> crate::ship_plugin::ShipConfigComponent {
    use crate::ship::config::{ShipConfig, SystemInstanceConfig};
    crate::ship_plugin::ShipConfigComponent(ShipConfig {
        stations: vec![],
        systems: vec![SystemInstanceConfig {
            id: SystemId("helm".into()),
            kind: "helm".into(),
            station: Some(StationId("helm".into())),
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        }],
        power_groups: Default::default(),
        coordination_lag_secs: 2.0,
    })
}

/// Build an app that runs the full per-entity admitted repair pipeline —
/// `operate_repair_ai` (emit) → `handle_dispatch_repair_team` (apply) →
/// `tick_repair_teams` — chained so the same-tick emit→apply→repair shape of
/// production holds. `Sessions` is present because `validate_and_admit`
/// consults it (the `ai:` path only needs the resource to exist).
fn npc_repair_app() -> App {
    let mut app = App::new();
    crate::ai::host::register_ai_host_env(&mut app);
    app.add_plugins(bevy::time::TimePlugin);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_millis(1000),
    ));
    app.insert_resource(crate::lobby::Sessions(
        crate::lobby::session::SessionManager::new(),
    ));
    // Stand in for `admit_system_commands`, which clears `AdmittedCommands`
    // once per tick before the AI decide systems refill it. Without this the
    // AI's `DispatchRepairTeam` would re-apply every tick and recall the team
    // (Travelling → Returning) forever, so it would never reach Repairing.
    app.add_systems(
        Update,
        (
            clear_admitted_commands,
            operate_repair_ai,
            crate::console::repair::dispatch::handle_dispatch_repair_team,
            tick_repair_teams,
        )
            .chain(),
    );
    app
}

/// Test-only mirror of admission's per-tick `AdmittedCommands` clear.
fn clear_admitted_commands(mut q: Query<&mut crate::core::messages::AdmittedCommands>) {
    for mut admitted in q.iter_mut() {
        admitted.0.clear();
    }
}

/// Spawn an NPC ship (Ship marker, no LocalShip) whose Repair system is
/// under the given control source, with a `helm` hull damaged by `damage`,
/// a queue entry naming the `helm` station, an `EntityUuid` for its `ai:`
/// token, and an empty `AdmittedCommands`.
fn spawn_npc_repair(
    app: &mut App,
    source: crate::ship::control_source::ControlSource,
    damage: f32,
) -> Entity {
    use crate::ship::control_source::ControlSourceResolver;
    let mut resolver = ControlSourceResolver::new();
    resolver.set(repair_system_id(), source);

    let mut hull =
        crate::ship::damage::SystemHull::from_config(&[(SystemId("helm".into()), 100.0_f32)]);
    let mut rng = crate::sim_rng::unseeded_test_rng();
    hull.apply_damage(damage, &mut rng);

    let mut queue = RepairRequestQueue::default();
    queue.push_or_merge(RepairQueueEntry {
        station_id: "helm".into(),
        station_label: "Helm".into(),
        tier: DamageTier::Disabled,
        deficit: damage,
    });

    app.world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid("npc-repair-1".into()),
            ShipSystemControlSources(resolver),
            ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(2)),
            crate::entities::spawner::EntitySystemHull(hull),
            crate::modifiers::ShipModifiers::new(),
            queue,
            npc_repair_config(),
            crate::core::messages::AdmittedCommands::default(),
            // The AUTHORED `[repair.selector]` block: since #885b stage 5d
            // an NPC with no selector component ranks nothing and dispatches
            // no team.
            RepairTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("repair")
                    .to_selector()
                    .expect("the shipped Repair selector decodes"),
                power_rating: None,
            },
        ))
        .id()
}

/// The NPC applier consumes the AI operator's admitted `DispatchRepairTeam`
/// in the same tick and sends a team travelling — proving the per-entity
/// emit→admit→apply chain runs on an NPC ship with no LocalShip marker.
#[test]
fn npc_applier_consumes_ai_emitted_dispatch_same_tick() {
    let mut app = npc_repair_app();
    let npc = spawn_npc_repair(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        80.0,
    );

    // One warm-up tick (TimePlugin baseline). The AI emits into the NPC's
    // own AdmittedCommands and the applier dispatches on the same tick.
    app.update();

    let teams = app
        .world()
        .get::<ShipRepairTeams>(npc)
        .expect("NPC must have ShipRepairTeams");
    assert!(
        teams
            .0
            .slots()
            .iter()
            .any(|s| matches!(s, TeamSlot::Travelling { .. })),
        "the NPC applier must have dispatched a team from the AI's own \
         AdmittedCommands, got {:?}",
        teams.0.slots()
    );
}

/// Regression for PRD #597 gap-5 (retained through #830): an NPC ship's
/// AI-driven repair restores its own hull over time — now flowing through
/// admission (`operate_repair_ai` emits, `handle_dispatch_repair_team`
/// applies, `tick_repair_teams` heals) rather than a direct team write.
#[test]
fn npc_ship_with_repair_teams_regenerates_hull_over_time() {
    let mut app = npc_repair_app();
    let npc = spawn_npc_repair(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        80.0,
    );
    let hp_before = app
        .world()
        .get::<crate::entities::spawner::EntitySystemHull>(npc)
        .unwrap()
        .0
        .total_current();

    // 200 iterations comfortably covers the 5 s travel + repair time.
    for _ in 0..200 {
        app.update();
    }

    let hp_after = app
        .world()
        .get::<crate::entities::spawner::EntitySystemHull>(npc)
        .expect("NPC must still have hull component")
        .0
        .total_current();
    assert!(
        hp_after > hp_before,
        "NPC hull HP must increase after AI-admitted dispatch + repair \
         (before={hp_before}, after={hp_after})"
    );
}

/// A human-held Repair system rejects an `ai:` emission at the admission
/// gate: `validate_and_admit` returns false and nothing is admitted. This is
/// the symmetry contract — the AI operator gates on `operate_ai` before
/// emitting, and admission independently enforces it.
#[test]
fn human_held_repair_rejects_ai_emission() {
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    let mut resolver = ControlSourceResolver::new();
    resolver.set(repair_system_id(), ControlSource::Human);
    let sources = ShipSystemControlSources(resolver);
    let sessions = crate::lobby::Sessions(crate::lobby::session::SessionManager::new());
    let config = npc_repair_config();
    let mut admitted = crate::core::messages::AdmittedCommands::default();

    let admitted_ok = crate::command_admission::validate_and_admit(
        "ai:npc-repair-1",
        repair_system_id(),
        SystemControlPayload::DispatchRepairTeam {
            team_idx: 0,
            target: RepairTarget::Station(StationId("helm".into())),
        },
        &sources,
        &sessions,
        &config.0,
        &mut admitted,
    );
    assert!(
        !admitted_ok,
        "ai: emission must be rejected when repair is human-held"
    );
    assert!(
        admitted.0.is_empty(),
        "no command may be admitted for a human-held repair system"
    );
}

/// SetRepairPriority is ignored when the team is not in Repairing state
/// (e.g. idle). The handler runs but `RepairTeams::set_priority` returns
/// false and the slot stays unaffected.
#[test]
fn set_repair_priority_on_idle_team_is_ignored() {
    let mut app = test_app();
    start_game(&mut app);
    tick(&mut app);

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: SystemId("repair".into()),
            payload: SystemControlPayload::SetRepairPriority {
                team_idx: 0,
                priority: 3,
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        team_is_idle(&teams, 0),
        "team 0 should remain idle after SetRepairPriority on idle team"
    );
}

/// SetRepairPriority sets the team's priority when the team is actually
/// in Repairing state. First dispatch the team, wait for it to arrive,
/// then set priority.
#[test]
fn set_repair_priority_on_repairing_team_sets_priority() {
    let mut app = test_app();
    start_game(&mut app);
    // Damage a system the `helm` station OWNS so the dispatch resolves to
    // it and the team has work to do on arrival rather than leaving again.
    // (Damaging the bare `helm` hull ROW no longer works: it is a station
    // NAME, and since the #1013 review `resolve_repair_target` refuses to
    // fall back to a name that is also an ownerless hull row.)
    damage_owned_fine_systems(&mut app, &["helm-engine-port"]);

    // Dispatch team 0 to helm.
    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    // Tick past travel time (5s default) so team arrives and enters Repairing.
    for _ in 0..30 {
        tick(&mut app);
    }

    // Verify team is Repairing.
    {
        let teams = local_teams(&mut app);
        assert!(
            matches!(&teams.0.slots()[0], TeamSlot::Repairing { .. }),
            "team 0 should be Repairing after travel, got {:?}",
            teams.0.slots()[0]
        );
    }

    // Now send SetRepairPriority.
    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: SystemId("repair".into()),
            payload: SystemControlPayload::SetRepairPriority {
                team_idx: 0,
                priority: 2,
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        matches!(
            &teams.0.slots()[0],
            TeamSlot::Repairing {
                priority: Some(2),
                ..
            }
        ),
        "team 0 should have priority=2 after SetRepairPriority, got {:?}",
        teams.0.slots()[0]
    );
}

// ── Naming a system instead of an ordinal (issue #1015) ──────────────────

/// Put team 0 on site at the `helm` station with three systems in three
/// different states, so the remaining work has a non-trivial ranking:
/// the team lands on `helm-thrust` (the worst, Disabled) and what is left
/// ranks `helm-engine-port` first and `helm-engine-starboard` second.
fn team_on_site_at_helm_with_two_jobs_left(app: &mut App) {
    damage_owned_fine_systems_to(
        app,
        &[
            ("helm-thrust", 0.2),           // Disabled — the dispatch target
            ("helm-engine-port", 0.5),      // Damaged, rank 1 of what remains
            ("helm-engine-starboard", 0.6), // Damaged, rank 2
        ],
    );
    push(
        app,
        "eng",
        ClientMessage::ControlSystem {
            target: SystemId("repair".into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    for _ in 0..30 {
        tick(app);
    }
    let teams = local_teams(app);
    assert!(
        matches!(
            &teams.0.slots()[0],
            TeamSlot::Repairing { system_id: Some(s), .. } if s.0 == "helm-thrust"
        ),
        "fixture precondition: team 0 works the worst helm system first, got {:?}",
        teams.0.slots()[0]
    );
}

/// The repair console's damaged-systems tap, end to end through admission:
/// the client names a SYSTEM and the host records that system on the team's
/// slot. It records no ordinal — `priority` is #1013's standing per-team
/// instruction and a tap does not touch it, because a rank frozen at tap
/// time can select a different system by the time the hand-off consumes it
/// (see `RepairTeams::prioritise_system`).
#[test]
fn set_repair_target_priority_pins_the_tapped_system_host_side() {
    let mut app = test_app();
    start_game(&mut app);
    team_on_site_at_helm_with_two_jobs_left(&mut app);

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: SystemId("repair".into()),
            payload: SystemControlPayload::SetRepairTargetPriority {
                system_id: SystemId("helm-engine-starboard".into()),
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        matches!(
            &teams.0.slots()[0],
            TeamSlot::Repairing {
                priority: None,
                priority_system_id: Some(pinned),
                ..
            } if pinned.0 == "helm-engine-starboard"
        ),
        "the tap must pin the named system and write no ordinal, got {:?}",
        teams.0.slots()[0]
    );
}

/// The sweep actually goes there: after the tapped system is prioritised the
/// team hands off to it rather than to the worse-ranked one it would
/// otherwise have taken. This is the acceptance criterion in observable
/// form — no assertion on the ordinal at all.
#[test]
fn set_repair_target_priority_sends_the_sweep_to_the_tapped_system() {
    let mut app = test_app();
    start_game(&mut app);
    team_on_site_at_helm_with_two_jobs_left(&mut app);

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: SystemId("repair".into()),
            payload: SystemControlPayload::SetRepairTargetPriority {
                system_id: SystemId("helm-engine-starboard".into()),
            },
        },
    );
    // Long enough for the team to finish `helm-thrust` and hand off.
    for _ in 0..600 {
        tick(&mut app);
        let teams = local_teams(&mut app);
        if let TeamSlot::Repairing {
            system_id: Some(sid),
            ..
        } = &teams.0.slots()[0]
        {
            if sid.0 != "helm-thrust" {
                assert_eq!(
                    sid.0, "helm-engine-starboard",
                    "the sweep must hand off to the TAPPED system, not the \
                     worst remaining one"
                );
                return;
            }
        }
    }
    panic!("team 0 never handed off to its next job");
}

/// A tap naming a system no on-site team is sweeping is a silent no-op —
/// the same nothing-happens a dispatch to an undamaged station produces.
#[test]
fn set_repair_target_priority_for_an_unswept_system_changes_nothing() {
    let mut app = test_app();
    start_game(&mut app);
    team_on_site_at_helm_with_two_jobs_left(&mut app);

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: SystemId("repair".into()),
            payload: SystemControlPayload::SetRepairTargetPriority {
                system_id: SystemId("tactical-phaser-fore".into()),
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        matches!(
            &teams.0.slots()[0],
            TeamSlot::Repairing {
                priority: None,
                priority_system_id: None,
                ..
            }
        ),
        "got {:?}",
        teams.0.slots()[0]
    );
}

/// The same station-ownership gate every repair command sits behind: a token
/// that does not hold Engineering cannot steer the sweep. Admission decides
/// this from the TARGET system, so the new payload inherits it — this test
/// is what pins that inheritance rather than assuming it.
#[test]
fn set_repair_target_priority_from_a_non_engineering_token_is_rejected() {
    let mut app = test_app();
    start_game(&mut app);
    team_on_site_at_helm_with_two_jobs_left(&mut app);

    push(
        &mut app,
        "captain",
        ClientMessage::ControlSystem {
            target: SystemId("repair".into()),
            payload: SystemControlPayload::SetRepairTargetPriority {
                system_id: SystemId("helm-engine-starboard".into()),
            },
        },
    );
    tick(&mut app);

    let teams = local_teams(&mut app);
    assert!(
        matches!(
            &teams.0.slots()[0],
            TeamSlot::Repairing {
                priority: None,
                priority_system_id: None,
                ..
            }
        ),
        "an unauthorised tap must not reach the team, got {:?}",
        teams.0.slots()[0]
    );
}

// ── Authored repair-target ranking (issue #785) ──────────────────────────
//
// AC6: every assertion below reads OBSERVABLE state — `TeamSlot` variants
// and their `system_id`, `RepairRequestQueue.entries`, and
// `EntitySystemHull` HP — never a `TargetSelector::select` return value.
// The selector's own semantics are unit-tested in `src/ai/selector.rs`.

/// Two stations, each owning one fine system, so a station dispatch resolves
/// to a distinct observable `system_id`.
fn two_station_config() -> crate::ship_plugin::ShipConfigComponent {
    use crate::ship::config::{ShipConfig, SystemInstanceConfig};
    let sys = |id: &str, station: &str| SystemInstanceConfig {
        id: SystemId(id.into()),
        kind: "generic".into(),
        station: Some(StationId(station.into())),
        ai_only: false,
        human_seeking: false,
        seek_order: Vec::new(),
        power_group: None,
        marker: None,
        config: None,
    };
    crate::ship_plugin::ShipConfigComponent(ShipConfig {
        stations: vec![],
        systems: vec![sys("alpha-sys", "alpha"), sys("bravo-sys", "bravo")],
        power_groups: Default::default(),
        coordination_lag_secs: 2.0,
    })
}

/// Spawn a two-station NPC. `alpha_hp` / `bravo_hp` are absolute HP out of
/// 100, and each station gets a repair-request queue entry carrying the tier
/// the hull actually reports (the coordination-delivered reading the AI
/// ranks). `teams` is the team count. An optional authored selector is
/// attached as `RepairTargetSelector`; `None` uses the
/// SHIPPED authored one (there is no host fallback since #885b stage 5d).
fn spawn_two_station_npc(
    app: &mut App,
    source: crate::ship::control_source::ControlSource,
    alpha_hp: f32,
    bravo_hp: f32,
    teams: usize,
    selector: Option<crate::entities::config::FineSystemAiSelectorToml>,
) -> Entity {
    use crate::ship::control_source::ControlSourceResolver;
    let mut resolver = ControlSourceResolver::new();
    resolver.set(repair_system_id(), source);

    let mut hull = crate::ship::damage::SystemHull::from_config(&[
        (SystemId("alpha-sys".into()), 100.0_f32),
        (SystemId("bravo-sys".into()), 100.0_f32),
    ]);
    hull.set_hp(&SystemId("alpha-sys".into()), alpha_hp);
    hull.set_hp(&SystemId("bravo-sys".into()), bravo_hp);

    let mut queue = RepairRequestQueue::default();
    for (station, sid, hp) in [
        ("alpha", "alpha-sys", alpha_hp),
        ("bravo", "bravo-sys", bravo_hp),
    ] {
        // Everything non-Operational is queued, Destroyed included: since
        // issue #1013 a destroyed station is a real repair job, and the
        // production enqueue (`RepairRequestQueue::push_or_merge`) no
        // longer drops it either.
        let tier = hull.tier_for(&SystemId(sid.into()));
        if tier == DamageTier::Operational {
            continue;
        }
        queue.push_or_merge(RepairQueueEntry {
            station_id: station.into(),
            station_label: station.into(),
            tier,
            deficit: 100.0 - hp,
        });
    }

    let entity = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid("npc-repair-2".into()),
            ShipSystemControlSources(resolver),
            ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(teams)),
            crate::entities::spawner::EntitySystemHull(hull),
            crate::modifiers::ShipModifiers::new(),
            queue,
            two_station_config(),
            crate::core::messages::AdmittedCommands::default(),
            // The AUTHORED `[repair.selector]` block, unless the caller
            // supplies its own below. Since #885b stage 5d there is no host
            // fallback: a ship with no selector component dispatches nothing.
            RepairTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("repair")
                    .to_selector()
                    .expect("the shipped Repair selector decodes"),
                power_rating: None,
            },
        ))
        .id();
    if let Some(cfg) = selector {
        app.world_mut()
            .entity_mut(entity)
            .insert(RepairTargetSelector {
                selector: cfg.to_selector().expect("test selector must parse"),
                power_rating: None,
            });
    }
    entity
}

/// The observable target system of a team slot, if it has one.
fn slot_system(slot: &TeamSlot) -> Option<String> {
    match slot {
        TeamSlot::Travelling { system_id, .. } | TeamSlot::Repairing { system_id, .. } => {
            system_id.as_ref().map(|s| s.0.clone())
        }
        _ => None,
    }
}

fn team_systems(app: &App, entity: Entity) -> Vec<Option<String>> {
    app.world()
        .get::<ShipRepairTeams>(entity)
        .expect("ship must carry ShipRepairTeams")
        .0
        .slots()
        .iter()
        .map(slot_system)
        .collect()
}

/// Issue #891 stage 2, per-host both-directions proof for the Repair
/// target selector: an authored eligibility gated on a world flag
/// dispatches nothing while the flag is clear and dispatches the damaged
/// station once it is set.
#[test]
fn operate_repair_ai_flag_guard_reads_the_world_in_both_directions() {
    use crate::entities::config::{FineSystemAiSelectorToml, ScoreTermToml};
    let flag_gated = FineSystemAiSelectorToml {
        param: std::collections::HashMap::new(),
        sources: vec![crate::entities::config::SELECTOR_SOURCE_DAMAGED_STATIONS.to_string()],
        horizon: 1.0e9,
        switch_margin: 0.0,
        eligibility: "candidate_fact(source_repair_request) > 0 \
                      and flag(damage_control_released)"
            .to_string(),
        score: vec![ScoreTermToml {
            when: "candidate_fact(source_repair_request) > 0".to_string(),
            weight: 1.0,
        }],
    };

    let mut app = npc_repair_app();
    app.init_resource::<crate::world::server::WorldContentRuntime>();
    // alpha 50/100 → Damaged, one team free.
    let npc = spawn_two_station_npc(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        50.0,
        100.0,
        1,
        Some(flag_gated),
    );

    // Flag CLEAR → nothing is eligible, the team stays idle.
    app.update();
    assert_eq!(
        team_systems(&app, npc)[0],
        None,
        "with the world flag clear the eligibility must dispatch nothing"
    );

    // Flag SET → the SAME eligibility dispatches to the damaged station.
    app.world_mut()
        .resource_mut::<crate::world::server::WorldContentRuntime>()
        .flags
        .set_flag("damage_control_released");
    app.update();
    assert_eq!(
        team_systems(&app, npc)[0].as_deref(),
        Some("alpha-sys"),
        "with the world flag set the same eligibility must dispatch the team"
    );
}

/// AC2 baseline: with the canonical default selector the worst-tier station
/// wins, exactly as the retired `(tier desc, deficit desc)` comparator did.
#[test]
fn default_repair_selector_dispatches_worst_tier_station_first() {
    let mut app = npc_repair_app();
    // alpha 50/100 → Damaged; bravo 10/100 → Disabled (worse tier).
    let npc = spawn_two_station_npc(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        50.0,
        10.0,
        1,
        None,
    );
    app.update();

    assert_eq!(
        team_systems(&app, npc)[0].as_deref(),
        Some("bravo-sys"),
        "the worse-tier station must win the default ranking"
    );
}

/// AC2: an AUTHORED eligibility changes which station is dispatched, proving
/// the decision comes from data and not from a Rust comparator. Here the
/// author restricts eligibility to the merely-Damaged tier, so the team goes
/// to `alpha` — the opposite of the default ranking asserted above.
#[test]
fn authored_repair_selector_drives_dispatch() {
    use crate::entities::config::{FineSystemAiSelectorToml, ScoreTermToml};
    let authored = FineSystemAiSelectorToml {
        param: std::collections::HashMap::from([("tier_weight".to_string(), 10.0_f32)]),
        sources: vec![crate::entities::config::SELECTOR_SOURCE_DAMAGED_STATIONS.to_string()],
        horizon: 1.0e9,
        switch_margin: 0.0,
        eligibility: "candidate_fact(source_repair_request) > 0 \
                      and candidate_fact(assigned) < 1 \
                      and candidate_fact(tier_ordinal) == 1"
            .to_string(),
        score: vec![ScoreTermToml {
            when: "candidate_fact(tier_ordinal) >= 1".to_string(),
            weight: 10.0,
        }],
    };

    let mut app = npc_repair_app();
    let npc = spawn_two_station_npc(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        50.0,
        10.0,
        1,
        Some(authored),
    );
    app.update();

    assert_eq!(
        team_systems(&app, npc)[0].as_deref(),
        Some("alpha-sys"),
        "the authored eligibility must override the default worst-tier pick"
    );
}

/// AC2/AC4: two free teams pick two DISTINCT stations in one tick — the
/// per-team exclusion, expressed through the authored `assigned` fact.
#[test]
fn two_free_teams_are_dispatched_to_distinct_stations() {
    let mut app = npc_repair_app();
    let npc = spawn_two_station_npc(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        50.0,
        10.0,
        2,
        None,
    );
    app.update();

    let systems = team_systems(&app, npc);
    assert_eq!(
        systems,
        vec![Some("bravo-sys".to_string()), Some("alpha-sys".to_string())],
        "ascending team indices must take the ranking in order, without \
         both teams piling onto the same station"
    );
}

/// AC4 determinism: two stations at the SAME tier and the SAME deficit must
/// resolve to the same station on every run — the selector's smallest-key
/// tie-break, not queue insertion order. Repeated across fresh apps because
/// a single run cannot observe executor variation.
#[test]
fn tied_repair_candidates_resolve_deterministically() {
    for _ in 0..20 {
        let mut app = npc_repair_app();
        let npc = spawn_two_station_npc(
            &mut app,
            crate::ship::control_source::ControlSource::Ai,
            50.0,
            50.0,
            1,
            None,
        );
        app.update();
        assert_eq!(
            team_systems(&app, npc)[0].as_deref(),
            Some("alpha-sys"),
            "a full tie must always resolve to the smallest station id"
        );
    }
}

/// AC4 "completed targets removed": once a station's systems are back to
/// Operational its repair-request entry is pruned and no further team is
/// sent there. AC5 falls out of the same observation — the retained pick
/// lives only in the authoritative `TeamSlot`, so a healed station simply
/// stops being a candidate with no AI state to reset.
#[test]
fn repaired_station_entry_is_removed_and_not_redispatched() {
    let mut app = npc_repair_app();
    let npc = spawn_two_station_npc(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        50.0,
        100.0,
        1,
        None,
    );
    app.update();
    assert_eq!(
        team_systems(&app, npc)[0].as_deref(),
        Some("alpha-sys"),
        "the only damaged station must be picked first"
    );

    // Heal alpha outright, then tick: the entry must vanish.
    app.world_mut()
        .get_mut::<crate::entities::spawner::EntitySystemHull>(npc)
        .unwrap()
        .0
        .set_hp(&SystemId("alpha-sys".into()), 100.0);
    app.update();

    assert!(
        app.world()
            .get::<RepairRequestQueue>(npc)
            .unwrap()
            .entries
            .is_empty(),
        "a fully repaired station's queue entry must be removed"
    );

    // Free every team and tick again: nothing is eligible, so nothing is sent.
    app.world_mut().get_mut::<ShipRepairTeams>(npc).unwrap().0 =
        crate::modifiers::repair_teams::RepairTeams::new(1);
    app.update();
    assert!(
        team_systems(&app, npc).iter().all(|s| s.is_none()),
        "no team may be dispatched once every reported station is repaired"
    );
}

/// The #779 EMPTY-FACTS lesson: candidate facts are really seeded, so an
/// authored `candidate_fact(...)` guard actually fires. The same selector is
/// run twice with only its threshold param moved across the observed
/// `damage_fraction` (0.5) — below it dispatches, above it does not.
#[test]
fn authored_candidate_fact_guard_fires_on_seeded_damage_fraction() {
    use crate::entities::config::{FineSystemAiSelectorToml, ScoreTermToml};
    let selector_with = |threshold: f32| FineSystemAiSelectorToml {
        param: std::collections::HashMap::from([("min_damage".to_string(), threshold)]),
        sources: vec![crate::entities::config::SELECTOR_SOURCE_DAMAGED_STATIONS.to_string()],
        horizon: 1.0e9,
        switch_margin: 0.0,
        eligibility: "candidate_fact(assigned) < 1 \
                      and candidate_fact(damage_fraction) >= param(min_damage)"
            .to_string(),
        score: vec![ScoreTermToml {
            when: "candidate_fact(damage_fraction) >= param(min_damage)".to_string(),
            weight: 1.0,
        }],
    };

    // Threshold below the seeded 0.5 damage fraction → the guard fires.
    let mut app = npc_repair_app();
    let npc = spawn_two_station_npc(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        50.0,
        100.0,
        1,
        Some(selector_with(0.4)),
    );
    app.update();
    assert_eq!(
        team_systems(&app, npc)[0].as_deref(),
        Some("alpha-sys"),
        "a guard under the seeded damage_fraction must fire — if facts were \
         empty this would never dispatch"
    );

    // Threshold above it → the same guard cannot fire, so nothing is sent.
    let mut app = npc_repair_app();
    let npc = spawn_two_station_npc(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        50.0,
        100.0,
        1,
        Some(selector_with(0.9)),
    );
    app.update();
    assert!(
        team_systems(&app, npc)[0].is_none(),
        "a guard above the seeded damage_fraction must not fire"
    );
}

/// AC5 human takeover: with Repair human-held the AI never emits, so no team
/// leaves the bay however damaged the ship is.
#[test]
fn human_held_repair_stops_ai_dispatch() {
    let mut app = npc_repair_app();
    let npc = spawn_two_station_npc(
        &mut app,
        crate::ship::control_source::ControlSource::Human,
        50.0,
        10.0,
        2,
        None,
    );
    for _ in 0..5 {
        app.update();
    }
    assert!(
        team_systems(&app, npc).iter().all(|s| s.is_none()),
        "a human-held Repair system must not auto-dispatch"
    );
}

/// AC3 + observable outcome: the authored ranking flows through the ordinary
/// typed input and the ordinary team-assignment path, so the picked
/// station's fine system actually gains HP.
#[test]
fn authored_ranking_restores_hull_through_the_normal_dispatch_path() {
    let mut app = npc_repair_app();
    let npc = spawn_two_station_npc(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        50.0,
        10.0,
        1,
        None,
    );
    let before = app
        .world()
        .get::<crate::entities::spawner::EntitySystemHull>(npc)
        .unwrap()
        .0
        .current_for(&SystemId("bravo-sys".into()))
        .unwrap();
    for _ in 0..200 {
        app.update();
    }
    let after = app
        .world()
        .get::<crate::entities::spawner::EntitySystemHull>(npc)
        .unwrap()
        .0
        .current_for(&SystemId("bravo-sys".into()))
        .unwrap();
    assert!(
        after > before,
        "the ranked station's system must actually heal (before={before}, \
         after={after})"
    );
}

/// AC1/AC2 for the `core-bucket` source: a repair request naming the
/// ownerless `core` bucket dispatches `RepairTarget::Core` — observable as
/// the team slot's `core` system id — AND outranks a SAME-TIER but less
/// damaged real station.
///
/// This is the regression that the station-owned reading path could not
/// see: `core` owns no station in `ShipConfig` (validation forbids one), so
/// the station scan reports `(0.0, 0.0, 0)` for it. Left seeded, the core
/// candidate scores nothing from the deficit ladder and the healthier
/// `helm` wins.
#[test]
fn core_bucket_request_outranks_less_damaged_same_tier_station() {
    use crate::ship::config::{ShipConfig, SystemInstanceConfig};

    let mut app = npc_repair_app();
    let mut resolver = crate::ship::control_source::ControlSourceResolver::new();
    resolver.set(
        repair_system_id(),
        crate::ship::control_source::ControlSource::Ai,
    );

    // `helm-sys` at 20/100 → Disabled, damage fraction 0.80, deficit 80.
    // The ownerless `core` hull entry at 10/100 → Disabled, damage fraction
    // 0.90, deficit 90. Same tier, so only the deficit ladder separates
    // them — and only if the core candidate carries its real hull reading.
    let mut hull = crate::ship::damage::SystemHull::from_config(&[
        (SystemId("helm-sys".into()), 100.0_f32),
        (SystemId(REPAIR_CORE_BUCKET_KEY.into()), 100.0_f32),
    ]);
    hull.set_hp(&SystemId("helm-sys".into()), 20.0);
    hull.set_hp(&SystemId(REPAIR_CORE_BUCKET_KEY.into()), 10.0);
    assert_eq!(
        hull.tier_for(&SystemId("helm-sys".into())),
        DamageTier::Disabled
    );
    assert_eq!(
        hull.tier_for(&SystemId(REPAIR_CORE_BUCKET_KEY.into())),
        DamageTier::Disabled,
        "the scenario only bites while both candidates share a tier"
    );

    let mut queue = RepairRequestQueue::default();
    queue.push_or_merge(RepairQueueEntry {
        station_id: "helm".into(),
        station_label: "helm".into(),
        tier: DamageTier::Disabled,
        deficit: 80.0,
    });
    // `damage_sync` files an ownerless system's request under this id.
    queue.push_or_merge(RepairQueueEntry {
        station_id: REPAIR_CORE_BUCKET_KEY.into(),
        station_label: REPAIR_CORE_BUCKET_KEY.into(),
        tier: DamageTier::Disabled,
        deficit: 90.0,
    });

    // NOTE: no station named `core` — `ShipConfig` validation forbids it,
    // which is exactly why the core bucket needs its hull-side reading.
    let config = crate::ship_plugin::ShipConfigComponent(ShipConfig {
        stations: vec![],
        systems: vec![SystemInstanceConfig {
            id: SystemId("helm-sys".into()),
            kind: "generic".into(),
            station: Some(StationId("helm".into())),
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        }],
        power_groups: Default::default(),
        coordination_lag_secs: 2.0,
    });

    let npc = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid("npc-repair-core".into()),
            ShipSystemControlSources(resolver),
            ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(1)),
            crate::entities::spawner::EntitySystemHull(hull),
            crate::modifiers::ShipModifiers::new(),
            queue,
            config,
            crate::core::messages::AdmittedCommands::default(),
            // The AUTHORED `[repair.selector]` block — the deficit ladder
            // under test lives in it, and since #885b stage 5d nothing
            // supplies one for a ship that carries no component.
            RepairTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("repair")
                    .to_selector()
                    .expect("the shipped Repair selector decodes"),
                power_rating: None,
            },
        ))
        .id();

    app.update();

    assert_eq!(
        team_systems(&app, npc)[0].as_deref(),
        Some(REPAIR_CORE_BUCKET_KEY),
        "the more-damaged core bucket must win over the same-tier `helm` \
         station, and must dispatch as RepairTarget::Core"
    );
}

// ── The ownerless GROUP, not just the `core` row (issue #1013 review) ─────

/// A CRUISER-SHAPED hull: two ownerless rows, not one.
///
/// `alliance_cruiser` authors a `science` `[[hull.system_hull]]` with no
/// `[[system]]` behind it, so it joins `core` in the ownerless bucket —
/// every other shipped hull's ownerless group is `{core}` alone, which is
/// why unit fixtures never exposed this. `hull` carries `core` at full HP,
/// the ownerless `science` row destroyed, and a station-owned `helm-sys` at
/// full HP that must NOT be mistaken for ownerless.
fn spawn_two_ownerless_rows(
    app: &mut App,
    team_count: usize,
    core_hp: f32,
    science_max: f32,
) -> Entity {
    use crate::ship::config::{ShipConfig, SystemInstanceConfig};
    let mut resolver = crate::ship::control_source::ControlSourceResolver::new();
    resolver.set(
        repair_system_id(),
        crate::ship::control_source::ControlSource::Ai,
    );

    let mut hull = crate::ship::damage::SystemHull::from_config(&[
        (SystemId(REPAIR_CORE_BUCKET_KEY.into()), 20.0_f32),
        (SystemId("science".into()), science_max),
        (SystemId("helm-sys".into()), 20.0),
    ]);
    hull.set_hp(&SystemId(REPAIR_CORE_BUCKET_KEY.into()), core_hp);
    hull.set_hp(&SystemId("science".into()), 0.0);

    // `damage_sync` files EVERY ownerless system's request under this one
    // id, so the destroyed `science` row is reported as a `core` request.
    let mut queue = RepairRequestQueue::default();
    queue.push_or_merge(RepairQueueEntry {
        station_id: REPAIR_CORE_BUCKET_KEY.into(),
        station_label: REPAIR_CORE_BUCKET_KEY.into(),
        tier: DamageTier::Destroyed,
        deficit: science_max,
    });

    // Only `helm-sys` is described; `core` and `science` are ownerless.
    let config = crate::ship_plugin::ShipConfigComponent(ShipConfig {
        stations: vec![],
        systems: vec![SystemInstanceConfig {
            id: SystemId("helm-sys".into()),
            kind: "generic".into(),
            station: Some(StationId("helm".into())),
            ai_only: false,
            human_seeking: false,
            seek_order: Vec::new(),
            power_group: None,
            marker: None,
            config: None,
        }],
        power_groups: Default::default(),
        coordination_lag_secs: 2.0,
    });

    app.world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::entities::spawner::EntityUuid("npc-two-ownerless".into()),
            ShipSystemControlSources(resolver),
            ShipRepairTeams(crate::modifiers::repair_teams::RepairTeams::new(team_count)),
            crate::entities::spawner::EntitySystemHull(hull),
            crate::modifiers::ShipModifiers::new(),
            queue,
            config,
            crate::core::messages::AdmittedCommands::default(),
            RepairTargetSelector {
                selector: crate::entities::authored_ai_pins::shipped_selector_toml("repair")
                    .to_selector()
                    .expect("the shipped Repair selector decodes"),
                power_rating: None,
            },
        ))
        .id()
}

fn queue_station_ids(app: &App, entity: Entity) -> Vec<String> {
    app.world()
        .get::<RepairRequestQueue>(entity)
        .expect("ship must carry RepairRequestQueue")
        .entries
        .iter()
        .map(|e| e.station_id.clone())
        .collect()
}

/// The core request must survive while ANY ownerless row still needs a team,
/// not just while the literal `core` row does — and the team it keeps alive
/// must actually reach and repair that row.
///
/// Before the fix the prune tested `tier_for(SystemId("core"))` alone, so a
/// cruiser with `core` Operational and `science` destroyed had the entry
/// evicted on the first tick: no request, no candidate, no team, and nothing
/// else in the game clears a Destroyed latch — the row stayed destroyed for
/// the rest of the match.
#[test]
fn core_request_survives_while_a_non_core_ownerless_row_is_damaged() {
    let mut app = npc_repair_app();
    // `core` at FULL HP — the whole point: the literal core row is
    // Operational and only its ownerless sibling needs work.
    let npc = spawn_two_ownerless_rows(&mut app, 1, 20.0, 4.0);

    app.update();

    assert_eq!(
        queue_station_ids(&app, npc),
        vec![REPAIR_CORE_BUCKET_KEY.to_string()],
        "the core request must be retained while a non-core ownerless row \
         is non-Operational, even with `core` itself Operational"
    );
    assert_eq!(
        team_systems(&app, npc)[0].as_deref(),
        Some(REPAIR_CORE_BUCKET_KEY),
        "and a team must be dispatched for it"
    );

    // 5 s travel, then the sweep on to `science` and 8 s to restore 4 HP at
    // 0.5 HP/s.
    // The virtual clock is clamped to its 0.25 s `max_delta`, so an update is
    // 0.25 s: 20 ticks of travel, then 4 HP at 0.5 HP/s is 32 more.
    for _ in 0..120 {
        app.update();
    }

    let hull = &app
        .world()
        .get::<crate::entities::spawner::EntitySystemHull>(npc)
        .expect("hull")
        .0;
    assert!(
        hull.current_for(&SystemId("science".into())).unwrap() > 0.0,
        "the dispatched team must have swept from `core` on to the destroyed \
         ownerless `science` row and restored it"
    );
    assert_eq!(
        hull.tier_for(&SystemId("science".into())),
        DamageTier::Operational,
        "and worked it back to Operational"
    );
    assert!(
        queue_station_ids(&app, npc).is_empty(),
        "only once EVERY ownerless row is Operational is the request pruned"
    );
}

/// AC4 ("N free teams pick N DISTINCT stations") must hold for the WHOLE
/// visit, including after the team sweeps off the literal `core` row.
///
/// A team that walks from `core` to the ownerless `science` row is still
/// committed to the core bucket, but the config cannot say so — an ownerless
/// row is by definition one the config does not describe. Before the fix
/// `committed_station_for_slot` returned `None` for it, the core bucket
/// dropped out of `excluded`, and a second team was dispatched to the group
/// the first was already sweeping.
#[test]
fn a_second_team_is_not_dispatched_to_a_core_bucket_being_swept() {
    let mut app = npc_repair_app();
    // `core` damaged but cheap to finish (2 of 20 HP missing ⇒ 4 s of work
    // after 5 s of travel), so the team sweeps on to the long `science` job
    // (40 HP ⇒ 80 s) well inside the run and is still on it at the end.
    let npc = spawn_two_ownerless_rows(&mut app, 2, 18.0, 40.0);

    let mut team_zero_reached_science = false;
    for tick_idx in 0..120 {
        app.update();
        let systems = team_systems(&app, npc);
        assert_eq!(
            systems[1], None,
            "team 1 must stay idle while team 0 sweeps the core bucket \
             (tick {tick_idx}, slots {systems:?})"
        );
        if systems[0].as_deref() == Some("science") {
            team_zero_reached_science = true;
        }
    }
    assert!(
        team_zero_reached_science,
        "fixture precondition: team 0 must actually sweep off the `core` row \
         on to `science`, or the regression is not being exercised"
    );
}

/// `pop_worst` / `peek` must not depend on queue insertion order when two
/// entries tie on tier and deficit (the residual `max_by` last-wins edge).
#[test]
fn queue_severity_tie_breaks_on_smallest_station_id() {
    let entry = |station: &str| RepairQueueEntry {
        station_id: station.into(),
        station_label: station.into(),
        tier: DamageTier::Damaged,
        deficit: 10.0,
    };
    for order in [["alpha", "bravo"], ["bravo", "alpha"]] {
        let mut rq = RepairRequestQueue::default();
        for s in order {
            rq.push_or_merge(entry(s));
        }
        assert_eq!(rq.peek().unwrap().station_id, "alpha");
        assert_eq!(rq.pop_worst().unwrap().station_id, "alpha");
    }
}

fn repair_delivery_app() -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<crate::lobby::LobbyOutbox>()
        .add_message::<DeliveredCoordination>()
        .add_message::<OrderedCoordinationPopup>()
        .add_systems(
            Update,
            (
                receive_repair_coordination,
                crate::ship_plugin::flush_coordination_popups,
            )
                .chain(),
        );
    let ship = app
        .world_mut()
        .spawn((
            crate::server_app::Ship,
            crate::server_app::LocalShip,
            crate::ship_plugin::ShipConfigComponent::default(),
            RepairRequestQueue::default(),
            RepairHumanAlerted::default(),
        ))
        .id();
    (app, ship)
}

fn deliver_repair_request(
    app: &mut App,
    ship: Entity,
    delivery: CoordinationDelivery,
    station_id: &str,
    tier: DamageTier,
    deficit: Option<f32>,
) {
    app.world_mut()
        .resource_mut::<Messages<DeliveredCoordination>>()
        .write(DeliveredCoordination {
            source_entity: ship,
            address: CoordinationAddress::Station(StationId("repair".into())),
            payload: CoordinationPayload::RepairRequest {
                system_id: SystemId(format!("{station_id}-system")),
                station_id: station_id.into(),
                station_label: station_id.into(),
                tier,
                deficit,
            },
            presentation: CoordinationPresentation::titled("coordination.repair.title")
                .with_title_param("label", station_id),
            delivery,
        });
}

/// Issue #1257: the delivered-message receiver, rather than the lag router,
/// owns the queue's existing same-station merge and severity inputs.
#[test]
fn ai_repair_deliveries_merge_in_the_repair_owned_queue() {
    let (mut app, ship) = repair_delivery_app();
    deliver_repair_request(
        &mut app,
        ship,
        CoordinationDelivery::Ai,
        "helm",
        DamageTier::Damaged,
        Some(18.0),
    );
    deliver_repair_request(
        &mut app,
        ship,
        CoordinationDelivery::Ai,
        "helm",
        DamageTier::Disabled,
        Some(12.0),
    );

    app.update();

    let queue = app
        .world()
        .get::<RepairRequestQueue>(ship)
        .expect("fixture carries Repair's queue");
    assert_eq!(queue.len(), 1, "same-station requests remain deduplicated");
    let entry = queue.peek().expect("merged request");
    assert_eq!(entry.station_id, "helm");
    assert_eq!(entry.tier, DamageTier::Disabled, "worst tier wins");
    assert_eq!(entry.deficit, 18.0, "largest exact deficit wins");
    assert!(
        app.world()
            .resource::<crate::lobby::LobbyOutbox>()
            .0
            .is_empty(),
        "AI consumption never also raises a popup"
    );
}

/// The intentionally asymmetric legacy rule is load-bearing: the first
/// sub-Disabled report is shown once, while Disabled and Destroyed reports are
/// always shown even when the same tier repeats. Moving the latch behind the
/// delivered-message seam must not quietly turn that into worsening-only.
#[test]
fn human_repair_deliveries_preserve_first_damage_and_urgent_repeat_policy() {
    let (mut app, ship) = repair_delivery_app();
    let popup = || CoordinationDelivery::HumanPopup {
        token: "engineer".into(),
        sender_label: "station.helm.name".into(),
        order: 0,
    };
    for tier in [
        DamageTier::Damaged,
        DamageTier::Damaged,
        DamageTier::Disabled,
        DamageTier::Disabled,
        DamageTier::Destroyed,
    ] {
        deliver_repair_request(&mut app, ship, popup(), "helm", tier, None);
    }

    app.update();

    let outbox = &app.world().resource::<crate::lobby::LobbyOutbox>().0;
    assert_eq!(
        outbox.len(),
        4,
        "first Damaged + both Disabled deliveries + Destroyed; only repeated Damaged is suppressed"
    );
    let expected_presentation = CoordinationPresentation::titled("coordination.repair.title")
        .with_title_param("label", "helm");
    assert!(outbox.iter().all(|(target, message)| {
        matches!(target, crate::lobby::handler::Target::Token(token) if token == "engineer")
            && matches!(
                message,
                ServerMessage::CoordinationPopup {
                    payload: CoordinationPayload::RepairRequest { deficit: None, .. },
                    presentation,
                    sender_label,
                    ..
                } if sender_label == "station.helm.name"
                    && presentation == &expected_presentation
            )
    }));
    assert!(
        app.world()
            .get::<RepairRequestQueue>(ship)
            .expect("fixture carries Repair's queue")
            .is_empty(),
        "human presentation never mutates the AI queue"
    );
}

/// `seed_repair_facts` exposes every reading an authored guard can name.
#[test]
fn seed_repair_facts_exposes_observable_damage_readings() {
    let facts = seed_repair_facts(&RepairCandidateReading {
        tier_ordinal: 2,
        deficit: 40.0,
        damage_fraction: 0.4,
        worst_system_damage_fraction: 0.6,
        system_count: 3,
        is_core: false,
        source_repair_request: true,
        assigned: false,
    });
    let near = |got: Option<f64>, want: f64| {
        assert!(
            got.is_some_and(|v| (v - want).abs() < 1e-6),
            "expected ~{want}, got {got:?}"
        );
    };
    assert_eq!(facts.get("tier_ordinal"), Some(2.0));
    assert_eq!(facts.get("deficit"), Some(40.0));
    near(facts.get("damage_fraction"), 0.4);
    near(facts.get("worst_system_damage_fraction"), 0.6);
    assert_eq!(facts.get("system_count"), Some(3.0));
    assert_eq!(facts.get("is_core"), Some(0.0));
    assert_eq!(facts.get("source_core_bucket"), Some(0.0));
    assert_eq!(facts.get("source_repair_request"), Some(1.0));
    assert_eq!(facts.get("assigned"), Some(0.0));

    let self_facts = seed_repair_self_facts(2, 0.75, Some(3.0), true);
    assert_eq!(self_facts.get("free_team_count"), Some(2.0));
    near(self_facts.get("total_hull_health_fraction"), 0.75);
    assert_eq!(self_facts.get("power_rating"), Some(3.0));
    assert_eq!(self_facts.get("red_alert"), Some(1.0));
}

// ── Destroyed systems are repair work now (issue #1013) ──────────────────

/// The PRODUCTION prune, not the copy in
/// `queue_entry_retained_when_all_systems_destroyed`: `operate_repair_ai`'s
/// `rq.entries.retain` must keep an all-Destroyed station's request, because
/// the on-site sweep can repair it. Before #1013 this entry was evicted on
/// the first AI tick and the station was stranded.
///
/// Retention alone is not the property that matters, so this also asserts
/// the free team is actually DISPATCHED to the destroyed station. A queue
/// entry the AUTHORED eligibility then refuses (the retired
/// `candidate_fact(tier_ordinal) < 3` clause) is a queue entry that survives
/// the prune and steers nothing — the team sits idle beside a station only
/// it can fix. That is the assertion the sweep's own tests could not make,
/// because they never run the selector.
#[test]
fn prune_retains_an_all_destroyed_station_through_the_ai_loop() {
    let mut app = npc_repair_app();
    // alpha flattened to 0 → Destroyed; bravo untouched.
    let npc = spawn_two_station_npc(
        &mut app,
        crate::ship::control_source::ControlSource::Ai,
        0.0,
        100.0,
        1,
        None,
    );
    assert_eq!(
        app.world()
            .get::<crate::entities::spawner::EntitySystemHull>(npc)
            .unwrap()
            .0
            .tier_for(&SystemId("alpha-sys".into())),
        DamageTier::Destroyed,
        "fixture precondition: alpha must be Destroyed"
    );
    assert_eq!(
        app.world().get::<RepairRequestQueue>(npc).unwrap().len(),
        1,
        "fixture precondition: the destroyed station is queued"
    );

    app.update();

    let stations: Vec<String> = app
        .world()
        .get::<RepairRequestQueue>(npc)
        .unwrap()
        .entries
        .iter()
        .map(|e| e.station_id.clone())
        .collect();
    assert_eq!(
        stations,
        vec!["alpha".to_string()],
        "an all-Destroyed station's request must survive the prune"
    );
    assert_eq!(
        team_systems(&app, npc)[0].as_deref(),
        Some("alpha-sys"),
        "the free team must actually be dispatched to the destroyed station — \
         a retained request the authored eligibility refuses would leave the \
         team idle and the station stranded exactly as before #1013"
    );
}

/// `push_or_merge` no longer drops a Destroyed-tier request on the floor,
/// and a station already queued at a lighter tier takes the UPGRADE instead
/// of the whole call bailing out.
#[test]
fn destroyed_tier_requests_are_queued_and_merged() {
    let mut rq = RepairRequestQueue::default();
    rq.push_or_merge(RepairQueueEntry {
        station_id: "alpha".into(),
        station_label: "Alpha".into(),
        tier: DamageTier::Destroyed,
        deficit: 100.0,
    });
    assert_eq!(rq.len(), 1, "a Destroyed request must be queued");

    let mut rq = RepairRequestQueue::default();
    rq.push_or_merge(RepairQueueEntry {
        station_id: "alpha".into(),
        station_label: "Alpha".into(),
        tier: DamageTier::Disabled,
        deficit: 40.0,
    });
    rq.push_or_merge(RepairQueueEntry {
        station_id: "alpha".into(),
        station_label: "Alpha".into(),
        tier: DamageTier::Destroyed,
        deficit: 100.0,
    });
    assert_eq!(rq.len(), 1, "same station, so still one entry");
    assert_eq!(rq.peek().unwrap().tier, DamageTier::Destroyed);
    assert_eq!(rq.peek().unwrap().deficit, 100.0);
}

/// End-to-end through the human dispatch path: a station whose only system
/// is Destroyed accepts a team, which arrives, repairs it, and un-latches
/// the tier. Before #1013 `resolve_repair_target` skipped the 0-HP system
/// and the team bounced off it on arrival.
#[test]
fn destroyed_station_is_dispatched_to_and_repaired_end_to_end() {
    let mut app = test_app();
    start_game(&mut app);

    let destroyed = SystemId("helm-engine-port".into());
    {
        let mut query = app.world_mut().query_filtered::<
            &mut crate::entities::spawner::EntitySystemHull,
            With<crate::server_app::LocalShip>,
        >();
        let mut hull = query
            .single_mut(app.world_mut())
            .expect("test fixture must contain one LocalShip hull");
        hull.0.set_hp(&destroyed, 0.0);
        assert_eq!(hull.0.tier_for(&destroyed), DamageTier::Destroyed);
    }

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: SystemId(REPAIR_SYSTEM_ID.into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );
    tick(&mut app);

    {
        let teams = local_teams(&mut app);
        let TeamSlot::Travelling { system_id, .. } = &teams.0.slots()[0] else {
            panic!(
                "team 0 must be sent to the Destroyed system, got {:?}",
                teams.0.slots()[0]
            );
        };
        assert_eq!(
            system_id.as_ref(),
            Some(&destroyed),
            "the worst system on the station is the Destroyed one"
        );
    }

    // 5 s travel at 0.2 s per update, then a few ticks of repair.
    for _ in 0..30 {
        tick(&mut app);
    }

    let mut query = app.world_mut().query_filtered::<
        &crate::entities::spawner::EntitySystemHull,
        With<crate::server_app::LocalShip>,
    >();
    let hull = query
        .single(app.world())
        .expect("test fixture must contain one LocalShip hull");
    assert!(
        hull.0.current_for(&destroyed).unwrap() > 0.0,
        "the arrived team must restore HP to the Destroyed system"
    );
    assert_ne!(
        hull.0.tier_for(&destroyed),
        DamageTier::Destroyed,
        "any positive HP un-latches the Destroyed tier"
    );
}

/// The sweep through the real Bevy tick: `tick_repair_teams` hands its
/// ship's own `ShipConfigComponent` to `RepairTeams::tick`, so a team that
/// finishes one system moves to the next one its station needs without
/// going `Returning` in between.
#[test]
fn tick_repair_teams_sweeps_the_station_using_the_ship_config() {
    let mut app = test_app();
    start_game(&mut app);

    // `helm-engine-port` and `helm-engine-starboard` are BOTH owned by the
    // `helm` station in the shipped battleship config
    // `ShipConfigComponent::default()` loads, so they share a sweep group.
    // The fixture REPLACES the hull with exactly those two rows: the shipped
    // battleship authors 13 `[[hull.system_hull]]` rows and none of them is
    // named `helm` (a station name is not a hull row), so nothing in this
    // fixture touches the ownerless bucket at all.
    let first = SystemId("helm-engine-port".into());
    let second = SystemId("helm-engine-starboard".into());
    {
        let local_ship = {
            let mut query = app
                .world_mut()
                .query_filtered::<Entity, With<crate::server_app::LocalShip>>();
            query.single(app.world()).expect("one LocalShip")
        };
        // Rebuild the hull so it carries both helm engines.
        app.world_mut()
            .entity_mut(local_ship)
            .insert(crate::entities::spawner::EntitySystemHull(
                SystemHull::from_config(&[(first.clone(), 10.0), (second.clone(), 10.0)]),
            ));
        let mut hull = app
            .world_mut()
            .get_mut::<crate::entities::spawner::EntitySystemHull>(local_ship)
            .unwrap();
        hull.0.set_hp(&first, 1.0);
        hull.0.set_hp(&second, 0.0);
    }

    push(
        &mut app,
        "eng",
        ClientMessage::ControlSystem {
            target: SystemId(REPAIR_SYSTEM_ID.into()),
            payload: SystemControlPayload::DispatchRepairTeam {
                team_idx: 0,
                target: RepairTarget::Station(StationId("helm".into())),
            },
        },
    );

    // Walk the whole visit, recording every system the team works on and
    // whether it ever heads home mid-way.
    let mut visited: Vec<String> = vec![];
    let mut returned = false;
    for _ in 0..400 {
        tick(&mut app);
        match &local_teams(&mut app).0.slots()[0] {
            TeamSlot::Repairing {
                system_id: Some(s), ..
            } if visited.last() != Some(&s.0) => {
                visited.push(s.0.clone());
            }
            TeamSlot::Returning { .. } => {
                returned = true;
                break;
            }
            _ => {}
        }
    }

    assert_eq!(
        visited,
        vec![
            "helm-engine-starboard".to_string(),
            "helm-engine-port".to_string()
        ],
        "the team must sweep both helm engines in one visit, worst first"
    );
    assert!(returned, "and only then head home");
}
