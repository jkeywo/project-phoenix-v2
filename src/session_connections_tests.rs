use super::*;

#[test]
fn replacement_owns_both_routing_directions_before_old_close() {
    let mut registry = ConnectionRegistry::default();
    let lan = registry.new_leg();
    let cloud = registry.new_leg();
    let old = registry.open(lan);
    let current = registry.open(cloud);
    assert_eq!(registry.sender(old), None);
    assert_eq!(registry.bind(old, "opaque:crew-A"), Ok(None));
    assert_eq!(registry.bind(current, "opaque:crew-A"), Ok(Some(old)));
    assert_eq!(registry.sender(old), None);
    assert_eq!(registry.sender(current), Some("opaque:crew-A"));
    assert_eq!(
        registry.bind(old, "opaque:crew-A"),
        Err(BindRefusal::StaleConnection)
    );
    assert_eq!(registry.recipients(&Target::All), vec![current]);
    assert_eq!(registry.close(old), None);
    assert_eq!(registry.close(current), Some("opaque:crew-A".into()));
    assert_eq!(registry.close(current), None);
    let fresh = registry.open(cloud);
    assert_ne!(fresh, current);
    assert_eq!(
        registry.bind(current, "opaque:crew-A"),
        Err(BindRefusal::StaleConnection)
    );
}

#[test]
fn same_link_cannot_rename_itself_or_change_recipient_selection() {
    let mut registry = ConnectionRegistry::default();
    let leg = registry.new_leg();
    let a = registry.open(leg);
    let b = registry.open(leg);
    registry.bind(a, "A").unwrap();
    registry.bind(b, "B").unwrap();
    assert_eq!(registry.bind(a, "A"), Ok(None));
    assert_eq!(registry.bind(a, "B"), Err(BindRefusal::ChangedIdentity));
    assert_eq!(registry.recipients(&Target::Token("A".into())), vec![a]);
    assert_eq!(registry.recipients(&Target::AllExcept("A".into())), vec![b]);
    assert_eq!(registry.sender(a), Some("A"));
    assert_eq!(registry.sender(b), Some("B"));
}
