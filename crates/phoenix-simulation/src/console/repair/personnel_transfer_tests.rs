use super::*;
fn fixture() -> (World, Entity) {
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/personnel_carrier.toml",
    )
    .unwrap();
    let mut world = World::new();
    world.insert_resource(crate::sim_tick::SimTick(20));
    let actor = crate::entities::spawner::spawn_entity(
        &mut world.commands(),
        &config,
        Vec3::ZERO,
        "owner".into(),
        None,
    );
    world.flush();
    (world, actor)
}
fn projection(
    world: &World,
    actor: Entity,
    roster: &PersonnelRoster,
    context: &PersonnelTransferContext,
    permitted: &BTreeSet<String>,
) -> PersonnelView {
    let profile = world.get::<CompiledPersonnel>(actor).unwrap();
    let teams = world.get::<ShipRepairTeams>(actor).unwrap();
    let mut view = roster.state("owner").unwrap().view(
        profile,
        teams.0.job_state(),
        teams.0.layout().unwrap(),
        20,
    );
    decorate_view(
        &mut view,
        Some(context),
        "owner",
        profile,
        roster,
        teams,
        None,
        20,
        permitted,
    );
    view
}
#[test]
fn remote_deployed_foreign_carried_and_lost_variants_are_identical_permitted_facts() {
    let (world, actor) = fixture();
    let accepted = world.resource::<PersonnelRoster>().clone();
    let context = PersonnelTransferContext {
        tick: Some(20),
        ..Default::default()
    };
    let mut expected = None;
    for (location, condition) in [
        (
            OfficerLocation::AboardCraft {
                craft: "deployed-pod".into(),
                berth: "passenger".into(),
            },
            PersonnelCondition::Fit,
        ),
        (
            OfficerLocation::AboardCraft {
                craft: "foreign-carried".into(),
                berth: "passenger".into(),
            },
            PersonnelCondition::Severe,
        ),
        (
            OfficerLocation::Lost {
                actor: "lost-remote".into(),
            },
            PersonnelCondition::Minor,
        ),
    ] {
        let mut roster = accepted.clone();
        let person = roster
            .state_mut("owner")
            .unwrap()
            .officers
            .get_mut("repair-chief")
            .unwrap();
        person.location = location;
        person.condition = condition;
        person.changed_tick = 19;
        let view = projection(&world, actor, &roster, &context, &BTreeSet::new());
        assert!(view
            .officers
            .iter()
            .all(|person| person.id != "repair-chief"));
        assert!(view.carried_destinations.is_empty());
        if let Some(expected) = &expected {
            assert_eq!(&view, expected);
        } else {
            expected = Some(view);
        }
    }
}
#[test]
fn only_current_owned_stored_pod_and_explicit_feed_offer_actual_finite_berths() {
    let (world, actor) = fixture();
    let roster = world.resource::<PersonnelRoster>();
    let teams = world.get::<ShipRepairTeams>(actor).unwrap();
    let profile = world.get::<CompiledPersonnel>(actor).unwrap();
    let pod = StoredPod {
        bay: SystemId("probe-bay".into()),
        compartment: "probe-bay-room".into(),
        berths: vec!["passenger".into()],
        available: true,
        bay_busy: false,
        name: None,
    };
    let context = PersonnelTransferContext {
        tick: Some(20),
        carried: BTreeMap::from([(("owner".into(), "pod".into()), pod)]),
    };
    let request = PersonnelDestinationRequest {
        destination: PersonnelDestination::Craft {
            craft: "pod".into(),
        },
        requested_tick: 20,
    };
    let permitted = BTreeSet::from(["pod".into()]);
    assert!(plan(
        &context,
        "owner",
        profile,
        roster,
        teams,
        "repair-chief",
        &request,
        20,
        &BTreeSet::new()
    )
    .is_none());
    assert!(plan(
        &context,
        "owner",
        profile,
        roster,
        teams,
        "repair-chief",
        &request,
        21,
        &permitted
    )
    .is_none());
    assert!(plan(
        &context,
        "other",
        profile,
        roster,
        teams,
        "repair-chief",
        &request,
        20,
        &permitted
    )
    .is_none());
    assert!(plan(
        &context,
        "owner",
        profile,
        roster,
        teams,
        "repair-chief",
        &request,
        20,
        &permitted
    )
    .is_some());
    let mut occupied = roster.clone();
    occupied
        .state_mut("owner")
        .unwrap()
        .officers
        .get_mut("technician")
        .unwrap()
        .location = OfficerLocation::AboardCraft {
        craft: "pod".into(),
        berth: "passenger".into(),
    };
    assert!(plan(
        &context,
        "owner",
        profile,
        &occupied,
        teams,
        "repair-chief",
        &request,
        20,
        &permitted
    )
    .is_none());
    let visible = projection(&world, actor, &occupied, &context, &permitted);
    assert_eq!(visible.carried_destinations[0].free_berths, 0);
    assert_eq!(visible.officers.len(), 2);
    let hidden = projection(&world, actor, &occupied, &context, &BTreeSet::new());
    assert_eq!(hidden.officers.len(), 1);
    assert!(hidden.carried_destinations.is_empty());
}
