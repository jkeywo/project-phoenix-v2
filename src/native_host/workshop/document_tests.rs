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
