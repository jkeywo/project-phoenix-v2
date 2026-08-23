#![allow(clippy::field_reassign_with_default)]

use super::*;
use crate::entities::tags::EntityTag;
use crate::simmath;

/// Minimal `[helm_console]` fixture with only `max_speed` set — reused by
/// the tests that assert every *other* helm-console field defaults to
/// `None`/absent when the TOML omits it.
const MINIMAL_HELM_CONSOLE_TOML: &str = r##"
[helm_console]
max_speed = 30.0
"##;

/// Minimal `[asteroid_field]` fixture predating the optional `shape` and
/// `anchor` fields — reused by the back-compat regression tests asserting
/// both still default correctly when the authored TOML lacks them.
const MINIMAL_ASTEROID_FIELD_TOML: &str = r##"
[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;

/// One shipped hull through the REAL load path — include resolution and all.
///
/// Since issue #878 every Harrow hull is COMPOSED: its movement doctrine and
/// its ship-level declarations arrive from `assets/entities/fragments/ai/`,
/// so the authored file alone is not the document the game spawns. An
/// `include_str!` here would assert on unresolved text and pass while the
/// resolved hull said something else entirely;
/// `include_resolve::tests::shipped_tree::include_str_baked_hulls_are_all_uncomposed`
/// is the guard that names any site which forgets.
/// The resolved document as TEXT: every assertion below parses it exactly as
/// the loader does, and the tests that strike a line out of it to prove the
/// load fails without that line find the line wherever it is now authored —
/// hull or fragment.
fn resolved_text(stem: &str) -> String {
    crate::entities::include_resolve::resolve_from_disk(&format!("assets/entities/{stem}.toml"))
        .unwrap_or_else(|e| panic!("{stem} must resolve: {e}"))
        .toml
}

fn harrow_destroyer_toml() -> String {
    resolved_text("ship_harrow_destroyer")
}

fn harrow_cruiser_toml() -> String {
    resolved_text("ship_harrow_cruiser")
}

fn harrow_warhawk_toml() -> String {
    resolved_text("ship_harrow_warhawk")
}

#[test]
fn all_sections_present_deserializes_to_some() {
    let toml_str = r##"
tags = ["gameplay", "combat", "primary"]

[hull]
hull_integrity = 100

[collider]
shape = "Ball"
radius = 2.0
length = 0.0

[appearance]
colour = "#ff0000"
size_min = 1.0
size_max = 3.0

[helm_console]
max_speed = 50.0
max_reverse_speed = 25.0
acceleration = 16.7
deceleration = 50.0
max_yaw_rate = 0.785

[helm_console.radar]
range = 50.0
shows = ["asteroid"]

[weapons_console]

[engineering_console]

[captain_console]
"##;
    // Lenient: this fixture is about which SECTIONS deserialize to `Some`,
    // and its bare `[weapons_console]` owes a `weapons_doctrine`
    // declaration under the default strict mode (issue #956 — the kind
    // gates on the console, not on `[behaviour]`).
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");

    assert_eq!(
        config.tags,
        vec![
            "gameplay".to_string(),
            "combat".to_string(),
            "primary".to_string()
        ]
    );

    assert!(config.hull.is_some());
    assert!((config.hull.as_ref().unwrap().hull_integrity - 100.0).abs() < 1e-6);

    assert!(config.collider.is_some());
    let c = config.collider.as_ref().unwrap();
    assert_eq!(c.shape, ColliderShape::Ball);
    assert_eq!(c.radius, 2.0);

    assert!(config.appearance.is_some());
    assert_eq!(config.appearance.as_ref().unwrap().colour, "#ff0000");

    assert!(config.helm_console.is_some());
    let h = config.helm_console.as_ref().unwrap();
    assert_eq!(h.max_speed, 50.0);
    assert_eq!(h.effective_radar_range(), 50.0);

    assert!(config.weapons_console.is_some());

    assert!(config.engineering_console.is_some());

    assert!(config.captain_console.is_some());
}

/// The red-alert hostile weapon-arc overlay colour (issue #874) is authored
/// per hull, not inlined in the client — AGENTS.md #11.
#[test]
fn helm_console_parses_the_hostile_arc_color() {
    let config = EntityConfig::from_toml(
        r##"
[helm_console]
max_speed = 50.0
hostile_arc_color = [ 1, 0.3, 0.3, 0.07 ]
"##,
    )
    .expect("parse must succeed");
    assert_eq!(
        config.helm_console.as_ref().unwrap().hostile_arc_color,
        vec![1.0, 0.3, 0.3, 0.07]
    );
}

/// A hull that omits it keeps the wire default rather than failing to parse.
#[test]
fn helm_console_hostile_arc_color_is_optional() {
    let config =
        EntityConfig::from_toml("[helm_console]\nmax_speed = 50.0\n").expect("parse must succeed");
    assert!(config
        .helm_console
        .as_ref()
        .unwrap()
        .hostile_arc_color
        .is_empty());
}

/// Every hull that renders `ph-helm-radar` must author the colour, or the
/// overlay silently falls back to a value no designer chose.
#[test]
fn the_player_hulls_author_a_hostile_arc_color() {
    for path in [
        "assets/entities/alliance_battleship.toml",
        "assets/entities/alliance_cruiser.toml",
        "assets/entities/alliance_destroyer.toml",
    ] {
        // Through the include resolver (issue #906) so a composed hull is
        // judged on its resolved document — a raw read would assert on the
        // unresolved text and silently stop covering the hull.
        let config = crate::entities::include_resolve::load_entity_config(path)
            .expect("hull TOML must parse");
        let color = &config
            .helm_console
            .as_ref()
            .expect("hull declares [helm_console]")
            .hostile_arc_color;
        assert_eq!(color.len(), 4, "{path} must author an RGBA quad: {color:?}");
        assert!(
            color[3] < 0.25,
            "{path}: the overlay must stay FAINTER than the Tactical radar's \
                 own arc fills (0.30 / 0.25); got alpha {}",
            color[3]
        );
    }
}

#[test]
fn helm_console_engine_pfx_deserializes_optional_block() {
    let toml_str = r##"
[helm_console]
max_speed = 50.0

[helm_console.engine_pfx]
color = [0.2, 0.7, 1.0, 0.8]
markers = ["engine_port", "engine_starboard"]
roll_degrees = 20.0
scale = 1.25
trail_lifetime_secs = 0.45
trail_spawn_interval_secs = 0.04
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let pfx = config
        .helm_console
        .as_ref()
        .and_then(|helm| helm.engine_pfx.as_ref())
        .expect("engine_pfx block must parse");

    assert_eq!(pfx.color, Some([0.2, 0.7, 1.0, 0.8]));
    assert_eq!(
        pfx.markers,
        vec!["engine_port".to_string(), "engine_starboard".to_string()]
    );
    assert_eq!(pfx.roll_degrees, Some(20.0));
    assert_eq!(pfx.scale, Some(1.25));
    assert_eq!(pfx.trail_lifetime_secs, Some(0.45));
    assert_eq!(pfx.trail_spawn_interval_secs, Some(0.04));
}

#[test]
fn helm_console_engine_pfx_fields_default_when_block_is_sparse() {
    let toml_str = r##"
[helm_console]
max_speed = 50.0

[helm_console.engine_pfx]
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let pfx = config
        .helm_console
        .as_ref()
        .and_then(|helm| helm.engine_pfx.as_ref())
        .expect("engine_pfx block must parse");

    assert_eq!(pfx.color, None);
    assert!(pfx.markers.is_empty());
    assert_eq!(pfx.roll_degrees, None);
    assert_eq!(pfx.scale, None);
    assert_eq!(pfx.trail_lifetime_secs, None);
    assert_eq!(pfx.trail_spawn_interval_secs, None);
}

#[test]
fn only_hull_and_tags_produces_none_for_console_fields() {
    let toml_str = r##"
tags = ["gameplay", "asteroid"]

[hull]
hull_integrity = 80
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");

    assert_eq!(
        config.tags,
        vec!["gameplay".to_string(), "asteroid".to_string()]
    );
    assert!(config.hull.is_some());
    assert!((config.hull.as_ref().unwrap().hull_integrity - 80.0).abs() < 1e-6);
    assert!(config.collider.is_none());
    assert!(config.appearance.is_none());
    assert!(config.helm_console.is_none());
    assert!(config.weapons_console.is_none());
    assert!(config.engineering_console.is_none());
    assert!(config.captain_console.is_none());
    assert!(
        config.radar_appearance.is_none(),
        "radar_appearance should default to None when not in TOML"
    );
}

#[test]
fn malformed_field_returns_error() {
    let toml_str = r##"
[hull]
hull_integrity = "not_an_integer"
"##;
    let result = EntityConfig::from_toml(toml_str);
    assert!(result.is_err());
}

#[test]
fn tags_field_deserializes_to_vec_string() {
    let toml_str = r##"
tags = ["foo", "bar", "baz", "quux"]
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    assert_eq!(
        config.tags,
        vec![
            "foo".to_string(),
            "bar".to_string(),
            "baz".to_string(),
            "quux".to_string()
        ]
    );
}

#[test]
fn collider_capsule_shape_round_trips() {
    let toml_str = r##"
[collider]
shape = "Capsule"
radius = 1.5
length = 6.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    assert_eq!(
        config.collider.as_ref().unwrap().shape,
        ColliderShape::Capsule
    );
}

#[test]
fn collider_cylinder_shape_round_trips() {
    let toml_str = r##"
[collider]
shape = "Cylinder"
radius = 17.04
half_height = 7.16
length = 0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let collider = config.collider.as_ref().unwrap();
    assert_eq!(collider.shape, ColliderShape::Cylinder);
    assert_eq!(collider.radius, 17.04);
    assert_eq!(collider.half_height, Some(7.16));
}

/// `half_height` is optional in the serde shape so that every Ball and
/// Capsule already on disk parses untouched — which means serde cannot be
/// the thing that catches a Cylinder without one.
///
/// It has to be caught SOMEWHERE, because the failure is silent and it is
/// the exact bug the station-collider work was fixing: a cylinder of zero
/// half-height is a body with no interior, so ships fly through a structure
/// they can see, and nothing anywhere says why.
#[test]
fn a_cylinder_without_a_half_height_is_a_load_error() {
    let err = EntityConfig::from_toml(
        r##"
[collider]
shape = "Cylinder"
radius = 17.04
length = 0
"##,
    )
    .expect_err("a Cylinder with no half_height must not load");
    assert!(
        err.to_string().contains("half_height"),
        "the error must name the missing field, got: {err}"
    );
}

/// Zero and negative are the same failure as absent — a disc with no
/// thickness — and are rejected for the same reason.
#[test]
fn a_cylinder_with_a_non_positive_half_height_is_a_load_error() {
    for bad in ["0", "0.0", "-7.16"] {
        let toml_str = format!(
            r##"
[collider]
shape = "Cylinder"
radius = 17.04
half_height = {bad}
length = 0
"##
        );
        let err = EntityConfig::from_toml(&toml_str)
            .err()
            .unwrap_or_else(|| panic!("half_height = {bad} must not load"));
        assert!(
            err.to_string().contains("half_height"),
            "the error must name the offending field, got: {err}"
        );
    }
}

/// The other two shapes are untouched by the new field: neither reads it,
/// and neither is required to author it. A Ball that omits `half_height`
/// (which is every Ball and Capsule template in `assets/entities/`) must go
/// on loading exactly as it did.
#[test]
fn ball_and_capsule_do_not_require_a_half_height() {
    for shape in ["Ball", "Capsule"] {
        let toml_str = format!(
            r##"
[collider]
shape = "{shape}"
radius = 1.5
length = 4.0
"##
        );
        let config = EntityConfig::from_toml(&toml_str)
            .unwrap_or_else(|e| panic!("a {shape} with no half_height must parse: {e}"));
        assert_eq!(config.collider.as_ref().unwrap().half_height, None);
    }
}

/// Issue #1154: an authored `mass` parses straight through onto the
/// config, not into some intermediate representation the spawner then has
/// to reinterpret.
#[test]
fn mass_authored_value_is_parsed() {
    let config = EntityConfig::from_toml("mass = 45000.0\n").expect("parse must succeed");
    assert_eq!(config.mass, 45_000.0);
}

/// Issue #1154, AC1: an entity that authors no `mass` at all takes the
/// documented parse-time default, never a bare `0.0` — a zero-weight tow
/// is an exploit, not an empty field.
#[test]
fn mass_defaults_when_unauthored() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert_eq!(
        config.mass, DEFAULT_ENTITY_MASS,
        "an unauthored entity must get a real weight, not zero"
    );
    assert!(
        config.mass > 0.0,
        "the default itself must be a positive weight"
    );
}

/// Issue #1154, AC5: a non-positive or non-finite `mass` is refused at
/// load, naming the field, rather than shipping a NaN or a zero into a
/// mass-driven mechanic three systems downstream.
#[test]
fn non_positive_or_non_finite_mass_is_rejected() {
    for bad in ["0", "0.0", "-1500.0", "nan", "inf", "-inf"] {
        let toml_str = format!("mass = {bad}\n");
        let err = EntityConfig::from_toml(&toml_str)
            .err()
            .unwrap_or_else(|| panic!("mass = {bad} must not load"));
        assert!(
            err.to_string().contains("mass"),
            "the error must name the offending field, got: {err}"
        );
    }
}

/// Issue #1154, AC2: every shipped hull an operation could Tow, and every
/// shipped structure an operation could Stabilise/Transfer/repair, carries
/// an EXPLICIT, class-appropriate mass rather than leaning on the shared
/// default — a battleship and a courier that both silently fell back to
/// the same number would make the tow penalty read identically for either.
#[test]
fn shipped_hulls_and_operable_structures_author_an_explicit_mass() {
    for (path, expected) in [
        ("assets/entities/alliance_battleship.toml", 55_000.0),
        ("assets/entities/alliance_cruiser.toml", 24_000.0),
        ("assets/entities/alliance_destroyer.toml", 14_000.0),
        ("assets/entities/alliance_courier.toml", 3_500.0),
        ("assets/entities/ship_civilian_hauler.toml", 9_000.0),
        ("assets/entities/ship_harrow_warhawk.toml", 52_000.0),
        ("assets/entities/ship_harrow_cruiser.toml", 22_000.0),
        ("assets/entities/ship_harrow_destroyer.toml", 13_000.0),
        ("assets/entities/ship_harrow_patrol.toml", 18_000.0),
        ("assets/entities/ship_requiem_courier.toml", 3_200.0),
        ("assets/entities/skyhook.toml", 250_000.0),
        ("assets/entities/depot_transfer.toml", 180_000.0),
    ] {
        let config = crate::entities::include_resolve::load_entity_config(path)
            .expect("hull TOML must parse");
        assert_eq!(
            config.mass, expected,
            "{path} must author mass = {expected}"
        );
        assert_ne!(
            config.mass, DEFAULT_ENTITY_MASS,
            "{path} must author its OWN mass rather than coasting on the shared default"
        );
    }
}

/// Issue #958: `[collider] movable` is the authored dynamic/static split the
/// hazard rule reads. A template that omits it is TERRAIN — the safe
/// direction, since terrain is never dropped by the ignore-smaller rule.
#[test]
fn collider_movable_defaults_to_static_terrain() {
    let unauthored = EntityConfig::from_toml(
        r##"
[collider]
shape = "Ball"
radius = 12.0
length = 0.0
"##,
    )
    .expect("parse must succeed");
    assert!(
        !unauthored.collider.as_ref().unwrap().movable,
        "an unauthored collider must default to static terrain"
    );

    let authored = EntityConfig::from_toml(
        r##"
[collider]
shape = "Capsule"
radius = 1.5
length = 4.0
movable = true
"##,
    )
    .expect("parse must succeed");
    assert!(
        authored.collider.as_ref().unwrap().movable,
        "`movable = true` must parse into a mobile contact"
    );
}

/// Issue #958: shipped authoring, not just the parser, and a walk rather
/// than a list so a NEW template cannot quietly land on the wrong side.
///
/// A template that declares a helm capability is a hull somebody flies, so
/// it must author `movable = true` and take its chances with a bigger hull's
/// `hazard_ignore_size_ratio`. Everything else with a collider is terrain —
/// station, planet, moon, star, asteroid — and must stay static, so it is
/// avoided at any relative size.
///
/// The walk is RECURSIVE, mirroring `spawnable_templates_under` in
/// `src/headless/app.rs`, which issue #954 made recursive for the same
/// reason: that issue filed a spawned hull under
/// `assets/entities/test/rng_coverage_lancer.toml`, and
/// `assets/worlds/rng_coverage.toml` fields it twice. A top-level
/// `read_dir` would leave that hull — and anything else a later issue files
/// in a subdirectory — outside a guard whose whole purpose is to catch the
/// template nobody remembered to author.
///
/// `fragments/` is the one exclusion, and it is excluded for a property of
/// its contents rather than of its name: nothing in it is spawnable. A
/// fragment is a partial document that hulls compose FROM, so it is never
/// itself a body publishing a hazard, and `composed_escort.toml` is a
/// mechanism fixture rather than shipped content. That does leave the
/// ship-shaped `npc_escort_core.toml` unguarded by construction: it authors
/// `movable = true` because anything composing from it is by construction a
/// hull, but that authoring is a convention this test cannot hold.
#[test]
fn shipped_hulls_are_mobile_and_shipped_terrain_is_not() {
    /// Every `.toml` under `dir` except the fragment tree, sorted so the
    /// failure a designer sees is the same on every filesystem.
    fn templates_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()));
        let mut paths: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "fragments") {
                    continue;
                }
                templates_under(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                out.push(path);
            }
        }
    }

    let dir = std::path::Path::new("assets/entities");
    let mut templates = Vec::new();
    templates_under(dir, &mut templates);
    assert!(
        !templates.is_empty(),
        "no templates found under {}",
        dir.display()
    );

    let (mut hulls, mut terrain) = (0, 0);
    for path in templates {
        let key = path.to_string_lossy().replace('\\', "/");
        let cfg = crate::entities::include_resolve::load_entity_config(&key)
            .unwrap_or_else(|e| panic!("{key} must parse: {e}"));
        let Some(collider) = cfg.collider.as_ref() else {
            continue;
        };
        if cfg.helm_capability.is_some() || cfg.helm_console.is_some() {
            assert!(
                collider.movable,
                "{key} declares a helm capability, so it is a flyable hull \
                     and must author `[collider] movable = true`"
            );
            hulls += 1;
        } else {
            assert!(
                !collider.movable,
                "{key} has no helm capability, so it is static terrain and \
                     must never claim `[collider] movable = true`"
            );
            terrain += 1;
        }
    }
    assert!(hulls > 0, "no flyable hulls found in {}", dir.display());
    assert!(terrain > 0, "no static terrain found in {}", dir.display());
}

#[test]
fn empty_toml_string_produces_all_none() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.tags.is_empty());
    assert!(config.hull.is_none());
    assert!(config.collider.is_none());
    assert!(config.appearance.is_none());
    assert!(config.helm_console.is_none());
    assert!(config.weapons_console.is_none());
    assert!(config.engineering_console.is_none());
    assert!(config.captain_console.is_none());
    assert!(
        config.radar_appearance.is_none(),
        "radar_appearance should default to None"
    );
}

#[test]
fn helm_console_partial_fields_work() {
    let toml_str = MINIMAL_HELM_CONSOLE_TOML;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let h = config.helm_console.expect("helm_console must be Some");
    assert_eq!(h.max_speed, 30.0);
    assert_eq!(h.max_reverse_speed, 0.0);
}

#[test]
fn helm_console_radar_table_parses_into_nested_field() {
    let toml_str = r##"
[helm_console]
max_speed = 30.0

[helm_console.radar]
range = 750.0
shows = ["asteroid", "ship"]
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let h = config.helm_console.expect("helm_console must be Some");
    let radar = h.radar.as_ref().expect("helm_console.radar must parse");
    assert_eq!(radar.range, 750.0);
    assert_eq!(h.effective_radar_range(), 750.0);
}

#[test]
fn helm_console_effective_radar_range_zero_when_no_radar_table() {
    let toml_str = MINIMAL_HELM_CONSOLE_TOML;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let h = config.helm_console.expect("helm_console must be Some");
    assert!(h.radar.is_none());
    assert_eq!(h.effective_radar_range(), 0.0);
}

#[test]
fn helm_console_boost_table_parses_when_present() {
    let toml_str = r##"
[helm_console]
max_speed = 30.0

[helm_console.boost]
multiplier = 3.0
steering_multiplier = 2.0
active_duration = 4.0
recharge_duration = 20.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let h = config.helm_console.expect("helm_console must be Some");
    let boost = h.boost.as_ref().expect("helm_console.boost must parse");
    assert_eq!(boost.multiplier, 3.0);
    assert_eq!(boost.steering_multiplier, 2.0);
    assert_eq!(boost.active_duration, 4.0);
    assert_eq!(boost.recharge_duration, 20.0);
}

#[test]
fn helm_console_boost_steering_multiplier_defaults_to_identity() {
    let toml_str = r##"
[helm_console]
max_speed = 30.0

[helm_console.boost]
multiplier = 3.0
active_duration = 4.0
recharge_duration = 20.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let h = config.helm_console.expect("helm_console must be Some");
    let boost = h.boost.as_ref().expect("helm_console.boost must parse");
    assert_eq!(
        boost.steering_multiplier,
        crate::ship::boost::BOOST_STEERING_MULTIPLIER
    );
}

#[test]
fn helm_console_boost_none_when_table_absent() {
    let toml_str = MINIMAL_HELM_CONSOLE_TOML;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let h = config.helm_console.expect("helm_console must be Some");
    assert!(
        h.boost.is_none(),
        "missing boost table must disable the feature"
    );
}

#[test]
fn weapons_console_beam_color_parses_rgba() {
    let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0
beam_color = [1.0, 0.5, 0.2, 0.9]
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let w = config
        .weapons_console
        .expect("weapons_console must be Some");
    assert_eq!(w.phaser_banks[0].beam_color, vec![1.0, 0.5, 0.2, 0.9]);
}

#[test]
fn weapons_console_beam_color_defaults_to_empty_when_omitted() {
    let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let w = config
        .weapons_console
        .expect("weapons_console must be Some");
    assert!(
        w.phaser_banks[0].beam_color.is_empty(),
        "beam_color should default to empty vec when omitted"
    );
}

// ── Power section tests ────────────────────────────────────────────────

#[test]
fn power_section_parses_capacity_rates_emergency_threshold() {
    let toml_str = r##"
[power]
capacity = 150.0
rates = [10.0, 8.0, 6.0, 4.0, -4.0, -10.0]
emergency_threshold = 30.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let p = config.power.expect("power must be Some");
    assert!((p.capacity - 150.0).abs() < 0.001);
    assert_eq!(p.rates, [10.0, 8.0, 6.0, 4.0, -4.0, -10.0]);
    assert!((p.emergency_threshold - 30.0).abs() < 0.001);
}

#[test]
fn power_section_omitted_when_not_in_toml() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(
        config.power.is_none(),
        "power should be None when not specified"
    );
}

#[test]
fn alliance_hulls_author_the_six_plus_two_reactor_budget() {
    for path in [
        "assets/entities/alliance_courier.toml",
        "assets/entities/alliance_destroyer.toml",
        "assets/entities/alliance_cruiser.toml",
        "assets/entities/alliance_battleship.toml",
    ] {
        let config = crate::entities::include_resolve::load_entity_config(path)
            .unwrap_or_else(|error| panic!("{path}: {error}"));
        let power = config.power.expect("Alliance hull authors [power]");
        assert_eq!(power.sustainable_total, 6, "{path}");
        assert_eq!(power.max_commanded_total, 8, "{path}");
        let minimum = power.max_commanded_total - (power.rates.len() as u8 - 1);
        for (offset, rate) in power.rates.into_iter().enumerate() {
            let total = minimum + offset as u8;
            assert_eq!(rate < 0.0, total > 6, "{path}: total {total}");
        }
    }
}

#[test]
fn sensors_console_parses_with_long_range_radar() {
    let toml_str = r##"
tags = ["player", "ship"]

[sensors_console.long_range_radar]
range = 200.0
shows = ["region", "asteroid_field", "asteroid", "ship"]
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let sensors = config
        .sensors_console
        .expect("sensors_console must be Some");
    assert_eq!(sensors.long_range_radar.range, 200.0);
    assert!(sensors.long_range_radar.shows.contains(&EntityTag::Region));
    assert!(sensors
        .long_range_radar
        .shows
        .contains(&EntityTag::AsteroidField));
    assert!(sensors
        .long_range_radar
        .shows
        .contains(&EntityTag::Asteroid));
}

#[test]
fn sensors_console_omitted_when_not_in_toml() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.sensors_console.is_none());
}

/// `power_multipliers` lives on `[shields_console]` since issue #952 moved
/// the third power group from `sensors` to `shields`. Replaces the
/// `[sensors_console]` half of `sensors_console_parses_with_long_range_radar`,
/// whose assertion was that a curve authored there was READ — which is no
/// longer true of any curve on that console, because `RadarRange` has no
/// power producer left to read it.
#[test]
fn shields_console_power_multipliers_parses() {
    let toml_str = r##"
[shields_console]
power_multipliers = [0.0, 0.25, 0.5, 1.0]
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let shields = config
        .shields_console
        .expect("shields_console must be Some");
    assert_eq!(shields.power_multipliers, Some([0.0, 0.25, 0.5, 1.0]));
}

/// A curve on `[sensors_console]` is now an unknown field, and the section
/// is `deny_unknown_fields`, so an author who leaves one behind is told at
/// load rather than watching it silently do nothing.
#[test]
fn sensors_console_power_multipliers_is_rejected() {
    let err =
        EntityConfig::from_toml("[sensors_console]\npower_multipliers = [-0.5, 0.0, 0.25, 0.5]\n")
            .expect_err("the field moved to [shields_console] in #952")
            .to_string();
    assert!(err.contains("power_multipliers"), "got: {err}");
}

#[test]
fn helm_console_power_multipliers_parses() {
    let toml_str = r##"
[helm_console]
power_multipliers = [-0.8, 0.0, 0.4, 0.8]
max_speed = 50.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let h = config.helm_console.expect("helm_console must be Some");
    assert_eq!(h.power_multipliers, Some([-0.8, 0.0, 0.4, 0.8]));
}

/// Lenient, like the other `[weapons_console]` schema fixtures around it:
/// since issue #956 a weapons console owes a `weapons_doctrine` declaration
/// (the kind gates on the CONSOLE, not on `[behaviour]`), and this fixture
/// is about one power curve rather than about AI authoring.
#[test]
fn weapons_console_power_multipliers_parses() {
    let toml_str = r##"
[weapons_console]
power_multipliers = [-0.3, 0.0, 0.15, 0.3]
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let w = config
        .weapons_console
        .expect("weapons_console must be Some");
    assert_eq!(w.power_multipliers, Some([-0.3, 0.0, 0.15, 0.3]));
}

#[test]
fn power_multipliers_defaults_to_none_when_omitted() {
    let toml_str = MINIMAL_HELM_CONSOLE_TOML;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let h = config.helm_console.expect("helm_console must be Some");
    assert!(h.power_multipliers.is_none());
}

/// Unknown keys in an entity TOML must be rejected, not silently ignored.
#[test]
fn unknown_section_and_field_are_rejected() {
    assert!(EntityConfig::from_toml("[helm_consol]\nmax_speed = 1.0").is_err());
    assert!(EntityConfig::from_toml("[helm_console]\nmax_sped = 1.0").is_err());
}

// ── AsteroidField section tests ────────────────────────────────────────

#[test]
fn asteroid_field_section_parses_from_template() {
    let toml_str = r##"
tags = ["field", "main"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
spawn_distance = 150.0
despawn_distance = 250.0
asteroid_type_paths = ["assets/entities/asteroid_small.toml", "assets/entities/asteroid_large.toml"]
cosmetic_type_paths = ["assets/entities/asteroid_cosmetic.toml"]
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let field = config.asteroid_field.expect("asteroid_field must be Some");
    assert!((field.inner_radius - 100.0).abs() < 1e-6);
    assert_eq!(field.asteroid_type_paths.len(), 2);
    assert_eq!(field.cosmetic_type_paths.len(), 1);
}

#[test]
fn asteroid_field_section_omitted_when_not_in_toml() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.asteroid_field.is_none());
}

#[test]
fn asteroid_field_shape_defaults_to_none_when_omitted() {
    // Back-compat: TOMLs that pre-date the `shape` field must continue
    // to deserialise unchanged, with `shape = None`.
    let toml_str = MINIMAL_ASTEROID_FIELD_TOML;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let field = config.asteroid_field.expect("asteroid_field must be Some");
    assert!(field.shape.is_none());
}

#[test]
fn asteroid_field_anchor_parses_as_optional_string() {
    // PRD #397 fix 5: `[asteroid_field] anchor = "name"` carries the
    // reference verbatim. The serde-skipped `anchor_offset` defaults
    // to `[0,0,0]` and is filled in at spawn time against the world's
    // anchor table.
    let toml_str = r##"
[asteroid_field]
shape = "torus"
anchor = "belt_origin"
inner_radius = 300.0
outer_radius = 350.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let field = config.asteroid_field.expect("asteroid_field must be Some");
    assert_eq!(field.anchor.as_deref(), Some("belt_origin"));
    assert_eq!(
        field.anchor_offset,
        [0.0, 0.0, 0.0],
        "anchor_offset is serde-skipped and defaults to origin until spawn-time resolution"
    );
}

#[test]
fn asteroid_field_anchor_omitted_defaults_to_none() {
    // Regression guard: existing TOML without an `anchor` key must keep
    // `anchor = None` and `anchor_offset = [0,0,0]` (legacy behaviour).
    let toml_str = MINIMAL_ASTEROID_FIELD_TOML;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let field = config.asteroid_field.expect("asteroid_field must be Some");
    assert!(field.anchor.is_none(), "missing anchor key → None");
    assert_eq!(field.anchor_offset, [0.0, 0.0, 0.0]);
}

#[test]
fn asteroid_field_shape_torus_parses() {
    // Schema: `shape = "torus"` as a sibling of `inner_radius`/`outer_radius`.
    let toml_str = r##"
[asteroid_field]
shape = "torus"
inner_radius = 300.0
outer_radius = 350.0
density = 0.005
asteroid_type_paths = ["x.toml"]
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let field = config.asteroid_field.expect("asteroid_field must be Some");
    assert_eq!(
        field.shape,
        Some(crate::entities::config::AsteroidFieldShape::Torus)
    );
    assert!((field.inner_radius - 300.0).abs() < 1e-6);
    assert!((field.outer_radius - 350.0).abs() < 1e-6);
}

#[test]
fn asteroid_field_shape_unknown_value_errors() {
    let toml_str = r##"
[asteroid_field]
shape = "donut"
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
"##;
    let result = EntityConfig::from_toml(toml_str);
    assert!(
        result.is_err(),
        "unknown shape variant must be a parse error"
    );
}

// ── name / mesh.emissive / [[light]] tests (PRD: schema refactor slice 3) ──

#[test]
fn name_field_parses() {
    let toml_str = r#"name = "Sun""#;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    assert_eq!(config.name.as_deref(), Some("Sun"));
}

#[test]
fn name_field_defaults_to_none() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.name.is_none());
}

#[test]
fn mesh_emissive_field_parses() {
    let toml_str = r##"
[mesh]
shape = "sphere"
colour = [1.0, 0.8, 0.0]
radius = 50.0
emissive = 2.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let mesh = config.mesh.expect("mesh must be Some");
    assert_eq!(mesh.emissive, Some(2.0));
}

#[test]
fn mesh_emissive_defaults_to_none() {
    let toml_str = r##"
[mesh]
shape = "sphere"
colour = [1.0, 1.0, 1.0]
radius = 1.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let mesh = config.mesh.expect("mesh must be Some");
    assert!(mesh.emissive.is_none());
}

#[test]
fn light_array_parses_multiple_entries() {
    let toml_str = r##"
[[light]]
kind = "point"
colour = [1.0, 0.95, 0.85]
intensity = 150000.0
range = 5000.0

[[light]]
kind = "point"
colour = [0.5, 0.5, 1.0]
intensity = 1000.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    assert_eq!(config.light.len(), 2);
    assert_eq!(config.light[0].kind, LightKind::Point);
    assert_eq!(config.light[0].colour, [1.0, 0.95, 0.85]);
    assert!((config.light[0].intensity - 150000.0).abs() < 1e-3);
    assert_eq!(config.light[0].range, Some(5000.0));
    assert_eq!(config.light[1].range, None);
}

#[test]
fn light_defaults_to_empty_vec() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.light.is_empty());
}

#[test]
fn light_directional_kind_parses() {
    let toml_str = r##"
[[light]]
kind = "directional"
colour = [1.0, 1.0, 1.0]
intensity = 10000.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    assert_eq!(config.light.len(), 1);
    assert_eq!(config.light[0].kind, LightKind::Directional);
}

// ── Region shape tests ───────────────────────────────────────────────

#[test]
fn region_shape_sphere_parses_from_toml() {
    let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "sphere"
radius = 100.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let shape = config.shape.expect("shape must be Some");
    assert_eq!(
        shape,
        crate::regions::shape::RegionShape::Sphere { radius: 100.0 }
    );
}

#[test]
fn region_shape_box_parses_from_toml() {
    let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "box"
half_extents = [50.0, 30.0, 40.0]
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let shape = config.shape.expect("shape must be Some");
    assert_eq!(
        shape,
        crate::regions::shape::RegionShape::Box {
            half_extents: [50.0, 30.0, 40.0],
            yaw: 0.0
        }
    );
}

#[test]
fn region_shape_torus_parses_from_toml() {
    let toml_str = r##"
tags = ["region", "test"]

[shape]
type = "torus"
inner_radius = 50.0
outer_radius = 80.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let shape = config.shape.expect("shape must be Some");
    assert_eq!(
        shape,
        crate::regions::shape::RegionShape::Torus {
            inner_radius: 50.0,
            outer_radius: 80.0
        }
    );
}

#[test]
fn region_shape_parses_with_effects() {
    let toml_str = r##"
tags = ["region", "nebula"]

[shape]
type = "sphere"
radius = 150.0

[effects]
[effects.comms_jammed]
[effects.sensor_blind]
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    assert!(config.shape.is_some());
    let effects = config.effects.expect("effects must be Some");
    assert!(effects.comms_jammed.is_some());
    assert!(effects.sensor_blind.is_some());
}

#[test]
fn region_effects_without_shape_returns_error() {
    let toml_str = r##"
tags = ["region"]

[effects]
[effects.comms_jammed]
"##;
    let result = EntityConfig::from_toml(toml_str);
    assert!(
        result.is_err(),
        "region entity with effects but no shape should error"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("shape"),
        "error should mention missing shape: {err}"
    );
}

#[test]
fn shape_alone_without_effects_is_valid() {
    let toml_str = r##"
tags = ["region"]

[shape]
type = "sphere"
radius = 100.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    assert!(config.shape.is_some());
    assert!(config.effects.is_none());
}

#[test]
fn empty_toml_produces_no_shape_or_effects() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.shape.is_none());
    assert!(config.effects.is_none());
}

// ── Ship audio tests ─────────────────────────────────────────────────

/// `EntityConfig` is `deny_unknown_fields`, so an `[audio]` block in a
/// shipped template would break *every* load of that template — not just
/// audio — if the field were ever removed. These parse the real files.
/// One shipped hull, through the REAL load path (issue #875).
///
/// `include_str!` bakes a template's bytes at compile time, so a baked site
/// can never see include resolution — and since the player destroyer became
/// a COMPOSED hull, its baked bytes are no longer the document the game
/// loads. `include_str_baked_hulls_are_all_uncomposed` is the tripwire that
/// names such sites; this helper is what they move to.
fn shipped_hull(stem: &str) -> EntityConfig {
    let path = format!("assets/entities/{stem}.toml");
    crate::entities::include_resolve::load_entity_config(&path)
        .unwrap_or_else(|e| panic!("{stem}.toml must compose and parse: {e}"))
}

/// Every shipped ship hull authors a player-facing NAME (`display_name`)
/// and a CLASS, so a ship is identified by "AEV Phoenix" / "Cruiser" in
/// scans, comms and the picker — never by its bare class, a wave number, or
/// an "Unknown" subtitle (player-facing ship names / Unknown→class).
#[test]
fn every_ship_hull_authors_a_display_name_and_a_class() {
    // (stem, expected class token — reused by the picker badge label set).
    let hulls: [(&str, &str); 9] = [
        ("alliance_cruiser", "cruiser"),
        ("alliance_destroyer", "destroyer"),
        ("alliance_battleship", "battleship"),
        ("alliance_courier", "courier"),
        ("ship_harrow_patrol", "cruiser"),
        ("ship_harrow_cruiser", "cruiser"),
        ("ship_harrow_destroyer", "destroyer"),
        ("ship_harrow_warhawk", "battleship"),
        ("ship_requiem_courier", "courier"),
    ];
    for (stem, class) in hulls {
        let config = shipped_hull(stem);
        let display = config
            .display_name
            .as_deref()
            .unwrap_or_else(|| panic!("{stem}.toml must author a top-level display_name"));
        assert_eq!(
            display,
            format!("entity.{stem}.display_name"),
            "{stem} display_name must be its own strings id"
        );
        assert_eq!(
            config.class.as_deref(),
            Some(class),
            "{stem} must author a class the picker badge can label"
        );
    }
}

/// The cruiser's proper name is the flagship John named: "AEV Phoenix". The
/// value lives in strings.csv (bracketed per the unratified-copy convention);
/// here we pin that the hull points at the id that carries it.
#[test]
fn the_cruiser_is_the_aev_phoenix() {
    assert_eq!(
        shipped_hull("alliance_cruiser").display_name.as_deref(),
        Some("entity.alliance_cruiser.display_name"),
    );
}

#[test]
fn player_ship_templates_parse_audio_block() {
    for name in [
        "alliance_cruiser",
        "alliance_destroyer",
        "alliance_battleship",
    ] {
        let config = shipped_hull(name);
        let audio = config
            .audio
            .as_ref()
            .unwrap_or_else(|| panic!("{name}.toml must have [audio]"));

        assert_eq!(
            audio.ambient.as_ref().expect("[audio.ambient]").file,
            "assets/sounds/Ambient.mp3",
            "{name}"
        );
        assert_eq!(
            audio.engine.as_ref().expect("[audio.engine]").file,
            "assets/sounds/Engine.mp3",
            "{name}"
        );
        assert_eq!(
            audio.blaster.as_ref().expect("[audio.blaster]").file,
            "assets/sounds/Blaster.mp3",
            "{name}"
        );
        assert_eq!(
            audio
                .phaser_loop
                .as_ref()
                .expect("[audio.phaser_loop]")
                .file,
            "assets/sounds/PhaserLoop.mp3",
            "{name}"
        );
        assert_eq!(
            audio.forcefield.as_ref().expect("[audio.forcefield]").file,
            "assets/sounds/ForcefieldHit.mp3",
            "{name}"
        );
    }
}

/// Preserves the volumes the JS previously hardcoded (`hum.volume = 0.25`,
/// `engine.volume = thrust * 0.15`), so making them data-driven did not
/// silently change how the game sounds.
#[test]
fn cruiser_audio_preserves_legacy_volumes() {
    let config = shipped_hull("alliance_cruiser");
    let audio = config.audio.as_ref().expect("[audio]");
    assert_eq!(audio.ambient.as_ref().unwrap().volume, 0.25);
    assert_eq!(audio.engine.as_ref().unwrap().volume_at_full_thrust, 0.15);
    assert_eq!(audio.engine.as_ref().unwrap().idle_volume, 0.0);
}

#[test]
fn entity_without_audio_block_parses_to_none() {
    let config = EntityConfig::from_toml(include_str!("../../assets/entities/station_axiom.toml"))
        .expect("must parse");
    assert!(config.audio.is_none());
}

// ── Station hull tests (post-[station] removal; PRD slice 2) ──────────

#[test]
fn station_axiom_template_parses_hull_integrity() {
    let toml_str = include_str!("../../assets/entities/station_axiom.toml");
    let config = EntityConfig::from_toml(toml_str).expect("station_axiom.toml must parse");
    let hull = config.hull.as_ref().expect("must have [hull]");
    // (#474) Buffed 200 → 800 for the combat-test scenario, then 800 → 1600
    // in the stationary-station combat retune so the station survives the
    // eight-wave raid alongside its tripled point-defence damage.
    assert!((hull.hull_integrity - 1600.0).abs() < 1e-6);
}

#[test]
fn station_outpost_template_parses_hull_integrity() {
    let toml_str = include_str!("../../assets/entities/station_outpost.toml");
    let config = EntityConfig::from_toml(toml_str).expect("station_outpost.toml must parse");
    let hull = config.hull.as_ref().expect("must have [hull]");
    assert!((hull.hull_integrity - 200.0).abs() < 1e-6);
}

#[test]
fn station_research_outpost_template_parses_hull_integrity() {
    let toml_str = include_str!("../../assets/entities/station_research_outpost.toml");
    let config =
        EntityConfig::from_toml(toml_str).expect("station_research_outpost.toml must parse");
    let hull = config.hull.as_ref().expect("must have [hull]");
    assert!((hull.hull_integrity - 60.0).abs() < 1e-6);
}

#[test]
fn all_sections_parsed_in_full_template() {
    let toml_str = r##"
tags = ["full"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005

[hull]
hull_integrity = 100
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    assert!(
        config.asteroid_field.is_some(),
        "asteroid_field should be Some"
    );
    assert!(config.hull.is_some(), "hull should be Some");
    assert_eq!(config.tags, vec!["full"]);
}

// ── SystemId-keyed hull entries (parent issue #516 sub-issue #616) ────────

#[test]
fn hull_system_hull_parses_from_toml() {
    let toml_str = r##"
[hull]
hull_integrity = 100

[[hull.system_hull]]
system_id = "phaser-fore"
display_name = "Phaser Bank (Fore)"
max_hp = 25.0
damaged_threshold_pct = 0.6
disabled_threshold_pct = 0.2
debuff_magnitude = 0.25
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let hull = config.hull.as_ref().expect("hull must parse");
    assert_eq!(hull.system_hull.len(), 1);
    let entry = &hull.system_hull[0];
    assert_eq!(
        entry.system_id,
        crate::core::messages::SystemId("phaser-fore".into())
    );
    assert_eq!(entry.display_name.as_deref(), Some("Phaser Bank (Fore)"));
    assert!((entry.max_hp - 25.0).abs() < 1e-6);
    assert!((entry.damaged_threshold_pct - 0.6).abs() < 1e-6);
    assert!((entry.disabled_threshold_pct - 0.2).abs() < 1e-6);
    assert!((entry.debuff_magnitude - 0.25).abs() < 1e-6);
}

#[test]
fn hull_system_hull_defaults_when_absent() {
    // Legacy TOML without [[hull.system_hull]] must still parse; the new
    // field defaults to an empty Vec.
    let toml_str = r##"
[hull]
hull_integrity = 100
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let hull = config.hull.as_ref().expect("hull must parse");
    assert!(hull.system_hull.is_empty());
}

#[test]
fn hull_system_hull_entry_optional_fields_default() {
    // Only the required fields (system_id, max_hp) are provided; every
    // other field has a serde default.
    let toml_str = r##"
[hull]
[[hull.system_hull]]
system_id = "helm"
max_hp = 30.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let entry = &config.hull.as_ref().unwrap().system_hull[0];
    assert_eq!(
        entry.system_id,
        crate::core::messages::SystemId("helm".into())
    );
    assert!(entry.display_name.is_none());
    assert!((entry.damaged_threshold_pct - 0.75).abs() < 1e-6);
    assert!((entry.disabled_threshold_pct - 0.25).abs() < 1e-6);
    assert!((entry.debuff_magnitude - 0.15).abs() < 1e-6);
}

// ── Shipped template TOML files referenced by assets/worlds/default.toml ──
//
// These tests embed each template at compile time via include_str! so
// the build fails if a referenced template is missing or malformed.

#[test]
fn empty_star_section_uses_defaults() {
    let config = EntityConfig::from_toml("[star]\n").expect("parse must succeed");
    let star = config.star.as_ref().expect("must parse [star]");
    assert!((star.radius - 40.0).abs() < 1e-6);
    assert_eq!(star.longitude_segments, 64);
    assert_eq!(star.latitude_segments, 32);
    assert_eq!(star.surface_colour, [1.0, 0.72, 0.12]);
    assert_eq!(star.hot_colour, [1.0, 0.96, 0.65]);
    assert_eq!(star.cell_colour, [0.95, 0.32, 0.04]);
    assert_eq!(star.halo_colour, [1.0, 0.78, 0.18]);
    assert!((star.halo_radius_multiplier - 2.4).abs() < 1e-6);
    assert!((star.animation_speed - 1.0).abs() < 1e-6);
}

#[test]
fn star_section_overrides_defaults() {
    let toml_str = r#"
[star]
radius = 75.0
longitude_segments = 96
latitude_segments = 48
surface_colour = [0.9, 0.7, 0.2]
hot_colour = [1.0, 1.0, 0.8]
cell_colour = [0.8, 0.2, 0.1]
halo_colour = [1.0, 0.6, 0.1]
halo_radius_multiplier = 3.0
animation_speed = 0.5
"#;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let star = config.star.as_ref().expect("must parse [star]");
    assert!((star.radius - 75.0).abs() < 1e-6);
    assert_eq!(star.longitude_segments, 96);
    assert_eq!(star.latitude_segments, 48);
    assert_eq!(star.surface_colour, [0.9, 0.7, 0.2]);
    assert_eq!(star.hot_colour, [1.0, 1.0, 0.8]);
    assert_eq!(star.cell_colour, [0.8, 0.2, 0.1]);
    assert_eq!(star.halo_colour, [1.0, 0.6, 0.1]);
    assert!((star.halo_radius_multiplier - 3.0).abs() < 1e-6);
    assert!((star.animation_speed - 0.5).abs() < 1e-6);
}

#[test]
fn star_section_rejects_unknown_fields() {
    let result = EntityConfig::from_toml(
        r#"
[star]
radius = 40.0
surfase_colour = [1.0, 0.7, 0.1]
"#,
    );
    assert!(result.is_err());
}

#[test]
fn star_sun_template_parses_with_star_and_lights() {
    let toml_str = include_str!("../../assets/entities/star_sun.toml");
    let config = EntityConfig::from_toml(toml_str).expect("star_sun.toml must parse");
    // Display text lives in assets/strings/strings.csv; the TOML holds the
    // string id, which is what Rust passes through to the client.
    assert_eq!(config.name.as_deref(), Some("entity.star_sun.name"));
    assert!(config.star.is_some(), "star_sun.toml must have [star]");
    assert!(config.mesh.is_none(), "star_sun.toml must not keep [mesh]");
    assert!(
        !config.light.is_empty(),
        "star_sun.toml must have at least one [[light]]"
    );
    assert_eq!(config.light[0].kind, LightKind::Directional);
    let collider = config
        .collider
        .as_ref()
        .expect("star_sun.toml must have [collider]");
    assert_eq!(collider.shape, ColliderShape::Ball);
}

#[test]
fn planet_earth_template_parses_with_mesh_and_collider() {
    let toml_str = include_str!("../../assets/entities/planet_earth.toml");
    let config = EntityConfig::from_toml(toml_str).expect("planet_earth.toml must parse");
    assert_eq!(config.name.as_deref(), Some("entity.planet_earth.name"));
    assert!(config.mesh.is_some(), "planet_earth.toml must have [mesh]");
    let collider = config
        .collider
        .as_ref()
        .expect("planet_earth.toml must have [collider]");
    assert_eq!(collider.shape, ColliderShape::Ball);

    // Textured-planet section: earth has clouds, atmosphere, and
    // night-gated city-light emission without a separate mask.
    let planet = config
        .planet
        .as_ref()
        .expect("planet_earth.toml must have [planet]");
    assert!(planet.surface.normal.is_some());
    assert!(planet.surface.emissive_colour.is_some());
    assert!(planet.surface.emissive_mask.is_none());
    assert!(planet.surface.emissive_night_only);
    assert!(planet.clouds.is_some());
    assert!(planet.atmosphere.is_some());
}

#[test]
fn planet_lava_template_parses_with_dayside_emission() {
    let toml_str = include_str!("../../assets/entities/planet_lava.toml");
    let config = EntityConfig::from_toml(toml_str).expect("planet_lava.toml must parse");
    let planet = config
        .planet
        .as_ref()
        .expect("planet_lava.toml must have [planet]");
    // Lava glows on the day side too — the night gate must be off.
    assert!(!planet.surface.emissive_night_only);
    assert!(planet.surface.emissive_mask.is_some());
    let clouds = planet.clouds.as_ref().expect("ash shell expected");
    assert!((clouds.scale - 1.03).abs() < 1e-6);
    assert!(
        (clouds.drift_speed - 0.0).abs() < 1e-6,
        "no motion by default"
    );
}

#[test]
fn moon_luna_template_parses_surface_only() {
    let toml_str = include_str!("../../assets/entities/moon_luna.toml");
    let config = EntityConfig::from_toml(toml_str).expect("moon_luna.toml must parse");
    let planet = config
        .planet
        .as_ref()
        .expect("moon_luna.toml must have [planet]");
    assert!(planet.surface.emissive_colour.is_none());
    assert!(planet.clouds.is_none());
    assert!(planet.atmosphere.is_none());
}

#[test]
fn asteroid_field_main_template_parses_with_field_and_grid() {
    let toml_str = include_str!("../../assets/entities/asteroid_field_main.toml");
    let config = EntityConfig::from_toml(toml_str).expect("asteroid_field_main.toml must parse");
    let field = config
        .asteroid_field
        .as_ref()
        .expect("must have [asteroid_field]");
    field
        .grid
        .as_ref()
        .expect("must have [asteroid_field.grid]");
    // 4 common + 4 uncommon + 4 rare models, each in a small and a large
    // size (issue #946), plus the four commons again at the huge size
    // (issue #947). The cosmetic backdrop stays commons-only.
    assert_eq!(field.asteroid_type_paths.len(), 28);
    assert_eq!(field.cosmetic_type_paths.len(), 4);
}

/// The authored rarity groups, pinned to the currently-shipped weights.
///
/// Two axes, deliberately kept apart. *Material* rarity is the class: an
/// uncommon rock is drawn a tenth as often as a common and a rare a
/// hundredth (issue #946). *Size* rarity multiplies it: the huge size
/// (issue #947) is authored at a tenth of its class weight, because a rock
/// that big is a landmark and at the class weight it would be a third of
/// every gameplay rock in the field.
///
/// The expected weights below are restated, not read off the TOML, so a
/// deliberate retune of any group must update this test alongside the
/// config; only group *membership* (which paths carry which class and
/// size) is read from the file. If someone drops the weights and the
/// entries fall back to bare paths, it fails too.
#[test]
fn asteroid_field_main_declares_three_rarity_tiers() {
    let toml_str = include_str!("../../assets/entities/asteroid_field_main.toml");
    let config = EntityConfig::from_toml(toml_str).expect("asteroid_field_main.toml must parse");
    let field = config
        .asteroid_field
        .as_ref()
        .expect("must have [asteroid_field]");

    let weights_of = |tier: &str, size: &str| -> Vec<f32> {
        field
            .asteroid_type_paths
            .iter()
            .filter(|t| {
                t.path().contains(&format!("asteroid_{tier}_"))
                    && t.path().ends_with(&format!("_{size}.toml"))
            })
            .map(|t| t.weight())
            .collect()
    };
    let groups = [
        ("common", "small", 1.0f32),
        ("common", "large", 1.0),
        // The size-rarity multiplier, not a class of its own: 1.0 x 0.1.
        ("common", "huge", 0.1),
        ("uncommon", "small", 0.1),
        ("uncommon", "large", 0.1),
        ("rare", "small", 0.01),
        ("rare", "large", 0.01),
    ];
    let mut accounted = 0;
    for (tier, size, expected) in groups {
        let weights = weights_of(tier, size);
        assert_eq!(weights.len(), 4, "{tier} {size}: one entry per model");
        for w in &weights {
            assert!(
                (w - expected).abs() < 1e-6,
                "{tier} {size} entries must be authored at weight {expected}, found {w}"
            );
        }
        accounted += weights.len();
    }
    // Every entry belongs to a group named above, so a new class or size
    // cannot land unweighted and unnoticed.
    assert_eq!(
        accounted,
        field.asteroid_type_paths.len(),
        "an entry matches no (class, size) group this test knows about"
    );

    // Only the commons are scaled up. A landmark's job is to be recognised
    // at range, so it is the same four silhouettes every time; scaling the
    // uncommon and rare scans as well would make "that rock is enormous"
    // and "that rock is unusual" the same signal.
    for tier in ["uncommon", "rare"] {
        assert!(
            weights_of(tier, "huge").is_empty(),
            "the huge size is authored on the common class only"
        );
    }

    // The cosmetic layers carry no rarity tiers, so their entries keep the
    // bare-string spelling — which must still read as weight 1.0.
    for entry in &field.cosmetic_type_paths {
        assert!(matches!(entry, AsteroidTypeRef::Path(_)));
        assert!((entry.weight() - 1.0).abs() < 1e-6);
    }
}

/// The two shipped fields carry the same authored type lists, and are
/// rewritten together by `scripts/import-asteroids.mjs` from one class
/// table. Asserted rather than left to reviewer diligence: they were last
/// edited by a script that touches both, and a hand-edit to one is exactly
/// the change nobody would notice until a belt spawned a different mix of
/// rocks from a field.
#[test]
fn both_shipped_asteroid_fields_carry_the_same_type_lists() {
    let field_of = |text: &str| {
        EntityConfig::from_toml(text)
            .expect("field template must parse")
            .asteroid_field
            .expect("must have [asteroid_field]")
    };
    let main = field_of(include_str!(
        "../../assets/entities/asteroid_field_main.toml"
    ));
    let belt = field_of(include_str!(
        "../../assets/entities/asteroid_belt_axiom.toml"
    ));

    let entries = |f: &AsteroidFieldConfig, gameplay: bool| -> Vec<(String, f32)> {
        let list = if gameplay {
            &f.asteroid_type_paths
        } else {
            &f.cosmetic_type_paths
        };
        list.iter()
            .map(|t| (t.path().to_string(), t.weight()))
            .collect()
    };
    assert_eq!(entries(&main, true), entries(&belt, true));
    assert_eq!(entries(&main, false), entries(&belt, false));
}

/// The huge size class (issue #947): a triple-size rock, authored as its
/// own set of entity templates over the SAME four common models.
///
/// `radius` is 12 against large's 4 — the "triple-size" the issue asks
/// for. `hull_integrity` is 300, three times large's 100 and so linear in
/// radius rather than in volume: the rule is that time-to-clear scales with
/// how big the thing looks, and a cruiser's two phaser banks put out 8 hull
/// a second, so a large rock is ~12 s of sustained fire and a huge one
/// ~37 s. Cubing it to 2700 would be 5.6 minutes on one rock and would
/// read as indestructible scenery that happens to have a health bar.
///
/// It keeps `[target]` and `[hull]`: it spawns in the gameplay layer beside
/// its small and large siblings, and a rock there that could not be
/// targeted or destroyed would be the one exception the weapons and radar
/// paths have to learn about. Hull-less, target-less rocks are the cosmetic
/// backdrop, and the huge class is not that.
#[test]
fn the_huge_asteroid_size_is_a_targetable_triple_size_rock() {
    for n in 1..=4 {
        let path = format!("assets/entities/asteroid_common_{n}_huge.toml");
        let cfg = crate::entities::include_resolve::load_entity_config(&path)
            .unwrap_or_else(|e| panic!("{path} must parse: {e}"));

        // Every asteroid variant shares one display id since the strings
        // consolidation (9b89a37b) — the per-variant names were folded into
        // `entity.asteroid.name`.
        assert_eq!(cfg.name.as_deref(), Some("entity.asteroid.name"));
        let collider = cfg
            .collider
            .as_ref()
            .unwrap_or_else(|| panic!("{path}: [collider]"));
        assert_eq!(collider.radius, 12.0, "{path}: three times large's 4");
        let hull = cfg
            .hull
            .as_ref()
            .unwrap_or_else(|| panic!("{path}: [hull]"));
        assert_eq!(
            hull.hull_integrity, 300.0,
            "{path}: three times large's 100"
        );
        assert!(
            cfg.target.is_some(),
            "{path}: a gameplay rock is targetable"
        );

        // Reuses the common model rather than shipping new geometry; the
        // size lives in the `huge` rig variant's scale.
        let mesh = cfg
            .mesh
            .as_ref()
            .unwrap_or_else(|| panic!("{path}: [mesh]"));
        assert_eq!(
            mesh.model.as_deref(),
            Some(&*format!("assets/models/asteroid_common_{n}.glb"))
        );
        assert_eq!(mesh.variant.as_deref(), Some("huge"));
        assert_eq!(mesh.radius, 12.0);
    }
}

/// Back-compat: the pre-#946 spelling (a flat list of path strings) still
/// parses, and means weight 1.0. Every field TOML written before rarity
/// existed depends on this.
#[test]
fn asteroid_type_paths_accept_bare_strings_and_weighted_tables() {
    let toml_str = r#"
tags = ["field"]

[asteroid_field]
inner_radius = 100.0
outer_radius = 200.0
density = 0.005
asteroid_type_paths = [
    "assets/entities/plain.toml",
    { path = "assets/entities/weighted.toml", weight = 0.25 },
    { path = "assets/entities/defaulted.toml" },
]
"#;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let field = config.asteroid_field.expect("must have [asteroid_field]");
    let types = &field.asteroid_type_paths;
    assert_eq!(types.len(), 3);
    assert_eq!(types[0].path(), "assets/entities/plain.toml");
    assert!((types[0].weight() - 1.0).abs() < 1e-6);
    assert_eq!(types[1].path(), "assets/entities/weighted.toml");
    assert!((types[1].weight() - 0.25).abs() < 1e-6);
    // A table that omits `weight` is the same as a bare string.
    assert_eq!(types[2].path(), "assets/entities/defaulted.toml");
    assert!((types[2].weight() - 1.0).abs() < 1e-6);
}

// ── Faction field tests ────────────────────────────────────────────────

#[test]
fn faction_field_parses_from_entity_toml() {
    let toml_str = r#"
tags = ["ship"]
faction = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
"#;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let faction = config.faction.expect("faction must be Some");
    assert_eq!(faction.to_string(), "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa");
}

#[test]
fn faction_field_defaults_to_none_when_absent() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.faction.is_none());
}

#[test]
fn battleship_toml_parses_with_federation_faction() {
    let config = shipped_hull("alliance_battleship");
    let faction = config
        .faction
        .expect("alliance_battleship must declare a faction");
    // Must match the Federation UUID in assets/factions/federation.toml
    let fed_toml = include_str!("../../assets/factions/federation.toml");
    let fed = crate::ai::faction::parse_faction_config(fed_toml).unwrap();
    assert_eq!(faction, fed.uuid, "battleship faction must be Federation");
}

// ── Behaviour block tests ─────────────────────────────────────────────

#[test]
fn behaviour_block_absent_when_not_in_toml() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.behaviour.is_none());
}

#[test]
fn entity_with_hull_and_behaviour_has_both_sections() {
    let toml_str = r##"
tags = ["npc"]

[hull]
hull_integrity = 50.0

[behaviour]
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    assert!(config.hull.is_some());
    assert!(config.behaviour.is_some());
}

// ── DoctrineObjective tests ────────────────────────────────────────────

#[test]
fn behaviour_with_patrol_doctrine_parses() {
    let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "patrol-sector"
text = "Patrol the sector"
directive_kind = "Patrol"
directive_anchors = ["alpha", "beta"]
directive_loop = true
base_priority = 20.0
target_speed = 0.5
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let behaviour = config.behaviour.expect("behaviour must be Some");
    assert_eq!(behaviour.doctrine.len(), 1);
    let d = &behaviour.doctrine[0];
    assert_eq!(d.id, "patrol-sector");
    assert_eq!(d.directive_kind.as_deref(), Some("Patrol"));
    assert_eq!(d.directive_anchors, vec!["alpha", "beta"]);
    assert!(d.directive_loop);
    assert!((d.base_priority - 20.0).abs() < 1e-5);
    assert!((d.target_speed - 0.5).abs() < 1e-5);
}

#[test]
fn behaviour_with_destroy_doctrine_parses() {
    let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
text = "Engage and destroy hostile ships"
directive_kind = "Destroy"
base_priority = 35.0
target_speed = 0.8
maintain_range = 25.0
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let d = &config.behaviour.unwrap().doctrine[0];
    assert_eq!(d.id, "destroy-hostiles");
    assert_eq!(d.directive_kind.as_deref(), Some("Destroy"));
    assert!((d.base_priority - 35.0).abs() < 1e-5);
    assert!((d.maintain_range - 25.0).abs() < 1e-5);
}

#[test]
fn doctrine_target_speed_clamped_to_zero_when_negative() {
    let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "patrol"
text = "Patrol"
base_priority = 10.0
target_speed = -0.5
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let d = &config.behaviour.unwrap().doctrine[0];
    assert_eq!(d.target_speed, 0.0, "negative target_speed must clamp to 0");
}

#[test]
fn doctrine_target_speed_clamped_to_one_when_above_one() {
    let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "pursue"
text = "Pursue"
base_priority = 10.0
target_speed = 1.5
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let d = &config.behaviour.unwrap().doctrine[0];
    assert_eq!(d.target_speed, 1.0, "target_speed > 1 must clamp to 1");
}

#[test]
fn behaviour_doctrine_empty_by_default() {
    let toml_str = r##"
[behaviour]
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let behaviour = config.behaviour.expect("behaviour must be Some");
    assert!(
        behaviour.doctrine.is_empty(),
        "doctrine array must default to empty"
    );
}

#[test]
fn behaviour_multiple_doctrine_objectives_parse() {
    let toml_str = r##"
[behaviour]

[[behaviour.doctrine]]
id = "patrol"
text = "Patrol"
directive_kind = "Patrol"
directive_anchors = ["wp1", "wp2"]
base_priority = 20.0

[[behaviour.doctrine]]
id = "destroy"
text = "Destroy"
directive_kind = "Destroy"
base_priority = 35.0
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let behaviour = config.behaviour.expect("behaviour must be Some");
    assert_eq!(behaviour.doctrine.len(), 2);
    assert_eq!(behaviour.doctrine[0].id, "patrol");
    assert_eq!(behaviour.doctrine[1].id, "destroy");
}

// ── ship_harrow_destroyer.toml compile-time template tests ─────────────
//
// (#892) These three used to load `pirate_raider.toml`, which was retired
// as a duplicate: its display string was literally "Harrow Destroyer", the
// same one `ship_harrow_destroyer.toml` publishes, on a 30-hull ship rather
// than a 900-hull one. They are re-pointed at the surviving hull rather
// than dropped — the claims (a Harrow NPC declares the Harrow faction, a
// positive hull, and both consoles) are about the shipped enemy destroyer,
// not about which file it lived in.

#[test]
fn harrow_destroyer_template_parses_with_harrow_faction() {
    // (#472) The enemy destroyer is Harrow-factioned so the player ship's
    // auto-fire (Federation faction) engages it.
    let toml_str = &resolved_text("ship_harrow_destroyer");
    let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_destroyer.toml must parse");
    let faction = config
        .faction
        .expect("the Harrow Destroyer must declare a faction");
    assert_eq!(
        faction.to_string(),
        "cccccccc-3333-4333-8333-cccccccccccc",
        "the Harrow Destroyer's faction must be Harrow (#472)"
    );
}

#[test]
fn harrow_destroyer_template_has_hull() {
    let toml_str = &resolved_text("ship_harrow_destroyer");
    let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_destroyer.toml must parse");
    assert!(
        config.hull.is_some(),
        "the Harrow Destroyer must have a [hull] section"
    );
    let hull = config.hull.as_ref().unwrap();
    assert!(
        hull.hull_integrity > 0.0,
        "the Harrow Destroyer's [hull] must have a positive hull_integrity value"
    );
}

#[test]
fn harrow_destroyer_template_has_helm_and_weapons_console() {
    let toml_str = &resolved_text("ship_harrow_destroyer");
    let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_destroyer.toml must parse");
    assert!(
        config.helm_console.is_some(),
        "the Harrow Destroyer must have a [helm_console]"
    );
    assert!(
        config.weapons_console.is_some(),
        "the Harrow Destroyer must have a [weapons_console]"
    );
}

// ── Shield arc auto-synthesis tests (issue #514) ─────────────────────────

#[test]
fn shield_arc_toml_block_synthesises_system_instance() {
    // Minimal ship TOML with a single `[[shield_arc]]` block. The
    // parser must synthesise a matching `[[system]]` entry with
    // `kind = "shield_arc"` and `SystemId("shield-arc-<id>")`.
    let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 90
"#;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    assert_eq!(config.shield_arcs.len(), 1);
    assert_eq!(config.shield_arcs[0].id, "fore");

    let ship_config = config
        .ship_config
        .expect("shield_arc must synthesise a ship_config");
    assert_eq!(ship_config.systems.len(), 1);
    let sys = &ship_config.systems[0];
    assert_eq!(sys.id.0, "shield-arc-fore");
    assert_eq!(sys.kind, "shield_arc");
    // No `[shields]` station on this bare ship → arc is ai_only + ownerless.
    assert!(sys.ai_only, "ownerless arc must be ai_only");
    assert!(sys.station.is_none());
}

#[test]
fn shield_arc_synthesises_with_shields_station_when_present() {
    // A ship that declares a `shields` station gets arcs owned by that
    // station with `ai_only = false`.
    let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 180

[[shield_arc]]
id = "aft"
label = "Aft"
center_deg = 180
width_deg = 180

[[station]]
id = "shields"
name = "Shields"
description = "Manage shield systems."
rank = "Ens."
short_code = "SHD"
console = "shields"

[[station.rating]]
name = "Std"
automated_systems = []
"#;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let ship_config = config.ship_config.expect("ship_config present");
    assert_eq!(ship_config.systems.len(), 2);
    for sys in &ship_config.systems {
        assert!(
            !sys.ai_only,
            "with a shields station, arcs are player-controlled"
        );
        assert_eq!(
            sys.station,
            Some(crate::core::messages::StationId("shields".into()))
        );
    }
}

/// The Courier is the two-station player hull. Its TOML-authored system
/// ownership and support loops are individually easy to break, so pin them
/// against the real asset.
#[test]
fn courier_toml_is_a_valid_two_station_hull() {
    use crate::core::messages::{StationId, SystemId};

    let toml_str = include_str!("../../assets/entities/alliance_courier.toml");
    let config = EntityConfig::from_toml(toml_str).expect("alliance_courier must parse");
    let ship_config = config.ship_config.clone().expect("ship_config present");

    // Two stations, one rating each.
    assert_eq!(ship_config.stations.len(), 2);
    let captain = StationId("captain".into());
    let tactical = StationId("tactical".into());
    assert_eq!(ship_config.stations[0].id, captain);
    assert_eq!(ship_config.stations[1].id, tactical);
    assert_eq!(ship_config.stations[0].ratings.len(), 1);
    assert_eq!(ship_config.stations[1].ratings.len(), 1);
    assert_eq!(
        ship_config.stations[0].console.as_deref(),
        Some("gui/courier/captain.html")
    );
    assert_eq!(
        ship_config.stations[1].console.as_deref(),
        Some("gui/courier/tactical.html")
    );

    // The guns live on Tactical, so every ship-level Tactical gate and the
    // WeaponsUpdate broadcast must resolve there.
    assert_eq!(ship_config.weapons_station(), Some(tactical.clone()));

    // Exactly one weapon: one blaster, no phasers, no torpedoes.
    let weapons = config.weapons_console.as_ref().expect("weapons_console");
    assert_eq!(weapons.blaster_banks.len(), 1);
    assert_eq!(weapons.blaster_banks[0].id, "fore");
    assert!(
        weapons.phaser_banks.is_empty(),
        "courier has no phasers — an absent list must not synthesise a default bank"
    );
    assert!(config.torpedoes.is_none(), "courier has no torpedoes");

    // Power is fully authored, with the three canonical groups.
    assert!(
        config.power.is_some(),
        "courier has an authored [power] block"
    );
    assert_eq!(ship_config.power_groups.len(), 3);

    // Every system is owned by Captain or Tactical. Ownerless + ai_only
    // would be inert on the player spawn path.
    for sys in &ship_config.systems {
        assert!(
            matches!(sys.station.as_ref(), Some(station) if station == &captain || station == &tactical),
            "system {:?} must be station-owned",
            sys.id
        );
        assert!(!sys.ai_only, "system {:?} must not rely on ai_only", sys.id);
    }

    // Two arcs, fore and aft, hang off Captain's shields system.
    assert_eq!(config.shield_arcs.len(), 2);
    let arc_ids: Vec<&str> = config.shield_arcs.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(arc_ids, vec!["fore", "aft"]);

    // Red Alert is human-controlled at the Captain station.
    let automated = &ship_config.stations[0].ratings[0].automated_systems;
    assert!(!automated.contains(&SystemId("red-alert".into())));
    assert!(automated.is_empty());

    // Cinematic button only resolves when this block exists.
    assert!(config.cinematic_camera.is_some());

    // One team serves both stations.
    let repair = config.repair.as_ref().expect("[repair] present");
    assert_eq!(repair.repair_team_count, 1);
    assert!(repair.repair_rate_hp_per_sec < 0.5);
}

#[test]
fn battleship_toml_produces_five_shield_arcs() {
    let config = shipped_hull("alliance_battleship");
    assert_eq!(
        config.shield_arcs.len(),
        5,
        "battleship has 5 arcs (fore, starboard, aft, port, omni)"
    );
    let ids: Vec<&str> = config.shield_arcs.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, vec!["fore", "starboard", "aft", "port", "omni"]);

    // Synthesised systems have the expected shape.
    let ship_config = config.ship_config.expect("ship_config present");
    let arc_systems: Vec<_> = ship_config
        .systems
        .iter()
        .filter(|s| s.kind == "shield_arc")
        .collect();
    assert_eq!(arc_systems.len(), 5);
    let sys_ids: Vec<&str> = arc_systems.iter().map(|s| s.id.0.as_str()).collect();
    assert!(sys_ids.contains(&"shield-arc-fore"));
    assert!(sys_ids.contains(&"shield-arc-port"));
    assert!(sys_ids.contains(&"shield-arc-aft"));
    assert!(sys_ids.contains(&"shield-arc-starboard"));
    assert!(sys_ids.contains(&"shield-arc-omni"));
    // Player ship has a shields station → arcs are player-controlled.
    for sys in &arc_systems {
        assert!(!sys.ai_only);
        assert_eq!(
            sys.station,
            Some(crate::core::messages::StationId("shields".into()))
        );
        assert_eq!(sys.power_group, None);
    }
}

#[test]
fn npc_ship_with_single_shield_arc_produces_one_arc_system() {
    // Verify each NPC TOML produces exactly one arc system, ai_only,
    // ownerless (no `shields` station declared on NPCs).
    for (path, expected_max_hp) in [
        // (#892) `pirate_raider.toml` + `pirate_raider_reinforcement.toml`
        // (15 each) were retired as duplicates; the Harrow Destroyer that
        // replaced them in the combat-test waves takes their place here.
        ("../../assets/entities/ship_harrow_destroyer.toml", 40),
        ("../../assets/entities/ship_harrow_patrol.toml", 60),
        ("../../assets/entities/ship_harrow_warhawk.toml", 120),
    ] {
        let toml_str = match path {
            "../../assets/entities/ship_harrow_destroyer.toml" => {
                &resolved_text("ship_harrow_destroyer")
            }
            "../../assets/entities/ship_harrow_patrol.toml" => &resolved_text("ship_harrow_patrol"),
            "../../assets/entities/ship_harrow_warhawk.toml" => {
                &resolved_text("ship_harrow_warhawk")
            }
            _ => unreachable!(),
        };
        let config =
            EntityConfig::from_toml(toml_str).unwrap_or_else(|e| panic!("{path} must parse: {e}"));
        assert_eq!(
            config.shield_arcs.len(),
            1,
            "{path} must declare exactly one shield arc"
        );
        let arc = &config.shield_arcs[0];
        assert_eq!(arc.id, "all", "{path} NPC arc id must be 'all'");
        assert_eq!(arc.max_hp, Some(expected_max_hp), "{path} arc max_hp");

        let ship_config = config
            .ship_config
            .unwrap_or_else(|| panic!("{path} must have ship_config after arc synthesis"));
        let arc_systems: Vec<_> = ship_config
            .systems
            .iter()
            .filter(|s| s.kind == "shield_arc")
            .collect();
        assert_eq!(arc_systems.len(), 1, "{path} exactly one arc system");
        let sys = arc_systems[0];
        assert_eq!(sys.id.0, "shield-arc-all", "{path} SystemId shape");
        assert!(sys.ai_only, "{path} NPC arc must be ai_only");
        assert!(sys.station.is_none(), "{path} NPC arc must be ownerless");
    }
}

#[test]
fn shield_arc_with_hull_max_hp_captures_tier_config() {
    let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 90
hull_max_hp = 7
hull_damaged_threshold_pct = 0.60
hull_disabled_threshold_pct = 0.20
hull_debuff_magnitude = 0.30
"#;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let arc = &config.shield_arcs[0];
    assert_eq!(arc.hull_max_hp, 7.0);
    assert!((arc.hull_damaged_threshold_pct - 0.60).abs() < 1e-6);
    assert!((arc.hull_disabled_threshold_pct - 0.20).abs() < 1e-6);
    assert!((arc.hull_debuff_magnitude - 0.30).abs() < 1e-6);
}

#[test]
fn shield_arc_hull_thresholds_default_when_omitted() {
    let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "fore"
label = "Fore"
center_deg = 0
width_deg = 90
hull_max_hp = 6
"#;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let arc = &config.shield_arcs[0];
    assert!((arc.hull_damaged_threshold_pct - 0.75).abs() < 1e-6);
    assert!((arc.hull_disabled_threshold_pct - 0.25).abs() < 1e-6);
    assert!((arc.hull_debuff_magnitude - 0.15).abs() < 1e-6);
}

#[test]
fn harrow_destroyer_template_has_shields_block() {
    // (#474) The Harrow Destroyer has a single omni shield (#471).
    // (#514) Migrated to `[[shield_arc]]` block; `[shields_console]`
    // block was retired for NPCs.
    // (#892) Re-pointed off the retired `pirate_raider.toml` duplicate. The
    // regen rate is load-bearing here, not incidental: the hull's #788
    // recovery doctrine sits out its standoff orbit at exactly this rate.
    let toml_str = &resolved_text("ship_harrow_destroyer");
    let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_destroyer.toml must parse");
    assert_eq!(
        config.shield_arcs.len(),
        1,
        "the Harrow Destroyer must declare exactly one [[shield_arc]] block"
    );
    let arc = &config.shield_arcs[0];
    assert_eq!(arc.id, "all");
    assert_eq!(arc.max_hp, Some(40));
    assert!((arc.regen_per_sec.expect("regen") - 4.0).abs() < 1e-6);
}

#[test]
fn ship_harrow_patrol_phaser_has_shield_pierce() {
    // (#474) Harrow weapons all have 0.1 pierce.
    // (#892) Re-pointed off the retired `pirate_raider.toml`. The Ironveil,
    // not the Harrow Destroyer, inherits this claim: the Destroyer carries
    // no phaser bank at all (blasters only — see
    // `harrow_destroyer_carries_forward_blasters_and_no_torpedoes`), so it
    // could not carry a phaser-pierce assertion.
    let toml_str = &resolved_text("ship_harrow_patrol");
    let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_patrol.toml must parse");
    let wc = config.weapons_console.as_ref().unwrap();
    let bank = wc.phaser_banks.first().expect("must have a phaser bank");
    assert_eq!(bank.shield_pierce, Some(0.1));
}

#[test]
fn ship_harrow_patrol_template_has_two_phaser_banks_and_shields() {
    // (#474) Cruiser gained weapons + shields.
    // (#514) Migrated to `[[shield_arc]]` block.
    let toml_str = &resolved_text("ship_harrow_patrol");
    let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_patrol.toml must parse");
    let wc = config
        .weapons_console
        .as_ref()
        .expect("cruiser must have [weapons_console] (#474)");
    assert_eq!(
        wc.phaser_banks.len(),
        2,
        "cruiser must have port + starboard banks"
    );
    assert_eq!(
        config.shield_arcs.len(),
        1,
        "cruiser must declare one [[shield_arc]] block"
    );
    let arc = &config.shield_arcs[0];
    assert_eq!(arc.id, "all");
    assert_eq!(arc.max_hp, Some(60));
    assert!((arc.regen_per_sec.expect("regen") - 1.0).abs() < 1e-6);
}

#[test]
fn ship_harrow_warhawk_template_has_full_behaviour_and_weapons() {
    // (#474) Battleship gained a full behaviour tree + weapons +
    // shields. Previously was a stub.
    // (#792) Gained a bow artillery blaster bank alongside the two beam
    // banks — asserted here as well, because "two banks" alone would go on
    // passing if the artillery piece were dropped.
    let toml_str = &resolved_text("ship_harrow_warhawk");
    let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_warhawk.toml must parse");
    let wc = config
        .weapons_console
        .as_ref()
        .expect("battleship must have [weapons_console] (#474)");
    assert_eq!(
        wc.phaser_banks.len(),
        2,
        "battleship must have 2 beam banks"
    );
    let bank = &wc.phaser_banks[0];
    assert!((bank.beam_damage_per_sec - 12.0).abs() < 1e-6);
    assert!((bank.beam_range - 75.0).abs() < 1e-6);
    assert_eq!(
        wc.blaster_banks.len(),
        1,
        "battleship must carry exactly one artillery bank (#792) — the helm \
             doctrine reads its flight speed as the lead speed, and a second, \
             longer-reaching bank would silently become the one it leads by"
    );
    // (#514) Battleship migrated to `[[shield_arc]]` block.
    assert_eq!(
        config.shield_arcs.len(),
        1,
        "battleship must declare one [[shield_arc]] block"
    );
    let arc = &config.shield_arcs[0];
    assert_eq!(arc.id, "all");
    assert_eq!(arc.max_hp, Some(120));
    let behaviour = config.behaviour.as_ref().expect("must have [behaviour]");
    let directive_kinds: Vec<Option<&str>> = behaviour
        .doctrine
        .iter()
        .map(|d| d.directive_kind.as_deref())
        .collect();
    assert!(
        directive_kinds.contains(&Some("Patrol")),
        "battleship must have a Patrol doctrine (#572 doctrine-based AI)"
    );
    assert!(
        directive_kinds.contains(&Some("Destroy")),
        "battleship must have a Destroy doctrine (#572 doctrine-based AI)"
    );
}

// ── The Harrow Battleship artillery platform (issue #792) ────────────────

/// AC1/AC2/AC3, as content: both travel axes author the three-state machine,
/// the yaw channel resolves the SEVENTH mode verb in the hold and tracks on
/// the way in, and every scalar the host reads by name is present on the
/// Steering axis.
///
/// The verb assertion carries the whole of the "why a new verb" argument, so
/// it is spelled out rather than left to the constant: `pivot_to_reengage`
/// has identical geometry to nothing here — its host gate is the six
/// shield-RECOVERY scalars, all of them statements about a ring derived from
/// the TARGET's reach, and an artillery platform authoring five unrelated
/// standoff numbers in order to borrow one turn is exactly the invention
/// AGENTS.md #11 forbids. `hold_torpedo_bearing` is closer and still wrong:
/// it tracks the target's LIVE position with no lead at all, which at this
/// hull's flight time is a different bearing from the one the gun fires on.
#[test]
fn harrow_warhawk_authors_the_artillery_machine_on_both_travel_axes() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    let hc = cfg
        .helm_console
        .as_ref()
        .expect("the hull declares [helm_console]");

    for (name, ai) in [
        ("engines_ai", hc.engines_ai.as_ref()),
        ("steering_ai", hc.steering_ai.as_ref()),
    ] {
        let ai = ai.unwrap_or_else(|| panic!("{name} must be authored"));
        assert!(
            ai.rule.is_empty(),
            "{name} must be state-only (rule XOR state)"
        );
        let ids: Vec<&str> = ai.state.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["shadow", "acquire", "reposition", "hold"],
            "{name} resolves to the class artillery machine"
        );
        // `shadow` and `initial_state = "shadow"` arrive with the class
        // doctrine (issue #878): the shared fragment RESTS defensive on a
        // standoff ring and a hull unlocks the gun line by posture. This hull
        // authors `press_posture = 0.0`, the lowest rung, so the gate is open
        // on the first tick and the defensive leg is left immediately and
        // never re-entered.
        assert_eq!(ai.initial_state.as_deref(), Some("shadow"));
        assert!(
            ai.to_policy().expect("must decode").machine().is_some(),
            "{name} must decode to a machine"
        );
    }

    let steering = hc.steering_ai.as_ref().unwrap();
    let verb_of = |state_id: &str| -> String {
        let state = steering
            .state
            .iter()
            .find(|s| s.id == state_id)
            .unwrap_or_else(|| panic!("steering_ai must declare '{state_id}'"));
        assert_eq!(
            state.rule.len(),
            1,
            "'{state_id}' answers yaw with one rule"
        );
        state.rule[0].verb.clone()
    };
    assert_eq!(verb_of("acquire"), HELM_ACTUATE_DESIRED_FACING_VERB);
    assert_eq!(
        verb_of("reposition"),
        HELM_ACTUATE_DESIRED_FACING_VERB,
        "the run-in tracks the target itself: nothing is being fired at this \
             range, and a run-in aimed at an intercept would arrive beside it"
    );
    assert_eq!(
        verb_of("hold"),
        HELM_HOLD_ARTILLERY_POSITION_VERB,
        "the firing position is the SEVENTH yaw verb — NOT `pivot_to_reengage`, \
             whose host gate is the six shield-recovery scalars this hull would \
             have to invent, and NOT `hold_torpedo_bearing`, which points at where \
             the target IS rather than where the bolt and the target meet"
    );

    // Every scalar the host reads off this axis BY NAME. A rename in either
    // direction lights this up, and it must: the host's response to a missing
    // one is to decline the whole arm and fly ordinary doctrine travel.
    for required in crate::ship::helm_ai::ARTILLERY_PARAMS {
        assert!(
            steering.param.contains_key(*required),
            "steering_ai must author `{required}`: the host gates the whole \
                 artillery arm on all three together, and the throttle this hull \
                 wants (0.0) is indistinguishable from an omission unless the NAME \
                 is present"
        );
    }
    for required in [
        crate::ship::helm_ai::TRACKING_DEADBAND_PARAM,
        crate::ship::helm_ai::TRACKING_FULL_STEER_PARAM,
    ] {
        assert!(
            steering.param.contains_key(required),
            "steering_ai must author `{required}`"
        );
    }
    // ...and the absences that are still absences.
    //
    // Issue #878 composed this hull on `fragments/ai/movement_artillery.toml`,
    // and the class doctrine's DEFENSIVE leg — the standoff ring it rests on
    // while the alert is down — genuinely circles, so the six recovery
    // scalars and the circulation slot arrive with it and are no longer
    // absences to assert. What has NOT changed is that this hull's gun line
    // borrows nothing: it authors no combat-orbit and no bow-hold scalar, so
    // the artillery arm and the class standoff are the only leg sets the host
    // can publish for it. (`press_posture = 0.0` then makes the standoff
    // unreachable in practice — see the doctrine-tuning note on the hull —
    // but the fragment gates those six as one unit, so they stay declared.)
    assert!(
        steering
            .memory
            .contains_key(crate::ship::helm_ai::ORBIT_DIRECTION_MEMORY),
        "the class standoff ring needs its circulation slot declared, so its \
             pre-engagement value is authored rather than implicit"
    );
    for absent in crate::ship::helm_ai::COMBAT_ORBIT_PARAMS
        .iter()
        .chain(crate::ship::helm_ai::TORPEDO_BEARING_PARAMS)
    {
        assert!(
            !steering.param.contains_key(*absent),
            "steering_ai must NOT author `{absent}`: this hull flies one leg \
                 set and borrowing another's scalars is how a doctrine acquires \
                 behaviour nobody authored"
        );
    }
}

/// AC2, as content: the hold band is a PAIR of authored values, the inner one
/// is ninety per cent of the outer, and the outer matches the bolt's own reach.
///
/// The ratio is asserted here rather than computed in Rust deliberately — the
/// point of AGENTS.md #11 is that a designer retunes the band by editing two
/// numbers, and this test is what tells them if they broke the relationship
/// the acceptance criterion names.
#[test]
fn harrow_warhawk_hold_range_is_ninety_percent_of_its_artillery_envelope() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    let steering = cfg
        .helm_console
        .as_ref()
        .and_then(|hc| hc.steering_ai.as_ref())
        .expect("hull authors [helm_console.steering_ai]");
    let max = steering.param[crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM];
    let hold = steering.param[crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM];

    assert!(
        (hold - max * 0.9).abs() < 1e-3,
        "repositioning must stop at ninety per cent of the envelope: \
             {hold} vs {max} * 0.9"
    );
    assert!(
        hold < max,
        "the band must have a gap — one threshold is not hysteresis, it is a \
             boundary the hull sits on and chatters across"
    );

    // The outer edge names the bolt's own reach, so the hull never holds a
    // gun line it cannot shoot down.
    let bank = &cfg
        .weapons_console
        .as_ref()
        .expect("hull declares [weapons_console]")
        .blaster_banks[0];
    assert!(
        (max - bank.range).abs() < 1e-3,
        "the artillery envelope ({max}) must be the bank's own range ({})",
        bank.range
    );

    // Engines runs its own copy of the machine and must reason about the SAME
    // band; a drift between the two axes is a ship whose thrust and yaw
    // disagree about which leg it is flying.
    let engines = cfg
        .helm_console
        .as_ref()
        .and_then(|hc| hc.engines_ai.as_ref())
        .expect("hull authors [helm_console.engines_ai]");
    for name in [
        crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM,
        crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM,
    ] {
        assert_eq!(
            engines.param.get(name),
            steering.param.get(name),
            "both travel axes must author the same `{name}`"
        );
    }
}

/// AC4, as content: the bow bolt is POWERFUL and SLOW, and its slowness is
/// what buys a manoeuvring target time to leave the predicted intercept.
#[test]
fn harrow_warhawk_bow_bolt_is_powerful_and_slow() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    let wc = cfg.weapons_console.as_ref().unwrap();
    let bolt = &wc.blaster_banks[0];
    assert_eq!(bolt.facing_deg, 0.0, "the artillery piece is a BOW mount");

    // POWERFUL: one bolt lands more than either beam bank does in a second of
    // continuous fire, by a wide margin. Compared against the hull's own guns
    // rather than an absolute, so a future rebalance of the beams is what
    // this reads against.
    let beam_dps = wc
        .phaser_banks
        .iter()
        .map(|b| b.beam_damage_per_sec)
        .fold(0.0_f32, f32::max);
    assert!(
        bolt.damage as f32 > beam_dps * 4.0,
        "one artillery bolt ({}) must dwarf a second of beam fire ({beam_dps}): \
             the hull gets one shot every {} s and it has to be worth the wait",
        bolt.damage,
        bolt.cooldown_secs
    );

    // SLOW: slower than every other blaster the game ships, and slow enough
    // that crossing the hull's own envelope takes real seconds — which is the
    // window a course change after launch has to work in.
    //
    // Full paths rather than `shipped_hull` stems because the comparison set
    // deliberately reaches OUTSIDE the shipped fleet: issue #954 moved the
    // three-weapon RNG-coverage escort to `assets/entities/test/`, and its
    // `spike` bank is still a blaster this repo authors. Dropping it because
    // it stopped shipping would quietly shrink the set this claim is measured
    // against, which is the weaker test dressed up as the same one.
    for name in [
        "assets/entities/test/rng_coverage_lancer.toml",
        "assets/entities/ship_harrow_destroyer.toml",
        "assets/entities/alliance_destroyer.toml",
    ] {
        let other = crate::entities::include_resolve::load_entity_config(name)
            .unwrap_or_else(|e| panic!("{name} must compose and parse: {e}"));
        for bank in &other.weapons_console.as_ref().unwrap().blaster_banks {
            assert!(
                bolt.projectile_speed < bank.projectile_speed,
                "the artillery bolt ({}) must be slower than {name}'s '{}' \
                     ({}) — the flight time IS the mechanic",
                bolt.projectile_speed,
                bank.id,
                bank.projectile_speed
            );
        }
    }
    let flight_secs = bolt.range / bolt.projectile_speed;
    assert!(
        flight_secs > 4.0,
        "a bolt must take real seconds ({flight_secs}) to cross the envelope, \
             or 'rewards course changes after launch' is unobservable"
    );

    // How much crossing speed the bow cone admits, which is the failure mode
    // a tighter arc would hide: the fire gate reads the target's CURRENT
    // bearing while the hull is pointed at the intercept, so a cone sized for
    // a stationary target declines exactly the shots the prediction exists to
    // take.
    //
    // ## The lead angle is `asin(v/c)`, and it used to be `atan(v/c)`
    //
    // This derivation was re-authored when the intercept solver stopped being
    // a first-order estimate. For a target crossing square across the line of
    // sight at `v` against a bolt of speed `c`:
    //
    //   * the exact intercept solves `d² + (v·t)² = (c·t)²`, giving
    //     `t = d / sqrt(c² − v²)` and a lead angle of `asin(v/c)`;
    //   * the old estimate solved `t = d / c`, giving `atan(v/c)`.
    //
    // `asin` exceeds `atan` everywhere, and the gap widens fast as `v`
    // approaches `c`. So an EXACT solver asks the cone for MORE headroom than
    // the approximation ever did — the arc has not moved, the honest number
    // for what it must admit has.
    let hulls = [
        "ship_harrow_destroyer",
        "ship_harrow_cruiser",
        "ship_harrow_patrol",
        "alliance_courier",
        "alliance_destroyer",
    ]
    .into_iter()
    .map(shipped_hull)
    .filter_map(|c| c.helm_console)
    .collect::<Vec<_>>();
    let mut cruises = hulls.iter().map(|hc| hc.max_speed).collect::<Vec<_>>();
    cruises.sort_by(f32::total_cmp);
    let half_arc = bolt.fire_arc_deg * 0.5;
    let lead_angle = |v: f32| simmath::asin(v / bolt.projectile_speed).to_degrees();

    // Inverted, the cone's own admission limit: the fastest square-on crosser
    // whose lead still fits inside the half-arc.
    //
    // The inversion is `asin`'s, and `asin` only inverts `sin` on [0, 90].
    // Past a 90 deg half-arc `sin` turns back down, so `admits_crossing`
    // would start SHRINKING as the cone got wider and the pinned finding
    // below would pass trivially while the cone admitted every lead there
    // is — the exact silent pass this whole block exists to prevent. Assert
    // it rather than trusting the reader, because a designer widening
    // `fire_arc_deg` has no reason to come and read this.
    assert!(
        half_arc <= 90.0,
        "this derivation inverts `asin` and is only valid up to a 90 deg \
             half-arc; `fire_arc_deg` is now {} deg. A cone this wide admits \
             every lead, so re-derive the admission limit (or drop it) rather \
             than letting `sin` fold back and pass the finding below for free.",
        bolt.fire_arc_deg
    );
    let admits_crossing = bolt.projectile_speed * simmath::sin(half_arc.to_radians());

    // Every shipped hull is admitted at square-on cruise.
    // This is the property that actually has to hold, and a content change
    // that broke it — a slower bolt, a tighter cone, a general speed-up of the
    // fleet — fails here.
    for &v in &cruises {
        assert!(
            half_arc > lead_angle(v),
            "the bow cone ({} deg) must admit the {} deg lead a {v} u/s \
                 square-on crosser produces",
            bolt.fire_arc_deg,
            lead_angle(v)
        );
    }

    // ## FINDING, pinned rather than papered over
    //
    // The fastest shipped CRUISE no longer fits. The Harrow destroyer crosses
    // at 26 u/s against a 35 u/s bolt, which is `asin(26/35)` ≈ 48 deg of lead
    // — past the 45 deg half-arc. Under the old first-order estimate it read
    // `atan(26/35)` ≈ 37 deg and fitted, but that shot was never going to
    // connect: the estimate was under-leading by a ship's length and more.
    //
    // The consequence is the same bounded, benign one boost has always had:
    // the fire gate finds the target outside the arc and DECLINES, so the
    // battleship holds its round against a full-cruise square-on crosser
    // rather than loosing a mis-aimed bolt. It is only the SQUARE-ON case —
    // any closing or opening component shortens the lead and brings the shot
    // back inside the cone — and the destroyer is the only hull affected.
    //
    // The cone is deliberately NOT widened here. Admitting 26 u/s square-on
    // wants a 96 deg cone, and past that the boost case (2.4× = 62 u/s, which
    // is faster than the bolt and has no intercept at all) is unreachable at
    // any width. That is a tuning decision for a designer — either widen the
    // arc, or speed the bolt up — and it should be made on purpose, not
    // acquired by an assertion quietly relaxing.
    assert!(
        cruises.iter().all(|&v| v <= admits_crossing),
        "the bow cone ({} deg) admits square-on crossers up to \
             {admits_crossing} u/s, so every shipped cruise must fit inside it.",
        bolt.fire_arc_deg
    );
    let fastest_boosted = hulls
        .iter()
        .map(|hc| hc.max_speed * hc.boost.as_ref().map(|b| b.multiplier).unwrap_or(1.0))
        .fold(0.0_f32, f32::max);
    assert!(
        fastest_boosted > bolt.projectile_speed,
        "and a BOOSTED crosser ({fastest_boosted} u/s) outruns the bolt \
             ({} u/s) outright — no cone admits a shot that has no intercept",
        bolt.projectile_speed
    );
}

/// AC4's plumbing: the artillery bank is declared as an AI-operable system
/// under the id the registry derives, or the battleship holds its gun line in
/// silence and every helm assertion above still passes.
#[test]
fn harrow_warhawk_declares_its_artillery_bank_as_a_system() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    let bank_id = cfg.weapons_console.as_ref().unwrap().blaster_banks[0]
        .id
        .clone();
    let expected = crate::ship::system_registry::blaster_bank_system_id(&bank_id)
        .expect("a non-empty bank id resolves to a system id");
    let systems = &cfg
        .ship_config
        .as_ref()
        .expect("hull declares [[system]] blocks")
        .systems;
    let declared = systems
        .iter()
        .find(|s| s.id == expected)
        .unwrap_or_else(|| panic!("hull must declare `{}`", expected.0));
    // Since #871 the hull carries crew stations, so the bank is owned by
    // Tactical rather than being ownerless + `ai_only`. It is still
    // AI-operated on an unmanned hull — the Tactical seat boots on the
    // implicit `Backfill` rating, which automates every system it owns —
    // but the ownership is now what says so, not the `ai_only` flag.
    assert!(
        !declared.ai_only,
        "a station-owned system must not rely on `ai_only`"
    );
    assert_eq!(
        declared.station,
        Some(crate::core::messages::StationId("tactical".into())),
        "the artillery bank belongs to the Tactical seat"
    );
}

/// AC6, as content: nothing in this doctrine is guarded on a hazard.
///
/// The three avoidance layers that DO apply — repulsion summed onto the
/// solved facing inside the pure planner, the lateral-thrust axis nudging the
/// hull off its held point, and the imminent-collision facing override — are
/// all stateless and all outside the machine. A transition guarded on hazard
/// urgency would turn a temporary bend into a state with an exit, which is
/// how an artillery platform becomes an orbiting one.
#[test]
fn harrow_warhawk_authors_no_hazard_guarded_transition() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    let hc = cfg.helm_console.as_ref().unwrap();
    for (name, ai) in [
        ("engines_ai", hc.engines_ai.as_ref().unwrap()),
        ("steering_ai", hc.steering_ai.as_ref().unwrap()),
    ] {
        for state in &ai.state {
            for transition in &state.transition {
                assert!(
                    !transition
                        .when
                        .contains(crate::ship::helm_ai::HAZARD_URGENCY_FACT)
                        && !transition.when.contains("collision"),
                    "{name} state '{}' guards a transition on a hazard reading \
                         (`{}`): avoidance must stay a stateless bend, never a leg",
                    state.id,
                    transition.when
                );
            }
        }
    }
}

/// The battleship switches its impulse drive off, and does it on the axis a
/// scenario cannot reach.
///
/// This is the content half of #792's blocking defect. `entities::spawner`
/// gives an `ImpulseConfigResource` to every hull that declares a
/// `[helm_console]` — parse defaults of engage 200 / cancel 40 — and the
/// impulse autopilot replaces commanded throttle with full thrust while the
/// drive runs. The authored hold band sits ENTIRELY inside that window, so an
/// engaged drive discards the whole doctrine and flies the hull to the drive's
/// release range instead. This hull is the first whose held radius lies there;
/// the cruiser's ring is inside the cancel distance and the destroyer's legs
/// are high-speed passes, so neither sibling ever noticed.
///
/// The two halves asserted here are both load-bearing:
///
/// * an explicit `idle` (not merely an absent block — absent means the
///   canonical UNCONDITIONAL PERMIT is synthesised at spawn, which is the
///   defect), and
/// * the band still sitting inside the drive's default window, which is the
///   reason the idle is needed. If a future retune moved the band clear, this
///   assertion is what says so rather than leaving the `idle` looking like
///   superstition.
///
/// Deliberately NOT expressed as `[[behaviour.doctrine]] use_impulse = false`:
/// doctrine is the part of a hull a scenario replaces wholesale, and both
/// `duel.toml` and `combat_test.toml`'s wave 8 do exactly that without
/// authoring `use_impulse` — which `effective_use_impulse()` then resolves to
/// TRUE. `harrow_warhawk_scenarios_cannot_re_enable_the_impulse_drive` pins
/// that this is not hypothetical.
#[test]
fn harrow_warhawk_holds_its_impulse_drive_idle() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    let hc = cfg.helm_console.as_ref().unwrap();
    let impulse_ai = hc.impulse_ai.as_ref().expect(
        "the battleship must author `[helm_console.impulse_ai]`: an ABSENT block \
             synthesises the canonical unconditional permit at spawn, which is the \
             defect, not the fix",
    );
    assert!(
        impulse_ai.idle,
        "the declaration must be an explicit idle — the impulse channel \
             resolving to nothing, whatever geometry or doctrine the host is handed"
    );
    assert!(
        impulse_ai.rule.is_empty() && impulse_ai.state.is_empty(),
        "an idle declaration carries no rules and no states (content validation \
             rejects the contradiction), so anything here is dead content"
    );

    // The reason it is needed: the authored band lies inside the drive's
    // default cruise window, so an engaged drive would cross the whole of it
    // at `thrust = 1.0`.
    let steering = hc.steering_ai.as_ref().unwrap();
    let hold = steering.param[crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM];
    assert!(
        hc.impulse_cancel_distance < hold && hold < hc.impulse_engage_distance,
        "the hold range ({hold}) sits inside the impulse cruise window \
             (engage {}, cancel {}) — if a retune ever moves it clear, revisit \
             whether the idle above is still earning its place",
        hc.impulse_engage_distance,
        hc.impulse_cancel_distance
    );
}

/// The deliberate absences named in the hull header. All three are exactly the
/// kind of content that gets helpfully filled in later, and each would quietly
/// take the battleship off the firing position this issue exists to hold.
#[test]
fn harrow_warhawk_authors_no_boost_drive_and_no_helm_radar() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    let hc = cfg.helm_console.as_ref().unwrap();
    assert!(
        hc.boost.is_none(),
        "the battleship mounts no boost drive: an artillery platform that lit \
             one would be leaving the firing position it just took up"
    );
    assert_idle_boost_declaration(
        hc,
        "the battleship: no boost doctrine to go with the drive it does not have",
    );
    assert!(
        hc.radar.is_none(),
        "and authors no `[helm_console.radar]`: an unauthored radar range means \
             UNLIMITED helm visibility, which is what lets a {}-unit envelope \
             resolve a target at all",
        hc.steering_ai.as_ref().unwrap().param[crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM]
    );
}

/// The scenarios that replace this hull's doctrine must not be able to switch
/// the drive back on.
///
/// A `use_impulse = false` on the hull's own `[[behaviour.doctrine]]` reads
/// like the natural lever and would have been erased by every scenario that
/// actually fields this hull. That is asserted against the shipped world files
/// rather than described, because the claim is about THEM: each replaces the
/// doctrine list wholesale and none authors `use_impulse`, so
/// `effective_use_impulse()` resolves TRUE for their non-Patrol directives.
/// The fix therefore has to live on the fine system's own policy, which is
/// what the test above pins.
#[test]
fn harrow_warhawk_scenarios_cannot_re_enable_the_impulse_drive() {
    let doctrine = DoctrineObjective {
        directive_kind: Some("Destroy".into()),
        use_impulse: None,
        ..Default::default()
    };
    assert!(
        doctrine.effective_use_impulse(),
        "precondition: an unauthored `use_impulse` on a Destroy directive \
             defaults to permitting the drive — that default is what makes a \
             doctrine-level fix worthless here"
    );

    // The doctrine-replacement marker each world writes. BOTH are
    // [script]-authored since issue #984, so both spell it as a Rhai map and
    // the declarative `behaviour = { doctrine = [` form appears in no
    // shipped world at all. It means the same thing either way: "this
    // scenario replaces a spawned hull's doctrine list".
    for (name, world, doctrine_marker) in [
        (
            "combat_test.toml",
            include_str!("../../assets/worlds/combat_test.toml"),
            "behaviour: #{ doctrine: [",
        ),
        (
            "duel.toml",
            include_str!("../../assets/worlds/duel.toml"),
            "behaviour: #{ doctrine: [",
        ),
    ] {
        assert!(
            world.contains(doctrine_marker),
            "precondition: {name} must replace a spawned hull's doctrine list \
                 for this to be the scenario shape under test"
        );
        assert!(
            !world.contains("use_impulse"),
            "{name} authors `use_impulse` somewhere — if a scenario has started \
                 speaking about the drive, re-read whether the battleship's \
                 `[helm_console.impulse_ai]` idle is still the whole story"
        );
    }
}

/// The structural half of "decline rather than invent": the two range params
/// cannot silently vanish from the FILE, because the doctrine's own
/// transition guards name them and content validation rejects an undeclared
/// `param(...)` at load.
///
/// A second lock on the same door as [`crate::ship::helm_ai::ARTILLERY_PARAMS`]
/// — the host-side gate — and worth having because the two fail at different
/// moments: this one stops the hull existing at all, that one stops a hull
/// that DOES load from flying a leg on a number nobody chose.
///
/// The param lines are struck out of the TOML text rather than out of the
/// parsed struct, because that is where the deletion would actually happen
/// and because `to_policy()` alone does not re-validate references.
#[test]
fn harrow_warhawk_cannot_drop_a_guard_referenced_artillery_range() {
    for (omitted, line) in [
        (
            crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM,
            "max_artillery_range = 200.0",
        ),
        (
            crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM,
            "artillery_hold_range = 180.0",
        ),
    ] {
        assert!(
            &harrow_warhawk_toml().contains(line),
            "precondition: the hull must author `{line}` for this to remove it"
        );
        let stripped = &harrow_warhawk_toml().replace(line, "");
        let err = EntityConfig::from_toml(stripped)
            .expect_err("a guard on an undeclared param must fail the entity load")
            .to_string();
        assert!(
            err.contains("undeclared parameter") && err.contains(omitted),
            "the hull without `{omitted}` must fail to load; got: {err}"
        );
    }
}

// ── The battleship's opportunistic close defence (issue #793) ────────────

/// AC1, as content: the beam battery answers a player who has closed inside
/// the artillery envelope with its WHOLE output, not half of it.
///
/// #792 authored two 180-degree banks on ±90 facings. That covers the circle
/// with no dead zone — but two half-planes only touch, they never overlap, so
/// exactly one bank could bear at any real bearing and the hull's close-in
/// output was 12 damage/s however it was engaged. The seam between them lies
/// dead ahead, which is the one bearing an artillery platform holding a
/// predictive lead solution keeps its target on.
///
/// The ±30 assertions are the ones that discriminate, and they are why the
/// test does not simply read the bow. A bearing of exactly 0 is admitted by
/// BOTH banks under the old 180-degree authoring too — `in_arc` compares with
/// `<=`, so the seam is a boundary tie and a fixture sitting on it proves
/// nothing. Thirty degrees off the bow is inside the new overlap and outside
/// the old one.
#[test]
fn harrow_warhawk_beams_double_up_across_the_bow_for_a_closing_player() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    let wc = cfg
        .weapons_console
        .as_ref()
        .expect("hull declares [weapons_console]");
    let banks = &wc.phaser_banks;
    assert!(banks.len() > 1, "precondition: more than one beam bank");

    for bank in banks {
        assert!(
            (bank.auto_arc_deg - bank.fire_arc_deg).abs() < 1e-3,
            "bank '{}': the AUTO arc must reach as far as the fire arc ({} vs \
                 {}) — the overlap this hull's close defence lives in reaches to \
                 the edge of each bank's cone, and a narrower auto arc switches off \
                 exactly the cover the widening exists to create",
            bank.id,
            bank.auto_arc_deg,
            bank.fire_arc_deg
        );
    }

    // Beam damage available at one bearing, through the same `in_arc` the
    // auto-fire gate uses, summed over every bank that bears.
    let total_dps: f32 = banks.iter().map(|b| b.beam_damage_per_sec).sum();
    let dps_at = |deg: f32| -> f32 {
        let bearing = deg.to_radians();
        banks
            .iter()
            .filter(|b| {
                crate::weapons::phaser::in_arc(
                    simmath::sin(bearing),
                    simmath::cos(bearing),
                    b.facing_deg,
                    b.auto_arc_deg,
                )
            })
            .map(|b| b.beam_damage_per_sec)
            .sum()
    };

    // No dead zone anywhere — the property #792 already had, kept.
    for deg in -179..=180 {
        assert!(
            dps_at(deg as f32) > 0.0,
            "no bearing may be uncovered: {deg} degrees has no bank on it"
        );
    }

    // ...and the bow cone, which is where the hold puts a closing player,
    // gets the WHOLE battery rather than half of it.
    for deg in [-30.0_f32, 0.0, 30.0] {
        assert!(
            (dps_at(deg) - total_dps).abs() < 1e-3,
            "a target {deg} degrees off the bow must be engaged by every bank \
                 ({} of {total_dps} damage/s bears): the artillery hold holds a \
                 closing player on the centreline, so a battery split across it \
                 fights every engagement at half output",
            dps_at(deg)
        );
    }
    // The stern gets it too, which is the half the aft launcher works with.
    assert!(
        (dps_at(180.0) - total_dps).abs() < 1e-3,
        "the stern cone must be covered by every bank as well: a hull turning \
             at 0.20 rad/s cannot keep its nose on a close crosser"
    );

    // Close defence, and only close defence: the beams are for the player who
    // has come inside the gun line, and the gun line is authored much further
    // out. If a retune ever made them reach the holding radius, this hull is
    // no longer holding a standoff it cannot shoot into.
    let hold = cfg
        .helm_console
        .as_ref()
        .and_then(|hc| hc.steering_ai.as_ref())
        .expect("hull authors [helm_console.steering_ai]")
        .param[crate::ship::helm_ai::ARTILLERY_HOLD_RANGE_PARAM];
    for bank in banks {
        assert!(
            bank.beam_range < hold,
            "bank '{}' reaches {} units, at or beyond the {hold}-unit holding \
                 radius — 'close defence' means the player has to close for it",
            bank.id,
            bank.beam_range
        );
    }
}

/// AC2/AC3, as content: two opposed launchers, each gating on ITS OWN
/// readiness, ITS OWN cone, and the arc ITS round would strike.
///
/// The guard's choice of fact is the whole of AC2 and the one thing that
/// cannot be read off behaviour alone, because the wrong fact fails silently:
/// `fact(tubes_full)` is ship-wide (every tube at `volley_max`), which is
/// right for the cruiser's committed salvo and wrong here — a loaded fore tube
/// bearing on a collapsed arc would refuse the shot because the aft tube is
/// eight seconds into a reload, and the two launchers would collapse into one.
/// So the presence of `loaded` and the ABSENCE of `tubes_full` are both
/// asserted.
#[test]
fn harrow_warhawk_carries_two_opposed_launchers_that_decide_independently() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    let torpedoes = cfg
        .torpedoes
        .as_ref()
        .expect("the battleship carries close-defence launchers");

    let ids: Vec<&str> = torpedoes.tubes.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["fore", "aft"],
        "two launchers on OPPOSED facings: the fore tube answers the player \
             who closes down the gun line, the aft one the player who gets behind \
             a hull that cannot turn fast enough to stop them"
    );
    let fore = &torpedoes.tubes[0];
    let aft = &torpedoes.tubes[1];
    assert_eq!(fore.facing_deg, 0.0, "'fore' is a bow launcher");
    assert_eq!(aft.facing_deg, 180.0, "'aft' is a stern launcher");

    for tube in &torpedoes.tubes {
        assert_eq!(
            tube.volley_max, 1,
            "tube '{}' spends ONE round per opportunity — the opposite of the \
                 cruiser's committed salvo, and what makes the two launchers' \
                 reloads independent",
            tube.id
        );
        assert_eq!(
            tube.ai_target_count,
            Some(tube.volley_max),
            "an AI crew keeps tube '{}' loaded between opportunities: the \
                 reload ({} s) outlasts any window it could start inside",
            tube.id,
            torpedoes.load_time
        );
    }

    // The two cones must leave a REAL gap on each beam rather than meeting
    // there. A fore/aft pair whose cones touch has an arc boundary running
    // down each beam line, and `is_in_arc` admits a bearing sitting exactly on
    // it — so every "out of arc" fixture would pass vacuously, and the
    // armament would in truth cover every bearing, which is a turret and not
    // the opportunistic pair this doctrine authors.
    let covered = fore.fire_arc_deg * 0.5 + aft.fire_arc_deg * 0.5;
    assert!(
        covered < 180.0,
        "the fore ({}) and aft ({}) cones must leave the beams uncovered; \
             together they reach {covered} degrees off the centreline",
        fore.fire_arc_deg,
        aft.fire_arc_deg
    );

    // A round that arrives at a recovered arc must do NOTHING — which is why
    // the launch guard below gates on the arc being down instead of treating
    // the shield as something to shoot through.
    assert_eq!(
        torpedoes.damage_shields, 0,
        "these rounds go through a hole the beams made; they cannot make one"
    );
    assert!(
        torpedoes.damage_hull > 0,
        "and they hurt the hull once they are through"
    );

    // Reach. There is no range fact a launch guard can read — the host seeds
    // `in_range` as a constant `true` for every candidate — so a round's own
    // reach is the only thing deciding whether a shot taken at the far edge of
    // the gun line can arrive at all.
    let envelope = cfg
        .helm_console
        .as_ref()
        .and_then(|hc| hc.steering_ai.as_ref())
        .expect("hull authors [helm_console.steering_ai]")
        .param[crate::ship::helm_ai::MAX_ARTILLERY_RANGE_PARAM];
    let reach = torpedoes.speed * torpedoes.lifespan;
    assert!(
        reach >= envelope,
        "a round reaches {reach} units ({} x {} s) but the doctrine holds a \
             {envelope}-unit gun line: shots taken at the far edge would expire \
             short and drain the magazine for nothing",
        torpedoes.speed,
        torpedoes.lifespan
    );

    // The authored per-tube policy — all three of AC2/AC3's conditions, on the
    // launch channel, on EVERY tube, and none of them ship-wide.
    for tube in &torpedoes.tubes {
        let ai = tube
            .ai
            .as_ref()
            .unwrap_or_else(|| panic!("tube '{}' must author its own policy", tube.id));
        assert!(
            validate_fine_system_ai_policy(ai, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS).is_ok(),
            "tube '{}' policy must pass content validation",
            tube.id
        );
        let load = ai
            .rule
            .iter()
            .find(|r| r.channel == TORPEDO_LOAD_CHANNEL)
            .unwrap_or_else(|| panic!("tube '{}' must author a load rule", tube.id));
        assert_eq!(load.verb, TORPEDO_LOAD_VERB);
        let launch = ai
            .rule
            .iter()
            .find(|r| r.channel == TORPEDO_LAUNCH_CHANNEL)
            .unwrap_or_else(|| panic!("tube '{}' must author a launch rule", tube.id));
        assert_eq!(launch.verb, TORPEDO_LAUNCH_VERB);
        for required in ["loaded", "target_facing_shields", "in_arc"] {
            assert!(
                launch.when.contains(required),
                "tube '{}': the launch guard must require `{required}` \
                     continuously, got `{}`",
                tube.id,
                launch.when
            );
        }
        assert!(
            !launch.when.contains("tubes_full"),
            "tube '{}': the launch guard must NOT gate on the SHIP-WIDE \
                 `tubes_full` — with it, a loaded launcher bearing on a downed arc \
                 holds fire because the OTHER launcher is reloading, which is the \
                 exact opposite of AC2's independence. Got `{}`",
            tube.id,
            launch.when
        );
    }

    // Fine systems: one per tube plus the shared magazine. Both the loader and
    // the launcher gate on the magazine before they look at a tube, so its
    // absence switches the whole armament off silently; a missing tube entry
    // leaves that one launcher unloadable, which is the half-battery
    // degradation the per-tube guard above exists to prevent.
    let ship_config = cfg.ship_config.as_ref().expect("hull declares systems");
    let declared =
        |id: &crate::core::messages::SystemId| ship_config.systems.iter().any(|s| &s.id == id);
    assert!(
        declared(&crate::ship::system_registry::torpedo_magazine_system_id()),
        "the shared magazine needs a [[system]] entry or neither loading nor \
             launching runs at all"
    );
    for tube in &torpedoes.tubes {
        let expected = crate::ship::system_registry::torpedo_tube_system_id(&tube.id)
            .expect("a non-empty tube id always resolves");
        assert!(
            declared(&expected),
            "tube '{}' must declare a [[system]] entry `{}`",
            tube.id,
            expected.0
        );
    }
}

/// AC4, as content: arming the hull changed nothing about how it points.
///
/// The torpedo path is launcher-side from end to end — `ai_torpedo_auto_fire`
/// only ever emits `FireTorpedo` at a tube's own system id, and nothing in it
/// writes `ShipPhysics.yaw` or reaches the helm — so AC4 is satisfied by
/// OMISSION, and an omission is exactly the kind of thing a later edit fills
/// in helpfully. This is the lock: the travel axes may not acquire a torpedo
/// leg, a torpedo param, or a torpedo-guarded transition, and the hold must
/// still answer with the artillery verb.
///
/// The cruiser is the counter-example that makes the assertion worth writing:
/// it authors a whole `torpedo_run` state and a `torpedo_bearing_speed`, and
/// copying that shape here would silently trade the predictive bow-artillery
/// facing for one aimed at where the target IS.
#[test]
fn harrow_warhawk_close_defence_adds_no_steering_content() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    assert!(
        cfg.torpedoes.as_ref().is_some_and(|t| !t.tubes.is_empty()),
        "precondition: the hull carries launchers, or this proves nothing"
    );
    let hc = cfg.helm_console.as_ref().unwrap();

    for (name, ai) in [
        ("engines_ai", hc.engines_ai.as_ref().unwrap()),
        ("steering_ai", hc.steering_ai.as_ref().unwrap()),
    ] {
        let ids: Vec<&str> = ai.state.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["shadow", "acquire", "reposition", "hold"],
            "{name} must still be the three-state artillery machine: the \
                 launchers take the bearing the gun line gives them and never ask \
                 for one"
        );
        for param in ai.param.keys() {
            assert!(
                !param.contains("torpedo"),
                "{name} authors a torpedo scalar `{param}`: the tubes are \
                     opportunistic and have no throttle or bearing of their own"
            );
        }
        for state in &ai.state {
            for rule in &state.rule {
                assert!(
                    !rule.verb.contains("torpedo") && !rule.when.contains("torpedo"),
                    "{name} state '{}' answers a channel with torpedo content \
                         (`{}` / `{}`)",
                    state.id,
                    rule.when,
                    rule.verb
                );
            }
            for transition in &state.transition {
                assert!(
                    !transition.when.contains("torpedo"),
                    "{name} state '{}' guards a transition on a torpedo reading \
                         (`{}`): a launcher may never become a leg",
                    state.id,
                    transition.when
                );
            }
        }
    }

    // The verb the whole doctrine turns on, unchanged — and the cruiser's
    // bow-hold scalars still absent, so the host cannot publish that leg for
    // this hull even if a state were added.
    let steering = hc.steering_ai.as_ref().unwrap();
    let hold = steering
        .state
        .iter()
        .find(|s| s.id == "hold")
        .expect("steering_ai declares 'hold'");
    assert_eq!(
        hold.rule[0].verb, HELM_HOLD_ARTILLERY_POSITION_VERB,
        "the firing position must still be aimed by the PREDICTIVE artillery \
             verb, not by anything the launchers wanted"
    );
    for absent in crate::ship::helm_ai::TORPEDO_BEARING_PARAMS {
        assert!(
            !steering.param.contains_key(*absent),
            "steering_ai must not author `{absent}`: it is the cruiser's \
                 bow-hold scalar, and the host gates that whole leg on the name \
                 being present"
        );
    }
}

/// AC5, as content: nothing in the close-defence armament can shove the hull
/// off the position it is holding.
///
/// Only one mechanism in the whole path could: `recoil_impulse`, which
/// `handle_fire_blaster` adds straight onto `ShipPhysics.forward_speed` when
/// it is positive. Phaser beams have no recoil mechanic and a torpedo launch
/// never writes physics at all, so the blaster banks are the entire surface —
/// and #792 authored the artillery piece without one only by leaving the field
/// off, which is a default rather than a decision until something says so.
#[test]
fn harrow_warhawk_close_defence_cannot_shove_it_off_the_firing_position() {
    let cfg = EntityConfig::from_toml(&harrow_warhawk_toml()).expect("hull must parse");
    let banks = &cfg.weapons_console.as_ref().unwrap().blaster_banks;
    assert!(!banks.is_empty(), "precondition: the hull mounts a blaster");
    for bank in banks {
        assert_eq!(
            bank.recoil_impulse, 0.0,
            "bank '{}' authors a recoil impulse ({}): it is added straight onto \
                 `forward_speed` at fire time, so an artillery platform firing one \
                 would walk itself off the gun line it just spent a run-in taking up",
            bank.id, bank.recoil_impulse
        );
    }

    // The other half of "holds station": the hold's own throttle. Restated
    // here because AC5 is about the whole close-defence path, and a non-zero
    // throttle would give ground to a closing player for a different reason.
    let steering = cfg
        .helm_console
        .as_ref()
        .and_then(|hc| hc.steering_ai.as_ref())
        .unwrap();
    assert_eq!(
        steering.param[crate::ship::helm_ai::ARTILLERY_HOLD_SPEED_PARAM],
        0.0,
        "the held throttle must stay zero: a player who closes cannot make \
             this ship give ground"
    );
}

#[test]
fn station_axiom_template_has_explicit_disc_collider() {
    // (#474) Explicit collider for robust hit detection.
    //
    // Both numbers come off the hull the station is DRAWN as, at John's
    // request that collision match visible size. `alliance_starbase.glb`
    // measures 1.8973 x 0.7958 x 1.8936 raw, and the [15, 18, 18] its
    // sidecar applies draws 28.46 x 14.33 x 34.08 — so the widest half-extent
    // is 17.04 and the drawn half-height is 7.16.
    //
    // The shape is what this test now exists to hold. A Ball at 17.04 was
    // right about the width and wrong about the height by a factor of two
    // and a bit; the 12.0 before it was wrong about both. Only a Cylinder
    // can carry the two independently, so a regression to EITHER of those is
    // a regression to a body the renderer does not draw.
    let toml_str = include_str!("../../assets/entities/station_axiom.toml");
    let config = EntityConfig::from_toml(toml_str).expect("station_axiom.toml must parse");
    let collider = config
        .collider
        .as_ref()
        .expect("station_axiom must have explicit [collider] (#474)");
    assert_eq!(collider.shape, ColliderShape::Cylinder);
    assert!(
        (collider.radius - 17.04).abs() < 1e-6,
        "expected the starbase hull's max half-extent, got {}",
        collider.radius
    );
    assert!(
        (collider
            .half_height
            .expect("a Cylinder must author a half-height")
            - 7.16)
            .abs()
            < 1e-6,
        "expected half the starbase hull's drawn height (14.325 / 2), got {:?}",
        collider.half_height
    );
}

/// A mesh's users must agree with each other: two bodies of different sizes
/// cannot both be the thing one GLB draws. That is what let `skyhook` carry
/// a 26 while `station_axiom` carried a 12 off the same starbase model.
///
/// WALKED, not listed. `assets/entities/` is enumerated and every template
/// whose `[mesh].model` is one of the two station GLBs is checked, so a
/// SIXTH user arrives already covered rather than waiting for someone to
/// remember this test. A hard-coded list would not have caught the fifth
/// one — `station_research_outpost.toml`, which draws
/// `alliance_research_outpost.glb` and, until the pass-through fix, authored
/// no `[collider]` at all.
///
/// That gap is now closed: the outpost authors the same disc its mesh-mates
/// do, so every station-mesh user has a body and none is exempt. The walk
/// still counts colliderless users separately, so a NEW one (a station-mesh
/// template that forgets its collider) is a visible failure here rather than
/// a silent pass-through.
#[test]
fn every_station_mesh_user_authors_the_disc_its_mesh_draws() {
    fn templates_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()));
        let mut paths: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "fragments") {
                    continue;
                }
                templates_under(&path, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                out.push(path);
            }
        }
    }

    // (model, radius, half_height) — radius is the widest half-extent of the
    // drawn hull and half_height is half its drawn height, both read off the
    // model's own rig sidecar `[extents].size`.
    let expected: [(&str, f32, f32); 2] = [
        ("assets/models/alliance_starbase.glb", 17.04, 7.16),
        ("assets/models/alliance_research_outpost.glb", 3.8, 1.68),
    ];

    let mut templates = Vec::new();
    templates_under(std::path::Path::new("assets/entities"), &mut templates);
    assert!(!templates.is_empty(), "no templates found");

    let (mut checked, mut colliderless) = (0, 0);
    for path in templates {
        let key = path.to_string_lossy().replace('\\', "/");
        let cfg = crate::entities::include_resolve::load_entity_config(&key)
            .unwrap_or_else(|e| panic!("{key} must parse: {e}"));
        let Some(model) = cfg.mesh.as_ref().and_then(|m| m.model.as_deref()) else {
            continue;
        };
        let Some(&(_, radius, half_height)) = expected.iter().find(|(m, ..)| *m == model) else {
            continue;
        };
        let Some(collider) = cfg.collider.as_ref() else {
            colliderless += 1;
            continue;
        };
        checked += 1;
        assert_eq!(
            collider.shape,
            ColliderShape::Cylinder,
            "{key} draws {model}, so its collider must be the disc that mesh draws"
        );
        assert!(
            (collider.radius - radius).abs() < 1e-6,
            "{key}: expected radius {radius}, got {}",
            collider.radius
        );
        assert!(
            (collider
                .half_height
                .expect("a Cylinder authors a half-height")
                - half_height)
                .abs()
                < 1e-6,
            "{key}: expected half_height {half_height}, got {:?}",
            collider.half_height
        );
    }
    // Five users, all with colliders now: station_axiom, skyhook,
    // depot_transfer, station_outpost, and station_research_outpost (the
    // last was the colliderless gap, closed by the pass-through fix). Pinned
    // so a walk that silently stopped matching anything cannot pass
    // vacuously, and so a new colliderless station-mesh user is a visible
    // failure here rather than a template ships fly straight through.
    assert_eq!(
        checked, 5,
        "expected five station-mesh users with colliders"
    );
    assert_eq!(
        colliderless, 0,
        "every station-mesh user must author a collider now that \
             station_research_outpost has one; a colliderless user is a new \
             pass-through gap, not an exemption"
    );
}

#[test]
fn ship_harrow_patrol_template_has_doctrine_objectives() {
    // (#572) FSM dissolved — NPC hulls use doctrine-based AI. Expects a
    // Patrol objective (sector sweep) and a higher-priority Destroy
    // objective (engage hostiles on sight).
    //
    // (#892) Re-pointed off the retired `pirate_raider.toml`. The Ironveil
    // rather than the Harrow Destroyer, because the Destroyer authors a
    // Destroy entry ONLY — it has no Patrol doctrine for the priority
    // ordering here to compare against, and after #892 the Ironveil is the
    // shipped hull that still carries both.
    let toml_str = &resolved_text("ship_harrow_patrol");
    let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_patrol.toml must parse");
    let behaviour = config
        .behaviour
        .expect("the Ironveil must have a [behaviour] block");
    let ids: Vec<&str> = behaviour.doctrine.iter().map(|d| d.id.as_str()).collect();
    assert!(
        ids.contains(&"patrol-ironveil"),
        "must have patrol-ironveil doctrine"
    );
    assert!(
        ids.contains(&"destroy-hostiles"),
        "must have destroy-hostiles doctrine"
    );
    let destroy = behaviour
        .doctrine
        .iter()
        .find(|d| d.id == "destroy-hostiles")
        .unwrap();
    let patrol = behaviour
        .doctrine
        .iter()
        .find(|d| d.id == "patrol-ironveil")
        .unwrap();
    assert!(
        destroy.base_priority > patrol.base_priority,
        "destroy-hostiles must outscore patrol-ironveil"
    );
}

#[test]
fn harrow_destroyer_doctrine_destroy_has_correct_directive_kind() {
    // (#572) FSM transitions dissolved — engagement logic now lives in the
    // utility scorer. Verify the destroy-hostiles objective carries the
    // Destroy directive kind so `ai_target_selection` picks it up.
    // (#892) Re-pointed off the retired `pirate_raider.toml`.
    let toml_str = &resolved_text("ship_harrow_destroyer");
    let config = EntityConfig::from_toml(toml_str).expect("ship_harrow_destroyer.toml must parse");
    let behaviour = config.behaviour.expect("behaviour must be Some");
    let destroy = behaviour
        .doctrine
        .iter()
        .find(|d| d.id == "destroy-hostiles")
        .expect("destroy-hostiles doctrine must be present");
    assert_eq!(
        destroy.directive_kind.as_deref(),
        Some("Destroy"),
        "destroy-hostiles must carry directive_kind = 'Destroy'"
    );
}

// ── validate_doctrine_directives ───────────────────────────────────────

/// A doctrine-only fixture. Lenient: the subject is directive validation, and
/// a bare `[behaviour]` snippet is not a hull — see
/// [`EntityConfig::from_toml_in_mode`].
fn doctrine_toml(body: &str) -> Result<EntityConfig, String> {
    EntityConfig::from_toml_in_mode(
        &format!("[behaviour]\n\n[[behaviour.doctrine]]\n{body}"),
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .map_err(|e| e.to_string())
}

#[test]
fn reach_directive_with_plural_patrol_anchors_is_rejected() {
    let err = doctrine_toml(
        r#"
id = "reach-destination"
directive_kind = "Reach"
directive_anchors = ["destination"]
"#,
    )
    .unwrap_err();
    assert!(
        err.contains("directive_anchors") && err.contains("Reach"),
        "the error must name the wrong field and the directive kind: {err}"
    );
}

#[test]
fn reach_directive_without_an_anchor_is_rejected() {
    let err = doctrine_toml(
        r#"
id = "reach-destination"
directive_kind = "Reach"
"#,
    )
    .unwrap_err();
    assert!(
        err.contains("directive_anchor"),
        "the error must name the missing field: {err}"
    );
}

#[test]
fn patrol_directive_with_singular_reach_anchor_is_rejected() {
    let err = doctrine_toml(
        r#"
id = "patrol-sector"
directive_kind = "Patrol"
directive_anchor = "alpha"
"#,
    )
    .unwrap_err();
    assert!(err.contains("directive_anchor"), "{err}");
}

#[test]
fn destroy_directive_with_an_anchor_is_rejected() {
    let err = doctrine_toml(
        r#"
id = "destroy-hostiles"
directive_kind = "Destroy"
directive_anchor = "somewhere"
"#,
    )
    .unwrap_err();
    assert!(
        err.contains("directive_anchor") && err.contains("Destroy"),
        "{err}"
    );
}

#[test]
fn directive_field_without_a_directive_kind_is_rejected() {
    let err = doctrine_toml(
        r#"
id = "hold-station"
directive_target = "Starbase Alpha"
"#,
    )
    .unwrap_err();
    assert!(err.contains("directive_target"), "{err}");
}

#[test]
fn unknown_directive_kind_is_rejected() {
    let err = doctrine_toml(
        r#"
id = "wander"
directive_kind = "Wander"
"#,
    )
    .unwrap_err();
    assert!(err.contains("Wander"), "{err}");
}

/// The shapes every shipped hull and world override actually authors.
#[test]
fn well_formed_directives_of_every_kind_are_accepted() {
    for body in [
            "id = \"hold-station\"\nbase_priority = 20.0",
            "id = \"destroy-hostiles\"\ndirective_kind = \"Destroy\"",
            "id = \"assault\"\ndirective_kind = \"Destroy\"\ndirective_target = \"Starbase Alpha\"",
            "id = \"patrol\"\ndirective_kind = \"Patrol\"\ndirective_anchors = [\"a\", \"b\"]\ndirective_loop = true",
            "id = \"reach\"\ndirective_kind = \"Reach\"\ndirective_anchor = \"home\"",
            "id = \"retreat\"\ndirective_kind = \"Retreat\"\ndirective_anchor = \"haven\"",
            "id = \"hail\"\ndirective_kind = \"Hail\"\ndirective_hail_target = \"Axiom Station\"",
        ] {
            assert!(
                doctrine_toml(body).is_ok(),
                "well-formed doctrine must load: {body}"
            );
        }
}

/// Regression: the courier's only goal is a `Reach`, and it must name the
/// singular anchor field or the directive resolves to `""` and never fires.
#[test]
fn requiem_courier_reach_directive_names_a_singular_anchor() {
    let toml_str = include_str!("../../assets/entities/ship_requiem_courier.toml");
    let config = EntityConfig::from_toml(toml_str).expect("ship_requiem_courier.toml must parse");
    let behaviour = config.behaviour.expect("behaviour must be Some");
    let reach = behaviour
        .doctrine
        .iter()
        .find(|d| d.id == "reach-destination")
        .expect("reach-destination doctrine must be present");
    assert_eq!(reach.directive_kind.as_deref(), Some("Reach"));
    assert_eq!(
        reach.directive_anchor.as_deref(),
        Some("requiem_courier_destination"),
        "Reach reads `directive_anchor`; the plural Patrol field is ignored"
    );
}

// ── [torpedoes] block tests ────────────────────────────────────────────

#[test]
fn torpedoes_block_full_round_trips() {
    let toml_str = r##"
[torpedoes]
count = 12
damage_hull = 60
damage_shields = 7
speed = 35.0
turn_rate_deg_per_sec = 90.0
lifespan = 25.0
load_time = 8.0
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let t = config.torpedoes.expect("torpedoes must be Some");
    assert_eq!(t.count, 12);
    assert_eq!(t.damage_hull, 60);
    assert_eq!(t.damage_shields, 7);
    assert_eq!(t.speed, 35.0);
    assert_eq!(t.turn_rate_deg_per_sec, 90.0);
    assert_eq!(t.lifespan, 25.0);
    assert_eq!(t.load_time, 8.0);
}

#[test]
fn torpedoes_block_absent_yields_none() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.torpedoes.is_none());
}

#[test]
fn torpedoes_block_partial_keeps_defaults_for_missing_fields() {
    let toml_str = r##"
[torpedoes]
count = 99
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let t = config.torpedoes.expect("torpedoes must be Some");
    assert_eq!(t.count, 99, "override applied");
    assert_eq!(t.damage_hull, 50, "default preserved");
    assert_eq!(t.damage_shields, 5, "default preserved");
    assert_eq!(t.speed, 15.0, "default preserved");
    assert_eq!(t.turn_rate_deg_per_sec, 45.0, "default preserved");
    assert_eq!(t.lifespan, 20.0, "default preserved");
    assert_eq!(t.load_time, 10.0, "default preserved");
}

#[test]
fn torpedoes_to_runtime_converts_degrees_to_radians() {
    let mut t = TorpedoesConfig::default();
    t.turn_rate_deg_per_sec = 45.0;
    let rt = t.to_runtime();
    assert!(
        (rt.turn_rate - std::f32::consts::FRAC_PI_4).abs() < 1e-5,
        "45 deg/s should convert to PI/4 rad/s, got {}",
        rt.turn_rate
    );
    assert_eq!(rt.count, 10);
    assert_eq!(rt.damage_hull, 50);
    assert_eq!(rt.load_time, 10.0);
}

#[test]
fn torpedoes_defaults_match_runtime_torpedo_config_default() {
    let toml_default = TorpedoesConfig::default().to_runtime();
    let runtime_default = crate::weapons::torpedo::TorpedoConfig::default();
    assert_eq!(toml_default.count, runtime_default.count);
    assert_eq!(toml_default.damage_hull, runtime_default.damage_hull);
    assert_eq!(toml_default.damage_shields, runtime_default.damage_shields);
    assert_eq!(toml_default.speed, runtime_default.speed);
    assert!((toml_default.turn_rate - runtime_default.turn_rate).abs() < 1e-5);
    assert_eq!(toml_default.lifespan, runtime_default.lifespan);
    assert_eq!(toml_default.load_time, runtime_default.load_time);
}

#[test]
fn battleship_toml_torpedoes_block_parses_correctly() {
    // Verify the [torpedoes] block in alliance_battleship.toml parses
    // and produces the expected runtime values.
    let config = shipped_hull("alliance_battleship");
    let t = config
        .torpedoes
        .expect("alliance_battleship must have [torpedoes]");
    let rt = t.to_runtime();
    // Values from alliance_battleship.toml [torpedoes] block
    assert_eq!(rt.count, 30, "battleship magazine size");
    assert_eq!(rt.damage_hull, 40);
    assert_eq!(rt.damage_shields, 4);
    assert_eq!(rt.speed, 15.0);
    assert!((rt.turn_rate - (45f32).to_radians()).abs() < 1e-5);
    assert_eq!(rt.lifespan, 20.0);
    assert_eq!(rt.load_time, 10.0);
}

/// Issue #942: the player destroyer's two launchers spend SMALL volleys.
///
/// The tube COUNT was never the lever and is not what moved — this hull has
/// always carried exactly one fore and one aft launcher, matched by its
/// `torpedo-tube-fore` / `torpedo-tube-aft` hull entries. What moved is the
/// volley: fore 4 -> 2, aft 2 -> 1. At the old sizes six rounds of a
/// twelve-round magazine sat in the tubes and a single bearing could spend
/// all six, so wave one met the whole payload and every wave after it met a
/// hull with nothing to launch.
///
/// This is authored content with no other guard on it: the sizes could drift
/// back up, or the two tubes could even out, and the hull would still parse,
/// still launch, and still pass every other test here. Hence the pin, and
/// hence it pins the ORDERING too — the fore tube is the one whose cone the
/// attack-pass doctrine actually brings to bear, so it is the tube that
/// fires a pair.
#[test]
fn the_player_destroyer_launchers_fire_small_volleys() {
    let config = shipped_hull("alliance_destroyer");
    let t = config
        .torpedoes
        .as_ref()
        .expect("the player destroyer carries torpedo tubes");

    let ids: Vec<&str> = t.tubes.iter().map(|tube| tube.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["fore", "aft"],
        "one launcher on each end and no more: a third tube would restore the \
             per-opportunity payload this hull just gave up"
    );
    let fore = &t.tubes[0];
    let aft = &t.tubes[1];
    assert_eq!(fore.facing_deg, 0.0, "'fore' is a bow launcher");
    assert_eq!(aft.facing_deg, 180.0, "'aft' is a stern launcher");

    assert_eq!(
        fore.volley_max, 2,
        "the bow launcher spends a PAIR per opportunity — the arc the attack \
             pass brings to bear is the one worth two rounds"
    );
    assert_eq!(
        aft.volley_max, 1,
        "the stern launcher spends ONE: a pair spent on whoever got behind is \
             a pair the bow tube does not have for the pass it is flying"
    );
    assert!(
        aft.volley_max < fore.volley_max,
        "the two launchers must stay asymmetric, or 'which tube is worth \
             loading' stops being a decision"
    );

    // Neither tube authors `ai_target_count` and the hull authors no
    // ship-wide `ai_volley_target`, so an AI backfill parks each tube at its
    // own `volley_max`: 3 rounds of the 12-round magazine, not 6. A future
    // `ai_target_count` above `volley_max` would clamp, but one BELOW it
    // would quietly disarm the backfilled hull relative to the human crew,
    // which #838's symmetry does not allow.
    let parked: u32 = t
        .tubes
        .iter()
        .map(|tube| {
            tube.ai_target_count
                .or(t.ai_volley_target)
                .unwrap_or(tube.volley_max)
                .min(tube.volley_max)
        })
        .sum();
    assert_eq!(
        parked, 3,
        "an AI crew must keep both tubes at their authored volleys ({parked} \
             rounds parked); a human crew can ask for the same 3 and no more"
    );
    assert!(
        parked * 3 <= t.count,
        "a full load ({parked}) must stay a small fraction of the {}-round \
             magazine — the magazine is what makes a launch a decision",
        t.count
    );
}

// ── [repair] block tests ───────────────────────────────────────────────

#[test]
fn repair_block_full_round_trips() {
    let toml_str = r##"
[repair]
travel_duration_secs = 7.5
repair_rate_hp_per_sec = 1.25
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let r = config.repair.expect("repair must be Some");
    assert_eq!(r.travel_duration_secs, 7.5);
    assert_eq!(r.repair_rate_hp_per_sec, 1.25);
}

#[test]
fn repair_block_absent_yields_none() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.repair.is_none());
}

#[test]
fn repair_block_partial_keeps_defaults_for_missing_fields() {
    let toml_str = r##"
[repair]
travel_duration_secs = 9.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let r = config.repair.expect("repair must be Some");
    assert_eq!(r.travel_duration_secs, 9.0, "override applied");
    assert_eq!(r.repair_rate_hp_per_sec, 0.5, "default preserved");
}

#[test]
fn repair_to_runtime_preserves_values() {
    let r = RepairConfig {
        travel_duration_secs: 3.0,
        repair_rate_hp_per_sec: 2.0,
        ..Default::default()
    };
    let rt = r.to_runtime();
    assert_eq!(rt.travel_duration, 3.0);
    assert_eq!(rt.repair_rate_hp_per_sec, 2.0);
}

#[test]
fn repair_defaults_match_runtime_repair_timings_default() {
    let toml_default = RepairConfig::default().to_runtime();
    let runtime_default = crate::modifiers::repair_teams::RepairTimings::default();
    assert_eq!(
        toml_default.travel_duration,
        runtime_default.travel_duration
    );
    assert_eq!(
        toml_default.repair_rate_hp_per_sec,
        runtime_default.repair_rate_hp_per_sec
    );
}

#[test]
fn battleship_toml_repair_block_matches_runtime_default_values() {
    // Drift guard: if the [repair] block in alliance_battleship.toml ever diverges
    // from RepairTimings::default(), this test fails so the owner can
    // confirm the change is intentional. (The defaults themselves match
    // the historical hardcoded constants in `repair_teams.rs`.)
    let config = shipped_hull("alliance_battleship");
    let r = config
        .repair
        .expect("alliance_battleship must have [repair]");
    let rt = r.to_runtime();
    let baseline = crate::modifiers::repair_teams::RepairTimings::default();
    assert_eq!(
        rt.travel_duration, baseline.travel_duration,
        "travel duration drift"
    );
    assert_eq!(
        rt.repair_rate_hp_per_sec, baseline.repair_rate_hp_per_sec,
        "repair rate drift"
    );
}

// ── [shields_console.base] block tests ────────────────────────────────

#[test]
fn shields_console_base_block_full_round_trips() {
    let toml_str = r##"
[shields_console]

[shields_console.base]
num_facings = 6
max_hp = 200
regen_per_sec = 7.5
offline_duration = 12.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let sc = config
        .shields_console
        .expect("shields_console must be Some");
    let base = sc.base.expect("base sub-block must be Some");
    assert_eq!(base.num_facings, 6);
    assert_eq!(base.max_hp, 200);
    assert_eq!(base.regen_per_sec, 7.5);
    assert_eq!(base.offline_duration, 12.0);
}

#[test]
fn shields_console_without_base_subblock_yields_none() {
    // The flat focus fields parse fine; absent `[shields_console.base]`
    // must produce `base: None` so the runtime falls back to
    // `ShieldConfig::default()`.
    let toml_str = r##"
[shields_console]
focus_bonus_max_hp = 99
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let sc = config
        .shields_console
        .expect("shields_console must be Some");
    assert!(
        sc.base.is_none(),
        "base sub-block must default to None when absent"
    );
    assert_eq!(sc.focus_bonus_max_hp, 99, "flat focus field still parses");
}

#[test]
fn shields_base_block_partial_keeps_defaults_for_missing_fields() {
    let toml_str = r##"
[shields_console.base]
max_hp = 250
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let base = config
        .shields_console
        .expect("shields_console")
        .base
        .expect("base");
    assert_eq!(base.max_hp, 250, "override applied");
    assert_eq!(base.num_facings, 4, "default preserved");
    assert_eq!(base.regen_per_sec, 2.0, "default preserved");
    assert_eq!(base.offline_duration, 10.0, "default preserved");
}

#[test]
fn shields_base_to_runtime_preserves_values() {
    let base = ShieldsBaseConfig {
        num_facings: 3,
        max_hp: 75,
        regen_per_sec: 2.5,
        offline_duration: 8.0,
    };
    let rt = base.to_runtime();
    assert_eq!(rt.num_facings, 3);
    assert_eq!(rt.max_hp, 75);
    assert_eq!(rt.regen_per_sec, 2.5);
    assert_eq!(rt.offline_duration, 8.0);
}

#[test]
fn shields_base_defaults_match_runtime_shield_config_default() {
    let toml_default = ShieldsBaseConfig::default().to_runtime();
    let runtime_default = crate::weapons::shield::ShieldConfig::default();
    assert_eq!(toml_default.num_facings, runtime_default.num_facings);
    assert_eq!(toml_default.max_hp, runtime_default.max_hp);
    assert_eq!(toml_default.regen_per_sec, runtime_default.regen_per_sec);
    assert_eq!(
        toml_default.offline_duration,
        runtime_default.offline_duration
    );
}

#[test]
fn battleship_toml_shields_base_block_parses_correctly() {
    // Verify the [shields_console.base] block in alliance_battleship.toml
    // parses and produces the expected runtime values.
    let config = shipped_hull("alliance_battleship");
    let base = config
        .shields_console
        .expect("alliance_battleship must have [shields_console]")
        .base
        .expect("alliance_battleship must have [shields_console.base]");
    let rt = base.to_runtime();
    // Values from alliance_battleship.toml [shields_console.base] block
    assert_eq!(rt.max_hp, 140, "battleship shield facing max_hp");
    assert_eq!(rt.regen_per_sec, 3.5, "battleship shield regen");
    assert_eq!(rt.offline_duration, 10.0, "offline duration");
}

// ── PhaserCombatConfig (player phaser tuning) tests ───────────────────
//
// PhaserCombatConfig is built from the per-bank fields on
// [[weapons_console.phaser_banks]]. All combat tuning is per-bank.

#[test]
fn phaser_combat_config_from_weapons_console_clones_banks() {
    let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0
beam_range = 99.0
beam_damage_per_sec = 12.0
beam_duration_secs = 4.0
cooldown_secs = 7.5
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let wc = config.weapons_console.expect("weapons_console");
    let combat = PhaserCombatConfig::from_weapons_console(&wc);
    assert_eq!(combat.banks.len(), 1);
    assert_eq!(combat.banks[0].beam_range, 99.0);
    assert_eq!(combat.banks[0].beam_damage_per_sec, 12.0);
    assert_eq!(combat.banks[0].beam_duration_secs, 4.0);
    assert_eq!(combat.banks[0].cooldown_secs, 7.5);
}

#[test]
fn phaser_combat_config_default_has_empty_banks() {
    let combat = PhaserCombatConfig::default();
    assert!(combat.banks.is_empty());
}

// ── PhaserBankConfig / TorpedoTubeConfig schema tests (Phase A) ───────

#[test]
fn phaser_banks_array_parses_full_entries() {
    let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
beam_range = 35.0

[[weapons_console.phaser_banks]]
id = "starboard"
facing_deg = 90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let wc = config.weapons_console.expect("weapons_console");
    assert_eq!(wc.phaser_banks.len(), 2);
    assert_eq!(wc.phaser_banks[0].id, "port");
    assert_eq!(wc.phaser_banks[0].facing_deg, -90.0);
    assert_eq!(wc.phaser_banks[0].fire_arc_deg, 180.0);
    assert_eq!(wc.phaser_banks[0].auto_arc_deg, 120.0);
    assert_eq!(wc.phaser_banks[0].beam_range, 35.0);
    assert_eq!(wc.phaser_banks[1].id, "starboard");
    assert_eq!(
        wc.phaser_banks[1].beam_range, 0.0,
        "missing beam_range defaults to 0 (caller falls back to parent)"
    );
}

#[test]
fn phaser_bank_shield_pierce_defaults_to_none_when_absent() {
    let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let wc = config.weapons_console.expect("weapons_console");
    assert_eq!(wc.phaser_banks[0].shield_pierce, None);
}

#[test]
fn phaser_bank_shield_pierce_parses_when_present() {
    let toml_str = r##"
[weapons_console]

[[weapons_console.phaser_banks]]
id = "port"
facing_deg = -90.0
fire_arc_deg = 180.0
auto_arc_deg = 120.0
shield_pierce = 0.6
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let wc = config.weapons_console.expect("weapons_console");
    assert_eq!(wc.phaser_banks[0].shield_pierce, Some(0.6));
}

#[test]
fn torpedoes_shield_pierce_defaults_to_zero() {
    let toml_str = r##"
[torpedoes]
count = 5

[[torpedoes.tubes]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let t = config.torpedoes.expect("torpedoes");
    assert_eq!(t.shield_pierce, 0.0);
    // Propagates into the runtime config that the in-flight torpedo
    // snapshots at launch.
    assert_eq!(t.to_runtime().shield_pierce, 0.0);
}

#[test]
fn torpedoes_shield_pierce_parses_when_present() {
    let toml_str = r##"
[torpedoes]
count = 5
shield_pierce = 0.5

[[torpedoes.tubes]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let t = config.torpedoes.expect("torpedoes");
    assert!((t.shield_pierce - 0.5).abs() < 1e-6);
    assert!((t.to_runtime().shield_pierce - 0.5).abs() < 1e-6);
}

#[test]
fn torpedo_in_flight_snapshots_shield_pierce_at_launch() {
    // Wiring proof: changing the in-flight torpedo's snapshot mid-flight
    // doesn't affect future launches (it's a per-torpedo copy).
    use crate::weapons::torpedo::{TorpedoConfig, TorpedoSystem};
    use std::collections::HashMap;
    let mut cfg = TorpedoConfig::default();
    cfg.shield_pierce = 0.75;
    let tubes = vec![TorpedoTubeConfig {
        id: "fore".into(),
        facing_deg: 0.0,
        fire_arc_deg: 90.0,
        load_time: None,
        marker: None,
        barrels: Vec::new(),
        pattern: Vec::new(),
        volley_max: 1,
        ai_target_count: None,
        ai: None,
    }];
    let mut sys = TorpedoSystem::from_configs(&tubes, cfg);
    assert!(sys.start_load("fore"));
    let targets: HashMap<String, (f32, f32, f32)> = HashMap::new();
    sys.tick(sys.config.load_time, &targets, &mut || "test".into());
    sys.launch("fore", "t1".into(), 0.0, 0.0, 0.0, 0.0, None, None);
    assert!((sys.in_flight[0].shield_pierce - 0.75).abs() < 1e-6);

    let det = sys.handle_collision_full("t1").unwrap();
    assert!((det.shield_pierce - 0.75).abs() < 1e-6);
}

#[test]
fn phaser_banks_defaults_to_empty_vec_when_absent() {
    let toml_str = r##"
[weapons_console]
"##;
    // Lenient: a bare `[weapons_console]` owes a `weapons_doctrine`
    // declaration since issue #956, and this fixture is about the serde
    // default for an absent bank list.
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let wc = config.weapons_console.expect("weapons_console");
    assert!(
        wc.phaser_banks.is_empty(),
        "phaser_banks defaults to empty when [[phaser_banks]] absent"
    );
}

#[test]
fn validate_phaser_banks_accepts_valid_list() {
    let banks = vec![
        PhaserBankConfig {
            id: "port".into(),
            facing_deg: -90.0,
            fire_arc_deg: 180.0,
            auto_arc_deg: 120.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
            ..Default::default()
        },
        PhaserBankConfig {
            id: "starboard".into(),
            facing_deg: 90.0,
            fire_arc_deg: 180.0,
            auto_arc_deg: 120.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
            ..Default::default()
        },
    ];
    assert!(validate_phaser_banks(&banks).is_ok());
}

#[test]
fn validate_phaser_banks_rejects_empty_list() {
    let err = validate_phaser_banks(&[]).unwrap_err();
    assert!(err.contains("empty"), "error mentions empty: {err}");
}

#[test]
fn validate_phaser_banks_rejects_duplicate_ids() {
    let banks = vec![
        PhaserBankConfig {
            id: "port".into(),
            facing_deg: -90.0,
            fire_arc_deg: 180.0,
            auto_arc_deg: 90.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
            ..Default::default()
        },
        PhaserBankConfig {
            id: "port".into(),
            facing_deg: 90.0,
            fire_arc_deg: 180.0,
            auto_arc_deg: 90.0,
            beam_range: 0.0,
            shield_pierce: None,
            marker: None,
            ..Default::default()
        },
    ];
    let err = validate_phaser_banks(&banks).unwrap_err();
    assert!(err.contains("duplicate"), "error mentions duplicate: {err}");
    assert!(err.contains("port"));
}

#[test]
fn validate_phaser_banks_rejects_auto_arc_greater_than_fire_arc() {
    let banks = vec![PhaserBankConfig {
        id: "port".into(),
        facing_deg: -90.0,
        fire_arc_deg: 90.0,
        auto_arc_deg: 180.0,
        beam_range: 0.0,
        shield_pierce: None,
        marker: None,
        ..Default::default()
    }];
    let err = validate_phaser_banks(&banks).unwrap_err();
    assert!(
        err.contains("auto_arc_deg"),
        "error mentions auto arc: {err}"
    );
}

/// `cycle_jitter` is rejected at load outside `[0.0, 1.0)` (issue #929).
///
/// The upper bound is the interesting half: the factor scales BOTH the burn and
/// the cooldown, so at 1.0 a draw of exactly zero is admissible — a beam that
/// lights and expires in the same tick, followed by no cooldown at all. Rejected
/// rather than clamped at apply time, because a hull that authored it meant
/// something by it and a silent clamp would hide the mistake in a balance sweep.
#[test]
fn validate_phaser_banks_rejects_cycle_jitter_outside_its_range() {
    let bank = |jitter: f32| PhaserBankConfig {
        id: "port".into(),
        facing_deg: 0.0,
        fire_arc_deg: 270.0,
        auto_arc_deg: 270.0,
        beam_range: 0.0,
        shield_pierce: None,
        marker: None,
        cycle_jitter: jitter,
        ..Default::default()
    };

    for bad in [1.0_f32, 1.5, -0.1] {
        let err = validate_phaser_banks(&[bank(bad)]).unwrap_err();
        assert!(
            err.contains("cycle_jitter"),
            "jitter {bad} must be refused by name: {err}"
        );
    }
    // …and the shipped value, plus the default, are accepted — without which
    // the rows above would pass on a validator that refused everything.
    for good in [0.0_f32, 0.33, 0.99] {
        assert!(
            validate_phaser_banks(&[bank(good)]).is_ok(),
            "jitter {good} is authorable"
        );
    }
}

#[test]
fn validate_phaser_banks_rejects_fire_arc_out_of_range() {
    let banks = vec![PhaserBankConfig {
        id: "port".into(),
        facing_deg: 0.0,
        fire_arc_deg: 400.0,
        auto_arc_deg: 90.0,
        beam_range: 0.0,
        shield_pierce: None,
        marker: None,
        ..Default::default()
    }];
    let err = validate_phaser_banks(&banks).unwrap_err();
    assert!(
        err.contains("fire_arc_deg"),
        "error mentions fire arc: {err}"
    );

    let banks = vec![PhaserBankConfig {
        id: "port".into(),
        facing_deg: 0.0,
        fire_arc_deg: 0.0,
        auto_arc_deg: 0.0,
        beam_range: 0.0,
        shield_pierce: None,
        marker: None,
        ..Default::default()
    }];
    let err = validate_phaser_banks(&banks).unwrap_err();
    assert!(err.contains("fire_arc_deg"), "zero arc rejected: {err}");
}

#[test]
fn torpedo_tubes_array_parses_full_entries() {
    let toml_str = r##"
[torpedoes]
count = 10

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = -30.0
fire_arc_deg = 90.0

[[torpedoes.tubes]]
id = "fore_starboard"
facing_deg = 30.0
fire_arc_deg = 90.0

[[torpedoes.tubes]]
id = "aft"
facing_deg = 180.0
fire_arc_deg = 90.0
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let t = config.torpedoes.expect("torpedoes");
    assert_eq!(t.tubes.len(), 3);
    assert_eq!(t.tubes[0].id, "fore_port");
    assert_eq!(t.tubes[0].facing_deg, -30.0);
    assert_eq!(t.tubes[0].fire_arc_deg, 90.0);
    assert_eq!(t.tubes[2].id, "aft");
    assert_eq!(t.tubes[2].facing_deg, 180.0);
}

#[test]
fn torpedo_tubes_defaults_to_empty_vec_when_absent() {
    let toml_str = r##"
[torpedoes]
count = 10
"##;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let t = config.torpedoes.expect("torpedoes");
    assert!(
        t.tubes.is_empty(),
        "tubes defaults to empty when [[torpedoes.tubes]] absent"
    );
}

#[test]
fn validate_torpedo_tubes_accepts_valid_list() {
    let tubes = vec![
        TorpedoTubeConfig {
            id: "fore_port".into(),
            facing_deg: -30.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        },
        TorpedoTubeConfig {
            id: "aft".into(),
            facing_deg: 180.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        },
    ];
    assert!(validate_torpedo_tubes(&tubes).is_ok());
}

#[test]
fn validate_torpedo_tubes_rejects_empty_list() {
    let err = validate_torpedo_tubes(&[]).unwrap_err();
    assert!(err.contains("empty"));
}

#[test]
fn validate_torpedo_tubes_rejects_duplicate_ids() {
    let tubes = vec![
        TorpedoTubeConfig {
            id: "aft".into(),
            facing_deg: 180.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        },
        TorpedoTubeConfig {
            id: "aft".into(),
            facing_deg: 0.0,
            fire_arc_deg: 90.0,
            load_time: None,
            marker: None,
            barrels: Vec::new(),
            pattern: Vec::new(),
            volley_max: 1,
            ai_target_count: None,
            ai: None,
        },
    ];
    let err = validate_torpedo_tubes(&tubes).unwrap_err();
    assert!(err.contains("duplicate"));
    assert!(err.contains("aft"));
}

#[test]
fn validate_torpedo_tubes_rejects_fire_arc_out_of_range() {
    let tubes = vec![TorpedoTubeConfig {
        id: "aft".into(),
        facing_deg: 180.0,
        fire_arc_deg: 0.0,
        load_time: None,
        marker: None,
        barrels: Vec::new(),
        pattern: Vec::new(),
        volley_max: 1,
        ai_target_count: None,
        ai: None,
    }];
    let err = validate_torpedo_tubes(&tubes).unwrap_err();
    assert!(err.contains("fire_arc_deg"));
}

// ── Torpedo tube barrel-pattern validation (issue #766) ──────────────────

fn torpedo_tube(id: &str) -> TorpedoTubeConfig {
    TorpedoTubeConfig {
        id: id.into(),
        facing_deg: 0.0,
        fire_arc_deg: 90.0,
        load_time: None,
        marker: None,
        barrels: Vec::new(),
        pattern: Vec::new(),
        volley_max: 1,
        ai_target_count: None,
        ai: None,
    }
}

#[test]
fn validate_torpedo_tubes_accepts_legacy_single_barrel() {
    // No barrels + no pattern is the backward-compat single-barrel tube.
    assert!(validate_torpedo_tubes(&[torpedo_tube("fore")]).is_ok());
}

#[test]
fn validate_torpedo_tubes_accepts_valid_pattern() {
    let mut t = torpedo_tube("fore");
    t.barrels = vec!["b0".into(), "b1".into()];
    t.pattern = vec![
        crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![0],
            offset_secs: 0.0,
        },
        crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![0, 1],
            offset_secs: 0.3,
        },
    ];
    assert!(validate_torpedo_tubes(&[t]).is_ok());
}

#[test]
fn validate_torpedo_tubes_rejects_barrel_index_out_of_range() {
    let mut t = torpedo_tube("fore");
    t.barrels = vec!["b0".into(), "b1".into()];
    t.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
        barrels: vec![2], // only 0,1 exist
        offset_secs: 0.0,
    }];
    let err = validate_torpedo_tubes(&[t]).unwrap_err();
    assert!(err.contains("barrel index 2"), "{err}");
}

#[test]
fn validate_torpedo_tubes_rejects_negative_offset() {
    let mut t = torpedo_tube("fore");
    t.barrels = vec!["b0".into()];
    t.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
        barrels: vec![0],
        offset_secs: -0.5,
    }];
    let err = validate_torpedo_tubes(&[t]).unwrap_err();
    assert!(err.contains("offset_secs"), "{err}");
}

#[test]
fn validate_torpedo_tubes_rejects_empty_step() {
    let mut t = torpedo_tube("fore");
    t.barrels = vec!["b0".into()];
    t.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
        barrels: vec![],
        offset_secs: 0.0,
    }];
    assert!(validate_torpedo_tubes(&[t]).is_err());
}

#[test]
fn validate_torpedo_tubes_rejects_multi_barrel_without_pattern() {
    let mut t = torpedo_tube("fore");
    t.barrels = vec!["b0".into(), "b1".into()];
    // No pattern: under-specified for >1 barrel.
    let err = validate_torpedo_tubes(&[t]).unwrap_err();
    assert!(err.contains("pattern"), "{err}");
}

// ── Blaster bank validation (issue #765) ─────────────────────────────────

fn blaster_bank(id: &str) -> BlasterBankConfig {
    BlasterBankConfig {
        id: id.into(),
        fire_arc_deg: 90.0,
        ..BlasterBankConfig::default()
    }
}

#[test]
fn validate_blaster_banks_accepts_empty_list() {
    // Most hulls carry no blasters; an empty list is fine.
    assert!(validate_blaster_banks(&[]).is_ok());
}

#[test]
fn validate_blaster_banks_accepts_legacy_single_barrel() {
    // No barrels + no pattern is the backward-compat single-barrel bank.
    let banks = vec![blaster_bank("fore")];
    assert!(validate_blaster_banks(&banks).is_ok());
}

#[test]
fn validate_blaster_banks_accepts_valid_pattern() {
    let mut b = blaster_bank("fore");
    b.barrels = vec!["b0".into(), "b1".into()];
    b.pattern = vec![
        crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![0],
            offset_secs: 0.0,
        },
        crate::weapons::pattern::BarrelPatternStep {
            barrels: vec![0, 1],
            offset_secs: 0.3,
        },
    ];
    assert!(validate_blaster_banks(&[b]).is_ok());
}

#[test]
fn validate_blaster_banks_rejects_barrel_index_out_of_range() {
    let mut b = blaster_bank("fore");
    b.barrels = vec!["b0".into(), "b1".into()];
    b.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
        barrels: vec![2], // only 0,1 exist
        offset_secs: 0.0,
    }];
    let err = validate_blaster_banks(&[b]).unwrap_err();
    assert!(err.contains("barrel index 2"), "{err}");
}

#[test]
fn validate_blaster_banks_rejects_negative_offset() {
    let mut b = blaster_bank("fore");
    b.barrels = vec!["b0".into()];
    b.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
        barrels: vec![0],
        offset_secs: -0.5,
    }];
    let err = validate_blaster_banks(&[b]).unwrap_err();
    assert!(err.contains("offset_secs"), "{err}");
}

#[test]
fn validate_blaster_banks_rejects_empty_step() {
    let mut b = blaster_bank("fore");
    b.barrels = vec!["b0".into()];
    b.pattern = vec![crate::weapons::pattern::BarrelPatternStep {
        barrels: vec![],
        offset_secs: 0.0,
    }];
    assert!(validate_blaster_banks(&[b]).is_err());
}

#[test]
fn validate_blaster_banks_rejects_multi_barrel_without_pattern() {
    let mut b = blaster_bank("fore");
    b.barrels = vec!["b0".into(), "b1".into()];
    // No pattern: under-specified for >1 barrel.
    let err = validate_blaster_banks(&[b]).unwrap_err();
    assert!(err.contains("pattern"), "{err}");
}

#[test]
fn validate_blaster_banks_rejects_duplicate_ids() {
    let banks = vec![blaster_bank("fore"), blaster_bank("fore")];
    let err = validate_blaster_banks(&banks).unwrap_err();
    assert!(err.contains("duplicate"));
}

// ── Inline fine-system AI policy (issue #775) ────────────────────────────

const CHANNELS: &[&str] = &[CAPTAIN_RED_ALERT_CHANNEL];
const VERBS: &[&str] = &[CAPTAIN_SET_RED_ALERT_VERB];

fn captain_ai_toml() -> &'static str {
    r#"
name = "Test Cruiser"

[captain_console.ai]
param = { combat_window_secs = 8.0 }

[[captain_console.ai.rule]]
priority = 10
channel = "red_alert"
when = "fact(secs_since_combat) < param(combat_window_secs)"
verb = "set_red_alert"
value = true

[[captain_console.ai.rule]]
priority = 0
channel = "red_alert"
when = "true"
verb = "set_red_alert"
value = false
"#
}

#[test]
fn captain_ai_policy_parses_and_resolves_to_typed_policy() {
    let cfg = EntityConfig::from_toml(captain_ai_toml()).expect("parse must succeed");
    let ai = cfg
        .captain_console
        .as_ref()
        .and_then(|c| c.ai.as_ref())
        .expect("captain_console.ai present");
    assert_eq!(ai.param.get("combat_window_secs"), Some(&8.0));
    assert_eq!(ai.rule.len(), 2);
    let policy = ai.to_policy().expect("policy resolves");
    assert_eq!(policy.rules.len(), 2);
    assert!(!policy.idle);
}

#[test]
fn default_captain_policy_validates_and_resolves() {
    let cfg = crate::entities::authored_ai_pins::shipped_policy_toml("captain");
    assert!(validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).is_ok());
    assert!(cfg.to_policy().is_ok());
}

// ── Optional stateful policy schema (issue #882) ─────────────────────────

/// AC7 — THE back-compat guard. Every shipped stateless block still parses
/// AND decodes to a policy with NO machine, no states and no memory: the
/// #882 schema fields are all `#[serde(default)]`, so nothing an author
/// wrote before this issue changed meaning. Enumerates all fourteen
/// canonical defaults behind the twelve Group A hosts.
#[test]
fn every_shipped_stateless_default_still_parses_as_stateless() {
    let shipped: Vec<(&str, FineSystemAiConfigToml)> = vec![
        (
            "captain",
            crate::entities::authored_ai_pins::shipped_policy_toml("captain"),
        ),
        (
            "comms_response",
            crate::entities::authored_ai_pins::shipped_policy_toml("comms_response"),
        ),
        (
            "engines",
            crate::entities::authored_ai_pins::shipped_policy_toml("engines"),
        ),
        (
            "steering",
            crate::entities::authored_ai_pins::shipped_policy_toml("steering"),
        ),
        (
            "lateral",
            crate::entities::authored_ai_pins::shipped_policy_toml("lateral"),
        ),
        (
            "vertical",
            crate::entities::authored_ai_pins::shipped_policy_toml("vertical"),
        ),
        (
            "impulse",
            crate::entities::authored_ai_pins::shipped_policy_toml("impulse"),
        ),
        (
            "boost",
            crate::entities::authored_ai_pins::shipped_policy_toml("boost"),
        ),
        (
            "phaser_bank",
            crate::entities::authored_ai_pins::shipped_policy_toml("phaser_bank"),
        ),
        (
            "blaster_bank",
            crate::entities::authored_ai_pins::shipped_policy_toml("blaster_bank"),
        ),
        (
            "torpedo_tube",
            crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_tube"),
        ),
        (
            "torpedo_magazine",
            crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_magazine"),
        ),
        (
            "shields_focus",
            crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus"),
        ),
        (
            "power",
            crate::entities::authored_ai_pins::shipped_policy_toml("power"),
        ),
    ];
    for (name, cfg) in shipped {
        assert!(
            cfg.initial_state.is_none() && cfg.state.is_empty() && cfg.memory.is_empty(),
            "{name}: a shipped default must declare no #882 state machine"
        );
        let policy = cfg
            .to_policy()
            .unwrap_or_else(|e| panic!("{name}: must decode: {e}"));
        assert!(
            policy.machine().is_none() && policy.initial_state().is_none(),
            "{name}: a stateless block must decode to `machine: None`"
        );
        assert_eq!(
            policy.rules.len(),
            cfg.rule.len(),
            "{name}: every authored rule still decodes to a top-level rule"
        );
    }
}

/// A minimal authored stateful block: `initial_state`, two states with
/// their own continuous rules, explicitly prioritised transitions, and a
/// typed private memory declaration (AC1).
fn stateful_boost_toml() -> &'static str {
    r#"
name = "Stateful"
[helm_console.boost_ai]
initial_state = "cruise"

[helm_console.boost_ai.param]
surge_urgency = 0.5
surge_dwell_secs = 3.0
max_engagements = 3.0

[helm_console.boost_ai.memory]
engagements = 0.0

[[helm_console.boost_ai.state]]
id = "cruise"

[[helm_console.boost_ai.state.transition]]
priority = 10
to = "surge"
when = "fact(hazard_urgency) > param(surge_urgency) and memory(engagements) < param(max_engagements)"

[[helm_console.boost_ai.state]]
id = "surge"

[[helm_console.boost_ai.state.rule]]
priority = 0
channel = "boost"
when = "true"
verb = "engage_boost"

[[helm_console.boost_ai.state.transition]]
priority = 0
to = "cruise"
when = "state_time >= param(surge_dwell_secs)"
"#
}

/// AC1: an authored stateful block round-trips through the TOML schema into
/// the typed machine, with per-state rules and prioritised transitions.
#[test]
fn stateful_policy_round_trips_from_toml_to_typed_machine() {
    let cfg = EntityConfig::from_toml(stateful_boost_toml()).expect("parse must succeed");
    let ai = cfg
        .helm_console
        .as_ref()
        .and_then(|h| h.boost_ai.as_ref())
        .expect("helm_console.boost_ai present");
    assert_eq!(ai.initial_state.as_deref(), Some("cruise"));
    assert_eq!(ai.state.len(), 2);
    assert_eq!(ai.memory.get("engagements"), Some(&0.0));

    let policy = ai.to_policy().expect("policy resolves");
    let machine = policy.machine().expect("machine decoded");
    assert_eq!(machine.initial, "cruise");
    assert_eq!(machine.states.len(), 2);
    assert!(
        policy.rules.is_empty(),
        "a purely stateful policy carries no top-level rules"
    );
    let cruise = machine.state("cruise").expect("cruise declared");
    assert!(cruise.rules.is_empty());
    assert_eq!(cruise.transitions.len(), 1);
    assert_eq!(cruise.transitions[0].to, "surge");
    assert_eq!(cruise.transitions[0].priority, 10);
    let surge = machine.state("surge").expect("surge declared");
    assert_eq!(surge.rules.len(), 1);
    assert_eq!(surge.rules[0].channel, HELM_BOOST_CHANNEL);
    assert_eq!(
        surge.rules[0].verb,
        crate::ai::policy::AiPolicyVerb::EngageBoost
    );
    assert_eq!(machine.initial_memory.get("engagements"), Some(0.0));
}

/// **Issue #918: whether a doctrine leg yields its solved facing to a
/// channel-3 arc-bearing request is AUTHORED on the leg.**
///
/// Three properties, and the first is the one that keeps #673-#684 working:
/// a leg that says nothing yields, so every hull authored before this field
/// existed — and every helm with no doctrine at all — behaves exactly as it
/// did. The second is that `false` reaches the typed policy. The third is
/// that the host's question is answered off the CURRENT leg and off nothing
/// else: not off the verb, not off the state's name, and with no parameter
/// through which the requester could be consulted.
#[test]
fn a_doctrine_leg_authors_whether_it_yields_to_arc_requests() {
    let hull = r#"
name = "Committed"
[helm_console.steering_ai]
initial_state = "travel"

[[helm_console.steering_ai.state]]
id = "travel"

  [[helm_console.steering_ai.state.rule]]
  priority = 0
  channel = "yaw"
  when = "true"
  verb = "actuate_desired_facing"

  [[helm_console.steering_ai.state.transition]]
  priority = 0
  to = "committed"
  when = "true"

[[helm_console.steering_ai.state]]
id = "committed"
yields_to_arc_requests = false

  [[helm_console.steering_ai.state.rule]]
  priority = 0
  channel = "yaw"
  when = "true"
  verb = "hold_committed_heading"
"#;
    let cfg = EntityConfig::from_toml(hull).expect("the authored hull must parse and validate");
    let steering = cfg
        .helm_console
        .as_ref()
        .and_then(|h| h.steering_ai.as_ref())
        .expect("hull declares [helm_console.steering_ai]");
    assert!(
        steering.state[0].yields_to_arc_requests,
        "an omitted declaration must parse as YIELDING — the pre-#918 behaviour \
             every authored hull and every doctrine-less helm depends on"
    );
    assert!(!steering.state[1].yields_to_arc_requests);

    let policy = steering.to_policy().expect("the authored policy decodes");
    let machine = policy.machine().expect("machine decoded");
    assert!(
        machine
            .state("travel")
            .expect("travel declared")
            .yields_to_arc_requests
    );
    assert!(
        !machine
            .state("committed")
            .expect("committed declared")
            .yields_to_arc_requests,
        "the declaration must survive into the typed policy the host reads"
    );

    // The host's question, asked of one leg at a time.
    assert!(policy.leg_yields_to_arc_requests(Some("travel")));
    assert!(!policy.leg_yields_to_arc_requests(Some("committed")));
    assert!(
        policy.leg_yields_to_arc_requests(None),
        "a machine that has entered nothing has committed to no heading"
    );
    assert!(
        policy.leg_yields_to_arc_requests(Some("no-such-leg")),
        "an unknown leg is not a licence to ignore Channel 3"
    );

    // ...and a STATELESS policy — the shape a helm with no authored
    // doctrine flies — has no legs to decline with, whatever it is asked.
    let stateless = crate::ai::policy::AiPolicy::default();
    assert!(stateless.leg_yields_to_arc_requests(None));
    assert!(stateless.leg_yields_to_arc_requests(Some("committed")));
}

/// Issue #918: the declaration is rejected on a system that could never read
/// it. An arc-bearing request is answered on the `yaw` channel; authored on
/// the boost machine, `yields_to_arc_requests = false` is a line a designer
/// would reasonably expect to do something and that nothing would ever
/// consult — so it fails the load rather than reading as a silent no-op.
#[test]
fn declining_arc_requests_is_rejected_on_a_system_that_does_not_steer() {
    let leg = |yields: bool| FineSystemAiStateToml {
        id: "cruise".to_string(),
        yields_to_arc_requests: yields,
        ..Default::default()
    };

    let err = validate_fine_system_ai_policy(
        &stateful_cfg(Some("cruise"), vec![leg(false)]),
        BOOST_CHANNELS,
        BOOST_VERBS,
    )
    .unwrap_err();
    assert!(err.contains("cruise"), "must name the state: {err}");
    assert!(
        err.contains("yields_to_arc_requests"),
        "must name the offending declaration: {err}"
    );

    // The same machine is fine on the axis that steers...
    assert!(validate_fine_system_ai_policy(
        &stateful_cfg(Some("cruise"), vec![leg(false)]),
        STEERING_CHANNELS,
        STEERING_VERBS,
    )
    .is_ok());
    // ...and leaving the default standing is fine anywhere, which is why
    // every already-authored hull keeps loading.
    assert!(validate_fine_system_ai_policy(
        &stateful_cfg(Some("cruise"), vec![leg(true)]),
        BOOST_CHANNELS,
        BOOST_VERBS,
    )
    .is_ok());
}

/// Build a stateful policy config for the AC6 rejection cases directly, so
/// each rejection is isolated from TOML surface noise.
fn stateful_cfg(
    initial: Option<&str>,
    states: Vec<FineSystemAiStateToml>,
) -> FineSystemAiConfigToml {
    FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: std::collections::HashMap::new(),
        rule: Vec::new(),
        initial_state: initial.map(str::to_string),
        state: states,
        memory: std::collections::HashMap::new(),
    }
}

fn boost_state(id: &str, to: &[&str]) -> FineSystemAiStateToml {
    FineSystemAiStateToml {
        id: id.to_string(),
        rule: Vec::new(),
        transition: to
            .iter()
            .map(|t| FineSystemAiTransitionToml {
                priority: 0,
                to: t.to_string(),
                when: "true".to_string(),
            })
            .collect(),
        ..Default::default()
    }
}

/// AC6: an `initial_state` naming a state that was never declared.
#[test]
fn undeclared_initial_state_is_rejected() {
    let cfg = stateful_cfg(Some("nowhere"), vec![boost_state("cruise", &[])]);
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(
        err.contains("initial_state") && err.contains("nowhere"),
        "got: {err}"
    );
}

/// AC6: states declared with no `initial_state` at all is the same defect —
/// there is no entry point.
#[test]
fn states_without_an_initial_state_are_rejected() {
    let cfg = stateful_cfg(None, vec![boost_state("cruise", &[])]);
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(err.contains("initial_state"), "got: {err}");
    // The decoder refuses it too, so a caller skipping validation cannot
    // build a half-machine.
    assert!(cfg.to_policy().is_err());
}

/// AC6: a transition targeting a state that was never declared.
#[test]
fn transition_to_undeclared_state_is_rejected() {
    let cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &["surge"])]);
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(
        err.contains("undeclared state") && err.contains("surge"),
        "got: {err}"
    );
}

/// AC6: duplicate state ids — "which `cruise` did you mean?" has no answer.
#[test]
fn duplicate_state_ids_are_rejected() {
    let cfg = stateful_cfg(
        Some("cruise"),
        vec![boost_state("cruise", &[]), boost_state("cruise", &[])],
    );
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(err.contains("duplicate state id"), "got: {err}");
}

/// AC6: an unreachable state — neither the initial state nor any
/// transition's target. A self-loop does NOT make a state reachable.
#[test]
fn unreachable_state_is_rejected() {
    let cfg = stateful_cfg(
        Some("cruise"),
        vec![
            boost_state("cruise", &[]),
            boost_state("orphan", &["orphan"]),
        ],
    );
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(
        err.contains("unreachable state") && err.contains("orphan"),
        "got: {err}"
    );
    // ...but wiring it up from the initial state makes it legal.
    let ok = stateful_cfg(
        Some("cruise"),
        vec![
            boost_state("cruise", &["orphan"]),
            boost_state("orphan", &[]),
        ],
    );
    assert!(validate_fine_system_ai_policy(&ok, BOOST_CHANNELS, BOOST_VERBS).is_ok());
}

/// AC6, the transitive case: a DISCONNECTED CLUSTER. `cruise` is the
/// initial state; `drift` and `wander` transition to each other but nothing
/// reaches either of them. Both are "the target of a transition", so a
/// single pass that credits every transition target regardless of whether
/// its source is itself reachable accepts this graph — which is exactly the
/// dead branch AC6 exists to reject. Reachability has to be a fixpoint walk
/// from `initial`.
#[test]
fn disconnected_state_cluster_is_rejected() {
    let cfg = stateful_cfg(
        Some("cruise"),
        vec![
            boost_state("cruise", &[]),
            boost_state("drift", &["wander"]),
            boost_state("wander", &["drift"]),
        ],
    );
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(
        err.contains("unreachable state") && (err.contains("drift") || err.contains("wander")),
        "got: {err}"
    );
    // Wiring ONE edge from the initial state into the cluster makes the
    // whole cluster reachable — the walk is transitive, not one-hop.
    let ok = stateful_cfg(
        Some("cruise"),
        vec![
            boost_state("cruise", &["drift"]),
            boost_state("drift", &["wander"]),
            boost_state("wander", &["drift"]),
        ],
    );
    assert!(validate_fine_system_ai_policy(&ok, BOOST_CHANNELS, BOOST_VERBS).is_ok());
}

/// Build one transition, spelled out, for the tie cases below —
/// `boost_state` hardcodes priority 0, which is the very thing under test.
fn transition(priority: i32, to: &str) -> FineSystemAiTransitionToml {
    FineSystemAiTransitionToml {
        priority,
        to: to.to_string(),
        when: "true".to_string(),
    }
}

/// One unconditional tube rule at an explicit `(priority, channel)`.
fn tube_rule(priority: i32, channel: &str, verb: &str) -> FineSystemAiRuleToml {
    FineSystemAiRuleToml {
        priority,
        channel: channel.to_string(),
        when: "true".to_string(),
        verb: verb.to_string(),
        value: false,
        level: 0,
        response_index: 0,
    }
}

/// Issue #794 / PRD #774: two transitions out of ONE state at the same
/// priority.
///
/// The runtime does not stall on this — it silently takes the
/// earliest-authored of the two, so the file reads as if the pair were
/// interchangeable while the outcome depends entirely on which table was
/// typed first.
#[test]
fn equal_priority_transitions_out_of_one_state_are_rejected() {
    let tie = stateful_cfg(
        Some("cruise"),
        vec![
            FineSystemAiStateToml {
                id: "cruise".to_string(),
                rule: Vec::new(),
                transition: vec![transition(3, "surge"), transition(3, "coast")],
                ..Default::default()
            },
            boost_state("surge", &[]),
            boost_state("coast", &[]),
        ],
    );
    let err = validate_fine_system_ai_policy(&tie, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    // The message has to be actionable without opening this file: WHICH
    // state, WHICH priority, and WHICH TWO targets are competing.
    assert!(err.contains("state 'cruise'"), "must name the state: {err}");
    assert!(
        err.contains("same priority 3"),
        "must name the duplicated priority: {err}"
    );
    assert!(
        err.contains("'surge'") && err.contains("'coast'"),
        "must name both competing targets: {err}"
    );

    // Separating them by one is the whole fix.
    let ok = stateful_cfg(
        Some("cruise"),
        vec![
            FineSystemAiStateToml {
                id: "cruise".to_string(),
                rule: Vec::new(),
                transition: vec![transition(3, "surge"), transition(2, "coast")],
                ..Default::default()
            },
            boost_state("surge", &[]),
            boost_state("coast", &[]),
        ],
    );
    assert!(validate_fine_system_ai_policy(&ok, BOOST_CHANNELS, BOOST_VERBS).is_ok());
}

/// The scope of the transition tie is ONE state's transition set. Two
/// different states each authoring a priority-0 exit are not competing —
/// only one of them is ever the current state — and rejecting that would
/// make the common two-state machine unauthorable.
#[test]
fn equal_priorities_in_different_states_are_not_a_transition_tie() {
    let cfg = stateful_cfg(
        Some("cruise"),
        vec![
            FineSystemAiStateToml {
                id: "cruise".to_string(),
                rule: Vec::new(),
                transition: vec![transition(0, "surge")],
                ..Default::default()
            },
            FineSystemAiStateToml {
                id: "surge".to_string(),
                rule: Vec::new(),
                transition: vec![transition(0, "cruise")],
                ..Default::default()
            },
        ],
    );
    assert!(validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).is_ok());
}

/// Issue #794 / PRD #774: two rules on ONE output channel at the same
/// priority, in a STATELESS policy's top-level list.
#[test]
fn equal_priority_rules_on_one_channel_are_rejected() {
    let stateless = |rules: Vec<FineSystemAiRuleToml>| FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: std::collections::HashMap::new(),
        rule: rules,
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let tie = stateless(vec![
        tube_rule(0, TORPEDO_LAUNCH_CHANNEL, TORPEDO_LAUNCH_VERB),
        tube_rule(0, TORPEDO_LAUNCH_CHANNEL, TORPEDO_LAUNCH_VERB),
    ]);
    let err = validate_fine_system_ai_policy(&tie, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS)
        .unwrap_err();
    assert!(
        err.contains(&format!("channel '{TORPEDO_LAUNCH_CHANNEL}'")),
        "must name the contested channel: {err}"
    );
    assert!(
        err.contains("same priority 0"),
        "must name the duplicated priority: {err}"
    );
    assert!(
        err.contains(&format!("verb '{TORPEDO_LAUNCH_VERB}'")),
        "must name the competing verbs: {err}"
    );

    // Distinct priorities on the same channel are the fix...
    let ok = stateless(vec![
        tube_rule(1, TORPEDO_LAUNCH_CHANNEL, TORPEDO_LAUNCH_VERB),
        tube_rule(0, TORPEDO_LAUNCH_CHANNEL, TORPEDO_LAUNCH_VERB),
    ]);
    assert!(validate_fine_system_ai_policy(&ok, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS).is_ok());

    // ...and the SAME priority on DIFFERENT channels was never a tie: those
    // rules do not compete. This is the shipped default tube policy
    // verbatim — a load rule and a launch rule, both at priority 0 — so a
    // check scoped to priority alone would have broken every tube on every
    // hull that authors no inline block.
    let default = crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_tube");
    assert_eq!(default.rule.len(), 2);
    assert_eq!(default.rule[0].priority, default.rule[1].priority);
    assert_ne!(default.rule[0].channel, default.rule[1].channel);
    assert!(
        validate_fine_system_ai_policy(&default, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS).is_ok()
    );
}

/// The same rule tie, inside a STATE. A machine resolves its channels
/// per-state, so the competing set is the current state's rule list — and
/// two states each answering the same channel at priority 0 is ordinary
/// content, not a tie.
#[test]
fn equal_priority_rules_inside_one_state_are_rejected() {
    let boost_rule = |priority: i32| FineSystemAiRuleToml {
        priority,
        channel: HELM_BOOST_CHANNEL.to_string(),
        when: "true".to_string(),
        verb: HELM_ENGAGE_BOOST_VERB.to_string(),
        value: false,
        level: 0,
        response_index: 0,
    };
    let tie = stateful_cfg(
        Some("surge"),
        vec![FineSystemAiStateToml {
            id: "surge".to_string(),
            rule: vec![boost_rule(4), boost_rule(4)],
            transition: Vec::new(),
            ..Default::default()
        }],
    );
    let err = validate_fine_system_ai_policy(&tie, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(err.contains("state 'surge'"), "must name the state: {err}");
    assert!(
        err.contains(&format!("channel '{HELM_BOOST_CHANNEL}'")) && err.contains("same priority 4"),
        "must name the channel and the priority: {err}"
    );

    // One rule per state at the same priority is not a tie.
    let ok = stateful_cfg(
        Some("cruise"),
        vec![
            FineSystemAiStateToml {
                id: "cruise".to_string(),
                rule: vec![boost_rule(0)],
                transition: vec![transition(0, "surge")],
                ..Default::default()
            },
            FineSystemAiStateToml {
                id: "surge".to_string(),
                rule: vec![boost_rule(0)],
                transition: Vec::new(),
                ..Default::default()
            },
        ],
    );
    assert!(validate_fine_system_ai_policy(&ok, BOOST_CHANNELS, BOOST_VERBS).is_ok());
}

/// AC6: a `memory(...)` reference in a STATELESS policy. Private memory has
/// no owner without a state machine, and reading a silent `false` would be
/// a trap rather than a diagnostic.
#[test]
fn memory_reference_in_a_stateless_policy_is_rejected() {
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_BOOST_CHANNEL.to_string(),
            when: "memory(engagements) > 0".to_string(),
            verb: HELM_ENGAGE_BOOST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(
        err.contains("memory") && err.contains("no states"),
        "got: {err}"
    );
}

/// AC6: a `state_time` reference in a STATELESS policy — the same defect on
/// the other private atom.
#[test]
fn state_time_reference_in_a_stateless_policy_is_rejected() {
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_BOOST_CHANNEL.to_string(),
            when: "state_time > 5".to_string(),
            verb: HELM_ENGAGE_BOOST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(
        err.contains("state_time") && err.contains("no states"),
        "got: {err}"
    );
}

// ── Authored history operators (issue #890) ─────────────────────────────

/// A `history(...)` guard in a STATELESS policy — the same defect on the
/// third private atom. The window is per-fine-system retained state that
/// the state-machine host advances; a policy with no machine is never
/// ticked, so nothing would ever fill it.
#[test]
fn history_reference_in_a_stateless_policy_is_rejected() {
    let mut cfg = FineSystemAiConfigToml {
        rule: vec![FineSystemAiRuleToml {
            priority: 0,
            channel: HELM_BOOST_CHANNEL.to_string(),
            when: "history(min, hazard_urgency, param(window_ticks)) >= 1".to_string(),
            verb: HELM_ENGAGE_BOOST_VERB.to_string(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        ..Default::default()
    };
    cfg.param.insert("window_ticks".to_string(), 8.0);
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(
        err.contains("history(min, hazard_urgency, param(window_ticks))")
            && err.contains("no states"),
        "got: {err}"
    );
}

/// The window length is a `param(...)` like any other reference, and an
/// undeclared one is rejected — the author never has to guess whether a
/// typo silently disabled the operator.
#[test]
fn an_undeclared_history_window_param_is_rejected() {
    let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
    cfg.state[0].transition = vec![FineSystemAiTransitionToml {
        priority: 0,
        to: "cruise".to_string(),
        when: "history(min, hazard_urgency, param(never_declared)) >= 1".to_string(),
    }];
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(
        err.contains("undeclared parameter") && err.contains("never_declared"),
        "got: {err}"
    );
}

/// The half of the malformed-window check the parser cannot make: only the
/// hull knows what its parameter is worth. A zero-length window retains
/// nothing and is never full, so it would disable the guard in silence.
#[test]
fn a_non_integral_or_zero_history_window_param_is_rejected() {
    for (value, needle) in [(8.5_f32, "8.5"), (0.0, "0"), (-3.0, "-3")] {
        let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
        cfg.param.insert("window_ticks".to_string(), value);
        cfg.state[0].transition = vec![FineSystemAiTransitionToml {
            priority: 0,
            to: "cruise".to_string(),
            when: "history(min, hazard_urgency, param(window_ticks)) >= 1".to_string(),
        }];
        let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
        assert!(
            err.contains("positive whole number") && err.contains(needle),
            "window length {value} must be rejected naming the value; got: {err}"
        );
    }
}

/// An authored window of a positive whole number of ticks is accepted, in
/// every position a stateful policy can carry a guard.
#[test]
fn an_authored_history_window_validates_in_rules_and_transitions() {
    let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
    cfg.param.insert("window_ticks".to_string(), 30.0);
    cfg.state[0].rule = vec![FineSystemAiRuleToml {
        priority: 0,
        channel: HELM_BOOST_CHANNEL.to_string(),
        when: "history(net_change, hazard_urgency, param(window_ticks)) > 0".to_string(),
        verb: HELM_ENGAGE_BOOST_VERB.to_string(),
        value: false,
        level: 0,
        response_index: 0,
    }];
    cfg.state[0].transition = vec![FineSystemAiTransitionToml {
        priority: 0,
        to: "cruise".to_string(),
        when: "history(min, hazard_urgency, param(window_ticks)) >= 1".to_string(),
    }];
    validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS)
        .expect("an authored positive whole window is valid content");
}

/// A target selector has no history bag on ANY host — it is evaluated per
/// candidate against a snapshot — so the rejection needs no host to answer
/// and fires on the host-less path too.
#[test]
fn a_history_reference_in_a_target_selector_is_rejected() {
    let cfg: FineSystemAiSelectorToml = toml::from_str(
        r#"
            horizon = 100.0
            switch_margin = 0.0
            eligibility = "candidate_fact(detectable) > 0 and history(min, range_to_target, 30) > 0"
            "#,
    )
    .expect("fixture selector parses");
    let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
    assert!(
        err.contains("history(min, range_to_target, 30)") && err.contains("no history bag"),
        "got: {err}"
    );
}

/// An undeclared `memory(...)` slot is rejected in a STATEFUL policy too —
/// the same contract `param(...)` has carried since #775.
#[test]
fn undeclared_memory_slot_is_rejected_in_a_stateful_policy() {
    let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
    cfg.state[0].transition = vec![FineSystemAiTransitionToml {
        priority: 0,
        to: "cruise".to_string(),
        when: "memory(never_declared) > 0".to_string(),
    }];
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(err.contains("undeclared memory"), "got: {err}");
}

/// The existing per-rule channel/verb/param checks run unchanged over each
/// STATE's rules, not just the top-level list.
#[test]
fn per_state_rules_get_the_same_channel_and_verb_checks() {
    let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
    cfg.state[0].rule = vec![FineSystemAiRuleToml {
        priority: 0,
        channel: "not_a_channel".to_string(),
        when: "true".to_string(),
        verb: HELM_ENGAGE_BOOST_VERB.to_string(),
        value: false,
        level: 0,
        response_index: 0,
    }];
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(
        err.contains("unknown channel") && err.contains("state 'cruise' rule 0"),
        "got: {err}"
    );
}

/// `idle = true` alongside states is as contradictory as `idle` alongside
/// rules.
#[test]
fn idle_alongside_states_is_rejected() {
    let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
    cfg.idle = true;
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(err.contains("idle") && err.contains("states"), "got: {err}");
}

/// Issue #883, carried forward from the #882 review: a policy declaring BOTH
/// top-level rules and states is rejected outright.
///
/// Before this, the shape validated and then quietly did nothing useful: a
/// machine resolves exclusively through `resolve_channel_in_state`, so the
/// top-level rules were dead code that looked live. Worse, `stateful` is
/// computed from the presence of states, so a `memory(...)` reference in one
/// of those dead top-level rules PASSED validation and then evaluated false
/// for ever (the stateless scan hands `best_in` an empty memory bag). Two
/// silent failures in one shape — the same class as #882's blocking bug.
#[test]
fn a_policy_with_both_top_level_rules_and_states_is_rejected() {
    let mut cfg = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
    cfg.rule = vec![FineSystemAiRuleToml {
        priority: 0,
        channel: HELM_BOOST_CHANNEL.to_string(),
        when: "true".to_string(),
        verb: HELM_ENGAGE_BOOST_VERB.to_string(),
        value: false,
        level: 0,
        response_index: 0,
    }];
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(
        err.contains("both top-level rules") && err.contains("states"),
        "got: {err}"
    );
}

/// The rejection above must not catch either honest shape: a purely
/// stateless policy and a purely stateful one both still validate. (Every
/// shipped default is the former; the destroyer's three policies are the
/// latter.)
#[test]
fn rule_xor_state_leaves_both_honest_shapes_valid() {
    let stateless = crate::entities::authored_ai_pins::shipped_policy_toml("boost");
    assert!(validate_fine_system_ai_policy(&stateless, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    let stateful = stateful_cfg(Some("cruise"), vec![boost_state("cruise", &[])]);
    assert!(stateful.rule.is_empty(), "the fixture must be state-only");
    assert!(validate_fine_system_ai_policy(&stateful, BOOST_CHANNELS, BOOST_VERBS).is_ok());
}

// ── The Harrow Destroyer hull (issue #883) ───────────────────────────────

/// AC4, both halves. Forward blasters are PRESENT and correctly forward
/// (narrow arc dead ahead); torpedoes are ABSENT — no magazine, no tubes,
/// and no torpedo system entry.
///
/// The absence is asserted explicitly because it is content that is very
/// easy to "helpfully" restore: every other armed NPC hull in the set has a
/// `[torpedoes]` block, so copying one as a starting point re-adds it
/// silently, and nothing else in the suite would notice.
#[test]
fn harrow_destroyer_carries_forward_blasters_and_no_torpedoes() {
    let cfg = EntityConfig::from_toml(&harrow_destroyer_toml())
        .expect("the destroyer hull must pass content validation");

    let wc = cfg
        .weapons_console
        .as_ref()
        .expect("the hull declares [weapons_console]");
    assert!(
        !wc.blaster_banks.is_empty(),
        "the destroyer's whole armament is its forward blasters"
    );
    for bank in &wc.blaster_banks {
        assert_eq!(
            bank.facing_deg, 0.0,
            "bank '{}' must face dead ahead: a fly-through fires off the bow",
            bank.id
        );
        assert!(
            bank.fire_arc_deg > 0.0 && bank.fire_arc_deg <= 90.0,
            "bank '{}' must be a NARROW forward arc, got {}",
            bank.id,
            bank.fire_arc_deg
        );
    }
    assert!(
        wc.phaser_banks.is_empty(),
        "the destroyer is blaster-armed only"
    );

    // The absence assertion (AC4).
    assert!(
        cfg.torpedoes.is_none(),
        "the destroyer must carry NO torpedo magazine"
    );
    let ship_config = cfg
        .ship_config
        .as_ref()
        .expect("the hull declares [[system]] blocks");
    for system in &ship_config.systems {
        assert!(
            !system.kind.contains("torpedo"),
            "the destroyer must declare no torpedo system, found '{:?}' ({})",
            system.id,
            system.kind
        );
    }
}

/// The doctrine itself: all three travel axes author a STATEFUL policy, the
/// two yaw mode verbs are both used, and boost is authored — the block
/// without which `ai_helm_boost` returns before it does anything and the
/// escape leg silently loses its back half.
#[test]
fn harrow_destroyer_authors_the_fly_through_machine_on_all_three_axes() {
    let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
    let hc = cfg
        .helm_console
        .as_ref()
        .expect("the hull declares [helm_console]");
    assert!(
        hc.boost.is_some(),
        "[helm_console.boost] is mandatory: without it the spawner inserts a \
             DISABLED BoostConfigResource and ai_helm_boost stands down"
    );

    for (name, ai) in [
        ("engines_ai", hc.engines_ai.as_ref()),
        ("steering_ai", hc.steering_ai.as_ref()),
        ("boost_ai", hc.boost_ai.as_ref()),
    ] {
        let ai = ai.unwrap_or_else(|| panic!("{name} must be authored"));
        assert!(
            ai.rule.is_empty(),
            "{name} must be state-only (rule XOR state)"
        );
        let ids: Vec<&str> = ai.state.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "shadow",
                "acquire",
                "inbound",
                "escape",
                "recover",
                "reenter",
                "pressed_pivot",
                "pressed_pass",
            ],
            "{name} resolves to the class pass + recovery + pressed machine \
                 (issues #789, #878)"
        );
        // `shadow` and `initial_state = "shadow"` arrive with the class
        // doctrine (issue #878): the shared fragment RESTS defensive and a
        // hull unlocks the aggressive half by posture. This hull authors
        // `press_posture = 0.0`, the lowest rung, so the gate is open on the
        // first tick and the defensive leg is left immediately and never
        // re-entered.
        assert_eq!(ai.initial_state.as_deref(), Some("shadow"));
        let policy = ai.to_policy().expect("must decode");
        assert!(
            policy.machine().is_some(),
            "{name} must decode to a machine"
        );
    }

    // The yaw channel carries ALL FOUR mode verbs, and which one wins is the
    // whole doctrine: tracking while inbound, frozen heading on the escape,
    // a ring while recovering, a cut-thrust pivot on the way back in.
    let steering = hc.steering_ai.as_ref().unwrap();
    let verbs: Vec<&str> = steering
        .state
        .iter()
        .flat_map(|s| s.rule.iter())
        .map(|r| r.verb.as_str())
        .collect();
    assert!(verbs.contains(&HELM_ACTUATE_DESIRED_FACING_VERB));
    assert!(verbs.contains(&HELM_HOLD_COMMITTED_HEADING_VERB));
    assert!(verbs.contains(&HELM_HOLD_RECOVERY_ORBIT_VERB));
    assert!(verbs.contains(&HELM_PIVOT_TO_REENGAGE_VERB));

    // Issue #788, AC7 / issue #789, AC4: none of the recovery states, and
    // not the pressed PASS, authors a boost rule. The absence is the
    // doctrine — a pass flown with the drive lit is not the "normal-speed
    // pass" the hull is supposed to be making — and an absence is exactly
    // the kind of content that gets helpfully filled in.
    let boost = hc.boost_ai.as_ref().unwrap();
    for id in ["recover", "reenter", "pressed_pass"] {
        let state = boost
            .state
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("boost_ai must declare '{id}'"));
        assert!(
            state.rule.is_empty(),
            "boost_ai '{id}' must author NO rule: boost is cancelled before the pass"
        );
    }
}

/// Issue #789, AC4, as content: the pressed PIVOT is the one state outside
/// the escape that lights the drive, and the hull's boost genuinely
/// *increases* turn authority rather than trading it away.
///
/// The second half is load-bearing and is not obvious from the doctrine
/// alone: `apply_ship_physics` multiplies `max_yaw_rate` by
/// `steering_multiplier`, so a hull authoring a value below 1.0 would boost
/// its pivot into turning SLOWER. Nothing in the state machine can detect
/// that; only this pin can.
#[test]
fn harrow_destroyer_boosts_the_pressed_pivot_with_a_drive_that_turns_harder() {
    let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
    let hc = cfg.helm_console.as_ref().unwrap();

    let pivot = hc
        .boost_ai
        .as_ref()
        .unwrap()
        .state
        .iter()
        .find(|s| s.id == "pressed_pivot")
        .expect("boost_ai must declare 'pressed_pivot'");
    assert_eq!(
        pivot.rule.len(),
        1,
        "the pressed pivot lights the drive with exactly one rule"
    );
    assert_eq!(pivot.rule[0].verb, HELM_ENGAGE_BOOST_VERB);
    assert!(
        !pivot.rule[0].when.contains("speed_fraction"),
        "the pivot is flown at ZERO throttle, so a minimum-speed guard would \
             refuse the one case this state exists for; got `{}`",
        pivot.rule[0].when
    );

    let boost = hc
        .boost
        .as_ref()
        .expect("[helm_console.boost] is mandatory");
    assert!(
        boost.steering_multiplier > 1.0,
        "boost must INCREASE the pivot's turn authority — physics multiplies \
             max_yaw_rate by this — got {}",
        boost.steering_multiplier
    );
}

/// Issue #789, AC1/AC3/AC5, as content, on all three machines.
///
/// Each conjunct here is doing distinct work and each has a distinct failure
/// mode if dropped, so they are asserted individually rather than as one
/// string match:
///
/// * the two pressed facts — drop either and the branch fires on a ship that
///   is escaping cleanly, or one that is nowhere near the target's guns;
/// * the shield conjunct — drop it and a destroyer with its shields UP
///   abandons the ordinary pass cycle to jab at a range it never needed to
///   leave;
/// * a higher priority than the recovery branch — equal or lower and the
///   pressed branch is unreachable, silently, because `recover`'s guard is a
///   strict subset of it.
#[test]
fn harrow_destroyer_presses_on_failed_progress_inside_the_targets_reach() {
    let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
    let hc = cfg.helm_console.as_ref().unwrap();
    for (name, ai) in [
        ("engines_ai", hc.engines_ai.as_ref().unwrap()),
        ("steering_ai", hc.steering_ai.as_ref().unwrap()),
        ("boost_ai", hc.boost_ai.as_ref().unwrap()),
    ] {
        let escape = ai.state.iter().find(|s| s.id == "escape").unwrap();
        let pressed = escape
            .transition
            .iter()
            .find(|t| t.to == "pressed_pivot")
            .unwrap_or_else(|| panic!("{name}: the escape must be able to reach the pressed arm"));
        for required in [
            crate::ship::helm_ai::SEPARATION_PROGRESS_FACT,
            crate::ship::helm_ai::INSIDE_THREAT_RANGE_FACT,
            crate::ship::helm_ai::PRESSED_MIN_PROGRESS_PARAM,
            crate::ship::helm_ai::SHIELD_FRACTION_FACT,
        ] {
            assert!(
                pressed.when.contains(required),
                "{name}: the pressed guard must reference `{required}`, got `{}`",
                pressed.when
            );
        }
        let recover = escape
            .transition
            .iter()
            .find(|t| t.to == "recover")
            .unwrap_or_else(|| panic!("{name}: the escape must still reach recovery"));
        assert!(
            pressed.priority > recover.priority,
            "{name}: the pressed branch must outrank recovery ({} vs {}) or it can \
                 never fire — recovery's guard is a subset of it",
            pressed.priority,
            recover.priority
        );

        // AC3: the way OUT of the pressed loop is the ordinary escape, and
        // neither pressed state waits on a shield threshold or a held
        // distance the way recovery does.
        for id in ["pressed_pivot", "pressed_pass"] {
            let state = ai
                .state
                .iter()
                .find(|s| s.id == id)
                .unwrap_or_else(|| panic!("{name} must declare '{id}'"));
            for transition in &state.transition {
                assert!(
                    !transition
                        .when
                        .contains(crate::ship::helm_ai::SAFE_DISTANCE_HELD_FACT)
                        && !transition.when.contains("reentry_shield_fraction"),
                    "{name} '{id}': pressed behaviour abandons the shield threshold and \
                         the standoff ring — it may not wait on either, got `{}`",
                    transition.when
                );
            }
        }
        assert!(
            ai.state
                .iter()
                .find(|s| s.id == "pressed_pass")
                .unwrap()
                .transition
                .iter()
                .any(|t| t.to == "escape"),
            "{name}: every short pass must end in another real escape attempt"
        );
    }
}

/// Issue #789: the SHORT pass is short because of an authored scalar, and it
/// is a different scalar from the ordinary pass's.
///
/// Authoring the same number twice would make this arm indistinguishable
/// from a re-run of the ordinary inbound leg while still passing every
/// structural assertion above.
#[test]
fn harrow_destroyer_breaks_off_the_pressed_pass_sooner_than_an_ordinary_one() {
    let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
    let steering = cfg
        .helm_console
        .as_ref()
        .unwrap()
        .steering_ai
        .as_ref()
        .unwrap();
    for required in crate::ship::helm_ai::PRESSED_PARAMS {
        assert!(
            steering.param.contains_key(*required),
            "steering_ai must author `{required}`: the host gates the whole pressed \
                 arm on all four together"
        );
    }
    let pressed = steering
        .param
        .get(crate::ship::helm_ai::PRESSED_HYSTERESIS_PARAM)
        .copied()
        .unwrap();
    let ordinary = steering
        .param
        .get("closest_approach_hysteresis")
        .copied()
        .unwrap();
    assert!(
        pressed > 0.0 && pressed < ordinary,
        "the pressed pass must break off sooner than an ordinary one \
             ({pressed} vs {ordinary}) — equal values make it the same pass"
    );
    // ...and the two history windows are independently authored lengths.
    let pressed_window = steering
        .param
        .get(crate::ship::helm_ai::PRESSED_WINDOW_TICKS_PARAM)
        .copied()
        .unwrap();
    assert!(
        pressed_window > 1.0 && pressed_window.is_finite(),
        "the progress window must be a real, finite bound, got {pressed_window}"
    );
}

/// Issue #788, AC6: re-entry is gated on BOTH the shield fraction and the
/// held distance, on every axis. Dropping either conjunct from any of the
/// three machines would let one axis re-enter early and desynchronise the
/// hull from itself — and would do it silently, because each machine runs
/// its own copy.
#[test]
fn harrow_destroyer_reentry_requires_both_shields_and_held_distance() {
    let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
    let hc = cfg.helm_console.as_ref().unwrap();
    for (name, ai) in [
        ("engines_ai", hc.engines_ai.as_ref().unwrap()),
        ("steering_ai", hc.steering_ai.as_ref().unwrap()),
        ("boost_ai", hc.boost_ai.as_ref().unwrap()),
    ] {
        let recover = ai
            .state
            .iter()
            .find(|s| s.id == "recover")
            .unwrap_or_else(|| panic!("{name} must declare 'recover'"));
        assert_eq!(
            recover.transition.len(),
            2,
            "{name}: recovery has exactly two ways out — re-entry, and the \
                 class doctrine's posture break-off (issue #878), which \
                 `press_posture = 0.0` makes unreachable on this hull"
        );
        let reenter_exit = recover
            .transition
            .iter()
            .find(|t| t.to == "reenter")
            .unwrap_or_else(|| panic!("{name}: recovery must reach re-entry"));
        let guard = &reenter_exit.when;
        assert!(
            guard.contains(crate::ship::helm_ai::SHIELD_FRACTION_FACT)
                && guard.contains("reentry_shield_fraction"),
            "{name}: re-entry must require the authored shield fraction, got `{guard}`"
        );
        assert!(
            guard.contains(crate::ship::helm_ai::SAFE_DISTANCE_HELD_FACT),
            "{name}: re-entry must require the HELD safe distance, got `{guard}`"
        );
        // ...and the escape must be able to reach recovery at all.
        let escape = ai.state.iter().find(|s| s.id == "escape").unwrap();
        assert!(
            escape.transition.iter().any(|t| t.to == "recover"),
            "{name}: the escape must hand off to recovery when the shields are gone"
        );
    }
}

/// Issue #788, AC2/AC3/AC5: every scalar the recovery manoeuvre needs is an
/// authored `param` on the Steering axis, found by the host BY NAME. A
/// rename in either direction lights this up — and it must, because the
/// host's response to a missing one is to decline the recovery arm and
/// quietly fly ordinary doctrine travel instead.
#[test]
fn harrow_destroyer_authors_every_recovery_scalar_as_a_steering_param() {
    let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
    let steering = cfg
        .helm_console
        .as_ref()
        .unwrap()
        .steering_ai
        .as_ref()
        .unwrap();
    for required in [
        crate::ship::helm_ai::SAFE_RANGE_MARGIN_PARAM,
        crate::ship::helm_ai::ORBIT_SPEED_PARAM,
        crate::ship::helm_ai::ORBIT_SPIRAL_GAIN_PARAM,
        crate::ship::helm_ai::SAFE_RING_TOLERANCE_PARAM,
        crate::ship::helm_ai::SAFE_DISTANCE_WINDOW_TICKS_PARAM,
        crate::ship::helm_ai::REENGAGE_SPEED_PARAM,
    ] {
        assert!(
            steering.param.contains_key(required),
            "steering_ai must author `{required}`"
        );
    }
    // AC7: the pivot is flown on CUT thrust, and that is authored, not
    // hardcoded anywhere in Rust.
    assert_eq!(
        steering
            .param
            .get(crate::ship::helm_ai::REENGAGE_SPEED_PARAM),
        Some(&0.0),
        "the re-entry pivot must cut thrust"
    );
    // AC6: the authored re-entry fraction is the issue's stated 75%.
    assert_eq!(
        steering.param.get("reentry_shield_fraction"),
        Some(&0.75),
        "the authored re-entry shield fraction is 75%"
    );
    // AC5: the distance history is BOUNDED, and its bound is authored.
    let window = steering
        .param
        .get(crate::ship::helm_ai::SAFE_DISTANCE_WINDOW_TICKS_PARAM)
        .copied()
        .unwrap();
    assert!(
        window > 1.0 && window.is_finite(),
        "the window must be a real, finite bound, got {window}"
    );
}

/// AC6: every manoeuvre threshold the doctrine flies by is an authored
/// `param`, and the host-side pass surface can find the four it reads by
/// name. A rename in either direction lights this up — which matters,
/// because the host's response to a missing param is to decline the pass
/// entirely and quietly fall back to ordinary doctrine travel.
#[test]
fn harrow_destroyer_authors_every_manoeuvre_threshold_as_a_param() {
    let cfg = EntityConfig::from_toml(&harrow_destroyer_toml()).expect("hull must parse");
    let hc = cfg.helm_console.as_ref().unwrap();
    let engines = hc.engines_ai.as_ref().unwrap();
    let steering = hc.steering_ai.as_ref().unwrap();
    let boost = hc.boost_ai.as_ref().unwrap();

    for required in [
        crate::ship::helm_ai::APPROACH_SPEED_PARAM,
        crate::ship::helm_ai::ESCAPE_SPEED_PARAM,
    ] {
        assert!(
            engines.param.contains_key(required),
            "engines_ai must author `{required}`"
        );
    }
    for required in [
        crate::ship::helm_ai::TRACKING_DEADBAND_PARAM,
        crate::ship::helm_ai::TRACKING_FULL_STEER_PARAM,
    ] {
        assert!(
            steering.param.contains_key(required),
            "steering_ai must author `{required}`"
        );
    }
    // Shared manoeuvre thresholds — every axis's guards reference them, so
    // every axis declares them (validation rejects an undeclared reference).
    for ai in [engines, steering, boost] {
        for required in [
            "commit_range",
            "closing_rate_epsilon",
            "closest_approach_hysteresis",
            "escape_duration_secs",
        ] {
            assert!(ai.param.contains_key(required), "must author `{required}`");
        }
        assert!(
            ai.memory
                .contains_key(crate::ship::helm_ai::MIN_RANGE_SEEN_MEMORY),
            "the closest-approach detector's running minimum must be declared"
        );
    }
    assert!(boost.param.contains_key("escape_boost_secs"));
}

// ── The Harrow Cruiser hull (issue #790) ─────────────────────────────────

/// AC4, as content: the two banks are on the CENTRELINE, one forward and one
/// aft, and each sweeps 270 degrees.
///
/// Every number here is load-bearing and each has its own silent failure
/// mode. Facings of ±90 (port/starboard, the shape every other beam cruiser
/// in the set uses) would give a hull whose banks cover one side each and
/// never overlap. An arc of 180 would give centreline banks with no overlap
/// either — they would meet exactly on the beam line and cover nothing
/// twice. 270 is the smallest arc for which two opposed banks overlap on
/// BOTH beams, which is the entire premise of the doctrine.
///
/// The overlap is asserted through the shared `in_arc` predicate rather than
/// by arithmetic on the authored numbers, so this pins the behaviour the
/// firing paths actually see.
#[test]
fn harrow_cruiser_carries_overlapping_fore_and_aft_270_degree_phaser_banks() {
    let cfg = EntityConfig::from_toml(&harrow_cruiser_toml())
        .expect("the cruiser hull must pass content validation");
    let wc = cfg
        .weapons_console
        .as_ref()
        .expect("the hull declares [weapons_console]");

    let ids: Vec<&str> = wc.phaser_banks.iter().map(|b| b.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["fore", "aft"],
        "the cruiser mounts exactly one forward and one aft beam bank"
    );
    for (id, facing) in [("fore", 0.0), ("aft", 180.0)] {
        let bank = wc.phaser_banks.iter().find(|b| b.id == id).unwrap();
        assert_eq!(
            bank.facing_deg, facing,
            "bank '{id}' must sit on the centreline facing {facing}"
        );
        assert_eq!(
            bank.fire_arc_deg, 270.0,
            "bank '{id}' must sweep 270 degrees — anything narrower removes the \
                 broadside overlap the whole doctrine is built on"
        );
        assert_eq!(
            bank.auto_arc_deg, 270.0,
            "bank '{id}': the AI fires on the same arc it may fire on. A narrower \
                 auto arc would switch off exactly the abeam overlap this hull exists for"
        );
    }

    // The overlap itself, through the shared predicate: a target directly
    // off either beam is inside BOTH banks' arcs, and a target dead astern
    // is outside the fore bank's (so the arcs are genuinely 270 and not 360).
    for (label, rx, ry) in [
        ("starboard beam", 10.0_f32, 0.0_f32),
        ("port beam", -10.0, 0.0),
    ] {
        for bank in &wc.phaser_banks {
            assert!(
                crate::weapons::phaser::in_arc(rx, ry, bank.facing_deg, bank.fire_arc_deg),
                "a target on the {label} must bear for bank '{}'",
                bank.id
            );
        }
    }
    // ...and each bank still has a blind wedge opposite its own facing, so
    // the arcs are genuinely 270 and not 360. A bank that covers everything
    // leaves the orbit nothing to solve.
    // Ship-local bearing is `radar_x.atan2(radar_y)`, so `(0, +r)` is dead
    // ahead and `(0, -r)` dead astern.
    let fore = wc.phaser_banks.iter().find(|b| b.id == "fore").unwrap();
    let aft = wc.phaser_banks.iter().find(|b| b.id == "aft").unwrap();
    assert!(
        !crate::weapons::phaser::in_arc(0.0, -10.0, fore.facing_deg, fore.fire_arc_deg),
        "the fore bank must be blind dead astern"
    );
    assert!(
        !crate::weapons::phaser::in_arc(0.0, 10.0, aft.facing_deg, aft.fire_arc_deg),
        "the aft bank must be blind dead ahead"
    );

    // The deliberate absence that survives: no blasters. The beams are still
    // the only continuous weapon on the hull.
    assert!(
        wc.blaster_banks.is_empty(),
        "the cruiser is beam-armed only between torpedo opportunities"
    );
    let ship_config = cfg
        .ship_config
        .as_ref()
        .expect("the hull declares [[system]] blocks");
    // Every bank needs its own fine system or it is never AI-operable, and
    // the id follows the `phaser-<bank_id>` convention the resolver uses.
    for bank in &wc.phaser_banks {
        let expected = crate::ship::system_registry::phaser_bank_system_id(&bank.id)
            .expect("a non-empty bank id always resolves");
        assert!(
            ship_config.systems.iter().any(|s| s.id == expected),
            "bank '{}' must declare a [[system]] entry `{}` — without it the bank \
                 is never registered as AI-operable and the hull never fires",
            bank.id,
            expected.0
        );
    }
}

/// AC2, as content — and the INVERSION of a #790 pin.
///
/// #790 asserted this hull carried no `[torpedoes]` table and no torpedo
/// `[[system]]` at all, because a ship that never presents its bow has
/// nothing to launch a fixed forward tube at. #791 changes that premise: the
/// cruiser now breaks its orbit to point at a shield gap, so the pin is
/// replaced rather than deleted, and the replacement is at least as specific.
/// Every number below has its own silent failure mode:
///
/// * fewer than two tubes and `fact(tubes_full)` degenerates to "this tube is
///   full" — the salvo doctrine would still parse and would pin nothing;
/// * a tube facing anywhere but dead ahead, or a wide arc, and the ORBIT
///   already satisfies `in_arc` — the whole bow-on phase becomes decoration
///   the hull never needs;
/// * a missing `[[system]]` entry and the tube is never AI-operable, so the
///   salvo can never be full and the phase never launches.
#[test]
fn harrow_cruiser_carries_two_narrow_bow_tubes_for_the_shield_opportunity() {
    let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
    let torpedoes = cfg
        .torpedoes
        .as_ref()
        .expect("the cruiser carries a torpedo magazine for the shield opportunity");

    let ids: Vec<&str> = torpedoes.tubes.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["bow_port", "bow_starboard"],
        "two forward tubes: with one, `tubes_full` says nothing a per-tube \
             `loaded` reading does not already say"
    );
    for tube in &torpedoes.tubes {
        assert_eq!(
            tube.facing_deg, 0.0,
            "tube '{}' must be a FIXED forward tube — the phase exists because \
                 the guns cannot be pointed without pointing the ship",
            tube.id
        );
        assert!(
            tube.fire_arc_deg > 0.0 && tube.fire_arc_deg <= 30.0,
            "tube '{}' must have a NARROW bow arc ({} deg): the orbit holds the \
                 target abeam, so an arc wide enough to cover the beam would let the \
                 cruiser launch without ever breaking off",
            tube.id,
            tube.fire_arc_deg
        );
        assert!(
            tube.volley_max > 1,
            "tube '{}' fires a salvo, not a round",
            tube.id
        );
        assert_eq!(
            tube.ai_target_count,
            Some(tube.volley_max),
            "an AI crew keeps tube '{}' at its full volley between \
                 opportunities — the load time is longer than the window",
            tube.id
        );
    }

    // The tubes are barely-homing hull-killers aimed by the bow, not a way
    // through a shield: a round that arrives after the arc recovers must do
    // nothing, which is what makes the abort transition matter.
    assert_eq!(
        torpedoes.damage_shields, 0,
        "these rounds go through the hole the beams made; they do not make one"
    );
    assert!(
        torpedoes.damage_hull > 0,
        "and they hurt the hull once they are through"
    );
    assert!(
        torpedoes.load_time > torpedoes.lifespan,
        "reloading ({}) must outlast a round's whole flight ({}), or the cruiser \
             could refill inside one opportunity and the doctrine would collapse into \
             holding the bow on and emptying the magazine",
        torpedoes.load_time,
        torpedoes.lifespan
    );

    // The authored per-tube policy — the first in the set. All three of AC2's
    // conditions, on the launch channel, on EVERY tube.
    for tube in &torpedoes.tubes {
        let ai = tube
            .ai
            .as_ref()
            .unwrap_or_else(|| panic!("tube '{}' must author its own policy", tube.id));
        assert!(
            validate_fine_system_ai_policy(ai, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS).is_ok(),
            "tube '{}' policy must pass content validation",
            tube.id
        );
        let load = ai
            .rule
            .iter()
            .find(|r| r.channel == TORPEDO_LOAD_CHANNEL)
            .unwrap_or_else(|| panic!("tube '{}' must author a load rule", tube.id));
        assert_eq!(load.verb, TORPEDO_LOAD_VERB);
        let launch = ai
            .rule
            .iter()
            .find(|r| r.channel == TORPEDO_LAUNCH_CHANNEL)
            .unwrap_or_else(|| panic!("tube '{}' must author a launch rule", tube.id));
        assert_eq!(launch.verb, TORPEDO_LAUNCH_VERB);
        for required in ["tubes_full", "target_facing_shields", "in_arc"] {
            assert!(
                launch.when.contains(required),
                "tube '{}': the launch guard must require `{required}` continuously, \
                     got `{}`",
                tube.id,
                launch.when
            );
        }
    }

    // Fine systems: one per tube plus the shared magazine. Both loaders gate
    // on the magazine before they look at a tube, so its absence would
    // silently switch the whole armament off.
    let ship_config = cfg.ship_config.as_ref().expect("hull declares systems");
    let declared =
        |id: &crate::core::messages::SystemId| ship_config.systems.iter().any(|s| &s.id == id);
    assert!(
        declared(&crate::ship::system_registry::torpedo_magazine_system_id()),
        "the shared magazine needs a [[system]] entry or neither loading nor \
             launching runs at all"
    );
    for tube in &torpedoes.tubes {
        let expected = crate::ship::system_registry::torpedo_tube_system_id(&tube.id)
            .expect("a non-empty tube id always resolves");
        assert!(
            declared(&expected),
            "tube '{}' must declare a [[system]] entry `{}`",
            tube.id,
            expected.0
        );
    }
}

/// AC5, as content: the fore phaser bank still bears on a target held dead
/// ahead, so ordinary beam pressure continues through the whole torpedo
/// phase rather than pausing for it.
///
/// This is a geometry claim, not a plumbing one — `ai_phaser_auto_fire` never
/// reads the Steering verb or the pass surface (pinned in the weapons tests)
/// — but the geometry is the half that could silently stop being true: narrow
/// the fore bank's arc and the cruiser would go quiet exactly while it was
/// most exposed.
#[test]
fn harrow_cruiser_fore_bank_still_bears_while_the_bow_is_held_on_the_target() {
    let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
    let wc = cfg.weapons_console.as_ref().unwrap();
    let fore = wc.phaser_banks.iter().find(|b| b.id == "fore").unwrap();
    // Ship-local bearing is `radar_x.atan2(radar_y)`, so `(0, +r)` is dead
    // ahead — where the bow hold puts the target.
    assert!(
        crate::weapons::phaser::in_arc(0.0, 10.0, fore.facing_deg, fore.auto_arc_deg),
        "a target dead ahead must be inside the fore bank's AUTO arc: the beams \
             keep working while the tubes line up"
    );
    // ...and the tubes' own cone sits inside that arc, so there is no bearing
    // at which the torpedoes may launch but the beams may not fire.
    let torpedoes = cfg.torpedoes.as_ref().unwrap();
    for tube in &torpedoes.tubes {
        let half = tube.fire_arc_deg * 0.5;
        for edge in [-half, half] {
            let (x, y) = (
                simmath::sin(edge.to_radians()) * 10.0,
                simmath::cos(edge.to_radians()) * 10.0,
            );
            assert!(
                crate::weapons::phaser::in_arc(x, y, fore.facing_deg, fore.auto_arc_deg),
                "the edge of tube '{}' arc ({edge} deg) must still be inside the \
                     fore bank's auto arc",
                tube.id
            );
        }
    }
}

/// AC1/AC2, as content: both travel axes author the three-state machine
/// (issue #791 adds `torpedo_run` to #790's pair), the yaw channel resolves
/// the combat-orbit verb in the ring and the bow-hold verb in the phase, and
/// every scalar the host reads by name is present on the Steering axis.
#[test]
fn harrow_cruiser_authors_the_broadside_orbit_machine_on_both_travel_axes() {
    let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
    let hc = cfg
        .helm_console
        .as_ref()
        .expect("the hull declares [helm_console]");

    for (name, ai) in [
        ("engines_ai", hc.engines_ai.as_ref()),
        ("steering_ai", hc.steering_ai.as_ref()),
    ] {
        let ai = ai.unwrap_or_else(|| panic!("{name} must be authored"));
        assert!(
            ai.rule.is_empty(),
            "{name} must be state-only (rule XOR state)"
        );
        let ids: Vec<&str> = ai.state.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["shadow", "acquire", "orbit", "torpedo_run"],
            "{name} resolves to the class orbit + shield-opportunity machine"
        );
        // `shadow` and `initial_state = "shadow"` arrive with the class
        // doctrine (issue #878): the shared fragment RESTS defensive and a
        // hull unlocks the aggressive half by posture. This hull authors
        // `press_posture = 0.0`, the lowest rung, so the gate is open on the
        // first tick and the defensive leg is left immediately and never
        // re-entered —
        // `the_harrow_hulls_unlock_their_class_doctrine_by_posture_alone`
        // (`authored_ai_pins.rs`) is what proves that rather than assuming it.
        assert_eq!(ai.initial_state.as_deref(), Some("shadow"));
        assert!(
            ai.to_policy().expect("must decode").machine().is_some(),
            "{name} must decode to a machine"
        );
    }

    // The yaw channel resolves the FIFTH mode verb in the orbit state, and
    // tracks in the approach.
    let steering = hc.steering_ai.as_ref().unwrap();
    let verb_of = |state_id: &str| -> String {
        let state = steering
            .state
            .iter()
            .find(|s| s.id == state_id)
            .unwrap_or_else(|| panic!("steering_ai must declare '{state_id}'"));
        assert_eq!(
            state.rule.len(),
            1,
            "'{state_id}' answers yaw with one rule"
        );
        state.rule[0].verb.clone()
    };
    assert_eq!(verb_of("acquire"), HELM_ACTUATE_DESIRED_FACING_VERB);
    assert_eq!(
        verb_of("orbit"),
        HELM_HOLD_COMBAT_ORBIT_VERB,
        "the orbit leg is the combat-orbit verb — NOT `hold_recovery_orbit`, \
             whose ring is derived from the target's reach and gated on a shield \
             doctrine this hull does not have"
    );
    assert_eq!(
        verb_of("torpedo_run"),
        HELM_HOLD_TORPEDO_BEARING_VERB,
        "the shield-opportunity leg is the SIXTH yaw verb — NOT \
             `pivot_to_reengage`, whose geometry is the same but whose host gate \
             is the six shield-recovery scalars this hull would have to invent"
    );

    // Every scalar the host reads off this axis BY NAME. A rename in either
    // direction lights this up, and it must: the host's response to a
    // missing one is to decline the whole arm and fly ordinary doctrine
    // travel instead.
    for required in crate::ship::helm_ai::COMBAT_ORBIT_PARAMS {
        assert!(
            steering.param.contains_key(*required),
            "steering_ai must author `{required}`: the host gates the whole \
                 combat-orbit arm on all three together"
        );
    }
    for required in crate::ship::helm_ai::TORPEDO_BEARING_PARAMS {
        assert!(
            steering.param.contains_key(*required),
            "steering_ai must author `{required}`: the host gates the whole \
                 bow-hold arm on it, and the value this hull wants (0.0) is \
                 indistinguishable from an omission unless the NAME is present"
        );
    }
    for required in [
        crate::ship::helm_ai::TRACKING_DEADBAND_PARAM,
        crate::ship::helm_ai::TRACKING_FULL_STEER_PARAM,
    ] {
        assert!(
            steering.param.contains_key(required),
            "steering_ai must author `{required}`"
        );
    }
    assert!(
        steering
            .memory
            .contains_key(crate::ship::helm_ai::ORBIT_DIRECTION_MEMORY),
        "the circulation direction slot must be declared so its pre-engagement \
             value is authored rather than implicit"
    );
}

/// Issue #794, as content: the cruiser's `torpedo_run` exits carry
/// DISTINCT priorities on both travel axes.
///
/// The state has four ways out and three of them land in `orbit`, so the
/// two that used to share priority 0 — "the salvo is spent" and "the
/// battery is gone" — were behaviourally interchangeable and read as if the
/// order were arbitrary. It was not arbitrary; it was the file order. The
/// pin is here rather than left to the generic validator test because a
/// re-author that collapsed them back would fail the load with a message
/// about a hull, not about a fixture, and the cruiser is the hull that
/// motivated the rule.
#[test]
fn harrow_cruiser_torpedo_run_exits_carry_distinct_priorities() {
    let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
    let hc = cfg.helm_console.as_ref().unwrap();
    for (name, ai) in [
        ("engines_ai", hc.engines_ai.as_ref().unwrap()),
        ("steering_ai", hc.steering_ai.as_ref().unwrap()),
    ] {
        let run = ai
            .state
            .iter()
            .find(|s| s.id == "torpedo_run")
            .unwrap_or_else(|| panic!("{name} must declare 'torpedo_run'"));
        let mut priorities: Vec<i32> = run.transition.iter().map(|t| t.priority).collect();
        let authored = priorities.len();
        // FIVE since issue #878 composed this hull on the class fragment:
        // the four documented here plus the class doctrine's posture
        // break-off to `shadow`, which `press_posture = 0.0` makes
        // unreachable on this hull.
        assert_eq!(authored, 5, "{name} authors the documented exits");
        priorities.sort_unstable();
        priorities.dedup();
        assert_eq!(
            priorities.len(),
            authored,
            "{name} `torpedo_run` must give every exit its own priority — a tie \
                 resolves by file order, which the file does not say out loud"
        );
        // The re-author is an ORDERING and not a re-aim: all three
        // window-closed exits still land back on the ring.
        let to_orbit = run.transition.iter().filter(|t| t.to == "orbit").count();
        assert_eq!(
            to_orbit, 3,
            "{name} keeps all three window-closed exits pointed at 'orbit'"
        );
    }
}

/// AC2, as content: the authored fighting ring sits INSIDE the banks' own
/// beam range, and the orbit is flown under power.
///
/// This is the assertion that makes the ring a fighting range rather than a
/// standoff. A ring authored at or beyond `beam_range` would produce a
/// cruiser that circles a target it cannot hit — every structural test above
/// would still pass, and the hull would look correct and do nothing.
#[test]
fn harrow_cruiser_orbits_inside_its_own_beam_envelope_and_under_power() {
    let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
    let steering = cfg
        .helm_console
        .as_ref()
        .unwrap()
        .steering_ai
        .as_ref()
        .unwrap();
    let ring = steering
        .param
        .get(crate::ship::helm_ai::COMBAT_ORBIT_RANGE_PARAM)
        .copied()
        .unwrap();
    let shortest_beam = cfg
        .weapons_console
        .as_ref()
        .unwrap()
        .phaser_banks
        .iter()
        .map(|b| b.beam_range)
        .fold(f32::INFINITY, f32::min);
    assert!(
        ring > 0.0 && ring < shortest_beam,
        "the fighting ring ({ring}) must sit inside every bank's beam range \
             ({shortest_beam}) — a ring outside it circles a target it cannot hit"
    );

    let speed = steering
        .param
        .get(crate::ship::helm_ai::COMBAT_ORBIT_SPEED_PARAM)
        .copied()
        .unwrap();
    assert!(
        speed > 0.0 && speed <= 1.0,
        "the ring is flown UNDER POWER: an orbit at zero throttle is a parked \
             ship inside a hostile's guns, got {speed}"
    );
    let gain = steering
        .param
        .get(crate::ship::helm_ai::COMBAT_ORBIT_SPIRAL_GAIN_PARAM)
        .copied()
        .unwrap();
    assert!(
        gain > 0.0 && gain.is_finite(),
        "a zero spiral gain flies the bare tangent and never corrects the \
             radius, got {gain}"
    );
}

/// AC3, as content, and the reason it is asserted at all: NO transition
/// anywhere in this hull's doctrine is guarded on a hazard reading.
///
/// Avoidance composes onto the orbit additively inside the pure planner and
/// through the stateless imminent-collision facing override — both temporary
/// and both outside the state machine. A `fact(hazard_urgency)` transition
/// here would replace that with a manoeuvre the hull has to be talked out
/// of, and re-entering the orbit afterwards would RE-DRAW the circulation
/// direction, so flying past an asteroid would randomise which way the
/// cruiser circles. An absence is exactly the kind of content that gets
/// helpfully filled in.
#[test]
fn harrow_cruiser_never_leaves_the_orbit_for_a_hazard() {
    let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
    let hc = cfg.helm_console.as_ref().unwrap();
    for (name, ai) in [
        ("engines_ai", hc.engines_ai.as_ref().unwrap()),
        ("steering_ai", hc.steering_ai.as_ref().unwrap()),
    ] {
        let orbit = ai.state.iter().find(|s| s.id == "orbit").unwrap();
        // Exactly TWO ways out, both of them named here. #790 pinned one;
        // #791 adds the shield opportunity and re-pins the whole set rather
        // than loosening the count, because "some number of exits" would let
        // a third grow in unnoticed.
        let exits: Vec<&str> = orbit.transition.iter().map(|t| t.to.as_str()).collect();
        assert_eq!(
            exits,
            vec!["shadow", "torpedo_run", "acquire"],
            "{name}: the orbit has exactly three ways out, in this priority order"
        );
        // The `shadow` exit is the class doctrine's posture break-off and is
        // UNREACHABLE on this hull (`press_posture = 0.0`), which is why the
        // hazard claim below is unaffected: it is still true that no exit
        // anywhere is guarded on hazard urgency.
        assert!(
            orbit.transition[1]
                .when
                .contains(crate::ship::helm_ai::TARGET_FACING_SHIELD_DOWN_FACT),
            "{name}: the shield opportunity is what interrupts the orbit, got `{}`",
            orbit.transition[1].when
        );
        // ...and the interruption stays an interruption, which takes BOTH
        // armament readings and not either alone.
        //
        // `tubes_full` is the load-bearing one and it is the LAUNCHER's
        // question: entry is what spends the broadside geometry, so it must
        // ask exactly what the `torpedo_launch` policy asks. Guarding on
        // `tubes_fillable` alone was measured at 506 bow-on ticks against
        // 431 orbiting over a 400 s run, only 29 of them with the tubes
        // actually full — reachability stays true through the whole 18 s
        // reload, so the ring broke on collapses with nothing loadable
        // inside the window.
        //
        // `tubes_fillable` stays beside it because it catches what
        // `tubes_full` cannot: a tube that is loaded but has been shot out,
        // and a magazine that can no longer top the battery up.
        //
        // Both pinned as content because the failure is invisible in any
        // test that fights a single engagement.
        for required in [
            crate::ship::helm_ai::TUBES_FULL_FACT,
            crate::ship::helm_ai::TUBES_FILLABLE_FACT,
        ] {
            assert!(
                orbit.transition[1].when.contains(required),
                "{name}: the orbit may only be given up with `{required}` \
                     satisfied — a salvo loaded, in a battery that can still fire \
                     it, got `{}`",
                orbit.transition[1].when
            );
        }
        assert!(
            orbit.transition[2]
                .when
                .contains(crate::ship::helm_ai::TARGET_VALID_FACT),
            "{name}: losing the target is the other thing that ends the orbit, got `{}`",
            orbit.transition[2].when
        );
        // And the phase resumes the ring THREE ways, which is the whole of
        // the trap fix. Pinned as a set rather than as "at least one exit
        // mentioning the right facts", because it is precisely the ones
        // after the first that are easy to lose and impossible to miss the
        // absence of in a fixture whose target has shields and whose tubes
        // survive the engagement.
        let phase = ai.state.iter().find(|s| s.id == "torpedo_run").unwrap();
        let resumes: Vec<&str> = phase
            .transition
            .iter()
            .filter(|t| t.to == "orbit")
            .map(|t| t.when.as_str())
            .collect();
        assert_eq!(
            resumes.len(),
            3,
            "{name}: the phase must have exactly three ways back to the ring — \
                 the window closing, the salvo being spent and the battery becoming \
                 unusable, got {resumes:?}"
        );

        // THE WINDOW CLOSED. Both conjuncts: the shield being back is not
        // enough while a salvo is still in the air, or the cruiser turns
        // away mid-flight the instant the arc regenerates — which, since it
        // regenerates the whole time the rounds are flying, is nearly always.
        for required in [
            crate::ship::helm_ai::TARGET_FACING_SHIELD_DOWN_FACT,
            crate::ship::helm_ai::TORPEDOES_IN_FLIGHT_FACT,
        ] {
            assert!(
                resumes[0].contains(required),
                "{name}: the window-closed resume must require `{required}`, \
                     got `{}`",
                resumes[0]
            );
        }

        // THE SALVO IS SPENT, and this one may not mention the target's
        // shields AT ALL. `target_facing_shield_down` reads a permanent 1.0
        // against any resolvable target with no `[shields]` block — a
        // station, a probe — so an exit that consulted it would be no exit
        // at all for those targets, and the cruiser would hold its nose on
        // one until something died. The bound has to be the hull's own
        // armament.
        for required in [
            crate::ship::helm_ai::TUBES_FULL_FACT,
            crate::ship::helm_ai::TORPEDOES_IN_FLIGHT_FACT,
        ] {
            assert!(
                resumes[1].contains(required),
                "{name}: the salvo-spent resume must require `{required}`, \
                     got `{}`",
                resumes[1]
            );
        }
        assert!(
            !resumes[1].contains(crate::ship::helm_ai::TARGET_FACING_SHIELD_DOWN_FACT),
            "{name}: the salvo-spent resume must not depend on the target ever \
                 raising a shield — that is the one thing a shieldless target never \
                 does, got `{}`",
            resumes[1]
        );

        // THE BATTERY IS GONE, and this one exists because the guard above
        // cannot see it. `tubes_full` reads the ROUNDS, and a tube that is
        // shot out mid-phase keeps the rounds already in it — so the
        // salvo-spent resume stays shut, `torpedoes_in_flight` is zero, and
        // against a target with no arc to raise the hull is trapped bow-on
        // for a salvo `handle_fire_torpedo` will decline. Reachability is
        // the reading that notices, and it must be on the EXIT and not only
        // on the entry guard.
        for required in [
            crate::ship::helm_ai::TUBES_FILLABLE_FACT,
            crate::ship::helm_ai::TORPEDOES_IN_FLIGHT_FACT,
        ] {
            assert!(
                resumes[2].contains(required),
                "{name}: the battery-lost resume must require `{required}`, \
                     got `{}`",
                resumes[2]
            );
        }
        assert!(
            !resumes[2].contains(crate::ship::helm_ai::TARGET_FACING_SHIELD_DOWN_FACT),
            "{name}: the battery-lost resume must not depend on the target \
                 either, got `{}`",
            resumes[2]
        );
        for state in &ai.state {
            for transition in &state.transition {
                for forbidden in [
                    crate::ship::helm_ai::HAZARD_URGENCY_FACT,
                    "hazard_present",
                    "moving_hazard_threat",
                ] {
                    assert!(
                        !transition.when.contains(forbidden),
                        "{name} '{}': no transition may be guarded on `{forbidden}` — \
                             a detour must bend the orbit, never exit it, got `{}`",
                        state.id,
                        transition.when
                    );
                }
            }
        }
    }
}

/// The deliberate absence of a boost drive (see the hull header). A cruiser
/// that lights the drive on the ring widens it; nothing in the doctrine asks
/// for that, and there is no `[helm_console.boost]` block for it to use.
#[test]
fn harrow_cruiser_authors_no_boost_drive_and_no_boost_doctrine() {
    let cfg = EntityConfig::from_toml(&harrow_cruiser_toml()).expect("hull must parse");
    let hc = cfg.helm_console.as_ref().unwrap();
    assert!(
        hc.boost.is_none(),
        "the cruiser mounts no boost drive: a broadside orbit is flown at a \
             steady authored throttle"
    );
    assert_idle_boost_declaration(
        hc,
        "the cruiser: no boost doctrine to go with the drive it does not have",
    );
}

/// A hull says "this axis engages no boost" by AUTHORING an idle
/// declaration, not by leaving the block out.
///
/// These assertions read `boost_ai.is_none()` until #885b stage 5c, when
/// every hull authored `[helm_console.boost_ai]`. Absence stopped meaning
/// "no boost doctrine" the moment a synthesised `idle = true` stopped
/// standing in for one — so the check moves onto the declaration rather than
/// off the property. It is strictly stronger than the old form: an empty
/// block, a rule on the `boost` channel, or a state machine all fail here,
/// where `is_none()` only ever caught the last two.
fn assert_idle_boost_declaration(hc: &HelmConsoleConfig, what: &str) {
    let boost_ai = hc.boost_ai.as_ref().unwrap_or_else(|| {
        panic!(
            "{what} — but the axis must still DECLARE that (PRD #774 US7): an \
                 omitted `[helm_console.boost_ai]` is silence, and silence is what \
                 gets a Rust-side policy synthesised for it"
        )
    });
    assert!(
        boost_ai.idle && boost_ai.rule.is_empty() && boost_ai.state.is_empty(),
        "{what} — the declaration must be an explicit `idle = true` and nothing \
             else, got {boost_ai:?}"
    );
}

/// The authored stateful block validates end to end through the real
/// content-load path (`EntityConfig::from_toml` runs the validator).
#[test]
fn authored_stateful_block_passes_content_validation() {
    let cfg = EntityConfig::from_toml(stateful_boost_toml()).expect("parse must succeed");
    let ai = cfg
        .helm_console
        .as_ref()
        .and_then(|h| h.boost_ai.as_ref())
        .expect("boost_ai present");
    assert!(validate_fine_system_ai_policy(ai, BOOST_CHANNELS, BOOST_VERBS).is_ok());
}

#[test]
fn empty_ai_declaration_is_rejected_as_silence() {
    // `[captain_console.ai]` with neither `idle` nor a rule is silence.
    let toml = r#"
name = "Silent"
[captain_console.ai]
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("empty") || err.contains("idle"), "got: {err}");
}

#[test]
fn explicit_idle_declaration_is_accepted() {
    let toml = r#"
name = "Idle"
[captain_console.ai]
idle = true
"#;
    let cfg = EntityConfig::from_toml(toml).expect("idle is a valid declaration");
    let ai = cfg.captain_console.unwrap().ai.unwrap();
    assert!(ai.idle);
    assert!(ai.to_policy().unwrap().idle);
}

// ── Per-bank weapon AI policy (issue #781) ───────────────────────────────

#[test]
fn default_phaser_and_blaster_bank_policies_validate_and_resolve() {
    let p = crate::entities::authored_ai_pins::shipped_policy_toml("phaser_bank");
    assert!(validate_fine_system_ai_policy(&p, PHASER_BANK_CHANNELS, PHASER_BANK_VERBS).is_ok());
    let pp = p.to_policy().expect("phaser default resolves");
    // Baseline: unconditional fire (not idle, one rule).
    assert!(!pp.idle);
    assert_eq!(pp.rules.len(), 1);

    let b = crate::entities::authored_ai_pins::shipped_policy_toml("blaster_bank");
    assert!(validate_fine_system_ai_policy(&b, BLASTER_BANK_CHANNELS, BLASTER_BANK_VERBS).is_ok());
    assert!(!b.to_policy().expect("blaster default resolves").idle);
}

#[test]
fn phaser_bank_inline_ai_policy_parses_from_toml() {
    let toml = r#"
name = "Gunboat"

# An armed hull owes its ship-level doctrine too since issue #956, whether or
# not it authors a `[behaviour]`. `idle = true` is the in-band way to say "this
# hull turns for nothing", which keeps the fixture about the BANK policy below.
[weapons_console.ai]
idle = true

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
auto_arc_deg = 60.0

[[weapons_console.phaser_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "fact(in_range) > 0 and fact(in_arc) > 0"
verb = "fire_phaser"
value = false
"#;
    let cfg = EntityConfig::from_toml(toml).expect("phaser bank ai must parse + validate");
    let bank = &cfg.weapons_console.unwrap().phaser_banks[0];
    let policy = bank.ai.as_ref().unwrap().to_policy().unwrap();
    assert_eq!(policy.rules.len(), 1);
    assert_eq!(
        policy.rules[0].verb,
        crate::ai::policy::AiPolicyVerb::FirePhaser
    );
}

#[test]
fn blaster_bank_inline_idle_ai_policy_parses_from_toml() {
    let toml = r#"
name = "Escort"

# The SHIP-LEVEL doctrine (issue #956), distinct from the bank's own idle
# declaration below — both are owed on an armed hull with no `[behaviour]`.
[weapons_console.ai]
idle = true

[[weapons_console.blaster_banks]]
id = "fore"

[weapons_console.blaster_banks.ai]
idle = true
"#;
    let cfg = EntityConfig::from_toml(toml).expect("blaster bank idle ai must parse");
    let bank = &cfg.weapons_console.unwrap().blaster_banks[0];
    assert!(bank.ai.as_ref().unwrap().to_policy().unwrap().idle);
}

#[test]
fn phaser_bank_ai_rejects_unknown_verb_at_load() {
    // The blaster verb on a phaser bank channel is an authoring error caught
    // by the from_toml validation loop, before any live tick.
    let toml = r#"
name = "Bad"

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
auto_arc_deg = 60.0

[[weapons_console.phaser_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "fire_blaster"
value = false
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("unknown verb"), "got: {err}");
}

#[test]
fn blaster_bank_ai_rejects_unknown_channel_at_load() {
    let toml = r#"
name = "Bad2"

[[weapons_console.blaster_banks]]
id = "fore"

[[weapons_console.blaster_banks.ai.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "fire_blaster"
value = false
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("unknown channel"), "got: {err}");
}

// ── Shields focus AI policy (issue #783) ─────────────────────────────────

#[test]
fn default_shields_focus_policy_validates_and_resolves() {
    let s = crate::entities::authored_ai_pins::shipped_policy_toml("shields_focus");
    assert!(validate_fine_system_ai_policy(&s, SHIELD_FOCUS_CHANNELS, SHIELD_FOCUS_VERBS).is_ok());
    let sp = s.to_policy().expect("shields focus default resolves");
    // Baseline: a priority-10 damage rule + a priority-0 imbalance fallback.
    assert!(!sp.idle);
    assert_eq!(sp.rules.len(), 2);
    // All four authored numbers seeded as params.
    assert!(sp.params.get(SHIELD_FOCUS_DAMAGE_WINDOW_PARAM).is_some());
    assert!(sp
        .params
        .get(SHIELD_FOCUS_MIN_DAMAGE_WINDOW_PARAM)
        .is_some());
    assert!(sp.params.get(SHIELD_FOCUS_DAMAGE_PCT_PARAM).is_some());
    assert!(sp.params.get(SHIELD_FOCUS_HEALTH_RATIO_PARAM).is_some());
}

#[test]
fn shields_ai_policy_parses_from_toml() {
    let toml = r#"
name = "Warden"

[shields_console.ai_policy]
param = { damage_window_secs = 6.0, min_damage_window_secs = 2.0, damage_pct_threshold = 60.0, health_ratio_threshold = 40.0 }

[[shields_console.ai_policy.rule]]
priority = 10
channel = "shield_focus"
when = "fact(recent_damage_pct_max) >= param(damage_pct_threshold)"
verb = "focus_shield_arc"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "true"
verb = "focus_shield_arc"
"#;
    let cfg = EntityConfig::from_toml(toml).expect("shields ai_policy must parse + validate");
    let ai = cfg
        .shields_console
        .as_ref()
        .and_then(|sc| sc.ai_policy.as_ref())
        .expect("shields_console.ai_policy present");
    assert_eq!(ai.param.get("damage_window_secs"), Some(&6.0));
    assert_eq!(ai.param.get("health_ratio_threshold"), Some(&40.0));
    let policy = ai.to_policy().expect("policy resolves");
    assert_eq!(policy.rules.len(), 2);
    assert_eq!(
        policy.rules[0].verb,
        crate::ai::policy::AiPolicyVerb::FocusShieldArc
    );
}

// ── Power allocation AI policy (issue #784) ──────────────────────────────

/// Minimal ship TOML carrying authored `[power_groups.*]` plus a
/// `[power.ai_policy]` block, used by the power-policy load tests.
fn power_policy_toml(rules: &str) -> String {
    format!(
        r#"
name = "Reactorer"

[power]
capacity = 90
rates = [ 5, 4, 3, 2, -2, -5 ]
emergency_threshold = 22

[power.ai_policy.param]
thrust_threshold = 0.7
min_reserve_helm = 50.0

{rules}

[power_groups.helm]
label = "HELM"
default_level = 2

[power_groups.weapons]
label = "WEAPONS"
default_level = 2

[power_groups.sensors]
label = "SENSORS"
default_level = 1

[power_groups.ops]
label = "OPS"
default_level = 1
"#
    )
}

#[test]
fn default_power_policy_validates_and_resolves() {
    // The shipped authored block is six rules since issue #1003 — hold,
    // elevate and baseline on each of helm and weapons — all emitting the
    // value-carrying allocation verb, and every rule declares a reserve
    // param.
    let cfg = crate::entities::authored_ai_pins::shipped_policy_toml("power");
    // Validated against the canonical group channels.
    assert!(validate_fine_system_ai_policy(
        &cfg,
        &["helm", "weapons", "sensors"],
        &[POWER_SET_ALLOCATION_VERB]
    )
    .is_ok());
    let p = cfg.to_policy().expect("default power policy resolves");
    assert!(!p.idle);
    assert_eq!(p.rules.len(), 6);
    // The elevate and hold rules carry the absolute magnitude in the verb
    // payload, and the elevated level is strictly above the baseline one —
    // the authored numbers themselves are the designer's business (#885b
    // stage 5d deleted the Rust constants they used to have to match).
    let levels: Vec<u8> = p
        .rules
        .iter()
        .filter_map(|r| match r.verb {
            crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(level) => Some(level),
            _ => None,
        })
        .collect();
    assert_eq!(levels.len(), 6, "every rule carries an allocation payload");
    assert!(
            levels.iter().max() > levels.iter().min(),
            "the elevate rules must raise their group above the baseline rules, or              the whole policy is a no-op: {levels:?}"
        );
    assert!(cfg.param.contains_key(POWER_HELM_RESERVE_PARAM));
    assert!(cfg.param.contains_key(POWER_WEAPONS_RESERVE_PARAM));
    // Each channel's SHED floor has a matching RESTORE floor above it: the
    // pair is the hysteresis, and one without the other is a ladder that
    // flips its channel every tick the charge rests on the floor.
    for (shed, restore) in [
        (POWER_HELM_RESERVE_PARAM, POWER_HELM_RESTORE_PARAM),
        (POWER_WEAPONS_RESERVE_PARAM, POWER_WEAPONS_RESTORE_PARAM),
    ] {
        let (lo, hi) = (
            cfg.param
                .get(shed)
                .unwrap_or_else(|| panic!("the shipped policy authors `{shed}`")),
            cfg.param
                .get(restore)
                .unwrap_or_else(|| panic!("the shipped policy authors `{restore}`")),
        );
        assert!(hi > lo, "`{restore}` ({hi}) must sit above `{shed}` ({lo})");
    }
}

#[test]
fn power_ai_policy_parses_and_decodes_magnitude_verb_from_toml() {
    let toml = power_policy_toml(
        r#"[[power.ai_policy.rule]]
priority = 10
channel = "helm"
when = "fact(thrust) >= param(thrust_threshold) and fact(battery_pct) >= param(min_reserve_helm)"
verb = "set_power_group_allocation"
level = 3

[[power.ai_policy.rule]]
priority = 0
channel = "ops"
when = "true"
verb = "set_power_group_allocation"
level = 1"#,
    );
    let cfg = EntityConfig::from_toml(&toml).expect("power ai_policy must parse + validate");
    let ai = cfg
        .power
        .as_ref()
        .and_then(|p| p.ai_policy.as_ref())
        .expect("power.ai_policy present");
    let policy = ai.to_policy().expect("policy resolves");
    assert_eq!(policy.rules.len(), 2);
    // The magnitude decodes into the verb payload (AC: absolute level).
    assert_eq!(
        policy.rules[0].verb,
        crate::ai::policy::AiPolicyVerb::SetPowerGroupAllocation(3)
    );
}

#[test]
fn power_ai_policy_rejects_non_authored_group_channel() {
    // AC1: channels are validated against the ship's AUTHORED power groups.
    // A rule targeting a group the ship does not author fails the load.
    let toml = power_policy_toml(
        r#"[[power.ai_policy.rule]]
priority = 0
channel = "shields"
when = "true"
verb = "set_power_group_allocation"
level = 2"#,
    );
    let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
    assert!(err.contains("unknown channel"), "got: {err}");
}

/// A hull that authors NO `[power_groups.*]` validates against the trio the
/// runtime seeds it with, not against an empty set.
///
/// `PowerSystem::from_authored_groups` falls back to
/// `seeded_with_defaults` — helm / weapons / shields at level 2 — for an
/// empty authored map, and `ai_power_allocation` then resolves the policy
/// against exactly those groups. Validating against nothing would have
/// rejected a policy the runtime was about to run, and that is not
/// hypothetical: the six NPC hulls that declare no power groups had to
/// author `[power.ai_policy]` in #885b stage 5c, so they are all in this
/// state today.
///
/// The negative case is `sensors`, which is the group that no longer
/// exists. It used to be `shields` — issue #952 swapped the two over in
/// `POWER_GROUP_ORDER`, and left as it was this test would have asserted
/// that a channel the runtime now seeds is rejected, i.e. the exact
/// opposite of the rule it is guarding.
#[test]
fn power_ai_policy_on_a_hull_with_no_authored_groups_validates_against_the_seeded_trio() {
    let toml = |channel: &str| {
        format!(
            r#"
name = "Grouper"

[power]
capacity = 100.0
rates = [ 6, 5, 4, 2, -2, -6 ]
emergency_threshold = 25.0

[[power.ai_policy.rule]]
priority = 0
channel = "{channel}"
when = "true"
verb = "set_power_group_allocation"
level = 2
"#
        )
    };
    for channel in crate::modifiers::power_system::POWER_GROUP_ORDER {
        let cfg = EntityConfig::from_toml(&toml(channel)).unwrap_or_else(|e| {
            panic!("`{channel}` is a group the runtime seeds, so it must validate: {e}")
        });
        assert!(
            cfg.ship_config
                .as_ref()
                .is_none_or(|sc| sc.power_groups.is_empty()),
            "precondition: this hull authors no `[power_groups.*]`"
        );
    }
    // …and the check has not simply been switched off: a group neither
    // authored nor seeded is still rejected.
    let err = EntityConfig::from_toml(&toml("sensors"))
        .expect_err("`sensors` is neither authored nor seeded")
        .to_string();
    assert!(err.contains("unknown channel"), "got: {err}");
}

#[test]
fn power_ai_policy_rejects_wrong_verb_at_load() {
    let toml = power_policy_toml(
        r#"[[power.ai_policy.rule]]
priority = 0
channel = "helm"
when = "true"
verb = "focus_shield_arc"
level = 2"#,
    );
    let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
    assert!(err.contains("unknown verb"), "got: {err}");
}

#[test]
fn power_ai_policy_rejects_undeclared_reserve_param() {
    // AC2 / AC6: a guard referencing an undeclared min-reserve param fails.
    let toml = power_policy_toml(
        r#"[[power.ai_policy.rule]]
priority = 10
channel = "weapons"
when = "fact(battery_pct) >= param(min_reserve_weapons)"
verb = "set_power_group_allocation"
level = 3"#,
    );
    let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
    assert!(err.contains("undeclared parameter"), "got: {err}");
}

#[test]
fn power_ai_policy_rejects_unparseable_guard() {
    let toml = power_policy_toml(
        r#"[[power.ai_policy.rule]]
priority = 0
channel = "helm"
when = "fact(thrust) >>> broken"
verb = "set_power_group_allocation"
level = 2"#,
    );
    let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
    assert!(err.contains("invalid `when`"), "got: {err}");
}

#[test]
fn power_ai_policy_rejects_empty_declaration() {
    let toml = power_policy_toml("");
    // `[power.ai_policy.param]` present but no rule and no idle → silence.
    let err = EntityConfig::from_toml(&toml).unwrap_err().to_string();
    assert!(err.contains("ai policy is empty"), "got: {err}");
}

#[test]
fn shields_idle_ai_policy_parses_from_toml() {
    let toml = r#"
name = "Passive"

[shields_console.ai_policy]
idle = true
"#;
    let cfg = EntityConfig::from_toml(toml).expect("shields idle ai_policy must parse");
    let ai = cfg.shields_console.unwrap().ai_policy.unwrap();
    assert!(ai.to_policy().unwrap().idle);
}

#[test]
fn shields_ai_policy_rejects_wrong_verb_at_load() {
    // A fire verb on the shield_focus channel is an authoring error caught by
    // the from_toml validation loop, before any live tick.
    let toml = r#"
name = "Bad"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "true"
verb = "fire_phaser"
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("unknown verb"), "got: {err}");
}

#[test]
fn shields_ai_policy_rejects_unknown_channel_at_load() {
    let toml = r#"
name = "Bad2"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "phaser_fire"
when = "true"
verb = "focus_shield_arc"
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("unknown channel"), "got: {err}");
}

#[test]
fn shields_ai_policy_rejects_undeclared_param_at_load() {
    let toml = r#"
name = "Bad3"

[[shields_console.ai_policy.rule]]
priority = 0
channel = "shield_focus"
when = "fact(recent_damage_pct_max) >= param(nonexistent)"
verb = "focus_shield_arc"
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("undeclared parameter"), "got: {err}");
}

// ── Torpedo tube + magazine AI policy (issue #782) ───────────────────────

#[test]
fn default_torpedo_tube_and_magazine_policies_validate_and_resolve() {
    let t = crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_tube");
    assert!(validate_fine_system_ai_policy(&t, TORPEDO_TUBE_CHANNELS, TORPEDO_TUBE_VERBS).is_ok());
    let tp = t.to_policy().expect("tube default resolves");
    // Baseline: unconditional load + launch (not idle, two rules).
    assert!(!tp.idle);
    assert_eq!(tp.rules.len(), 2);

    let m = crate::entities::authored_ai_pins::shipped_policy_toml("torpedo_magazine");
    assert!(
        validate_fine_system_ai_policy(&m, TORPEDO_MAGAZINE_CHANNELS, TORPEDO_MAGAZINE_VERBS)
            .is_ok()
    );
    assert!(!m.to_policy().expect("magazine default resolves").idle);
}

#[test]
fn torpedo_tube_inline_ai_policy_parses_from_toml() {
    let toml = r#"
name = "Bomber"

[torpedoes]
count = 8

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = 0.0
fire_arc_deg = 90.0

[[torpedoes.tubes.ai.rule]]
priority = 0
channel = "torpedo_load"
when = "fact(magazine) > 0"
verb = "load_torpedo"
value = false

[[torpedoes.tubes.ai.rule]]
priority = 0
channel = "torpedo_launch"
when = "fact(target_facing_shields) <= 0"
verb = "launch_torpedo"
value = false
"#;
    let cfg = EntityConfig::from_toml_in_mode(
        toml,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("tube ai must parse + validate");
    let tube = &cfg.torpedoes.unwrap().tubes[0];
    let policy = tube.ai.as_ref().unwrap().to_policy().unwrap();
    assert_eq!(policy.rules.len(), 2);
    assert_eq!(
        policy.rules[0].verb,
        crate::ai::policy::AiPolicyVerb::LoadTorpedo
    );
    assert_eq!(
        policy.rules[1].verb,
        crate::ai::policy::AiPolicyVerb::LaunchTorpedo
    );
}

#[test]
fn torpedo_magazine_inline_ai_policy_parses_from_toml() {
    let toml = r#"
name = "Bomber"

[torpedoes]
count = 8

[[torpedoes.ai.rule]]
priority = 0
channel = "torpedo_magazine_grant"
when = "fact(in_flight) < 3"
verb = "grant_torpedo_round"
value = false
"#;
    let cfg = EntityConfig::from_toml(toml).expect("magazine ai must parse + validate");
    let policy = cfg
        .torpedoes
        .unwrap()
        .ai
        .as_ref()
        .unwrap()
        .to_policy()
        .unwrap();
    assert_eq!(
        policy.rules[0].verb,
        crate::ai::policy::AiPolicyVerb::GrantTorpedoRound
    );
}

#[test]
fn torpedo_tube_inline_idle_ai_policy_parses_from_toml() {
    let toml = r#"
name = "Bomber"

[torpedoes]
count = 8

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = 0.0
fire_arc_deg = 90.0

[torpedoes.tubes.ai]
idle = true
"#;
    let cfg = EntityConfig::from_toml_in_mode(
        toml,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("tube idle ai must parse");
    let tube = &cfg.torpedoes.unwrap().tubes[0];
    assert!(tube.ai.as_ref().unwrap().to_policy().unwrap().idle);
}

#[test]
fn torpedo_tube_ai_rejects_unknown_verb_at_load() {
    // The magazine grant verb on a tube channel is an authoring error caught
    // by the from_toml validation loop, before any live tick.
    let toml = r#"
name = "Bad"

[torpedoes]
count = 8

[[torpedoes.tubes]]
id = "fore_port"
facing_deg = 0.0
fire_arc_deg = 90.0

[[torpedoes.tubes.ai.rule]]
priority = 0
channel = "torpedo_load"
when = "true"
verb = "grant_torpedo_round"
value = false
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("unknown verb"), "got: {err}");
}

#[test]
fn torpedo_magazine_ai_rejects_unknown_channel_at_load() {
    // A tube channel on the magazine block is rejected.
    let toml = r#"
name = "Bad"

[torpedoes]
count = 8

[[torpedoes.ai.rule]]
priority = 0
channel = "torpedo_launch"
when = "true"
verb = "grant_torpedo_round"
value = false
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("unknown channel"), "got: {err}");
}

#[test]
fn idle_with_rules_is_contradictory_and_rejected() {
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: true,
        param: Default::default(),
        rule: vec![FineSystemAiRuleToml {
            priority: 1,
            channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
            when: "true".into(),
            verb: CAPTAIN_SET_RED_ALERT_VERB.into(),
            value: true,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
    assert!(err.contains("idle"), "got: {err}");
}

#[test]
fn invalid_when_expression_is_rejected() {
    let toml = r#"
name = "BadExpr"
[captain_console.ai]
[[captain_console.ai.rule]]
priority = 1
channel = "red_alert"
when = "fact(x) &"
verb = "set_red_alert"
value = true
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(
        err.contains("invalid `when`") || err.contains("position"),
        "got: {err}"
    );
}

#[test]
fn unknown_channel_is_rejected() {
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![FineSystemAiRuleToml {
            priority: 1,
            channel: "shields".into(),
            when: "true".into(),
            verb: CAPTAIN_SET_RED_ALERT_VERB.into(),
            value: true,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
    assert!(err.contains("unknown channel"), "got: {err}");
}

#[test]
fn unknown_verb_is_rejected() {
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![FineSystemAiRuleToml {
            priority: 1,
            channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
            when: "true".into(),
            verb: "launch_torpedoes".into(),
            value: true,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
    assert!(err.contains("unknown verb"), "got: {err}");
}

#[test]
fn undeclared_parameter_reference_is_rejected() {
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(), // no params declared
        rule: vec![FineSystemAiRuleToml {
            priority: 1,
            channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
            when: "fact(secs_since_combat) < param(combat_window_secs)".into(),
            verb: CAPTAIN_SET_RED_ALERT_VERB.into(),
            value: true,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let err = validate_fine_system_ai_policy(&cfg, CHANNELS, VERBS).unwrap_err();
    assert!(err.contains("undeclared parameter"), "got: {err}");
}

#[test]
fn unknown_verb_surfaces_through_to_policy() {
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![FineSystemAiRuleToml {
            priority: 1,
            channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
            when: "true".into(),
            verb: "nope".into(),
            value: true,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    assert!(cfg.to_policy().is_err());
}

// ── Helm Engines/Steering AI policy (issue #779) ─────────────────────────

const ENGINES_CHANNELS: &[&str] = &[HELM_LONGITUDINAL_CHANNEL];
const ENGINES_VERBS: &[&str] = &[HELM_ACTUATE_DESIRED_TRAVEL_VERB];
const STEERING_CHANNELS: &[&str] = &[HELM_YAW_CHANNEL];
const STEERING_VERBS: &[&str] = &[HELM_ACTUATE_DESIRED_FACING_VERB];

#[test]
fn default_helm_policies_validate_and_resolve() {
    let eng = crate::entities::authored_ai_pins::shipped_policy_toml("engines");
    assert!(validate_fine_system_ai_policy(&eng, ENGINES_CHANNELS, ENGINES_VERBS).is_ok());
    let eng_policy = eng.to_policy().expect("engines policy resolves");
    assert_eq!(
        eng_policy.resolve_channel(
            HELM_LONGITUDINAL_CHANNEL,
            &crate::world::flags::AiFacts::new(),
            &[]
        ),
        Some(&crate::ai::policy::AiPolicyVerb::ActuateDesiredTravel),
        "the default Engines policy actuates desired travel unconditionally"
    );

    let steer = crate::entities::authored_ai_pins::shipped_policy_toml("steering");
    assert!(validate_fine_system_ai_policy(&steer, STEERING_CHANNELS, STEERING_VERBS).is_ok());
    let steer_policy = steer.to_policy().expect("steering policy resolves");
    assert_eq!(
        steer_policy.resolve_channel(HELM_YAW_CHANNEL, &crate::world::flags::AiFacts::new(), &[]),
        Some(&crate::ai::policy::AiPolicyVerb::ActuateDesiredFacing),
    );
}

#[test]
fn authored_helm_policies_parse_and_resolve_to_typed_policy() {
    let toml = r#"
name = "Test Cruiser"

[helm_console]
max_speed = 30.0

[helm_console.engines_ai]
param = { arrival_radius = 5.0 }

[[helm_console.engines_ai.rule]]
priority = 10
channel = "longitudinal"
# `range_to_target` is a real Engines-seeded fact (issue #1210): the fixture
# used a made-up `distance_to_dest`, which the fact registry now rejects at load.
when = "fact(range_to_target) > param(arrival_radius)"
verb = "actuate_desired_travel"

[helm_console.steering_ai]
idle = true
"#;
    let cfg = EntityConfig::from_toml(toml).expect("parse must succeed");
    let hc = cfg.helm_console.as_ref().expect("helm_console present");
    let engines = hc.engines_ai.as_ref().expect("engines_ai present");
    assert_eq!(engines.param.get("arrival_radius"), Some(&5.0));
    let engines_policy = engines.to_policy().expect("engines policy resolves");
    assert_eq!(engines_policy.rules.len(), 1);
    // An explicit idle Steering policy is a legal declaration (a ship whose
    // Steering never AI-actuates), distinct from silence.
    let steering = hc.steering_ai.as_ref().expect("steering_ai present");
    assert!(steering.to_policy().expect("steering resolves").idle);
}

#[test]
fn unknown_helm_engines_verb_is_rejected() {
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![FineSystemAiRuleToml {
            priority: 1,
            channel: HELM_LONGITUDINAL_CHANNEL.into(),
            // The Steering verb on the Engines channel is unknown here.
            verb: HELM_ACTUATE_DESIRED_FACING_VERB.into(),
            when: "true".into(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let err = validate_fine_system_ai_policy(&cfg, ENGINES_CHANNELS, ENGINES_VERBS).unwrap_err();
    assert!(err.contains("unknown verb"), "got: {err}");
}

#[test]
fn helm_wrong_channel_is_rejected() {
    // The Captain's `red_alert` channel is not a valid Steering channel.
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![FineSystemAiRuleToml {
            priority: 1,
            channel: CAPTAIN_RED_ALERT_CHANNEL.into(),
            verb: HELM_ACTUATE_DESIRED_FACING_VERB.into(),
            when: "true".into(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let err = validate_fine_system_ai_policy(&cfg, STEERING_CHANNELS, STEERING_VERBS).unwrap_err();
    assert!(err.contains("unknown channel"), "got: {err}");
}

#[test]
fn unknown_helm_verb_rejected_at_entity_load() {
    let toml = r#"
name = "BadHelm"
[helm_console]
max_speed = 30.0
[helm_console.engines_ai]
[[helm_console.engines_ai.rule]]
priority = 1
channel = "longitudinal"
when = "true"
verb = "warp_speed"
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("unknown verb"), "got: {err}");
}

#[test]
fn empty_helm_engines_declaration_is_rejected_as_silence() {
    let toml = r#"
name = "SilentHelm"
[helm_console]
max_speed = 30.0
[helm_console.engines_ai]
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("empty") || err.contains("idle"), "got: {err}");
}

// ── Helm secondary-actuator AI policy (issue #780) ───────────────────────

const LATERAL_CHANNELS: &[&str] = &[HELM_LATERAL_CHANNEL];
const LATERAL_VERBS: &[&str] = &[HELM_ACTUATE_LATERAL_THRUST_VERB];
const VERTICAL_CHANNELS: &[&str] = &[HELM_VERTICAL_CHANNEL];
const VERTICAL_VERBS: &[&str] = &[HELM_ACTUATE_VERTICAL_THRUST_VERB];
const IMPULSE_CHANNELS: &[&str] = &[HELM_IMPULSE_CHANNEL];
const IMPULSE_VERBS: &[&str] = &[HELM_ENGAGE_IMPULSE_VERB];
const BOOST_CHANNELS: &[&str] = &[HELM_BOOST_CHANNEL];
const BOOST_VERBS: &[&str] = &[HELM_ENGAGE_BOOST_VERB];

#[test]
fn default_secondary_helm_policies_validate_and_resolve() {
    // Lateral / vertical / impulse default to unconditional actuate/permit;
    // boost defaults to explicit idle (no AI boost).
    let lat = crate::entities::authored_ai_pins::shipped_policy_toml("lateral");
    assert!(validate_fine_system_ai_policy(&lat, LATERAL_CHANNELS, LATERAL_VERBS).is_ok());
    assert_eq!(
        lat.to_policy().unwrap().resolve_channel(
            HELM_LATERAL_CHANNEL,
            &crate::world::flags::AiFacts::new(),
            &[]
        ),
        Some(&crate::ai::policy::AiPolicyVerb::ActuateLateralThrust),
    );

    let vert = crate::entities::authored_ai_pins::shipped_policy_toml("vertical");
    assert!(validate_fine_system_ai_policy(&vert, VERTICAL_CHANNELS, VERTICAL_VERBS).is_ok());
    assert_eq!(
        vert.to_policy().unwrap().resolve_channel(
            HELM_VERTICAL_CHANNEL,
            &crate::world::flags::AiFacts::new(),
            &[]
        ),
        Some(&crate::ai::policy::AiPolicyVerb::ActuateVerticalThrust),
    );

    let imp = crate::entities::authored_ai_pins::shipped_policy_toml("impulse");
    assert!(validate_fine_system_ai_policy(&imp, IMPULSE_CHANNELS, IMPULSE_VERBS).is_ok());
    assert_eq!(
        imp.to_policy().unwrap().resolve_channel(
            HELM_IMPULSE_CHANNEL,
            &crate::world::flags::AiFacts::new(),
            &[]
        ),
        Some(&crate::ai::policy::AiPolicyVerb::EngageImpulse),
    );

    let boost = crate::entities::authored_ai_pins::shipped_policy_toml("boost");
    assert!(validate_fine_system_ai_policy(&boost, BOOST_CHANNELS, BOOST_VERBS).is_ok());
    let boost_policy = boost.to_policy().unwrap();
    assert!(
        boost_policy.idle,
        "default boost policy is an explicit idle"
    );
    assert_eq!(
        boost_policy.resolve_channel(
            HELM_BOOST_CHANNEL,
            &crate::world::flags::AiFacts::new(),
            &[]
        ),
        None,
        "an idle boost policy never engages"
    );
}

#[test]
fn authored_secondary_helm_policies_parse_at_entity_load() {
    let toml = r#"
name = "Test Cruiser"

[helm_console]
max_speed = 30.0

[helm_console.boost_ai.param]
boost_urgency = 0.5

[[helm_console.boost_ai.rule]]
priority = 10
channel = "boost"
when = "fact(hazard_urgency) > param(boost_urgency) and fact(boost_available) > 0"
verb = "engage_boost"

[helm_console.impulse_ai]
[[helm_console.impulse_ai.rule]]
priority = 10
channel = "impulse"
when = "fact(impulse_available) > 0"
verb = "engage_impulse"
"#;
    let cfg = EntityConfig::from_toml(toml).expect("parse must succeed");
    let hc = cfg.helm_console.as_ref().expect("helm_console present");
    let boost = hc.boost_ai.as_ref().expect("boost_ai present");
    assert_eq!(boost.to_policy().unwrap().rules.len(), 1);
    let impulse = hc.impulse_ai.as_ref().expect("impulse_ai present");
    assert_eq!(impulse.to_policy().unwrap().rules.len(), 1);
}

#[test]
fn wrong_verb_on_secondary_helm_channel_is_rejected() {
    // The impulse verb on the boost channel is unknown to the boost host.
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: Default::default(),
        rule: vec![FineSystemAiRuleToml {
            priority: 1,
            channel: HELM_BOOST_CHANNEL.into(),
            verb: HELM_ENGAGE_IMPULSE_VERB.into(),
            when: "true".into(),
            value: false,
            level: 0,
            response_index: 0,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let err = validate_fine_system_ai_policy(&cfg, BOOST_CHANNELS, BOOST_VERBS).unwrap_err();
    assert!(err.contains("unknown verb"), "got: {err}");
}

#[test]
fn wrong_secondary_helm_verb_rejected_at_entity_load() {
    // Authoring the lateral verb on the vertical channel fails the load.
    let toml = r#"
name = "BadHelm"
[helm_console]
max_speed = 30.0
[helm_console.vertical_ai]
[[helm_console.vertical_ai.rule]]
priority = 1
channel = "vertical"
when = "true"
verb = "actuate_lateral_thrust"
"#;
    let err = EntityConfig::from_toml(toml).unwrap_err().to_string();
    assert!(err.contains("unknown verb"), "got: {err}");
}

// ── Sensors target selector schema + validation (issue #776) ─────────────

fn sensors_selector_toml() -> &'static str {
    r##"
[sensors_console.selector]
horizon = 4000.0
switch_margin = 25.0
sources = ["combat-lock", "objective-destroy", "radar-contacts"]
eligibility = "candidate_fact(detectable) > 0 and candidate_fact(hostile) > 0"

[sensors_console.selector.param]
lock_weight = 900.0

[[sensors_console.selector.score]]
when = "candidate_fact(source_combat_lock) > 0"
weight = 900.0

[[sensors_console.selector.score]]
when = "candidate_fact(source_radar) > 0"
weight = 1.0
"##
}

#[test]
fn sensors_selector_parses_and_resolves_to_typed_selector() {
    let config = EntityConfig::from_toml(sensors_selector_toml()).expect("parse must succeed");
    let sel = config
        .sensors_console
        .as_ref()
        .and_then(|c| c.selector.as_ref())
        .expect("selector section present");
    let resolved = sel.to_selector().expect("selector resolves");
    assert_eq!(resolved.horizon, 4000.0);
    assert_eq!(resolved.switch_margin, 25.0);
    assert_eq!(resolved.score.len(), 2);
    assert!(validate_fine_system_ai_selector(sel, SENSORS_SELECTOR_SOURCES).is_ok());
}

#[test]
fn default_sensors_selector_is_valid_and_resolves() {
    let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("sensors");
    assert!(validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).is_ok());
    let resolved = cfg.to_selector().expect("default selector resolves");
    assert_eq!(resolved.score.len(), 3);
}

#[test]
fn selector_unknown_source_is_rejected() {
    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("sensors");
    cfg.sources.push("mystery-source".into());
    let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
    assert!(err.contains("mystery-source"), "got: {err}");
}

#[test]
fn selector_unparseable_eligibility_is_rejected() {
    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("sensors");
    cfg.eligibility = "candidate_fact(hostile) >".into();
    let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
    assert!(err.contains("eligibility"), "got: {err}");
}

#[test]
fn selector_undeclared_param_reference_is_rejected() {
    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("sensors");
    cfg.eligibility = "self_fact(power_rating) >= param(never_declared)".into();
    let err = validate_fine_system_ai_selector(&cfg, SENSORS_SELECTOR_SOURCES).unwrap_err();
    assert!(err.contains("never_declared"), "got: {err}");
}

#[test]
fn selector_bad_content_fails_entity_load() {
    let bad = r##"
[sensors_console.selector]
horizon = 100.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(detectable) > 0"
"##;
    assert!(
        EntityConfig::from_toml(bad).is_err(),
        "unknown selector source must fail from_toml before world activation"
    );
}

// ── Navigation target selector schema + validation (issue #778) ──────────

fn navigation_selector_toml() -> &'static str {
    r##"
[navigation_console.selector]
horizon = 5000.0
switch_margin = 30.0
sources = ["navigation-objectives", "chart-contacts"]
eligibility = "candidate_fact(reachable) > 0"

[navigation_console.selector.param]
objective_weight = 200.0

[[navigation_console.selector.score]]
when = "candidate_fact(source_nav_objective) > 0"
weight = 200.0

[[navigation_console.selector.score]]
when = "candidate_fact(source_chart_contact) > 0"
weight = 1.0
"##
}

#[test]
fn navigation_selector_parses_and_resolves_to_typed_selector() {
    let config = EntityConfig::from_toml(navigation_selector_toml()).expect("parse must succeed");
    let sel = config
        .navigation_console
        .as_ref()
        .and_then(|c| c.selector.as_ref())
        .expect("selector section present");
    let resolved = sel.to_selector().expect("selector resolves");
    assert_eq!(resolved.horizon, 5000.0);
    assert_eq!(resolved.switch_margin, 30.0);
    assert_eq!(resolved.score.len(), 2);
    assert!(validate_fine_system_ai_selector(sel, NAVIGATION_SELECTOR_SOURCES).is_ok());
}

#[test]
fn default_navigation_selector_is_valid_and_resolves() {
    let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("navigation");
    assert!(validate_fine_system_ai_selector(&cfg, NAVIGATION_SELECTOR_SOURCES).is_ok());
    let resolved = cfg.to_selector().expect("default selector resolves");
    // objective + chart-contact tiers.
    assert_eq!(resolved.score.len(), 2);
}

#[test]
fn navigation_selector_unknown_source_is_rejected() {
    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("navigation");
    cfg.sources.push("radar-contacts".into());
    let err = validate_fine_system_ai_selector(&cfg, NAVIGATION_SELECTOR_SOURCES).unwrap_err();
    assert!(err.contains("radar-contacts"), "got: {err}");
}

#[test]
fn navigation_selector_bad_content_fails_entity_load() {
    let bad = r##"
[navigation_console.selector]
horizon = 100.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(reachable) > 0"
"##;
    assert!(
        EntityConfig::from_toml(bad).is_err(),
        "unknown selector source must fail from_toml before world activation"
    );
}

// ── Tactical target selector schema + validation (issue #777) ────────────

fn tactical_selector_toml() -> &'static str {
    r##"
[weapons_console.selector]
horizon = 3000.0
switch_margin = 40.0
sources = ["sensors-designation", "objective-destroy", "last-attacker", "radar-contacts"]
eligibility = "candidate_fact(detectable) > 0 and (candidate_fact(source_objective) > 0 or candidate_fact(hostile) > 0)"

[weapons_console.selector.param]
sensors_designation_weight = 800.0

[[weapons_console.selector.score]]
when = "candidate_fact(source_sensors_designation) > 0"
weight = 800.0

[[weapons_console.selector.score]]
when = "candidate_fact(source_radar) > 0"
weight = 1.0
"##
}

#[test]
fn tactical_selector_parses_and_resolves_to_typed_selector() {
    // Lenient: the fixture's `[weapons_console]` owes a `weapons_doctrine`
    // declaration since issue #956, and this test is about the SELECTOR
    // schema beside it.
    let config = EntityConfig::from_toml_in_mode(
        tactical_selector_toml(),
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("parse must succeed");
    let sel = config
        .weapons_console
        .as_ref()
        .and_then(|c| c.selector.as_ref())
        .expect("selector section present");
    let resolved = sel.to_selector().expect("selector resolves");
    assert_eq!(resolved.horizon, 3000.0);
    assert_eq!(resolved.switch_margin, 40.0);
    assert_eq!(resolved.score.len(), 2);
    assert!(validate_fine_system_ai_selector(sel, TACTICAL_SELECTOR_SOURCES).is_ok());
}

#[test]
fn default_tactical_selector_is_valid_and_resolves() {
    let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("tactical");
    assert!(validate_fine_system_ai_selector(&cfg, TACTICAL_SELECTOR_SOURCES).is_ok());
    let resolved = cfg.to_selector().expect("default selector resolves");
    // objective, sensors-designation, retained, last-attacker, radar, and
    // the issue-#1162 operate order.
    assert_eq!(resolved.score.len(), 6);
}

/// The precedence invariant that prevents the #777 additive-stacking bug:
/// the objective weight must strictly dominate the maximum non-objective
/// stack (`sensors_designation + retained + last_attacker + radar`) by more
/// than `switch_margin`, so an in-range named Destroy objective always wins
/// the ranking AND survives hysteresis retention — even against the ship's
/// own current lock coinciding with its Sensors designation.
///
/// This was a `const {}` block over the synthesiser's Rust constants until
/// #885b stage 5d deleted them. It is now the arithmetic form of the same
/// invariant read off the SHIPPED authored weights, so a designer retuning
/// the block in TOML is held to it — which is what the constants were really
/// standing in for. The behavioural form lives in
/// `entities::authored_ai_pins::tactical_objective_beats_the_maximum_non_objective_stack`.
#[test]
fn shipped_tactical_selector_objective_dominates_max_non_objective_stack() {
    let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("tactical");
    let weight = |fact: &str| {
        cfg.score
            .iter()
            .find(|t| t.when.contains(fact))
            .unwrap_or_else(|| panic!("the authored Tactical selector scores `{fact}`"))
            .weight
    };
    let max_non_objective = weight("source_sensors_designation")
        + weight("source_retained")
        + weight("source_last_attacker")
        + weight("source_radar");
    assert!(
        max_non_objective < weight("source_objective") - cfg.switch_margin,
        "objective must dominate the max non-objective stack by more than the \
             switch margin, or a stacked non-objective candidate can beat — or be \
             retained over — an explicit Destroy objective (#777)."
    );
    assert!(
        weight("source_retained") > weight("source_last_attacker"),
        "retention must still outrank a fresh last attacker so an established \
             engagement is not broken off (the retired tier-2 > tier-3 ordering)."
    );
}

#[test]
fn tactical_selector_rejects_combat_lock_source() {
    // `combat-lock` is Tactical's OWN output — unioning it would be
    // circular, so it is not a registered Tactical source.
    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("tactical");
    cfg.sources.push(SELECTOR_SOURCE_COMBAT_LOCK.into());
    let err = validate_fine_system_ai_selector(&cfg, TACTICAL_SELECTOR_SOURCES).unwrap_err();
    assert!(err.contains(SELECTOR_SOURCE_COMBAT_LOCK), "got: {err}");
}

#[test]
fn tactical_selector_bad_content_fails_entity_load() {
    let bad = r##"
[weapons_console.selector]
horizon = 100.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(detectable) > 0"
"##;
    // Assert on the error TEXT, not just `is_err()`: since issue #956 a
    // bare `[weapons_console.selector]` also owes a `weapons_doctrine`
    // declaration under strict mode, so a bare `is_err()` here would keep
    // passing — for the wrong reason — if the unknown-source validation
    // were ever weakened, because the doctrine check runs later in
    // `from_toml` and would still reject the fixture. Pinning the message
    // to the source name keeps this test load-bearing on the Tactical
    // selector-source validation it names.
    let err = EntityConfig::from_toml(bad).unwrap_err().to_string();
    assert!(
        err.contains("not-a-real-source"),
        "unknown Tactical selector source must fail from_toml before world \
             activation, got: {err}"
    );
}

// ── Repair selector (issue #785) ────────────────────────────────────────

/// BASELINE PRESERVATION: the shipped Repair selector reproduces the retired
/// `(tier desc, deficit desc)` comparator, so a single damage-tier step must
/// strictly dominate the entire deficit ladder.
///
/// Read off the AUTHORED block since #885b stage 5d deleted the constants
/// this used to be a `const {}` block over. The behavioural form lives in
/// `entities::authored_ai_pins::repair_one_tier_step_beats_the_whole_deficit_ladder`.
#[test]
fn shipped_repair_selector_tier_dominates_max_deficit_stack() {
    let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("repair");
    let tier: Vec<f32> = cfg
        .score
        .iter()
        .filter(|t| t.when.contains("tier_ordinal"))
        .map(|t| t.weight)
        .collect();
    let deficit: Vec<f32> = cfg
        .score
        .iter()
        .filter(|t| t.when.contains("damage_fraction"))
        .map(|t| t.weight)
        .collect();
    assert_eq!(tier.len(), 3, "three tier rungs");
    assert_eq!(deficit.len(), 3, "three deficit bands");
    let max_deficit_stack: f32 = deficit.iter().sum();
    let one_tier_step = tier[0];
    assert!(
        max_deficit_stack < one_tier_step - cfg.switch_margin,
        "the whole deficit ladder must lose to ONE tier step, hysteresis included, \
             or the AI starts sending teams to nearly-dead minor stations ahead of \
             disabled critical ones."
    );

    let band = |key: &str| {
        *cfg.param
            .get(key)
            .unwrap_or_else(|| panic!("the authored Repair selector declares `{key}`"))
    };
    let (low, mid, high) = (
        band("deficit_band_low"),
        band("deficit_band_mid"),
        band("deficit_band_high"),
    );
    assert!(low < mid && mid < high && high < 1.0, "a monotone ladder");
    // ...and they sit INSIDE the urgent range, strictly above the
    // Damaged→Disabled damage-fraction boundary (1 − 0.25 HP). Bands placed
    // AT the tier thresholds all fire together for every Disabled station
    // and discriminate nothing.
    assert!(low > 1.0 - 0.25);
}

#[test]
fn default_repair_selector_config_validates() {
    let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("repair");
    assert!(
        validate_fine_system_ai_selector(&cfg, REPAIR_SELECTOR_SOURCES).is_ok(),
        "the canonical Repair selector must validate against its own sources"
    );
    assert!(
        cfg.to_selector().is_ok(),
        "the canonical Repair selector must resolve to a typed selector"
    );
}

#[test]
fn repair_selector_rejects_unregistered_source() {
    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("repair");
    cfg.sources.push(SELECTOR_SOURCE_RADAR_CONTACTS.into());
    let err = validate_fine_system_ai_selector(&cfg, REPAIR_SELECTOR_SOURCES).unwrap_err();
    assert!(err.contains(SELECTOR_SOURCE_RADAR_CONTACTS), "got: {err}");
}

#[test]
fn repair_selector_undeclared_param_is_rejected() {
    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("repair");
    cfg.eligibility = "candidate_fact(damage_fraction) >= param(nope)".to_string();
    let err = validate_fine_system_ai_selector(&cfg, REPAIR_SELECTOR_SOURCES).unwrap_err();
    assert!(err.contains("nope"), "got: {err}");
}

/// `[repair.selector]` is the first selector block outside a `*_console`
/// section; it parses, and bad content fails the entity load before any
/// live tick.
#[test]
fn repair_selector_parses_from_toml_and_bad_content_fails_entity_load() {
    let good = r##"
[repair]
repair_team_count = 2

[repair.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["damaged-stations", "core-bucket"]
eligibility = "candidate_fact(source_repair_request) > 0"

[[repair.selector.score]]
when = "candidate_fact(tier_ordinal) >= 2"
weight = 100.0
"##;
    let cfg = EntityConfig::from_toml(good).expect("valid [repair.selector] must parse");
    let sel = cfg
        .repair
        .expect("repair section present")
        .selector
        .expect("selector present");
    assert_eq!(sel.sources.len(), 2);
    assert_eq!(sel.score.len(), 1);

    let bad = r##"
[repair]
repair_team_count = 2

[repair.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["not-a-real-source"]
eligibility = "candidate_fact(source_repair_request) > 0"
"##;
    assert!(
        EntityConfig::from_toml(bad).is_err(),
        "unknown Repair selector source must fail from_toml before world activation"
    );
}

#[test]
fn repair_config_without_selector_defaults_to_none() {
    let cfg =
        EntityConfig::from_toml("[repair]\nrepair_team_count = 2\n").expect("parse must succeed");
    assert!(cfg.repair.expect("repair present").selector.is_none());
}

// ── Comms console AI (issue #786) ───────────────────────────────────────

/// BAND PLACEMENT (the #785 lesson): the objective-score ladder must be a
/// strictly increasing set of thresholds that actually straddles the
/// population of authored `base_priority` values (20 … 100), or every hail
/// scores identically and the "ranking" collapses onto the selector's
/// smallest-UUID tie-break.
///
/// Read off the AUTHORED block since #885b stage 5d deleted the constants
/// this used to be a `const {}` block over. The behavioural form lives in
/// `entities::authored_ai_pins::comms_band_ladder_ranks_hails_by_objective_utility`.
#[test]
fn shipped_comms_selector_bands_are_a_monotone_ladder_over_real_scores() {
    let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
    let band = |key: &str| {
        *cfg.param
            .get(key)
            .unwrap_or_else(|| panic!("the authored Comms selector declares `{key}`"))
    };
    let (low, mid, high) = (
        band("score_band_low"),
        band("score_band_mid"),
        band("score_band_high"),
    );
    assert!(low < mid && mid < high, "a monotone ladder");
    // Straddles the shipped authoring range: the lowest band sits above the
    // cheapest authored priority (20) and the highest below the dearest
    // (100), so all four buckets are reachable.
    assert!(low > 20.0);
    assert!(high < 100.0);
    // A hail is a one-shot event: nothing to retain, so no hysteresis.
    assert_eq!(cfg.switch_margin, 0.0);
}

#[test]
fn default_comms_selector_config_validates() {
    let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
    assert!(
        validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).is_ok(),
        "the canonical Comms selector must validate against its own sources"
    );
    assert!(
        cfg.to_selector().is_ok(),
        "the canonical Comms selector must resolve to a typed selector"
    );
}

/// The two eligibility terms the #786 review added, pinned by name so a
/// future edit cannot quietly drop them:
///   - `has_open_hail_thread` (NOT `has_unread_from_sender`) is the
///     anti-respam gate — it must key on hails WE issued, or a
///     scenario-pushed greeting permanently suppresses a legitimate hail;
///   - `self_fact(comms_available)` is the AC2 system-availability gate,
///     which the AC names explicitly and which nothing else in the hail path
///     enforces.
#[test]
fn default_comms_selector_eligibility_names_the_anti_respam_and_availability_gates() {
    let cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
    assert!(
        cfg.eligibility
            .contains("candidate_fact(has_open_hail_thread) < 1"),
        "got: {}",
        cfg.eligibility
    );
    assert!(
        !cfg.eligibility.contains("has_unread_from_sender"),
        "inbound traffic of unknown provenance must NOT gate hailing; got: {}",
        cfg.eligibility
    );
    assert!(
        cfg.eligibility.contains("self_fact(comms_available) > 0"),
        "got: {}",
        cfg.eligibility
    );
}

#[test]
fn comms_selector_rejects_unregistered_source() {
    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
    cfg.sources.push(SELECTOR_SOURCE_RADAR_CONTACTS.into());
    let err = validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).unwrap_err();
    assert!(err.contains(SELECTOR_SOURCE_RADAR_CONTACTS), "got: {err}");
}

#[test]
fn comms_selector_undeclared_param_is_rejected() {
    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
    cfg.eligibility = "candidate_fact(objective_score) >= param(nope)".to_string();
    let err = validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).unwrap_err();
    assert!(err.contains("nope"), "got: {err}");
}

#[test]
fn comms_selector_bad_guard_is_rejected() {
    let mut cfg = crate::entities::authored_ai_pins::shipped_selector_toml("comms_hail");
    cfg.eligibility = "candidate_fact(in_range) >>> 0".to_string();
    assert!(validate_fine_system_ai_selector(&cfg, COMMS_SELECTOR_SOURCES).is_err());
}

/// BASELINE PRESERVATION: the canonical response policy reproduces the
/// retired `handle_comms_channel2` stub's decision — a single rule answering
/// with index 0 — while routing it through admission.
///
/// The rule is not `when = "true"`. The stub ran ONLY on channel-2 arrival,
/// so it could not repeat and its sender was in range by construction; this
/// policy is re-resolved every tick against every open dialogue. The two
/// guard terms restore exactly those two implicit preconditions:
/// `sender_in_range` (or the router rejects the response, forever) and
/// `comms_available` (AC2 — a Destroyed Comms system answers nothing).
#[test]
fn default_comms_response_ai_config_reproduces_the_retired_stub_decision() {
    let cfg = crate::entities::authored_ai_pins::shipped_policy_toml("comms_response");
    assert!(
        validate_fine_system_ai_policy(&cfg, COMMS_RESPOND_CHANNELS, COMMS_RESPOND_VERBS).is_ok(),
        "the canonical Comms response policy must validate"
    );
    assert_eq!(cfg.rule.len(), 1);
    assert_eq!(cfg.rule[0].channel, COMMS_RESPOND_CHANNEL);
    assert_eq!(
        cfg.rule[0].when, "fact(comms_available) > 0 and fact(sender_in_range) > 0",
        "AC2 system availability and the router's range precondition are both \
             named — an unguarded rule re-emits rejected responses every tick"
    );
    assert_eq!(
            cfg.rule[0].response_index, 0,
            "the shipped policy answers with the FIRST response, reproducing the              retired `record_response(id, 0)` stub's decision"
        );
    let policy = cfg.to_policy().expect("must resolve to a typed policy");
    assert_eq!(
        policy.rules[0].verb,
        crate::ai::policy::AiPolicyVerb::RespondToMessage(0),
        "the authored response_index must ride the verb"
    );
}

/// The `response_index` payload decodes onto the verb (the SECOND
/// value-carrying verb, after `set_power_group_allocation`), and is a
/// SEPARATE field from `level` so a rule's meaning never depends on its verb.
#[test]
fn comms_respond_verb_decodes_its_own_response_index_field() {
    let cfg = FineSystemAiConfigToml {
        evaluate_every_ticks: default_evaluate_every_ticks(),
        idle: false,
        param: std::collections::HashMap::new(),
        rule: vec![FineSystemAiRuleToml {
            priority: 5,
            channel: COMMS_RESPOND_CHANNEL.to_string(),
            when: "true".to_string(),
            verb: COMMS_RESPOND_VERB.to_string(),
            value: true,
            // A non-zero `level` must be ignored by this verb.
            level: 3,
            response_index: 2,
        }],
        initial_state: None,
        state: Vec::new(),
        memory: std::collections::HashMap::new(),
    };
    let policy = cfg.to_policy().expect("must resolve");
    assert_eq!(
        policy.rules[0].verb,
        crate::ai::policy::AiPolicyVerb::RespondToMessage(2)
    );
}

#[test]
fn comms_response_policy_rejects_wrong_verb_and_unknown_channel() {
    let mut wrong_verb = crate::entities::authored_ai_pins::shipped_policy_toml("comms_response");
    wrong_verb.rule[0].verb = POWER_SET_ALLOCATION_VERB.to_string();
    let err =
        validate_fine_system_ai_policy(&wrong_verb, COMMS_RESPOND_CHANNELS, COMMS_RESPOND_VERBS)
            .unwrap_err();
    assert!(err.contains(POWER_SET_ALLOCATION_VERB), "got: {err}");

    let mut wrong_channel =
        crate::entities::authored_ai_pins::shipped_policy_toml("comms_response");
    wrong_channel.rule[0].channel = "shield_focus".to_string();
    let err =
        validate_fine_system_ai_policy(&wrong_channel, COMMS_RESPOND_CHANNELS, COMMS_RESPOND_VERBS)
            .unwrap_err();
    assert!(err.contains("shield_focus"), "got: {err}");
}

#[test]
fn comms_response_policy_rejects_undeclared_param() {
    let mut cfg = crate::entities::authored_ai_pins::shipped_policy_toml("comms_response");
    cfg.rule[0].when = "fact(response_count) > param(nope)".to_string();
    let err = validate_fine_system_ai_policy(&cfg, COMMS_RESPOND_CHANNELS, COMMS_RESPOND_VERBS)
        .unwrap_err();
    assert!(err.contains("nope"), "got: {err}");
}

/// `[comms_console]` carries BOTH machines, parses, and bad content in
/// either fails the entity load before any live tick.
#[test]
fn comms_console_parses_both_blocks_and_bad_content_fails_entity_load() {
    let good = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["hail-objectives", "comms-contacts"]
eligibility = "candidate_fact(source_hail_objective) > 0 and candidate_fact(in_range) > 0"

[[comms_console.selector.score]]
when = "candidate_fact(objective_score) > 0"
weight = 100.0

[[comms_console.ai.rule]]
priority = 10
channel = "comms_respond"
when = "fact(is_urgent) > 0"
verb = "respond_to_message"
response_index = 1
"##;
    let cfg = EntityConfig::from_toml(good).expect("valid [comms_console] must parse");
    let console = cfg.comms_console.expect("comms_console present");
    let sel = console.selector.expect("selector present");
    assert_eq!(sel.sources.len(), 2);
    assert_eq!(sel.score.len(), 1);
    let ai = console.ai.expect("ai present");
    assert_eq!(ai.rule.len(), 1);
    assert_eq!(ai.rule[0].response_index, 1);

    let bad_source = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["radar-contacts"]
eligibility = "candidate_fact(source_hail_objective) > 0"
"##;
    assert!(
        EntityConfig::from_toml(bad_source).is_err(),
        "unknown Comms selector source must fail from_toml"
    );

    let bad_guard = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["hail-objectives"]
eligibility = "candidate_fact(in_range) >>> 0"
"##;
    assert!(
        EntityConfig::from_toml(bad_guard).is_err(),
        "an unparseable Comms selector guard must fail from_toml"
    );

    let undeclared_param = r##"
[comms_console.selector]
horizon = 1000.0
switch_margin = 0.0
sources = ["hail-objectives"]
eligibility = "candidate_fact(objective_score) > param(nope)"
"##;
    assert!(
        EntityConfig::from_toml(undeclared_param).is_err(),
        "an undeclared selector param must fail from_toml"
    );

    let bad_verb = r##"
[[comms_console.ai.rule]]
priority = 0
channel = "comms_respond"
when = "true"
verb = "set_red_alert"
"##;
    assert!(
        EntityConfig::from_toml(bad_verb).is_err(),
        "a non-Comms verb on the comms_respond channel must fail from_toml"
    );

    let bad_channel = r##"
[[comms_console.ai.rule]]
priority = 0
channel = "not_a_channel"
when = "true"
verb = "respond_to_message"
"##;
    assert!(
        EntityConfig::from_toml(bad_channel).is_err(),
        "an unknown Comms channel must fail from_toml"
    );
}

/// `[comms]` (per-entity comms RANGE) and `[comms_console]` (the console's
/// AI) are deliberately different sections; authoring one must not imply the
/// other.
#[test]
fn comms_range_section_does_not_carry_the_console_ai() {
    let cfg = EntityConfig::from_toml("[comms]\nrange = 8000.0\n").expect("parse must succeed");
    assert!(cfg.comms.is_some());
    assert!(
        cfg.comms_console.is_none(),
        "[comms] is the entity's comms RANGE; the console AI lives in [comms_console]"
    );
}

#[test]
fn comms_config_parses_range_from_toml() {
    let toml_str = r##"
[comms]
range = 8000.0
"##;
    let config = EntityConfig::from_toml(toml_str).expect("parse must succeed");
    let comms = config.comms.expect("comms section present");
    assert_eq!(comms.range, 8000.0);
}

#[test]
fn comms_config_is_none_when_section_absent() {
    let config = EntityConfig::from_toml("").expect("parse must succeed");
    assert!(config.comms.is_none(), "no [comms] → field is None");
}

// ── LOD selection ──────────────────

/// Two levels: GLB near (< 50), sphere fallback beyond.
fn two_level_lods() -> Vec<LodLevel> {
    vec![
        LodLevel {
            max_distance: Some(50.0),
            model: Some("assets/models/rock.glb".into()),
            ..Default::default()
        },
        LodLevel {
            max_distance: None,
            shape: Some(MeshShape::Sphere),
            ..Default::default()
        },
    ]
}

#[test]
fn select_lod_empty_returns_zero() {
    assert_eq!(select_lod(&[], 123.0, None), 0);
    assert_eq!(select_lod(&[], 0.0, Some(3)), 0);
}

#[test]
fn select_lod_single_level_always_zero() {
    let levels = vec![LodLevel {
        max_distance: None,
        shape: Some(MeshShape::Sphere),
        ..Default::default()
    }];
    assert_eq!(select_lod(&levels, 0.0, None), 0);
    assert_eq!(select_lod(&levels, 9999.0, None), 0);
    assert_eq!(select_lod(&levels, 9999.0, Some(0)), 0);
}

#[test]
fn select_lod_basic_band_selection() {
    let levels = two_level_lods();
    // Near band → level 0.
    assert_eq!(select_lod(&levels, 0.0, None), 0);
    assert_eq!(select_lod(&levels, 49.0, None), 0);
    // Far band → level 1.
    assert_eq!(select_lod(&levels, 60.0, None), 1);
    assert_eq!(select_lod(&levels, 100_000.0, None), 1);
}

#[test]
fn select_lod_boundary_is_exclusive_upper() {
    let levels = two_level_lods();
    // Exactly at the boundary belongs to the far band (upper bound exclusive).
    assert_eq!(select_lod(&levels, 50.0, None), 1);
    // Just below stays near.
    assert_eq!(select_lod(&levels, 49.999, None), 0);
}

#[test]
fn select_lod_hysteresis_holds_when_moving_outward() {
    let levels = two_level_lods();
    // Currently at near level 0; distance crept just past the boundary but
    // within the margin → hold at 0.
    assert_eq!(select_lod(&levels, 52.0, Some(0)), 0);
    assert_eq!(
        select_lod(&levels, 50.0 + LOD_HYSTERESIS_MARGIN, Some(0)),
        0,
        "exactly boundary + margin still holds (strict >)"
    );
    // Clear of the margin → switch outward to level 1.
    assert_eq!(
        select_lod(&levels, 50.0 + LOD_HYSTERESIS_MARGIN + 0.1, Some(0)),
        1
    );
}

#[test]
fn select_lod_hysteresis_holds_when_moving_inward() {
    let levels = two_level_lods();
    // Currently at far level 1; distance dropped just below the boundary but
    // within the margin → hold at 1.
    assert_eq!(select_lod(&levels, 48.0, Some(1)), 1);
    assert_eq!(
        select_lod(&levels, 50.0 - LOD_HYSTERESIS_MARGIN, Some(1)),
        1,
        "exactly boundary - margin still holds (strict <)"
    );
    // Clear of the margin → switch inward to level 0.
    assert_eq!(
        select_lod(&levels, 50.0 - LOD_HYSTERESIS_MARGIN - 0.1, Some(1)),
        0
    );
}

#[test]
fn select_lod_no_thrash_across_repeated_calls_at_boundary() {
    let levels = two_level_lods();
    // Sit right on the boundary and re-evaluate repeatedly: whatever level we
    // start at, we should stay there (no oscillation).
    let mut level = 0usize;
    for _ in 0..10 {
        level = select_lod(&levels, 50.0, Some(level));
    }
    assert_eq!(level, 0, "started near, stays near at the boundary");

    let mut level = 1usize;
    for _ in 0..10 {
        level = select_lod(&levels, 50.0, Some(level));
    }
    assert_eq!(level, 1, "started far, stays far at the boundary");
}

#[test]
fn select_lod_three_levels_and_out_of_range() {
    let levels = vec![
        LodLevel {
            max_distance: Some(30.0),
            model: Some("a.glb".into()),
            ..Default::default()
        },
        LodLevel {
            max_distance: Some(80.0),
            shape: Some(MeshShape::Sphere),
            ..Default::default()
        },
        LodLevel {
            max_distance: None,
            shape: Some(MeshShape::Sphere),
            ..Default::default()
        },
    ];
    assert_eq!(select_lod(&levels, 10.0, None), 0);
    assert_eq!(select_lod(&levels, 50.0, None), 1);
    assert_eq!(select_lod(&levels, 500.0, None), 2);
    // Negative / zero distances clamp to the nearest band.
    assert_eq!(select_lod(&levels, -5.0, None), 0);
    // A stale current index beyond the list is clamped and re-resolved.
    assert_eq!(select_lod(&levels, 500.0, Some(99)), 2);
}

// ── `[[mesh.lod]]` has moved to the model sidecar (issue #914) ─────────

/// The old location cannot come back silently: rejected at parse, with a
/// message that names the sidecar the chain belongs in — not the generic
/// "unknown field `lod`" that `deny_unknown_fields` would emit.
#[test]
fn mesh_lod_in_entity_toml_is_rejected_with_a_targeted_message() {
    let toml_str = r##"
[mesh]
model = "assets/models/rock.glb"
variant = "small"
shape = "sphere"
colour = [0.5, 0.5, 0.5]
radius = 2.0

[[mesh.lod]]
max_distance = 50.0
model = "assets/models/rock.glb"
"##;
    let err = EntityConfig::from_toml(toml_str)
        .expect_err("[[mesh.lod]] must not parse from an entity TOML");
    let msg = err.to_string();
    assert!(
        msg.contains("assets/models/rock.small.toml"),
        "the error must name the sidecar the chain moved to; got: {msg}"
    );
    assert!(
        msg.contains("[[lod]]"),
        "the error must name the new block; got: {msg}"
    );
    assert!(
        !msg.contains("unknown field"),
        "the targeted check must run before deny_unknown_fields; got: {msg}"
    );
}

/// A template with no `model` still gets a pointer, just a generic one —
/// the check must not depend on the mesh naming a GLB.
#[test]
fn mesh_lod_is_rejected_even_without_a_model_reference() {
    let toml_str = "[mesh]\nshape = \"sphere\"\ncolour = [0.5, 0.5, 0.5]\n\n[[mesh.lod]]\nshape = \"sphere\"\n";
    let err = EntityConfig::from_toml(toml_str).expect_err("must not parse");
    assert!(err.to_string().contains("model rig sidecar"));
}

/// The guard is scoped: a mesh without a ladder is untouched, and `lod`
/// elsewhere in the document is not this field.
#[test]
fn a_mesh_without_a_ladder_still_parses() {
    let config = EntityConfig::from_toml(
            "[mesh]\nmodel = \"assets/models/rock.glb\"\nshape = \"sphere\"\ncolour = [0.5, 0.5, 0.5]\n",
        )
        .expect("a plain [mesh] must still parse");
    let mesh = config.mesh.expect("mesh section present");
    assert_eq!(mesh.model.as_deref(), Some("assets/models/rock.glb"));
}

// ── NPC red-alert provisioning (issue #749) ─────────────────────────────────

fn red_alert_systems(config: &EntityConfig) -> Vec<&crate::ship::config::SystemInstanceConfig> {
    config
        .ship_config
        .as_ref()
        .map(|sc| {
            sc.systems
                .iter()
                .filter(|s| s.kind == crate::ship::system_registry::RED_ALERT_KIND)
                .collect()
        })
        .unwrap_or_default()
}

/// A minimal `[behaviour]` hull that authors no `red_alert` system, so the
/// #749 provision is exercised on a fixture rather than on a shipped hull.
///
/// Every shipped hull authors its own `red_alert` since #871 gave the NPC
/// hulls a Captain seat to own it, so the provision has no shipped hull left
/// to fire on — but it is still live code for any hull authored without one,
/// which is what these fixtures keep covered.
const BARE_BEHAVIOUR_HULL: &str = r#"
tags = ["ship"]

[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
text = "Destroy hostiles"
directive_kind = "Destroy"
base_priority = 35.0

[[system]]
id = "helm-thrust"
kind = "helm_thrust"
ai_only = true
"#;

#[test]
fn behaviour_npc_without_red_alert_gets_ai_only_ownerless_provision() {
    // A hull that authors [behaviour] but no red_alert system. Spawn
    // provisioning must add exactly one AI-only, ownerless red_alert
    // capability so the AI captain can raise it.
    let config = EntityConfig::from_toml_in_mode(
        BARE_BEHAVIOUR_HULL,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("fixture must parse");
    let reds = red_alert_systems(&config);
    assert_eq!(
        reds.len(),
        1,
        "behaviour NPC must be provisioned exactly one red_alert system"
    );
    let sys = reds[0];
    assert_eq!(sys.id.0, crate::ship::system_registry::RED_ALERT_SYSTEM_ID);
    assert!(sys.ai_only, "provisioned red_alert must be ai_only");
    assert!(
        sys.station.is_none(),
        "provisioned red_alert must be ownerless"
    );
}

#[test]
fn behaviour_npc_red_alert_provision_is_not_hull_specific() {
    // A second, differently-shaped behaviour hull — shield arcs and a weapon
    // rather than a bare helm axis — to confirm the provisioning keys off
    // `[behaviour]` alone.
    let toml_str = r#"
tags = ["ship"]

[[shield_arc]]
id = "all"
label = "All"
center_deg = 0
width_deg = 360
max_hp = 15

[weapons_console]

[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 180.0
auto_arc_deg = 180.0

[behaviour]

[[behaviour.doctrine]]
id = "destroy-hostiles"
text = "Destroy hostiles"
directive_kind = "Destroy"
base_priority = 35.0

[[system]]
id = "phaser-fore"
kind = "phaser_bank"
ai_only = true
"#;
    let config = EntityConfig::from_toml_in_mode(
        toml_str,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("fixture must parse");
    let reds = red_alert_systems(&config);
    assert_eq!(reds.len(), 1, "behaviour NPC provisioned one red_alert");
    assert!(reds[0].ai_only && reds[0].station.is_none());
}

#[test]
fn harrow_destroyer_authors_its_own_captain_owned_red_alert() {
    // Since #871 this NPC hull carries a Captain seat and authors its own
    // red_alert on it. Provisioning must be idempotent — no second system —
    // and the authored ownership must survive. The control source is
    // unchanged from the provisioned era: an unmanned Captain seat boots on
    // `Backfill`, which automates every system it owns, so
    // `operate_captain_ai` still raises Red Alert.
    // (#892) Re-pointed off the retired `pirate_raider.toml`.
    let toml_str = &resolved_text("ship_harrow_destroyer");
    let config = EntityConfig::from_toml(toml_str).expect("the Harrow Destroyer must parse");
    let reds = red_alert_systems(&config);
    assert_eq!(
        reds.len(),
        1,
        "authored red_alert must not be double-provisioned"
    );
    assert_eq!(
        reds[0].station,
        Some(crate::core::messages::StationId("captain".into())),
        "the Captain seat owns Red Alert"
    );
    assert!(
        !reds[0].ai_only,
        "a station-owned system must not rely on `ai_only`"
    );
}

#[test]
fn explicit_red_alert_is_left_untouched() {
    // The Alliance Destroyer authors an explicit red_alert system owned by
    // the captain station. Provisioning must be idempotent: no second
    // system, and the authored ownership survives (AC4).
    // Through the resolver, not `include_str!`: this hull is COMPOSED since
    // #875, so its baked bytes are no longer the document that spawns. The
    // assertion below is unchanged — provisioning idempotence is the claim,
    // and it is now made against the real resolved hull.
    let config = shipped_hull("alliance_destroyer");
    let reds = red_alert_systems(&config);
    assert_eq!(
        reds.len(),
        1,
        "authored red_alert must not be double-provisioned"
    );
    assert_eq!(
        reds[0].station,
        Some(crate::core::messages::StationId("captain".into())),
        "authored captain ownership must survive provisioning"
    );
    assert!(
        !reds[0].ai_only,
        "authored player red_alert must remain non-ai_only"
    );
}

#[test]
fn non_behaviour_entity_gets_no_red_alert_provision() {
    // An asteroid has no [behaviour] block → no ship capabilities at all.
    let toml_str = r#"
tags = ["asteroid"]
"#;
    let config = EntityConfig::from_toml(toml_str).expect("asteroid must parse");
    assert!(
        config.ship_config.is_none(),
        "non-behaviour entity must not synthesise a ship_config"
    );
    assert!(
        red_alert_systems(&config).is_empty(),
        "non-behaviour entity must get no red_alert system"
    );
}

// ── The #929 steering modifier sets, refused at LOAD ─────────────────────────
//
// The leg gates these sit beside answer a half-authored set by declining the
// whole arm at runtime, and that is right for them: a leg that does not happen
// is a behaviour a designer can watch not happen. Arc-keeping and the
// weak-broadside flip MODIFY a ring that is already running, so the same mistake
// produces a hull that flies a slightly wrong ring for ever and never says why.
// These four pin the load errors that close that gap.

/// The shipped cruiser's own text, with `mutate` applied to its
/// `[helm_console.steering_ai.param]` table, parsed exactly as the loader does.
fn cruiser_with_steering_params(mutate: impl Fn(&mut String)) -> Result<(), String> {
    let mut text = resolved_text("alliance_cruiser");
    mutate(&mut text);
    crate::entities::config::EntityConfig::from_toml(&text)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Every #929 modifier set is authored complete on the shipped hull, and the
/// hull loads. The control for the three refusals below — without it they could
/// all be passing because the file never had the params in the first place.
#[test]
fn the_shipped_cruiser_authors_both_steering_modifier_sets_complete() {
    let text = resolved_text("alliance_cruiser");
    let cfg = crate::entities::config::EntityConfig::from_toml(&text).expect("hull must parse");
    let steering = cfg
        .helm_console
        .as_ref()
        .and_then(|hc| hc.steering_ai.as_ref())
        .expect("hull authors [helm_console.steering_ai]");
    for required in crate::ship::helm_ai::ARC_KEEP_PARAMS
        .iter()
        .chain(crate::ship::helm_ai::WEAK_SHIELD_FLIP_PARAMS)
    {
        assert!(
            steering.param.contains_key(*required),
            "steering_ai must author `{required}` — the host reads each set whole"
        );
    }
}

/// Half a set is a load error, in both directions and for both sets.
#[test]
fn a_half_authored_steering_modifier_set_is_refused_at_load() {
    for dropped in crate::ship::helm_ai::ARC_KEEP_PARAMS
        .iter()
        .chain(crate::ship::helm_ai::WEAK_SHIELD_FLIP_PARAMS)
    {
        let err = cruiser_with_steering_params(|text| {
            let before = text.len();
            *text = text
                .lines()
                .filter(|l| l.split('=').next().map(str::trim) != Some(dropped))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                text.len() < before,
                "`{dropped}` must be present to drop it"
            );
        })
        .expect_err(
            "a hull authoring part of a modifier set must be refused, not quietly \
             flown with half a lever",
        );
        assert!(
            err.contains(dropped),
            "the load error must NAME the missing scalar so the author can fix it \
             without reading the host: dropping `{dropped}` said {err}"
        );
    }
}

/// `arc_keep_speed = 0.0` is not a slow ring. It is a parked ship inside a
/// hostile's guns — the exact hazard the combat-orbit set's all-or-nothing gate
/// exists to prevent, arriving through a different door.
#[test]
fn a_zero_arc_keep_speed_is_refused_as_a_parked_ship() {
    let err = cruiser_with_steering_params(|text| {
        *text = text.replace("arc_keep_speed = 0.3", "arc_keep_speed = 0.0");
    })
    .expect_err("a zero arc-keeping throttle must be refused");
    assert!(
        err.contains("arc_keep_speed"),
        "the load error must name the scalar: {err}"
    );
}

/// An inverted deadband is refused rather than clamped.
///
/// The read site used to do `restore.max(flip)` and carry on, which is the shape
/// AGENTS.md #11 warns about: substituting the value an author appears to have
/// meant leaves a hull flying a doctrine nobody wrote down, and the file still
/// says the wrong thing.
#[test]
fn an_inverted_weak_shield_deadband_is_refused_rather_than_silently_clamped() {
    let err = cruiser_with_steering_params(|text| {
        *text = text.replace(
            "weak_shield_restore_hp = 60.0",
            "weak_shield_restore_hp = 10.0",
        );
    })
    .expect_err("a restore floor below the flip floor must be refused");
    assert!(
        err.contains("weak_shield_restore_hp"),
        "the load error must name the scalar: {err}"
    );
}
