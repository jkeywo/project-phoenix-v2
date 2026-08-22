use super::*;
use crate::entities::config::*;

/// Helper: build a minimal Bevy app for spawning tests.
fn test_app() -> App {
    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    app
}

/// Call `spawn_entity` then flush commands via app.update() so components
/// are queryable.
fn spawn_and_flush(
    app: &mut App,
    config: &EntityConfig,
    position: Vec3,
    uuid: String,
    id: Option<String>,
) -> Entity {
    let entity = {
        let mut commands = app.world_mut().commands();
        spawn_entity(&mut commands, config, position, uuid, id)
    };
    app.update();
    entity
}

// ── Issue #1025: `[infrastructure]`, on both authoring paths ──

/// The exemplar depot template, trimmed to the two blocks under test.
const DEPOT_TOML: &str = r#"
[hull]
hull_integrity = 400.0

[infrastructure]
condition_max = 100.0
hull_damage_share = 0.5

[[infrastructure.capacity]]
id = "depot_transfer_throughput"
amount = 40

[[infrastructure.threshold]]
flag = "depot_transfer_capable"
fails_below = 0.4
"#;

fn lenient(source: &str) -> EntityConfig {
    EntityConfig::from_toml_in_mode(
        source,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("the fixture parses")
}

/// **AC1.** A template that authors `[infrastructure]` spawns with a live
/// condition track; one that does not spawns exactly as it did before the
/// section existed.
///
/// Both directions, because a gate that only ever reads true would pass the
/// first half alone — and the second half is the whole "omitting it changes
/// nothing" claim.
#[test]
fn an_authored_infrastructure_table_attaches_a_condition_track_and_omitting_it_attaches_none() {
    let mut app = test_app();
    let e = spawn_and_flush(
        &mut app,
        &lenient(DEPOT_TOML),
        Vec3::ZERO,
        "depot".into(),
        None,
    );
    let track = app
        .world()
        .get::<crate::infrastructure::InfrastructureCondition>(e)
        .expect("an authored [infrastructure] table must attach the track");
    assert_eq!(
        track.0.condition(),
        100.0,
        "intact unless authored otherwise"
    );
    assert_eq!(
        track.0.capacity("depot_transfer_throughput"),
        Some(40),
        "the authored capacity travels onto the entity"
    );
    assert_eq!(
        track.0.flag("depot_transfer_capable"),
        Some(true),
        "and its operational flag starts level-evaluated against the condition"
    );

    let mut app = test_app();
    let e = spawn_and_flush(
        &mut app,
        &lenient("[hull]\nhull_integrity = 400.0\n"),
        Vec3::ZERO,
        "plain".into(),
        None,
    );
    assert!(
        app.world()
            .get::<crate::infrastructure::InfrastructureCondition>(e)
            .is_none(),
        "an entity that authors no [infrastructure] must carry no track — every station, \
             asteroid and hull in the repository is in this arm"
    );
    assert!(
        app.world().get::<EntitySystemHull>(e).is_some(),
        "…and is otherwise spawned exactly as before, hull and all"
    );
}

/// **AC1.** The world's `[[entity]].overrides` path reaches the same table.
///
/// This is the half a template test cannot cover: a scenario placing a
/// shared depot has to be able to say "this one arrives already battered"
/// without forking the template.
#[test]
fn a_world_entity_override_retunes_the_authored_infrastructure_table() {
    let overrides: toml::Value = toml::from_str("[infrastructure]\ncondition = 80.0\n")
        .expect("the override document parses");
    let merged = crate::entities::loader::apply_overrides(&lenient(DEPOT_TOML), &overrides)
        .expect("the override merges onto the template");
    let infrastructure = merged
        .infrastructure
        .as_ref()
        .expect("the merged config still has the table");
    assert_eq!(
        infrastructure.condition,
        Some(80.0),
        "the world's starting condition wins"
    );
    assert_eq!(
        infrastructure.condition_max, 100.0,
        "…while everything the override was silent about survives from the template"
    );
    assert_eq!(
        infrastructure.capacities.len(),
        1,
        "including the authored capacity, which a plain table-replacing merge would have \
             dropped"
    );
    assert_eq!(infrastructure.thresholds.len(), 1, "…and the threshold");

    let mut app = test_app();
    let e = spawn_and_flush(&mut app, &merged, Vec3::ZERO, "depot".into(), None);
    let track = app
        .world()
        .get::<crate::infrastructure::InfrastructureCondition>(e)
        .expect("the merged config still attaches a track");
    assert_eq!(track.0.condition(), 80.0);
}

// ── Issue #1158: `[held_response]`, on both authoring paths ──

/// **AC1.** A target that authors `[held_response]` spawns carrying it; one
/// that authors nothing carries no component and is merely held in place.
///
/// Both directions: the second arm is the whole "a target that authors
/// nothing is merely held in place" claim — every derelict and structure
/// written before this existed is in it.
#[test]
fn an_authored_held_response_attaches_a_section_and_omitting_it_attaches_none() {
    let mut app = test_app();
    let e = spawn_and_flush(
        &mut app,
        &lenient(
            "[hull]\nhull_integrity = 400.0\n\n[held_response]\nkind = \"arrest-decline\"\n\
                 recover_per_sec = 20.0\n",
        ),
        Vec3::ZERO,
        "structure".into(),
        None,
    );
    let held = app
        .world()
        .get::<crate::tractor::HeldResponseSection>(e)
        .expect("an authored [held_response] table must attach the section");
    assert_eq!(held.0.kind, crate::tractor::HeldResponseKind::ArrestDecline);
    assert_eq!(held.0.recover_per_sec, Some(20.0));

    let mut app = test_app();
    let e = spawn_and_flush(
        &mut app,
        &lenient("[hull]\nhull_integrity = 400.0\n"),
        Vec3::ZERO,
        "plain".into(),
        None,
    );
    assert!(
        app.world()
            .get::<crate::tractor::HeldResponseSection>(e)
            .is_none(),
        "a target that authors no [held_response] carries no component and is merely held in \
             place — every derelict and structure before this slice is in this arm"
    );
}

/// **AC1.** The world's `[[entity]].overrides` path reaches the table too —
/// the authoring path `probe_held_response.toml` uses to make a shipped depot
/// arrest-decline without forking the template.
#[test]
fn a_world_entity_override_authors_a_held_response_onto_a_target() {
    let overrides: toml::Value =
        toml::from_str("[held_response]\nkind = \"arrest-decline\"\nrecover_per_sec = 20.0\n")
            .expect("the override document parses");
    let merged = crate::entities::loader::apply_overrides(&lenient(DEPOT_TOML), &overrides)
        .expect("the override merges onto a target that authored none");
    let held = merged
        .held_response
        .as_ref()
        .expect("the merged config carries the authored held-response");
    assert_eq!(held.kind, crate::tractor::HeldResponseKind::ArrestDecline);

    let mut app = test_app();
    let e = spawn_and_flush(&mut app, &merged, Vec3::ZERO, "depot".into(), None);
    assert!(
        app.world()
            .get::<crate::tractor::HeldResponseSection>(e)
            .is_some(),
        "…and the merged target spawns carrying it, over its condition track"
    );
}

/// **AC7.** A `[held_response]` table whose fields do not match its kind is a
/// load error naming the field, not a hold that silently arrests nothing.
#[test]
fn an_unauthorable_held_response_fails_the_entity_load() {
    let err = EntityConfig::from_toml_in_mode(
        "[hull]\nhull_integrity = 400.0\n\n[held_response]\nkind = \"arrest-decline\"\n",
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect_err("arrest-decline with no recover_per_sec must be refused");
    assert!(
        err.to_string().contains("recover_per_sec"),
        "the refusal must name the missing field, got {err}"
    );
}

/// **AC1.** An `[infrastructure]` table that cannot mean anything is a load
/// error naming the field, not a structure that silently never degrades.
#[test]
fn an_unauthorable_infrastructure_table_fails_the_entity_load() {
    let source = format!(
        "{DEPOT_TOML}\n[[infrastructure.threshold]]\nflag = \"other\"\nfails_below = 40.0\n"
    );
    let err = EntityConfig::from_toml_in_mode(
        &source,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect_err("a threshold authored in points rather than fractions must be refused");
    assert!(
        err.to_string().contains("FRACTION"),
        "the refusal must say what the field wanted, got {err}"
    );
}

/// A `[repair]` table that exists only to carry `[repair.selector]` gives
/// the ship NO repair teams.
///
/// This is the gate #885b stage 5b had to move. Every hull now authors
/// `[repair.selector]`, and TOML cannot write that sub-table without
/// bringing `[repair]` into existence — so the old "the block is present"
/// gate would have crewed two repair teams onto six NPC hulls that never had
/// any, purely as a side effect of a table header. The gate is the COUNT:
/// a ship has repair teams when its TOML says how many.
///
/// Both directions are asserted, because a gate that only ever reads false
/// would pass the first half alone.
#[test]
fn repair_teams_are_gated_on_the_authored_count_not_on_the_repair_table() {
    let selector_only = r#"
[hull]
hull_integrity = 100.0

[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
directive_kind = "Destroy"
base_priority = 40.0

[repair.selector]
sources = ["damaged-stations"]
horizon = 1e9
switch_margin = 0.0
eligibility = "candidate_fact(source_repair_request) > 0"
"#;
    let config = EntityConfig::from_toml_in_mode(
        selector_only,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("selector-only [repair] parses");
    assert!(
        config
            .repair
            .as_ref()
            .is_some_and(|r| r.selector.is_some() && !r.declares_teams()),
        "precondition: `[repair.selector]` alone brings `[repair]` into existence \
             but declares no teams"
    );
    let mut app = test_app();
    let e = spawn_and_flush(&mut app, &config, Vec3::ZERO, "selector-only".into(), None);
    assert!(
        app.world()
            .get::<crate::console::repair::server::ShipRepairTeams>(e)
            .is_none(),
        "a `[repair]` table carrying only a selector must NOT crew repair teams — \
             authoring a ranking policy is not the same as declaring the capability it \
             ranks for."
    );
    assert!(
        app.world()
            .get::<crate::console::repair::server::RepairTargetSelector>(e)
            .is_some(),
        "…while the selector itself is still attached: the teams gate dispatch, so \
             a ship that gains them later already has its ranking."
    );

    let with_teams = format!("{selector_only}\n[repair]\nrepair_team_count = 3\n");
    let config = EntityConfig::from_toml_in_mode(
        &with_teams,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("[repair] with a count parses");
    let mut app = test_app();
    let e = spawn_and_flush(&mut app, &config, Vec3::ZERO, "with-teams".into(), None);
    let teams = app
        .world()
        .get::<crate::console::repair::server::ShipRepairTeams>(e)
        .expect("an authored `repair_team_count` must crew that many teams");
    assert_eq!(
        teams.0.slots().len(),
        3,
        "the authored count is the team count, verbatim"
    );
}

#[test]
fn spawn_entity_with_comms_inserts_comms_range_component() {
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec![],
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: Some(crate::entities::config::CommsConfig {
            range: 8000.0,
            hailable: false,
            display_name: None,
        }),
        asteroid_field: None,
        shape: None,
        effects: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        star: None,
        planet: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    let world = app.world_mut();
    let range = world
        .get::<crate::comms::CommsRange>(spawned)
        .expect("CommsRange component should be inserted when [comms] is present");
    assert_eq!(range.0, 8000.0);
    assert!(
        world.get::<crate::comms::CommsHailable>(spawned).is_none(),
        "a range-only [comms] block must NOT put the entity on the hail roster (#985)"
    );
}

/// Issue #985: `[comms] hailable = true` is the opt-in that puts an entity
/// on the hail roster, and `display_name` rides along as the contact label.
#[test]
fn spawn_entity_with_hailable_comms_inserts_the_opt_in_marker() {
    let mut app = test_app();
    let config = EntityConfig {
        name: Some("world.entity.outpost.name".into()),
        comms: Some(crate::entities::config::CommsConfig {
            range: 800.0,
            hailable: true,
            display_name: Some("Relay Outpost".into()),
        }),
        ..EntityConfig::default()
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    let world = app.world_mut();
    let hailable = world
        .get::<crate::comms::CommsHailable>(spawned)
        .expect("hailable = true must insert CommsHailable");
    assert_eq!(hailable.display_name.as_deref(), Some("Relay Outpost"));
}

#[test]
fn spawn_entity_without_comms_omits_comms_range_component() {
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec![],
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        star: None,
        planet: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    let world = app.world_mut();
    assert!(world.get::<crate::comms::CommsRange>(spawned).is_none());
}

#[test]
fn spawn_entity_with_name_inserts_entity_name_component() {
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: Some("Sun".to_string()),
        display_name: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec![],
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        star: None,
        planet: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let world = app.world_mut();
    let name_comp = world
        .get::<EntityName>(spawned)
        .expect("should have EntityName");
    assert_eq!(name_comp.0, "Sun");
}

#[test]
fn spawn_entity_without_name_omits_entity_name_component() {
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec![],
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        star: None,
        planet: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let world = app.world_mut();
    assert!(world.get::<EntityName>(spawned).is_none());
}

#[test]
fn spawn_entity_with_collider_has_rapier_components() {
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec![],
        collider: Some(ColliderConfig {
            shape: ColliderShape::Ball,
            radius: 3.0,
            length: 0.0,
            half_height: None,
            movable: true,
        }),
        hull: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let world = app.world_mut();
    assert!(
        world.get::<ColliderSection>(spawned).is_some(),
        "should have ColliderSection"
    );
    assert!(
        world.get::<Collider>(spawned).is_some(),
        "should have Rapier Collider"
    );
    assert!(
        world.get::<RigidBody>(spawned).is_some(),
        "should have RigidBody"
    );
}

/// The `Cylinder` variant's whole meaning is the rapier body it builds, so
/// this asserts on that body rather than on the config that produced it.
/// The half-height goes in FIRST and the radius second: getting the two the
/// wrong way round would build a 17-tall, 7-wide pillar out of a station
/// that is 34 across and 14 tall, and no other assertion in the tree would
/// notice.
///
/// The numbers are the shipped starbase's, so this pins the axis too — a
/// `Collider::cylinder` is Y-axis by construction, and the disc it makes is
/// the one a station is.
#[test]
fn a_cylinder_collider_spawns_a_y_axis_disc_of_the_authored_size() {
    let mut app = test_app();
    let config = EntityConfig {
        collider: Some(ColliderConfig {
            shape: ColliderShape::Cylinder,
            radius: 17.04,
            length: 0.0,
            half_height: Some(7.16),
            movable: false,
        }),
        ..Default::default()
    };

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let collider = app
        .world()
        .get::<Collider>(spawned)
        .expect("a [collider] section must build a rapier Collider");
    let cylinder = collider
        .as_cylinder()
        .expect("shape = \"Cylinder\" must build a rapier cylinder, not a ball or a capsule");
    assert!(
        (cylinder.half_height() - 7.16).abs() < 1e-5,
        "expected the authored half-height, got {}",
        cylinder.half_height()
    );
    assert!(
        (cylinder.radius() - 17.04).abs() < 1e-5,
        "expected the authored radius, got {}",
        cylinder.radius()
    );
}

/// A degenerate `Cylinder` cannot arrive from a TOML template — the load
/// path rejects one — but `spawn_entity` takes an `EntityConfig`, and a
/// caller that builds one in code bypasses that. The fallback errs UPWARDS
/// to the enclosing sphere rather than collapsing to nothing: a zero-height
/// disc is a structure ships fly straight through, an over-tall one merely
/// stops them early, and only one of those is a bug worth shipping.
#[test]
fn a_cylinder_built_in_code_without_a_half_height_falls_back_to_its_radius() {
    let mut app = test_app();
    let config = EntityConfig {
        collider: Some(ColliderConfig {
            shape: ColliderShape::Cylinder,
            radius: 3.8,
            length: 0.0,
            half_height: None,
            movable: false,
        }),
        ..Default::default()
    };

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let collider = app.world().get::<Collider>(spawned).unwrap();
    let cylinder = collider.as_cylinder().expect("still a cylinder");
    assert!(
        (cylinder.half_height() - 3.8).abs() < 1e-5,
        "an unauthored half-height must fall back to the radius, not to zero; got {}",
        cylinder.half_height()
    );
}

#[test]
fn spawn_entity_with_lights_inserts_lights_component() {
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: vec![LightConfig {
            kind: LightKind::Point,
            colour: [1.0, 0.95, 0.85],
            intensity: 150000.0,
            range: Some(5000.0),
            face_player: false,
        }],
        tags: vec![],
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        star: None,
        planet: None,
        ship_config: None,
        shield_arcs: Vec::new(),
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let world = app.world_mut();
    let lights = world.get::<Lights>(spawned).expect("should have Lights");
    assert_eq!(lights.0.len(), 1);
    assert_eq!(lights.0[0].kind, LightKind::Point);
    assert_eq!(lights.0[0].range, Some(5000.0));
}

#[test]
fn spawn_entity_with_asteroid_field_section() {
    use crate::entities::config::AsteroidFieldConfig;
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec!["field".to_string()],
        asteroid_field: Some(AsteroidFieldConfig {
            inner_radius: 100.0,
            outer_radius: 200.0,
            density: 0.005,
            weight: 1.0,
            spawn_distance: 150.0,
            despawn_distance: 250.0,
            asteroid_type_paths: vec!["small.toml".into()],
            cosmetic_type_paths: vec![],
            tags: vec![],
            grid: None,
            shield_pierce: 0.0,
            shape: None,
            anchor: None,
            anchor_offset: [0.0, 0.0, 0.0],
            random_rotation: None,
        }),
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        shape: None,
        effects: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let world = app.world_mut();
    let field = world
        .get::<AsteroidFieldSection>(spawned)
        .expect("should have AsteroidFieldSection");
    assert!((field.0.inner_radius - 100.0).abs() < 1e-6);
}

#[test]
fn spawn_entity_with_appearance_section() {
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec![],
        appearance: Some(AppearanceConfig {
            colour: "#ff0000".to_string(),
            size_min: 1.0,
            size_max: 3.0,
        }),
        hull: None,
        collider: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let world = app.world_mut();
    let appearance = world
        .get::<AppearanceSection>(spawned)
        .expect("should have AppearanceSection");
    assert_eq!(appearance.0.colour, "#ff0000");
}

#[test]
fn spawn_entity_with_id_carries_id_component() {
    let mut app = test_app();
    let config = EntityConfig::from_toml("").unwrap();

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(
        &mut app,
        &config,
        Vec3::ZERO,
        uuid,
        Some("player-ship".to_string()),
    );

    let world = app.world_mut();
    let id_comp = world
        .get::<EntityId>(spawned)
        .expect("should have EntityId");
    assert_eq!(id_comp.0, "player-ship");
}

#[test]
fn spawn_entity_without_id_has_no_id_component() {
    let mut app = test_app();
    let config = EntityConfig::from_toml("").unwrap();

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let world = app.world_mut();
    assert!(
        world.get::<EntityId>(spawned).is_none(),
        "should NOT have EntityId"
    );
}

/// Issue #1154: the authored `mass` — already defaulted at parse time —
/// carries straight onto the spawned entity, unconditionally, exactly as
/// [`EntityUuid`] does.
#[test]
fn spawn_entity_carries_authored_mass_onto_entity_mass_component() {
    let mut app = test_app();
    let config = EntityConfig::from_toml("mass = 45000.0\n").unwrap();

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let world = app.world_mut();
    let mass = world
        .get::<EntityMass>(spawned)
        .expect("every spawned entity must carry EntityMass");
    assert_eq!(mass.0, 45_000.0);
}

/// An entity that authors no `mass` still gets a real, non-zero weight on
/// the spawned entity — the documented default rides through the spawner
/// exactly as an authored value does, never falling back to a bare zero.
#[test]
fn spawn_entity_without_authored_mass_carries_the_documented_default() {
    let mut app = test_app();
    let config = EntityConfig::from_toml("").unwrap();

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let world = app.world_mut();
    let mass = world
        .get::<EntityMass>(spawned)
        .expect("every spawned entity must carry EntityMass");
    assert_eq!(mass.0, crate::entities::config::DEFAULT_ENTITY_MASS);
    assert!(
        mass.0 > 0.0,
        "an unauthored entity must never spawn at zero mass"
    );
}

#[test]
fn spawn_entity_with_region_shape_and_effects() {
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec!["region".to_string(), "nebula".to_string()],
        shape: Some(RegionShape::Sphere { radius: 150.0 }),
        effects: Some(crate::regions::effects::RegionEffectsConfig {
            comms_jammed: Some(crate::regions::effects::CommsJamEffect {}),
            sensor_blind: Some(crate::regions::effects::SensorBlindEffect {}),
            ..Default::default()
        }),
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
    };

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::new(100.0, 0.0, 50.0), uuid, None);

    let world = app.world_mut();
    let shape_comp = world
        .get::<RegionShapeSection>(spawned)
        .expect("should have RegionShapeSection");
    assert_eq!(shape_comp.0, RegionShape::Sphere { radius: 150.0 });

    let effects_comp = world
        .get::<RegionEffectsSection>(spawned)
        .expect("should have RegionEffectsSection");
    assert_eq!(effects_comp.0.len(), 2);
    assert!(effects_comp
        .0
        .contains(&crate::regions::effects::RegionEffectKind::CommsJam));
    assert!(effects_comp
        .0
        .contains(&crate::regions::effects::RegionEffectKind::SensorBlind));
}

#[test]
fn spawn_entity_with_shape_alone_has_no_effects_comp() {
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec!["region".to_string()],
        shape: Some(RegionShape::Sphere { radius: 100.0 }),
        effects: None,
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let world = app.world_mut();
    assert!(
        world.get::<RegionShapeSection>(spawned).is_some(),
        "should have RegionShapeSection"
    );
    assert!(
        world.get::<RegionEffectsSection>(spawned).is_none(),
        "should NOT have RegionEffectsSection"
    );
}

#[test]
fn spawn_entity_with_faction_uuid_has_faction_component() {
    let mut app = test_app();
    let faction_id = uuid::Uuid::parse_str("aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa").unwrap();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec![],
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: Some(faction_id),
        hull: None,
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    let world = app.world_mut();
    let comp = world
        .get::<FactionComponent>(spawned)
        .expect("should have FactionComponent");
    assert_eq!(comp.0, faction_id);
}

#[test]
fn spawn_entity_without_faction_has_no_faction_component() {
    let mut app = test_app();
    let config = EntityConfig::from_toml("").unwrap();
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    let world = app.world_mut();
    assert!(
        world.get::<FactionComponent>(spawned).is_none(),
        "should NOT have FactionComponent"
    );
}

#[test]
fn spawn_entity_position_matches_input() {
    let mut app = test_app();
    let config = EntityConfig::from_toml("").unwrap();

    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::new(42.0, 0.0, -7.0), uuid, None);

    let world = app.world_mut();
    let transform = world
        .get::<Transform>(spawned)
        .expect("should have Transform");
    assert_eq!(transform.translation.x, 42.0);
    assert_eq!(transform.translation.z, -7.0);
}

// -- EntitySystemHull component tests --

#[test]
fn spawn_entity_with_hull_integrity_attaches_captain_chair_slot() {
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec![],
        hull: Some(crate::entities::config::HullConfig {
            hull_integrity: 60.0,
            ..Default::default()
        }),
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    let world = app.world_mut();
    let hull_comp = world
        .get::<EntitySystemHull>(spawned)
        .expect("should have EntitySystemHull when hull_integrity > 0");
    assert!(
        (hull_comp.0.total_max() - 60.0).abs() < 1e-6,
        "max HP should be 60"
    );
    assert!(
        (hull_comp.0.total_current() - 60.0).abs() < 1e-6,
        "current HP should start at 60"
    );
}

#[test]
fn spawn_entity_without_hull_has_no_entity_console_hull() {
    let mut app = test_app();
    let config = EntityConfig::from_toml("").unwrap();
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    let world = app.world_mut();
    assert!(
        world.get::<EntitySystemHull>(spawned).is_none(),
        "entity with no hull config must not have EntitySystemHull"
    );
}

// ── ShipShields spawner attachment tests ────────────────────────────────

#[test]
fn spawn_entity_with_shields_console_block_attaches_ship_shields() {
    let mut app = test_app();
    let toml = r#"
[hull]
hull_integrity = 60.0

[shields_console.base]
num_facings = 1
max_hp = 30
regen_per_sec = 1.5
"#;
    let config = EntityConfig::from_toml(toml).expect("toml must parse");
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    let shields = app
        .world()
        .get::<crate::ship::shields::ShipShields>(spawned)
        .expect("entity with [shields_console] block must have ShipShields component");
    assert_eq!(shields.0.facings.len(), 1);
    assert_eq!(shields.0.facings[0].max_hp, 30);
    assert_eq!(shields.0.facings[0].hp, 30);
    assert_eq!(shields.0.facings[0].regen_per_sec, 1.5);
    assert!(shields.0.facings[0].is_online());
}

#[test]
fn spawn_entity_without_shields_console_block_omits_ship_shields() {
    let mut app = test_app();
    let toml = r#"
[hull]
hull_integrity = 60.0
"#;
    let config = EntityConfig::from_toml(toml).expect("toml must parse");
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    assert!(
        app.world()
            .get::<crate::ship::shields::ShipShields>(spawned)
            .is_none(),
        "entity without [shields_console] block must not have ShipShields"
    );
}

/// A `[shields_console]` that carries only an AI policy is not a shield
/// system.
///
/// This is the shape #885b stage 5c created: `[shields_console.ai_policy]`
/// is a required declaration on every AI-bearing hull, and writing it brings
/// `[shields_console]` into existence whether or not the hull has shields.
/// `ship_requiem_courier.toml` is exactly this — no `[[shield_arc]]`, no
/// `[shields_console.base]` — and it must keep having no shields at all, so
/// the gate reads the shield CONTENT rather than the table header. The rule
/// change rides along: a block authoring neither used to mean a default
/// four-facing system and now means none.
#[test]
fn a_shields_console_holding_only_an_ai_policy_attaches_no_ship_shields() {
    let mut app = test_app();
    let toml = r#"
[hull]
hull_integrity = 60.0

[shields_console.ai_policy]

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "true"
verb = "focus_shield_arc"
"#;
    let config = EntityConfig::from_toml(toml).expect("toml must parse");
    assert!(
        config.shields_console.is_some() && config.shield_arcs.is_empty(),
        "precondition: the table exists but declares no shields"
    );
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    assert!(
        app.world()
            .get::<crate::ship::shields::ShipShields>(spawned)
            .is_none(),
        "declaring a shields-focus POLICY must not conjure a shield system onto a \
             hull that carries no arcs and no base config"
    );
}

/// …and a hull with arcs is unaffected by which of the two branches it takes.
///
/// The five Harrow hulls each declare one `[[shield_arc]]` and gained a
/// `[shields_console]` in stage 5c, moving them from the arcs-only fallback
/// branch onto the console branch. Both paths must build the same system, or
/// authoring a policy would have silently retuned five ships' shields.
#[test]
fn arcs_build_the_same_shield_system_with_or_without_a_shields_console() {
    const ARC: &str = r#"
[hull]
hull_integrity = 60.0

[[shield_arc]]
id = "all"
label = "test.arc"
center_deg = 0.0
width_deg = 360.0
max_hp = 5
regen_per_sec = 0.0
"#;
    let read = |toml: &str| {
        let mut app = test_app();
        let config = EntityConfig::from_toml(toml).expect("toml must parse");
        let uuid = uuid::Uuid::new_v4().to_string();
        let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
        let s = app
            .world()
            .get::<crate::ship::shields::ShipShields>(spawned)
            .expect("a hull with an arc has shields");
        (
            format!("{:?}", s.0.facings),
            format!("{:?}", s.0.focus_config),
            s.1,
        )
    };
    let (arcs_only, focus_only, freq_only) = read(ARC);
    let (arcs_console, focus_console, freq_console) = read(&format!("{ARC}\n[shields_console]\n"));
    assert_eq!(arcs_only, arcs_console, "the facings must be identical");
    assert_eq!(
        focus_only, focus_console,
        "an unauthored `[shields_console]` must supply exactly \
             ShieldFocusConfig::default(), or stage 5c retuned five hulls' focus \
             bonuses by writing a policy next door"
    );
    assert_eq!(freq_only, freq_console, "the first arc owns the frequency");
}

#[test]
fn hull_integrity_maps_to_captain_chair_slot() {
    // Stations and asteroids still use hull_integrity in TOML â€” must keep working.
    let mut app = test_app();
    let config = EntityConfig {
        reference_grid: None,
        name: None,
        display_name: None,
        star: None,
        planet: None,
        class: None,
        hull_id: None,
        power_rating: None,
        mass: crate::entities::config::DEFAULT_ENTITY_MASS,
        css: None,
        light: Vec::new(),
        ship_config: None,
        shield_arcs: Vec::new(),
        tags: vec![],
        hull: Some(crate::entities::config::HullConfig {
            hull_integrity: 200.0,
            ..Default::default()
        }),
        collider: None,
        appearance: None,
        helm_console: None,
        helm_capability: None,
        weapons_console: None,
        engineering_console: None,
        captain_console: None,
        comms_console: None,
        power: None,
        sensors_console: None,
        navigation_console: None,
        shields_console: None,
        torpedoes: None,
        repair: None,
        audio: None,
        comms: None,
        asteroid_field: None,
        shape: None,
        effects: None,
        infrastructure: None,
        scan: None,
        tractor: None,
        held_response: None,
        dock: None,
        umbilical: None,
        civilian: None,
        faction: None,
        behaviour: None,
        radar_appearance: None,
        target: None,
        mesh: None,
        cinematic_camera: None,
        ai_profile: None,
        lod_bubble: None,
    };
    let uuid = uuid::Uuid::new_v4().to_string();
    let spawned = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);
    let world = app.world_mut();
    let hull_comp = world
        .get::<EntitySystemHull>(spawned)
        .expect("entity with hull_integrity should still get EntitySystemHull");
    assert!((hull_comp.0.total_max() - 200.0).abs() < 1e-6);
    let entries: Vec<_> = hull_comp.0.entries().collect();
    assert_eq!(
        entries[0].0,
        &crate::core::messages::SystemId("captain".to_string())
    );
}

// -- Channel-3 NPC routing smoke test (#552) --------------------------------

#[test]
fn npc_channel3_coordination_is_consumed() {
    // Pure routing logic: when both sender and target are Ai-controlled,
    // route_coordination must return Consume (not Popup).
    use crate::ship::control_source::ControlSource;
    use crate::ship::coordination::{route_coordination, DeliverAction};
    assert_eq!(
        route_coordination(ControlSource::Ai, ControlSource::Ai),
        DeliverAction::Consume,
    );
}

// ── #573: NPC all-AI roster ───────────────────────────────────────────────

/// NPC ships spawned with a [behaviour] block must have every registered
/// NPC ships now carry the `Ship` marker (same as player ships).
/// The `LocalShip` marker is the selector for the viewscreen entity.
/// All registered systems must be set to `ControlSource::Ai`.
#[test]
fn npc_ship_spawn_gives_all_ai_roster_and_no_ship_marker() {
    use crate::entities::config::{BehaviourConfig, DoctrineObjective, EntityConfig};
    use crate::server_app::Ship;
    use crate::ship_plugin::ShipSystemControlSources;
    use bevy::prelude::*;

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);

    let config = EntityConfig {
        behaviour: Some(BehaviourConfig {
            doctrine: vec![DoctrineObjective {
                id: "destroy-hostiles".into(),
                text: "Destroy hostiles".into(),
                directive_kind: Some("Destroy".into()),
                base_priority: 35.0,
                ..Default::default()
            }],
            ..Default::default()
        }),
        ..Default::default()
    };

    let mut cmds = app.world_mut().commands();
    let entity = spawn_entity(
        &mut cmds,
        &config,
        bevy::math::Vec3::ZERO,
        "npc-001".into(),
        None,
    );
    app.world_mut().flush();

    // NPC ship MUST carry Ship marker (same as player ship after #581 unification)
    assert!(
        app.world().get::<Ship>(entity).is_some(),
        "NPC ship must carry Ship marker (same as player ship after PRD #581)"
    );

    // All registered systems must be AI-controlled
    let sources = app
        .world()
        .get::<ShipSystemControlSources>(entity)
        .expect("NPC ship must have ShipSystemControlSources");
    let config_comp = app
        .world()
        .get::<crate::ship_plugin::ShipConfigComponent>(entity)
        .expect("NPC ship must have ShipConfigComponent");
    for sys in &config_comp.0.systems {
        let policy = sources.0.policy_for(&sys.id);
        assert!(
            policy.operate_ai,
            "system '{}' must be AI-controlled on NPC ship",
            sys.id.0
        );
        assert!(
            !policy.accept_human_input,
            "system '{}' must not accept human input on NPC ship",
            sys.id.0
        );
    }
}

#[test]
fn npc_ship_gets_shipconfig_from_its_own_toml_stations_and_systems() {
    // Regression test for PRD #597 PR-3 (correct redo): NPC ship TOMLs with
    // [[system]] blocks must produce a ShipConfigComponent containing those
    // systems — not the player ship's config, and not an empty config.
    use bevy::prelude::*;

    let toml = r#"
tags = ["ship", "npc"]

[collider]
shape = "Capsule"
radius = 2.0
length = 4.0

[behaviour]

[[behaviour.doctrine]]
id = "test-doctrine"
text = "Test"
directive_kind = "Destroy"
base_priority = 1.0

[[system]]
id = "helm-thrust"
kind = "helm_thrust"
ai_only = true

[[system]]
id = "tactical-radar"
kind = "tactical_radar"
ai_only = true
"#;
    let config = EntityConfig::from_toml_in_mode(
        toml,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("toml must parse");
    assert!(
        config.ship_config.is_some(),
        "EntityConfig.ship_config must be populated from [[system]] blocks"
    );
    let sc = config.ship_config.as_ref().unwrap();
    // Two authored systems plus the AI-only red-alert capability that issue
    // #749 provisions for every behaviour-driven NPC that omits one.
    assert_eq!(
        sc.systems.len(),
        3,
        "expected two authored systems + provisioned red-alert"
    );
    assert!(
        sc.systems
            .iter()
            .any(|s| s.kind == crate::ship::system_registry::RED_ALERT_KIND && s.ai_only),
        "behaviour NPC must be provisioned an ai_only red-alert system (#749)"
    );
    assert_eq!(sc.stations.len(), 0, "NPCs have no stations");

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    let mut cmds = app.world_mut().commands();
    let entity = spawn_entity(
        &mut cmds,
        &config,
        bevy::math::Vec3::ZERO,
        "npc-shipconfig-test".into(),
        None,
    );
    app.world_mut().flush();

    let comp = app
        .world()
        .get::<crate::ship_plugin::ShipConfigComponent>(entity)
        .expect("NPC ship must have ShipConfigComponent");
    assert_eq!(
        comp.0.systems.len(),
        3,
        "spawned NPC entity carries its two declared systems + provisioned red-alert"
    );
    let system_ids: Vec<&str> = comp.0.systems.iter().map(|s| s.id.0.as_str()).collect();
    assert!(
        system_ids.contains(&"helm-thrust"),
        "helm-thrust system must be present"
    );
    assert!(
        system_ids.contains(&"tactical-radar"),
        "tactical-radar system must be present"
    );
}

/// Issue #839: a world-spawned Alliance hull (i.e. spawned as an NPC, not
/// selected by the player) must present as a plain ship — no `player` tag,
/// ordinary `ship` radar icon. Player identity is injected only at the
/// player game-start spawn (see `player_ship_identity` in `server_app.rs`),
/// so the checked-in template must not author it. Parses the real template
/// so it regresses if the `player` tag / `playerShip` icon creep back in.
#[test]
fn world_spawned_alliance_hull_has_no_player_identity() {
    use bevy::prelude::*;

    // Through the resolver (issue #876): this hull is COMPOSED, so its baked
    // bytes are no longer the document the game spawns.
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/alliance_cruiser.toml",
    )
    .expect("cruiser template must compose and parse");

    let mut app = App::new();
    app.add_plugins(bevy::time::TimePlugin);
    let entity = {
        let mut cmds = app.world_mut().commands();
        spawn_entity(&mut cmds, &config, Vec3::ZERO, "world-cruiser".into(), None)
    };
    app.world_mut().flush();

    let tags = app
        .world()
        .get::<EntityTagsSection>(entity)
        .expect("hull must carry EntityTagsSection");
    assert!(
        tags.0.iter().any(|t| t == "ship"),
        "world-spawned hull keeps the ship tag; got {:?}",
        tags.0
    );
    assert!(
        !tags.0.iter().any(|t| t == "player"),
        "world-spawned hull must NOT carry the player tag; got {:?}",
        tags.0
    );

    let radar = app
        .world()
        .get::<RadarAppearanceSection>(entity)
        .expect("hull must carry RadarAppearanceSection");
    assert_eq!(
        radar.0.icon.as_deref(),
        Some("ship"),
        "world-spawned hull shows the ordinary ship icon, not playerShip"
    );
}

/// Issue #749: a behaviour NPC that omits an explicit red_alert system must
/// still spawn with the provisioned red_alert control source resolving to
/// `Ai` — the causal link that lets `operate_captain_ai` raise its Red
/// Alert. The RNG-coverage escort authors [behaviour] but no red_alert block.
///
/// That template moved out of the shipped fleet in issue #954 — it is a test
/// fixture under `assets/entities/test/`, kept only so `rng_coverage.toml`
/// has a hull that fires all three weapon kinds. It still loads through the
/// real composed path, which is all this test needs of it, and pointing at a
/// fixture is honest about what it is: the #749 provision is a property of
/// the SPAWNER, not of any one shipped hull.
#[test]
fn spawned_behaviour_npc_red_alert_is_ai_operated() {
    use bevy::prelude::*;

    // Through the REAL load path: the hull is composed since issue #878, so
    // its ship-level AI declarations arrive from the fragment library and
    // `include_str!` would spawn an unresolved document.
    let config = crate::entities::include_resolve::load_entity_config(
        "assets/entities/test/rng_coverage_lancer.toml",
    )
    .expect("the rng-coverage escort must resolve and parse");

    let mut app = test_app();
    let uuid = uuid::Uuid::new_v4().to_string();
    let entity = spawn_and_flush(&mut app, &config, Vec3::ZERO, uuid, None);

    let sources = app
        .world()
        .get::<crate::ship_plugin::ShipSystemControlSources>(entity)
        .expect("behaviour NPC must carry ShipSystemControlSources");
    assert!(
        sources
            .0
            .policy_for(&crate::ship::system_registry::red_alert_system_id())
            .operate_ai,
        "provisioned red_alert must resolve to Ai so operate_captain_ai can raise it"
    );
    // And the ship carries the Red Alert capability component.
    assert!(
        app.world()
            .get::<crate::ship::state::ShipRedAlert>(entity)
            .is_some(),
        "behaviour NPC must carry the ShipRedAlert capability"
    );
}
