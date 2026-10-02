use super::*;

#[test]
#[allow(clippy::disallowed_methods)] // UUID isolates a disposable temp fixture.
fn only_the_launch_relative_implicit_shelf_is_created() {
    let root = std::env::temp_dir().join(format!(
        "phoenix-default-mod-shelf-{}",
        uuid::Uuid::new_v4()
    ));
    let implicit = resolve_launch_path(&root, "./mod-packs");
    prepare_implicit_mod_pack_shelf(&implicit, true).unwrap();
    assert!(implicit.is_dir());

    let explicit = resolve_launch_path(&root, "operator-shelf");
    prepare_implicit_mod_pack_shelf(&explicit, false).unwrap();
    assert!(!explicit.exists());

    let _ = std::fs::remove_dir_all(root);
}
