use super::*;

#[test]
fn parses_known_modes() {
    assert_eq!(Mode::parse("off"), Mode::Off);
    assert_eq!(Mode::parse("Ambient"), Mode::Ambient);
    assert_eq!(Mode::parse(" DIRECTIONAL "), Mode::Directional);
}

#[test]
fn unknown_mode_falls_back_to_ambient() {
    assert_eq!(Mode::parse("sideways"), Mode::Ambient);
}

#[test]
fn default_matches_the_games_ambient_fill() {
    let lighting = LightingMode::default();
    assert_eq!(
        lighting.ambient_brightness,
        crate::render_setup::DEFAULT_AMBIENT_BRIGHTNESS
    );
    assert_eq!(lighting.mode, Mode::Ambient);
}
