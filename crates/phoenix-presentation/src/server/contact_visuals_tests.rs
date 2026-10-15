use super::*;
use crate::server::measured_contacts::tests::{config, grant_to_viewscreen, science_board, track};

fn app() -> (App, Entity, Entity) {
    let mut app = App::new();
    app.insert_resource(State::new(GamePhase::InProgress));
    app.init_resource::<crate::actor_visibility::ObserverVisibilityRoots>();
    app.add_systems(
        Update,
        (sync_contact_visuals, crate::actor_visibility::sync).chain(),
    );
    let mut config = config();
    grant_to_viewscreen(&mut config);
    let observer = app
        .world_mut()
        .spawn((
            LocalShip,
            EntityUuid("observer".into()),
            SensingControls(Default::default()),
            ShipConfigComponent(config),
            science_board(track("observed-alias")),
            ShipViewMode::default(),
            Visibility::Visible,
            Transform::IDENTITY,
        ))
        .id();
    let actor = app
        .world_mut()
        .spawn((
            EntityUuid("private-live-actor".into()),
            Visibility::Visible,
            Transform::from_xyz(900.0, 40.0, 800.0).with_rotation(Quat::from_rotation_y(1.5)),
        ))
        .id();
    (app, observer, actor)
}

#[test]
fn proxy_uses_recorded_position_and_style_without_mutating_live_pose() {
    let (mut app, observer, actor) = app();
    let actual_pose = *app.world().get::<Transform>(actor).unwrap();
    let mut world = WorldConfig::default();
    world.render = Some(
        toml::from_str("[contact_proxy]\nminimum_radius = 12\ncolour = [0.1,0.2,0.3]").unwrap(),
    );
    app.insert_resource(world);
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(actor),
        Some(&Visibility::Hidden)
    );
    assert_eq!(app.world().get::<Transform>(actor), Some(&actual_pose));
    assert_eq!(
        app.world().get::<Visibility>(observer),
        Some(&Visibility::Visible)
    );
    let mut proxies = app
        .world_mut()
        .query::<(&ContactProxy, &Transform, Option<&EntityUuid>)>();
    let (proxy, transform, id) = proxies.single(app.world()).unwrap();
    assert_eq!(proxy.contact, "observed-alias");
    assert_eq!(proxy.radius, 12.0);
    assert_eq!(proxy.colour, [0.1, 0.2, 0.3]);
    assert_eq!(transform.translation, Vec3::new(11.0, 0.0, 22.0));
    assert_eq!(transform.rotation, Quat::IDENTITY);
    assert!(
        id.is_none(),
        "presentation proxies are not simulated actors"
    );
}

#[test]
fn loss_or_missing_knowledge_removes_proxy_without_restoring_omniscient_meshes() {
    let (mut app, observer, actor) = app();
    app.update();
    assert_eq!(
        app.world_mut()
            .query::<&ContactProxy>()
            .iter(app.world())
            .count(),
        1
    );
    app.world_mut()
        .entity_mut(observer)
        .remove::<ShipSystemBlackboards>();
    app.update();
    assert_eq!(
        app.world_mut()
            .query::<&ContactProxy>()
            .iter(app.world())
            .count(),
        0
    );
    assert_eq!(
        app.world().get::<Visibility>(actor),
        Some(&Visibility::Hidden)
    );
    app.world_mut()
        .entity_mut(observer)
        .remove::<SensingControls>();
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(actor),
        Some(&Visibility::Visible)
    );
}

#[test]
fn charted_bodies_remain_visible_and_phase_exit_restores_original_visibility() {
    let (mut app, _, actor) = app();
    let landmark = app
        .world_mut()
        .spawn((
            EntityUuid("known-moon".into()),
            EntityTagsSection(vec![crate::console_delivery::CHARTED_TAG.into()]),
            Visibility::Visible,
        ))
        .id();
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(landmark),
        Some(&Visibility::Visible)
    );
    app.insert_resource(State::new(GamePhase::Lobby));
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(actor),
        Some(&Visibility::Visible)
    );
    assert_eq!(
        app.world_mut()
            .query::<&ContactProxy>()
            .iter(app.world())
            .count(),
        0
    );
}

#[test]
fn accepted_ship_destruction_keeps_foreign_meshes_hidden_and_clears_proxies() {
    let (mut app, observer, actor) = app();
    let mut accepted = crate::lobby::server::ShipClientConfigResource::default();
    accepted.0.system_kinds.insert(
        "sensor-radar".into(),
        crate::ship::system_registry::SENSOR_RADAR_KIND.into(),
    );
    app.insert_resource(accepted);
    app.update();
    app.world_mut().entity_mut(observer).despawn();
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(actor),
        Some(&Visibility::Hidden)
    );
    assert_eq!(
        app.world_mut()
            .query::<&ContactProxy>()
            .iter(app.world())
            .count(),
        0
    );
}

#[test]
fn observer_conceal_overrides_charted_mesh_permission_and_normal_restores_it() {
    let (mut app, _, _) = app();
    let landmark = app
        .world_mut()
        .spawn((
            EntityUuid("known-moon".into()),
            EntityTagsSection(vec![crate::console_delivery::CHARTED_TAG.into()]),
            Visibility::Visible,
        ))
        .id();
    let mut runtime = crate::world::server::WorldContentRuntime::default();
    crate::gm_contact::set(
        &mut runtime.contact_overrides,
        "observer",
        "known-moon",
        crate::gm_contact::ContactMode::Conceal,
    );
    app.insert_resource(runtime);
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(landmark),
        Some(&Visibility::Hidden)
    );
    crate::gm_contact::set(
        &mut app
            .world_mut()
            .resource_mut::<crate::world::server::WorldContentRuntime>()
            .contact_overrides,
        "observer",
        "known-moon",
        crate::gm_contact::ContactMode::Normal,
    );
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(landmark),
        Some(&Visibility::Visible)
    );
}

#[test]
fn late_model_children_cannot_override_hidden_actor_with_explicit_visible() {
    let (mut app, _, actor) = app();
    app.update();
    let late_child = app.world_mut().spawn(Visibility::Visible).id();
    app.world_mut().entity_mut(actor).add_child(late_child);
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(late_child),
        Some(&Visibility::Hidden)
    );
    app.insert_resource(State::new(GamePhase::Lobby));
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(late_child),
        Some(&Visibility::Visible)
    );
}

#[test]
fn actual_generated_rock_bundle_does_not_bypass_measured_visibility() {
    let (mut app, _, _) = app();
    let authored = crate::entities::include_resolve::load_entity_config(
        "assets/entities/asteroid_common_1_small.toml",
    )
    .unwrap();
    let rock = crate::asteroids::lifecycle::RockConfig {
        collider: authored.collider.clone().unwrap(),
        max_hp: authored.hull.as_ref().unwrap().hull_integrity,
        tags: authored.tags.clone(),
        target_tags: authored.target.as_ref().unwrap().tags.clone(),
        mesh: authored.mesh.clone(),
        radar_icon: authored.radar_appearance.as_ref().unwrap().icon.clone(),
        radar_colour: None,
        radar_size: None,
    };
    let actual = Transform::from_xyz(80.0, 3.0, -20.0).with_rotation(Quat::from_rotation_y(0.7));
    let generated = app
        .world_mut()
        .spawn((
            crate::asteroids::lifecycle::rock_bundle(
                "private-rock",
                &rock,
                actual.translation,
                actual.rotation,
                0.0,
                rock.max_hp,
            ),
            Visibility::Visible,
        ))
        .id();
    assert!(app.world().get::<EntityUuid>(generated).is_none());
    assert_eq!(
        app.world().get::<AsteroidUuid>(generated).unwrap().0,
        "private-rock"
    );
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(generated),
        Some(&Visibility::Hidden)
    );
    assert_eq!(app.world().get::<Transform>(generated), Some(&actual));
    // Only the separately held observation is drawn, never a UUID/pose copied
    // from the generated hazard's actual private body.
    let mut proxies = app.world_mut().query::<(&ContactProxy, &Transform)>();
    let (proxy, pose) = proxies.single(app.world()).unwrap();
    assert_eq!(proxy.contact, "observed-alias");
    assert_eq!(pose.translation, Vec3::new(11.0, 0.0, 22.0));
    // An explicit authored chart grant is still honoured for this ordinary
    // generated entity, whose tags live in the accepted world roster.
    let mut world = crate::lobby::WorldResource::default();
    world
        .0
        .entities
        .push(crate::core::messages::EntitySnapshot {
            uuid: "private-rock".into(),
            tags: vec![crate::console_delivery::CHARTED_TAG.into()],
            ..Default::default()
        });
    app.insert_resource(world);
    app.update();
    assert_eq!(
        app.world().get::<Visibility>(generated),
        Some(&Visibility::Visible)
    );
}

#[test]
fn released_custody_never_reveals_a_concealed_or_unobserved_foreign_actor() {
    use bevy::ecs::entity_disabling::Disabled;
    use phoenix_model::{craft::CraftLocation, identity::SystemId};
    use phoenix_simulation::entities::craft::ProbeCustody;
    for original in [Visibility::Visible, Visibility::Hidden] {
        let (mut app, observer, actor) = app();
        app.world_mut()
            .entity_mut(actor)
            .insert((original, ProbeCustody(CraftLocation::Deployed {})));
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(actor),
            Some(&Visibility::Hidden)
        );
        app.world_mut().entity_mut(actor).insert((
            Disabled,
            ProbeCustody(CraftLocation::Carried {
                parent_uuid: "carrier".into(),
                bay: SystemId("bay".into()),
            }),
        ));
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(actor),
            Some(&Visibility::Hidden)
        );
        app.world_mut()
            .entity_mut(actor)
            .remove::<Disabled>()
            .insert(ProbeCustody(CraftLocation::Deployed {}));
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(actor),
            Some(&Visibility::Hidden),
            "release never bypasses observer privacy"
        );
        app.world_mut()
            .entity_mut(observer)
            .remove::<SensingControls>();
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(actor),
            Some(&original),
            "only all hide reasons clearing restores prior visibility"
        );
    }
}
