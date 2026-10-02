use super::*;

#[test]
fn the_payload_carries_the_shelf_the_landing_draws() {
    let mut shelf = ModPackShelfResource {
        dir: "mods".into(),
        content_dir: ".".into(),
        manifest_rel: "assets/scenarios.toml".into(),
        ..Default::default()
    };
    shelf.shelf =
        mod_packs::shelf_from_listing(&[mod_packs::ShelfListingEntry::file("thin-margin.zip")]);
    let json = shelf.payload().to_json();
    assert!(json.contains(r#""file":"thin-margin.zip""#));
    assert!(json.contains(r#""label":"thin-margin""#));
    assert!(json.contains(r#""dir":"mods""#));
    // …and nothing about which row is highlighted or which entry is open.
    // Both are the page's own memory, for the reason `landing` states.
    assert!(!json.contains("chosen"));
    assert!(!json.contains("open_entry"));
}

#[test]
fn a_failed_scan_is_reported_rather_than_shown_as_an_empty_folder() {
    // "There are no packs in this folder" and "there is no such folder" ask
    // the operator to do two different things, so they must not be the same
    // screen. The scan is against a path this test knows cannot exist, which
    // is the one thing about it that needs no filesystem to arrange.
    let shelf = ModPackShelfResource::new(
        "no-such-directory-for-issue-1366",
        ".",
        "assets/scenarios.toml",
    );
    assert!(shelf.shelf.is_empty());
    let error = shelf
        .scan_error
        .clone()
        .expect("an unreadable folder says so");
    assert!(
        error.contains("no-such-directory-for-issue-1366"),
        "the message must name the folder the operator wrote: {error}"
    );
    assert!(shelf.payload().scan_error.is_some());
}

#[test]
fn an_adapter_raised_refusal_looks_exactly_like_a_validators() {
    // From where the operator is standing, "that archive is not on the
    // shelf any more" and "that archive is missing its manifest" are the
    // same event: they chose one and it did not go in. So they are one
    // shape, and the panel needs no second way to draw one.
    let finding = PackFinding::error("unknown-pack", "gone.zip", "it went away".into());
    assert_eq!(finding.severity, "error");
    assert_eq!(finding.category, "unknown-pack");
    assert_eq!(finding.file, "gone.zip");
}

#[test]
fn a_pack_that_is_not_an_archive_at_all_is_refused_and_installs_nothing() {
    // The whole atomic-acceptance claim, exercised through this adapter
    // rather than restated: the validation is `validate_mod_pack`'s and the
    // only thing checked here is that a refusal reaches the surface as
    // findings and leaves the overlay stack alone.
    let _overlay = crate::entities::config_cache::overlay_test_guard();
    let before = active_packs().len();
    let outcome = install_pack(
        b"this is not a zip file",
        "[content]\nid = \"phoenix-base\"\nepoch = 1\n",
        |_| None,
        &crate::entities::loader::WasmTemplateLoader,
    );
    assert!(!outcome.accepted);
    assert!(
        outcome.findings.iter().any(|f| f.severity == "error"),
        "a refusal has to say what is wrong: {:?}",
        outcome.findings
    );
    assert_eq!(active_packs().len(), before, "nothing partial is installed");
}

#[test]
fn conflicts_name_the_winner_so_it_is_clear_which_pack_is_flying() {
    // Read straight off `config_cache::overlay_conflicts`, which is issue
    // #987's own report — this only reshapes it for the wire. Driven through
    // the real stack so the two cannot drift — and off the browser that stack
    // is process-global, so the guard is what keeps this test's two packs its
    // own.
    let _overlay = crate::entities::config_cache::overlay_test_guard();
    let path = "assets/entities/__i1366_conflict.toml";
    let pack = |id: &str, body: &str| ActivePack {
        id: id.to_string(),
        name: id.to_string(),
        version: "1".to_string(),
        files: [(path.to_string(), body.to_string())].into_iter().collect(),
        manifest_toml: String::new(),
        ..Default::default()
    };
    push_mod_pack(pack("i1366-a", "A"));
    push_mod_pack(pack("i1366-b", "B"));
    let conflicts = active_conflicts();
    let found = conflicts
        .iter()
        .find(|c| c.path == path)
        .expect("both packs carry the path");
    assert_eq!(found.winner, "i1366-b", "the latest loaded wins");
    assert_eq!(found.losers, vec!["i1366-a".to_string()]);
    // And the installed list is what the panel shows beside it.
    let ids: Vec<String> = installed_packs().into_iter().map(|p| p.id).collect();
    assert!(ids.contains(&"i1366-a".to_string()));
    assert!(ids.contains(&"i1366-b".to_string()));
}
