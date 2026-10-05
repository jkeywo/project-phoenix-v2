use super::*;

#[test]
fn default_args_point_at_a_real_model() {
    assert_eq!(
        ViewerArgs::default().model.as_deref(),
        Some("assets/models/alliance_cruiser.glb")
    );
}
