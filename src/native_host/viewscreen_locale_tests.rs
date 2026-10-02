use super::*;

#[test]
fn locale_round_trips_and_rejects_script_content() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("phoenix-locale-{nonce}"));
    let store = ViewscreenLocaleStore::at(&dir);
    assert_eq!(store.load(), None);
    store.save("de-DE").unwrap();
    assert_eq!(store.load().as_deref(), Some("de-DE"));
    assert!(!valid_locale("de'; alert(1)"));
    assert!(store.save("de'; alert(1)").is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
