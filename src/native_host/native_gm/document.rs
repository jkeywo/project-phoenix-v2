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
        "<script>{}\n{}\n{}</script><script type=\"module\">{}</script>",
        include_str!("queue.js"),
        crate::native_host::panes::document::OPERATOR_STORAGE_JS,
        include_str!("../audio/private_boot.js"),
        include_str!("boot.js")
    );
    page.replace("</body>", &format!("{boot}</body>"))
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
