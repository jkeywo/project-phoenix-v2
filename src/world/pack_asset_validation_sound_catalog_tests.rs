use super::*;

#[test]
fn sound_catalog_dependencies_are_validated_local_paths_and_decoded_once() {
    const SOURCE: &str = include_str!("../../tests/fixtures/sound-cue-pack.toml");
    const SOUND: &str = "assets/sounds/custom/sonar ping.ogg";
    assert_eq!(
        required_assets(crate::sound_cues::PATH, SOURCE.as_bytes()).unwrap(),
        [SOUND.to_owned()].into()
    );
    let reads = std::cell::Cell::new(0);
    let catalog = validate_sound_catalog(SOURCE, &|path| {
        assert_eq!(path, SOUND);
        reads.set(reads.get() + 1);
        Some(Arc::from(
            include_bytes!("../../assets/sounds/ui_click.ogg").as_slice(),
        ))
    })
    .unwrap();
    assert_eq!(catalog.cues.len(), 2);
    assert_eq!(
        reads.get(),
        1,
        "two definitions sharing one file decode once"
    );
    for path in [
        "https://example.invalid/tone.ogg",
        "assets/sounds/../secret.ogg",
    ] {
        assert!(required_assets(
            crate::sound_cues::PATH,
            SOURCE.replace(SOUND, path).as_bytes()
        )
        .is_err());
    }
    assert!(required_assets(crate::sound_cues::PATH, b"not [valid").is_err());
    assert!(required_assets(crate::sound_cues::PATH, &[255]).is_err());
}
