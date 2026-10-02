use super::*;

#[cfg(feature = "server")]
#[test]
fn switched_widgets_do_not_render_or_fit_stale_objective_sources_after_resolution() {
    use crate::core::messages::ViewMode;
    use crate::entities::spawner::EntityUuid;
    use crate::lobby::WorldResource;
    use crate::server_app::LocalShip;
    use crate::ship::state::{ShipPhysics, ShipViewMode};
    use crate::world::server::ObjectiveManagerRes;
    use bevy::ecs::system::RunSystemOnce;

    fn render(world: &mut World) {
        world
            .run_system_once(crate::server::radar::sync_server_radar_bridge)
            .unwrap();
        world.run_system_once(sync_radar_blip_nodes).unwrap();
    }
    fn drawn_counts(world: &World, widget: Entity) -> (usize, usize, usize, usize) {
        let children = world.get::<Children>(widget).unwrap();
        (
            children
                .iter()
                .filter(|child| world.get::<RadarBlipNode>(*child).is_some())
                .count(),
            children
                .iter()
                .filter(|child| world.get::<RadarRegionNode>(*child).is_some())
                .count(),
            children
                .iter()
                .filter(|child| world.get::<RadarLabelNode>(*child).is_some())
                .count(),
            world
                .get::<RadarObjectiveRingEntities>(widget)
                .unwrap()
                .0
                .len(),
        )
    }
    for resolution in ["complete", "fail", "unload"] {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<Image>()
            .init_asset::<RadarBlipMaterial>()
            .init_resource::<RadarIconLookup>()
            .init_resource::<WorldResource>()
            .init_resource::<ObjectiveManagerRes>();
        app.world_mut()
            .resource_mut::<RadarIconLookup>()
            .0
            .insert("waypoint".into(), Handle::default());
        app.world_mut()
            .resource_mut::<WorldResource>()
            .0
            .entities
            .push(EntitySnapshot {
                uuid: "marker".into(),
                name: Some("Objective beacon".into()),
                tags: tags(&["objective_marker", "region"]),
                position: Some([500.0, 0.0, 0.0]),
                radar_icon: Some("waypoint".into()),
                radar_size: Some(4.0),
                region_colour: Some([0.1, 0.2, 0.3]),
                objective_target: true,
                ..default()
            });
        app.world_mut().resource_mut::<ObjectiveManagerRes>().0.add(
            "private",
            "objective.test",
            true,
            vec!["marker".into()],
        );
        app.world_mut()
            .resource_mut::<ObjectiveManagerRes>()
            .0
            .set_recipients("private", vec!["ship-a".into()]);
        let mut mode = ShipViewMode::default();
        mode.view_mode = ViewMode::SystemChart;
        let ship = app
            .world_mut()
            .spawn((
                LocalShip,
                EntityUuid("ship-a".into()),
                ShipPhysics::default(),
                mode,
            ))
            .id();
        let mut widgets = Vec::new();
        for console in [
            ConsoleRadar::ViewscreenSystemChart,
            ConsoleRadar::ViewscreenNav,
        ] {
            let widget = app
                .world_mut()
                .spawn((
                    console,
                    GenericRadarWidget {
                        range: 1000.0,
                        orientation: OrientationMode::WorldFixed,
                        filter: filter(&["objective_marker"]),
                        clip_mode: RadarClipMode::Circle,
                        face_fraction: 1.0,
                    },
                    Node::default(),
                    ComputedNode {
                        size: Vec2::splat(200.0),
                        ..default()
                    },
                    InheritedVisibility::VISIBLE,
                    RadarBlipMap::default(),
                    RadarBlipLabels,
                    RadarObjectiveRingEntities::default(),
                ))
                .id();
            widgets.push(widget);
        }
        let [system, nav] = [widgets[0], widgets[1]];
        app.world_mut().entity_mut(nav).insert((
            WorldCentredRadar,
            AutoScaleRadar {
                margin: 1.1,
                min_range: 50.0,
            },
            InheritedVisibility::HIDDEN,
        ));
        render(app.world_mut());
        assert_eq!(drawn_counts(app.world(), system), (1, 1, 1, 1));
        let stale_source = app.world().get::<RadarBlipMap>(system).unwrap().blips["marker"];

        app.world_mut()
            .get_mut::<ShipViewMode>(ship)
            .unwrap()
            .view_mode = ViewMode::NavigationChart;
        app.world_mut()
            .entity_mut(system)
            .insert(InheritedVisibility::HIDDEN);
        app.world_mut()
            .entity_mut(nav)
            .insert(InheritedVisibility::VISIBLE);
        render(app.world_mut());
        assert_eq!(
            drawn_counts(app.world(), nav),
            (1, 1, 1, 1),
            "only the active widget's own source is drawn"
        );
        assert!(app.world().get::<GenericRadarWidget>(nav).unwrap().range > 500.0);

        let objectives = &mut app.world_mut().resource_mut::<ObjectiveManagerRes>().0;
        match resolution {
            "complete" => assert!(objectives.complete("private")),
            "fail" => assert!(objectives.fail("private")),
            _ => assert!(objectives.remove("private")),
        }
        render(app.world_mut());
        assert!(
            app.world()
                .get::<RadarAppearance>(stale_source)
                .unwrap()
                .objective_target,
            "hidden source deliberately remains stale"
        );
        assert_eq!(
            drawn_counts(app.world(), nav),
            (0, 0, 0, 0),
            "{resolution}: no stale icon, region, label or Objective ring"
        );
        assert_eq!(
            app.world().get::<GenericRadarWidget>(nav).unwrap().range,
            50.0,
            "{resolution}: hidden and inactive markers cannot enlarge the chart"
        );

        // Returning to the old widget refreshes its bridge and removes the
        // actual UI children it drew before the switch.
        app.world_mut()
            .get_mut::<ShipViewMode>(ship)
            .unwrap()
            .view_mode = ViewMode::SystemChart;
        app.world_mut()
            .entity_mut(nav)
            .insert(InheritedVisibility::HIDDEN);
        app.world_mut()
            .entity_mut(system)
            .insert(InheritedVisibility::VISIBLE);
        render(app.world_mut());
        assert_eq!(drawn_counts(app.world(), system), (0, 0, 0, 0));
        assert!(
            app.world().resource::<WorldResource>().0.entities[0].objective_target,
            "presentation never rewrites shared metadata"
        );
    }
}

// ── should_label predicate ────────────────────────────────────────────────

#[test]
fn should_label_named_nav_entity() {
    assert!(should_label(
        &Some("Sol".to_string()),
        &["star".to_string()]
    ));
    assert!(should_label(
        &Some("Starbase 12".to_string()),
        &["station".to_string(), "friendly".to_string()]
    ));
}

#[test]
fn should_label_rejects_missing_or_empty_name() {
    assert!(!should_label(&None, &["star".to_string()]));
    assert!(!should_label(
        &Some("".to_string()),
        &["planet".to_string()]
    ));
    assert!(!should_label(
        &Some("   ".to_string()),
        &["planet".to_string()]
    ));
}

#[test]
fn should_label_rejects_non_nav_tags() {
    assert!(!should_label(
        &Some("Raider".to_string()),
        &["pirate".to_string()]
    ));
    assert!(!should_label(
        &Some("Torp".to_string()),
        &["torpedo".to_string()]
    ));
    assert!(!should_label(
        &Some("You".to_string()),
        &["player".to_string()]
    ));
}

// ── icon_asset_path ────────────────────────────────────────────────────────

#[test]
fn icon_asset_path_capitalizes_first_letter_only() {
    assert_eq!(icon_asset_path("star"), "radar_icons/Icon-Star.png");
    assert_eq!(icon_asset_path("asteroid"), "radar_icons/Icon-Asteroid.png");
    assert_eq!(
        icon_asset_path("playerShip"),
        "radar_icons/Icon-PlayerShip.png"
    );
}

#[test]
fn contact_override_existing_material_changes_ordinary_basic_and_back() {
    use bevy::ecs::system::RunSystemOnce;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
        .init_asset::<Image>()
        .init_asset::<RadarBlipMaterial>()
        .init_resource::<RadarIconLookup>();
    let authored = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::default());
    let basic = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::default());
    assert_ne!(authored, basic);
    app.insert_resource(RadarBlipFallbackIcon(basic.clone()));
    app.world_mut()
        .resource_mut::<RadarIconLookup>()
        .0
        .insert("frigate".into(), authored.clone());
    let widget = app
        .world_mut()
        .spawn((
            GenericRadarWidget {
                range: 100.0,
                orientation: OrientationMode::WorldFixed,
                filter: filter(&["ship", crate::gm_contact::BASIC_RADAR_TAG]),
                clip_mode: RadarClipMode::Circle,
                face_fraction: 1.0,
            },
            Node::default(),
            ComputedNode {
                size: Vec2::splat(200.0),
                ..default()
            },
            InheritedVisibility::VISIBLE,
            RadarBlipMap::default(),
        ))
        .id();
    let overrides = [("target".into(), crate::gm_contact::ContactMode::Reveal)].into();
    let shows = ["ship".into(), crate::gm_contact::BASIC_RADAR_TAG.into()].into();
    let mut retained_node = None;
    for (x, expected) in [(20.0, authored.clone()), (200.0, basic), (20.0, authored)] {
        let entities = crate::gm_contact::viewscreen_contacts(
            &[EntitySnapshot {
                uuid: "target".into(),
                position: Some([x, 0.0, 0.0]),
                tags: tags(&["ship"]),
                radar_icon: Some("frigate".into()),
                radar_size: Some(4.0),
                ..default()
            }],
            &overrides,
            0.0,
            0.0,
            100.0,
            &shows,
        );
        app.world_mut()
            .run_system_once(
                move |mut commands: Commands, mut maps: Query<&mut RadarBlipMap>| {
                    bridge_sim_to_radar(
                        &mut commands,
                        widget,
                        &mut maps.get_mut(widget).unwrap(),
                        RadarCenterPose {
                            x: 0.0,
                            z: 0.0,
                            yaw: 0.0,
                        },
                        &entities,
                    );
                },
            )
            .unwrap();
        app.world_mut()
            .run_system_once(sync_radar_blip_nodes)
            .unwrap();
        let source = app.world().get::<RadarBlipMap>(widget).unwrap().blips["target"];
        let mut nodes = app
            .world_mut()
            .query::<(Entity, &RadarBlipNode, &MaterialNode<RadarBlipMaterial>)>();
        let (node, _, material) = nodes
            .iter(app.world())
            .find(|(_, tag, _)| tag.source == source)
            .unwrap();
        let identity = (node, material.0.clone());
        if let Some(previous) = &retained_node {
            assert_eq!(
                &identity, previous,
                "existing UI node and material must reconcile in place"
            );
        }
        retained_node = Some(identity);
        assert_eq!(
            app.world()
                .resource::<Assets<RadarBlipMaterial>>()
                .get(&material.0)
                .unwrap()
                .icon,
            expected,
            "actual texture must follow the current contact picture at x={x}"
        );
    }
}

#[test]
fn contact_override_projection_uses_its_current_widget_center_after_a_view_switch() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    let helm = world.spawn_empty().id();
    let stale_center = world.spawn(RadarCenter::default()).id();
    world.entity_mut(helm).add_child(stale_center);
    let sensors = world.spawn_empty().id();
    let current_center = world
        .spawn(RadarCenter {
            world_x: 1000.0,
            world_z: 2000.0,
            yaw: 1.0,
        })
        .id();
    world.entity_mut(sensors).add_child(current_center);
    let center = world
        .run_system_once(
            move |widgets: Query<&Children>, centers: Query<&RadarCenter>| {
                widget_radar_center(widgets.get(sensors).ok(), &centers).unwrap()
            },
        )
        .unwrap();
    // Reveal's edge-clamped contact is visible around the newly active
    // Sensors bridge even while Helm still carries the old position.
    let projected = project_radar_entity(
        1096.0,
        2000.0,
        center.world_x,
        center.world_z,
        center.yaw,
        100.0,
        2.0,
        &OrientationMode::WorldFixed,
    );
    assert_eq!(projected, Some((0.96, 0.0)));
    assert!(project_radar_entity(
        1096.0,
        2000.0,
        0.0,
        0.0,
        0.0,
        100.0,
        2.0,
        &OrientationMode::WorldFixed
    )
    .is_none());
}

#[test]
fn icon_asset_path_is_free_form_no_whitelist() {
    // Any icon name resolves by convention; there is no closed enum to
    // reject unrecognised names.
    assert_eq!(icon_asset_path("frigate"), "radar_icons/Icon-Frigate.png");
}

// ── helper constructors ───────────────────────────────────────────────────

fn filter(tags: &[&str]) -> RadarFilter {
    RadarFilter(tags.iter().map(|s| s.to_string()).collect())
}

fn tags(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// ── is_on_radar ──────────────────────────────────────────────────────────

#[test]
fn matching_tag_passes_filter() {
    assert!(is_on_radar(&filter(&["ship"]), &tags(&["ship"])));
}

#[test]
fn non_matching_tag_fails_filter() {
    assert!(!is_on_radar(&filter(&["ship"]), &tags(&["asteroid"])));
}

#[test]
fn empty_filter_excludes_all() {
    assert!(!is_on_radar(&filter(&[]), &tags(&["ship"])));
    assert!(!is_on_radar(&filter(&[]), &tags(&["asteroid"])));
}

#[test]
fn empty_tags_excluded_by_any_filter() {
    assert!(!is_on_radar(&filter(&["ship"]), &tags(&[])));
}

#[test]
fn multi_tag_entity_passes_if_any_match() {
    // entity has "pirate" and "ship"; filter includes "ship"
    assert!(is_on_radar(&filter(&["ship"]), &tags(&["pirate", "ship"])));
}

#[test]
fn player_tag_passes_player_filter() {
    assert!(is_on_radar(&filter(&["player"]), &tags(&["player"])));
}

#[test]
fn player_does_not_pass_ship_only_filter() {
    // player and ship are distinct tags
    assert!(!is_on_radar(&filter(&["ship"]), &tags(&["player"])));
}

#[test]
fn all_tags_filter_accepts_all_known_tags() {
    let f = filter(&[
        "player",
        "ship",
        "asteroid",
        "asteroid_field",
        "station",
        "missile",
        "planet",
        "star",
        "region",
    ]);
    for tag in &[
        "player",
        "ship",
        "asteroid",
        "asteroid_field",
        "station",
        "missile",
        "planet",
        "star",
        "region",
    ] {
        assert!(is_on_radar(&f, &tags(&[tag])), "tag {tag} should pass");
    }
}

// ── project_radar_entity ─────────────────────────────────────────────────

#[test]
fn center_entity_projects_to_zero() {
    let result = project_radar_entity(
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        100.0,
        0.0,
        &OrientationMode::ShipRelative,
    );
    let (x, y) = result.unwrap();
    assert!((x).abs() < 1e-5);
    assert!((y).abs() < 1e-5);
}

#[test]
fn entity_beyond_range_returns_none() {
    let result = project_radar_entity(
        200.0,
        0.0,
        0.0,
        0.0,
        0.0,
        100.0,
        0.0,
        &OrientationMode::ShipRelative,
    );
    assert!(result.is_none());
}

#[test]
fn zero_range_returns_none_safely() {
    let result = project_radar_entity(
        10.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        &OrientationMode::ShipRelative,
    );
    assert!(result.is_none());
}

#[test]
fn ship_relative_yaw_zero_ahead_entity_gives_positive_y() {
    // At yaw=0: forward = (sin(0), -cos(0)) = (0,-1) in XZ.
    // Entity at dz=-100 (ahead) → radar_y = dx*sin(0) - dz*cos(0) = 0 - (-100)*1 = +1.0
    let result = project_radar_entity(
        0.0,
        -100.0,
        0.0,
        0.0,
        0.0,
        100.0,
        0.0,
        &OrientationMode::ShipRelative,
    );
    let (_, y) = result.unwrap();
    assert!((y - 1.0).abs() < 1e-5, "expected radar_y=1.0, got {y}");
}

#[test]
fn world_fixed_ignores_yaw() {
    let yaw = std::f32::consts::FRAC_PI_2; // 90 degrees
    let ship_relative = project_radar_entity(
        50.0,
        0.0,
        0.0,
        0.0,
        yaw,
        100.0,
        0.0,
        &OrientationMode::ShipRelative,
    );
    let world_fixed = project_radar_entity(
        50.0,
        0.0,
        0.0,
        0.0,
        yaw,
        100.0,
        0.0,
        &OrientationMode::WorldFixed,
    );
    // WorldFixed should give the same as ShipRelative at yaw=0
    let world_fixed_0 = project_radar_entity(
        50.0,
        0.0,
        0.0,
        0.0,
        0.0,
        100.0,
        0.0,
        &OrientationMode::ShipRelative,
    );
    assert_ne!(
        ship_relative, world_fixed,
        "ship_relative should differ from world_fixed at non-zero yaw"
    );
    assert_eq!(
        world_fixed, world_fixed_0,
        "world_fixed should equal ship_relative@yaw=0"
    );
}

#[test]
fn entity_at_range_boundary_is_included() {
    // Exactly at range (dx=100, dz=0, range=100): dx²+dz² = 10000 = range²  → included.
    let result = project_radar_entity(
        100.0,
        0.0,
        0.0,
        0.0,
        0.0,
        100.0,
        0.0,
        &OrientationMode::WorldFixed,
    );
    assert!(result.is_some());
}

#[test]
fn entity_radius_extends_detection_range() {
    // Entity center at 120 units, range=100, entity_radius=25 → 120 <= 125 → included.
    let result = project_radar_entity(
        120.0,
        0.0,
        0.0,
        0.0,
        0.0,
        100.0,
        25.0,
        &OrientationMode::WorldFixed,
    );
    assert!(result.is_some());
}

#[test]
fn entity_radius_does_not_include_fully_out_of_range() {
    // Entity center at 200 units, range=100, entity_radius=25 → 200 > 125 → excluded.
    let result = project_radar_entity(
        200.0,
        0.0,
        0.0,
        0.0,
        0.0,
        100.0,
        25.0,
        &OrientationMode::WorldFixed,
    );
    assert!(result.is_none());
}

// ── world_size_to_px ─────────────────────────────────────────────────────

#[test]
fn world_size_to_px_returns_min_when_range_zero() {
    assert_eq!(world_size_to_px(50.0, 0.0, 140.0), MIN_BLIP_PX);
}

#[test]
fn world_size_to_px_returns_min_when_radius_zero() {
    assert_eq!(world_size_to_px(50.0, 500.0, 0.0), MIN_BLIP_PX);
}

#[test]
fn world_size_to_px_below_min_clamps_up() {
    assert_eq!(world_size_to_px(0.1, 500.0, 140.0), MIN_BLIP_PX);
}

#[test]
fn world_size_to_px_linear_interior() {
    // 50 / 500 * 140 * 2 = 28.0
    assert!((world_size_to_px(50.0, 500.0, 140.0) - 28.0).abs() < 1e-5);
}

#[test]
fn world_size_to_px_above_diameter_clamps() {
    // raw would be 1000/500*140*2 = 560 → clamps to diameter 280.
    assert!((world_size_to_px(1000.0, 500.0, 140.0) - 280.0).abs() < 1e-5);
}

// ── blip_local_offset ────────────────────────────────────────────────────

#[test]
fn blip_local_offset_centre_projection() {
    // Square radar: center == radar_radius == 140.
    let (left, top) = blip_local_offset(0.0, 0.0, 140.0, 140.0, 140.0, 8.0);
    assert!((left - 132.0).abs() < 1e-5);
    assert!((top - 132.0).abs() < 1e-5);
}

#[test]
fn blip_local_offset_right_edge() {
    let (left, top) = blip_local_offset(1.0, 0.0, 140.0, 140.0, 140.0, 8.0);
    assert!((left - 272.0).abs() < 1e-5);
    assert!((top - 132.0).abs() < 1e-5);
}

#[test]
fn blip_local_offset_top_edge() {
    // ny = 1 → top = 140 - 1*140 - 8 = -8 (Y flipped to UI's y-down).
    let (left, top) = blip_local_offset(0.0, 1.0, 140.0, 140.0, 140.0, 8.0);
    assert!((left - 132.0).abs() < 1e-5);
    assert!((top - (-8.0)).abs() < 1e-5);
}

#[test]
fn blip_local_offset_bottom_edge() {
    let (left, top) = blip_local_offset(0.0, -1.0, 140.0, 140.0, 140.0, 8.0);
    assert!((left - 132.0).abs() < 1e-5);
    assert!((top - 272.0).abs() < 1e-5);
}

#[test]
fn blip_local_offset_non_square_widget_centers_correctly() {
    // Widget is 300 wide × 200 tall → center_x=150, center_y=100, radius=100.
    // Player ship at (0,0) should land at (center_x - half, center_y - half).
    let half = 8.0;
    let (left, top) = blip_local_offset(0.0, 0.0, 150.0, 100.0, 100.0, half);
    assert!((left - (150.0 - half)).abs() < 1e-5, "left={left}");
    assert!((top - (100.0 - half)).abs() < 1e-5, "top={top}");
}

// ── HiDPI / scale-factor regression ──────────────────────────────────────
//
// `ComputedNode::size()` and `GlobalTransform::translation()` are in
// **physical** pixels, but blip nodes are positioned with `Val::Px(..)`
// which Bevy treats as **logical** pixels. On HiDPI displays
// (`scale_factor != 1`) `sync_radar_blip_nodes` must convert physical
// values to logical (multiply by `inverse_scale_factor`) before feeding
// them into `blip_local_offset`, otherwise blips render at the wrong
// distance from radar centre and `on_tactical_radar_tap` projects
// entities into a different space than the pointer.

#[test]
fn blip_local_offset_at_phone_scale_factor_lays_out_in_logical_px() {
    // Phone: scale_factor = 2, so a 240-logical-px radar widget has
    // ComputedNode::size() == 480 physical px.
    let scale_factor: f32 = 2.0;
    let inv_sf = 1.0 / scale_factor;
    let physical_size = 480.0_f32;

    // The renderer must convert to logical before computing layout, so
    // the values flowing into blip_local_offset are 240, 240, 120.
    let logical_size = physical_size * inv_sf;
    let centre_logical = logical_size * 0.5;
    let radius_logical = logical_size * 0.5;

    // Entity projected at right edge of the radar (nx=1, ny=0).
    let (left, top) = blip_local_offset(
        1.0,
        0.0,
        centre_logical,
        centre_logical,
        radius_logical,
        8.0,
    );

    // Expected (in logical px, ready for Val::Px):
    //   left = 120 + 1.0 * 120 - 8 = 232
    //   top  = 120 - 0.0 * 120 - 8 = 112
    assert!((left - 232.0).abs() < 1e-5, "left={left}");
    assert!((top - 112.0).abs() < 1e-5, "top={top}");

    // Cross-check the bug we just fixed: had the renderer fed the raw
    // physical values straight in (centre=240, radius=240), the right-edge
    // blip would have ended up at left = 240 + 240 - 8 = 472, which is
    // far outside the 240-logical-px-wide radar widget → blips drift /
    // clip / tap math diverges from visual layout on phones.
    let (bad_left, _) = blip_local_offset(
        1.0,
        0.0,
        physical_size * 0.5,
        physical_size * 0.5,
        physical_size * 0.5,
        8.0,
    );
    assert!(
        bad_left > logical_size,
        "regression sentinel: bad_left={bad_left} should overflow {logical_size}"
    );
}

// ── apply_zoom_step ───────────────────────────────────────────────────────

#[test]
fn apply_zoom_step_multiplies_in_range() {
    assert!((apply_zoom_step(1.0, 1.25) - 1.25).abs() < 1e-6);
    assert!((apply_zoom_step(2.0, 0.5) - 1.0).abs() < 1e-6);
}

#[test]
fn apply_zoom_step_clamps_at_max() {
    assert_eq!(apply_zoom_step(RADAR_MAX_ZOOM, 4.0), RADAR_MAX_ZOOM);
}

#[test]
fn apply_zoom_step_clamps_at_min() {
    assert_eq!(apply_zoom_step(RADAR_MIN_ZOOM, 0.1), RADAR_MIN_ZOOM);
}

// ── pinch_zoom ────────────────────────────────────────────────────────────

#[test]
fn pinch_zoom_applies_distance_ratio() {
    // Fingers move apart 2× → zoom in 2×.
    assert!((pinch_zoom(1.0, 50.0, 100.0) - 2.0).abs() < 1e-6);
    // Fingers move together → zoom out.
    assert!((pinch_zoom(2.0, 100.0, 50.0) - 1.0).abs() < 1e-6);
}

#[test]
fn pinch_zoom_zero_prev_dist_is_noop() {
    assert_eq!(pinch_zoom(1.5, 0.0, 100.0), 1.5);
}

#[test]
fn pinch_zoom_respects_clamp() {
    assert_eq!(pinch_zoom(RADAR_MAX_ZOOM, 10.0, 1000.0), RADAR_MAX_ZOOM);
}

// ── px_to_world_delta ─────────────────────────────────────────────────────

#[test]
fn px_to_world_delta_zero_radius_is_zero() {
    assert_eq!(px_to_world_delta(50.0, 1000.0, 0.0), 0.0);
    assert_eq!(px_to_world_delta(50.0, 1000.0, -1.0), 0.0);
}

#[test]
fn px_to_world_delta_scales_linearly() {
    // Half the radar half-width across a 1000-unit range = 500 world units.
    assert!((px_to_world_delta(100.0, 1000.0, 200.0) - 500.0).abs() < 1e-4);
    // Doubling the pixel delta doubles the world delta.
    let a = px_to_world_delta(10.0, 800.0, 160.0);
    let b = px_to_world_delta(20.0, 800.0, 160.0);
    assert!((b - 2.0 * a).abs() < 1e-4);
}

#[test]
fn radar_blip_press_triggers_event_on_radar_with_source_payload() {
    // detect_radar_blip_press must fire RadarBlipClicked targeting the
    // **radar widget** entity (the parent of the blip UI node), carrying
    // the **source** ECS blip entity in the payload. This is the contract
    // every per-console observer (tactical, sensors, …) relies on.

    use bevy::ecs::observer::On;
    use std::sync::{Arc, Mutex};

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_systems(Update, detect_radar_blip_press);

    // Spawn the source ECS entity (what the wire-level uuid lives on).
    let source = app.world_mut().spawn_empty().id();

    // Spawn the radar widget entity.
    let radar = app.world_mut().spawn_empty().id();

    // Spawn the UI blip node as a child of the radar, with Pressed
    // interaction so the system picks it up next frame.
    let mut blip_cmd = app
        .world_mut()
        .spawn((RadarBlipNode { source }, Interaction::Pressed));
    blip_cmd.insert(ChildOf(radar));

    // Capture observer fires. Register on the **radar** entity — this is
    // the per-radar registration pattern consoles use.
    let captured: Arc<Mutex<Vec<(Entity, Entity)>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_for_obs = captured.clone();
    app.world_mut()
        .entity_mut(radar)
        .observe(move |trigger: On<RadarBlipClicked>| {
            let ev = trigger.event();
            captured_for_obs.lock().unwrap().push((ev.radar, ev.source));
        });

    // One frame: detect_radar_blip_press queues the trigger; observer runs.
    app.update();

    let hits = captured.lock().unwrap().clone();
    assert_eq!(hits.len(), 1, "observer fired exactly once: {hits:?}");
    assert_eq!(hits[0].0, radar, "event.radar is the radar entity");
    assert_eq!(
        hits[0].1, source,
        "event.source is the source ECS blip entity"
    );
}

// ── arc_contains ────────────────────────────────────────────────────────

#[test]
fn arc_contains_centre_bearing() {
    assert!(arc_contains(0.0, 90.0, 0.0));
}

#[test]
fn arc_contains_at_arc_edge_inclusive() {
    assert!(arc_contains(0.0, 90.0, 45.0));
    assert!(arc_contains(0.0, 90.0, -45.0));
}

#[test]
fn arc_contains_rejects_outside_arc() {
    assert!(!arc_contains(0.0, 90.0, 46.0));
    assert!(!arc_contains(0.0, 90.0, -46.0));
}

#[test]
fn arc_contains_wraps_around_180() {
    // Facing aft (180°) with 90° arc covers (135° .. -135°).
    assert!(arc_contains(180.0, 90.0, 170.0));
    assert!(arc_contains(180.0, 90.0, -170.0));
    assert!(!arc_contains(180.0, 90.0, 90.0));
}

#[test]
fn arc_contains_negative_arc_is_treated_as_absolute() {
    // A negative arc width is a TOML mistake; absolute keeps behaviour sane.
    assert!(arc_contains(0.0, -90.0, 30.0));
}

#[test]
fn arc_contains_starboard_phaser_at_90_deg() {
    // PRD example: starboard phaser bank facing 90° with 90° arc covers
    // (45° .. 135°) — beam can lock anywhere on the right hemisphere.
    assert!(arc_contains(90.0, 90.0, 90.0));
    assert!(arc_contains(90.0, 90.0, 45.0));
    assert!(arc_contains(90.0, 90.0, 135.0));
    assert!(!arc_contains(90.0, 90.0, 0.0));
    assert!(!arc_contains(90.0, 90.0, 180.0));
}
