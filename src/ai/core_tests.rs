use super::*;

/// No `ObjectiveCursors` entries — the right input for every test that is
/// not about patrol routes. `helm_patrol` treats a missing cursor as index
/// 0, so this is "the ship is at the start of any route it has".
const NO_CURSORS: &[crate::ai::patrol_cursor::PatrolCursor] = &[];

#[test]
fn surface_distance_xz_removes_both_hull_radii() {
    assert_eq!(
        surface_distance_xz([0.0, 0.0, 0.0], 10.0, [40.0, 0.0, 0.0], 5.0),
        25.0
    );
}

// ── hostile_arc_exposure: the all-round case (issue #874) ─────────────

/// A hostile carrying an all-round bank (`fire_arc_deg = 360`, which the
/// Alliance destroyer's `omni` suppression phaser authors) must reach the
/// fact reduction as INESCAPABLE with no escape magnitude — even when a
/// nearer hostile's finite arcs offer a real one, because turning out of
/// those does not turn out of the all-round one.
#[test]
fn an_all_round_hostile_suppresses_the_escape_magnitude_across_the_view() {
    let hostile_faction = uuid::Uuid::new_v4();
    let own_faction = uuid::Uuid::new_v4();
    let mut registry = crate::ai::faction::FactionRegistry::new();
    registry.insert(crate::ai::faction::FactionConfig {
        display_name: None,
        uuid: own_faction,
        name: "Own".into(),
        enemies: vec![hostile_faction],
        compliance: None,
    });

    let armed = |z: f32, half: f32| AiWorldEntity {
        uuid: uuid::Uuid::new_v4(),
        position: [0.0, 0.0, z],
        faction: Some(hostile_faction),
        // Bearing 0 from either hostile astern of us points straight at us.
        weapon_arcs: vec![crate::weapons::arc_geometry::WeaponArcSector {
            bearing_deg: 0.0,
            half_angle_deg: half,
            range: 500.0,
        }],
        ..Default::default()
    };

    // Nearest hostile is the narrow one, so it owns `escape_offset_deg` —
    // and the far all-round hull must still veto it.
    let view = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        self_faction: Some(own_faction),
        entities: vec![armed(50.0, 30.0), armed(200.0, 180.0)],
        ..Default::default()
    };
    let e = hostile_arc_exposure(&view, &registry);
    assert_eq!(e.covering_count, 2, "{e:?}");
    assert!(e.inescapable, "{e:?}");
    assert_eq!(e.escape_offset_deg, 0.0, "{e:?}");

    // Drop the all-round hull and the nearest one's real exit comes back.
    let escapable = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        self_faction: Some(own_faction),
        entities: vec![armed(50.0, 30.0)],
        ..Default::default()
    };
    let e = hostile_arc_exposure(&escapable, &registry);
    assert_eq!(e.covering_count, 1, "{e:?}");
    assert!(!e.inescapable, "{e:?}");
    assert!(e.escape_offset_deg.abs() > 0.0, "{e:?}");
}

// ── steer_toward ──────────────────────────────────────────────────────

#[test]
fn steer_toward_returns_zero_within_deadband() {
    // Forward yaw = 0 → forward direction = (0, -1) in XZ.
    // Target slightly ahead → nearly zero error, within deadband.
    let result = steer_toward(0.0, [0.0, -1.0], PATROL_DEADBAND_RAD, PATROL_FULL_STEER_RAD);
    assert_eq!(result, 0.0);
}

#[test]
fn steer_toward_positive_for_target_to_right() {
    // Yaw = 0, forward = (0, -1). Target at (1, 0) = to the right.
    let dir = [1.0_f32, 0.0_f32];
    let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
    let unit = [dir[0] / len, dir[1] / len];
    let result = steer_toward(0.0, unit, 0.0, PATROL_FULL_STEER_RAD);
    assert!(
        result > 0.0,
        "target to the right must give positive steering"
    );
}

#[test]
fn steer_toward_negative_for_target_to_left() {
    let dir = [-1.0_f32, 0.0_f32];
    let len = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
    let unit = [dir[0] / len, dir[1] / len];
    let result = steer_toward(0.0, unit, 0.0, PATROL_FULL_STEER_RAD);
    assert!(
        result < 0.0,
        "target to the left must give negative steering"
    );
}

#[test]
fn steer_toward_saturates_at_one() {
    // Target directly to the right (90°) saturates.
    let result = steer_toward(0.0, [1.0, 0.0], 0.0, PATROL_FULL_STEER_RAD);
    assert!(result >= 1.0 || result <= -1.0 || result.abs() <= 1.0);
    assert!(result.abs() <= 1.0, "steering must be clamped to [-1, 1]");
}

// ── should_emit ───────────────────────────────────────────────────────

#[test]
fn should_emit_returns_false_when_within_epsilon() {
    assert!(!should_emit(0.5, 0.5 + 0.001, 0.01));
}

#[test]
fn should_emit_returns_true_when_outside_epsilon() {
    assert!(should_emit(0.0, 0.5, 0.01));
}

#[test]
fn should_emit_returns_false_when_equal() {
    assert!(!should_emit(0.3, 0.3, 0.0));
}

// ── visible_entities ──────────────────────────────────────────────────

#[test]
fn visible_entities_includes_in_range_entity() {
    let near = AiWorldEntity {
        uuid: Uuid::from_u128(1),
        position: [10.0, 0.0, 0.0],
        ..Default::default()
    };
    let result = visible_entities([0.0, 0.0, 0.0], 20.0, std::slice::from_ref(&near));
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].uuid, near.uuid);
}

#[test]
fn visible_entities_excludes_out_of_range_entity() {
    let far = AiWorldEntity {
        uuid: Uuid::from_u128(1),
        position: [100.0, 0.0, 0.0],
        ..Default::default()
    };
    let result = visible_entities([0.0, 0.0, 0.0], 20.0, &[far]);
    assert!(result.is_empty());
}

#[test]
fn visible_entities_includes_entity_exactly_at_boundary() {
    let boundary = AiWorldEntity {
        uuid: Uuid::from_u128(1),
        position: [20.0, 0.0, 0.0],
        ..Default::default()
    };
    let result = visible_entities([0.0, 0.0, 0.0], 20.0, &[boundary]);
    assert_eq!(result.len(), 1, "entity exactly at range must be included");
}

#[test]
fn visible_entities_ignores_y_component() {
    // Same XZ position, wildly different Y — should still be in range.
    let above = AiWorldEntity {
        uuid: Uuid::from_u128(1),
        position: [5.0, 500.0, 0.0],
        ..Default::default()
    };
    let result = visible_entities([0.0, 0.0, 0.0], 20.0, &[above]);
    assert_eq!(result.len(), 1);
}

#[test]
fn visible_entities_unlimited_when_range_zero() {
    let far = AiWorldEntity {
        uuid: Uuid::from_u128(1),
        position: [10_000.0, 0.0, 0.0],
        ..Default::default()
    };
    let result = visible_entities([0.0, 0.0, 0.0], 0.0, &[far]);
    assert_eq!(result.len(), 1, "range <= 0 must mean unlimited");
}

#[test]
fn visible_entities_unlimited_when_range_negative() {
    let far = AiWorldEntity {
        uuid: Uuid::from_u128(1),
        position: [10_000.0, 0.0, 0.0],
        ..Default::default()
    };
    let result = visible_entities([0.0, 0.0, 0.0], -5.0, &[far]);
    assert_eq!(result.len(), 1, "negative range must mean unlimited");
}

#[test]
fn visible_entities_unlimited_when_range_nan() {
    let far = AiWorldEntity {
        uuid: Uuid::from_u128(1),
        position: [10_000.0, 0.0, 0.0],
        ..Default::default()
    };
    let result = visible_entities([0.0, 0.0, 0.0], f32::NAN, &[far]);
    assert_eq!(result.len(), 1, "NaN range must mean unlimited");
}

#[test]
fn visible_entities_unlimited_when_range_infinite() {
    let far = AiWorldEntity {
        uuid: Uuid::from_u128(1),
        position: [10_000.0, 0.0, 0.0],
        ..Default::default()
    };
    let result = visible_entities([0.0, 0.0, 0.0], f32::INFINITY, &[far]);
    assert_eq!(result.len(), 1, "infinite range must mean unlimited");
}

#[test]
fn avoidance_steering_is_zero_when_stationary() {
    let obstacle = AiWorldEntity {
        uuid: Uuid::from_u128(2),
        position: [0.0, 0.0, -2.0],
        radius: 20.0,
        ..Default::default()
    };

    let steering = avoidance_steering(
        [0.0, 0.0, 0.0],
        0.0,
        0.0,
        2.0,
        Uuid::nil(),
        &[obstacle],
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
    );

    assert_eq!(
        steering, 0.0,
        "stationary ships should not yaw away from nearby bodies"
    );
}

// ── operate_helm patrol ───────────────────────────────────────────────

fn patrol_pool() -> Vec<crate::core::messages::ScoredObjective> {
    vec![crate::core::messages::ScoredObjective {
        id: "patrol".into(),
        score: 20.0,
        directive: crate::core::messages::AiDirective::Patrol {
            anchors: vec!["alpha".into()],
            loop_path: true,
        },
        source: crate::core::messages::ObjectiveSource::Doctrine,
        relevance: vec![crate::core::messages::SystemAffinity::Helm],
        snapshot: crate::core::messages::ObjectiveSnapshot {
            id: "patrol".into(),
            text: "Patrol".into(),
            text_params: Default::default(),
            mandatory: false,
            status: crate::core::messages::ObjectiveStatus::Active,
            targets: vec![],
            source: crate::core::messages::ObjectiveSource::Doctrine,
        },
    }]
}

fn patrol_doctrine() -> Vec<crate::entities::config::DoctrineObjective> {
    vec![crate::entities::config::DoctrineObjective {
        id: "patrol".into(),
        text: "Patrol".into(),
        directive_kind: Some("Patrol".into()),
        directive_anchors: vec!["alpha".into()],
        directive_loop: true,
        base_priority: 20.0,
        target_speed: 0.5,
        ..Default::default()
    }]
}

/// A two-waypoint, non-looping Patrol over `wp0` then `wp1`, with matching
/// doctrine (`target_speed` 0.5). The caller supplies the anchor positions,
/// so the same route can be posed as "arrived", "en route" or "terminal".
fn two_waypoint_patrol() -> (
    Vec<crate::core::messages::ScoredObjective>,
    Vec<crate::entities::config::DoctrineObjective>,
) {
    let pool = vec![crate::core::messages::ScoredObjective {
        id: "patrol".into(),
        score: 20.0,
        directive: crate::core::messages::AiDirective::Patrol {
            anchors: vec!["wp0".into(), "wp1".into()],
            loop_path: false,
        },
        source: crate::core::messages::ObjectiveSource::Doctrine,
        relevance: vec![crate::core::messages::SystemAffinity::Helm],
        snapshot: crate::core::messages::ObjectiveSnapshot {
            id: "patrol".into(),
            text: "".into(),
            text_params: Default::default(),
            mandatory: false,
            status: crate::core::messages::ObjectiveStatus::Active,
            targets: vec![],
            source: crate::core::messages::ObjectiveSource::Doctrine,
        },
    }];
    let doctrine = vec![crate::entities::config::DoctrineObjective {
        id: "patrol".into(),
        text: "".into(),
        directive_kind: Some("Patrol".into()),
        directive_anchors: vec!["wp0".into(), "wp1".into()],
        directive_loop: false,
        target_speed: 0.5,
        ..Default::default()
    }];
    (pool, doctrine)
}

/// A single Reach objective naming `anchor`, scored `score`.
fn reach_pool(anchor: &str, score: f32) -> Vec<crate::core::messages::ScoredObjective> {
    vec![crate::core::messages::ScoredObjective {
        id: "reach".into(),
        score,
        directive: crate::core::messages::AiDirective::Reach {
            anchor: anchor.into(),
        },
        source: crate::core::messages::ObjectiveSource::Doctrine,
        relevance: vec![crate::core::messages::SystemAffinity::Helm],
        snapshot: crate::core::messages::ObjectiveSnapshot {
            id: "reach".into(),
            text: "".into(),
            text_params: Default::default(),
            mandatory: false,
            status: crate::core::messages::ObjectiveStatus::Active,
            targets: vec![],
            source: crate::core::messages::ObjectiveSource::Doctrine,
        },
    }]
}

fn anchors_with_alpha() -> std::collections::HashMap<String, [f32; 3]> {
    let mut m = std::collections::HashMap::new();
    m.insert("alpha".into(), [100.0, 0.0, 0.0]);
    m
}

fn world_at_origin() -> WorldView {
    WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        ..Default::default()
    }
}

#[test]
fn operate_helm_patrol_generates_nonzero_steering_toward_waypoint() {
    let world = world_at_origin();
    let pool = patrol_pool();
    let doctrine = patrol_doctrine();
    let anchors = anchors_with_alpha();

    let (thrust, _steering) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &anchors,
        NO_CURSORS,
        None,
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(thrust > 0.0, "should thrust toward waypoint");
}

#[test]
fn operate_helm_empty_pool_returns_zero() {
    let world = world_at_origin();
    let (thrust, steering) = plan_helm_travel(
        &world,
        &[],
        &[],
        &std::collections::HashMap::new(),
        NO_CURSORS,
        None,
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert_eq!(thrust, 0.0);
    assert_eq!(steering, 0.0);
}

#[test]
fn operate_helm_zeroed_pool_returns_zero() {
    let world = world_at_origin();
    let mut pool = patrol_pool();
    pool[0].score = 0.0; // zero-gated
    let doctrine = patrol_doctrine();
    let anchors = anchors_with_alpha();

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &anchors,
        NO_CURSORS,
        None,
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert_eq!(thrust, 0.0, "zero-gated pool must produce no thrust");
    assert_eq!(steering, 0.0);
}

/// `helm_patrol` steers toward the waypoint the *cursor* names, not
/// blindly toward the route's first anchor (issue #702).
///
/// This is the core of the `waypoint_index` migration. `operate_helm` used
/// to own a private `AiMemory.waypoint_index` and advance it itself, giving
/// the high-LOD helm a second cursor that could disagree with the
/// `ObjectiveCursors` the low-LOD path and the scenario triggers used.
/// There is now one cursor and the helm reads it.
#[test]
fn operate_helm_patrol_steers_to_the_cursors_waypoint() {
    let mut anchors = std::collections::HashMap::new();
    // wp0 is dead ahead (negative Z at yaw 0); wp1 is to starboard.
    anchors.insert("wp0".into(), [0.0, 0.0, -100.0]);
    anchors.insert("wp1".into(), [100.0, 0.0, 0.0]);
    let (pool, doctrine) = two_waypoint_patrol();
    let world = world_at_origin();

    // A cursor sitting on index 1 must produce a turn to starboard.
    let mut cursor = crate::ai::patrol_cursor::PatrolCursor::new("patrol");
    crate::ai::patrol_cursor::advance_cursor(
        &mut cursor,
        &["wp0".to_string(), "wp1".to_string()],
        false,
        [0.0, 0.0, -100.0], // sitting on wp0 → the cursor advances to wp1
        &anchors,
        WAYPOINT_ARRIVAL_RADIUS,
    );
    assert_eq!(cursor.index(), 1, "precondition: cursor must be on wp1");

    let (_thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &anchors,
        std::slice::from_ref(&cursor),
        None,
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(
        steering > 0.0,
        "the helm must steer toward the cursor's waypoint (wp1, to starboard),              not the route's first anchor (wp0, dead ahead, which would steer ~0);              got steering={steering}"
    );
}

/// On the arrival tick the helm flies straight through rather than
/// advancing anything itself — `advance_objective_cursors`
/// (`SimSet::Modifiers`) owns advancement, and lands it later the same
/// tick. This is why moving the cursor out of `operate_helm` is benign:
/// the tick the cursor moves was already a zero-steering tick.
#[test]
fn operate_helm_patrol_flies_straight_through_on_arrival() {
    let mut anchors = std::collections::HashMap::new();
    anchors.insert("wp0".into(), [0.0, 0.0, 0.0]); // at origin = arrived
    anchors.insert("wp1".into(), [100.0, 0.0, 0.0]);
    let (pool, doctrine) = two_waypoint_patrol();
    let world = world_at_origin();

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &anchors,
        NO_CURSORS,
        None,
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert_eq!(
        (thrust, steering),
        (0.5, 0.0),
        "arrival tick must hold course at the doctrine's target_speed"
    );
}

/// A non-looping route walked past its final waypoint is resolved
/// idleness — hold station rather than falling through to a lower-priority
/// directive.
#[test]
fn operate_helm_patrol_terminal_stop_holds_station() {
    let mut anchors = std::collections::HashMap::new();
    anchors.insert("wp0".into(), [0.0, 0.0, -100.0]);
    anchors.insert("wp1".into(), [100.0, 0.0, 0.0]);
    let (mut pool, doctrine) = two_waypoint_patrol();
    // Add a resolvable lower-priority Reach; a terminal Patrol must not
    // fall through to it.
    pool.extend(reach_pool("wp1", 1.0));
    let world = world_at_origin();

    let mut cursor = crate::ai::patrol_cursor::PatrolCursor::new("patrol");
    // Walk the cursor off the end of the non-looping route.
    for pos in [[0.0, 0.0, -100.0], [100.0, 0.0, 0.0]] {
        crate::ai::patrol_cursor::advance_cursor(
            &mut cursor,
            &["wp0".to_string(), "wp1".to_string()],
            false,
            pos,
            &anchors,
            WAYPOINT_ARRIVAL_RADIUS,
        );
    }
    assert!(cursor.index() >= 2, "precondition: cursor must be terminal");

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &anchors,
        std::slice::from_ref(&cursor),
        None,
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert_eq!(
        (thrust, steering),
        (0.0, 0.0),
        "a finished non-looping patrol holds station; it must not fall              through to the lower-priority Reach"
    );
}

// ── operate_helm fallback ─────────────────────────────────────────────

/// Build a scored pool with Destroy (high score, unresolvable target) first,
/// then Patrol (lower score, resolvable anchor) second.
fn destroy_then_patrol_pool(
    anchors: &std::collections::HashMap<String, [f32; 3]>,
) -> Vec<crate::core::messages::ScoredObjective> {
    let _ = anchors; // anchors used externally; pool just carries names
    vec![
        crate::core::messages::ScoredObjective {
            id: "destroy-wave-1".into(),
            score: 90.0,
            directive: crate::core::messages::AiDirective::Destroy {
                target: "wave_1".into(), // entity not in world_view → unresolvable
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![
                crate::core::messages::SystemAffinity::Helm,
                crate::core::messages::SystemAffinity::Weapons,
                crate::core::messages::SystemAffinity::Captain,
            ],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "destroy-wave-1".into(),
                text: "Destroy wave 1".into(),
                text_params: Default::default(),
                mandatory: true,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec!["wave_1".into()],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        },
        crate::core::messages::ScoredObjective {
            id: "patrol-base".into(),
            score: 30.0,
            directive: crate::core::messages::AiDirective::Patrol {
                anchors: vec!["alpha".into()],
                loop_path: true,
            },
            source: crate::core::messages::ObjectiveSource::Mission,
            relevance: vec![crate::core::messages::SystemAffinity::Helm],
            snapshot: crate::core::messages::ObjectiveSnapshot {
                id: "patrol-base".into(),
                text: "Patrol".into(),
                text_params: Default::default(),
                mandatory: true,
                status: crate::core::messages::ObjectiveStatus::Active,
                targets: vec![],
                source: crate::core::messages::ObjectiveSource::Mission,
            },
        },
    ]
}

#[test]
fn operate_helm_falls_through_unresolvable_destroy_to_patrol() {
    // Regression: when the top Destroy directive has no valid target in the
    // world snapshot (entity not yet spawned / not in WorldSnapshot),
    // operate_helm must fall through to the next lower-priority directive
    // (Patrol) rather than leaving the ship idle.  Matches the
    // combat_test.toml scenario where wave objectives are added on the same
    // tick as the entities spawn, before the WorldSnapshot is rebuilt.
    let world = world_at_origin(); // entities list is empty → wave_1 not found
    let anchors = anchors_with_alpha();
    let pool = destroy_then_patrol_pool(&anchors);

    let (thrust, _steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &anchors,
        NO_CURSORS,
        None,
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(
        thrust > 0.0,
        "should fall through to Patrol and produce thrust when Destroy target is unresolvable"
    );
}

/// The Helm pursues the ship's `TacticalRadarSelection` and does not fall through to
/// Patrol (issue #702).
///
/// This is the `target` migration in one test. `operate_helm` used to
/// resolve the Destroy directive's authored name itself, via a private
/// four-tier `resolve_destroy_target` (explicit → current → last_attacker →
/// nearest) over its own radar horizon — the same four tiers
/// `ai_target_selection` runs over Tactical's. Two selectors, two horizons,
/// so a ship could close on one ship while shooting another. Now Tactical
/// selects and the Helm reads the selection.
///
/// Geometry: the Destroy target sits dead ahead and the Patrol anchor to
/// starboard, so "which directive won" is legible from the steering alone —
/// ~0 means Destroy, positive means it fell through to Patrol.
#[test]
fn operate_helm_destroy_pursues_the_weapons_target() {
    let (world, pool, anchors, target_uuid) = destroy_vs_patrol_scene();

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &anchors,
        NO_CURSORS,
        Some(target_uuid),
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );

    assert!(thrust > 0.0, "should close on the target");
    assert!(
        steering.abs() < PATROL_DEADBAND_RAD,
        "the helm must pursue the TacticalRadarSelection (dead ahead → ~0 steering),              not fall through to Patrol (to starboard → positive steering);              got steering={steering}"
    );
}

/// The converse, and the reason the Helm may not acquire on its own: with
/// Tactical holding no lock, a Destroy directive resolves to nobody and
/// falls through — *even though* a perfectly good hostile is sitting in the
/// Helm's world view. A Helm that scanned for its own target would pursue
/// it and diverge from what Tactical is shooting.
#[test]
fn operate_helm_destroy_without_a_weapons_target_falls_through() {
    let (world, pool, anchors, _target_uuid) = destroy_vs_patrol_scene();

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &anchors,
        NO_CURSORS,
        None, // Tactical has locked nothing
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );

    assert!(
        thrust > 0.0,
        "should fall through to Patrol and keep flying"
    );
    assert!(
        steering > 0.0,
        "with no Tactical lock the Destroy directive resolves to nobody and              must fall through to Patrol (to starboard → positive steering); a              ~0 steering means the helm acquired the visible hostile itself,              which is the divergence #702 removed; got steering={steering}"
    );
}

/// The starbase-assault bug. Tactical holds no lock — the target is over the
/// horizon, or factionless and never auto-acquired — but Navigation *has*
/// cleared a waypoint to it. The Destroy directive must consume that
/// waypoint at its own priority, not fall through to the lower-scored
/// Patrol. Previously Patrol resolved and returned first, so a raider
/// ordered to assault the starbase flew its patrol circuit instead and the
/// waypoint fallback at the end of `operate_helm` was never reached.
#[test]
fn operate_helm_destroy_without_a_weapons_target_flies_the_nav_waypoint() {
    let (world, pool, anchors, _target_uuid) = destroy_vs_patrol_scene();

    // Patrol anchor alpha is at [100, 0, 0] — to starboard. Put Navigation's
    // waypoint to *port* so the two are unambiguously distinguishable.
    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &anchors,
        NO_CURSORS,
        None,                // Tactical has locked nothing
        Some([-100.0, 0.0]), // but Navigation has cleared a waypoint
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );

    assert!(thrust > 0.0, "should be under way toward the waypoint");
    assert!(
        steering < 0.0,
        "must steer to port toward Navigation's waypoint; positive steering              means it fell through to the starboard Patrol anchor, which is the              bug — got steering={steering}"
    );
}

/// The fallback is conditional, not a takeover: a Destroy that resolves
/// neither a target nor a waypoint still yields to Patrol.
#[test]
fn operate_helm_destroy_still_yields_to_patrol_without_a_nav_waypoint() {
    let (world, pool, anchors, _target_uuid) = destroy_vs_patrol_scene();

    let (_thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &anchors,
        NO_CURSORS,
        None,
        None, // no lock and no waypoint
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );

    assert!(
        steering > 0.0,
        "with nothing to close on, Destroy must still fall through to the              starboard Patrol anchor; got steering={steering}"
    );
}

/// A Tactical lock the Helm's own radar cannot see is not pursuable: the
/// directive falls through rather than flying at a bearing it cannot
/// confirm. (`world_view.entities` is already radar-filtered by the caller.)
#[test]
fn operate_helm_destroy_ignores_a_target_outside_its_world_view() {
    let (world, pool, anchors, _target_uuid) = destroy_vs_patrol_scene();

    let (_thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &anchors,
        NO_CURSORS,
        Some(Uuid::new_v4()), // locked, but not in the world view
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(
        steering > 0.0,
        "an invisible target must fall through to Patrol, not steer at a              bearing the helm cannot confirm; got steering={steering}"
    );
}

/// A scene where Destroy and Patrol are told apart by steering alone:
/// the Destroy target is dead ahead (yaw 0 → forward is -Z), the Patrol
/// anchor `alpha` is to starboard. Destroy outscores Patrol.
///
/// Returns `(world, pool, anchors, target_uuid)`.
#[allow(clippy::type_complexity)]
fn destroy_vs_patrol_scene() -> (
    WorldView,
    Vec<crate::core::messages::ScoredObjective>,
    std::collections::HashMap<String, [f32; 3]>,
    Uuid,
) {
    let target_uuid = Uuid::new_v4();
    let anchors = anchors_with_alpha(); // alpha = [100, 0, 0], to starboard
    let world = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        entities: vec![crate::ai::AiWorldEntity {
            uuid: target_uuid,
            name: Some("wave_1".into()),
            position: [0.0, 0.0, -200.0], // dead ahead
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut pool = vec![crate::core::messages::ScoredObjective {
        id: "destroy-wave-1".into(),
        score: 90.0,
        directive: crate::core::messages::AiDirective::Destroy {
            target: "wave_1".into(),
        },
        source: crate::core::messages::ObjectiveSource::Mission,
        relevance: vec![
            crate::core::messages::SystemAffinity::Helm,
            crate::core::messages::SystemAffinity::Weapons,
            crate::core::messages::SystemAffinity::Captain,
        ],
        snapshot: crate::core::messages::ObjectiveSnapshot {
            id: "destroy-wave-1".into(),
            text: "Destroy".into(),
            text_params: Default::default(),
            mandatory: true,
            status: crate::core::messages::ObjectiveStatus::Active,
            targets: vec![],
            source: crate::core::messages::ObjectiveSource::Mission,
        },
    }];
    pool.extend(patrol_pool()); // score 20 — below Destroy
    (world, pool, anchors, target_uuid)
}

// ── operate_helm Retreat ──────────────────────────────────────────────

/// Build a single-objective Retreat scored pool naming `anchor`.
fn retreat_pool(anchor: &str, score: f32) -> Vec<crate::core::messages::ScoredObjective> {
    vec![crate::core::messages::ScoredObjective {
        id: "retreat".into(),
        score,
        directive: crate::core::messages::AiDirective::Retreat {
            anchor: anchor.into(),
        },
        source: crate::core::messages::ObjectiveSource::Doctrine,
        relevance: vec![crate::core::messages::SystemAffinity::Helm],
        snapshot: crate::core::messages::ObjectiveSnapshot {
            id: "retreat".into(),
            text: "Retreat".into(),
            text_params: Default::default(),
            mandatory: false,
            status: crate::core::messages::ObjectiveStatus::Active,
            targets: vec![],
            source: crate::core::messages::ObjectiveSource::Doctrine,
        },
    }]
}

#[test]
fn operate_helm_retreat_steers_toward_valid_anchor() {
    // A Retreat directive with a known anchor name must steer toward that
    // anchor, mirroring the Reach directive. Anchor "rally" is at
    // [100, 0, 0] — to the right of a ship at origin facing yaw 0
    // (forward = (0, -1)), so steering must be positive (see
    // steer_toward_positive_for_target_to_right).
    let world = world_at_origin();
    let mut anchors = std::collections::HashMap::new();
    anchors.insert("rally".to_string(), [100.0, 0.0, 0.0]);
    let pool = retreat_pool("rally", 50.0);

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &anchors,
        NO_CURSORS,
        None,
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(thrust > 0.0, "Retreat should thrust toward the anchor");
    assert!(
        steering > 0.0,
        "Retreat anchor to the right must give positive steering"
    );
}

/// The other side of `operate_helm_retreat_steers_toward_valid_anchor`: a
/// Retreat naming an anchor the world does not declare resolves to nowhere
/// and falls through to the next directive, exactly as `Reach` does.
///
/// This test used to assert the opposite — that an empty anchor fell back to
/// `AiMemory.home_position` — because a *synthetic* hull-triggered Retreat
/// injected by `aggregate_doctrine_blackboards` always carried an empty
/// anchor and needed somewhere to go. #702 deleted that injector and the
/// `home_position` it leaned on. The old fallback was never the safety net
/// it looked like: `home_position` was never seeded in production, so it was
/// world origin, and "retreat" meant "fly to [0,0,0]" for every shipped
/// ship. Falling through to Patrol is both the honest answer and the useful
/// one. Retreat is now authored doctrine with a real anchor.
#[test]
fn operate_helm_retreat_with_unknown_anchor_falls_through() {
    let world = world_at_origin();
    // "alpha" is known and Patrol wants it; the Retreat anchor is not.
    let anchors = anchors_with_alpha();
    let mut pool = retreat_pool("nowhere-in-particular", 50.0);
    pool.extend(patrol_pool());
    let doctrine = patrol_doctrine();

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &anchors,
        NO_CURSORS,
        None,
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );

    // Patrol's "alpha" is at [100, 0, 0] — to the right of a ship at origin
    // at yaw 0, so a positive steering means we are flying the *Patrol*, not
    // idling and not retreating to a phantom origin.
    assert!(
        thrust > 0.0 && steering > 0.0,
        "an unresolvable Retreat must fall through to the next directive              (here Patrol toward `alpha`), not resolve to a fabricated position;              got thrust={thrust}, steering={steering}"
    );
}

/// And with nothing to fall through *to*, an unresolvable Retreat is idle —
/// not a flight to world origin.
#[test]
fn operate_helm_lone_unresolvable_retreat_is_idle() {
    let world = world_at_origin();
    let anchors = std::collections::HashMap::new();
    let pool = retreat_pool("", 50.0);

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &anchors,
        NO_CURSORS,
        None,
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert_eq!(
        (thrust, steering),
        (0.0, 0.0),
        "a Retreat that names nowhere is a Retreat to nowhere"
    );
}

// ── helm_destroy proportional approach ────────────────────────────────

/// Build a minimal Destroy scored pool that targets `uuid` with
/// `target_speed` and `maintain_range` taken from matching doctrine.
fn destroy_pool_for(
    target_name: &str,
    target_speed: f32,
    maintain_range: f32,
) -> (
    Vec<crate::core::messages::ScoredObjective>,
    Vec<crate::entities::config::DoctrineObjective>,
) {
    let pool = vec![crate::core::messages::ScoredObjective {
        id: "destroy-target".into(),
        score: 50.0,
        directive: crate::core::messages::AiDirective::Destroy {
            target: target_name.into(),
        },
        source: crate::core::messages::ObjectiveSource::Doctrine,
        relevance: vec![
            crate::core::messages::SystemAffinity::Helm,
            crate::core::messages::SystemAffinity::Weapons,
        ],
        snapshot: crate::core::messages::ObjectiveSnapshot {
            id: "destroy-target".into(),
            text: "".into(),
            text_params: Default::default(),
            mandatory: false,
            status: crate::core::messages::ObjectiveStatus::Active,
            targets: vec![target_name.into()],
            source: crate::core::messages::ObjectiveSource::Doctrine,
        },
    }];
    let doctrine = vec![crate::entities::config::DoctrineObjective {
        id: "destroy-target".into(),
        text: "".into(),
        directive_kind: Some("Destroy".into()),
        target_speed,
        maintain_range,
        ..Default::default()
    }];
    (pool, doctrine)
}

#[test]
fn helm_destroy_full_thrust_far_from_target() {
    // Ship is far beyond the decel zone — should emit full target_speed thrust.
    let target_uuid = Uuid::new_v4();
    let target_speed = 0.8_f32;
    let maintain_range = 25.0_f32;
    // stop_dist = 25 * 0.8 = 20; decel_start = 20 * 1.5 = 30; place at 100
    let world = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        entities: vec![AiWorldEntity {
            uuid: target_uuid,
            name: Some("enemy".into()),
            position: [0.0, 0.0, -100.0],
            ..Default::default()
        }],
        ..Default::default()
    };
    let (pool, doctrine) = destroy_pool_for("enemy", target_speed, maintain_range);
    let (thrust, _) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &std::collections::HashMap::new(),
        NO_CURSORS,
        Some(target_uuid), // Tactical's lock — what the helm pursues (#702)
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(
        (thrust - target_speed).abs() < 1e-4,
        "beyond decel zone: expected full thrust {target_speed}, got {thrust}"
    );
}

#[test]
fn helm_destroy_reduced_thrust_inside_decel_zone() {
    // Ship is halfway between decel_start and stop_dist — thrust should be
    // roughly half of target_speed (proportional ramp).
    let target_uuid = Uuid::new_v4();
    let target_speed = 0.8_f32;
    let maintain_range = 25.0_f32;
    // Surface stop distance = 20, decel_start = 30; the centre at 25 is the
    // midpoint clearance for this point-sized target.
    let world = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        entities: vec![AiWorldEntity {
            uuid: target_uuid,
            name: Some("enemy".into()),
            position: [0.0, 0.0, -25.0], // surface distance = 25
            ..Default::default()
        }],
        ..Default::default()
    };
    let (pool, doctrine) = destroy_pool_for("enemy", target_speed, maintain_range);
    let (thrust, _) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &std::collections::HashMap::new(),
        NO_CURSORS,
        Some(target_uuid), // Tactical's lock — what the helm pursues (#702)
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    // At dist=25, t = (25-20)/(30-20) = 0.5 → expected thrust = 0.4
    let expected = target_speed * 0.5;
    assert!(
        (thrust - expected).abs() < 0.01,
        "inside decel zone (midpoint): expected ~{expected}, got {thrust}"
    );
}

#[test]
fn helm_destroy_zero_thrust_at_station() {
    // Ship is inside stop_dist — thrust must be exactly 0.
    let target_uuid = Uuid::new_v4();
    let maintain_range = 25.0_f32;
    // stop_dist = 20; place at 10 (inside)
    let world = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        entities: vec![AiWorldEntity {
            uuid: target_uuid,
            name: Some("enemy".into()),
            position: [0.0, 0.0, -10.0], // dist = 10
            ..Default::default()
        }],
        ..Default::default()
    };
    let (pool, doctrine) = destroy_pool_for("enemy", 0.8, maintain_range);
    let (thrust, _) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &std::collections::HashMap::new(),
        NO_CURSORS,
        Some(target_uuid), // Tactical's lock — what the helm pursues (#702)
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert_eq!(thrust, 0.0, "inside stop_dist: thrust must be 0");
}

#[test]
fn helm_destroy_holding_station_does_not_avoid_destroy_target() {
    // While holding weapons range, the active Destroy target is the thing
    // the ship intentionally faces. Treating that same target as an
    // avoidance obstacle makes a stationary AI ship yaw away from a nearby
    // enemy even when already lined up.
    let target_uuid = Uuid::new_v4();
    let world = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        entities: vec![AiWorldEntity {
            uuid: target_uuid,
            name: Some("enemy".into()),
            position: [0.0, 0.0, -10.0],
            radius: 20.0,
            ..Default::default()
        }],
        ..Default::default()
    };
    let (pool, doctrine) = destroy_pool_for("enemy", 0.8, 25.0);

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &std::collections::HashMap::new(),
        NO_CURSORS,
        Some(target_uuid), // Tactical's lock — what the helm pursues (#702)
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );

    assert_eq!(thrust, 0.0, "inside stop_dist: thrust must be 0");
    assert_eq!(
        steering, 0.0,
        "active destroy target must not push avoidance steering while holding station"
    );
}

#[test]
fn helm_destroy_holding_station_does_not_fall_through_to_patrol() {
    let target_uuid = Uuid::new_v4();
    let world = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        entities: vec![AiWorldEntity {
            uuid: target_uuid,
            name: Some("enemy".into()),
            position: [0.0, 0.0, -10.0],
            ..Default::default()
        }],
        ..Default::default()
    };
    let (mut pool, doctrine) = destroy_pool_for("enemy", 0.8, 25.0);
    pool.push(crate::core::messages::ScoredObjective {
        id: "patrol-base".into(),
        score: 10.0,
        directive: crate::core::messages::AiDirective::Patrol {
            anchors: vec!["alpha".into()],
            loop_path: true,
        },
        source: crate::core::messages::ObjectiveSource::Doctrine,
        relevance: vec![crate::core::messages::SystemAffinity::Helm],
        snapshot: crate::core::messages::ObjectiveSnapshot {
            id: "patrol-base".into(),
            text: "Patrol".into(),
            text_params: Default::default(),
            mandatory: false,
            status: crate::core::messages::ObjectiveStatus::Active,
            targets: vec![],
            source: crate::core::messages::ObjectiveSource::Doctrine,
        },
    });
    let anchors = anchors_with_alpha();

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &anchors,
        NO_CURSORS,
        Some(target_uuid), // Tactical's lock - what the helm pursues (#702)
        None,
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );

    assert_eq!(thrust, 0.0);
    assert_eq!(
        steering, 0.0,
        "resolved Destroy should hold station instead of falling through to Patrol"
    );
}

// ── score_doctrine_pool ───────────────────────────────────────────────

#[test]
fn score_doctrine_pool_patrol_always_scores() {
    use crate::entities::config::DoctrineObjective;
    use crate::objectives::WorldConditions;

    let doctrine = vec![DoctrineObjective {
        id: "patrol".into(),
        text: "Patrol".into(),
        directive_kind: Some("Patrol".into()),
        base_priority: 20.0,
        ..Default::default()
    }];
    let cond = WorldConditions {
        red_alert: false,
        hull_fraction: 1.0,
        attacked: false,
    };
    let pool = score_doctrine_pool(&doctrine, &cond);
    assert_eq!(pool.len(), 1);
    assert!((pool[0].score - 20.0).abs() < 1e-5);
}

#[test]
fn score_doctrine_pool_zero_gate_vetoes_destroy() {
    use crate::entities::config::DoctrineObjective;
    use crate::objectives::{WorldConditions, ZeroGateCondition};

    // Zero gate: hull must be below 0.3 (but hull = 1.0 → gate fails → score 0)
    let doctrine = vec![DoctrineObjective {
        id: "flee".into(),
        text: "Flee".into(),
        directive_kind: Some("Reach".into()),
        base_priority: 50.0,
        zero_gates: vec![ZeroGateCondition {
            condition: "hull_below".into(),
            threshold: Some(0.3),
        }],
        ..Default::default()
    }];
    let cond = WorldConditions {
        red_alert: false,
        hull_fraction: 1.0,
        attacked: false,
    };
    let pool = score_doctrine_pool(&doctrine, &cond);
    assert_eq!(pool[0].score, 0.0, "zero-gate must veto at full hull");
}

#[test]
fn score_doctrine_pool_sorted_descending_by_score() {
    use crate::entities::config::DoctrineObjective;
    use crate::objectives::WorldConditions;

    let doctrine = vec![
        DoctrineObjective {
            id: "a".into(),
            text: "A".into(),
            base_priority: 10.0,
            ..Default::default()
        },
        DoctrineObjective {
            id: "b".into(),
            text: "B".into(),
            base_priority: 35.0,
            ..Default::default()
        },
        DoctrineObjective {
            id: "c".into(),
            text: "C".into(),
            base_priority: 20.0,
            ..Default::default()
        },
    ];
    let cond = WorldConditions {
        red_alert: false,
        hull_fraction: 1.0,
        attacked: false,
    };
    let pool = score_doctrine_pool(&doctrine, &cond);
    assert_eq!(pool[0].id, "b");
    assert_eq!(pool[1].id, "c");
    assert_eq!(pool[2].id, "a");
}

// ── decide_impulse ────────────────────────────────────────────────────

fn impulse_input(
    pos: [f32; 2],
    yaw: f32,
    target_pos: [f32; 3],
    phase: crate::ship::impulse::ImpulsePhase,
    engage_dist: f32,
    cancel_dist: f32,
) -> ImpulseDecisionInput {
    ImpulseDecisionInput {
        pos,
        yaw,
        target_pos,
        phase,
        engage_distance: engage_dist,
        cancel_distance: cancel_dist,
        angle_tolerance: IMPULSE_ANGLE_TOLERANCE_RAD,
    }
}

#[test]
fn impulse_decide_engage_when_ahead_and_far() {
    // Ship at (0,0) facing -Z (yaw=0), target at (0, 0, -300)
    let input = impulse_input(
        [0.0, 0.0],
        0.0,                // yaw = 0 → facing -Z
        [0.0, 0.0, -300.0], // target 300 units ahead
        crate::ship::impulse::ImpulsePhase::Idle,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::Engage);
}

#[test]
fn impulse_decide_engage_when_ahead_at_exact_threshold() {
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [0.0, 0.0, -200.0],
        crate::ship::impulse::ImpulsePhase::Idle,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::Engage);
}

#[test]
fn impulse_decide_no_engage_when_too_close() {
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [0.0, 0.0, -150.0],
        crate::ship::impulse::ImpulsePhase::Idle,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
}

#[test]
fn impulse_decide_no_engage_when_target_not_ahead() {
    // Target at 90 degrees to the right
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [300.0, 0.0, 0.0],
        crate::ship::impulse::ImpulsePhase::Idle,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
}

#[test]
fn impulse_decide_cancel_when_close_during_charging() {
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [0.0, 0.0, -20.0],
        crate::ship::impulse::ImpulsePhase::Charging,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::Cancel);
}

#[test]
fn impulse_decide_cancel_when_close_during_active() {
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [0.0, 0.0, -20.0],
        crate::ship::impulse::ImpulsePhase::Active,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::Cancel);
}

#[test]
fn impulse_decide_noop_when_idle_and_not_ahead() {
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [300.0, 0.0, -100.0],
        crate::ship::impulse::ImpulsePhase::Idle,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
}

#[test]
fn impulse_decide_noop_when_active_and_ahead() {
    // Already active, target is ahead and far — no change needed
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [0.0, 0.0, -500.0],
        crate::ship::impulse::ImpulsePhase::Active,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
}

#[test]
fn impulse_decide_engage_at_angle_tolerance_boundary() {
    // Target at the edge of the tolerance cone
    let angle = IMPULSE_ANGLE_TOLERANCE_RAD; // 0.08 rad
    let target_x = 300.0 * simmath::sin(angle);
    let target_z = -300.0 * simmath::cos(angle);
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [target_x, 0.0, target_z],
        crate::ship::impulse::ImpulsePhase::Idle,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::Engage);
}

#[test]
fn impulse_decide_noop_past_angle_tolerance() {
    let angle = IMPULSE_ANGLE_TOLERANCE_RAD + 0.01; // just past boundary
    let target_x = 300.0 * simmath::sin(angle);
    let target_z = -300.0 * simmath::cos(angle);
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [target_x, 0.0, target_z],
        crate::ship::impulse::ImpulsePhase::Idle,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
}

#[test]
fn impulse_decide_cancel_at_cancel_distance_boundary() {
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [0.0, 0.0, -40.0],
        crate::ship::impulse::ImpulsePhase::Active,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::Cancel);
}

#[test]
fn impulse_decide_noop_barely_above_cancel_distance() {
    let input = impulse_input(
        [0.0, 0.0],
        0.0,
        [0.0, 0.0, -41.0],
        crate::ship::impulse::ImpulsePhase::Active,
        200.0,
        40.0,
    );
    assert_eq!(decide_impulse(&input), ImpulseDecision::NoChange);
}

// ── Navigation waypoint handoff (issues #681, #702) ────────────────────
//
// `nav_waypoint` is the position of the ship's `NavigationWaypoint`,
// supplied by the caller only once the Channel-3 clearance matches its
// generation. It replaced `AiMemory.nav_goal`, a private copy of the same
// position laundered through the coordination message.
//
// These tests lost their "…and clears nav_goal" halves, because there is no
// longer anything to clear: the waypoint belongs to Navigation, and a Helm
// that resolves a local objective simply never consults it. The
// clearing-on-arrival and clearing-on-resolve rules existed only to stop the
// private copy drifting out of sync with the real waypoint.

/// With no Helm-relevant objective, the Helm travels to the cleared
/// Navigation waypoint.
#[test]
fn operate_helm_falls_through_to_nav_waypoint_when_no_objective() {
    let world = world_at_origin(); // entities list empty, anchors empty
    let pool: Vec<crate::core::messages::ScoredObjective> = vec![];

    let (thrust, _steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &std::collections::HashMap::new(),
        NO_CURSORS,
        None,
        Some([100.0, 0.0]),
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(
        thrust > 0.0,
        "the nav-waypoint fallthrough must produce positive thrust"
    );
}

/// An *uncleared* waypoint is not followed. The caller passes `None` until
/// `HelmWaypointClearance` matches the waypoint's generation, so this is
/// where the Channel-3 lag is visible from `operate_helm`'s side: the Helm
/// has been given a waypoint but not yet the order to fly it.
#[test]
fn operate_helm_ignores_an_uncleared_nav_waypoint() {
    let world = world_at_origin();
    let pool: Vec<crate::core::messages::ScoredObjective> = vec![];

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &std::collections::HashMap::new(),
        NO_CURSORS,
        None,
        None, // clearance has not caught up with the waypoint's generation
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert_eq!(
        (thrust, steering),
        (0.0, 0.0),
        "an uncleared waypoint must not be followed - that lag is the whole \
         job of the Channel-3 handoff"
    );
}

/// Arriving at the waypoint stops the ship. It does *not* clear the
/// waypoint: the waypoint is Navigation's, and the Helm holds station on it
/// rather than reaching into another console's state.
#[test]
fn operate_helm_holds_station_on_reaching_the_nav_waypoint() {
    // Ship at origin, waypoint at [0, -1] => dist = 1 < 20 => arrived.
    let world = world_at_origin();
    let pool = vec![];

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &std::collections::HashMap::new(),
        NO_CURSORS,
        None,
        Some([0.0, -1.0]),
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert_eq!(
        (thrust, steering),
        (0.0, 0.0),
        "arrived at the nav waypoint must produce zero thrust"
    );
}

/// A resolvable local objective outranks the nav waypoint - the ship must
/// fly the objective, not blend the two bearings.
#[test]
fn operate_helm_prefers_a_local_objective_over_the_nav_waypoint() {
    let world = world_at_origin();
    let pool = patrol_pool(); // Patrol toward "alpha" at [100, 0, 0] (starboard)
    let doctrine = patrol_doctrine();
    let anchors = anchors_with_alpha();

    let (thrust, steering) = plan_helm_travel(
        &world,
        &pool,
        &doctrine,
        &anchors,
        NO_CURSORS,
        None,
        Some([0.0, -999.0]), // cleared, and nowhere near `alpha`
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(
        thrust > 0.0 && steering > 0.0,
        "Patrol resolves, so the helm must fly it (toward `alpha`, to \
         starboard = positive steering) and ignore the nav waypoint; \
         got thrust={thrust}, steering={steering}"
    );
}

/// The fallthrough is per-tick and stateless: an objective that *cannot*
/// resolve yields to the nav waypoint. This is the case the handoff exists
/// for - a Navigation AI steering a short-range Helm toward an objective the
/// Helm cannot see yet.
#[test]
fn operate_helm_falls_through_unresolvable_objectives_to_the_nav_waypoint() {
    let world = world_at_origin(); // no entities -> Destroy unresolvable
                                   // Destroy (90, unresolvable) then Patrol (30); with empty anchors the
                                   // Patrol cannot resolve either, so both fall through.
    let pool = destroy_then_patrol_pool(&std::collections::HashMap::new());

    let (thrust, _steering) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &std::collections::HashMap::new(),
        NO_CURSORS,
        None,
        Some([100.0, 0.0]),
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(
        thrust > 0.0,
        "must fall through to the nav waypoint when no objective resolves"
    );
}

/// End-to-end: the Helm flies the nav waypoint while it has nothing better
/// to do, then switches to Destroy the moment Tactical locks a target the
/// Helm can see.
#[test]
fn operate_helm_transitions_from_nav_waypoint_to_destroy() {
    let (world, pool, _anchors, target_uuid) = destroy_vs_patrol_scene();
    let no_anchors = std::collections::HashMap::new();

    // Phase 1: no objective resolves (no anchors for the Patrol, no lock for
    // the Destroy) -> fly the waypoint, which sits to starboard.
    let (thrust1, steering1) = plan_helm_travel(
        &world,
        &[],
        &[],
        &no_anchors,
        NO_CURSORS,
        None,
        Some([200.0, 0.0]),
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(
        thrust1 > 0.0 && steering1 > 0.0,
        "phase 1: must fly toward the nav waypoint (to starboard)"
    );

    // Phase 2: Tactical locks the hostile, which is dead ahead -> Destroy
    // resolves and outranks the waypoint.
    let (thrust2, steering2) = plan_helm_travel(
        &world,
        &pool,
        &[],
        &no_anchors,
        NO_CURSORS,
        Some(target_uuid),
        Some([200.0, 0.0]),
        WAYPOINT_ARRIVAL_RADIUS,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_THREAT_EXPONENT,
        0.0,
        0.6,
    );
    assert!(thrust2 > 0.0, "phase 2: must close on the hostile");
    assert!(
        steering2.abs() < PATROL_DEADBAND_RAD,
        "phase 2: a resolved Destroy must win over the nav waypoint - the \
         target is dead ahead (~0 steering), the waypoint to starboard; \
         got steering={steering2}"
    );
}

// ── Desired-motion codec (issue #741) ─────────────────────────────────

#[test]
fn thrust_velocity_codec_round_trips() {
    for thrust in [-1.0, -0.37, 0.0, 0.42, 1.0] {
        let v = encode_local_velocity(thrust, 0.0);
        // Forward is local -Z, no lateral/vertical component.
        assert_eq!(v[0], 0.0);
        assert!((decode_thrust_from_velocity(v) - thrust).abs() < 1e-6);
    }
    // Forward thrust yields a negative Z (local forward) component.
    assert!(encode_local_velocity(0.8, 0.0)[2] < 0.0);
}

#[test]
fn steering_facing_codec_round_trips_and_preserves_sign() {
    for steering in [-1.0, -0.5, 0.0, 0.25, 1.0] {
        let f = encode_local_facing(steering);
        // Unit-length facing direction.
        assert!(((f[0] * f[0] + f[2] * f[2]).sqrt() - 1.0).abs() < 1e-6);
        let decoded = decode_steering_from_facing(f);
        assert!(
            (decoded - steering).abs() < 1e-6,
            "steering {steering} round-tripped to {decoded}"
        );
    }
    // Zero steering faces exactly local forward (-Z); decode is exactly 0.
    assert_eq!(decode_steering_from_facing(encode_local_facing(0.0)), 0.0);
    // Starboard steering points the facing to +X.
    assert!(encode_local_facing(0.5)[0] > 0.0);
}

// ── Docking close manoeuvre (issue #742) ──────────────────────────────

#[test]
fn docking_reverses_for_a_dock_directly_astern() {
    // Ship at origin facing -Z (forward). A dock at +Z is dead astern.
    let m = docking_close_manoeuvre(0.0, 0.0, 0.0, 0.0, 10.0, 40.0, 0.3)
        .expect("dock inside engage distance must yield a close manoeuvre");
    assert!(
        m[1] > 0.0,
        "an astern dock must command controlled reverse (aft > 0); got {m:?}"
    );
    assert!(
        m[0].abs() < 1e-6,
        "a dock straight astern needs no lateral translation; got {m:?}"
    );
    assert!(
        m[1] <= 0.3 + 1e-6,
        "reverse must be capped by approach_speed"
    );
}

#[test]
fn docking_translates_laterally_for_a_dock_abeam() {
    // Ship at origin facing -Z; a dock at +X is off the starboard beam.
    let m = docking_close_manoeuvre(0.0, 0.0, 0.0, 10.0, 0.0, 40.0, 0.3)
        .expect("dock inside engage distance must yield a close manoeuvre");
    assert!(
        m[0] > 0.0,
        "a starboard-beam dock must command starboard lateral translation; got {m:?}"
    );
    assert!(
        m[1].abs() < 1e-6,
        "a dock straight abeam needs no fore/aft translation; got {m:?}"
    );
}

#[test]
fn docking_holds_off_beyond_engage_distance() {
    // Dock 100 units away, engage distance 40 — still normal approach.
    assert_eq!(
        docking_close_manoeuvre(0.0, 0.0, 0.0, 0.0, 100.0, 40.0, 0.3),
        None,
        "a dock beyond engage distance must not trigger a close manoeuvre"
    );
}

#[test]
fn assess_hazards_flags_a_projected_collision_ahead() {
    // Ship at origin facing -Z, moving forward; obstacle dead ahead.
    let view = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        entities: vec![AiWorldEntity {
            uuid: Uuid::from_u128(9),
            position: [0.0, 0.0, -10.0],
            radius: 5.0,
            ..Default::default()
        }],
        ..Default::default()
    };
    // Speed 3 over the 3 s look-ahead projects the ship to z=-9, right up
    // against the obstacle at z=-10 (projected distance 1 < radius 12).
    let hz = assess_hazards(
        &view,
        3.0,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_IGNORE_SIZE_RATIO,
        HAZARD_THREAT_EXPONENT,
    );
    assert!(
        hz.urgency > 0.0,
        "an imminent head-on must register urgency"
    );
    assert_eq!(hz.primary, Some(Uuid::from_u128(9)));
    // Repulsion pushes aft (local +Z) to brake off the obstacle ahead.
    assert!(
        hz.forces_local[2] > 0.0,
        "expected an aft-pushing repulsion, got {:?}",
        hz.forces_local
    );
    // The contributing hazard is exposed with its published facts and the
    // force it added (issue #743).
    assert_eq!(hz.contributions.len(), 1);
    let c = &hz.contributions[0];
    assert_eq!(c.uuid, Uuid::from_u128(9));
    assert_eq!(c.size_rating, 0.0);
    assert!(
        c.dangerous,
        "a contributing hazard is dangerous by definition"
    );
    assert!(
        c.force_local[2] > 0.0,
        "the contribution's own force must push aft, got {:?}",
        c.force_local
    );
    assert!((c.threat_fraction - hz.urgency).abs() < 1e-6);
    // Issue #780: the surface is 3D, but a STATIC hazard contributes no
    // vertical component — the vertical axis is a moving-hazard concern (AC5).
    assert_eq!(
        hz.forces_local[1], 0.0,
        "a static hazard must not push the vertical axis, got {:?}",
        hz.forces_local
    );
    assert_eq!(c.force_local[1], 0.0);
}

/// Issue #780 (AC2/AC5): `assess_hazards` is genuinely 3D — an ELIGIBLE
/// MOVING hazard populates a vertical (local +Y) force so a bounded/full-3D
/// hull can climb to clear it, while an identically-placed STATIC hazard
/// leaves the vertical axis untouched. Both register a horizontal threat, so
/// the only difference is the `movable` fact.
#[test]
fn assess_hazards_computes_vertical_force_for_moving_only() {
    // A co-planar obstacle dead ahead that projects into collision.
    let make = |movable: bool| WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        entities: vec![AiWorldEntity {
            uuid: Uuid::from_u128(7),
            position: [0.0, 0.0, -10.0],
            radius: 5.0,
            movable,
            dangerous: true,
            ..Default::default()
        }],
        ..Default::default()
    };

    let moving = assess_hazards(
        &make(true),
        3.0,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_IGNORE_SIZE_RATIO,
        HAZARD_THREAT_EXPONENT,
    );
    assert!(
        moving.forces_local[1] > 0.0,
        "a co-planar MOVING hazard must produce an upward (climb) vertical \
         force, got {:?}",
        moving.forces_local
    );
    assert!(moving.contributions[0].force_local[1] > 0.0);

    let static_hz = assess_hazards(
        &make(false),
        3.0,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_IGNORE_SIZE_RATIO,
        HAZARD_THREAT_EXPONENT,
    );
    assert_eq!(
        static_hz.forces_local[1], 0.0,
        "a STATIC hazard must leave the vertical axis at zero, got {:?}",
        static_hz.forces_local
    );
    // Both still registered a horizontal (aft) repulsion, proving the vertical
    // difference is the `movable` fact and not just a missed collision.
    assert!(moving.forces_local[2] > 0.0 && static_hz.forces_local[2] > 0.0);
}

/// Issue #780: an off-plane moving hazard drives the climb along the ACTUAL
/// vertical separation — a hazard below the ship pushes it up, one above
/// pushes it down.
#[test]
fn assess_hazards_follows_vertical_separation_sign() {
    let make = |hazard_y: f32| WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        entities: vec![AiWorldEntity {
            uuid: Uuid::from_u128(8),
            position: [0.0, hazard_y, -10.0],
            radius: 5.0,
            movable: true,
            dangerous: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    // Hazard below (y = -5): dy = self(0) - (-5) = +5 → climb up (+).
    let below = assess_hazards(
        &make(-5.0),
        3.0,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_IGNORE_SIZE_RATIO,
        HAZARD_THREAT_EXPONENT,
    );
    assert!(below.forces_local[1] > 0.0, "hazard below must push up");
    // Hazard above (y = +5): dy = -5 → descend (-).
    let above = assess_hazards(
        &make(5.0),
        3.0,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_IGNORE_SIZE_RATIO,
        HAZARD_THREAT_EXPONENT,
    );
    assert!(above.forces_local[1] < 0.0, "hazard above must push down");
}

#[test]
fn assess_hazards_is_quiet_with_no_entities() {
    let view = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        self_radius: 2.0,
        ..Default::default()
    };
    let hz = assess_hazards(
        &view,
        10.0,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_IGNORE_SIZE_RATIO,
        HAZARD_THREAT_EXPONENT,
    );
    assert_eq!(hz, HazardAssessmentRaw::default());
}

#[test]
fn assess_hazards_skips_non_dangerous_entities() {
    // A non-dangerous entity dead ahead must not register as a hazard: the
    // published `dangerous` fact, not the geometry, decides (issue #743).
    let view = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        self_size_rating: 2.0,
        entities: vec![AiWorldEntity {
            uuid: Uuid::from_u128(9),
            position: [0.0, 0.0, -10.0],
            radius: 5.0,
            dangerous: false,
            ..Default::default()
        }],
        ..Default::default()
    };
    let hz = assess_hazards(
        &view,
        3.0,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        HAZARD_IGNORE_SIZE_RATIO,
        HAZARD_THREAT_EXPONENT,
    );
    assert_eq!(
        hz,
        HazardAssessmentRaw::default(),
        "a non-dangerous entity must contribute no force"
    );
}

#[test]
fn assess_hazards_ignores_hazards_smaller_than_self_when_authored() {
    // Large self (size_rating 10) versus a small SHIP (size_rating 1) dead
    // ahead. With the ignore rule authored on (ratio 1.0), a mobile contact
    // strictly smaller than self is skipped entirely — "large ships do not
    // avoid smaller ships at all" (issue #743). `movable: true` is
    // load-bearing here: the rule is mobile-only since issue #958, and the
    // static counterpart is pinned by the sibling test below.
    let small_obstacle = AiWorldEntity {
        uuid: Uuid::from_u128(9),
        position: [0.0, 0.0, -10.0],
        radius: 1.0,
        size_rating: 1.0,
        dangerous: true,
        movable: true,
        ..Default::default()
    };
    let view = WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        self_size_rating: 10.0,
        entities: vec![small_obstacle.clone()],
        ..Default::default()
    };

    // Rule off (ratio 0.0, the default): the small obstacle is a hazard.
    let assessed = assess_hazards(
        &view,
        3.0,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        0.0,
        HAZARD_THREAT_EXPONENT,
    );
    assert!(
        assessed.urgency > 0.0,
        "with the ignore rule off, even a small obstacle is a hazard"
    );

    // Rule on (ratio 1.0): the smaller hazard is ignored → no force at all.
    let ignored = assess_hazards(
        &view,
        3.0,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        1.0,
        HAZARD_THREAT_EXPONENT,
    );
    assert_eq!(
        ignored,
        HazardAssessmentRaw::default(),
        "an authored ignore-smaller rule must skip a hazard below self's size rating"
    );

    // A same-or-larger hazard is still assessed under the same ratio.
    let big_view = WorldView {
        entities: vec![AiWorldEntity {
            size_rating: 10.0,
            ..small_obstacle
        }],
        ..view
    };
    let big = assess_hazards(
        &big_view,
        3.0,
        AVOIDANCE_BUFFER,
        AVOIDANCE_LOOK_AHEAD_SECS,
        1.0,
        HAZARD_THREAT_EXPONENT,
    );
    assert!(
        big.urgency > 0.0,
        "a hazard at or above self's size rating is never ignored"
    );
}

/// Issue #958: the ignore-smaller rule is a MOBILE-contact rule. A rock, a
/// station or a planet cannot manoeuvre out of a big ship's way, so it stays
/// in the hazard picture at any relative size — the same authored ratio that
/// drops an identically-sized *ship* must leave it alone.
///
/// Pinned directly rather than through shipped content on purpose: no entity
/// TOML authors `hazard_ignore_size_ratio` today, so the rule is inert in
/// production and a content-level assertion would pass for the wrong reason.
#[test]
fn assess_hazards_never_ignores_static_terrain_below_own_size() {
    // The one difference between the two hazards is the authored `movable`
    // fact; geometry, size rating and danger are identical.
    let terrain = |size_rating: f32, movable: bool| WorldView {
        entity_pos: [0.0, 0.0, 0.0],
        entity_yaw: 0.0,
        self_radius: 2.0,
        self_size_rating: 10.0,
        entities: vec![AiWorldEntity {
            uuid: Uuid::from_u128(9),
            position: [0.0, 0.0, -10.0],
            radius: 1.0,
            size_rating,
            dangerous: true,
            movable,
            ..Default::default()
        }],
        ..Default::default()
    };
    // Ratio 1.0 = "ignore anything strictly smaller than self", the most
    // aggressive setting a designer can author short of ignoring equals.
    let assess = |view: &WorldView| {
        assess_hazards(
            view,
            3.0,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            1.0,
            HAZARD_THREAT_EXPONENT,
        )
    };

    // A static hazard rated 1 against self's 10 is still avoided.
    let small_static = assess(&terrain(1.0, false));
    assert!(
        small_static.urgency > 0.0,
        "static terrain below own size must still be avoided, got {small_static:?}"
    );
    assert_eq!(small_static.primary, Some(Uuid::from_u128(9)));
    assert_eq!(
        small_static.contributions.len(),
        1,
        "the static hazard must survive into the contribution list"
    );
    assert!(!small_static.contributions[0].movable);

    // The identical hazard published as a mobile contact IS ignorable —
    // this is what makes the assertion above about `movable` and not about
    // the geometry.
    assert_eq!(
        assess(&terrain(1.0, true)),
        HazardAssessmentRaw::default(),
        "a SHIP below own size stays ignorable under the same authored ratio"
    );

    // Static terrain at or above own size is avoided too — the rule never
    // had anything to say about it, and still does not.
    let big_static = assess(&terrain(10.0, false));
    assert!(
        big_static.urgency > 0.0,
        "static terrain at or above own size must be avoided, got {big_static:?}"
    );
    let huge_static = assess(&terrain(25.0, false));
    assert!(
        huge_static.urgency > 0.0,
        "static terrain larger than self must be avoided, got {huge_static:?}"
    );
}

/// Issue #968: the response at a given SURFACE clearance must not depend on
/// how big the obstacle is.
///
/// This is the defect the issue reports, stated as a unit: the shipped
/// destroyer (collider radius 1.2) sitting exactly against a rock's skin used
/// to register 0.61 urgency for a `small` rock (radius 2) and only 0.27 for
/// the `huge` class added in issue #947 (radius 12) — less than half the push
/// for the obstacle that is three times harder to get around, because the old
/// threat fraction divided the CENTRE separation by the whole avoidance
/// radius. Contact is contact: all three sizes must read full urgency, and a
/// hull one buffer-width clear of any of them must read none.
#[test]
fn hazard_severity_is_the_same_at_contact_for_every_obstacle_size() {
    const SELF_RADIUS: f32 = 1.2;
    const BUFFER: f32 = 5.0;

    // Stationary hull at the origin, so the projected position is the actual
    // one and `centre_distance` is exactly what this test places.
    let at_centre_distance = |rock_radius: f32, centre_distance: f32| {
        let view = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: SELF_RADIUS,
            entities: vec![AiWorldEntity {
                uuid: Uuid::from_u128(1),
                position: [0.0, 0.0, -centre_distance],
                radius: rock_radius,
                ..Default::default()
            }],
            ..Default::default()
        };
        assess_hazards(
            &view,
            0.0,
            BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            HAZARD_THREAT_EXPONENT,
        )
        .urgency
    };

    for rock_radius in [2.0_f32, 4.0, 12.0] {
        // Skin-to-skin contact: centre distance == the two radii summed.
        let touching = at_centre_distance(rock_radius, SELF_RADIUS + rock_radius);
        assert!(
            (touching - 1.0).abs() < 1e-5,
            "a hull touching a radius-{rock_radius} obstacle must read full \
             urgency, got {touching}"
        );

        // Half a buffer of clear space left: the squared ramp's 0.25, and
        // again the same figure whatever the obstacle's size. Size-invariance
        // is the property under test; the RAMP SHAPE is stated here too so
        // that changing it is a deliberate re-bless rather than a silent one
        // (see `hazard_threat_fraction` for why it is not linear).
        let half_clear = at_centre_distance(rock_radius, SELF_RADIUS + rock_radius + BUFFER / 2.0);
        assert!(
            (half_clear - 0.25).abs() < 1e-5,
            "half a buffer clear of a radius-{rock_radius} obstacle must read \
             the same quarter urgency at every size, got {half_clear}"
        );

        // What squaring COSTS at mid-ramp, stated rather than glossed. The
        // old centre-distance form scored `buffer * spent / (self_radius +
        // hazard_radius + buffer)` here, and the two curves cross at
        // `spent = buffer / (self_radius + hazard_radius + buffer)`. Below
        // that crossing the new response is the weaker of the two — for a
        // `small` rock the crossing is 0.61, so at this mid-ramp sample the
        // hull now pushes 0.25 where it used to push 0.30. Pinned so a
        // future exponent change has to face the number.
        let old_at_half_ramp = BUFFER * 0.5 / (SELF_RADIUS + rock_radius + BUFFER);
        let delta = half_clear - old_at_half_ramp;
        let expected_delta = match rock_radius as i32 {
            2 => -0.055,
            4 => 0.005,
            _ => 0.113,
        };
        assert!(
            (delta - expected_delta).abs() < 5e-3,
            "mid-ramp response against a radius-{rock_radius} obstacle moved \
             by {delta:+.3} (old {old_at_half_ramp:.3} → new {half_clear:.3}); \
             the recorded figure is {expected_delta:+.3}"
        );

        // A full buffer clear: nothing to react to, at any size.
        let clear = at_centre_distance(rock_radius, SELF_RADIUS + rock_radius + BUFFER);
        assert_eq!(
            clear, 0.0,
            "a hull a full buffer clear of a radius-{rock_radius} obstacle \
             must read no urgency, got {clear}"
        );
    }
}

/// Issue #968: the four Alliance hulls' authored
/// `imminent_collision_facing_threshold = 0.6` now trips at a fixed SURFACE
/// clearance instead of a fixed fraction of the avoidance radius, and that
/// re-scales it — for the better against a `huge` rock, and much earlier
/// against a small one.
///
/// Old trigger: projected centre separation below `0.4 × (self_radius +
/// hazard_radius + buffer)`. New trigger: clearance at or under
/// `buffer × (1 − √0.6)` = 1.127 units, whatever the obstacle. Against the
/// `huge` class the old form did not fire until the hull was already 5.9
/// units INSIDE the rock, which is the bug; against a `small` rock it fired
/// at 0.08 units of clearance and now fires at 1.13, roughly fourteen times
/// earlier. The override snaps `desired_facing_local` off the gunnery
/// solution, so that end is a real behaviour change and is pinned here.
#[test]
fn imminent_collision_facing_threshold_now_crosses_at_a_fixed_surface_clearance() {
    const SELF_RADIUS: f32 = 1.2;
    const BUFFER: f32 = 5.0;
    const THRESHOLD: f32 = 0.6;

    let urgency_at_clearance = |rock_radius: f32, clearance: f32| {
        let view = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: SELF_RADIUS,
            entities: vec![AiWorldEntity {
                uuid: Uuid::from_u128(1),
                position: [0.0, 0.0, -(SELF_RADIUS + rock_radius + clearance)],
                radius: rock_radius,
                ..Default::default()
            }],
            ..Default::default()
        };
        assess_hazards(
            &view,
            0.0,
            BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            HAZARD_THREAT_EXPONENT,
        )
        .urgency
    };

    let crossing = BUFFER * (1.0 - THRESHOLD.sqrt());
    assert!(
        (crossing - 1.127).abs() < 1e-3,
        "the recorded crossing clearance is 1.127 units, computed {crossing}"
    );

    for rock_radius in [2.0_f32, 4.0, 12.0] {
        assert!(
            urgency_at_clearance(rock_radius, crossing - 0.01) >= THRESHOLD,
            "just inside the crossing, a radius-{rock_radius} obstacle must \
             trip the authored 0.6 facing override"
        );
        assert!(
            urgency_at_clearance(rock_radius, crossing + 0.01) < THRESHOLD,
            "just outside the crossing, a radius-{rock_radius} obstacle must \
             not trip it — the trigger is a clearance, not a size"
        );
    }

    // The small-rock end, said out loud: the old form put this crossing at
    // 0.08 units of clearance, so a `small` rock now takes the hull's facing
    // roughly fourteen times further out than the authored 0.6 used to mean.
    let old_small_crossing = (1.0 - THRESHOLD) * (SELF_RADIUS + 2.0 + BUFFER) - SELF_RADIUS - 2.0;
    assert!(
        (old_small_crossing - 0.08).abs() < 1e-2,
        "the old crossing against a radius-2 rock is 0.08 units of clearance, \
         computed {old_small_crossing}"
    );
    assert!(
        crossing / old_small_crossing > 13.0,
        "the small-rock trigger moved {}× further out; if that ratio changes \
         the note on `hazard_threat_fraction` needs changing with it",
        crossing / old_small_crossing
    );
}

/// The overlapping case, which is where issue #968's ships actually ended up:
/// a hull INSIDE a `huge` rock must saturate rather than fade. Saturation is
/// what makes the authored `imminent_collision_facing_threshold` reachable at
/// its `1.0` default, i.e. what gives a buried hull a way back out.
///
/// AT SPEED as well as at rest, which is the half that actually matters: the
/// hulls this issue is about were doing 13.5 u/s. `assess_hazards` measures
/// from a point projected `forward_speed × 3 s` ahead, so at that speed the
/// measuring point is 40.5 units up-track while the whole avoidance radius
/// for a `huge` rock is 18.2 — the rock the ship is buried in falls out of
/// the projected picture entirely and urgency reads 0.0. Both readings are
/// taken and the worse one kept, so it does not.
#[test]
fn hazard_severity_saturates_while_a_hull_is_inside_a_collider() {
    let buried = |penetration: f32, forward_speed: f32| {
        let view = WorldView {
            entity_pos: [0.0, 0.0, 0.0],
            entity_yaw: 0.0,
            self_radius: 1.2,
            entities: vec![AiWorldEntity {
                uuid: Uuid::from_u128(2),
                position: [0.0, 0.0, -(1.2 + 12.0 - penetration)],
                radius: 12.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        assess_hazards(
            &view,
            forward_speed,
            AVOIDANCE_BUFFER,
            AVOIDANCE_LOOK_AHEAD_SECS,
            0.0,
            HAZARD_THREAT_EXPONENT,
        )
    };

    // 6.5 units inside is what the instrumented `combat_test` run measured;
    // 13.5 u/s is the destroyer's authored cruise there (`target_speed` 0.9
    // of a 15 u/s hull), which projects the look-ahead 40.5 units past the
    // rock.
    for penetration in [0.5_f32, 3.0, 6.5] {
        for forward_speed in [0.0_f32, 13.5] {
            let hz = buried(penetration, forward_speed);
            assert_eq!(
                hz.urgency, 1.0,
                "a hull {penetration} units inside a radius-12 collider must \
                 read full urgency at {forward_speed} u/s, got {:?}",
                hz.urgency
            );
            assert!(
                hz.forces_local[2] > 0.0,
                "the repulsion must still push aft, off the obstacle ahead, \
                 got {:?}",
                hz.forces_local
            );
        }
    }

    // Anti-vacuity: the projected reading really would have lost the rock at
    // that speed, so the loop above is exercising the max and not merely
    // agreeing with what the projection said anyway. A hull 6.5 units inside
    // sits 6.7 from the centre; projected 40.5 units on it is 33.8 the far
    // side, well outside the 18.2 avoidance radius.
    let projected_separation = (40.5_f32 - (1.2 + 12.0 - 6.5)).abs();
    assert!(
        projected_separation > 1.2 + 12.0 + AVOIDANCE_BUFFER,
        "the projected point must fall outside the avoidance radius for this \
         test to be about the un-projected reading, got {projected_separation}"
    );
}

// ── Target-relative motion + fly-through pass (issue #883) ───────────────

/// The closing rate is the signed rate of change of range: positive while
/// the gap shrinks, negative once it opens. Closest approach is exactly that
/// sign flip, which is what the destroyer doctrine's transition guard reads.
#[test]
fn closing_rate_is_positive_closing_and_negative_opening() {
    // Ship at the origin at yaw 0 (forward = -Z), target 100 units ahead.
    let closing = target_relative_motion([0.0, 0.0, 0.0], 0.0, 10.0, [0.0, 0.0, -100.0], None, 0.0);
    assert!((closing.range - 100.0).abs() < 1e-3);
    assert!(
        (closing.closing_rate - 10.0).abs() < 1e-3,
        "flying straight at a stationary target closes at our own speed, got {}",
        closing.closing_rate
    );
    assert!(
        closing.bearing_rad.abs() < 1e-3,
        "dead ahead is zero bearing"
    );

    // Same geometry, but the target is now ASTERN (we already flew past).
    let opening = target_relative_motion([0.0, 0.0, 0.0], 0.0, 10.0, [0.0, 0.0, 100.0], None, 0.0);
    assert!(
        opening.closing_rate < 0.0,
        "a target astern of a forward-moving ship must be opening, got {}",
        opening.closing_rate
    );
    assert!(
        (opening.bearing_rad.abs() - std::f32::consts::PI).abs() < 1e-3,
        "a target dead astern bears +/-pi"
    );
}

/// `AiWorldEntity` carries no velocity, so the target's contribution to the
/// closing rate is reconstructed from its yaw + forward speed. A target
/// running away faster than we chase must read as OPENING even though we are
/// pointed straight at it — the case a range-only detector gets wrong.
#[test]
fn closing_rate_reconstructs_the_targets_own_velocity() {
    // Both at yaw 0 (heading -Z); the target ahead of us and faster.
    let m = target_relative_motion(
        [0.0, 0.0, 0.0],
        0.0,
        5.0,
        [0.0, 0.0, -50.0],
        Some(0.0),
        20.0,
    );
    assert!(
        m.closing_rate < 0.0,
        "a target outrunning us is opening the range, got {}",
        m.closing_rate
    );
    // Target running head-on toward us (yaw pi => heading +Z).
    let head_on = target_relative_motion(
        [0.0, 0.0, 0.0],
        0.0,
        5.0,
        [0.0, 0.0, -50.0],
        Some(std::f32::consts::PI),
        20.0,
    );
    assert!(
        (head_on.closing_rate - 25.0).abs() < 1e-2,
        "head-on closure is the sum of both speeds, got {}",
        head_on.closing_rate
    );
}

/// A target off the starboard bow bears positive; off the port bow,
/// negative — the same sign convention [`steer_toward`] uses, so a policy
/// guard on `bearing_to_target` and the steering solution cannot disagree
/// about which way "right" is.
#[test]
fn bearing_to_target_is_signed_starboard_positive() {
    let starboard =
        target_relative_motion([0.0, 0.0, 0.0], 0.0, 0.0, [50.0, 0.0, -50.0], None, 0.0);
    assert!(starboard.bearing_rad > 0.0);
    let port = target_relative_motion([0.0, 0.0, 0.0], 0.0, 0.0, [-50.0, 0.0, -50.0], None, 0.0);
    assert!(port.bearing_rad < 0.0);
}

/// Degenerate (co-located) geometry yields zeroes, never NaN — the seeding
/// path has no way to poison a policy guard with a NaN comparison.
#[test]
fn target_relative_motion_is_degenerate_safe() {
    let m = target_relative_motion([7.0, 0.0, -3.0], 1.2, 9.0, [7.0, 0.0, -3.0], Some(0.4), 4.0);
    assert_eq!(m, TargetRelativeMotion::default());
}

fn pass_input(leg: FlyThroughLeg, entities: &[AiWorldEntity]) -> FlyThroughPassInput<'_> {
    FlyThroughPassInput {
        leg,
        self_pos: [0.0, 0.0, 0.0],
        self_yaw: 0.0,
        self_speed: 10.0,
        self_radius: 1.0,
        target_pos: [60.0, 0.0, -60.0],
        target_uuid: Uuid::nil(),
        escape_heading_rad: 0.0,
        approach_speed: 0.85,
        escape_speed: 1.0,
        reengage_speed: 0.0,
        // Deliberately different from `reengage_speed` so a leg that took
        // the wrong scalar would be visible rather than accidentally right.
        torpedo_bearing_speed: 0.25,
        tracking_deadband_rad: 0.03,
        tracking_full_steer_rad: 0.6,
        entities,
        avoidance_buffer: AVOIDANCE_BUFFER,
        avoidance_look_ahead_secs: AVOIDANCE_LOOK_AHEAD_SECS,
        hazard_threat_exponent: HAZARD_THREAT_EXPONENT,
    }
}

/// The inbound leg flies its authored approach throttle FLAT and steers at
/// the target's current position. Contrast `helm_destroy`, which would be
/// ramping thrust toward zero at the near range — the pass never brakes.
#[test]
fn inbound_leg_tracks_the_target_without_braking() {
    let none: [AiWorldEntity; 0] = [];
    let far = plan_fly_through_pass(&pass_input(FlyThroughLeg::Inbound, &none));
    let mut near_input = pass_input(FlyThroughLeg::Inbound, &none);
    near_input.target_pos = [3.0, 0.0, -3.0];
    let near = plan_fly_through_pass(&near_input);
    assert_eq!(
        far.0, near.0,
        "throttle must not fall off as the target gets closer: a fly-through \
         pass does not decelerate into the merge"
    );
    assert!(
        (far.0 - 0.85).abs() < 1e-6,
        "throttle is the authored approach fraction"
    );
    assert!(
        far.1 > 0.0,
        "a target off the starboard bow must command a starboard turn"
    );
}

/// The escape leg ignores the target completely: moving it to the opposite
/// side of the ship changes nothing, because the heading is frozen. This is
/// the observable difference between "hold the command" and "hold the
/// heading".
#[test]
fn escape_leg_flies_the_frozen_heading_and_ignores_the_target() {
    let none: [AiWorldEntity; 0] = [];
    let mut a = pass_input(FlyThroughLeg::Escape, &none);
    a.target_pos = [500.0, 0.0, 0.0];
    let mut b = pass_input(FlyThroughLeg::Escape, &none);
    b.target_pos = [-500.0, 0.0, 0.0];
    assert_eq!(
        plan_fly_through_pass(&a),
        plan_fly_through_pass(&b),
        "the escape solution must not depend on the target at all"
    );
    // Already on the frozen heading (yaw 0 == heading 0) -> no yaw demanded.
    assert_eq!(plan_fly_through_pass(&a).1, 0.0);
    assert!((plan_fly_through_pass(&a).0 - 1.0).abs() < 1e-6);

    // A frozen heading off to starboard turns the ship onto it and holds.
    let mut turned = pass_input(FlyThroughLeg::Escape, &none);
    turned.escape_heading_rad = 1.0;
    assert!(plan_fly_through_pass(&turned).1 > 0.0);
}

/// AC3 at the pure layer: a hazard beside the escape path bends the escape
/// steering while the leg — the caller's pass state — is untouched. The arm
/// takes the leg as an INPUT and returns only actuator scalars, so avoidance
/// has no channel through which it could change the pass at all.
#[test]
fn hazard_bends_the_escape_without_changing_the_leg() {
    let none: [AiWorldEntity; 0] = [];
    let clear = plan_fly_through_pass(&pass_input(FlyThroughLeg::Escape, &none));
    assert_eq!(clear.1, 0.0, "nothing to avoid: dead-ahead escape, no yaw");

    // A rock just off the projected escape path (10 u/s * 3 s look-ahead
    // puts our projection at z = -30).
    let rock = [AiWorldEntity {
        uuid: Uuid::new_v4(),
        position: [3.0, 0.0, -30.0],
        radius: 2.0,
        size_rating: 2.0,
        ..Default::default()
    }];
    let bent = plan_fly_through_pass(&pass_input(FlyThroughLeg::Escape, &rock));
    assert!(
        bent.1.abs() > 0.0,
        "a hazard on the escape path must bend the escape steering"
    );
    assert_eq!(
        bent.0, clear.0,
        "avoidance bends the heading, it does not change the leg's throttle"
    );
}

/// Issue #788, AC7: the re-entry pivot tracks the target exactly as the
/// inbound leg does, but flies the authored re-engage throttle. With the
/// destroyer's authored `0.0` that is a cut-thrust turn — the observable
/// difference between "turning to start a pass" and "running the pass".
#[test]
fn reengage_leg_tracks_the_target_on_the_authored_reengage_throttle() {
    let none: [AiWorldEntity; 0] = [];
    let inbound = plan_fly_through_pass(&pass_input(FlyThroughLeg::Inbound, &none));
    let pivot = plan_fly_through_pass(&pass_input(FlyThroughLeg::Reengage, &none));
    assert_eq!(
        pivot.1, inbound.1,
        "the pivot IS the tracking solution: same steering as the inbound leg"
    );
    assert_eq!(pivot.0, 0.0, "and it cuts thrust to make the turn");

    // The throttle is the authored scalar, not a hardcoded zero.
    let mut powered = pass_input(FlyThroughLeg::Reengage, &none);
    powered.reengage_speed = 0.4;
    assert!((plan_fly_through_pass(&powered).0 - 0.4).abs() < 1e-6);
}

/// Issue #791: the torpedo-opportunity hold tracks the target's LIVE
/// position (so a manoeuvring target keeps being followed onto the bow) and
/// flies its OWN authored throttle — not the re-engage one, and not the
/// approach one.
///
/// The throttle half is the load-bearing assertion. The fixture authors
/// `reengage_speed = 0.0` and `torpedo_bearing_speed = 0.25` precisely so a
/// leg that quietly took the wrong scalar would look like a cut-thrust turn
/// and pass every other check here.
#[test]
fn torpedo_bearing_leg_tracks_the_live_target_on_its_own_authored_throttle() {
    let none: [AiWorldEntity; 0] = [];
    let inbound = plan_fly_through_pass(&pass_input(FlyThroughLeg::Inbound, &none));
    let hold = plan_fly_through_pass(&pass_input(FlyThroughLeg::TorpedoBearing, &none));
    assert_eq!(
        hold.1, inbound.1,
        "the hold IS a tracking solution: same steering as the inbound leg"
    );
    assert!(
        (hold.0 - 0.25).abs() < 1e-6,
        "the throttle is `torpedo_bearing_speed`, not `reengage_speed` (0.0) \
         and not `approach_speed` (0.85), got {}",
        hold.0
    );

    // A target that MOVES to the other side flips the commanded turn: the
    // solution is re-derived from the live position every call, which is
    // what separates this leg from the frozen-heading escape.
    let mut port = pass_input(FlyThroughLeg::TorpedoBearing, &none);
    port.target_pos = [-60.0, 0.0, -60.0];
    assert!(
        plan_fly_through_pass(&port).1 < 0.0 && hold.1 > 0.0,
        "moving the target across the bow must reverse the commanded turn"
    );

    // An authored cut-thrust hold is a real authored value, not a default.
    let mut cut = pass_input(FlyThroughLeg::TorpedoBearing, &none);
    cut.torpedo_bearing_speed = 0.0;
    assert_eq!(plan_fly_through_pass(&cut).0, 0.0);
}

// ── The artillery firing position (issue #792) ───────────────────────────

/// A battleship at the origin facing `-Z`, holding station on a target 180
/// units dead ahead. `hold_speed` is deliberately NON-zero here so the tests
/// below pin "the authored throttle" rather than accidentally agreeing with a
/// hardcoded stop.
fn artillery_input(entities: &[AiWorldEntity]) -> ArtilleryPositionInput<'_> {
    ArtilleryPositionInput {
        self_pos: [0.0, 0.0, 0.0],
        self_yaw: 0.0,
        self_speed: 12.0,
        self_radius: 3.0,
        target_pos: [0.0, 0.0, -180.0],
        target_yaw: None,
        target_speed: 0.0,
        target_uuid: Uuid::from_u128(9),
        hold_speed: 0.15,
        projectile_speed: 35.0,
        tracking_deadband_rad: 0.03,
        tracking_full_steer_rad: 0.6,
        entities,
        avoidance_buffer: AVOIDANCE_BUFFER,
        avoidance_look_ahead_secs: AVOIDANCE_LOOK_AHEAD_SECS,
        hazard_threat_exponent: HAZARD_THREAT_EXPONENT,
    }
}

/// AC3/AC4: the hold flies its OWN authored throttle, and the facing is a
/// PREDICTIVE intercept rather than a bearing to the target.
#[test]
fn artillery_position_leads_a_crossing_target_on_its_authored_throttle() {
    let none: [AiWorldEntity; 0] = [];

    // A stationary target is the degenerate case: lead nothing, and with the
    // bow already on it command no turn.
    let still = plan_artillery_position(&artillery_input(&none));
    assert!(
        (still.0 - 0.15).abs() < 1e-6,
        "the throttle is the authored `hold_speed`, got {}",
        still.0
    );
    assert_eq!(
        still.1, 0.0,
        "a stationary target dead ahead needs no correction"
    );

    // Crossing square across the line of sight, at a named heading and speed.
    let crossing = |yaw: f32, bolt: f32| {
        let mut input = artillery_input(&none);
        input.target_yaw = Some(yaw);
        input.target_speed = 24.0;
        input.projectile_speed = bolt;
        plan_artillery_position(&input)
    };

    // To starboard (+X): the aim point moves ahead of it, so the commanded
    // turn is to starboard.
    let led = crossing(std::f32::consts::FRAC_PI_2, 35.0);
    assert!(
        led.1 > 0.0,
        "the bow must turn toward where the target is GOING, got {}",
        led.1
    );

    // ...and the other way, so the sign follows the target rather than being
    // a fixed bias.
    assert!(crossing(-std::f32::consts::FRAC_PI_2, 35.0).1 < 0.0);

    // The lead is the FLIGHT TIME's doing: a bolt that arrives instantly has
    // nothing to lead by, and the same crossing target then commands no turn
    // at all. This is the assertion that would fail if the leg quietly
    // tracked the live position and got its sign right by luck.
    assert!(
        crossing(std::f32::consts::FRAC_PI_2, 100_000.0).1.abs() < 1e-3,
        "a bolt with no flight time has nothing to lead by"
    );

    // A target whose heading is unknown contributes no velocity: the solution
    // degrades to "aim at where it is" rather than inventing a course.
    let mut unknown = artillery_input(&none);
    unknown.target_speed = 24.0;
    assert_eq!(plan_artillery_position(&unknown).1, 0.0);

    // ...as does an unresolvable lead speed, which is what a hull carrying no
    // artillery bank at all publishes.
    assert_eq!(
        crossing(std::f32::consts::FRAC_PI_2, 0.0).1,
        0.0,
        "no flight speed must fall back to the target's live bearing, not to \
         whichever way the hull happened to be pointing"
    );
}

/// AC6, the additive half: a hazard BENDS the intercept facing and changes
/// nothing else. The thrust is untouched, so avoidance can never turn the
/// hold into a translation, and the bend is a sum rather than a substitution
/// — the leg still knows where its target is.
#[test]
fn artillery_position_folds_avoidance_onto_the_intercept_facing() {
    let none: [AiWorldEntity; 0] = [];
    let clean = plan_artillery_position(&artillery_input(&none));

    let obstacle = [AiWorldEntity {
        uuid: Uuid::from_u128(77),
        // On the hull's projected path, off the starboard bow.
        position: [6.0, 0.0, -34.0],
        radius: 8.0,
        size_rating: 8.0,
        dangerous: true,
        ..Default::default()
    }];
    let bent = plan_artillery_position(&artillery_input(&obstacle));

    assert!(
        bent.1 < clean.1,
        "an obstacle off the starboard bow must push the facing to port \
         ({} vs {})",
        bent.1,
        clean.1
    );
    assert_eq!(
        bent.0, clean.0,
        "and it must not touch the throttle: a hold that accelerated around a \
         rock would be flying, not holding"
    );

    // The target itself is excluded from the scan — a hull deliberately
    // holding station on a ship must not treat it as something to swerve
    // around.
    let target_as_obstacle = [AiWorldEntity {
        uuid: Uuid::from_u128(9),
        position: [0.0, 0.0, -180.0],
        radius: 400.0,
        size_rating: 400.0,
        dangerous: true,
        ..Default::default()
    }];
    assert_eq!(
        plan_artillery_position(&artillery_input(&target_as_obstacle)).1,
        clean.1,
        "the target must be excluded from the avoidance scan"
    );
}

// ── The shield-recovery standoff orbit (issue #788) ──────────────────────

fn orbit_input(entities: &[AiWorldEntity]) -> RecoveryOrbitInput<'_> {
    RecoveryOrbitInput {
        self_pos: [0.0, 0.0, 0.0],
        self_yaw: 0.0,
        self_speed: 10.0,
        self_radius: 1.0,
        // Target dead ahead at 200; the ring below sits at 200 too, so the
        // default fixture starts exactly ON the ring.
        target_pos: [0.0, 0.0, -200.0],
        target_uuid: Uuid::nil(),
        safe_range: 200.0,
        orbit_direction: 1.0,
        spiral_gain: 1.2,
        orbit_speed: 0.7,
        tracking_deadband_rad: 0.02,
        tracking_full_steer_rad: 0.5,
        entities,
        avoidance_buffer: AVOIDANCE_BUFFER,
        avoidance_look_ahead_secs: AVOIDANCE_LOOK_AHEAD_SECS,
        hazard_threat_exponent: HAZARD_THREAT_EXPONENT,
    }
}

/// The heading the orbit commands, in world radians, recovered from the
/// steering it demands. Only meaningful for the fixture's yaw of 0 and a
/// non-saturated turn, which is why the tests below check the SIGN of the
/// steering rather than reconstructing angles.
fn orbit_steer(input: &RecoveryOrbitInput) -> f32 {
    plan_recovery_orbit(input).1
}

/// AC3, the core claim: on the ring the ship flies the TANGENT — it neither
/// closes nor opens. With the target dead ahead and the ring at the current
/// range, a tangential course is a hard turn, not "carry on" and not "stop".
#[test]
fn on_the_ring_the_orbit_flies_the_tangent() {
    let none: [AiWorldEntity; 0] = [];
    let input = orbit_input(&none);
    let (thrust, steering) = plan_recovery_orbit(&input);
    assert!(
        (thrust - 0.7).abs() < 1e-6,
        "the ring is flown at the authored orbit throttle, not coasted"
    );
    assert!(
        steering.abs() > 0.9,
        "a target dead ahead means the tangent is 90 degrees off the bow: \
         the orbit must command a hard turn, got {steering}"
    );
}

/// AC3's "spirals rather than stopping or retreating indefinitely".
///
/// The ship is pointed ALONG the pure tangent, so the steering the orbit
/// demands is exactly the spiral correction and nothing else: zero means
/// "hold the ring", and the sign says which way the correction bends. With
/// the target dead ahead of the ring's centre and a starboard-hand orbit, a
/// positive demand turns further off the target (opening the range) and a
/// negative one turns back onto it (closing).
#[test]
fn the_orbit_spirals_outward_when_inside_and_inward_when_outside() {
    let none: [AiWorldEntity; 0] = [];
    // Facing +X: for a target dead astern-of-ring at -Z, that is the pure
    // starboard-hand tangent.
    let tangent_yaw = std::f32::consts::FRAC_PI_2;

    let mut on_ring = orbit_input(&none);
    on_ring.self_yaw = tangent_yaw;
    assert_eq!(
        plan_recovery_orbit(&on_ring).1,
        0.0,
        "already on the ring and already on the tangent: no correction at all"
    );

    let mut inside = orbit_input(&none);
    inside.self_yaw = tangent_yaw;
    inside.target_pos = [0.0, 0.0, -60.0]; // range 60 vs a 200 ring
    let inside_steer = plan_recovery_orbit(&inside).1;

    let mut outside = orbit_input(&none);
    outside.self_yaw = tangent_yaw;
    outside.target_pos = [0.0, 0.0, -600.0]; // range 600 vs a 200 ring
    let outside_steer = plan_recovery_orbit(&outside).1;

    assert!(
        inside_steer > 0.0,
        "inside the ring the orbit must bend AWAY from the target and work \
         its way out, got {inside_steer}"
    );
    assert!(
        outside_steer < 0.0,
        "outside the ring it must bend BACK toward the target rather than \
         running away indefinitely, got {outside_steer}"
    );
    // And it never stops: the throttle is the same on the ring and off it.
    assert_eq!(
        plan_recovery_orbit(&inside).0,
        plan_recovery_orbit(&on_ring).0
    );
    assert_eq!(
        plan_recovery_orbit(&outside).0,
        plan_recovery_orbit(&on_ring).0
    );
}

/// The gain is fractional, so the same authored value produces the same
/// correction for a small ring and a large one. Without this a designer
/// would have to re-tune `orbit_spiral_gain` every time a weapon's range
/// changed, which is exactly the coupling the safe ring exists to avoid.
#[test]
fn the_spiral_correction_is_scale_free() {
    let none: [AiWorldEntity; 0] = [];
    let mut small = orbit_input(&none);
    small.safe_range = 80.0;
    small.target_pos = [0.0, 0.0, -40.0]; // 50% of the ring
    let mut large = orbit_input(&none);
    large.safe_range = 800.0;
    large.target_pos = [0.0, 0.0, -400.0]; // also 50% of the ring
    assert!(
        (orbit_steer(&small) - orbit_steer(&large)).abs() < 1e-5,
        "the same FRACTIONAL error must produce the same correction: {} vs {}",
        orbit_steer(&small),
        orbit_steer(&large)
    );
}

/// The circulation direction is an input, and reversing it reverses the turn
/// — which is what makes a seeded ±1 a meaningful choice rather than
/// decoration.
#[test]
fn reversing_the_orbit_direction_reverses_the_turn() {
    let none: [AiWorldEntity; 0] = [];
    let mut cw = orbit_input(&none);
    cw.orbit_direction = 1.0;
    let mut ccw = orbit_input(&none);
    ccw.orbit_direction = -1.0;
    let (a, b) = (orbit_steer(&cw), orbit_steer(&ccw));
    assert!(
        a * b < 0.0,
        "the two directions must turn opposite ways, got {a} and {b}"
    );
}

/// AC3 again, at the pure layer: a hazard bends the orbit the same way it
/// bends the escape, and the throttle is untouched.
#[test]
fn hazard_bends_the_orbit_without_changing_its_throttle() {
    let none: [AiWorldEntity; 0] = [];
    // Put the ship well inside the ring so the commanded course is nearly
    // straight ahead and a rock ahead of it is genuinely in the way.
    let mut clear_input = orbit_input(&none);
    clear_input.target_pos = [200.0, 0.0, 0.0];
    clear_input.safe_range = 200.0;
    let clear = plan_recovery_orbit(&clear_input);

    let rock = [AiWorldEntity {
        uuid: Uuid::new_v4(),
        position: [3.0, 0.0, -30.0],
        radius: 2.0,
        size_rating: 2.0,
        ..Default::default()
    }];
    let mut bent_input = orbit_input(&rock);
    bent_input.target_pos = [200.0, 0.0, 0.0];
    bent_input.safe_range = 200.0;
    let bent = plan_recovery_orbit(&bent_input);

    assert_ne!(
        bent.1, clear.1,
        "a hazard on the orbit path must bend the orbit steering"
    );
    assert_eq!(
        bent.0, clear.0,
        "avoidance bends the heading, never the leg's throttle"
    );
}

/// Degenerate geometry must not produce NaN steering: sitting on top of the
/// target, and an un-derivable ring, both hold the current heading.
#[test]
fn degenerate_orbit_geometry_holds_the_current_heading() {
    let none: [AiWorldEntity; 0] = [];
    let mut on_top = orbit_input(&none);
    on_top.target_pos = on_top.self_pos;
    let (thrust, steering) = plan_recovery_orbit(&on_top);
    assert!(steering.is_finite() && thrust.is_finite());
    assert_eq!(
        steering, 0.0,
        "already on the held heading: no turn demanded"
    );

    let mut no_ring = orbit_input(&none);
    no_ring.safe_range = 0.0;
    let (_, steering) = plan_recovery_orbit(&no_ring);
    assert!(steering.is_finite());
    assert_eq!(steering, 0.0);
}
