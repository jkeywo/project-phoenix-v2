#[test]
fn model_reader_limits_match_the_installed_renderer() {
    use phoenix_content::pack_asset_validation::{MAX_JOINTS, MAX_MORPH_WEIGHTS};
    assert_eq!(MAX_JOINTS, bevy::pbr::MAX_JOINTS);
    assert_eq!(MAX_MORPH_WEIGHTS, bevy::mesh::morph::MAX_MORPH_WEIGHTS);
}
