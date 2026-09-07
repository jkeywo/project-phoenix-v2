//! Real native shelf installation -> catalogue broadcast -> world lock ->
//! reconnect. Its own process isolates native template and mod-pack caches.
use bevy::prelude::*;
use project_phoenix::boot::NativeRenderSurface;
use project_phoenix::core::codec::{encode_scenario_catalog, JsonCodec, MessageCodec};
use project_phoenix::core::messages::{ClientMessage, ScenarioCatalogPayload, ServerMessage};
use project_phoenix::delivery::serve::ManifestSource;
use project_phoenix::lobby::handler::Target;
use project_phoenix::native_host::host_lobby::{
    pump_host_lobby, LocalHostLobby, ModPackShelfResource,
};
use project_phoenix::native_host::panes::RecordingSurface;
use project_phoenix::native_host::transport::{LoopbackHandle, NativeTransportLink};
use project_phoenix::native_host::{
    build_native_host_app, preload_content_templates, NativeHostConfig,
};
use project_phoenix::world::config::WorldConfig;

fn pump(app: &mut App, frames: usize) {
    for _ in 0..frames {
        app.update();
    }
}

fn last_catalogue(handle: &LoopbackHandle, token: Option<&str>) -> ScenarioCatalogPayload {
    handle
        .drain_outbound()
        .into_iter()
        .filter_map(|(target, msg, _)| match msg {
            ServerMessage::ScenarioCatalog(payload)
                if token.is_none_or(|t| target == Target::Token(t.to_string())) =>
            {
                Some(payload)
            }
            _ => None,
        })
        .next_back()
        .expect("the actual native transport publishes a catalogue")
}

fn assert_browser_and_surface(
    native: &ScenarioCatalogPayload,
    lobby: &LocalHostLobby,
    surface: &mut RecordingSurface,
) {
    let catalog = ManifestSource::read(".", "assets/scenarios.toml")
        .unwrap()
        .merged_catalog()
        .catalog;
    let json = project_phoenix::server::bridge::browser_scenario_catalog_message(
        &encode_scenario_catalog(&project_phoenix::delivery::payload::catalog_payload(
            &catalog,
        ))
        .unwrap(),
        native.locked_scenario.clone(),
        native.locked_ship.clone(),
    )
    .unwrap();
    assert_eq!(
        JsonCodec.decode_server(&json).unwrap(),
        ServerMessage::ScenarioCatalog(native.clone())
    );
    pump_host_lobby(&lobby.bridge, surface);
    let latest = surface
        .pushed
        .iter()
        .rev()
        .find_map(|script| {
            let body = script
                .strip_prefix("window.__phoenixHostLobbyScenario('")?
                .strip_suffix("')")?;
            serde_json::from_str::<serde_json::Value>(
                &body.replace("\\'", "'").replace("\\\\", "\\"),
            )
            .ok()
        })
        .expect("the same update reaches the real native surface bridge");
    let mut expected: serde_json::Value = serde_json::from_str(&json).unwrap();
    expected["data"]["locked"] = latest["locked"].clone();
    assert_eq!(latest, expected["data"]);
}

/// The same exact fixtures the phone reducer reads, projected after this
/// process's real native shelf installation. Its base manifest curates one
/// of two hulls; the two pack worlds come from the installed ZIP overlays.
fn assert_phone_fixture(index: usize) {
    use project_phoenix::delivery::payload::{catalog_payload, catalogue_snapshot};
    use project_phoenix::entities::config_cache;
    use project_phoenix::world::manifest::{build_merged_catalog, parse_manifest};
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/scenario-catalogue-wire.json")).unwrap();
    let base = parse_manifest(fixture["base_manifest"].as_str().unwrap()).unwrap();
    let active = config_cache::active_packs();
    let parsed: Vec<_> = active
        .iter()
        .map(|pack| {
            (
                pack.id.as_str(),
                parse_manifest(&pack.manifest_toml).unwrap(),
            )
        })
        .collect();
    let mods: Vec<_> = parsed
        .iter()
        .map(|(id, manifest)| (*id, manifest))
        .collect();
    let merged = build_merged_catalog(&base, &mods, |path| {
        config_cache::mod_pack_overlay_get(path).or_else(|| {
            (path == "assets/worlds/catalogue_base.toml")
                .then(|| fixture["base_world"].as_str().unwrap().to_string())
        })
    });
    let expected = &fixture["snapshots"][index]["message"];
    let snapshot = catalogue_snapshot(
        catalog_payload(&merged.catalog),
        &active,
        expected["data"]["locked_scenario"]
            .as_str()
            .map(str::to_string),
        expected["data"]["locked_ship"].as_str().map(str::to_string),
    );
    let browser = project_phoenix::server::bridge::browser_scenario_catalog_message(
        &encode_scenario_catalog(&catalog_payload(&merged.catalog)).unwrap(),
        snapshot.locked_scenario.clone(),
        snapshot.locked_ship.clone(),
    )
    .unwrap();
    let json = JsonCodec
        .encode_server(&ServerMessage::ScenarioCatalog(snapshot))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&json).unwrap(),
        *expected
    );
    assert_eq!(
        JsonCodec.decode_server(&browser).unwrap(),
        JsonCodec.decode_server(&json).unwrap()
    );
}

#[test]
fn real_native_pack_installation_publishes_updates_locks_and_reconnect_metadata() {
    let preload = preload_content_templates(".").unwrap();
    let source = ManifestSource::read(".", "assets/scenarios.toml").unwrap();
    let mut cfg = NativeHostConfig::lobby(source.merged_catalog().catalog);
    cfg.surface = NativeRenderSurface::Contract;
    cfg.solo = false;
    let lobby = LocalHostLobby::open("127.0.0.1:8080");
    cfg.host_lobby = Some(lobby.clone());
    cfg.mod_pack_shelf = Some(ModPackShelfResource::new(
        "tests/fixtures/mod-packs",
        ".",
        "assets/scenarios.toml",
    ));
    let mut app = build_native_host_app(&cfg, &preload).unwrap();
    let handle = LoopbackHandle::default();
    app.insert_resource(NativeTransportLink::new(handle.transport()));
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1.0 / 60.0),
    ));
    app.finish();
    app.cleanup();
    let mut surface = RecordingSurface::ready();
    let identify = |token: &str| {
        handle.send(
            token,
            ClientMessage::Identify {
                token: token.into(),
                name: "Catalogue crew".into(),
            },
        )
    };
    const TOKEN: &str = "3f1a6c2e-0a11-4b3c-9d55-000000001407";
    identify(TOKEN);
    pump(&mut app, 8);
    let before = last_catalogue(&handle, Some(TOKEN));
    assert!(before.active_packs.is_empty());
    assert_phone_fixture(0);
    assert!(before.scenarios.iter().all(|s| s.source == "base"));
    assert_browser_and_surface(&before, &lobby, &mut surface);

    for (file, ids) in [
        ("valid-v1.zip", vec!["aurora-skirmish"]),
        ("script-valid.zip", vec!["aurora-skirmish", "script-valid"]),
    ] {
        surface.queue_record(&format!(r#"{{"kind":"install_mod_pack","pack":"{file}"}}"#));
        pump_host_lobby(&lobby.bridge, &mut surface);
        pump(&mut app, 8);
        assert!(app.world().resource::<ModPackShelfResource>().accepted);
        let updated = last_catalogue(&handle, None);
        assert_phone_fixture(ids.len());
        assert_eq!(
            updated
                .active_packs
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            ids
        );
        for pack in &updated.active_packs {
            assert!(!pack.name.is_empty());
            assert!(!pack.version.is_empty());
            assert!(updated.scenarios.iter().any(|s| s.source == pack.id));
        }
        assert_browser_and_surface(&updated, &lobby, &mut surface);
    }

    let selected = before
        .scenarios
        .iter()
        .find(|s| s.id == "combat_test")
        .unwrap();
    let hull = selected.ships.first().unwrap().template_path.clone();
    handle.send(
        TOKEN,
        ClientMessage::SelectScenario {
            scenario_id: selected.id.clone(),
        },
    );
    pump(&mut app, 8);
    let scenario_locked = last_catalogue(&handle, None);
    assert_phone_fixture(3);
    assert_eq!(
        scenario_locked.locked_scenario.as_deref(),
        Some("combat_test")
    );
    assert_eq!(scenario_locked.locked_ship, None);
    assert_browser_and_surface(&scenario_locked, &lobby, &mut surface);
    handle.send(
        TOKEN,
        ClientMessage::SelectPlayerShip {
            template_path: hull.clone(),
        },
    );
    pump(&mut app, 30);
    assert!(app.world().contains_resource::<WorldConfig>());
    let locked = last_catalogue(&handle, None);
    assert_phone_fixture(4);
    assert_phone_fixture(5);
    assert_eq!(locked.locked_ship.as_deref(), Some(hull.as_str()));
    assert_eq!(locked.active_packs, scenario_locked.active_packs);
    assert_browser_and_surface(&locked, &lobby, &mut surface);

    // Both a fresh phone and the same participant reconnect after world load.
    for token in ["3f1a6c2e-0a11-4b3c-9d55-000000001408", TOKEN] {
        identify(token);
        pump(&mut app, 8);
        let restored = last_catalogue(&handle, Some(token));
        assert_eq!(restored.active_packs, locked.active_packs);
        assert_eq!(restored.locked_scenario, locked.locked_scenario);
        assert_eq!(restored.locked_ship, locked.locked_ship);
        assert_eq!(
            restored
                .scenarios
                .iter()
                .map(|s| (&s.id, &s.source))
                .collect::<Vec<_>>(),
            locked
                .scenarios
                .iter()
                .map(|s| (&s.id, &s.source))
                .collect::<Vec<_>>()
        );
    }
}
