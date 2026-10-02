use super::*;
use crate::entities::config_cache::{
    push_mod_pack, remove_mod_pack, reorder_mod_packs, ActivePack,
};
use std::sync::Arc;

#[test]
fn native_dependency_paths_resolve_to_portable_pack_keys() {
    let dependency = Path::new("models").join("probe").join("texture.png");
    assert_eq!(
        authored_path(&dependency).as_deref(),
        Some("assets/models/probe/texture.png")
    );
    assert_eq!(authored_path(Path::new("")).as_deref(), Some("assets"));
    assert!(authored_path(Path::new("../models/escape.glb")).is_none());
    assert!(authored_path(Path::new("/models/escape.glb")).is_none());
    #[cfg(windows)]
    {
        assert_eq!(
            authored_path(Path::new(r"models\probe\texture.png")).as_deref(),
            Some("assets/models/probe/texture.png")
        );
        assert!(authored_path(Path::new(r"C:\models\escape.glb")).is_none());
    }
}

#[test]
fn asset_reads_follow_stack_reorder_remove_and_disk_fallback_without_utf8_conversion() {
    let _guard = crate::entities::config_cache::overlay_test_guard();
    let memory = bevy::asset::io::memory::Dir::default();
    memory.insert_asset(Path::new("models/probe.glb"), vec![1, 2, 3]);
    let reader = PackAssetReader::new(Box::new(bevy::asset::io::memory::MemoryAssetReader {
        root: memory,
    }));
    let path = "assets/models/probe.glb";
    let install = |id: &str, bytes: &[u8]| {
        push_mod_pack(ActivePack {
            id: id.into(),
            assets: [(path.into(), Arc::from(bytes))].into(),
            ..default()
        })
    };
    let read = || {
        bevy::tasks::block_on(async {
            let mut stream = reader.read(Path::new("models/probe.glb")).await.unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).await.unwrap();
            bytes
        })
    };
    assert_eq!(read(), [1, 2, 3]);
    install("first", &[0, 255, 1]);
    install("second", &[254, 0, 2]);
    assert_eq!(read(), [254, 0, 2]);
    reorder_mod_packs(&["second".into(), "first".into()]);
    assert_eq!(read(), [0, 255, 1]);
    remove_mod_pack("first");
    assert_eq!(read(), [254, 0, 2]);
    remove_mod_pack("second");
    assert_eq!(read(), [1, 2, 3]);
}
