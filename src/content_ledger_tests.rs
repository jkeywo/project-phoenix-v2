use super::*;

#[test]
fn record_then_reset_empties_the_ledger() {
    reset();
    record("assets/entities/a.toml", "a");
    assert!(!snapshot().is_empty());
    reset();
    assert!(snapshot().is_empty());
    assert!(frozen_or_live().is_empty());
}

#[test]
fn fold_does_not_depend_on_record_order() {
    reset();
    record("assets/entities/a.toml", "a");
    record("assets/entities/b.toml", "b");
    let forward = snapshot().fold();

    reset();
    record("assets/entities/b.toml", "b");
    record("assets/entities/a.toml", "a");
    let backward = snapshot().fold();

    assert_eq!(forward, backward, "record order must not move the digest");
    reset();
}

#[test]
fn different_content_moves_the_fold() {
    reset();
    record("assets/entities/a.toml", "a");
    let before = snapshot().fold();

    reset();
    record("assets/entities/a.toml", "a-edited");
    let after = snapshot().fold();

    assert_ne!(before, after, "an edited file's text must move the digest");
    reset();
}

#[test]
fn freeze_is_stable_against_later_recording() {
    reset();
    record("assets/entities/a.toml", "a");
    freeze();
    let frozen = frozen_or_live().fold();

    // A later record — simulating a template streaming in after the
    // world's declared set was already frozen — must not move the
    // digest a save is checked against.
    record("assets/entities/b.toml", "b");
    assert_eq!(
        frozen_or_live().fold(),
        frozen,
        "recording after freeze must not move the frozen digest"
    );
    reset();
}

#[test]
fn backslash_and_forward_slash_paths_key_the_same() {
    reset();
    record("assets\\entities\\a.toml", "a");
    record("assets/entities/a.toml", "a");
    assert_eq!(
        snapshot().len(),
        1,
        "the two spellings of the same path must collapse to one entry"
    );
    reset();
}
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_eager_walk_records_unselected_available_hulls() {
    use crate::entities::loader::{FsTemplateLoader, TemplateLoader};

    let world = crate::world::config::parse_world(
            "[global]\nseed = 1\n\n[[available_ships]]\ntemplate_path = \"assets/entities/alliance_courier.toml\"\n",
        )
        .unwrap();
    assert!(world.entities.is_empty());
    assert!(world.gm_palette.is_empty());
    assert!(crate::world::config::script_spawned_templates(&world).is_empty());

    // Resolve the browser's actual declared set independently, including
    // the composed hull and its model sidecars, without selecting a ship.
    reset();
    for path in crate::world::config::entity_template_paths(&world, &[]) {
        FsTemplateLoader
            .load_template(&path)
            .expect("declared hull exists");
    }
    let declared = snapshot();
    assert!(declared
        .get("assets/entities/alliance_courier.toml")
        .is_some());
    reset();
    eager_record_world_entities(&world);
    freeze();
    assert_eq!(frozen_or_live(), declared);
    reset();
}

// ── Script-spawned templates (issue #1047) ───────────────────────────────

/// The issue's exact shape: a world where ONLY a script names a template.
///
/// The eager walk used to see `[[entity]]` alone, so this hull never entered
/// the ledger — and a save taken in such a world loaded happily after the
/// template changed on disk, while the same edit to a declaratively-listed
/// hull refused it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_eager_walk_records_a_template_only_a_script_names() {
    const HULL: &str = "assets/entities/ship_harrow_destroyer.toml";
    let world = crate::world::config::parse_world(&format!(
        "[global]
seed = 1

[script]
setup = \"\"\"
             fn wave(ctx) {{
             ctx.effects.spawn_entity(#{{ template_path: \"{HULL}\", name: \"r1\" }});
             }}
\"\"\"
"
    ))
    .expect("fixture world parses");
    assert!(
        world.entities.is_empty(),
        "the point of the fixture: nothing declarative names the hull"
    );
    assert_eq!(
        crate::world::config::script_spawned_templates(&world)
            .into_iter()
            .map(|s| s.template_path)
            .collect::<Vec<_>>(),
        vec![HULL.to_string()],
        "and the shared enumeration does see it"
    );

    reset();
    eager_record_world_entities(&world);
    assert!(
        snapshot().get(HULL).is_some(),
        "a script-only hull must be recorded: {:?}",
        snapshot().entries().map(|(k, _)| k).collect::<Vec<_>>()
    );
    reset();
}

/// Production native boot must take its script-template set from the same
/// resolved sources it compiled, not re-scan the parsed config's inline-only
/// bodies. The two paths deliberately disagree here: the config names one
/// hull inline while the compiled sibling names another.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_eager_walk_uses_the_compiled_sibling_source_set_when_available() {
    const INLINE_HULL: &str = "assets/entities/alliance_destroyer.toml";
    const SIBLING_HULL: &str = "assets/entities/ship_harrow_destroyer.toml";
    let world = crate::world::config::parse_world(&format!(
        "[global]
seed = 1

[script]
setup = \"\"\"
             fn inline_wave(ctx) {{
             ctx.effects.spawn_entity(#{{ template_path: \"{INLINE_HULL}\", name: \"inline\" }});
             }}
\"\"\"
"
    ))
    .expect("fixture world parses");
    let compiled = crate::world::script::load::compile_scripts(&[vellum_script::ScriptSource {
            path: "tests/fixtures/script_only_spawn.rhai".into(),
            source: format!(
                "fn sibling_wave(ctx) {{\n\
                 ctx.effects.spawn_entity(#{{ template_path: \"{SIBLING_HULL}\", name: \"sibling\" }});\n\
                 }}\n"
            ),
        }]);

    reset();
    eager_record_world_entities_with_scripts(&world, Some(&compiled));
    let recorded = snapshot();
    assert!(
        recorded.get(SIBLING_HULL).is_some(),
        "the resolved sibling's hull must enter the native frozen set"
    );
    assert!(
            recorded.get(INLINE_HULL).is_none(),
            "CompiledScripts is authoritative when present; the inline fallback must not append a second source set"
        );
    reset();
}

/// The acceptance, stated as the save-compat check sees it: editing that
/// script-only template moves the content digest, which is what refuses the
/// save. Before #1047 the two digests were equal and the save loaded.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn editing_a_script_only_template_moves_the_content_digest() {
    const HULL: &str = "assets/entities/ship_harrow_destroyer.toml";
    let world = crate::world::config::parse_world(&format!(
        "[global]
seed = 1

[script]
setup = \"\"\"
             fn wave(ctx) {{
             ctx.effects.spawn_entity(#{{ template_path: \"{HULL}\", name: \"r1\" }});
             }}
\"\"\"
"
    ))
    .expect("fixture world parses");

    // The digest a save would be stamped with at load time.
    reset();
    eager_record_world_entities(&world);
    freeze();
    let at_save = crate::snapshot::content_digest(&frozen_or_live());

    // The same world after the hull file changed on disk. Simulated by
    // recording different bytes under the same key — what the walk would do
    // for real on the next boot, without this test editing the repo.
    reset();
    eager_record_world_entities(&world);
    record(
        HULL,
        "# edited by a designer
",
    );
    freeze();
    let at_load = crate::snapshot::content_digest(&frozen_or_live());

    assert_ne!(
            at_save, at_load,
            "an edit to a script-spawned hull must move the content digest —              equality here is the #1047 bug"
        );
    reset();
}

/// The computed-path arm: a template the static walk cannot see is reported
/// once, and only while it is genuinely uncovered.
#[test]
fn an_uncovered_spawn_is_reported_once_and_a_covered_one_never() {
    const COMPUTED: &str = "assets/entities/computed_hull.toml";
    const DECLARED: &str = "assets/entities/declared_hull.toml";

    reset();
    record(
        DECLARED, "[hull]
",
    );
    freeze();

    assert!(
        note_uncovered_spawn(COMPUTED),
        "the first spawn of an uncovered template reports"
    );
    assert!(
        !note_uncovered_spawn(COMPUTED),
        "and a wave that spawns it sixty more times says nothing further"
    );
    assert!(
        !note_uncovered_spawn(DECLARED),
        "a template the frozen set covers is never reported"
    );

    // Deliberately: reporting does NOT fold it in. A save taken after this
    // spawn must carry the same digest a freshly-booted resume computes, or
    // the resume refuses a valid save.
    assert!(
        !frozen_covers(COMPUTED),
        "note_uncovered_spawn must not quietly extend the frozen set"
    );
    reset();
}

/// `reset` clears the reported set with everything else, so a second world
/// load in one process reports its own uncovered spawns rather than
/// inheriting the previous world's silence.
#[test]
fn reset_re_arms_the_uncovered_spawn_report() {
    const COMPUTED: &str = "assets/entities/computed_hull.toml";
    reset();
    freeze();
    assert!(note_uncovered_spawn(COMPUTED));
    reset();
    freeze();
    assert!(
        note_uncovered_spawn(COMPUTED),
        "a new load must be able to report the same path again"
    );
    reset();
}
