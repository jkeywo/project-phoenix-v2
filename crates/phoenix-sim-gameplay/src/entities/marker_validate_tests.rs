use super::*;

const RIG: &str = r##"
[markers.phasers_fore]
position = [0.0, 0.0, -6.0]
direction = [0.0, 0.0, -1.0]

[markers.torpedo_port]
position = [-1.0, 0.0, -4.0]
direction = [0.0, 0.0, -1.0]

[markers.blaster_fore]
position = [0.0, 0.5, -5.0]
direction = [0.0, 0.0, -1.0]

[markers.engine_port]
position = [-1.0, 0.0, 5.0]
direction = [0.0, 0.0, 1.0]

[markers.camera_fore]
position = [0.0, 1.0, -3.0]
direction = [0.0, 0.0, -1.0]
"##;

fn rig() -> ModelRig {
    parse_model_rig(RIG).expect("fixture rig parses")
}

fn entity(body: &str) -> (String, EntityConfig) {
    let toml = format!(
        r##"
name = "Fixture"

[mesh]
model = "assets/models/fixture.glb"
shape = "cuboid"
colour = [1.0, 1.0, 1.0]
{body}
"##
    );
    let cfg = EntityConfig::from_toml_in_mode(
        &toml,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect("fixture entity parses");
    (toml, cfg)
}

const PHASER_OK: &str = r##"
[[weapons_console.phaser_banks]]
id = "fore"
facing_deg = 0.0
fire_arc_deg = 90.0
auto_arc_deg = 45.0
marker = "phasers_fore"
"##;

#[test]
fn phaser_bank_marker_resolves() {
    let (toml, cfg) = entity(PHASER_OK);
    let findings = validate_entity_markers("fixture.toml", &toml, &cfg, Some(&rig()));
    assert!(findings.is_empty(), "expected clean, got {findings:?}");
}

#[test]
fn phaser_bank_missing_marker_is_located_error() {
    let (toml, cfg) = entity(&PHASER_OK.replace("phasers_fore", "phasers_front"));
    let findings = validate_entity_markers("fixture.toml", &toml, &cfg, Some(&rig()));
    assert_eq!(findings.len(), 1, "{findings:?}");
    let f = &findings[0];
    assert!(f.is_error());
    assert_eq!(f.category, CATEGORY_MISSING);
    assert_eq!(f.source.file, "fixture.toml");
    assert_eq!(f.source.reference, "phasers_front");
    assert!(f.source.line.is_some(), "finding must be source-located");
    assert!(f.message.contains("phaser bank 'fore'"), "{}", f.message);
}

#[test]
fn blaster_bank_marker_success_and_failure() {
    let body = r##"
[[weapons_console.blaster_banks]]
id = "fore"
facing_deg = 0.0
marker = "blaster_fore"
"##;
    let (toml, cfg) = entity(body);
    assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());

    let (toml, cfg) = entity(&body.replace("blaster_fore", "blaster_nose"));
    let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, CATEGORY_MISSING);
    assert!(
        findings[0].message.contains("blaster bank 'fore'"),
        "{}",
        findings[0].message
    );
}

#[test]
fn blaster_barrel_markers_validated_per_barrel() {
    // Two authored barrels + a pattern; one barrel marker is misspelled.
    let body = r##"
[[weapons_console.blaster_banks]]
id = "twin"
facing_deg = 0.0
barrels = [ "blaster_fore", "blaster_nose" ]
[[weapons_console.blaster_banks.pattern]]
barrels = [ 0 ]
offset_secs = 0.0
[[weapons_console.blaster_banks.pattern]]
barrels = [ 1 ]
offset_secs = 0.2
"##;
    let (toml, cfg) = entity(body);
    let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
    // `blaster_fore` resolves; `blaster_nose` does not.
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].category, CATEGORY_MISSING);
    assert!(
        findings[0].message.contains("blaster bank 'twin' barrel 1"),
        "{}",
        findings[0].message
    );

    // Both barrels valid → clean.
    let (toml, cfg) = entity(&body.replace("blaster_nose", "blaster_fore"));
    assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());
}

#[test]
fn torpedo_tube_marker_success_and_failure() {
    let body = r##"
[[torpedoes.tubes]]
id = "port"
facing_deg = 0.0
fire_arc_deg = 90.0
marker = "torpedo_port"
"##;
    let (toml, cfg) = entity(body);
    assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());

    let (toml, cfg) = entity(&body.replace("torpedo_port", "torpdo_port"));
    let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, CATEGORY_MISSING);
    assert!(
        findings[0].message.contains("torpedo tube 'port'"),
        "{}",
        findings[0].message
    );
}

#[test]
fn torpedo_barrel_markers_validated_per_barrel() {
    // Two authored barrels + a pattern; one barrel marker is misspelled.
    let body = r##"
[[torpedoes.tubes]]
id = "twin"
facing_deg = 0.0
fire_arc_deg = 90.0
barrels = [ "torpedo_port", "torpedo_nose" ]
[[torpedoes.tubes.pattern]]
barrels = [ 0 ]
offset_secs = 0.0
[[torpedoes.tubes.pattern]]
barrels = [ 1 ]
offset_secs = 0.2
"##;
    let (toml, cfg) = entity(body);
    let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
    // `torpedo_port` resolves; `torpedo_nose` does not.
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].category, CATEGORY_MISSING);
    assert!(
        findings[0].message.contains("torpedo tube 'twin' barrel 1"),
        "{}",
        findings[0].message
    );

    // Both barrels valid → clean.
    let (toml, cfg) = entity(&body.replace("torpedo_nose", "torpedo_port"));
    assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());
}

#[test]
fn engine_pfx_markers_success_and_failure() {
    let body = r##"
[helm_console.engine_pfx]
markers = [ "engine_port" ]
"##;
    let (toml, cfg) = entity(body);
    assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());

    let (toml, cfg) = entity(&body.replace("engine_port", "engine_starbord"));
    let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, CATEGORY_MISSING);
    assert!(
        findings[0].message.contains("engine exhaust PFX"),
        "{}",
        findings[0].message
    );
}

#[test]
fn camera_view_success_and_failure() {
    let rig = rig();
    assert!(validate_camera_view("f.toml", "", &rig, "camera_fore").is_empty());

    // Missing camera marker.
    let findings = validate_camera_view("f.toml", "", &rig, "camera_aft");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, CATEGORY_MISSING);

    // Present, but outside the reserved camera namespace → incompatible.
    let findings = validate_camera_view("f.toml", "", &rig, "engine_port");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, CATEGORY_INCOMPATIBLE);
    assert!(findings[0].is_error());
}

#[test]
fn weapon_referencing_camera_marker_is_incompatible() {
    let (toml, cfg) = entity(&PHASER_OK.replace("phasers_fore", "camera_fore"));
    let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&rig()));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].category, CATEGORY_INCOMPATIBLE);
    assert_eq!(findings[0].source.reference, "camera_fore");
}

#[test]
fn markers_without_a_rig_are_errors() {
    let (toml, cfg) = entity(PHASER_OK);
    let findings = validate_entity_markers("f.toml", &toml, &cfg, None);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, CATEGORY_NO_RIG);
    assert!(has_error(&findings));
}

/// The default-camera warning fires for a hull a player can fly, and only
/// for one.
///
/// `[captain_console]` answered "can a player fly this?" on its own until
/// #885b stage 5c, when every AI-bearing hull — NPC designs included —
/// authored `[captain_console.ai]` and brought the section into existence.
/// The `npc` tag carries the distinction now, and both halves are asserted
/// here so re-pointing the gate cannot have quietly switched the check off:
/// the same rig, the same console, tag present ⇒ silent, tag absent ⇒ warned.
#[test]
fn the_default_camera_warning_follows_the_npc_tag_not_the_console() {
    let bare_rig = parse_model_rig(
        "[markers.engine_port]\nposition = [0.0, 0.0, 0.0]\ndirection = [0.0, 0.0, 1.0]\n",
    )
    .expect("fixture rig parses");

    let (toml, cfg) = entity("\n[captain_console]\n");
    assert!(cfg.tags.is_empty(), "precondition: not an NPC design");
    assert!(is_player_flyable(&cfg));
    let findings = validate_entity_markers("f.toml", &toml, &cfg, Some(&bare_rig));
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].category, CATEGORY_MISSING_CAMERA);
    assert!(!findings[0].is_error(), "the hull is still playable");
    // …and satisfied by a rig that declares the default viewpoint.
    assert!(validate_entity_markers("f.toml", &toml, &cfg, Some(&rig())).is_empty());

    // The same file, one tag different.
    let (npc_toml, mut npc_cfg) = entity("\n[captain_console]\n");
    npc_cfg.tags = vec!["ship".to_string(), NPC_HULL_TAG.to_string()];
    assert!(npc_cfg.captain_console.is_some(), "precondition");
    assert!(!is_player_flyable(&npc_cfg));
    assert!(
        validate_entity_markers("f.toml", &npc_toml, &npc_cfg, Some(&bare_rig)).is_empty(),
        "an NPC design's rig owes the viewscreen nothing: no bridge crew ever \
             boards it, and since #885b its `[captain_console]` exists only to hold \
             the Red Alert policy it is now required to declare"
    );
}

#[test]
fn entity_without_marker_refs_and_without_rig_is_clean() {
    let (toml, cfg) = entity("");
    assert!(validate_entity_markers("f.toml", &toml, &cfg, None).is_empty());
}

#[test]
fn duplicate_marker_declaration_is_located() {
    let sidecar = r##"
[markers.engine_port]
position = [0.0, 0.0, 0.0]
direction = [0.0, 0.0, 1.0]

[markers.engine_port]
position = [1.0, 0.0, 0.0]
direction = [0.0, 0.0, 1.0]
"##;
    let findings = duplicate_marker_findings("rig.model.toml", sidecar);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].category, CATEGORY_DUPLICATE);
    assert_eq!(findings[0].source.reference, "engine_port");
    assert_eq!(findings[0].source.line, Some(6));
    assert!(has_error(&findings));
}

#[test]
fn distinct_marker_declarations_are_clean() {
    assert!(duplicate_marker_findings("rig.model.toml", RIG).is_empty());
}

#[test]
fn role_namespace_rules() {
    assert!(MarkerRole::Camera.accepts("camera_fore"));
    assert!(!MarkerRole::Camera.accepts("phasers_fore"));
    assert!(MarkerRole::Weapon.accepts("phasers_fore"));
    assert!(!MarkerRole::Weapon.accepts("camera_fore"));
    assert!(MarkerRole::Effect.accepts("engine_port"));
    assert!(!MarkerRole::Effect.accepts("camera_aft"));
}

#[test]
fn collect_marker_refs_skips_system_marker() {
    // `[[system]] marker` is declared-but-unread; it must not produce refs.
    let toml = r##"
name = "Fixture"

[mesh]
model = "assets/models/fixture.glb"
shape = "cuboid"
colour = [1.0, 1.0, 1.0]

[[station]]
id = "Helm"
name = "Helm"
description = "Fixture station"
rank = "Cmdr."

[[system]]
id = "shields"
kind = "shields"
station = "Helm"
marker = "not_a_rig_marker"
"##;
    let cfg = EntityConfig::from_toml(toml).expect("parses");
    assert!(collect_marker_refs(&cfg).is_empty());
    assert!(validate_entity_markers("f.toml", toml, &cfg, Some(&rig())).is_empty());
}

use phoenix_content::rig::parse_model_rig;
