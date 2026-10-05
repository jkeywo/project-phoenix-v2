use super::*;
use crate::command_admission::log::HostSlot;
use crate::lockstep::{FleetGm, FleetRoster};

#[test]
fn a_fleetless_peer_binds_the_operator_admission_reads() {
    let mut world = World::new();
    world.insert_resource(FleetRoster::default());
    world.init_resource::<GmRoster>();

    let bound = bind_standalone_game_master(&mut world).expect("a solo peer binds");
    assert_eq!(bound.id, SOLO_GM_OPERATOR_ID);
    assert!(bound.connected);
    assert!(!bound.ready);

    let roster = world.resource::<FleetRoster>();
    assert_eq!(
        roster.gm_operator(roster.local()),
        Some(SOLO_GM_OPERATOR_ID)
    );
    // Still exactly the session it was: one peer, flying the one hull the
    // landing picked. Binding an identity must not cost the world its ship.
    assert!(roster.is_solo());
    assert_eq!(roster.len(), 1);
    assert!(world
        .resource::<GmRoster>()
        .is_connected(SOLO_GM_OPERATOR_ID));
    assert_eq!(
        local_gm_operator(&world).map(|row| row.id),
        Some(SOLO_GM_OPERATOR_ID.to_string())
    );
}

#[test]
fn an_unbound_peer_reads_back_no_operator() {
    let mut world = World::new();
    world.insert_resource(FleetRoster::default());
    world.init_resource::<GmRoster>();
    assert!(local_gm_operator(&world).is_none());
}

#[test]
fn a_bound_operator_absent_or_disconnected_from_the_public_roster_reads_back_none() {
    let mut world = World::new();
    world.insert_resource(FleetRoster::solo_game_master(SOLO_GM_OPERATOR_ID).expect("bounded id"));
    world.init_resource::<GmRoster>();
    assert!(local_gm_operator(&world).is_none());

    world.insert_resource(
        GmRoster::try_new(vec![GmOperator {
            id: SOLO_GM_OPERATOR_ID.into(),
            name: String::new(),
            connected: false,
            ready: false,
        }])
        .expect("bounded row"),
    );
    assert!(local_gm_operator(&world).is_none());
}

#[test]
fn a_page_roster_that_omits_the_standalone_operator_does_not_unseat_it() {
    let mut world = World::new();
    world.insert_resource(FleetRoster::default());
    world.init_resource::<GmRoster>();
    bind_standalone_game_master(&mut world).expect("a solo peer binds");

    // What the host page publishes for a session that has no fleet.
    let empty = GmRoster::default();
    let kept = preserved_standalone_presence(
        world.resource::<FleetRoster>(),
        Some(world.resource::<GmRoster>()),
        &empty,
    )
    .expect("the peer's own presence survives an empty fleet projection");
    assert!(kept.is_connected(SOLO_GM_OPERATOR_ID));
}

#[test]
fn a_page_roster_that_names_the_operator_is_left_exactly_as_published() {
    let mut world = World::new();
    world.insert_resource(FleetRoster::default());
    world.init_resource::<GmRoster>();
    bind_standalone_game_master(&mut world).expect("a solo peer binds");

    // Anything the page actually says about this operator wins — including
    // that it has gone.
    let published = GmRoster::try_new(vec![GmOperator {
        id: SOLO_GM_OPERATOR_ID.into(),
        name: "Morgan".into(),
        connected: false,
        ready: false,
    }])
    .unwrap();
    assert!(preserved_standalone_presence(
        world.resource::<FleetRoster>(),
        Some(world.resource::<GmRoster>()),
        &published,
    )
    .is_none());
}

#[test]
fn a_fleet_peers_published_roster_is_never_second_guessed() {
    let fleet = FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), HostSlot(2)],
        vec![FleetGm {
            host: HostSlot(2),
            operator_id: "gm-2".into(),
        }],
        HostSlot(2),
        HostSlot(1),
    )
    .expect("a GM-only fleet participant");
    assert!(preserved_standalone_presence(&fleet, None, &GmRoster::default()).is_none());
}

#[test]
fn an_already_admitted_fleet_peer_is_never_rebound_underneath_its_mesh() {
    let mut world = World::new();
    let fleet = FleetRoster::with_participants_and_gms(
        Vec::new(),
        vec![HostSlot(1), HostSlot(2)],
        vec![FleetGm {
            host: HostSlot(2),
            operator_id: "gm-7".into(),
        }],
        HostSlot(2),
        HostSlot(1),
    )
    .expect("a GM-only fleet participant");
    world.insert_resource(fleet.clone());
    world.init_resource::<GmRoster>();

    assert!(bind_standalone_game_master(&mut world).is_none());
    assert_eq!(world.resource::<FleetRoster>(), &fleet);
}

#[test]
fn binding_preserves_the_other_operators_the_public_roster_already_carries() {
    let mut world = World::new();
    world.insert_resource(FleetRoster::default());
    world.insert_resource(
        GmRoster::try_new(vec![GmOperator {
            id: "gm-9".into(),
            name: "Morgan".into(),
            connected: true,
            ready: false,
        }])
        .expect("bounded row"),
    );

    bind_standalone_game_master(&mut world).expect("a solo peer binds");
    assert_eq!(
        world
            .resource::<GmRoster>()
            .operators()
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        vec![SOLO_GM_OPERATOR_ID, "gm-9"]
    );
}
