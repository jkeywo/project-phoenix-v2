//! Same built Workshop page, with its private native adapter in place of the
//! ordinary browser boot. No selected path or filesystem capability is served.
pub fn document_path(nonce: &str) -> String {
    format!("/native-workshop-{nonce}.html")
}
pub fn drain_script() -> String {
    vellum_ultralight::bridge::drain_call("window.__phoenixNativeWorkshopDrain")
}
pub fn build_document(html: &str) -> Result<String, String> {
    const ENTRY: &str = "<script type=\"module\" src=\"gui/workshop-boot.js\"></script>";
    if html.matches(ENTRY).count() != 1 {
        return Err("Built Workshop page must contain exactly one shared authoring entry".into());
    }
    Ok(html.replace(
        ENTRY,
        &format!(
            "<script>{}</script><script type=\"module\">{}</script>",
            include_str!("queue.js"),
            include_str!("boot.js")
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_workshop_keeps_shared_markup_and_replaces_only_browser_boot() {
        let source = include_str!("../../../workshop.html");
        let page = build_document(source).unwrap();
        assert!(page.contains("id=\"workshop\""));
        assert!(page.contains("gui/native-workshop.js"));
        assert!(page.contains("PhoenixOperatorStorage"));
        assert!(page.contains("NativeWorkshopReady"));
        assert!(!page.contains("src=\"gui/workshop-boot.js\""));
        assert!(!page.contains("server.html"));
        assert!(!page.contains("__nativeGm"));
        assert!(build_document("<main id=\"workshop\"></main>").is_err());
        assert!(build_document(&format!("{source}{source}")).is_err());
    }
}
