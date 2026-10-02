use super::*;

#[test]
fn a_minted_token_is_a_uuid_and_two_panes_never_share_one() {
    let a = PaneIdentity::mint("Ada");
    let b = PaneIdentity::mint("Grace");
    assert_ne!(a.token(), b.token());
    assert_eq!(a.token().len(), 36, "UUIDv4 hyphenated: {}", a.token());
    assert_eq!(a.name(), "Ada");
    // And it is admissible by the lobby's own test, which is the claim that
    // matters: the seam refuses reserved tokens, so a minted one that
    // happened to look reserved would produce a pane that silently never
    // joined.
    assert!(!crate::lobby::handler::is_reserved_token(a.token()));
}

#[test]
fn the_host_operators_token_cannot_become_a_panes_identity() {
    // `LOCAL_CONSOLE_TOKEN` skips the station-tenure branch of
    // `is_command_authorized` entirely and carries mission-abort authority.
    // A pane built around it would satisfy every other acceptance criterion
    // and violate the one that matters.
    let err = PaneIdentity::adopt(crate::console_bridge::LOCAL_CONSOLE_TOKEN, "impostor")
        .expect_err("the host operator's token is not a participant identity");
    assert!(matches!(err, IdentityRefusal::Reserved(_)));
    assert!(
        err.to_string().contains("ordinary session token"),
        "the refusal must say what to do instead: {err}"
    );
}

#[test]
fn an_ai_control_source_label_cannot_become_a_panes_identity() {
    // `ai:`-prefixed tokens take the `policy.operate_ai` branch, which is
    // the other half of the same hole.
    assert!(matches!(
        PaneIdentity::adopt("ai:helm", "impostor"),
        Err(IdentityRefusal::Reserved(_))
    ));
}

#[test]
fn an_empty_token_is_refused_rather_than_producing_an_unkeyable_session() {
    assert_eq!(PaneIdentity::adopt("", "Ada"), Err(IdentityRefusal::Empty));
}

#[test]
fn an_ordinary_token_is_adopted_unchanged() {
    let identity = PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap();
    assert_eq!(identity.token(), "3f1a6c2e-0a11-4b3c-9d55-000000000001");
}
