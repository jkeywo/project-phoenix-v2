use super::*;

#[test]
fn ai_token_uses_the_entity_uuid_when_present() {
    let uuid = crate::entities::spawner::EntityUuid("abc-123".to_string());
    assert_eq!(ai_token_for(Some(&uuid)), "ai:abc-123");
}

#[test]
fn ai_token_falls_back_to_the_unregistered_backfill_token() {
    assert_eq!(ai_token_for(None), AI_BACKFILL_TOKEN);
    // The backfill token must stay unregistered-by-shape: admission routes
    // any `ai:` token it cannot resolve to the LocalShip.
    assert!(AI_BACKFILL_TOKEN.starts_with("ai:"));
}
