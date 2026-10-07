use super::*;

#[test]
fn layered_flag_chain_walks_loader_path_innermost_first() {
    let mut base = crate::world::flags::FlagStore::default();
    base.set_flag("base_flag");
    let mut inner = WorldRuntime {
        loader_path: Some("assets/worlds/outer.toml".into()),
        ..Default::default()
    };
    inner.flags.set_flag("inner_flag");
    let mut outer = WorldRuntime::default();
    outer.flags.set_flag("outer_flag");
    let mut map = WorldLayerMap::default();
    map.0.insert("assets/worlds/inner.toml".into(), inner);
    map.0.insert("assets/worlds/outer.toml".into(), outer);

    // The store-only walk (`layered_flag_chain`) — the shape every AI
    // host reads through `entity_flag_chain`.
    let flags = layered_flag_chain(Some("assets/worlds/inner.toml"), &base, Some(&map));
    // Innermost-first: an unprefixed name reads the origin layer; each
    // `parent:` steps one entry outward — the reader's own scope first,
    // never a flattened union.
    assert!(crate::world::flags::flag_in_chain(&flags, "inner_flag"));
    assert!(!crate::world::flags::flag_in_chain(&flags, "outer_flag"));
    assert!(crate::world::flags::flag_in_chain(
        &flags,
        "parent:outer_flag"
    ));
    assert!(crate::world::flags::flag_in_chain(
        &flags,
        "parent:parent:base_flag"
    ));

    // The with-paths wrapper (`layered_flag_chain_with_paths`) —
    // `tick_trigger_pipeline`'s own shape — carries the identical store
    // chain plus the layer-path chain climbing to the base world.
    let (flags2, layers) =
        layered_flag_chain_with_paths(Some("assets/worlds/inner.toml"), &base, Some(&map));
    assert_eq!(
        layers,
        vec![
            Some("assets/worlds/inner.toml".to_string()),
            Some("assets/worlds/outer.toml".to_string()),
            None
        ],
        "the layer-path chain climbs loader_path to the base world"
    );
    assert!(crate::world::flags::flag_in_chain(&flags2, "inner_flag"));
}

#[test]
fn layered_flag_chain_from_the_base_world_is_the_base_store_alone() {
    let mut base = crate::world::flags::FlagStore::default();
    base.set_flag("base_flag");
    let flags = layered_flag_chain(None, &base, None);
    assert_eq!(flags.len(), 1);
    assert!(crate::world::flags::flag_in_chain(&flags, "base_flag"));

    let (flags2, layers) = layered_flag_chain_with_paths(None, &base, None);
    assert_eq!(layers, vec![None]);
    assert_eq!(flags2.len(), 1);
    assert!(crate::world::flags::flag_in_chain(&flags2, "base_flag"));
}
