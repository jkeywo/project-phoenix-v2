use super::*;
use crate::world::config::parse_world;
use crate::world::load::MemoryTemplateLoader;

fn cfg(toml: &str) -> WorldConfig {
    parse_world(toml).expect("fixture parses")
}

// ── name vs display_name roles (AC1) ─────────────────────────────────────

#[test]
fn name_is_reference_display_name_is_player_text() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/station.toml"
name = "axiom_station"
display_name = "Axiom Station"
"#;
    let c = cfg(toml);
    // Reference id used by name_to_uuid / composition:
    assert_eq!(c.entities[0].name.as_deref(), Some("axiom_station"));
    // Player-facing text, independent:
    assert_eq!(c.entities[0].display_text(), "Axiom Station");
}

#[test]
fn display_text_falls_back_to_name_when_absent() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/station.toml"
name = "world.entity.axiom_station.name"
"#;
    let c = cfg(toml);
    assert_eq!(c.entities[0].display_name, None);
    // Falls back to the name reference id (a localization key) — the
    // pre-#750 conflated behaviour is preserved for existing worlds.
    assert_eq!(
        c.entities[0].display_text(),
        "world.entity.axiom_station.name"
    );
}

// ── duplicate reference name (AC2, source-located) ───────────────────────

#[test]
fn duplicate_reference_name_is_source_located_error() {
    let toml = r#"
[[entity]]
template_path = "assets/entities/a.toml"
name = "outpost"

[[entity]]
template_path = "assets/entities/b.toml"
name = "outpost"
"#;
    let c = cfg(toml);
    let findings = validate_entity_identity("root.toml", toml, &c.entities);
    assert_eq!(findings.len(), 1, "one finding per duplicated name");
    let f = &findings[0];
    assert!(f.is_error());
    assert_eq!(f.category, "duplicate-name");
    assert_eq!(f.source.file, "root.toml");
    assert_eq!(f.source.reference, "outpost");
    // Best-effort line points at the *first* occurrence of the name.
    assert_eq!(f.source.line, Some(4));
}

// ── invalid qualified reference (AC2) ────────────────────────────────────

#[test]
fn malformed_parent_qualifier_is_invalid() {
    assert_eq!(
        parse_qualified_reference("parent.", &[]),
        QualifiedRef::Invalid("parent.".to_string())
    );
    assert_eq!(
        parse_qualified_reference("parent.parent.", &[]),
        QualifiedRef::Invalid("parent.parent.".to_string())
    );
}

#[test]
fn parent_and_child_qualifiers_parse() {
    assert_eq!(
        parse_qualified_reference("parent.axiom", &[]),
        QualifiedRef::Parent {
            depth: 1,
            name: "axiom".to_string()
        }
    );
    assert_eq!(
        parse_qualified_reference("parent.parent.axiom", &[]),
        QualifiedRef::Parent {
            depth: 2,
            name: "axiom".to_string()
        }
    );
    let aliases = vec!["btf_path_a".to_string()];
    assert_eq!(
        parse_qualified_reference("btf_path_a.ironveil", &aliases),
        QualifiedRef::Child {
            alias: "btf_path_a".to_string(),
            name: "ironveil".to_string()
        }
    );
    // Localization-key names keep their dots — not a child qualifier.
    assert_eq!(
        parse_qualified_reference("world.entity.axiom.name", &aliases),
        QualifiedRef::Bare("world.entity.axiom.name".to_string())
    );
}

// ── parent.<name> resolution (AC3) ───────────────────────────────────────

// Fixtures below that assert on the ABSENCE of errors name templates that
// are really on disk (issue #973): a `template_path` that resolves nowhere
// is itself an error now, on any host whose loader is authoritative, and
// `validate_composition`'s default loader is authoritative on native.

// ── child_world.<name> resolution (AC3) ──────────────────────────────────

// ── ambiguous reference (AC2) ────────────────────────────────────────────

// ── unresolved bare reference is a non-blocking warning ──────────────────

// ── objective authoring validation (AC2, issue #752) ─────────────────────

// ── doctrine anchor references (issue #888) ──────────────────────────────

/// Entity-template source for the doctrine-anchor fixtures: a fixed
/// path → TOML map, so they never reach the filesystem or the process-wide
/// config cache. Authoritative about absence (issue #973) — the map holds
/// everything it will ever hold, the same answer the `HashMap`
/// [`FragmentSource`] gives one layer down.
fn fake_templates(entries: &[(&str, &str)]) -> MemoryTemplateLoader {
    MemoryTemplateLoader::from_toml(entries.iter().copied())
}

/// A loader that has DELIVERED its templates but is still filling — the
/// browser mid-preload, holding the hull in hand while other paths are in
/// flight (issue #973 review, F6).
///
/// The one fixture that can tell the two halves of
/// [`validate_template_resolution_in`] apart: absence is *not* final here,
/// so the presence check stays silent, but the merge check must not, since
/// a template in hand decides its own merge on any target.
fn still_filling(entries: &[(&str, &str)]) -> MemoryTemplateLoader {
    entries.iter().fold(
        MemoryTemplateLoader::still_filling(),
        |loader, (path, toml)| loader.with_toml(*path, toml),
    )
}

/// A loader that can serve nothing AND knows it cannot: the browser's
/// answer, which a native suite cannot otherwise reach (issue #973).
///
/// Used by the fixtures below that are about some *other* check and whose
/// worlds name templates no source in the test holds. Before #973 every
/// loader was implicitly blind in this way, which is why those fixtures
/// were written with paths that resolve nowhere.
fn blind_templates() -> MemoryTemplateLoader {
    MemoryTemplateLoader::blind()
}

/// The ship-level AI declarations an AI-bearing hull owes, appended to the
/// fixtures below (issue #885b stage 5d).
///
/// Strict AI-declaration mode makes a `[behaviour]` hull that omits any of
/// them fail to load, and `entity_override::apply_overrides` re-parses the
/// MERGED document strictly — so an override-carrying fixture has to be a
/// hull a real world could ship, not a doctrine snippet. Taken verbatim from
/// the shared fragment rather than restated, so it cannot drift from it.
const BASELINE_AI: &str = include_str!("../../assets/entities/fragments/ai/fleet_baseline.toml");

/// The one declaration the shared fragment deliberately leaves to its
/// includer (see that file's header).
const CAPTAIN_AI: &str = r#"
[captain_console.ai]
param = { combat_window_secs = 10.0 }

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
"#;

/// A hull that patrols two named anchors.
const PATROLLER: &str = r#"
name = "entity.patroller.name"

[behaviour]

[[behaviour.doctrine]]
id                = "patrol-route"
text              = "entity.patroller.doctrine.patrol.text"
directive_kind    = "Patrol"
directive_anchors = ["route_a", "route_b"]
directive_loop    = true
base_priority     = 20.0
"#;

/// The patroller hull's authored `[behaviour]` document, shared by
/// [`patroller_templates`] (authoritative) and the
/// `an_unmergeable_override_is_reported_even_where_absence_is_not_final`
/// test below (same content, non-final absence).
fn patroller_toml() -> String {
    format!("{PATROLLER}\n{CAPTAIN_AI}\n{BASELINE_AI}")
}

fn patroller_templates() -> MemoryTemplateLoader {
    fake_templates(&[("assets/entities/patroller.toml", &patroller_toml())])
}

#[test]
fn doctrine_anchor_declared_nowhere_is_rejected() {
    let root = cfg(r#"
[[entity]]
template_path = "assets/entities/patroller.toml"
name = "ashrender"
"#);
    let root_toml = "name = \"ashrender\"";
    let src = WorldSource::new("assets/worlds/scenario.toml", root_toml, &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());

    let errs: Vec<_> = findings
        .iter()
        .filter(|f| f.category == "unresolved-anchor")
        .collect();
    assert_eq!(
        errs.len(),
        2,
        "one error per unresolved route waypoint: {findings:?}"
    );
    assert!(
        has_error(&findings),
        "an unresolved anchor blocks the world"
    );
    for (err, anchor) in errs.iter().zip(["route_a", "route_b"]) {
        assert_eq!(err.source.reference, anchor);
        assert_eq!(err.source.file, "assets/worlds/scenario.toml");
        // The message has to name all three so an author can act on it
        // without opening the entity template.
        assert!(err.message.contains("ashrender"), "{}", err.message);
        assert!(err.message.contains(anchor), "{}", err.message);
        assert!(
            err.message.contains("assets/worlds/scenario.toml"),
            "{}",
            err.message
        );
    }
}

#[test]
fn doctrine_anchor_the_world_declares_resolves() {
    let root = cfg(r#"
[anchors]
route_a = [10.0, 0.0, 20.0]
route_b = [30.0, 0.0, 40.0]

[[entity]]
template_path = "assets/entities/patroller.toml"
name = "ashrender"
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        !has_error(&findings),
        "declared anchors resolve: {findings:?}"
    );
}

/// The case that makes the check non-trivial: a sub-world spawns a hull
/// whose route is declared by the world it layers onto (the `btf_path_*`
/// shape). A per-file linter would report two typos here.
#[test]
fn layered_world_inheriting_base_anchors_does_not_false_positive() {
    let root = cfg(r#"
[anchors]
route_a = [10.0, 0.0, 20.0]
route_b = [30.0, 0.0, 40.0]
"#);
    let child = cfg(r#"
[[entity]]
template_path = "assets/entities/patroller.toml"
name = "reinforcement"
"#);
    let root_src = WorldSource::new("assets/worlds/base.toml", "", &root);
    let child_src = WorldSource::new("assets/worlds/layer.toml", "", &child);
    let findings = validate_composition_with(&root_src, &[child_src], &patroller_templates());
    assert!(
        !has_error(&findings),
        "a layer inherits its base world's anchors: {findings:?}"
    );
}

/// The lever the shipped worlds use to say "this hull has no patrol here"
/// (`before_the_fire.toml`, `probe_artillery_standoff.toml`): the doctrine
/// read is the *effective* one, so a by-id override that stands the
/// directive down resolves clean.
#[test]
fn override_standing_the_directive_down_resolves() {
    let root = cfg(r#"
[[entity]]
template_path = "assets/entities/patroller.toml"
name = "ashrender"
overrides = { behaviour = { doctrine = [
  { id = "patrol-route", directive_kind = "None", directive_anchors = [], directive_loop = false },
] } }
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        !has_error(&findings),
        "a stood-down directive references no anchor: {findings:?}"
    );
}

/// The converse: an override that *introduces* a directive is checked too,
/// so a scenario cannot smuggle in an unresolvable anchor by override.
#[test]
fn override_introducing_a_reach_anchor_is_validated() {
    let root = cfg(r#"
[anchors]
route_a = [10.0, 0.0, 20.0]
route_b = [30.0, 0.0, 40.0]

[[entity]]
template_path = "assets/entities/patroller.toml"
name = "courier"
overrides = { behaviour = { doctrine = [
  { id = "deliver", directive_kind = "Reach", directive_anchor = "destination", base_priority = 90.0 },
] } }
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    let err = findings
        .iter()
        .find(|f| f.category == "unresolved-anchor")
        .expect("an override-introduced Reach anchor must resolve too");
    assert_eq!(err.source.reference, "destination");
    assert!(err.message.contains("Reach"), "{}", err.message);
}

/// A template the loader cannot serve must not turn into an ANCHOR
/// complaint — the validator cannot know what doctrine a template it never
/// read declares, and inventing one would send an author hunting for an
/// anchor bug that is really a path bug.
///
/// Since #973 it is not silent either: the same world now carries an
/// `unresolvable-template` error, which is the finding that actually names
/// the defect. This test pins the split, not the silence.
#[test]
fn unloadable_template_produces_no_anchor_finding() {
    let root = cfg(r#"
[[entity]]
template_path = "assets/entities/nowhere.toml"
name = "ghost"
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        !findings.iter().any(|f| f.category == "unresolved-anchor"),
        "{findings:?}"
    );
    assert!(
        findings
            .iter()
            .any(|f| f.category == "unresolvable-template" && f.is_error()),
        "the defect is reported as what it is, by the check that owns it \
         (issue #973): {findings:?}"
    );
}

// ── civilian routes (issue #1028) ────────────────────────────────────────

/// **AC1.** A route leg naming an anchor nobody declares blocks the world.
///
/// Routes go through the same gate doctrine anchors do, and for the same
/// reason: the anchor table is parsed once and never written again, so a
/// leg that misses at load misses forever and the hauler flying that lane
/// silently skips it.
#[test]
fn a_route_leg_naming_an_undeclared_anchor_is_rejected() {
    let root = cfg(r#"
[anchors]
depot_north = [10.0, 0.0, 20.0]

[[route]]
id = "depot_run"

[[route.leg]]
anchor = "depot_north"

[[route.leg]]
anchor = "depot_south"
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    let err = findings
        .iter()
        .find(|f| f.category == "unresolved-anchor")
        .expect("the undeclared leg anchor must be an error");
    assert_eq!(err.source.reference, "depot_south");
    assert!(err.message.contains("depot_run"), "{}", err.message);
    assert!(err.is_error(), "it blocks activation rather than warning");
}

/// …and a route whose every leg resolves is silent, including one whose
/// anchors are declared by a *sibling layer* rather than by its own file.
#[test]
fn a_route_resolving_against_the_composition_is_accepted() {
    let root = cfg(r#"
[anchors]
depot_north = [10.0, 0.0, 20.0]
"#);
    let layer = cfg(r#"
[[route]]
id = "depot_run"

[[route.leg]]
anchor = "depot_north"
"#);
    let root_src = WorldSource::new("assets/worlds/base.toml", "", &root);
    let layer_src = WorldSource::new("assets/worlds/layer.toml", "", &layer);
    let findings = validate_composition_with(&root_src, &[layer_src], &patroller_templates());
    assert!(
        !findings.iter().any(|f| f.category == "unresolved-anchor"),
        "a lane may cross the base world's anchors: {findings:?}"
    );
}

/// **AC1.** A civilian assigned a lane nobody declares blocks the world too
/// — otherwise it spawns, installs no directive, and holds station for the
/// whole mission with no diagnostic anywhere.
#[test]
fn a_civilian_assigned_an_undeclared_route_is_rejected() {
    let templates = fake_templates(&[(
        "assets/entities/hauler.toml",
        &format!(
            "{PATROLLER}
[civilian]
route = \"storm_detour\"
{CAPTAIN_AI}
{BASELINE_AI}"
        ),
    )]);
    let root = cfg(r#"
[anchors]
depot_north = [10.0, 0.0, 20.0]

[[route]]
id = "depot_run"

[[route.leg]]
anchor = "depot_north"

[[entity]]
template_path = "assets/entities/hauler.toml"
name = "kestrel"
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "name = \"kestrel\"", &root);
    let findings = validate_composition_with(&src, &[], &templates);
    let err = findings
        .iter()
        .find(|f| f.category == "unresolved-route")
        .expect("an unresolved civilian route assignment must be an error");
    assert_eq!(err.source.reference, "storm_detour");
    assert!(err.message.contains("kestrel"), "{}", err.message);

    // The control: the same world with the lane declared is clean.
    let mut with_lane = root.clone();
    with_lane.routes[0].id = "storm_detour".into();
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &with_lane);
    assert!(
        !validate_composition_with(&src, &[], &templates)
            .iter()
            .any(|f| f.category == "unresolved-route"),
        "a civilian pointed at a declared lane is silent"
    );
}

// ── relative_to positioning references (issue #969) ──────────────────────

/// The failure case the issue asks for: an entity positioned against a
/// reference nothing declares must FAIL VALIDATION, not spawn-loop past a
/// log line and leave the world one entity short.
#[test]
fn an_unresolvable_relative_to_blocks_activation_instead_of_dropping_the_entity() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "earth"
transform = { position = [400.0, 0.0, 400.0] }

[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "luna"
transform = { relative_to = "erth", offset = [60.0, 0.0, 30.0] }
"#);
    let toml = r#"transform = { relative_to = "erth", offset = [60.0, 0.0, 30.0] }"#;
    let src = WorldSource::new("assets/worlds/typo.toml", toml, &config);
    let findings = validate_relative_to_in(&src);
    let err = findings
        .iter()
        .find(|f| f.category == "unresolved-relative-to")
        .expect("a typo'd relative_to must be reported");
    assert!(err.is_error(), "must block activation, not warn");
    assert_eq!(err.source.reference, "erth");
    assert_eq!(
        err.source.line,
        Some(1),
        "the finding points at the authored reference"
    );
    assert!(has_error(&findings));
}

/// It reaches the composition gate, not just the helper.
#[test]
fn an_unresolvable_relative_to_is_reported_by_validate_composition() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "luna"
transform = { relative_to = "nobody", offset = [1.0, 0.0, 0.0] }
"#);
    let src = WorldSource::new("assets/worlds/typo.toml", "", &config);
    let findings = validate_composition(&src, &[]);
    assert!(
        findings
            .iter()
            .any(|f| f.category == "unresolved-relative-to" && f.is_error()),
        "{findings:?}"
    );
}

/// Both resolution directions pass validation, since the runtime table they
/// are checked against is built over the whole file before anything moves.
#[test]
fn relative_to_declared_earlier_or_later_validates_clean() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "forward-moon"
transform = { relative_to = "planet", offset = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "planet"
name = "world.entity.earth.name"
transform = { position = [100.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/moon_ice.toml"
id = "backward-moon"
transform = { relative_to = "world.entity.earth.name", offset = [0.0, 0.0, 5.0] }
"#);
    let src = WorldSource::new("assets/worlds/both.toml", "", &config);
    assert!(
        validate_relative_to_in(&src).is_empty(),
        "a reference by `id` (forward) or by `name` (backward) both resolve"
    );
}

/// A chain reads as "missing entity" to the lookup table but is a distinct
/// authoring mistake, so the message says which one it is.
#[test]
fn a_relative_to_chain_is_named_as_a_chain_not_a_missing_entity() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "planet"
transform = { position = [100.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "moon"
transform = { relative_to = "planet", offset = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/station_axiom.toml"
id = "outpost"
transform = { relative_to = "moon", offset = [0.0, 1.0, 0.0] }
"#);
    let src = WorldSource::new("assets/worlds/chain.toml", "", &config);
    let findings = validate_relative_to_in(&src);
    assert_eq!(findings.len(), 1, "only the chained entity errors");
    assert!(
        findings[0].message.contains("chains are not supported"),
        "{}",
        findings[0].message
    );
}

/// No template is loaded to decide any of this, so the check is the same on
/// a host whose template loader can see nothing — the browser included.
/// (Contrast `unloadable_template_produces_no_anchor_finding` above.)
#[test]
fn relative_to_validation_needs_no_template_loader() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/nowhere.toml"
id = "ghost"
transform = { position = [0.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/also-nowhere.toml"
id = "tag-along"
transform = { relative_to = "ghost", offset = [1.0, 0.0, 0.0] }
"#);
    let src = WorldSource::new("assets/worlds/unloadable.toml", "", &config);
    assert!(
        validate_relative_to_in(&src).is_empty(),
        "unloadable is fine"
    );
}

// ── ambiguous positioning spellings (issue #969) ─────────────────────────

/// The one shape in which admitting `id` as a positioning key can re-point a
/// reference that already worked: entity A holds `foo` as its `name`,
/// entity B later claims `foo` as its `id`. It still resolves — to A, by
/// rule — but the author who added B may well have meant B, so say so.
/// Warning, not error: nothing is broken until something references it.
#[test]
fn an_id_shadowing_another_entitys_name_warns_without_blocking() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/station_axiom.toml"
name = "beacon"
transform = { position = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "beacon"
transform = { position = [2.0, 0.0, 0.0] }
"#);
    let findings = validate_entity_identity("root.toml", "", &config.entities);
    let f = findings
        .iter()
        .find(|f| f.category == "ambiguous-entity-reference")
        .expect("the shadowed spelling must be reported");
    assert!(!f.is_error(), "ambiguity is a warning, not a block");
    assert_eq!(f.source.reference, "beacon");
    assert!(!has_error(&findings), "the world still activates");
}

/// Two entities sharing an `id` have no principled winner at all — only file
/// order — so the same warning covers them.
#[test]
fn a_duplicate_id_warns_that_a_relative_to_would_be_ambiguous() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "twin"
transform = { position = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "twin"
transform = { position = [2.0, 0.0, 0.0] }
"#);
    let findings = validate_entity_identity("root.toml", "", &config.entities);
    assert_eq!(
        findings
            .iter()
            .filter(|f| f.category == "ambiguous-entity-reference")
            .count(),
        1,
        "one spelling, one finding: {findings:?}"
    );
    assert!(!has_error(&findings));
}

/// A duplicate `name` is already an error in the reference namespace. It
/// must not also produce the positioning warning — one collision, one
/// finding, and the error is the one worth reading.
#[test]
fn a_duplicate_name_errors_once_and_is_not_also_warned_about() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
name = "outpost"
transform = { position = [1.0, 0.0, 0.0] }

[[entity]]
template_path = "assets/entities/moon_luna.toml"
name = "outpost"
transform = { position = [2.0, 0.0, 0.0] }
"#);
    let findings = validate_entity_identity("root.toml", "", &config.entities);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].category, "duplicate-name");
}

/// An entity whose `id` and `name` are the same string claims that spelling
/// once. That is one entity, and nothing about it is ambiguous.
#[test]
fn an_entity_whose_id_equals_its_own_name_is_not_ambiguous() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/planet_earth.toml"
id = "earth"
name = "earth"
transform = { position = [1.0, 0.0, 0.0] }
"#);
    assert!(
        validate_entity_identity("root.toml", "", &config.entities).is_empty(),
        "a single entity cannot collide with itself"
    );
}

// ── the shared Startup activation gate (issue #969) ──────────────────────

/// [`activation_findings`] is what both immediate-spawn systems consult, so
/// a `relative_to` failure has to reach it — not only `validate_composition`,
/// which the browser host never calls.
#[test]
fn activation_findings_carries_the_relative_to_error() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/moon_luna.toml"
id = "luna"
transform = { relative_to = "nobody", offset = [1.0, 0.0, 0.0] }
"#);
    let findings = activation_findings(
        &config,
        &crate::entities::include_resolve::HostFragmentSource,
        &crate::entities::loader::WasmTemplateLoader,
    );
    assert!(
        findings
            .iter()
            .any(|f| f.category == "unresolved-relative-to" && f.is_error()),
        "{findings:?}"
    );
    assert!(has_error(&findings));
}

// ── Template composition joins the finding flow (issue #906) ─────────────

fn fragments(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(p, t)| (p.to_string(), t.to_string()))
        .collect()
}

fn one_entity_world(template_path: &str, name: &str) -> String {
    format!("[[entity]]\ntemplate_path = \"{template_path}\"\nname = \"{name}\"\n")
}

/// The whole point of the issue: a world whose entity template cannot be
/// composed FAILS VALIDATION instead of quietly losing the entity.
#[test]
fn a_missing_fragment_blocks_activation_instead_of_dropping_the_entity() {
    let world_toml = one_entity_world("assets/entities/broken.toml", "broken_one");
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
    let source = fragments(&[(
        "assets/entities/broken.toml",
        "includes = [\"fragments/absent.toml\"]\nname = \"B\"\n",
    )]);

    let findings = validate_composition_with_fragments(&src, &[], &patroller_templates(), &source);

    assert!(
        has_error(&findings),
        "a broken include must gate activation: {findings:?}"
    );
    let f = findings
        .iter()
        .find(|f| f.category == "include-missing")
        .unwrap_or_else(|| panic!("expected an include-missing finding: {findings:?}"));
    assert_eq!(
        f.source.file, "assets/entities/broken.toml",
        "the finding names the file that DECLARED the bad include"
    );
    assert_eq!(f.source.line, Some(1));
    assert!(
        f.message.contains("broken_one") && f.message.contains("assets/worlds/w.toml"),
        "the message names the entity and the world that pulled it in: {}",
        f.message
    );
}

#[test]
fn an_include_cycle_blocks_activation() {
    let world_toml = one_entity_world("assets/entities/a.toml", "looper");
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
    let source = fragments(&[
        ("assets/entities/a.toml", "includes = [\"b.toml\"]\n"),
        ("assets/entities/b.toml", "includes = [\"a.toml\"]\n"),
    ]);

    let findings = validate_composition_with_fragments(&src, &[], &patroller_templates(), &source);
    assert!(findings.iter().any(|f| f.category == "include-cycle"));
    assert!(has_error(&findings));
}

#[test]
fn a_malformed_includes_declaration_blocks_activation() {
    let world_toml = one_entity_world("assets/entities/bad.toml", "bad_one");
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
    let source = fragments(&[("assets/entities/bad.toml", "includes = \"not-an-array\"\n")]);

    let findings = validate_composition_with_fragments(&src, &[], &patroller_templates(), &source);
    assert!(findings.iter().any(|f| f.category == "include-malformed"));
    assert!(has_error(&findings));
}

/// A hull spawned by both layers has ONE broken include, not two.
#[test]
fn one_broken_template_is_reported_once_across_the_composition() {
    let root_toml = one_entity_world("assets/entities/broken.toml", "root_one");
    let root = cfg(&root_toml);
    let child_toml = one_entity_world("assets/entities/broken.toml", "child_one");
    let child = cfg(&child_toml);
    let root_src = WorldSource::new("assets/worlds/root.toml", &root_toml, &root);
    let child_src = WorldSource::new("assets/worlds/child.toml", &child_toml, &child);
    let source = fragments(&[(
        "assets/entities/broken.toml",
        "includes = [\"fragments/absent.toml\"]\n",
    )]);

    let findings = validate_composition_with_fragments(
        &root_src,
        &[child_src],
        &patroller_templates(),
        &source,
    );
    assert_eq!(
        findings
            .iter()
            .filter(|f| f.category.starts_with("include-"))
            .count(),
        1,
        "{findings:?}"
    );
}

/// Two authored SPELLINGS of one path are still one broken hull.
///
/// Authors write `./assets/…` as readily as `assets/…`, and the resolver
/// canonicalises before it does anything, so both spellings reach the same
/// template and produce the same fault. Deduplicating on the raw string
/// would report it twice — the same duplication
/// `one_broken_template_is_reported_once_across_the_composition` rules out
/// for two worlds, arriving instead through the back door of punctuation.
#[test]
fn two_spellings_of_one_template_are_reported_once() {
    let world_toml = format!(
        "{}{}",
        one_entity_world("assets/entities/broken.toml", "plain"),
        one_entity_world("./assets/entities/broken.toml", "dotted"),
    );
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
    let source = fragments(&[(
        "assets/entities/broken.toml",
        "includes = [\"fragments/absent.toml\"]\n",
    )]);

    let findings = validate_composition_with_fragments(&src, &[], &patroller_templates(), &source);
    assert_eq!(
        findings
            .iter()
            .filter(|f| f.category.starts_with("include-"))
            .count(),
        1,
        "one broken hull, spelled two ways, is one finding: {findings:?}"
    );
}

/// A template the fragment source cannot see is NOT a composition error —
/// a validator must not manufacture one out of its own blindness.
///
/// The template loader is [`blind_templates`] rather than the fixture map
/// so the claim stays about *composition*: an authoritative loader would
/// (correctly, since #973) report the same world as
/// `unresolvable-template`, and this test would then be asserting two
/// things at once.
#[test]
fn a_template_the_source_cannot_see_produces_no_composition_finding() {
    let world_toml = one_entity_world("assets/entities/nowhere.toml", "ghost");
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
    let source = fragments(&[]);

    let findings = validate_composition_with_fragments(&src, &[], &blind_templates(), &source);
    assert!(!findings.iter().any(|f| f.category.starts_with("include-")));
    assert!(!has_error(&findings), "{findings:?}");
}

#[test]
fn a_template_that_composes_cleanly_produces_no_composition_finding() {
    let world_toml = one_entity_world("assets/entities/good.toml", "good_one");
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
    let source = fragments(&[
        ("assets/entities/hull_core.toml", "class = \"escort\"\n"),
        (
            "assets/entities/good.toml",
            "includes = [\"hull_core.toml\"]\nname = \"G\"\n",
        ),
    ]);

    // Blind loader for the same reason as the test above: `good.toml` is a
    // fragment-source fixture, not a template the loader holds.
    let findings = validate_composition_with_fragments(&src, &[], &blind_templates(), &source);
    assert!(!findings.iter().any(|f| f.category.starts_with("include-")));
    assert!(!has_error(&findings), "{findings:?}");
}

// ── Template presence (issue #973) ───────────────────────────────────────

/// The defect the issue is about: a world naming a template that does not
/// resolve used to load "successfully" and simply spawn nothing, because
/// every spawn caller logs the resolve failure and `continue`s. On a host
/// whose loader is authoritative it now FAILS VALIDATION, loudly, before
/// anything spawns.
#[test]
fn an_unresolvable_template_blocks_activation_on_an_authoritative_host() {
    let world_toml = one_entity_world("assets/entities/nowhere.toml", "ghost");
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

    // `patroller_templates()` holds one template and knows it holds
    // everything it will ever hold — a fixture map, like the filesystem, is
    // authoritative about absence.
    let templates = patroller_templates();
    assert!(templates.absence_is_final(), "precondition");

    let findings = validate_composition_with(&src, &[], &templates);
    let err = findings
        .iter()
        .find(|f| f.category == "unresolvable-template")
        .unwrap_or_else(|| panic!("expected an unresolvable-template finding: {findings:?}"));
    assert!(err.is_error(), "it must block activation, not warn");
    assert!(has_error(&findings));
    assert_eq!(err.source.reference, "assets/entities/nowhere.toml");
    assert_eq!(err.source.file, "assets/worlds/w.toml");
    assert_eq!(
        err.source.line,
        Some(2),
        "the finding points at the authored template_path"
    );
    // The message has to name the entity and the world, so an author can
    // act on it without opening every world file.
    assert!(err.message.contains("ghost"), "{}", err.message);
    assert!(
        err.message.contains("assets/entities/nowhere.toml"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains("assets/worlds/w.toml"),
        "{}",
        err.message
    );
}

/// The other half, and the half that keeps the shipped client working: a
/// host that cannot tell "absent" from "not yet" reports NOTHING.
///
/// This is the browser. Its preloaded config cache fills one delivery at a
/// time while the runtime layer load spawns the moment a layer's TOML
/// arrives, so a template the loader cannot serve at validation time
/// routinely resolves fine a moment later. Erroring there would blank the
/// whole world, permanently — the layer is marked loaded and never retried.
#[test]
fn an_unresolvable_template_is_silent_on_a_host_that_cannot_be_authoritative() {
    let world_toml = one_entity_world("assets/entities/nowhere.toml", "ghost");
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

    assert!(!blind_templates().absence_is_final(), "precondition");

    let findings = validate_composition_with(&src, &[], &blind_templates());
    assert!(
        !findings
            .iter()
            .any(|f| f.category == "unresolvable-template"),
        "a host that cannot see the template must not manufacture an error \
         out of its own blindness: {findings:?}"
    );
    assert!(!has_error(&findings), "{findings:?}");
}

/// One hull, spelled two ways, is one finding — the same canonicalisation
/// `two_spellings_of_one_template_are_reported_once` pins for composition.
#[test]
fn two_spellings_of_one_absent_template_are_reported_once() {
    let world_toml = format!(
        "{}{}",
        one_entity_world("assets/entities/nowhere.toml", "plain"),
        one_entity_world("./assets/entities/nowhere.toml", "dotted"),
    );
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert_eq!(
        findings
            .iter()
            .filter(|f| f.category == "unresolvable-template")
            .count(),
        1,
        "{findings:?}"
    );
}

/// The gate the Bevy `Startup` spawn consults must carry it too, not only
/// `validate_composition` — the browser host never calls the latter, and
/// on native the two immediate-spawn systems are what actually drop the
/// entity. (Same reasoning as
/// `activation_findings_carries_the_relative_to_error`.)
#[test]
fn activation_findings_carries_the_unresolvable_template_error() {
    let config = cfg(r#"
[[entity]]
template_path = "assets/entities/nowhere.toml"
name = "ghost"
"#);
    let findings = activation_findings(
        &config,
        &crate::entities::include_resolve::HostFragmentSource,
        &patroller_templates(),
    );
    assert!(
        findings
            .iter()
            .any(|f| f.category == "unresolvable-template" && f.is_error()),
        "{findings:?}"
    );

    // …and stays silent through the same gate on a blind host.
    let findings = activation_findings(
        &config,
        &crate::entities::include_resolve::HostFragmentSource,
        &blind_templates(),
    );
    assert!(
        !findings
            .iter()
            .any(|f| f.category == "unresolvable-template"),
        "{findings:?}"
    );
}

// ── Override resolution (issue #973 review, F6) ──────────────────────────

/// A world file for one entity on the patroller hull, carrying `overrides`.
fn overridden_entity_world(name: &str, overrides: &str) -> String {
    format!(
        "[[entity]]\ntemplate_path = \"assets/entities/patroller.toml\"\n\
         name = \"{name}\"\noverrides = {overrides}\n"
    )
}

/// The other half of the silent drop #973 is about, and the half its first
/// pass missed. `resolve_entity_via` returns `Err` for a failed
/// `apply_overrides` exactly as it does for a template it cannot find, and
/// every spawn caller answers `Err` the same way: log and `continue`. So an
/// `overrides` table that does not merge cost precisely one entity, with
/// the rest of the world spawning around the hole.
///
/// The `_remove` tombstone is issue #911's case: subtractive at a layer
/// that does not honour subtraction, so `reject_unhonoured_removals`
/// refuses it outright rather than letting it look like it worked.
#[test]
fn an_override_that_does_not_merge_blocks_activation() {
    let world_toml = overridden_entity_world(
        "ghost",
        "{ behaviour = { doctrine = [{ id = \"patrol\", _remove = true }] } }",
    );
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

    let findings = validate_composition_with(&src, &[], &patroller_templates());
    let err = findings
        .iter()
        .find(|f| f.category == "unmergeable-override")
        .unwrap_or_else(|| panic!("expected an unmergeable-override finding: {findings:?}"));
    assert!(err.is_error(), "it must block activation, not warn");
    assert!(has_error(&findings));
    assert!(err.message.contains("ghost"), "{}", err.message);
    assert!(
        err.message.contains("assets/entities/patroller.toml"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains("assets/worlds/w.toml"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains("_remove"),
        "the merge's own reason has to reach the author: {}",
        err.message
    );
}

/// The other failure shape of the same call: the merged document is
/// well-formed TOML but no longer a valid `EntityConfig`, so the strict
/// re-parse in `apply_overrides` refuses it.
#[test]
fn an_override_that_breaks_the_merged_documents_types_blocks_activation() {
    let world_toml = overridden_entity_world("ghost", "{ tags = \"not-an-array\" }");
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        findings
            .iter()
            .any(|f| f.category == "unmergeable-override" && f.is_error()),
        "{findings:?}"
    );
}

/// **Not** host-gated, unlike the presence half — and this is the test that
/// says so.
///
/// A host that cannot be authoritative about absence can still be
/// authoritative about a merge: once the template is in hand, no later
/// delivery can change the answer, because the template a delivery would
/// bring is the one already merged against. Gating this on
/// `absence_is_final` would leave the browser running the exact silent drop
/// the validator exists to refuse.
#[test]
fn an_unmergeable_override_is_reported_even_where_absence_is_not_final() {
    let world_toml = overridden_entity_world(
        "ghost",
        "{ behaviour = { doctrine = [{ id = \"patrol\", _remove = true }] } }",
    );
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

    let loader = still_filling(&[("assets/entities/patroller.toml", &patroller_toml())]);
    assert!(!loader.absence_is_final(), "precondition");
    assert!(
        loader
            .load_template("assets/entities/patroller.toml")
            .is_some(),
        "precondition: this host HAS the hull, it is merely still filling"
    );

    let findings = validate_composition_with(&src, &[], &loader);
    assert!(
        findings
            .iter()
            .any(|f| f.category == "unmergeable-override" && f.is_error()),
        "a merge decided entirely from content in hand must be decided: {findings:?}"
    );
    // …while the presence half stays silent on the same host, which is what
    // keeps the browser's world from being blanked by a template in flight.
    let absent = one_entity_world("assets/entities/nowhere.toml", "not-yet");
    let absent_cfg = cfg(&absent);
    let absent_src = WorldSource::new("assets/worlds/w.toml", &absent, &absent_cfg);
    let findings = validate_composition_with(&absent_src, &[], &loader);
    assert!(
        !findings
            .iter()
            .any(|f| f.category == "unresolvable-template"),
        "{findings:?}"
    );
}

/// A failed merge is a property of the INSTANCE, not the hull, so two
/// entries on one template each get their own finding — the deliberate
/// difference from `unresolvable-template`, which is deduped by canonical
/// path (`two_spellings_of_one_absent_template_are_reported_once`).
#[test]
fn two_instances_of_one_hull_each_report_their_own_broken_override() {
    let world_toml = format!(
        "{}{}",
        overridden_entity_world("first", "{ tags = \"not-an-array\" }"),
        overridden_entity_world("second", "{ tags = 7 }"),
    );
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert_eq!(
        findings
            .iter()
            .filter(|f| f.category == "unmergeable-override")
            .count(),
        2,
        "one finding per instance, because each carries its own table: {findings:?}"
    );
}

/// The Bevy `Startup` gate carries the merge half too, for the same reason
/// it carries the presence half: the browser never calls
/// `validate_composition`, and on native the two immediate-spawn systems
/// are what actually drop the entity.
#[test]
fn activation_findings_carries_the_unmergeable_override_error() {
    let config = cfg(&overridden_entity_world(
        "ghost",
        "{ behaviour = { doctrine = [{ id = \"patrol\", _remove = true }] } }",
    ));
    let findings = activation_findings(
        &config,
        &crate::entities::include_resolve::HostFragmentSource,
        &patroller_templates(),
    );
    assert!(
        findings
            .iter()
            .any(|f| f.category == "unmergeable-override" && f.is_error()),
        "{findings:?}"
    );
}

// ── Override-absent-table warning (issue #1043) ──────────────────────────

/// The #1043 foot-gun itself: an override names a top-level table
/// (`[civilian]`) the resolved template does not declare at all —
/// `patroller.toml` has no `[civilian]` anywhere in its fixture TOML. The
/// merge still succeeds (it inserts the table fresh), so this is NOT an
/// `unmergeable-override`; it is the quieter defect that survives a clean
/// merge, and is why this check exists at all.
#[test]
fn override_targeting_an_absent_template_table_warns() {
    let world_toml = overridden_entity_world("ghost", "{ civilian = { route = \"lane-a\" } }");
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

    let findings = validate_composition_with(&src, &[], &patroller_templates());
    let warning = findings
        .iter()
        .find(|f| f.category == "override-absent-table")
        .unwrap_or_else(|| panic!("expected an override-absent-table finding: {findings:?}"));
    assert!(!warning.is_error(), "the foot-gun is silent, not blocking");
    assert!(warning.message.contains("ghost"), "{}", warning.message);
    assert!(warning.message.contains("civilian"), "{}", warning.message);
    assert!(
        warning.message.contains("assets/entities/patroller.toml"),
        "{}",
        warning.message
    );
}

/// The other side of the same fixture: `patroller.toml` DOES declare
/// `[behaviour]` (it is a patrolling hull — see the `PATROLLER` const), so
/// an override that adds a second doctrine entry merges into a real table
/// and must not warn.
///
/// Declares `[anchors] route_a / route_b` — the same pair `PATROLLER`'s own
/// baked-in `patrol-route` doctrine already names — so this world is clean
/// under `validate_doctrine_anchors_in` too and the "no errors" assertion
/// below is not contaminated by a pre-existing, unrelated anchor gap in the
/// fixture.
#[test]
fn override_targeting_a_declared_template_table_does_not_warn() {
    let world_toml = format!(
        "{}\n[anchors]\nroute_a = [10.0, 0.0, 20.0]\nroute_b = [30.0, 0.0, 40.0]\n",
        overridden_entity_world(
            "ghost",
            "{ behaviour = { doctrine = [{ id = \"reinforce\", text = \"x\", \
             directive_kind = \"Reach\", directive_anchor = \"route_a\" }] } }",
        )
    );
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        !findings
            .iter()
            .any(|f| f.category == "override-absent-table"),
        "'[behaviour]' is declared by the template, so this must not warn: {findings:?}"
    );
    assert!(
        !findings.iter().any(WorldFinding::is_error),
        "and this override is well-formed, so nothing should block activation either: \
         {findings:?}"
    );
}

/// A warning is exactly that: the world still activates. The deliberate
/// contrast with `an_override_that_does_not_merge_blocks_activation`, which
/// is the same fixture shape one severity up.
///
/// The override is an EMPTY `[civilian]` table — no `route` — so this stays
/// clean under `validate_civilian_routes_in` too (an authored route id that
/// resolves nowhere is its own, separate, unresolved-route error, not this
/// check's concern). `[anchors] route_a / route_b` is declared for the same
/// reason as the sibling "does not warn" test: `PATROLLER`'s own baked-in
/// doctrine names them regardless of what this test overrides.
#[test]
fn override_absent_table_warning_does_not_block_activation() {
    let world_toml = format!(
        "{}\n[anchors]\nroute_a = [10.0, 0.0, 20.0]\nroute_b = [30.0, 0.0, 40.0]\n",
        overridden_entity_world("ghost", "{ civilian = {} }")
    );
    let config = cfg(&world_toml);
    let src = WorldSource::new("assets/worlds/w.toml", &world_toml, &config);

    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        findings
            .iter()
            .any(|f| f.category == "override-absent-table"),
        "precondition: the warning itself must still be present: {findings:?}"
    );
    assert!(
        !has_error(&findings),
        "a warning must never block activation: {findings:?}"
    );
}

/// The Bevy `Startup` gate carries the warning too, for the same reason it
/// carries every other half of this function's findings: the browser never
/// calls `validate_composition`.
#[test]
fn activation_findings_carries_the_override_absent_table_warning() {
    let config = cfg(&overridden_entity_world(
        "ghost",
        "{ civilian = { route = \"lane-a\" } }",
    ));
    let findings = activation_findings(
        &config,
        &crate::entities::include_resolve::HostFragmentSource,
        &patroller_templates(),
    );
    assert!(
        findings
            .iter()
            .any(|f| f.category == "override-absent-table" && !f.is_error()),
        "{findings:?}"
    );
}

/// `probe_evidence.toml` and `probe_corroborate.toml` author a
/// `comms_console.ai.rule` override — live today via `player_hull_config`
/// (`src/server_app.rs`) — that must merge onto a table already declared by
/// the shared AI fragment library, not an absent one (issue #1036/#1043). A
/// `override-absent-table` warning for either world means that fix
/// regressed. The general `override-absent-table` case (an authoring slip
/// shaped like the hauler `[behaviour]` #1043 found by hand) is surfaced to
/// the world author at load time by `validate_template_resolution_in`
/// itself, not pinned here.
#[test]
fn evidence_and_corroborate_overrides_never_warn_override_absent_table() {
    let loader = crate::entities::loader::WasmTemplateLoader;
    for name in ["probe_evidence.toml", "probe_corroborate.toml"] {
        let path_str = format!("assets/worlds/{name}");
        let toml = std::fs::read_to_string(&path_str).expect("shipped world readable");
        let config = parse_world(&toml).expect("shipped world parses");
        let src = WorldSource::new(&path_str, &toml, &config);
        let mut seen = HashSet::new();
        let findings = validate_template_resolution_in(&src, &loader, &mut seen);
        assert!(
            !findings
                .iter()
                .any(|f| f.category == "override-absent-table"),
            "{path_str}'s `comms_console.ai.rule` override must merge onto a declared \
             table (issue #1036/#1043) — a warning here means the player-ship fix regressed: \
             {findings:?}"
        );
    }
}

// ── Script-authored spawns reach the composition gate (issue #1046) ──────
//
// `collect_action_lists` has been vacuous since #985 — the declarative
// `[[trigger.action]]` / `[[comms.response.action]]` arrays it walked no longer
// parse — so before this the gate saw `[[entity]]` blocks and nothing else, and
// every hull a shipped world spawns from script was unvalidated. These pin the
// cases that matter: a path that does not resolve, a doctrine anchor nothing
// declares, and the two shapes that must NOT be errored on (a computed path,
// and a template judged through an override the validator cannot read).

/// A `[script]` body whose `spawn_entity` names a template that does not exist
/// fails the composition gate, exactly as an `[[entity]]` block would.
#[test]
fn script_spawn_of_a_missing_template_is_rejected() {
    let root = cfg(r#"
[script]
waves = """
fn release(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/nowhere.toml",
        name: "wave_1", position: [0, 0, 0]
    });
}
"""
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    let err = findings
        .iter()
        .find(|f| f.category == "unresolvable-template")
        .unwrap_or_else(|| panic!("a scripted spawn's template must resolve: {findings:?}"));
    assert!(err.is_error());
    assert_eq!(err.source.reference, "assets/entities/nowhere.toml");
    assert!(
        has_error(&findings),
        "an unresolvable scripted template blocks the world"
    );
}

/// …and its doctrine is anchor-checked the same way too — issue #888's guard
/// reaching a script-spawned hull for the first time.
///
/// No `overrides` key in the spawn map, so the template's doctrine IS the
/// effective doctrine and the finding is an ERROR: the same severity the
/// declarative twin (`doctrine_anchor_declared_nowhere_is_rejected`) gets.
#[test]
fn script_spawn_with_an_undeclared_doctrine_anchor_is_rejected() {
    let root = cfg(r#"
[script]
waves = """
fn release(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/patroller.toml",
        name: "ashrender", position: [0, 0, 0]
    });
}
"""
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());

    let errs: Vec<_> = findings
        .iter()
        .filter(|f| f.category == "unresolved-anchor" && f.is_error())
        .collect();
    assert_eq!(
        errs.len(),
        2,
        "one error per unresolved route waypoint, as for a declarative spawn: {findings:?}"
    );
    for (err, anchor) in errs.iter().zip(["route_a", "route_b"]) {
        assert_eq!(err.source.reference, *anchor);
    }
    assert!(has_error(&findings));
}

/// Declaring the anchors clears it, which is what makes the check above a
/// statement about the WORLD rather than about the template.
#[test]
fn script_spawn_resolves_once_the_world_declares_the_anchors() {
    let root = cfg(r#"
[anchors]
route_a = [10.0, 0.0, 20.0]
route_b = [30.0, 0.0, 40.0]

[script]
waves = """
fn release(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/patroller.toml",
        name: "ashrender", position: [0, 0, 0]
    });
}
"""
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(!has_error(&findings), "{findings:?}");
}

/// **The shipped idiom must not be blocked.** A spawn that passes an
/// `overrides` map may be standing the offending doctrine entry down inside it
/// — `combat_test.toml` and `probe_artillery_standoff.toml` both do exactly
/// that, and both document it — and this validator cannot read a Rhai map built
/// at call time. The finding softens to a warning rather than being dropped:
/// the other half of the time it is a real unresolved anchor, and nothing else
/// reports it at all.
#[test]
fn script_spawn_passing_overrides_warns_instead_of_blocking() {
    let root = cfg(r#"
[script]
waves = """
fn release(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/patroller.toml",
        name: "ashrender", position: [0, 0, 0],
        overrides: stand_the_patrol_down()
    });
}
"""
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        !has_error(&findings),
        "an override this pass cannot read must not block the world: {findings:?}"
    );
    let warns: Vec<_> = findings
        .iter()
        .filter(|f| f.category == "unresolved-anchor")
        .collect();
    assert_eq!(
        warns.len(),
        2,
        "still reported, just not fatally: {findings:?}"
    );
    assert!(warns.iter().all(|f| !f.is_error()));
    assert!(
        warns[0].message.contains("overrides"),
        "the message has to say WHY it is only a warning: {}",
        warns[0].message
    );
}

/// A computed `template_path` is invisible and stays legal — `duel.toml` ships
/// one, because its hull comes from `--side-a`/`--side-b` at run time. The gate
/// must find nothing to say about it rather than guessing at a name.
#[test]
fn a_computed_script_template_path_is_not_guessed_at() {
    let root = cfg(r#"
[script]
arena = """
fn spawn_slot(ctx, name, template) {
    ctx.effects.spawn_entity(#{ template_path: template, name: name, anchor: name });
}
"""
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        findings.is_empty(),
        "a path this pass cannot compute must produce no finding at all: {findings:?}"
    );
}

/// The scan reads CODE, not prose: a `template_path` inside a `//` comment is
/// not a spawn.
#[test]
fn script_spawn_scan_ignores_comments() {
    let root = cfg(r#"
[script]
notes = """
// A wave spawns with template_path: "assets/entities/nowhere.toml" one day.
fn release(ctx) { ctx.effects.log("nothing here"); }
"""
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(findings.is_empty(), "{findings:?}");
}

/// Every shipped world's composition still validates — issue #1046's own
/// shipped-world impact statement, kept as a test.
///
/// This is the assertion the change is really made of. The gate went from
/// seeing `[[entity]]` blocks alone to seeing 41 script-spawned template
/// references across 15 of the 43 shipped worlds, and no shipped world may
/// break on the way. (Not 16: `duel.toml` authors a `[script]` block and
/// contributes ZERO references, because every slot in it reaches one
/// `spawn_slot` body whose map reads `template_path: template` — see
/// `headless::duel`.) The two worlds carrying an unresolved anchor behind an
/// override that may restate doctrine (`combat_test`,
/// `probe_artillery_standoff`) are warnings by design — see
/// `collect_spawned_instances` — so this asserts on ERRORS.
///
/// The second assertion is the anti-vacuity one, and it is the more important
/// of the two: every check here is built on the scan finding something, and a
/// scan that silently stopped seeing inside scripts would leave the whole suite
/// above passing over nothing.
#[test]
fn every_shipped_world_still_composes() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/worlds");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("assets/worlds must be readable")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    paths.sort();
    assert!(paths.len() > 20, "the world set did not load");

    let mut scripted_seen = 0usize;
    let mut broken: Vec<String> = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path).expect("world reads");
        // A world that does not PARSE is a broken world, and skipping it here
        // would let the sweep pass over the loudest possible failure.
        let config = parse_world(&text)
            .unwrap_or_else(|e| panic!("shipped world {} must parse: {e}", path.display()));
        let rel = format!(
            "assets/worlds/{}",
            path.file_name().expect("file").to_string_lossy()
        );
        scripted_seen += crate::world::config::script_spawned_templates(&config).len();
        let src = WorldSource::new(&rel, &text, &config);
        for f in validate_composition(&src, &[]) {
            if f.is_error() {
                broken.push(format!("{rel}: [{}] {}", f.category, f.message));
            }
        }
    }
    assert!(
        broken.is_empty(),
        "shipped worlds must still compose:\n{}",
        broken.join("\n")
    );
    assert!(
        scripted_seen > 30,
        "the scan found only {scripted_seen} scripted spawns across the shipped \
         worlds — it has stopped seeing inside scripts, and every check built on \
         it is passing vacuously"
    );
}

// ── The scan is CALL-SCOPED, and these are the inputs that proved it had to
// be (issue #1046, review round) ─────────────────────────────────────────────
//
// The first cut scanned the whole body for `template_path` and then asked
// whether the enclosing `{ … }` mentioned `overrides`. Every case below was a
// FALSE POSITIVE under that reading — and for a gate that refuses activation
// on native, a false positive is the expensive direction: it blanks both
// immediate-spawn halves of a world that is not actually broken.

/// A block-commented-out wave is not a spawn. `/* … */` was never stripped at
/// all, so its `template_path` queued a preload fetch AND reached this gate —
/// on native, an ERROR that aborts the boot over a hull the author had
/// deliberately taken out.
#[test]
fn a_block_commented_spawn_is_not_a_spawn() {
    let root = cfg(r#"
[script]
waves = """
/*
fn release(ctx) {
    ctx.effects.spawn_entity(#{ template_path: "assets/entities/nowhere.toml", name: "w" });
}
*/
fn live(ctx) { ctx.effects.log("nothing"); }
"""
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    assert!(
        crate::world::config::script_spawned_templates(&root).is_empty(),
        "a commented-out wave must not be scanned as a spawn"
    );
    assert!(
        validate_composition_with(&src, &[], &patroller_templates()).is_empty(),
        "…and must produce no finding"
    );
}

/// An apostrophe in prose must not desynchronise the matcher for the rest of
/// the file. Comments and strings are stripped in ONE pass precisely because
/// neither is decidable alone; done separately, the odd quote below opened a
/// string that swallowed the real spawn after it.
#[test]
fn an_odd_quote_in_a_comment_does_not_desync_the_scan() {
    let root = cfg(r#"
[script]
waves = """
// the cruiser's patrol route isn't staged here
fn release(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/patroller.toml", name: "ashrender"
    });
}
"""
"#);
    let refs = crate::world::config::script_spawned_templates(&root);
    assert_eq!(
        refs.len(),
        1,
        "the spawn after the apostrophe is still visible: {refs:?}"
    );
    assert_eq!(refs[0].template_path, "assets/entities/patroller.toml");
    assert_eq!(refs[0].overrides, None);
}

/// `let template_path = "…"` is an assignment, not a spawn map. The old scan
/// accepted `=` as a separator (a leftover from the TOML shape that predates
/// issue #985) and read this as a spawn of a hull the world never spawns.
#[test]
fn an_assignment_is_not_a_spawn_map() {
    let root = cfg(r#"
[script]
waves = """
fn pick(ctx) {
    let template_path = "assets/entities/nowhere.toml";
    ctx.flags.chosen = 1;
}
"""
"#);
    assert!(
        crate::world::config::script_spawned_templates(&root).is_empty(),
        "an assignment is not a spawn"
    );
}

/// A DATA map that happens to carry a `template_path` key is not a spawn map
/// either — this is the shape a table-driven refactor of `combat_test`'s waves
/// would take, and the old scan read every row of it as a spawn.
#[test]
fn a_nested_data_map_is_not_a_spawn_map() {
    let root = cfg(r#"
[script]
waves = """
fn table() {
    #{ w1: #{ template_path: "assets/entities/nowhere.toml", count: 2 },
       w2: #{ template_path: "assets/entities/patroller.toml", count: 1 } }
}
"""
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    assert!(
        crate::world::config::script_spawned_templates(&root).is_empty(),
        "a data table is not a roster of spawns"
    );
    assert!(validate_composition_with(&src, &[], &patroller_templates()).is_empty());
}

/// …and the same scoping in the other direction: a sibling sub-map's
/// `overrides` key must not silence the doctrine gate for the spawn ABOVE it.
/// Under the old reading this spawn came out as a WARNING; it is an ERROR,
/// because nothing here overrides anything.
#[test]
fn a_sibling_sub_maps_overrides_does_not_silence_the_spawn() {
    let root = cfg(r#"
[script]
waves = """
fn release(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/patroller.toml",
        name: "ashrender",
        extras: #{ overrides: 1 }
    });
}
"""
"#);
    let refs = crate::world::config::script_spawned_templates(&root);
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].overrides, None,
        "`overrides` inside a nested map is not this map's override"
    );

    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        findings
            .iter()
            .any(|f| f.category == "unresolved-anchor" && f.is_error()),
        "the anchor gate must be at full strength here: {findings:?}"
    );
}

/// An override that is legible end to end AND provably never mentions
/// `doctrine` cannot have stood a doctrine entry down, so the anchor gate stays
/// at ERROR (issue #1046 review round). This is what makes #888 reach the
/// script surface rather than softening away on the mere PRESENCE of an
/// override.
#[test]
fn an_override_that_cannot_touch_doctrine_keeps_the_gate_at_full_strength() {
    let root = cfg(r#"
[script]
waves = """
fn release(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/patroller.toml",
        name: "ashrender",
        overrides: #{ faction: "cccccccc-3333-4333-8333-cccccccccccc" }
    });
}
"""
"#);
    let refs = crate::world::config::script_spawned_templates(&root);
    assert_eq!(
        refs[0].overrides,
        Some(crate::world::config::OverrideShape::ReadableWithoutDoctrine)
    );

    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        findings
            .iter()
            .any(|f| f.category == "unresolved-anchor" && f.is_error()),
        "a faction-only override cannot stand a Patrol down: {findings:?}"
    );
}

/// …and an override that DOES restate doctrine softens it again, whether the
/// map is literal or (as in `combat_test.toml`) hidden behind a helper call.
/// Both shapes are unreadable in the way that matters.
#[test]
fn an_override_that_may_restate_doctrine_softens_the_gate() {
    for value in [
        "#{ behaviour: #{ doctrine: [] } }",
        "wave_8_overrides()",
        "cfg",
    ] {
        let root = cfg(&format!(
            r#"
[script]
waves = """
fn release(ctx) {{
    ctx.effects.spawn_entity(#{{
        template_path: "assets/entities/patroller.toml",
        name: "ashrender",
        overrides: {value}
    }});
}}
"""
"#
        ));
        let refs = crate::world::config::script_spawned_templates(&root);
        assert_eq!(
            refs[0].overrides,
            Some(crate::world::config::OverrideShape::MayRestateDoctrine),
            "`{value}` may restate doctrine"
        );

        let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
        let findings = validate_composition_with(&src, &[], &patroller_templates());
        assert!(!has_error(&findings), "`{value}`: {findings:?}");
        assert!(
            findings
                .iter()
                .any(|f| f.category == "unresolved-anchor" && !f.is_error()),
            "`{value}` still reports, as a warning: {findings:?}"
        );
    }
}

/// An empty override is legible and mentions no doctrine, so it is the
/// full-strength arm — the boundary case of the rule above.
#[test]
fn an_empty_override_keeps_the_gate_at_full_strength() {
    let root = cfg(r#"
[script]
waves = """
fn release(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/patroller.toml", name: "a", overrides: #{}
    });
}
"""
"#);
    let refs = crate::world::config::script_spawned_templates(&root);
    assert_eq!(
        refs[0].overrides,
        Some(crate::world::config::OverrideShape::ReadableWithoutDoctrine)
    );
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    assert!(has_error(&validate_composition_with(
        &src,
        &[],
        &patroller_templates()
    )));
}

/// A bare spawn's ERROR must not be eaten by an overridden spawn's WARNING.
/// Script instances are labelled by template path, so both spawns below share a
/// dedup key on (label, anchor) alone — and whichever was walked first silenced
/// the other.
#[test]
fn an_overridden_spawn_does_not_swallow_a_bare_spawns_error() {
    let root = cfg(r#"
[script]
waves = """
fn release(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/patroller.toml", name: "a",
        overrides: #{ behaviour: #{ doctrine: [] } }
    });
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/patroller.toml", name: "b"
    });
}
"""
"#);
    let src = WorldSource::new("assets/worlds/scenario.toml", "", &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    assert!(
        has_error(&findings),
        "the bare spawn's error survives the overridden spawn's warning: {findings:?}"
    );
}

/// A finding about a script spawn points at the SPAWN, not at the first place
/// the template path happens to appear in the world file.
#[test]
fn a_script_finding_points_at_the_spawn_call() {
    let toml = r#"[anchors]
route_a = [1.0, 0.0, 2.0]

[script]
waves = """
fn release(ctx) {
    ctx.effects.spawn_entity(#{
        template_path: "assets/entities/patroller.toml", name: "ashrender"
    });
}
"""
"#;
    let root = cfg(toml);
    let src = WorldSource::new("assets/worlds/scenario.toml", toml, &root);
    let findings = validate_composition_with(&src, &[], &patroller_templates());
    let err = findings
        .iter()
        .find(|f| f.category == "unresolved-anchor")
        .expect("route_b is undeclared");
    // Line 7 is `ctx.effects.spawn_entity(#{`, counting the world TOML from 1.
    assert_eq!(
        err.source.line,
        Some(7),
        "the finding must name the spawn call: {err:?}"
    );
}
