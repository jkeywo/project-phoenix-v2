//! Integration across the World parser/script engine and Gameplay schemas.
use crate::world::config::{parse_action_entry, RawActionEntry};
use crate::world::content::TriggerAction;
use crate::world::dispatch::ActionCmd;
use crate::world::script::effects::{BufferedEffect, EffectSink};
use crate::world::script::engine::runtime_engine;
use crate::world::script::flags::Flags;
use rhai::{Dynamic, Map};
use serde::Deserialize;
/// Build the `#{ effects, flags }` context one call reads. Flags share the one
/// ordered buffer (issue #981) so a flag write lands in `sink` alongside
/// effects; these tests write no flags.
fn make_ctx(sink: &EffectSink) -> Map {
    let mut ctx = Map::new();
    ctx.insert("effects".into(), Dynamic::from(sink.clone()));
    ctx.insert(
        "flags".into(),
        Dynamic::from(Flags::new(
            &crate::world::flags::FlagStore::new(),
            sink.clone(),
        )),
    );
    ctx
}

/// Compile `source` on a runtime engine and call `fn_name`, returning the
/// drained buffer verbatim. A local harness so this module's tests don't
/// depend on `RuntimeHost`'s failure-mode wrapper.
fn run_buffered(source: &str, fn_name: &str) -> Vec<BufferedEffect> {
    let engine = runtime_engine();
    let ast = engine.compile(source).expect("compiles");
    let sink = EffectSink::new();
    let ctx = make_ctx(&sink);
    let _ = vellum_script::call_fn(&engine, &ast, "t.rhai", fn_name, ctx).expect("calls");
    sink.take()
}

/// Like [`run_buffered`] but for the effect-only (`Cmd`) verbs: unwrap each
/// buffered effect to its `ActionCmd`. Panics on a name-resolving `Action`, so
/// a test that uses this on a spawn/objective/faction verb fails loudly.
fn run(source: &str, fn_name: &str) -> Vec<ActionCmd> {
    run_buffered(source, fn_name)
        .into_iter()
        .map(|e| match e {
            BufferedEffect::Cmd(cmd) => cmd.into_resolved(),
            BufferedEffect::Action(a) => {
                unreachable!("run(): expected only command effects, got {a:?}")
            }
        })
        .collect()
}

/// The single `TriggerAction` serde produces for one action table, built
/// independently of this module's map extraction — the independent source of
/// truth for the M6 structural-parity assertions.
///
/// It went through `parse_world` and a `[[trigger]]` wrapper until issue #985
/// deleted that container. The TABLE is what mattered and the table survives:
/// `RawActionEntry` is the same struct the script host populates, and
/// `parse_action_entry` the same shared rule, so this still reaches the
/// parity target by the route that is not the one under test.
fn toml_action(action_body: &str) -> TriggerAction {
    let raw: crate::world::config::RawActionEntry =
        toml::from_str(action_body).expect("the action table parses");
    crate::world::config::parse_action_entry(&raw).expect("the action parses")
}
/// `game_over(reason, outcome)` emits the outcome-declaring pair, and does so
/// identically to the declarative `game_over` action dispatched — the outcome
/// (`victory`) rides through in `Some(_)`, reason first.
#[test]
fn game_over_with_outcome_matches_toml() {
    let cmds = run(
        r#"fn end(ctx) { ctx.effects.game_over("world.win", "victory"); }"#,
        "end",
    );
    assert_eq!(
        cmds,
        vec![
            ActionCmd::SetGameOverReason {
                reason: "world.win".to_string(),
                outcome: Some(crate::core::balance::Outcome::Victory),
            },
            ActionCmd::SetNextState {
                phase: crate::core::messages::GamePhase::GameOver,
            },
        ]
    );
    // Structural parity: the same two commands the TOML `game_over` action
    // dispatches (`game_over` needs no context, so a bare dispatch suffices).
    assert_eq!(
        cmds,
        dispatch_bare(&toml_action(
            "type = \"game_over\"\nmessage = \"world.win\"\noutcome = \"victory\""
        ))
    );
}

/// The mirror of `flt_override_leaf_matches_declarative_float`: an `int(3)`
/// override leaf must produce the IDENTICAL `toml::Value` — a toml INTEGER,
/// not the ambient float default — the declarative
/// `repair.repair_team_count = 3` carries, AND that value must actually
/// deserialize into the genuine `u32` `EntityConfig` field it targets
/// (`entities::config::RepairConfig::repair_team_count`). That last step is
/// the whole point of #1048: before the marker existed, this same leaf
/// rendered as a toml FLOAT and could not deserialize into an integer field
/// at all.
#[test]
fn int_override_leaf_matches_declarative_integer_and_deserializes() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                    position: [0, 0, 0],
                    overrides: #{
                        repair: #{
                            repair_team_count: int(3),
                        },
                    },
                });
            }"#,
        "f",
    );
    let toml = toml_action(
        "type = \"spawn_entity\"\n\
             template_path = \"assets/entities/ship.toml\"\n\
             name = \"x\"\n\
             position = [0.0, 0.0, 0.0]\n\
             overrides = { repair = { repair_team_count = 3 } }",
    );
    assert_eq!(effs, vec![BufferedEffect::Action(toml)]);

    // Pin the conversion concretely: an `int(3)` leaf is the toml INTEGER
    // `3`, not the ambient toml FLOAT a bare `3` would render as.
    let BufferedEffect::Action(TriggerAction::SpawnEntity { overrides, .. }) = &effs[0] else {
        panic!("expected a spawn action, got {:?}", effs[0]);
    };
    let count = overrides
        .as_ref()
        .and_then(|o| o.get("repair"))
        .and_then(|r| r.get("repair_team_count"))
        .expect("override repair_team_count present");
    assert_eq!(count, &toml::Value::Integer(3));

    // The crux of #1048: the override actually deserializes into the
    // genuine integer field. Before the fix this `try_into` would fail
    // (`invalid type: floating point`3`, expected u32`).
    let repair: crate::entities::config::RepairConfig = overrides
        .as_ref()
        .and_then(|o| o.get("repair"))
        .cloned()
        .expect("repair override present")
        .try_into()
        .expect("an int(3) leaf must deserialize into RepairConfig's genuine u32 field");
    assert_eq!(repair.repair_team_count, 3);
}

/// The control, mirroring `spawn_entity_override_without_a_tombstone_still_applies`'s
/// role for the tombstone test: an UNMARKED int on the SAME integer-target
/// field still renders as the ambient toml FLOAT, exactly as before #1048.
/// The marker is opt-in, not a new schema-aware default — pinned here so a
/// regression that made `dynamic_to_toml` "smart" about `repair_team_count`
/// specifically (the hand-maintained field table issue #1048 explicitly
/// rejected) would fail this test, not silently pass it.
#[test]
fn a_bare_int_on_the_same_integer_field_still_renders_as_the_ambient_float() {
    let effs = run_buffered(
        r#"fn f(ctx) {
                ctx.effects.spawn_entity(#{
                    template_path: "assets/entities/ship.toml",
                    name: "x",
                    position: [0, 0, 0],
                    overrides: #{
                        repair: #{
                            repair_team_count: 3,
                        },
                    },
                });
            }"#,
        "f",
    );
    let BufferedEffect::Action(TriggerAction::SpawnEntity { overrides, .. }) = &effs[0] else {
        panic!("expected a spawn action, got {:?}", effs[0]);
    };
    let count = overrides
        .as_ref()
        .and_then(|o| o.get("repair"))
        .and_then(|r| r.get("repair_team_count"))
        .expect("override repair_team_count present");
    assert_eq!(
        count,
        &toml::Value::Float(3.0),
        "an unmarked int must still render as the ambient float default"
    );
    // And, unmarked, it does NOT deserialize into the integer field — the
    // authoring mistake `int(…)` exists to let an author avoid.
    let repair_result: Result<crate::entities::config::RepairConfig, _> = overrides
        .as_ref()
        .and_then(|o| o.get("repair"))
        .cloned()
        .expect("repair override present")
        .try_into();
    assert!(
        repair_result.is_err(),
        "a float leaf must NOT silently coerce into the integer field"
    );
}

/// A minimal context-free dispatch of one action to its `ActionCmd`s, for the
/// `game_over` structural-parity assertion (which needs no resolution). Mirrors
/// the comms module's `dispatch_toml`.
fn dispatch_bare(action: &TriggerAction) -> Vec<ActionCmd> {
    use crate::world::dispatch::{dispatch_action, DispatchContext};
    use std::collections::HashMap;
    let names: HashMap<String, String> = HashMap::new();
    let base_flags = crate::world::flags::FlagStore::new();
    let layers = HashMap::new();
    let anchors = HashMap::new();
    let uuid = || "uuid".to_string();
    let ctx = DispatchContext {
        origin_layer: None,
        entity_name: None,
        name_to_uuid: &names,
        base_flags: &base_flags,
        layers: &layers,
        base_anchors: &anchors,
        factions: None,
        uuid_source: &uuid,
        template_loader: &crate::entities::loader::WasmTemplateLoader,
    };
    dispatch_action(action, &ctx).commands
}

/// Parse a document of `[[action]]` tables into their [`TriggerAction`]s.
///
/// The tables below were `[[trigger.action]]` arrays until issue #985 deleted
/// the `[[trigger]]` container. Their SHAPE outlived it: [`RawActionEntry`] is
/// what the Rhai effect host populates from a `#{ ... }` script map before
/// running the shared [`parse_action_entry`], so every rule these tests pin —
/// the directive field ownership, the anchor/position XOR, the required
/// fields, the unknown-type refusal — is still live. It is reached from a
/// script now instead of from a trigger's action array, which is why the
/// container is what went and the table is what stayed.
fn actions(tables: &str) -> Result<Vec<TriggerAction>, String> {
    #[derive(Deserialize)]
    struct Doc {
        #[serde(default)]
        action: Vec<RawActionEntry>,
    }
    let doc: Doc = toml::from_str(tables).map_err(|e| e.to_string())?;
    doc.action.iter().map(parse_action_entry).collect()
}
#[test]
fn unknown_directive_fields_are_rejected_from_both_toml_surfaces() {
    let world_error = actions(
        r#"
[[action]]
type = "add_objective"
id = "unknown-world-field"
text = "Unknown"
directive_kind = "Patrol"
directive_waypoints = ["alpha"]
"#,
    )
    .expect_err("an unknown World field must fail");
    assert!(
        world_error.contains("unknown Directive field `directive_waypoints`"),
        "{world_error}"
    );

    let doctrine_error = crate::entities::config::EntityConfig::from_toml_in_mode(
        r#"
[behaviour]
[[behaviour.doctrine]]
id = "unknown-doctrine-field"
directive_kind = "Patrol"
directive_waypoints = ["alpha"]
"#,
        crate::entities::ai_declaration_manifest::AiDeclarationMode::Lenient,
    )
    .expect_err("an unknown doctrine field must fail")
    .to_string();
    assert!(
        doctrine_error.contains("unknown Directive field `directive_waypoints`"),
        "{doctrine_error}"
    );
}
