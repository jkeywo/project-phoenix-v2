//! Reuses the built host page's GM markup and styles without its WASM boot.

pub fn document_path(nonce: &str) -> String {
    format!("/native-gm-{nonce}.html")
}
pub fn drain_script() -> String {
    vellum_ultralight::bridge::drain_call("window.__phoenixNativeGmOutDrain")
}

pub fn build_document(host_html: &str) -> String {
    // Built bundles can contain Trunk's loader as well as authored module islands.
    // None may start another simulation in an embedded GM surface.
    let mut page = String::new();
    let mut remaining = host_html;
    while let Some(start) = remaining.find("<script") {
        page.push_str(&remaining[..start]);
        let Some(end) = remaining[start..].find("</script>") else {
            remaining = "";
            break;
        };
        remaining = &remaining[start + end + "</script>".len()..];
    }
    page.push_str(remaining);
    let boot = format!(
        "<script>{}</script><script type=\"module\">{}</script>",
        include_str!("queue.js"),
        include_str!("boot.js")
    );
    page.replace("</body>", &format!("{boot}</body>"))
}

#[cfg(test)]
mod tests {
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
        ] {
            assert!(html.contains(&format!("id=\"{id}\"")), "{id}");
        }
        assert!(!html.contains("wasm_init("));
        assert!(!html.contains("new WebSocket("));
        assert!(html.contains("mountNativeGmWorkspace"));
        assert!(html.contains("phoenixNativeGmOut"));
    }
}
