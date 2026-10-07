use super::*;

/// The scoping rule a scripted flag write resolves through (issue #1045), over
/// the one function that decides it.
#[test]
fn a_scripted_flag_write_resolves_against_its_handlers_own_layer() {
    fn write(name: &str) -> ActionCmd {
        ActionCmd::MutateFlag {
            target_layer: None,
            name: name.to_string(),
            mutation: crate::world::dispatch::FlagMutation::Set,
        }
    }
    // A live map holding one directly-loaded layer, so `parent:` has a real chain
    // to walk: `[Some("l1.toml"), None]`, terminating at the base world.
    let mut layers = WorldLayerMap::default();
    layers.0.insert("l1.toml".into(), WorldRuntime::default());
    let scoped = |name: &str, origin: Option<&str>| -> (bool, Option<String>, String) {
        let mut cmd = write(name);
        let kept = scope_scripted_flag_write(&mut cmd, "test", origin, Some(&layers));
        match cmd {
            ActionCmd::MutateFlag {
                target_layer, name, ..
            } => (kept, target_layer, name),
            _ => unreachable!(),
        }
    };

    // A base-world handler is untouched — every shipped world's behaviour.
    assert_eq!(scoped("armed", None), (true, None, "armed".to_string()));
    // A layer handler writes its OWN scope, which is where its conditions look.
    assert_eq!(
        scoped("armed", Some("l1.toml")),
        (true, Some("l1.toml".to_string()), "armed".to_string())
    );
    // `parent:` remains the deliberate escape outward; with no `loader_path`
    // recorded, one step out of a directly-loaded layer is the base world.
    assert_eq!(
        scoped("parent:armed", Some("l1.toml")),
        (true, None, "armed".to_string())
    );
    // Walking past the root is dropped rather than written somewhere arbitrary —
    // the same "no match" answer `resolve_layer_prefix` gives a condition.
    assert!(!scoped("parent:armed", None).0);
    assert!(!scoped("parent:parent:armed", Some("l1.toml")).0);
}
