use super::*;
#[test]
fn full_gm_markup_is_reused_without_any_browser_simulation_boot() {
    let html = build_document(include_str!("../../../server.html"));
    for id in [
        "gm-console",
        "gm-map-panel",
        "gm-session-controls",
        "gm-station-controls",
        "gm-activity",
        // The Live Inspector's entities/AI domain (issue #1489). A native
        // GM is an equal GM: the same reading surface, from the same
        // markup, not a browser-only panel.
        "gm-entity-fields-panel",
        "gm-entity-fields-list",
        "gm-ship-fields-panel",
        "gm-ship-fields-list",
        "gm-region-fields-panel",
        "gm-region-fields-occupants",
        "gm-presentation-fields-panel",
        "gm-presentation-fields-subject",
    ] {
        assert!(html.contains(&format!("id=\"{id}\"")), "{id}");
    }
    assert!(!html.contains("wasm_init("));
    assert!(!html.contains("new WebSocket("));
    assert!(html.contains("mountNativeGmWorkspace"));
    assert!(html.contains("phoenixNativeGmOut"));
}
