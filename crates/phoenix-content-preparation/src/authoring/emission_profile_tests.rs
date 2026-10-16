//! Actual native document edits emitted by the visual emission form. Entity
//! acceptance/physical operation are tested by their owners, not fabricated here.
use super::document::{edit, EditRequest};

#[test]
fn emission_profile_leaf_edits_preserve_nested_and_inline_configuration() {
    for source in [
        "# untouched head\r\n[[system]]\r\nid=\"orders\"\r\nkind=\"command\"\r\n# own machinery\r\n[system.config]\r\nother=\"retained\"\r\n[system.config.emission_profile]\r\ninitial=\"normal\" # original choice\r\nadvisory_rate=1.0\r\n",
        "# untouched head\n[[system]]\nid=\"orders\"\nkind=\"command\"\n# own machinery\nconfig={other=\"retained\",emission_profile={initial=\"normal\",advisory_rate=1.0}}\n",
    ] {
        let request: EditRequest = serde_json::from_value(serde_json::json!({
            "document_path":"assets/entities/own.toml", "expected_source":source,
            "edits":[
                {"op":"put","path":["system",0,"config","emission_profile","initial"],"value_source":"\"low_emission\""},
                {"op":"put","path":["system",0,"config","emission_profile","advisory_rate"],"value_source":"0.25"}
            ]
        })).unwrap();
        let actual = edit(source, &request).unwrap();
        let mut expected: toml::Value = toml::from_str(source).unwrap();
        expected["system"][0]["config"]["emission_profile"]["initial"] = toml::Value::String("low_emission".into());
        expected["system"][0]["config"]["emission_profile"]["advisory_rate"] = toml::Value::Float(0.25);
        assert_eq!(toml::from_str::<toml::Value>(&actual).unwrap(), expected);
        assert!(actual.contains("# untouched head"));
        assert!(actual.contains("# own machinery"));
        if source.contains("\r\n") { assert!(!actual.replace("\r\n", "").contains('\n')); }
    }
}

#[test]
fn inherited_profile_materialisation_appends_only_selected_system_leaves() {
    let source = "includes=[\"base.toml\"]\n# retain selected source\n";
    let request: EditRequest = serde_json::from_value(serde_json::json!({
        "document_path":"assets/entities/own.toml", "expected_source":source,
        "edits":[
            {"op":"append_table","path":["system"],"fields":[["id","\"orders\""]]},
            {"op":"put","path":["system",0,"config"],"value_source":"{}"},
            {"op":"put","path":["system",0,"config","emission_profile"],"value_source":"{}"},
            {"op":"put","path":["system",0,"config","emission_profile","initial"],"value_source":"\"low_emission\""},
            {"op":"put","path":["system",0,"config","emission_profile","advisory_rate"],"value_source":"0.25"}
        ]
    })).unwrap();
    let actual = edit(source, &request).unwrap();
    assert!(actual.starts_with(source));
    let actual: toml::Value = toml::from_str(&actual).unwrap();
    let system = actual["system"][0].as_table().unwrap();
    assert_eq!(system.len(), 2);
    assert_eq!(system["id"].as_str(), Some("orders"));
    assert_eq!(system["config"].as_table().unwrap().len(), 1);
    assert_eq!(
        system["config"]["emission_profile"]["initial"].as_str(),
        Some("low_emission")
    );
}
