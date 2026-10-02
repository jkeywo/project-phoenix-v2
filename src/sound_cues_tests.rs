use super::*;
#[derive(Deserialize)]
struct Case {
    name: String,
    cue: SoundDefinition,
    expected: Option<String>,
}
#[test]
fn shared_validation_examples_pin_metadata_and_audience_rules() {
    let catalog = bundled();
    catalog.validate_all().unwrap();
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("../tests/fixtures/sound-cue-validation.json")).unwrap();
    for case in cases {
        assert_eq!(
            catalog.validate(&case.cue).err(),
            case.expected.as_deref(),
            "{}",
            case.name
        );
    }
}
#[test]
fn authored_catalog_requires_real_resolved_assets_and_immutable_information_floors() {
    let source = include_str!("../assets/audio/sound-cues.toml");
    let available: BTreeSet<_> = bundled()
        .assets
        .into_iter()
        .map(|asset| asset.file)
        .collect();
    assert!(validate_source(source, |path| available.contains(path)).is_ok());
    assert!(validate_source(source, |path| path != "assets/sounds/Blaster.mp3").is_err());
    let changed = source.replacen("informative = true", "informative = false", 1);
    assert!(validate_source(&changed, |_| true).is_err());
    let unknown = source.replace("volume = 1.0", "volume = 1.0\nspeech = true");
    assert!(validate_source(&unknown, |_| true).is_err());
}
#[test]
fn new_nested_packaged_sound_does_not_require_stock_inventory_and_missing_bytes_refuse() {
    let source = r#"version=1
[[assets]]
file="assets/sounds/custom/sonar ping.wav"
category="alerts"
informative=true
[[cues]]
id="sonar"
label="Sonar report"
file="assets/sounds/custom/sonar ping.wav"
category="alerts"
audience="station"
volume=0.2
[cues.equivalent]
meaning="Contact report ready"
source="Sensors"
urgency="advisory"
"#;
    let catalog =
        validate_source(source, |path| path == "assets/sounds/custom/sonar ping.wav").unwrap();
    assert!(validate_source(source, |_| false).is_err());
    assert!(validate_definition(&catalog.cues[0], Some(catalog.assets[0].clone())).is_ok());
    assert!(validate_source(&source.replace("custom/", "../"), |_| true).is_err());
}
