//! Actual native source transaction emitted by the loader form.
use super::document::{edit, EditRequest};
#[test]
fn loader_leaf_put_and_clear_preserve_unrelated_native_source() {
    let source =
        "includes=[\"base.toml\"]\n# retained parent\n[torpedoes]\ncount=6 # finite\nspeed=15\n";
    let request: EditRequest = serde_json::from_value(serde_json::json!({
        "document_path":"assets/entities/own.toml", "expected_source":source,
        "edits":[{"op":"put","path":["torpedoes","loader_groups"],
            "value_source":"[{id=\"forward\",tubes=[\"port\",\"starboard\"],work_rate=1.0}]"}]
    }))
    .unwrap();
    let actual = edit(source, &request).unwrap();
    assert!(actual.starts_with(source));
    let value: toml::Value = toml::from_str(&actual).unwrap();
    assert_eq!(value["torpedoes"]["count"].as_integer(), Some(6));
    assert_eq!(
        value["torpedoes"]["loader_groups"][0]["work_rate"].as_float(),
        Some(1.0)
    );
    let clear: EditRequest = serde_json::from_value(serde_json::json!({
        "document_path":"assets/entities/own.toml", "expected_source":actual,
        "edits":[{"op":"put","path":["torpedoes","loader_groups"],"value_source":"[]"}]
    }))
    .unwrap();
    let cleared = edit(&actual, &clear).unwrap();
    assert!(cleared.starts_with(source));
    let value: toml::Value = toml::from_str(&cleared).unwrap();
    assert!(value["torpedoes"]["loader_groups"]
        .as_array()
        .unwrap()
        .is_empty());
}
