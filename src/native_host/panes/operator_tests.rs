#[test]
fn authoring_generations_keep_vocabulary_and_migration_orders_distinct() {
    assert_eq!(workshop_panels_for(1), workshop_panels_for(2));
    assert_eq!(
        workshop_panels_for(3),
        [
            "files",
            "source",
            "inspector",
            "add",
            "recovery",
            "findings",
            "feedback",
            "dependencies",
            "settings"
        ]
    );
    assert_eq!(
        &workshop_panels_added_after(2)[..4],
        &[
            ("dependencies", "files"),
            ("findings", "source"),
            ("feedback", "source"),
            ("settings", "inspector")
        ]
    );
    for version in 2..=10 {
        let vocabulary = workshop_panels_for(version);
        let added = workshop_panels_added_after(version);
        assert!(added.iter().all(|(panel, _)| !vocabulary.contains(panel)));
        assert_eq!(
            vocabulary.len() + added.len(),
            workshop_panels_for(10).len()
        );
    }
    assert!(workshop_panels_added_after(10).is_empty());
    assert!(!workshop_panels_for(10).contains(&"localisation"));
}
use super::*;
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("phoenix-operator-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if self.0.starts_with(std::env::temp_dir()) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
const PADS: &str = "window.__phoenixSetGamepads([{\"index\":0,\"id\":\"pad\",\"buttons\":[{\"pressed\":true,\"value\":1}],\"axes\":[1]}])";

#[test]
fn placement_normalizes_one_global_panel_inventory() {
    let stored = json!({
        "root": {"type":"split", "axis":"horizontal", "sizes":[0], "children":[
            {"type":"tabs", "tabs":["a","unknown","a"], "active":"unknown"},
            {"type":"tabs", "tabs":["b"], "active":"b"}
        ]},
        "floats":[{"panel":"a"},{"panel":"c", "x":-9, "width":1},{"panel":"draft"}],
        "closed":["b","d","unknown"], "selected":"d"
    });
    let actual =
        sanitize_placement(&stored, &["a", "b", "c", "d", "draft"], &["draft"], "a").unwrap();
    assert_eq!(actual["root"]["sizes"], json!([1.0, 1.0]));
    assert_eq!(actual["root"]["children"][0]["tabs"], json!(["a"]));
    assert_eq!(actual["root"]["children"][0]["active"], "a");
    assert_eq!(
        actual["floats"],
        json!([{"panel":"c","x":0.0,"y":12.0,"width":240.0,"height":360.0}])
    );
    assert_eq!(actual["closed"], json!(["d", "draft"]));
    assert_eq!(actual["selected"], "a");
}

#[test]
fn placement_distinguishes_invalid_empty_and_reset_roots() {
    assert!(matches!(
        sanitize_placement(&json!({}), &["a"], &[], "a"),
        Err(PlacementError::Invalid)
    ));
    let empty = sanitize_placement(&json!({"root":null}), &["a"], &[], "a").unwrap();
    assert!(empty["root"].is_null());
    assert_eq!(empty["closed"], json!(["a"]));
    assert!(matches!(
        sanitize_placement(&json!({"root":{}}), &["a"], &[], "a"),
        Err(PlacementError::Reset)
    ));
    assert!(sanitize_placement(
        &json!({"root":{},"floats":[{"panel":"a"}]}),
        &["a"],
        &[],
        "a"
    )
    .is_ok());
}

#[test]
fn live_layout_version_gate_preserves_reset_and_refusal() {
    for version in 1..19 {
        let layout = json!({"version": version, "root": {"type":"tabs", "tabs":["map"], "active":"map"},
                "floats":[], "closed":[], "selected":"map"});
        assert_eq!(sanitize_live_layout(&layout), Some(default_live_layout()));
    }
    for version in [json!(0), json!(20), json!(-1), json!("19"), Value::Null] {
        assert_eq!(sanitize_live_layout(&json!({"version":version})), None);
    }
    assert_eq!(sanitize_live_layout(&json!({"version":19})), None);
    assert!(sanitize_live_layout(&default_live_layout()).is_some());
}

#[test]
fn loading_an_old_live_layout_atomically_keeps_a_backup_and_persists_the_reset() {
    let dir = Scratch::new();
    let mut state = NativeOperators {
        root: Some(dir.0.clone()),
        scope: Some("gm".into()),
        ..Default::default()
    };
    let path = state.path("desk").unwrap();
    let mut layout = default_live_layout();
    layout["version"] = json!(18);
    let old = json!({"kind":"project-phoenix/operator-profile", "version":1,
            "liveLayout":layout, "gmDensity":"touch",
            "accessibility":{"presentation":{"textScale":1.25}}});
    crate::native_file::write_preferences(&path, &old.to_string()).unwrap();
    let request = r#"{"type":"NativeOperator","operation":"load"}"#;
    assert!(state.handle(PaneId(1), "desk", request));
    let first = std::fs::read_to_string(&path).unwrap();
    let saved: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(saved["liveLayout"], default_live_layout());
    assert!(saved["previousLiveLayout"].is_object());
    assert_eq!(saved["gmDensity"], "touch");
    assert_eq!(saved["accessibility"]["presentation"]["textScale"], 1.25);
    assert!(state.handle(PaneId(1), "desk", request));
    assert_eq!(std::fs::read_to_string(path).unwrap(), first);
}

#[test]
fn test_layout_matches_the_browser_model_case_for_case() {
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string("tests/fixtures/workshop-test-layout-migrations.json").unwrap(),
    )
    .unwrap();
    assert_eq!(fixture["version"], default_test_layout()["version"]);
    assert_eq!(fixture["default"], default_test_layout());
    for case in fixture["cases"].as_array().unwrap() {
        let actual = sanitize_test_layout(&case["stored"]).unwrap_or_else(default_test_layout);
        assert_eq!(
            numbers_as_floats(&actual),
            numbers_as_floats(&case["expected"]),
            "{}",
            case["name"]
        );
    }
}
#[test]
fn host_excludes_other_consoles_and_neutralizes_unowned_snapshots() {
    let mut state = NativeOperators::default();
    state.observe(PADS);
    state.handle(
        PaneId(1),
        "helm",
        r#"{"type":"NativeOperator","operation":"select","index":0}"#,
    );
    state.handle(
        PaneId(2),
        "tactical",
        r#"{"type":"NativeOperator","operation":"select","index":0}"#,
    );
    let owned = snapshot(&state.snapshot_for(PADS, PaneId(1))).unwrap();
    let other = snapshot(&state.snapshot_for(PADS, PaneId(2))).unwrap();
    assert_eq!(owned[0]["axes"][0], 1);
    assert_eq!(other[0]["axes"][0], 0);
    assert_eq!(other[0]["assignedTo"], "helm");
    assert_eq!(other[0]["available"], false);
    state.close(PaneId(1));
    assert_eq!(
        snapshot(&state.snapshot_for(PADS, PaneId(2))).unwrap()[0]["available"],
        true
    );
}
#[test]
fn disconnect_releases_controller_and_reconnect_needs_a_new_claim() {
    let mut state = NativeOperators::default();
    state.observe(PADS);
    state.handle(
        PaneId(1),
        "helm",
        r#"{"type":"NativeOperator","operation":"select","index":0}"#,
    );
    state.observe("window.__phoenixSetGamepads([])");
    state.observe(PADS);
    assert_eq!(
        snapshot(&state.snapshot_for(PADS, PaneId(1))).unwrap()[0]["nativeOwned"],
        false
    );
}
#[test]
fn preferences_round_trip_by_hull_and_label_without_credentials() {
    let dir = Scratch::new();
    let mut state = NativeOperators {
        root: Some(dir.0.clone()),
        scope: Some("cruiser".into()),
        ..Default::default()
    };
    let profile = json!({"kind":"project-phoenix/operator-profile", "version":1,
            "token":"secret", "gamepad":{"preferredDevice":{"id":"pad", "mapping":"standard", "token":"secret"}, "hideTouchControls":false},
            "accessibility":{"presentation":{"textScale":1.25,"contrast":"on","reducedMotion":"off","shake":0.2,"flash":0.4,"decorativeMotion":"default","unsafe":"secret"},
                "assistance":{"helm.course-keeping":"request"}},
            "bindings":{"helm.thrust":[{"type":"gamepad", "input":"axis", "control":"left-stick-y", "token":"secret"}, null]},
            "audio":{"version":1,"mono":true,"reducedRange":true,"output":"secret","history":["secret"],
                "mix":{"master":{"level":0.12,"muted":true},"music":{"level":0.5}},
                "cues":{"applied":true,"unknown":"secret"}},
            "authoringLayout":{"version":1,"root":{"type":"tabs","tabs":["source"],"active":"source","unsafe":"secret"},"floats":[],"closed":["files","inspector"],"selected":"source","unsafe":"secret"}});
    state.handle(
        PaneId(1),
        "helm",
        &json!({"type":"NativeOperator", "operation":"save", "profile":profile.to_string()})
            .to_string(),
    );
    let text = std::fs::read_to_string(state.path("helm").unwrap()).unwrap();
    assert!(!text.contains("secret"));
    let saved: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        saved["audio"]["mix"]["master"],
        json!({"level":0.12,"muted":true})
    );
    assert_eq!(
        saved["audio"]["mix"]["music"],
        json!({"level":0.5,"muted":false})
    );
    assert_eq!(saved["audio"]["cues"]["applied"], true);
    assert_eq!(saved["audio"]["mono"], true);
    assert_eq!(saved["audio"]["reducedRange"], true);
    assert_eq!(
        saved["accessibility"]["presentation"],
        json!({
            "textScale":1.25,"contrast":"on","reducedMotion":"off",
            "shake":0.2,"flash":0.4,"decorativeMotion":"default"
        })
    );
    assert_eq!(
        saved["accessibility"]["assistance"],
        json!({"helm.course-keeping":"request"})
    );
    assert_eq!(saved["authoringLayout"]["selected"], "source");
    assert!(saved["authoringLayout"].get("unsafe").is_none());
    state.handle(
        PaneId(2),
        "helm",
        r#"{"type":"NativeOperator","operation":"load"}"#,
    );
    assert!(state.replies[&PaneId(2)][0].contains("left-stick-y"));
    assert!(state.replies[&PaneId(2)][0].contains("0.12"));
    assert_ne!(state.path("../helm"), state.path("helm"));
    state.scope = Some("destroyer".into());
    assert!(!state.path("helm").unwrap().exists());
}

/// The browser and this sanitizer read the same operator profile, so they
/// must agree exactly on what a stored Live layout becomes. The expectations
/// are GENERATED from the browser model — transcribing them by hand went
/// wrong once per registered panel — by
/// `scripts/generate-live-layout-fixture.mjs`, which `npm run live-layout:check`
/// keeps current. What the browser model itself does is covered by
/// tests/client/live-layout-model.test.js; this pins parity.
#[test]
fn live_layout_matches_the_browser_model_case_for_case() {
    let fixture: Value = serde_json::from_str(
        &std::fs::read_to_string("tests/fixtures/live-layout-migrations.json").unwrap(),
    )
    .unwrap();
    assert_eq!(fixture["version"], default_live_layout()["version"]);
    assert_eq!(
        numbers_as_floats(&fixture["default"]),
        numbers_as_floats(&default_live_layout())
    );
    let cases = fixture["cases"].as_array().unwrap();
    // The version before this one is the migration every existing profile
    // will take, so it always has a case.
    let previous = default_live_layout()["version"].as_u64().unwrap() - 1;
    assert!(
        cases
            .iter()
            .any(|case| case["stored"]["version"].as_u64() == Some(previous)),
        "the fixture has no stored case at version {previous}"
    );
    for case in cases {
        let name = case["name"].as_str().unwrap();
        // Refusing is how this side says "use the default"; its caller then
        // does exactly that, which is what the browser returns directly.
        let sanitized = sanitize_live_layout(&case["stored"]).unwrap_or_else(default_live_layout);
        assert_eq!(
            numbers_as_floats(&sanitized),
            numbers_as_floats(&case["expected"]),
            "{name}"
        );
    }
}

/// JSON does not distinguish 1 from 1.0 but `serde_json::Value` does, and a
/// size written by JavaScript arrives as the former. Sizes are the only
/// numbers in a layout tree, so comparing them as f64 compares the values
/// rather than how each side happened to spell them.
fn numbers_as_floats(value: &Value) -> Value {
    match value {
        Value::Number(number) => json!(number.as_f64().unwrap_or_default()),
        Value::Array(items) => Value::Array(items.iter().map(numbers_as_floats).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), numbers_as_floats(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[test]
fn private_language_survives_profile_sanitising_without_script_content() {
    let source = json!({"kind":"project-phoenix/operator-profile", "version":1,
            "locale":"de-DE", "unknown":"not saved"});
    let saved: Value =
        serde_json::from_str(&sanitize_profile(&source.to_string()).unwrap()).unwrap();
    assert_eq!(saved["locale"], "de-DE");
    assert!(saved.get("unknown").is_none());
    let injected = json!({"kind":"project-phoenix/operator-profile", "version":1,
            "locale":"de'; alert(1)"});
    let saved: Value =
        serde_json::from_str(&sanitize_profile(&injected.to_string()).unwrap()).unwrap();
    assert!(saved.get("locale").is_none());
}

#[test]
fn live_layout_is_sanitized_separately_from_authoring_layout() {
    let profile = json!({
        "kind":"project-phoenix/operator-profile", "version":1,
        "authoringLayout":default_authoring_layout(),
        "liveLayout":{
            "version":1,
            "root":{"type":"tabs","tabs":["roster","roster","unsafe"],"active":"unsafe"},
            "floats":[{"panel":"join","x":30,"unsafe":"secret"}],
            "closed":["manual-save"], "selected":"unsafe", "unsafe":"secret"
        },
        "reconnectCredential":"secret"
    });
    let saved: Value =
        serde_json::from_str(&sanitize_profile(&profile.to_string()).unwrap()).unwrap();
    assert_eq!(saved["authoringLayout"]["selected"], "source");
    assert_eq!(
        saved["authoringLayout"]["root"]["children"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    // The stored layout is version 1, so it arrives migrated. What it
    // becomes is pinned against the browser by the fixture test above; what
    // matters here is that it is sanitized at all, separately from
    // Authoring, and that nothing private rides along with it.
    assert_eq!(
        saved["liveLayout"]["version"],
        default_live_layout()["version"]
    );
    assert_eq!(saved["liveLayout"], default_live_layout());
    assert_eq!(saved["previousLiveLayout"]["floats"][0]["panel"], "join");
    assert!(saved["previousLiveLayout"]["floats"][0]
        .get("unsafe")
        .is_none());
    assert!(saved.get("reconnectCredential").is_none());
    assert!(!saved.to_string().contains("secret"));
}

#[test]
fn awareness_panels_can_close_with_critical_warnings_owned_by_the_header() {
    let mut closed: Vec<&str> = LIVE_PANELS
        .iter()
        .filter(|panel| **panel != "roster")
        .copied()
        .collect();
    closed.sort_unstable();
    let layout = json!({
        "version": default_live_layout()["version"],
        "root":{"type":"tabs","tabs":["roster"],"active":"roster"},
        "floats":[], "closed":closed, "selected":"roster"
    });

    let repaired = sanitize_live_layout(&layout).unwrap();
    assert_eq!(
        repaired["root"],
        json!({"type":"tabs", "tabs":["roster"], "active":"roster"})
    );
    let closed = repaired["closed"].as_array().unwrap();
    for panel in ["attention", "health"] {
        assert!(
            closed.iter().any(|value| value == panel),
            "{panel} was forced open"
        );
    }
}

#[test]
fn over_depth_authoring_layout_is_rejected_as_a_whole() {
    let mut root = json!({"type":"tabs","tabs":["source"],"active":"source"});
    for _ in 0..6 {
        root = json!({"type":"split","axis":"horizontal","sizes":[1],"children":[root]});
    }
    let profile = json!({
        "kind":"project-phoenix/operator-profile",
        "version":1,
        "authoringLayout":{
            "version":2,
            "root":{
                "type":"split","axis":"horizontal","sizes":[1,1],
                "children":[
                    {"type":"tabs","tabs":["files"],"active":"files"},
                    root
                ]
            },
            "floats":[],"closed":[],"selected":"files"
        }
    });

    let saved: Value =
        serde_json::from_str(&sanitize_profile(&profile.to_string()).unwrap()).unwrap();
    assert_eq!(saved["authoringLayout"], default_authoring_layout());
}

#[test]
fn missing_authoring_root_is_rejected_for_default_recovery() {
    let profile = json!({
        "kind":"project-phoenix/operator-profile",
        "version":1,
        "authoringLayout":{"version":1,"floats":[],"closed":[],"selected":"source"}
    });

    let saved: Value =
        serde_json::from_str(&sanitize_profile(&profile.to_string()).unwrap()).unwrap();
    assert!(saved.get("authoringLayout").is_none());
}

#[test]
fn malformed_authoring_root_is_rejected_for_default_recovery() {
    let profile = json!({
        "kind":"project-phoenix/operator-profile",
        "version":1,
        "authoringLayout":{
            "version":2,
            "root":{"type":"unknown"},
            "floats":[],"closed":[],"selected":"source"
        }
    });

    let saved: Value =
        serde_json::from_str(&sanitize_profile(&profile.to_string()).unwrap()).unwrap();
    assert_eq!(saved["authoringLayout"], default_authoring_layout());
}

#[test]
fn explicit_null_authoring_root_preserves_all_panels_closed_state() {
    let profile = json!({
        "kind":"project-phoenix/operator-profile",
        "version":1,
        "authoringLayout":{
            "version":2,
            "root":null,
            "floats":[],
            "closed":["files","source","inspector","add","recovery"],
            "selected":"source"
        }
    });

    let saved: Value =
        serde_json::from_str(&sanitize_profile(&profile.to_string()).unwrap()).unwrap();
    assert_eq!(saved["authoringLayout"]["version"], 10);
    assert!(saved["authoringLayout"]["root"].is_null());
    assert_eq!(saved["authoringLayout"]["floats"], json!([]));
    assert_eq!(
        saved["authoringLayout"]["closed"],
        json!([
            "files",
            "source",
            "inspector",
            "add",
            "recovery",
            "dependencies",
            "findings",
            "feedback",
            "settings",
            "models",
            "model-preview",
            "sound",
            "changes",
            "definitions",
            "composition",
            "entity",
            "presets",
            "scripts"
        ])
    );
}

#[test]
fn authoring_layout_normalization_matches_browser_global_deduplication() {
    let layout = json!({
        "version":2,
        "root":null,
        "floats":[
            {"panel":"source"}, {"panel":"source"}, {"panel":"source"},
            {"panel":"inspector","x":30,"y":40}, {"panel":"files"}
        ],
        "closed":["source","inspector","files"], "selected":"inspector"
    });

    let repaired = sanitize_authoring_layout(&layout).unwrap();
    assert_eq!(repaired["version"], 10);
    assert_eq!(repaired["selected"], "inspector");
    assert_eq!(repaired["closed"], json!(["add", "recovery"]));
    fn placements(node: &Value, panel: &str) -> usize {
        match node["type"].as_str() {
            Some("tabs") => node["tabs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|candidate| candidate.as_str() == Some(panel))
                .count(),
            Some("split") => node["children"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|child| placements(child, panel))
                .sum(),
            _ => 0,
        }
    }
    for panel in [
        "source",
        "inspector",
        "files",
        "dependencies",
        "findings",
        "feedback",
        "settings",
        "models",
        "model-preview",
        "sound",
        "changes",
        "definitions",
        "composition",
        "entity",
        "presets",
        "scripts",
    ] {
        let count = placements(&repaired["root"], panel)
            + repaired["floats"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|entry| entry["panel"].as_str() == Some(panel))
                .count();
        assert_eq!(count, 1);
    }
}

#[test]
fn legacy_authoring_layout_adds_placement_only_for_lifecycle_panels() {
    let layout = json!({
        "version":1,
        "root":{"type":"tabs","tabs":["files","inspector"],"active":"files"},
        "floats":[],"closed":["source"],"selected":"files",
        "recovery":{"draft":"must not persist"}
    });

    let migrated = sanitize_authoring_layout(&layout).unwrap();
    assert_eq!(migrated["version"], 10);
    assert_eq!(
        migrated["root"]["tabs"],
        json!([
            "files",
            "inspector",
            "add",
            "recovery",
            "dependencies",
            "findings",
            "feedback",
            "settings",
            "models",
            "model-preview",
            "sound",
            "changes",
            "definitions",
            "composition",
            "entity",
            "presets",
            "scripts"
        ])
    );
    assert_eq!(migrated["selected"], "files");
    assert!(migrated.get("recovery").is_none());
}

#[test]
fn legacy_authoring_layout_rehomes_crafted_lifecycle_panels_without_duplicates() {
    let layout = json!({
        "version":1,
        "root":{"type":"split","axis":"horizontal","sizes":[10,30,60],"children":[
            {"type":"tabs","tabs":["files","add"],"active":"add"},
            {"type":"tabs","tabs":["source"],"active":"source"},
            {"type":"tabs","tabs":["inspector"],"active":"inspector"}
        ]},
        "floats":[{"panel":"recovery","x":7,"y":9,"width":300,"height":200}],
        "closed":[],"selected":"source"
    });

    assert_eq!(
        sanitize_authoring_layout(&layout).unwrap(),
        json!({
            "version":10,
            "root":{"type":"split","axis":"horizontal","sizes":[10.0,30.0,60.0],"children":[
                {"type":"tabs","tabs":["files","add","dependencies","changes","composition","presets"],"active":"add"},
                {"type":"tabs","tabs":["source","findings","feedback","model-preview","scripts"],"active":"source"},
                {"type":"tabs","tabs":["inspector","settings","models","sound","definitions","entity"],"active":"inspector"}
            ]},
            "floats":[{"panel":"recovery","x":7.0,"y":9.0,"width":300.0,"height":200.0}],
            "closed":[],"selected":"source"
        })
    );
}

#[test]
fn legacy_authoring_layouts_preserve_floating_preferred_targets() {
    for version in [1, 2] {
        let floats = json!([
            {"panel":"files","x":13.0,"y":17.0,"width":301.0,"height":211.0},
            {"panel":"source","x":41.0,"y":47.0,"width":503.0,"height":307.0}
        ]);
        let layout = json!({
            "version":version,
            "root":{"type":"split","axis":"vertical","sizes":[17,83],"children":[
                {"type":"tabs","tabs":["inspector"],"active":"inspector"},
                {"type":"tabs","tabs":["recovery"],"active":"recovery"}
            ]},
            "floats":floats, "closed":["add"], "selected":"source"
        });

        assert_eq!(
            sanitize_authoring_layout(&layout).unwrap(),
            json!({
                "version":10,
                "root":{"type":"split","axis":"vertical","sizes":[17.0,83.0],"children":[
                    {"type":"tabs","tabs":["inspector","dependencies","findings","feedback","settings","models","model-preview","sound","changes","definitions","composition","entity","presets","scripts"],"active":"inspector"},
                    {"type":"tabs","tabs":["recovery"],"active":"recovery"}
                ]},
                "floats":floats, "closed":["add"], "selected":"source"
            })
        );
    }
}

#[test]
fn all_closed_legacy_authoring_layouts_remain_all_closed() {
    for version in [1, 2] {
        let layout = json!({
            "version":version, "root":null, "floats":[],
            "closed":["files","source","inspector","add","recovery"], "selected":"source"
        });
        let migrated = sanitize_authoring_layout(&layout).unwrap();
        assert_eq!(migrated["version"], 10);
        assert!(migrated["root"].is_null());
        assert_eq!(migrated["floats"], json!([]));
        assert_eq!(
            migrated["closed"],
            json!([
                "files",
                "source",
                "inspector",
                "add",
                "recovery",
                "dependencies",
                "findings",
                "feedback",
                "settings",
                "models",
                "model-preview",
                "sound",
                "changes",
                "definitions",
                "composition",
                "entity",
                "presets",
                "scripts"
            ])
        );
    }
}

#[test]
fn current_authoring_layout_preserves_registered_panels_and_drops_unknown_fields() {
    let layout = json!({
        "version":10,
        "root":{"type":"tabs","tabs":["source","findings","feedback","dependencies","settings","models","model-preview","sound","unsafe"],"active":"feedback","unsafe":"secret"},
        "floats":[{"panel":"files","x":7,"y":9,"width":300,"height":200,"unsafe":"secret"}],
        "closed":["inspector","add","recovery"],"selected":"feedback","unsafe":"secret"
    });

    assert_eq!(
        sanitize_authoring_layout(&layout).unwrap(),
        json!({
            "version":10,
            "root":{"type":"tabs","tabs":["source","findings","feedback","dependencies","settings","models","model-preview","sound"],"active":"feedback"},
            "floats":[{"panel":"files","x":7.0,"y":9.0,"width":300.0,"height":200.0}],
            "closed":["inspector","add","recovery","changes","definitions","composition","entity","presets","scripts"],"selected":"feedback"
        })
    );
}

#[test]
fn stored_v3_authoring_layout_registers_media_panels_without_reopening_a_closed_panel() {
    let layout = json!({
        "version":3,
        "root":{"type":"split","axis":"horizontal","sizes":[22,56,22],"children":[
            {"type":"tabs","tabs":["files","dependencies"],"active":"files"},
            {"type":"tabs","tabs":["source","findings"],"active":"source"},
            {"type":"tabs","tabs":["inspector","add","recovery"],"active":"inspector"}
        ]},
        "floats":[],"closed":["feedback","settings"],"selected":"source"
    });

    assert_eq!(
        sanitize_authoring_layout(&layout).unwrap(),
        json!({
            "version":10,
            "root":{"type":"split","axis":"horizontal","sizes":[22.0,56.0,22.0],"children":[
                {"type":"tabs","tabs":["files","dependencies","changes","composition","presets"],"active":"files"},
                {"type":"tabs","tabs":["source","findings","model-preview","scripts"],"active":"source"},
                {"type":"tabs","tabs":["inspector","add","recovery","models","sound","definitions","entity"],"active":"inspector"}
            ]},
            "floats":[],"closed":["feedback","settings"],"selected":"source"
        })
    );
}

#[test]
fn stored_v6_authoring_layout_registers_composition_beside_changes_without_reopening_a_closed_panel(
) {
    let layout = json!({
        "version":6,
        "root":{"type":"split","axis":"horizontal","sizes":[22,56,22],"children":[
            {"type":"tabs","tabs":["files","dependencies","changes"],"active":"changes"},
            {"type":"tabs","tabs":["source","findings","feedback","model-preview"],"active":"source"},
            {"type":"tabs","tabs":["inspector","add","recovery","settings","sound","definitions"],"active":"sound"}
        ]},
        "floats":[],"closed":["models"],"selected":"source"
    });

    // Joins the files column at the end and leaves the group on the tab the
    // operator had open; the panel they closed under v6 stays closed. The
    // entity form v8 registered joins the inspector's column in the same pass,
    // and the v9 preset form joins the files column beside composition.
    assert_eq!(
        sanitize_authoring_layout(&layout).unwrap(),
        json!({
            "version":10,
            "root":{"type":"split","axis":"horizontal","sizes":[22.0,56.0,22.0],"children":[
                {"type":"tabs","tabs":["files","dependencies","changes","composition","presets"],"active":"changes"},
                {"type":"tabs","tabs":["source","findings","feedback","model-preview","scripts"],"active":"source"},
                {"type":"tabs","tabs":["inspector","add","recovery","settings","sound","definitions","entity"],"active":"sound"}
            ]},
            "floats":[],"closed":["models"],"selected":"source"
        })
    );
    // A v6 tree could not have named the panel: it enters through migration only.
    let crafted = json!({
        "version":6,
        "root":{"type":"tabs","tabs":["source","composition"],"active":"composition"},
        "floats":[{"panel":"composition","x":1,"y":2,"width":300,"height":200}],
        "closed":[],"selected":"composition"
    });
    let migrated = sanitize_authoring_layout(&crafted).unwrap();
    assert_eq!(migrated["version"], 10);
    assert_eq!(migrated["floats"], json!([]));
    assert_eq!(migrated["selected"], "source");
    assert_eq!(migrated["root"]["active"], "source");
    assert!(migrated["root"]["tabs"]
        .as_array()
        .unwrap()
        .contains(&json!("composition")));
}

#[test]
fn stored_v7_authoring_layout_registers_the_entity_form_beside_the_definitions_form() {
    let layout = json!({
        "version":7,
        "root":{"type":"split","axis":"horizontal","sizes":[22,56,22],"children":[
            {"type":"tabs","tabs":["files","dependencies","changes","composition"],"active":"composition"},
            {"type":"tabs","tabs":["source","findings","feedback","model-preview"],"active":"source"},
            {"type":"tabs","tabs":["inspector","add","recovery","settings","sound","definitions"],"active":"definitions"}
        ]},
        "floats":[],"closed":["models"],"selected":"source"
    });

    // Joins the inspector's column at the end and leaves the group on the tab
    // the operator had open; the panel they closed under v7 stays closed.
    assert_eq!(
        sanitize_authoring_layout(&layout).unwrap(),
        json!({
            "version":10,
            "root":{"type":"split","axis":"horizontal","sizes":[22.0,56.0,22.0],"children":[
                {"type":"tabs","tabs":["files","dependencies","changes","composition","presets"],"active":"composition"},
                {"type":"tabs","tabs":["source","findings","feedback","model-preview","scripts"],"active":"source"},
                {"type":"tabs","tabs":["inspector","add","recovery","settings","sound","definitions","entity"],"active":"definitions"}
            ]},
            "floats":[],"closed":["models"],"selected":"source"
        })
    );
    // A v7 tree could not have named the panel: it enters through migration only.
    let crafted = json!({
        "version":7,
        "root":{"type":"tabs","tabs":["source","entity"],"active":"entity"},
        "floats":[{"panel":"entity","x":1,"y":2,"width":300,"height":200}],
        "closed":[],"selected":"entity"
    });
    let migrated = sanitize_authoring_layout(&crafted).unwrap();
    assert_eq!(migrated["version"], 10);
    assert_eq!(migrated["floats"], json!([]));
    assert_eq!(migrated["selected"], "source");
    assert_eq!(migrated["root"]["active"], "source");
    assert!(migrated["root"]["tabs"]
        .as_array()
        .unwrap()
        .contains(&json!("entity")));
}

#[test]
fn stored_v8_authoring_layout_registers_the_preset_form_beside_the_composition_form() {
    let layout = json!({
        "version":8,
        "root":{"type":"split","axis":"horizontal","sizes":[22,56,22],"children":[
            {"type":"tabs","tabs":["files","dependencies","changes","composition"],"active":"composition"},
            {"type":"tabs","tabs":["source","findings","feedback","model-preview"],"active":"source"},
            {"type":"tabs","tabs":["inspector","add","recovery","settings","sound","definitions","entity"],"active":"entity"}
        ]},
        "floats":[],"closed":["models"],"selected":"source"
    });

    // Joins the files column at the end, beside the other form that edits a
    // world member, and leaves each group on the tab the operator had open;
    // the panel they closed under v8 stays closed.
    assert_eq!(
        sanitize_authoring_layout(&layout).unwrap(),
        json!({
            "version":10,
            "root":{"type":"split","axis":"horizontal","sizes":[22.0,56.0,22.0],"children":[
                {"type":"tabs","tabs":["files","dependencies","changes","composition","presets"],"active":"composition"},
                {"type":"tabs","tabs":["source","findings","feedback","model-preview","scripts"],"active":"source"},
                {"type":"tabs","tabs":["inspector","add","recovery","settings","sound","definitions","entity"],"active":"entity"}
            ]},
            "floats":[],"closed":["models"],"selected":"source"
        })
    );
    // A v8 tree could not have named the panel: it enters through migration only.
    let crafted = json!({
        "version":8,
        "root":{"type":"tabs","tabs":["source","presets"],"active":"presets"},
        "floats":[{"panel":"presets","x":1,"y":2,"width":300,"height":200}],
        "closed":[],"selected":"presets"
    });
    let migrated = sanitize_authoring_layout(&crafted).unwrap();
    assert_eq!(migrated["version"], 10);
    assert_eq!(migrated["floats"], json!([]));
    assert_eq!(migrated["selected"], "source");
    assert_eq!(migrated["root"]["active"], "source");
    assert!(migrated["root"]["tabs"]
        .as_array()
        .unwrap()
        .contains(&json!("presets")));
}

#[test]
fn a_stored_layout_cannot_name_a_panel_its_own_version_never_registered() {
    // v3 had no media vocabulary: these must enter through migration only.
    let layout = json!({
        "version":3,
        "root":{"type":"tabs","tabs":["source","model-preview","sound"],"active":"model-preview"},
        "floats":[{"panel":"models","x":7,"y":9,"width":300,"height":200}],
        "closed":[],"selected":"model-preview"
    });

    let migrated = sanitize_authoring_layout(&layout).unwrap();
    assert_eq!(migrated["version"], 10);
    assert_eq!(migrated["floats"], json!([]));
    assert_eq!(migrated["selected"], "source");
    assert_eq!(migrated["root"]["active"], "source");
}

#[test]
fn corrupt_and_unwritable_profiles_report_errors_without_breaking_controllers() {
    let dir = Scratch::new();
    let mut state = NativeOperators {
        root: Some(dir.0.clone()),
        scope: Some("cruiser".into()),
        ..Default::default()
    };
    let path = state.path("helm").unwrap();
    crate::native_file::write_preferences(&path, "corrupt").unwrap();
    state.handle(
        PaneId(1),
        "helm",
        r#"{"type":"NativeOperator","operation":"load"}"#,
    );
    assert!(state.replies[&PaneId(1)][0].contains("error"));
    let blocked = dir.0.join("blocked");
    std::fs::write(&blocked, "a file cannot be a settings directory").unwrap();
    state.root = Some(blocked);
    state.handle(
        PaneId(1),
        "helm",
        &json!({"type":"NativeOperator", "operation":"save", "profile":
            json!({"kind":"project-phoenix/operator-profile", "version":1}).to_string()})
        .to_string(),
    );
    assert!(state.replies[&PaneId(1)][1].contains("error"));
    state.observe(PADS);
    state.handle(
        PaneId(1),
        "helm",
        r#"{"type":"NativeOperator","operation":"select","index":0}"#,
    );
    assert_eq!(
        snapshot(&state.snapshot_for(PADS, PaneId(1))).unwrap()[0]["nativeOwned"],
        true
    );
}
