use super::*;

struct Alpha;
struct Beta<T>(#[allow(dead_code)] T);

#[test]
fn declare_state_keys_on_the_full_type_path() {
    let mut app = App::new();
    app.declare_state::<Alpha>(StateClass::Folded, "alpha-state");

    let census = app.world().resource::<StateCensus>();
    assert_eq!(census.len(), 1);
    // The key is the FULL path, so it carries the module, not just `Alpha`.
    let (key, (class, pasm)) = census.entries().iter().next().unwrap();
    assert!(
        key.ends_with("::Alpha") && key.contains("authoritative"),
        "expected a full module path ending in ::Alpha, got {key}"
    );
    assert_eq!(*class, StateClass::Folded);
    assert_eq!(*pasm, "alpha-state");
}

#[test]
fn distinct_generic_instantiations_do_not_collapse() {
    let mut app = App::new();
    app.declare_state::<Beta<Alpha>>(StateClass::Cache, "beta-alpha")
        .declare_state::<Beta<u32>>(StateClass::Derived, "beta-u32");

    // Two DISTINCT keys — the whole reason the census keys on the full path
    // rather than a short name truncated at the first `<`.
    let census = app.world().resource::<StateCensus>();
    assert_eq!(
        census.len(),
        2,
        "generic instantiations collapsed: {census:?}"
    );
}

#[test]
fn redeclaring_the_same_type_is_idempotent() {
    let mut app = App::new();
    app.declare_state::<Alpha>(StateClass::Folded, "alpha-state")
        .declare_state::<Alpha>(StateClass::Folded, "alpha-state");
    assert_eq!(app.world().resource::<StateCensus>().len(), 1);
}
