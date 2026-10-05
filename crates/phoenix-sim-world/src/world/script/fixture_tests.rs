use super::*;
use crate::world::dispatch::{ActionCmd, FlagMutation};

fn fixture() -> ScriptedWorld {
    ScriptedWorld::compile(
        "multi.toml",
        r#"
[script]
a = 'on_world_loaded("same_handler"); fn same_handler(ctx) { ctx.flags.value = 1; }'
b = 'on_world_loaded("same_handler"); on_world_loaded("unique"); fn same_handler(ctx) { ctx.flags.value = 2; } fn unique(ctx) { ctx.flags.value = 3; }'
"#,
    )
}
#[test]
fn registered_triggers_retain_script_identity() {
    let fixture = fixture();
    for (index, value) in [1, 2, 3].into_iter().enumerate() {
        assert_eq!(
            fixture
                .fire(index, &FlagStore::new(), &SchedClock::ZERO)
                .commands,
            vec![BufferedEffect::Cmd(ActionCmd::MutateFlag {
                target_layer: None,
                name: "value".into(),
                mutation: FlagMutation::SetValue(value)
            })]
        );
    }
    assert_eq!(
        fixture.call("unique", &FlagStore::new()).commands,
        fixture
            .fire(2, &FlagStore::new(), &SchedClock::ZERO)
            .commands
    );
}
#[test]
#[should_panic(expected = "ambiguous handler")]
fn a_direct_ambiguous_name_must_not_choose_map_order() {
    fixture().call("same_handler", &FlagStore::new());
}
