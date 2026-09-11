//! A standalone game master acting on its own session.
//!
//! The landing's `host_gm` route opens a session with exactly one peer in it.
//! That peer had no bound operator, so every typed action it submitted was
//! refused before it could reach a reducer and the page — which gates each
//! control on the same identity — disabled the whole desk. These tests drive the
//! ORDINARY admission path, the one a fleet game master uses, and assert that a
//! fleetless peer now reaches `Applied` through it, that nothing else may, and
//! that an already-admitted fleet peer is left alone.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::{ecs::system::RunSystemOnce, prelude::*};
use phoenix::{
    command_admission::HostSlot,
    gm_action::*,
    gm_roster::{GmOperator, GmRoster},
    gm_solo::{bind_standalone_game_master, local_gm_operator, SOLO_GM_OPERATOR_ID},
    lockstep::{FleetGm, FleetRoster},
    sim_tick::SimTick,
};
use project_phoenix as phoenix;

/// A browser game master exactly as `wasm_init` leaves it before binding: the
/// solo roster `register_lockstep` installs in every App, an empty crew-public
/// GM roster, and a running session.
fn standalone_peer() -> World {
    let mut world = World::new();
    world.insert_resource(FleetRoster::default());
    world.init_resource::<GmRoster>();
    world.insert_resource(SimTick(10));
    world.insert_resource(SimulationPaused(false));
    world.init_resource::<GmActionJournal>();
    world.init_resource::<GmActionLog>();
    world.init_resource::<LocalGmActionRefusals>();
    world.init_resource::<LastGmSessionProjection>();
    world.init_resource::<phoenix::lockstep::MeshOutbox>();
    world.insert_resource(Time::<Virtual>::default());
    world.insert_resource(Time::<Fixed>::default());
    world
}

fn pause(correlation: &str, operator: &str) -> GmActionRequest {
    GmActionRequest {
        operator_id: operator.into(),
        correlation: GmActionId::new(correlation).unwrap(),
        action: GmAction::SetSessionPaused { active: true },
    }
}

#[test]
fn a_fleetless_game_master_is_refused_until_it_is_bound_and_then_its_action_applies() {
    let mut world = standalone_peer();

    // The reported defect, stated as a fact: the desk's own operator is not
    // admitted, so nothing it presses can do anything.
    assert!(local_gm_operator(&world).is_none());
    assert_eq!(
        submit_local(&mut world, pause("solo-pause", SOLO_GM_OPERATOR_ID)),
        Err(GmActionRefusalReason::NotGameMaster)
    );

    let bound = bind_standalone_game_master(&mut world).expect("a fleetless peer binds");
    assert_eq!(bound.id, SOLO_GM_OPERATOR_ID);
    assert_eq!(
        local_gm_operator(&world).map(|row| row.id),
        Some(SOLO_GM_OPERATOR_ID.to_string()),
        "the page reads back the identity the simulation bound"
    );

    let GmActionSubmission::Granted(grant) =
        submit_local(&mut world, pause("solo-pause", SOLO_GM_OPERATOR_ID))
            .expect("the ordinary admission path now binds this operator")
    else {
        panic!("a bound standalone operator's action must be sequenced");
    };
    assert_eq!(grant.operator_id, SOLO_GM_OPERATOR_ID);
    assert_eq!(grant.from, HostSlot::SOLO);

    world.run_system_once(apply_due_actions).unwrap();
    assert!(
        world.resource::<SimulationPaused>().0,
        "the session actually pauses — the defect was an ingested action that never applied"
    );
    let fact = &world.resource::<GmActionLog>().entries()[0];
    assert_eq!(fact.operator_id, SOLO_GM_OPERATOR_ID);
    assert_eq!(fact.outcome, GmActionOutcome::Applied);

    // Nothing about this made it a fleet. The grant still takes the ordinary
    // owner-sequenced egress — that is the point, it is the SAME path — and a
    // session of one simply has nobody to relay it to.
    assert!(matches!(
        world
            .resource_mut::<phoenix::lockstep::MeshOutbox>()
            .drain()
            .as_slice(),
        [phoenix::lockstep::MeshFrame::GmAction(
            GmActionFrame::Granted(_)
        )]
    ));
    assert!(world.resource::<FleetRoster>().is_solo());
    assert_eq!(world.resource::<FleetRoster>().len(), 1);
    assert!(!world.contains_resource::<phoenix::lockstep::FleetLockstep>());
}

#[test]
fn binding_one_operator_admits_only_that_operator() {
    let mut world = standalone_peer();
    bind_standalone_game_master(&mut world).expect("a fleetless peer binds");

    assert_eq!(
        submit_local(&mut world, pause("spoofed", "gm-someone-else")),
        Err(GmActionRefusalReason::OperatorMismatch),
        "an invented operator id is still checked against the frozen binding"
    );

    // Losing public presence is losing admission, exactly as it is for a fleet
    // game master: the roster row is the presence half of the same check.
    world.insert_resource(
        GmRoster::try_new(vec![GmOperator {
            id: SOLO_GM_OPERATOR_ID.into(),
            name: String::new(),
            connected: false,
            ready: false,
        }])
        .unwrap(),
    );
    assert!(local_gm_operator(&world).is_none());
    assert_eq!(
        submit_local(&mut world, pause("absent", SOLO_GM_OPERATOR_ID)),
        Err(GmActionRefusalReason::NotGameMaster)
    );
    assert!(!world.resource::<SimulationPaused>().0);
}

#[test]
fn an_admitted_fleet_game_master_is_never_rebound_and_keeps_its_own_identity() {
    let mut world = standalone_peer();
    let slot = HostSlot(2);
    let fleet = FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), slot],
        vec![FleetGm {
            host: slot,
            operator_id: "gm-2".into(),
        }],
        slot,
        HostSlot(1),
    )
    .expect("a stationless fleet GM participant");
    world.insert_resource(fleet.clone());
    world.insert_resource(
        GmRoster::try_new(vec![GmOperator::new("gm-2".into(), "Morgan".into(), true)]).unwrap(),
    );

    assert!(bind_standalone_game_master(&mut world).is_none());
    assert_eq!(world.resource::<FleetRoster>(), &fleet);
    assert_eq!(
        local_gm_operator(&world).map(|row| row.id),
        Some("gm-2".to_string()),
        "the fleet's own digest-proven identity is what this peer acts as"
    );
    assert_eq!(
        submit_local(&mut world, pause("solo-claim", SOLO_GM_OPERATOR_ID)),
        Err(GmActionRefusalReason::OperatorMismatch)
    );
}
