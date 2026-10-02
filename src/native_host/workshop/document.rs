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
#[path = "document_tests.rs"]
mod tests;
