use super::*;
struct Owner;
struct OtherOwner;
struct Handle<const I: usize>;

#[test]
fn physical_aliases_leave_the_exact_canonical_map_unchanged() {
    let mut app = App::new();
    app.declare_state::<Owner>(StateClass::Folded, "owner-state");
    let before = app.world().resource::<StateCensus>().entries().clone();
    app.declare_state_alias::<Handle<0>, Owner>()
        .declare_state_alias::<Handle<1>, Owner>()
        .declare_state_alias::<Handle<0>, Owner>();
    let census = app.world().resource::<StateCensus>();
    assert_eq!(census.entries(), &before);
    assert_eq!(census.len(), before.len());
    assert_eq!(census.aliases().len(), 2);
    for alias in [
        std::any::type_name::<Handle<0>>(),
        std::any::type_name::<Handle<1>>(),
    ] {
        assert_eq!(
            census.alias_owner(alias),
            Some(std::any::type_name::<Owner>())
        );
        assert_eq!(
            census.get(alias),
            census.get(std::any::type_name::<Owner>())
        );
    }
    assert_eq!(census.get(std::any::type_name::<Handle<2>>()), None);
    assert_eq!(census.alias_owner("Handle<0>"), None);
}

#[test]
fn alias_rejects_unknown_owners_chains_shadowing_and_conflicting_ownership() {
    let mut census = StateCensus::default();
    let owner = std::any::type_name::<Owner>();
    let other = std::any::type_name::<OtherOwner>();
    let first = std::any::type_name::<Handle<0>>();
    let second = std::any::type_name::<Handle<1>>();
    assert!(census.declare_alias(first, owner).is_err());
    census.declare(owner, StateClass::Folded, "owner");
    census.declare(other, StateClass::Cache, "other");
    census.declare_alias(first, owner).unwrap();
    let before = census.aliases().clone();
    assert!(census.declare_alias(first, other).is_err());
    assert!(census.declare_alias(second, first).is_err());
    assert!(census.declare_alias(owner, other).is_err());
    assert!(census.declare_alias(second, second).is_err());
    assert_eq!(census.aliases(), &before);
    // Classification is inherited, never copied into a separately mutable row.
    census.declare(owner, StateClass::DeferredFold, "owner-revised");
    assert_eq!(
        census.get(first),
        Some((StateClass::DeferredFold, "owner-revised"))
    );
}

#[test]
#[should_panic(expected = "canonical declaration cannot shadow an ownership alias")]
fn canonical_declaration_cannot_reclassify_an_alias() {
    let mut census = StateCensus::default();
    census.declare("owner", StateClass::Folded, "owner");
    census.declare_alias("alias", "owner").unwrap();
    census.declare("alias", StateClass::Derived, "different");
}
