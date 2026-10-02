use super::*;

#[test]
fn the_payload_carries_only_process_state_the_page_cannot_know_for_itself() {
    // Everything else the landing shows is decided by the shared view model
    // from the shared entry table. These are facts about the PROCESS,
    // which is the whole reason there is a push at all.
    let json = LandingPanelPayload::new(false).to_json();
    assert!(json.contains(r#""dismissed":false"#));
    assert!(json.contains(&format!(r#""build":"{BUILD_ID}""#)));
    // …and nothing else. A `platform` or an `open_entry` here would be the
    // host holding an opinion the page already holds.
    assert!(!json.contains("platform"));
    assert!(!json.contains("open_entry"));
}

#[test]
fn fleet_join_status_is_optional_and_typed() {
    let json = LandingPanelPayload::new(false)
        .with_join_status(Some("pending".into()))
        .to_json();
    assert!(json.contains(r#""join_status":"pending""#));
}

#[test]
fn a_committed_world_is_pushed_as_dismissed() {
    let json = LandingPanelPayload::new(true).to_json();
    assert!(json.contains(r#""dismissed":true"#));
}

#[test]
fn the_build_id_is_this_binarys_own_and_never_the_bundles() {
    // The page is assembled out of somebody else's `--client-dir` bundle,
    // whose build id belongs to the CLIENT. A viewscreen naming that in a
    // bug report would send whoever reads it to the wrong commit.
    assert_eq!(LandingPanelPayload::new(false).build, BUILD_ID);
    assert!(!BUILD_ID.is_empty());
}
