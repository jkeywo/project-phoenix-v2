use super::*;

#[test]
fn hud_reading_preferences_follow_typed_controls_and_survive_cached_reapplication() {
    use crate::native_host::{
        host_lobby::{pump_host_lobby, HostLobbyBridge, HostLobbyRecord},
        panes::{os_prefs::OsAccessibilityPrefs, RecordingSurface},
        viewscreen_presentation::ViewscreenPresentation,
    };
    #[derive(serde::Deserialize)]
    struct Case {
        name: String,
        record: serde_json::Value,
        script: String,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-hud-reading.json"
    ))
    .unwrap();
    let bridge = HostLobbyBridge::new();
    bridge.set_hud_presentation(
        &ViewscreenPresentation::default(),
        Some(OsAccessibilityPrefs {
            text_scale: 1.25,
            ..Default::default()
        }),
    );
    let mut cache = HudScriptCache::default();
    cache.update(r#"{"heading":90}"#);
    for case in cases {
        let mut surface = RecordingSurface::ready();
        surface.queue_record(case.record.to_string());
        pump_host_lobby(&bridge, &mut surface);
        let records = bridge.take_records();
        assert_eq!(records.len(), 1);
        let record = HostLobbyRecord::decode(&records[0]).unwrap();
        let saved = bridge.apply_hud_presentation_record(&record).unwrap();
        cache.set_reading(bridge.hud_reading_script());
        assert_eq!(
            bridge.hud_reading_script().as_deref(),
            Some(case.script.as_str()),
            "{}",
            case.name
        );
        assert!(cache
            .script()
            .unwrap()
            .starts_with(&(case.script.clone() + ";")));
        assert!(cache.script().unwrap().contains("__updateHud"));
        assert!(
            !cache.set_reading(bridge.hud_reading_script()),
            "unchanged settings cost no new revision"
        );
        let mut recreated = HudScriptCache::default();
        recreated.set_reading(bridge.hud_reading_script());
        recreated.update(r#"{"heading":90}"#);
        assert_eq!(recreated.script(), cache.script());
        // The saved record can seed another process without borrowing this
        // bridge's current state; no new persistence channel is needed.
        let relaunched = HostLobbyBridge::new();
        relaunched.set_hud_presentation(
            &saved,
            Some(OsAccessibilityPrefs {
                text_scale: 1.25,
                ..Default::default()
            }),
        );
        assert_eq!(relaunched.hud_reading_script(), bridge.hud_reading_script());
    }
    assert!(HostLobbyBridge::new().hud_reading_script().is_none());
}

#[test]
fn revealed_lobby_clears_the_hud_without_covering_tiled_consoles() {
    use crate::core::messages::GamePhase;
    use crate::native_host::host_lobby::RevealState;

    let console_layer = 0;
    let native_scene_overlay_layer = 5;
    let mut reveal = RevealState::new();
    for phase in [GamePhase::InProgress, GamePhase::GameOver] {
        reveal.observe_phase(&phase);
        let in_play = hud_z_index(reveal.presence().composited);
        assert!(in_play > native_scene_overlay_layer);

        // F9 reveals the same lobby, including its Settings cog and popup.
        // Both textures remain visible; the passive HUD yields its layer.
        let with_chrome = hud_z_index(reveal.toggle().composited);
        assert!(with_chrome < LOBBY_Z_INDEX);
        assert!(LOBBY_Z_INDEX < console_layer);

        // Hiding chrome restores the HUD over scene UI and ending content.
        assert_eq!(hud_z_index(reveal.toggle().composited), in_play);
    }
}

#[test]
fn unchanged_hud_input_keeps_its_revision_and_retained_command() {
    let mut cache = HudScriptCache::default();
    assert_eq!(cache.revision(), 0);
    assert!(cache.script().is_none());
    assert!(cache.update(r#"{"heading":90}"#));
    assert_eq!(cache.revision(), 1);
    let encoded = cache.script().unwrap().to_owned();
    assert!(!cache.update(r#"{"heading":90}"#));
    assert_eq!(cache.revision(), 1);
    assert_eq!(cache.script(), Some(encoded.as_str()));
    assert!(cache.update(r#"{"heading":91}"#));
    assert_eq!(cache.revision(), 2);
    assert_ne!(cache.script(), Some(encoded.as_str()));
}

#[test]
fn the_effects_half_rides_the_same_retained_command_and_goes_first() {
    // One command, because the worker re-applies the retained one to a view
    // that was reloaded or re-created — two channels would be two things to
    // re-apply. Effects FIRST, so a document hearing both in one evaluation
    // has its bands in force before the alert class flips.
    let mut cache = HudScriptCache::default();
    assert!(cache.set_effects(Some(hud_effects_script(1.0, 0.0, 1.0))));
    let effects_only = cache
        .script()
        .expect("a display can be told about its effects before any state arrives");
    assert!(effects_only.starts_with("window.__phoenixSetHudEffects("));
    assert!(!effects_only.contains("__updateHud"));

    assert!(cache.update(r#"{"heading":90}"#));
    let both = cache.script().expect("both halves");
    let effects_at = both
        .find("__phoenixSetHudEffects")
        .expect("the effects half");
    let update_at = both.find("__updateHud").expect("the state half");
    assert!(
        effects_at < update_at,
        "bands land before the state: {both}"
    );

    // A frame that changes neither half sends nothing.
    let revision = cache.revision();
    assert!(!cache.set_effects(Some(hud_effects_script(1.0, 0.0, 1.0))));
    assert!(!cache.update(r#"{"heading":90}"#));
    assert_eq!(cache.revision(), revision);

    // A press that moves one control is a new revision, and the state half
    // survives it untouched.
    assert!(cache.set_effects(Some(hud_effects_script(1.0, 1.0, 1.0))));
    assert_ne!(cache.revision(), revision);
    assert!(cache.script().expect("still both").contains("__updateHud"));
}

#[test]
fn flashes_off_reaches_the_overlay_document_as_a_band_it_can_read() {
    // The claim the native Display tab's flash control makes: choosing Off
    // stops the red-alert pulse the room is looking at. On native that pulse
    // is drawn by `gui/viewscreen-hud.html`, whose only handle is this push
    // — the `@media (prefers-reduced-motion)` rule beside it is dead in an
    // Ultralight view. The band name is the page's: `hud.effectBand` maps
    // 0 to "off", and `:root[data-flash="off"]` holds the glow still.
    let script = hud_effects_script(1.0, 0.0, 1.0);
    assert_eq!(
        script,
        "window.__phoenixSetHudEffects({\"shake\":1.0,\"flash\":0.0,\"decorativeMotion\":1.0})"
    );

    // The gentler stop is a number the vignette divides its own period by,
    // not a switch — so it has to survive as a number.
    assert!(hud_effects_script(0.3, 0.3, 0.4).contains("\"flash\":0.3"));
    assert!(hud_effects_script(0.3, 0.3, 0.4).contains("\"decorativeMotion\":0.4"));
}

#[test]
fn an_intensity_that_is_not_a_number_reads_as_full_rather_than_off() {
    // A broken input must never silently take an effect away: the same
    // answer `clampEffectIntensity` gives in gui/visual-effects.js, and the
    // same one the page's own stamper gives.
    assert!(hud_effects_script(f32::NAN, 4.0, -1.0).contains("\"shake\":1.0"));
    assert!(hud_effects_script(f32::NAN, 4.0, -1.0).contains("\"flash\":1.0"));
    assert!(hud_effects_script(f32::NAN, 4.0, -1.0).contains("\"decorativeMotion\":0.0"));
}
