#[test]
fn slot_claim_sequence_is_owned_by_the_world_and_resets_for_a_new_fleet() {
    let mut world = bevy::prelude::World::new();
    world.init_resource::<super::SlotClaimSequence>();
    assert_eq!(
        world
            .resource_mut::<super::SlotClaimSequence>()
            .next_claim(),
        1
    );
    assert_eq!(
        world
            .resource_mut::<super::SlotClaimSequence>()
            .next_claim(),
        2
    );
    world.resource_mut::<super::SlotClaimSequence>().reset();
    assert_eq!(
        world
            .resource_mut::<super::SlotClaimSequence>()
            .next_claim(),
        1
    );
    let mut other = bevy::prelude::World::new();
    other.init_resource::<super::SlotClaimSequence>();
    assert_eq!(
        other
            .resource_mut::<super::SlotClaimSequence>()
            .next_claim(),
        1
    );
}
use super::*;
use crate::core::messages::{StationId, SystemControlPayload, SystemId};

fn roster() -> FleetRoster {
    FleetRoster::new(
        vec![
            FleetShip {
                host: HostSlot(2),
                ship_path: Some("b.toml".into()),
                authored_slot_id: None,
                crew: vec![(StationId("helm".into()), "Std".into())],
            },
            FleetShip {
                host: HostSlot(1),
                ship_path: Some("a.toml".into()),
                authored_slot_id: None,
                crew: vec![],
            },
        ],
        HostSlot(2),
    )
}

/// The adoption boundary validates the frozen Helm rating against selected
/// content. Use a world-local selected hull, avoiding the process-global
/// native template cache in these unit fixtures.
fn selected_hull_roster(world: &mut World) -> FleetRoster {
    let hull = crate::ship::config::ShipConfig::from_toml(
        r#"
[[station]]
id = "helm"
name = "Helm"
description = ""
rank = ""
[[station.rating]]
name = "Std"
automated_systems = []
[[system]]
id = "helm"
kind = "helm_thrust"
station = "helm"
"#,
        &["helm_thrust"],
    )
    .expect("the selected hull supports the frozen Helm rating");
    world.insert_resource(crate::ship_plugin::PendingShipConfig(hull));
    let mut roster = roster();
    roster
        .ships
        .iter_mut()
        .find(|ship| ship.host == HostSlot(2))
        .unwrap()
        .ship_path = None;
    assert!(crew::roster_crew_matches_hulls(world, &roster));
    roster
}

/// The roster walks in slot order whatever order it was handed in, because
/// that order decides which authored spawn each ship takes — and therefore
/// what the mint gives it.
#[test]
fn a_roster_is_ordered_by_slot_not_by_arrival() {
    let roster = roster();
    let slots: Vec<HostSlot> = roster.ships().iter().map(|s| s.host).collect();
    assert_eq!(slots, vec![HostSlot(1), HostSlot(2)]);
    assert_eq!(roster.ship(0).unwrap().ship_path.as_deref(), Some("a.toml"));
    assert!(roster.is_local(HostSlot(2)));
    assert!(!roster.is_local(HostSlot(1)));
    assert!(!roster.is_solo());
}

/// The default roster is the shipped single-player case, spelled out rather
/// than left implicit: one ship, this host's, flying the lobby's choice.
#[test]
fn the_default_roster_is_a_fleet_of_one() {
    let roster = FleetRoster::default();
    assert!(roster.is_solo());
    assert_eq!(roster.len(), 1);
    assert!(roster.is_local(HostSlot::SOLO));
    assert_eq!(roster.ship(0).unwrap().ship_path, None);
}

#[test]
fn private_gm_bindings_are_bounded_unique_and_may_share_a_ship_peer() {
    let roster = FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), HostSlot(2)],
        vec![FleetGm {
            host: HostSlot(2),
            operator_id: "gm-1".into(),
        }],
        HostSlot(2),
        HostSlot(1),
    )
    .expect("GM-only participants are valid");
    assert_eq!(roster.gm_operator(HostSlot(2)), Some("gm-1"));
    assert!(roster.ships().is_empty());

    let ship_and_gm_same_slot = FleetRoster::with_participants_and_gms(
        vec![FleetShip::new(HostSlot(2))],
        vec![HostSlot(1), HostSlot(2)],
        vec![FleetGm {
            host: HostSlot(2),
            operator_id: "gm-1".into(),
        }],
        HostSlot(2),
        HostSlot(1),
    );
    let combined =
        ship_and_gm_same_slot.expect("one technical peer may advertise both capabilities");
    assert_eq!(combined.participants(), &[HostSlot(1), HostSlot(2)]);
    assert_eq!(combined.ships().len(), 1);
    assert_eq!(combined.gm_operator(HostSlot(2)), Some("gm-1"));

    let duplicate_operator = FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), HostSlot(2)],
        vec![
            FleetGm {
                host: HostSlot(1),
                operator_id: "gm-1".into(),
            },
            FleetGm {
                host: HostSlot(2),
                operator_id: "gm-1".into(),
            },
        ],
        HostSlot(2),
        HostSlot(1),
    );
    assert!(duplicate_operator.is_none());
}

/// The frozen crewing is per slot, so a host can answer "is slot 2's Helm
/// crewed, and at what rating?" without a session for slot 2's crew — which
/// it never has, and never will.
#[test]
fn crewing_is_answerable_for_a_ship_this_host_has_no_sessions_for() {
    let roster = roster();
    assert_eq!(
        roster.crew_of(HostSlot(2)),
        &[(StationId("helm".into()), "Std".to_string())]
    );
    assert_eq!(
            roster.ship(1).unwrap().rating_at(&StationId("helm".into())),
            Some("Std"),
            "the RATING travels with the seat: it decides which systems the              station automates, so two hosts holding different ones would run              different AI on the same ship"
        );
    assert!(roster.crew_of(HostSlot(1)).is_empty());
    assert!(roster.crew_of(HostSlot(9)).is_empty());
}

/// A peer-local save preserves the ships it booted, but it is not a ticket
/// back into the old mesh. Starting it must release the old wait set and
/// crew assignments so the new App can advance by itself.
#[test]
fn a_saved_fleet_starts_as_an_uncrewed_standalone_session() {
    use crate::command_admission::log::PendingCommands;

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    app.init_resource::<PendingCommands>();
    register_lockstep(&mut app);
    let mut config = crate::world::config::WorldConfig::default();
    config.global.seed = Some(7);
    app.insert_resource(config);
    let saved = selected_hull_roster(app.world_mut());
    assert!(join_fleet(app.world_mut(), saved.clone(), 6));
    assert!(app.world().contains_resource::<FleetLockstep>());
    assert_eq!(app.world().resource::<CommandDelay>().0, 6);

    start_saved_fleet_standalone(app.world_mut(), saved);

    assert!(!app.world().contains_resource::<FleetLockstep>());
    assert_eq!(app.world().resource::<CommandDelay>().0, 0);
    let restored = app.world().resource::<FleetRoster>();
    assert_eq!(restored.len(), 2);
    assert!(restored.is_local(HostSlot(2)));
    assert!(restored.ships().iter().all(|ship| ship.crew.is_empty()));
}

#[test]
fn fresh_lobby_leave_clears_the_wait_set_and_can_reopen() {
    use crate::command_admission::log::PendingCommands;

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    app.init_resource::<PendingCommands>();
    app.init_resource::<crate::lobby::server::FleetManagedLobby>();
    app.init_resource::<crate::lobby::server::PendingStartGrants>();
    app.init_resource::<crate::lobby::server::StartGrantTracker>();
    app.init_resource::<crate::lobby::server::StartGrantResults>();
    register_lockstep(&mut app);
    let mut config = crate::world::config::WorldConfig::default();
    config.global.seed = Some(11);
    app.insert_resource(config);
    let frozen = selected_hull_roster(app.world_mut());
    assert!(join_fleet(app.world_mut(), frozen.clone(), 6));
    app.world_mut()
        .resource_mut::<Time<bevy::time::Virtual>>()
        .pause();
    app.world_mut()
        .resource_mut::<crate::lobby::server::FleetManagedLobby>()
        .set_enabled(true);

    assert_eq!(leave_fleet(app.world_mut()), Ok(()));
    assert!(!app.world().contains_resource::<FleetLockstep>());
    assert!(app.world().resource::<FleetRoster>().is_solo());
    assert_eq!(app.world().resource::<CommandDelay>().0, 0);
    assert!(
        !app.world()
            .resource::<crate::lobby::server::FleetManagedLobby>()
            .enabled
    );
    assert!(!app
        .world()
        .resource::<Time<bevy::time::Virtual>>()
        .is_paused());

    assert!(
        join_fleet(app.world_mut(), frozen, 6),
        "the new generation must not inherit the old wait-set identity"
    );
    assert!(app.world().contains_resource::<FleetLockstep>());
}

#[test]
fn unequal_gm_only_bootstraps_adopt_one_tick_clock_rng_and_mint_epoch() {
    use crate::command_admission::log::PendingCommands;
    use crate::sim_rng::{SeedSource, SimRng};
    use crate::world_id::{IdNamespace, WorldIdMint};

    let period = std::time::Duration::from_millis(20);
    let mut apps = Vec::new();
    let mut immediate_ids = Vec::new();
    for (local, bootstrap_tick, bootstrap_rng) in [(HostSlot(1), 17, 101), (HostSlot(2), 93, 202)] {
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        app.init_resource::<PendingCommands>();
        register_lockstep(&mut app);
        let mut config = crate::world::config::WorldConfig::default();
        config.global.seed = Some(77);
        app.insert_resource(config);
        app.insert_resource(crate::sim_tick::SimTick(bootstrap_tick));
        app.insert_sim_rng(SimRng::new(bootstrap_rng, SeedSource::World));
        app.insert_world_id_mint(WorldIdMint::default());
        let immediate = app
            .world()
            .resource::<WorldIdMint>()
            .mint(IdNamespace::Entity);
        immediate_ids.push(immediate);
        let mut fixed = Time::<Fixed>::from_duration(period);
        fixed.advance_to(period * u32::try_from(bootstrap_tick).unwrap());
        app.insert_resource(fixed);

        let roster = FleetRoster::with_participants(
            Vec::new(),
            vec![HostSlot(1), HostSlot(2)],
            local,
            HostSlot(1),
        )
        .unwrap();
        assert!(join_fleet(app.world_mut(), roster, 6));
        apps.push(app);
    }

    for (index, app) in apps.iter_mut().enumerate() {
        assert_eq!(
            app.world().resource::<crate::sim_tick::SimTick>().0,
            FLEET_ACTIVATION_TICK
        );
        let fixed = app.world().resource::<Time<Fixed>>();
        assert_eq!(fixed.timestep(), period);
        assert_eq!(fixed.elapsed(), period);
        assert_eq!(fixed.overstep(), std::time::Duration::ZERO);
        let mint = app.world().resource::<WorldIdMint>();
        assert_eq!(mint.tick(), FLEET_ACTIVATION_TICK);
        let game_start = mint.mint(IdNamespace::Entity);
        assert_ne!(
            game_start, immediate_ids[index],
            "the tick-1 fleet epoch must not reuse a live immediate tick-0 id"
        );
        assert_eq!(
            app.world()
                .resource::<FleetLockstep>()
                .watermark_of(if index == 0 { HostSlot(2) } else { HostSlot(1) }),
            Some(FLEET_ACTIVATION_TICK + 6),
            "the first wait-set frontier is based at the shared activation epoch"
        );
    }
    assert_eq!(
        apps[0].world().resource::<SimRng>().state(),
        apps[1].world().resource::<SimRng>().state(),
        "browser-equivalent hosts discard their different entropy and use the authored seed"
    );
    assert_eq!(
        apps[0].world().resource::<WorldIdMint>().state(),
        apps[1].world().resource::<WorldIdMint>().state(),
        "the same post-activation mint state produces identical GameStart ids"
    );

    use bevy::ecs::system::RunSystemOnce;
    fn first_live_draw(
        rng: crate::sim_rng::LiveStream<{ crate::sim_rng::SimStream::BeamCycleJitter as usize }>,
    ) -> u32 {
        crate::sim_rng::with_live_stream(rng.as_deref(), |stream| stream.next_u32())
    }
    for app in &mut apps {
        let reference = SimRng::new(77, SeedSource::World);
        assert_eq!(
            app.world_mut().run_system_once(first_live_draw).unwrap(),
            reference
                .stream(crate::sim_rng::SimStream::BeamCycleJitter)
                .next_u32(),
            "first live draw must use the adopted authored seed, not bootstrap handles"
        );
        let continued = app.world().resource::<SimRng>().state();
        let same_roster = app.world().resource::<FleetRoster>().clone();
        assert!(join_fleet(app.world_mut(), same_roster, 6));
        assert_eq!(
            app.world().resource::<SimRng>().state(),
            continued,
            "same-roster adoption must not reset any stream"
        );
        assert_eq!(
            app.world_mut().run_system_once(first_live_draw).unwrap(),
            reference
                .stream(crate::sim_rng::SimStream::BeamCycleJitter)
                .next_u32(),
            "repeated adoption continues the existing live cell"
        );
        assert_eq!(app.world().resource::<SimRng>().state(), reference.state());
    }
}

#[test]
fn fleet_leave_refuses_at_the_start_boundary_without_clearing_live_state() {
    use crate::command_admission::log::PendingCommands;

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    app.init_resource::<PendingCommands>();
    register_lockstep(&mut app);
    let mut config = crate::world::config::WorldConfig::default();
    config.global.seed = Some(13);
    app.insert_resource(config);
    let frozen = selected_hull_roster(app.world_mut());
    assert!(join_fleet(app.world_mut(), frozen, 6));
    let retained = MeshFrame::Digest(DigestFrame {
        from: HostSlot(1),
        tick: 1,
        digest: 13,
    });
    app.world_mut()
        .resource_mut::<MeshInbox>()
        .push(retained.clone());
    app.world_mut().resource_mut::<MeshOutbox>().push(retained);
    app.insert_resource(NextState::Pending(
        crate::core::messages::GamePhase::InProgress,
    ));

    assert_eq!(
        leave_fleet(app.world_mut()),
        Err(FleetLeaveError::NotFreshLobby)
    );
    assert!(app.world().contains_resource::<FleetLockstep>());
    assert_eq!(app.world().resource::<CommandDelay>().0, 6);
    assert_eq!(app.world().resource::<MeshInbox>().len(), 1);
    assert_eq!(
        app.world().resource::<MeshOutbox>().pending_frames().len(),
        1
    );

    app.insert_resource(NextState::<crate::core::messages::GamePhase>::Unchanged);
    app.insert_resource(crate::server_app::GameStartEntityUuids::default());

    assert_eq!(
        leave_fleet(app.world_mut()),
        Err(FleetLeaveError::NotFreshLobby)
    );
    assert!(app.world().contains_resource::<FleetLockstep>());
    assert_eq!(app.world().resource::<MeshInbox>().len(), 1);
    assert_eq!(
        app.world().resource::<MeshOutbox>().pending_frames().len(),
        1
    );
}

#[test]
fn only_a_standalone_rosters_local_ship_uses_live_sessions() {
    let restored = roster().into_uncrewed();

    assert!(uses_live_sessions(&restored, false, HostSlot(2)));
    assert!(
        !uses_live_sessions(&restored, false, HostSlot(1)),
        "a saved remote ship has no crew in this independent App"
    );
    assert!(
        !uses_live_sessions(&restored, true, HostSlot(2)),
        "active lockstep keeps even the local ship on frozen roster crew"
    );
}

/// A disagreement renders both digests and names the peer, because "the
/// fleet diverged" without a tick and a slot is not actionable.
#[test]
fn a_disagreement_names_the_tick_the_peer_and_both_digests() {
    let found = MeshDisagreement {
        tick: 240,
        peer: HostSlot(2),
        local_digest: 1,
        peer_digest: 2,
    };
    let text = found.to_string();
    assert!(text.contains("240"), "{text}");
    assert!(text.contains("slot-2"), "{text}");
}

#[test]
fn local_sampling_compares_a_peer_checkpoint_received_earlier_once() {
    let mut world = World::new();
    world.insert_resource(FleetLockstep(LockstepSession::new(
        HostSlot(1),
        [HostSlot(2)],
        6,
    )));
    world.insert_resource(crate::sim_tick::SimTick(300));
    world.insert_resource(MeshAgreement::new(300));
    world.init_resource::<MeshOutbox>();
    let expected = crate::sim_digest::world_digest(&world);
    let mut remote = crate::sim_digest::DigestLedger::new(300);
    remote.record(300, expected ^ 1);
    world
        .resource_mut::<MeshAgreement>()
        .peers
        .insert(HostSlot(2), remote);
    assert!(world.resource::<MeshAgreement>().agreed());
    sample_and_publish_digest(&mut world);
    let found = world
        .resource::<MeshAgreement>()
        .first_disagreement()
        .expect("compare the early peer when local sampling catches up");
    assert_eq!(
        found,
        MeshDisagreement {
            tick: 300,
            peer: HostSlot(2),
            local_digest: expected,
            peer_digest: expected ^ 1
        }
    );
    sample_and_publish_digest(&mut world);
    assert_eq!(world.resource::<MeshAgreement>().disagreements.len(), 1);
    assert_eq!(
        world.resource_mut::<MeshOutbox>().drain().len(),
        1,
        "duplicate sampling cannot republish"
    );
}

#[test]
fn checkpoint_comparison_preserves_discovery_order_and_recovery_cleanup() {
    let mut agreement = MeshAgreement::new(300);
    let peer = HostSlot(2);
    assert!(agreement.compare_sample(peer, 600, 2).is_none());
    agreement.local.record(300, 1);
    agreement.local.record(600, 1);
    assert!(agreement.compare_sample(peer, 600, 1).is_none());
    assert!(agreement.compare_sample(peer, 600, 2).is_some());
    assert!(agreement.compare_sample(peer, 600, 2).is_none());
    assert!(agreement.compare_sample(peer, 300, 2).is_some());
    assert_eq!(agreement.first_disagreement().unwrap().tick, 600);
    agreement.forget_through(300);
    assert_eq!(agreement.disagreements.len(), 1);
    assert!(agreement.compare_sample(peer, 300, 2).is_none());
    agreement.local.record(900, 3);
    assert!(agreement.compare_sample(peer, 900, 3).is_none());
}

/// Canonical marker geometry is an authority prerequisite even when no
/// fleet session exists. A rendererless GM therefore uses the same virtual
/// clock hold as a rendered host, and releases it as soon as its live
/// primary rig resolves.
#[test]
fn model_rig_hold_pauses_and_resumes_a_solo_clock() {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin)
        .init_resource::<crate::entities::model_markers::ModelRigReadiness>()
        .init_resource::<MeshDiagnostics>()
        .add_systems(PreUpdate, gate_lockstep_ticks);

    app.world_mut()
        .resource_mut::<crate::entities::model_markers::ModelRigReadiness>()
        .set_live_blocked_for_test(true);
    app.update();
    assert!(
        app.world().resource::<Time<Virtual>>().is_paused(),
        "a solo authoritative profile must not tick without live marker geometry"
    );

    app.world_mut()
        .resource_mut::<crate::entities::model_markers::ModelRigReadiness>()
        .set_live_blocked_for_test(false);
    app.update();
    assert!(
        !app.world().resource::<Time<Virtual>>().is_paused(),
        "the narrow marker hold releases immediately once geometry resolves"
    );
}

/// The diagnostics count a run of withheld frames, not just the fact of
/// one: a fleet that stalls for a frame is normal jitter and a fleet that
/// stalls for a thousand is broken, and the resource has to tell them apart.
#[test]
fn the_diagnostics_measure_the_longest_stall_not_just_the_last() {
    let mut diagnostics = MeshDiagnostics::default();
    let stall = |tick| Stall {
        tick,
        waiting_on: vec![(HostSlot(2), 0)],
    };
    diagnostics.stalled(stall(1));
    diagnostics.running();
    for tick in 0..3 {
        diagnostics.stalled(stall(tick));
    }
    assert_eq!(diagnostics.stalled_frames, 4);
    assert_eq!(diagnostics.longest_stall, 3);
    assert!(diagnostics.is_stalled());
    diagnostics.running();
    assert!(!diagnostics.is_stalled());
    assert_eq!(diagnostics.longest_stall, 3, "the peak is remembered");
}

/// The projection an accepted command crosses the wire as keeps everything
/// a peer needs and nothing it must not have.
#[test]
fn a_mesh_command_carries_no_session_token() {
    let admitted = crate::core::messages::AdmittedCommand {
        target: SystemId("helm".into()),
        payload: SystemControlPayload::SetRedAlert { active: true },
        response_token: Some("session-token-aaaa".into()),
        feedback_correlation: None,
    };
    let crossed = mesh_command(
        9,
        CommandOrder::new(HostSlot(1), 3),
        ShipKey("uuid-ship".into()),
        &admitted,
    );
    assert_eq!(crossed.tick, 9);
    assert_eq!(crossed.ship, ShipKey("uuid-ship".into()));
    assert!(
        !format!("{crossed:?}").contains("session-token"),
        "the token is a bearer credential and the wire is exactly where it \
             must not go"
    );
}

/// A participant mesh commits at most one fixed step before transport
/// egress, while a solo simulation retains Bevy's ordinary catch-up loop.
///
/// The oversized frame is the exact race a scheduled start exposed: without
/// the FixedLast overstep cap, its first step could seal the owner's grant
/// and four later steps could reach `apply_tick` before PostUpdate had any
/// chance to send the bearing TickFrame. The fractional remainder is kept
/// for render interpolation; only whole unstarted steps are discarded.
#[test]
fn a_multi_participant_frame_commits_one_step_before_egress() {
    use crate::sim_tick::{register_sim_tick, SimTick};

    let period = std::time::Duration::from_millis(10);

    let mut fleet = App::new();
    fleet.add_plugins(bevy::time::TimePlugin);
    register_sim_tick(&mut fleet);
    fleet.init_resource::<crate::command_admission::log::PendingCommands>();
    register_lockstep(&mut fleet);
    fleet.insert_resource(FleetLockstep(
        LockstepSession::new_at(
            HostSlot(1),
            vec![HostSlot(1), HostSlot(2)],
            6,
            FLEET_ACTIVATION_TICK,
        )
        .unwrap(),
    ));
    fleet
        .world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);
    // Prime Bevy's first frame, which intentionally carries zero delta.
    fleet.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::ZERO,
    ));
    fleet.update();
    fleet.world_mut().resource_mut::<MeshOutbox>().drain();

    fleet.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        period * 5 + period / 2,
    ));
    fleet.update();
    assert_eq!(fleet.world().resource::<SimTick>().0, 1);
    assert_eq!(
        fleet.world().resource::<Time<Fixed>>().overstep(),
        period / 2,
        "fractional interpolation remainder survives the whole-step cap"
    );
    let ticks: Vec<_> = fleet
        .world()
        .resource::<MeshOutbox>()
        .pending_frames()
        .iter()
        .filter_map(|frame| match frame {
            MeshFrame::Tick(frame) => Some(frame.tick),
            _ => None,
        })
        .collect();
    assert_eq!(ticks, vec![0], "one committed step seals one bearing frame");

    let mut solo = App::new();
    solo.add_plugins(bevy::time::TimePlugin);
    register_sim_tick(&mut solo);
    solo.init_resource::<crate::command_admission::log::PendingCommands>();
    register_lockstep(&mut solo);
    solo.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);
    solo.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::ZERO,
    ));
    solo.update();
    solo.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 5));
    solo.update();
    assert_eq!(
        solo.world().resource::<SimTick>().0,
        5,
        "a standalone simulation retains ordinary fixed-step catch-up"
    );
}

#[test]
fn a_solo_gm_pause_boundary_cannot_be_skipped_by_fixed_catch_up() {
    use crate::sim_tick::{register_sim_tick, SimTick};

    let period = std::time::Duration::from_millis(10);
    let local = HostSlot(1);
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    register_sim_tick(&mut app);
    app.init_resource::<crate::command_admission::log::PendingCommands>();
    register_lockstep(&mut app);
    app.insert_resource(FleetLockstep(LockstepSession::new(local, [local], 6)));
    app.world_mut()
        .resource_mut::<crate::gm_action::GmActionJournal>()
        .insert(crate::gm_action::GmActionGrant {
            from: local,
            sequenced_by: local,
            operator_id: "solo-gm".into(),
            correlation: crate::gm_action::GmActionId::new("catch-up-pause").unwrap(),
            recovery_generation: 0,
            apply_tick: 1,
            order: crate::gm_action::GmActionOrder::new(local, 1),
            action: crate::gm_action::GmAction::SetSessionPaused { active: true },
        })
        .unwrap();
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::ZERO,
    ));
    app.update();

    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 5));
    app.update();
    assert_eq!(
        app.world().resource::<SimTick>().0,
        1,
        "the oversized frame stops on the unapplied GM boundary"
    );
    assert_eq!(
        app.world()
            .resource::<crate::gm_action::GmActionJournal>()
            .applied_grants(),
        0,
        "the boundary is not misreported as applied before its next PreUpdate"
    );

    app.update();
    assert_eq!(app.world().resource::<SimTick>().0, 1);
    assert!(
        app.world()
            .resource::<crate::gm_action::SimulationPaused>()
            .0
    );
    assert_eq!(
        app.world()
            .resource::<crate::gm_action::GmActionJournal>()
            .applied_grants(),
        1
    );
    assert_eq!(
        app.world()
            .resource::<crate::gm_action::GmActionLog>()
            .entries()[0]
            .outcome,
        crate::gm_action::GmActionOutcome::Applied
    );
}

#[test]
fn a_new_peer_stall_withholds_the_current_frame_before_fixed_work() {
    use crate::sim_tick::{register_sim_tick, SimTick};

    let period = std::time::Duration::from_millis(10);
    let local = HostSlot(1);
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    register_sim_tick(&mut app);
    app.init_resource::<crate::command_admission::log::PendingCommands>();
    register_lockstep(&mut app);
    app.insert_resource(FleetLockstep(LockstepSession::new(
        local,
        [local, HostSlot(2)],
        6,
    )));
    app.world_mut()
        .resource_mut::<Time<Fixed>>()
        .set_timestep(period);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::ZERO,
    ));
    app.update();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    for _ in 0..6 {
        app.update();
    }
    assert_eq!(app.world().resource::<SimTick>().0, 6);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        period * 3 / 2,
    ));
    app.update();
    assert_eq!(app.world().resource::<SimTick>().0, 7);
    app.world_mut().resource_mut::<MeshOutbox>().drain();

    // First has already calculated this delta when PreUpdate discovers
    // that tick 7 has no peer input. No FixedUpdate system may run yet.
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 5));
    app.update();
    assert_eq!(app.world().resource::<SimTick>().0, 7);
    assert!(app
        .world_mut()
        .resource_mut::<MeshOutbox>()
        .drain()
        .is_empty());
    assert_eq!(app.world().resource::<Time<Fixed>>().overstep(), period / 2);

    app.world_mut()
        .resource_mut::<FleetLockstep>()
        .0
        .observe(HostSlot(2), 100);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.update();
    app.update();
    assert_eq!(app.world().resource::<SimTick>().0, 8);
}

#[test]
fn applying_pause_consumes_the_current_frame_delta_standalone_and_in_a_fleet() {
    use crate::sim_tick::{register_sim_tick, SimTick};

    let period = std::time::Duration::from_millis(10);
    for fleet in [false, true] {
        let local = HostSlot(1);
        let mut app = App::new();
        app.add_plugins(bevy::time::TimePlugin);
        register_sim_tick(&mut app);
        app.init_resource::<crate::command_admission::log::PendingCommands>();
        register_lockstep(&mut app);
        if fleet {
            let mut session = LockstepSession::new(local, [local, HostSlot(2)], 6);
            session.observe(HostSlot(2), 100);
            app.insert_resource(FleetLockstep(session));
        }
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .set_timestep(period);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        app.update();

        app.world_mut()
            .resource_mut::<crate::gm_action::GmActionJournal>()
            .insert(crate::gm_action::GmActionGrant {
                from: local,
                sequenced_by: local,
                operator_id: "gm".into(),
                correlation: crate::gm_action::GmActionId::new(if fleet {
                    "fleet-now-pause"
                } else {
                    "standalone-now-pause"
                })
                .unwrap(),
                recovery_generation: 0,
                apply_tick: 0,
                order: crate::gm_action::GmActionOrder::new(local, 1),
                action: crate::gm_action::GmAction::SetSessionPaused { active: true },
            })
            .unwrap();
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period * 5));
        app.update();

        assert_eq!(
            app.world().resource::<SimTick>().0,
            0,
            "an apply-at-now Pause leaked fixed work (fleet={fleet})"
        );
        assert!(
            app.world()
                .resource::<crate::gm_action::SimulationPaused>()
                .0
        );
        assert_eq!(
            app.world()
                .resource::<crate::gm_action::GmActionJournal>()
                .applied_grants(),
            1
        );
    }
}
