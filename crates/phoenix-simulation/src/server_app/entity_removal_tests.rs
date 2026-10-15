use super::*;
use crate::console_delivery::ConsoleDeliveryProjection;
use crate::core::messages::{DeliveryClass, EntitySnapshot, WorldData};
use crate::lobby::{OutboundMessage, Sessions};

fn fixture(target: &str, charted: bool, concealed: bool) -> (World, Entity) {
    let mut world = World::new();
    let mut sessions = crate::lobby::session::SessionManager::new();
    sessions.register("phone".into(), "Crew".into()).unwrap();
    world.insert_resource(Sessions(sessions));
    world.insert_resource(WorldResource(WorldData {
        entities: vec![
            EntitySnapshot {
                uuid: "own".into(),
                tags: vec!["ship".into()],
                ..Default::default()
            },
            EntitySnapshot {
                uuid: target.into(),
                tags: if charted {
                    vec![
                        crate::console_delivery::CHARTED_TAG.into(),
                        "structure".into(),
                        "gm_removable".into(),
                    ]
                } else {
                    vec!["structure".into(), "gm_removable".into()]
                },
                position: Some([12.0, 0.0, 34.0]),
                ..Default::default()
            },
        ],
        ..Default::default()
    }));
    world.insert_resource(TrackedEntities {
        reported: ["own".into(), target.into()].into_iter().collect(),
        seeded: true,
    });
    world.insert_resource(SimOutbox::default());
    world.insert_resource(super::super::LastBroadcastEntityPositions(
        [(target.into(), (Vec3::ZERO, 0.0))].into_iter().collect(),
    ));
    world.insert_resource(super::super::LastBroadcastEntityHealth(
        [(target.into(), (Some(1.0), None, None, None))]
            .into_iter()
            .collect(),
    ));
    let mut content = WorldContentRuntime::default();
    if concealed {
        content
            .contact_overrides
            .entry("own".into())
            .or_default()
            .insert(
                target.into(),
                crate::gm::operations::contact::ContactMode::Conceal,
            );
    }
    world.insert_resource(content);
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .unwrap();
    world.spawn((
        LocalShip,
        EntityUuid("own".into()),
        crate::science::broad::BroadProfile::from_config(&config)
            .unwrap()
            .unwrap(),
    ));
    let target = world
        .spawn((
            EntityUuid(target.into()),
            crate::entities::spawner::EntityTagsSection(vec![
                "structure".into(),
                "gm_removable".into(),
            ]),
        ))
        .id();
    (world, target)
}

fn drain_phone(world: &mut World) -> Vec<OutboundMessage> {
    let mut state = SystemState::<ConsoleDeliveryProjection>::new(world);
    let projection = state.get(world).capture();
    world
        .resource_mut::<SimOutbox>()
        .drain()
        .into_iter()
        .flat_map(|out| {
            projection.project(&OutboundMessage {
                target: out.target,
                delivery: out.delivery,
                msg: out.message,
            })
        })
        .filter(|out| out.target == Target::Token("phone".into()))
        .collect()
}

#[test]
fn shared_script_and_gm_removal_samples_conceal_before_erasing_knowledge() {
    for (charted, concealed, frames) in [(false, false, 0), (true, true, 0), (true, false, 1)] {
        let (mut world, entity) = fixture("removed", charted, concealed);
        // This is the same real policy check GM invokes; ordinary script
        // destruction then uses the same cleanup owner without a GM override.
        let mut validation =
            SystemState::<crate::gm::operations::despawn::RemovalQuery>::new(&mut world);
        assert!(crate::gm::operations::despawn::validate_target(
            &validation.get(&world),
            "removed"
        )
        .is_ok());
        let cleared = crate::gm::operations::despawn::remove_entity(&mut world, entity);
        assert!(world.get_entity(entity).is_err());
        assert!(!world
            .resource::<TrackedEntities>()
            .reported
            .contains("removed"));
        assert!(!world
            .resource::<super::super::LastBroadcastEntityPositions>()
            .0
            .contains_key("removed"));
        assert!(!world
            .resource::<super::super::LastBroadcastEntityHealth>()
            .0
            .contains_key("removed"));
        assert!(!world
            .resource::<WorldResource>()
            .0
            .entities
            .iter()
            .any(|entity| entity.uuid == "removed"));
        assert_eq!(cleared.len(), usize::from(concealed));
        assert!(world
            .resource::<WorldContentRuntime>()
            .contact_overrides
            .get("own")
            .is_none());
        let phone = drain_phone(&mut world);
        assert_eq!(phone.len(), frames, "chart={charted}, conceal={concealed}");
        if frames > 0 {
            assert_eq!(phone[0].delivery, DeliveryClass::Reliable);
            let ServerMessage::WorldSetup { world } = &phone[0].msg else {
                panic!("visible chart replacement")
            };
            assert_eq!(
                world
                    .entities
                    .iter()
                    .map(|entity| entity.uuid.as_str())
                    .collect::<Vec<_>>(),
                ["own"]
            );
        }
        // Metadata was forgotten at the actual removal boundary; repeating the
        // bookkeeping cannot publish another chart refresh or terminal packet.
        remove_scripted_entity_record(&mut world, "removed");
        assert!(drain_phone(&mut world).is_empty());
    }
}

#[test]
fn shared_kill_site_helper_keeps_optional_resources_and_removes_a_chart_once() {
    let (mut world, _) = fixture("chart", true, false);
    let mut state = SystemState::<(EntityRemoval, ResMut<SimOutbox>)>::new(&mut world);
    {
        let (mut removal, mut outbox) = state.get_mut(&mut world);
        removal.remove("chart", Some(&mut outbox));
        removal.remove("chart", Some(&mut outbox));
    }
    assert_eq!(drain_phone(&mut world).len(), 1);
    assert!(!world
        .resource::<TrackedEntities>()
        .reported
        .contains("chart"));
    let mut bare = World::new();
    let mut state = SystemState::<EntityRemoval>::new(&mut bare);
    state.get_mut(&mut bare).remove("absent", None);
}

#[test]
fn scripted_own_terminal_survives_the_same_phone_projection() {
    let (mut world, _) = fixture("other", false, false);
    remove_scripted_entity_record(&mut world, "own");
    let phone = drain_phone(&mut world);
    assert_eq!(phone.len(), 1);
    assert!(matches!(&phone[0].msg, ServerMessage::EntityDespawned { uuid } if uuid == "own"));
}
