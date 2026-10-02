use super::*;
use crate::world::load::MemoryTemplateLoader;

/// A `duel.toml`-shaped world in script form: the full `[anchors]` staging
/// block, the authored `spawn_slot` body and `on_world_loaded` prelude, the
/// marker, and a default roster below it. Everything below the marker is
/// what the transform replaces, so the one default driver here exists only
/// to prove it is discarded.
const DUEL_FIXTURE: &str = r##"
[global]
seed = 1

[anchors]
player_spawn = [0.0, 0.0, 0.0]
side_a_2 = [-15.0, 0.0, 30.0]
side_a_3 = [-15.0, 0.0, -30.0]
side_a_4 = [-40.0, 0.0, 30.0]
side_a_5 = [-40.0, 0.0, -30.0]
side_b_1 = [55.0, 0.0, 0.0]
side_b_2 = [55.0, 0.0, 30.0]
side_b_3 = [55.0, 0.0, -30.0]
side_b_4 = [80.0, 0.0, 30.0]
side_b_5 = [80.0, 0.0, -30.0]

[player_spawn]
anchor = "player_spawn"

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
spawn_on = "game_start"

[script]
setup = """
on_world_loaded("on_load");

fn on_load(ctx) {
    ctx.effects.add_faction_enemy("Alliance", "Harrow");
}

fn spawn_slot(ctx, name, template, faction, group) {
    ctx.effects.spawn_entity(#{
        template_path: template,
        name: name,
        anchor: name,
        groups: [group],
        overrides: #{ faction: faction },
    });
}

fn on_side_b_destroyed(ctx) { ctx.effects.game_over("", "victory"); }

// duel:slots
on_timer(0, "spawn_side_a_2");
on_all_destroyed("side_b", "on_side_b_destroyed");

fn spawn_side_a_2(ctx) { spawn_slot(ctx, "side_a_2", "assets/entities/placeholder.toml", "aaaa", "side_a"); }
"""
"##;

fn fixture() -> toml::Value {
    toml::from_str(DUEL_FIXTURE).expect("fixture parses")
}

/// A loader that finds nothing: the fixture's hulls do not exist on disk,
/// and a template that cannot be loaded contributes no anchors — so every
/// test but the doctrine ones below is unaffected by the #888 guard.
fn no_templates() -> MemoryTemplateLoader {
    MemoryTemplateLoader::authoritative_empty()
}

/// A loader over authored `[behaviour]` TOML, keyed by template path — for
/// the doctrine-anchor guard, which needs a hull that really does carry a
/// route.
///
/// The `EntityConfig` is assembled field-wise rather than parsed through
/// `EntityConfig::from_toml`, whose strict AI-declaration gate (PRD #774
/// US7) would demand a fully crewed hull — fifteen policy blocks — from a
/// fixture that is only ever asked about its doctrine.
fn fake_templates(entries: &'static [(&'static str, &'static str)]) -> MemoryTemplateLoader {
    entries.iter().fold(
        MemoryTemplateLoader::authoritative_empty(),
        |loader, (path, behaviour)| {
            loader.with_template(
                *path,
                crate::entities::config::EntityConfig {
                    behaviour: Some(toml::from_str(behaviour).expect("fixture parses")),
                    ..Default::default()
                },
            )
        },
    )
}

/// A fake resolver: `<name>` → `assets/entities/<name>.toml`, and a
/// reserved `unknown` name that always fails (so the error path is
/// filesystem-free).
fn fake_resolve(name: &str) -> Result<String, DuelError> {
    if name == "unknown" {
        return Err(DuelError::Unresolved {
            name: name.to_string(),
            tried: vec![format!("assets/entities/alliance_{name}.toml")],
        });
    }
    Ok(format!("assets/entities/{name}.toml"))
}

/// The transformed world's `[script.setup]` source.
fn source(world: &toml::Value) -> &str {
    world["script"]["setup"]
        .as_str()
        .expect("the script block survives as a string")
}

/// The part of the source the transform generated — everything after the
/// marker line. Also the shape of an AUTHORED roster, which is what makes
/// `the_generated_default_roster_is_byte_identical_to_the_authored_one`
/// a comparison of like with like.
fn generated(world: &toml::Value) -> String {
    generated_in(source(world))
}

/// [`generated`] over a raw script source.
fn generated_in(src: &str) -> String {
    let after = *marker_line_ends(src).first().expect("the marker survives");
    src[after..].to_string()
}

/// Every generated driver as `(slot, template, faction, group)`, in emission
/// order — read back out of the generated text, so these assertions read the
/// way the pre-script ones did over `spawn_entity` actions.
fn drivers(world: &toml::Value) -> Vec<(String, String, String, String)> {
    generated(world)
        .lines()
        .filter(|l| l.starts_with("fn spawn_side_"))
        .map(|l| {
            let (_, args) = l.split_once("spawn_slot(ctx, ").expect("delegates");
            let (args, _) = args.split_once(");").expect("call closes");
            let mut it = args.split(", ").map(|a| a.trim_matches('"').to_string());
            let mut next = || it.next().unwrap_or_default();
            (next(), next(), next(), next())
        })
        .collect()
}

/// The registration calls the generated block makes, in order.
fn registrations(world: &toml::Value) -> Vec<String> {
    generated(world)
        .lines()
        .filter(|l| l.starts_with("on_timer(") || l.starts_with("on_all_destroyed("))
        .map(|l| l.to_string())
        .collect()
}

fn has_victory(world: &toml::Value) -> bool {
    registrations(world)
        .iter()
        .any(|r| r.starts_with("on_all_destroyed(\"side_b\""))
}

/// A world with no marker must be rejected, not silently run with the ship
/// lists ignored — that produced a combat-free draw that looked like a
/// balance result.
#[test]
fn a_world_without_duel_slots_is_rejected() {
    const NO_SLOTS: &str = r#"
[global]
seed = 1

[script]
setup = """
on_world_loaded("on_load");
fn on_load(ctx) { ctx.effects.add_faction_enemy("Alliance", "Harrow"); }
"""
"#;
    let err = apply_duel_sides(
        toml::from_str(NO_SLOTS).expect("fixture parses"),
        &["cruiser".into()],
        &["destroyer".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect_err("a marker-free world must be rejected");
    assert_eq!(err, DuelError::NoDuelSlots);
    // The message has to name what is missing, or the user cannot act on it.
    let msg = err.to_string();
    assert!(msg.contains(SLOT_MARKER), "got {msg:?}");
    assert!(msg.contains("assets/worlds/duel.toml"), "got {msg:?}");
}

/// A world with no `[script]` at all — the pre-conversion shape, and any
/// ordinary declarative world — is rejected the same way.
#[test]
fn a_script_free_world_is_rejected() {
    let world = toml::from_str("[global]\nseed = 1\n").expect("parses");
    assert_eq!(
        apply_duel_sides(
            world,
            &["cruiser".into()],
            &[],
            &fake_resolve,
            &no_templates()
        )
        .unwrap_err(),
        DuelError::NoDuelSlots
    );
}

/// A sibling `script = "file.rhai"` is not an inline block, so there is
/// nothing here to rewrite — reject rather than edit a file beside the world.
#[test]
fn a_sibling_script_file_is_rejected() {
    let world = toml::from_str("script = \"duel.rhai\"\n").expect("parses");
    assert_eq!(
        apply_duel_sides(
            world,
            &["cruiser".into()],
            &[],
            &fake_resolve,
            &no_templates()
        )
        .unwrap_err(),
        DuelError::NoDuelSlots
    );
}

#[test]
fn generates_named_slots_with_resolved_template_and_side_faction() {
    let world = apply_duel_sides(
        fixture(),
        &["cruiser".into(), "courier".into()], // player + 1 escort
        &["destroyer".into(), "battleship".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect("applies");

    assert_eq!(
        drivers(&world),
        vec![
            // side_a_2 from side_a[1] = courier, Alliance faction.
            (
                "side_a_2".to_string(),
                "assets/entities/courier.toml".to_string(),
                ALLIANCE_FACTION.to_string(),
                "side_a".to_string(),
            ),
            // side_b_1 / side_b_2 from side_b[0] / side_b[1], Harrow faction.
            (
                "side_b_1".to_string(),
                "assets/entities/destroyer.toml".to_string(),
                HARROW_FACTION.to_string(),
                "side_b".to_string(),
            ),
            (
                "side_b_2".to_string(),
                "assets/entities/battleship.toml".to_string(),
                HARROW_FACTION.to_string(),
                "side_b".to_string(),
            ),
        ]
    );
}

/// The generated block is exactly this text — the golden that a reviewer can
/// read against `duel.toml`'s authored default roster.
#[test]
fn the_generated_block_is_the_expected_rhai() {
    let world = apply_duel_sides(
        fixture(),
        &["cruiser".into()],
        &["destroyer".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect("applies");
    assert_eq!(
        generated(&world),
        "on_timer(0, \"spawn_side_b_1\");\n\
             on_all_destroyed(\"side_b\", \"on_side_b_destroyed\");\n\
             \n\
             fn spawn_side_b_1(ctx) { spawn_slot(ctx, \"side_b_1\", \
             \"assets/entities/destroyer.toml\", \
             \"cccccccc-3333-4333-8333-cccccccccccc\", \"side_b\"); }\n"
    );
}

/// The pin the golden above cannot make on its own: run the harness over
/// the REAL `assets/worlds/duel.toml` with the roster its authored default
/// already states, and the generated block must come out byte for byte the
/// text sitting in the file.
///
/// Authored ≡ generated is the whole basis of the un-harnessed default
/// being a trustworthy control for a harnessed run: the two rosters have to
/// be the same content reached two ways, not two hand-kept copies that
/// happen to agree today. It is also the only test that reads the shipped
/// world, so a drift in either direction — someone edits the file's roster,
/// or the generator's shape moves — fails here rather than in a balance
/// number nobody can attribute.
#[test]
fn the_generated_default_roster_is_byte_identical_to_the_authored_one() {
    const WORLD: &str = "assets/worlds/duel.toml";
    let text = std::fs::read_to_string(WORLD).expect("the duel world is readable");
    let raw: toml::Value = toml::from_str(&text).expect("the duel world parses");
    let authored = generated_in(
        raw["script"]["setup"]
            .as_str()
            .expect("the duel world's script block is a string"),
    );

    // The authored default: player cruiser + four courier escorts against
    // five destroyers (`--side-a`'s first entry is the player's own hull).
    let courier = "courier".to_string();
    let side_a = vec![
        "cruiser".to_string(),
        courier.clone(),
        courier.clone(),
        courier.clone(),
        courier,
    ];
    let side_b = vec!["destroyer".to_string(); MAX_SIDE];
    let world = apply_duel_sides(
        raw,
        &side_a,
        &side_b,
        &resolve_template,
        &DuelTemplateLoader,
    )
    .expect("the shipped duel world applies");

    assert_eq!(
        generated(&world),
        authored,
        "the harness's 5v5 must regenerate duel.toml's own authored roster exactly"
    );
}

/// Registration order below the marker mirrors the declarative file: every
/// side-A escort, then every side-B ship, then the victory trigger. This is
/// the trigger order the world digest's spawn sequence rides on.
#[test]
fn registrations_keep_the_declarative_order() {
    let five = vec![
        "a".to_string(),
        "b".into(),
        "c".into(),
        "d".into(),
        "e".into(),
    ];
    let world = apply_duel_sides(fixture(), &five, &five, &fake_resolve, &no_templates())
        .expect("5v5 applies");
    assert_eq!(
        registrations(&world),
        vec![
            "on_timer(0, \"spawn_side_a_2\");",
            "on_timer(0, \"spawn_side_a_3\");",
            "on_timer(0, \"spawn_side_a_4\");",
            "on_timer(0, \"spawn_side_a_5\");",
            "on_timer(0, \"spawn_side_b_1\");",
            "on_timer(0, \"spawn_side_b_2\");",
            "on_timer(0, \"spawn_side_b_3\");",
            "on_timer(0, \"spawn_side_b_4\");",
            "on_timer(0, \"spawn_side_b_5\");",
            "on_all_destroyed(\"side_b\", \"on_side_b_destroyed\");",
        ]
    );
}

#[test]
fn generates_no_slot_the_lists_do_not_reach() {
    // Player only on side A (no escorts), one ship on side B.
    let world = apply_duel_sides(
        fixture(),
        &["cruiser".into()],
        &["destroyer".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect("applies");
    let names: Vec<String> = drivers(&world).into_iter().map(|(n, ..)| n).collect();
    // No side-A escorts; side_b_2..5 beyond the list → never generated.
    assert_eq!(names, vec!["side_b_1".to_string()]);
    // side_b has a filled slot → the victory registration is emitted.
    assert!(has_victory(&world));
}

#[test]
fn empty_side_b_omits_the_victory_trigger() {
    // Degenerate: nobody on side B. The victory `on_all_destroyed` group
    // would be empty and fire immediately — so it is not registered.
    let world = apply_duel_sides(
        fixture(),
        &["cruiser".into()],
        &[],
        &fake_resolve,
        &no_templates(),
    )
    .expect("applies");
    assert!(
        !has_victory(&world),
        "empty side_b must not register the victory trigger"
    );
    // And no slots are generated at all.
    assert!(
        drivers(&world).is_empty(),
        "no slots should be generated, got {:?}",
        drivers(&world)
    );
}

#[test]
fn full_five_v_five_fills_every_slot() {
    let five = vec![
        "a".to_string(),
        "b".into(),
        "c".into(),
        "d".into(),
        "e".into(),
    ];
    let world = apply_duel_sides(fixture(), &five, &five, &fake_resolve, &no_templates())
        .expect("5v5 applies");
    let names: Vec<String> = drivers(&world).into_iter().map(|(n, ..)| n).collect();
    assert_eq!(
        names,
        vec![
            // side_a[0] is the player; escorts start at slot 2.
            "side_a_2", "side_a_3", "side_a_4", "side_a_5", //
            "side_b_1", "side_b_2", "side_b_3", "side_b_4", "side_b_5",
        ]
    );
    // The hulls track the list positions: side_a[1] = b → side_a_2,
    // side_b[0] = a → side_b_1.
    let s = drivers(&world);
    assert_eq!(s[0].1, "assets/entities/b.toml");
    assert_eq!(s[4].1, "assets/entities/a.toml");
}

/// The prelude — the authored `spawn_slot` body, the faction handler, the
/// marker — survives verbatim, and the old roster below it does not.
#[test]
fn the_authored_prelude_survives_and_the_old_roster_does_not() {
    let world = apply_duel_sides(
        fixture(),
        &["cruiser".into()],
        &["destroyer".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect("applies");
    let src = source(&world);
    assert!(src.contains("fn spawn_slot(ctx, name, template, faction, group)"));
    assert!(src.contains("on_world_loaded(\"on_load\");"));
    assert!(
        src.contains(SLOT_MARKER),
        "the marker stays, so a re-run is a no-op"
    );
    assert!(
        !src.contains("placeholder.toml"),
        "the authored default roster must be replaced, got:\n{src}"
    );
}

/// Re-running the transform over its own output produces the same thing —
/// the marker line is kept precisely so the seam survives a second pass.
#[test]
fn the_transform_is_idempotent() {
    let once = apply_duel_sides(
        fixture(),
        &["cruiser".into()],
        &["destroyer".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect("applies");
    let twice = apply_duel_sides(
        once.clone(),
        &["cruiser".into()],
        &["destroyer".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect("applies again");
    assert_eq!(once, twice);
}

#[test]
fn rejects_a_side_longer_than_five() {
    let six: Vec<String> = (0..6).map(|i| i.to_string()).collect();
    assert_eq!(
        apply_duel_sides(fixture(), &six, &[], &fake_resolve, &no_templates()).unwrap_err(),
        DuelError::TooManyShips {
            side: "a",
            count: 6
        }
    );
    assert_eq!(
        apply_duel_sides(
            fixture(),
            &["x".into()],
            &six,
            &fake_resolve,
            &no_templates()
        )
        .unwrap_err(),
        DuelError::TooManyShips {
            side: "b",
            count: 6
        }
    );
}

#[test]
fn an_unresolved_name_aborts_the_transform() {
    let err = apply_duel_sides(
        fixture(),
        &["cruiser".into()],
        &["unknown".into()],
        &fake_resolve,
        &no_templates(),
    )
    .unwrap_err();
    assert!(matches!(err, DuelError::Unresolved { name, .. } if name == "unknown"));
}

/// A slot whose staging anchor the arena never declared is rejected: the
/// spawn would warn and no-op, quietly leaving that ship out of the fight.
#[test]
fn a_slot_without_a_declared_anchor_is_rejected() {
    // Same fixture, but the `[anchors]` table stops at side_b_1.
    let trimmed = DUEL_FIXTURE.replace("side_b_2 = [55.0, 0.0, 30.0]\n", "");
    let err = apply_duel_sides(
        toml::from_str(&trimmed).expect("fixture parses"),
        &["cruiser".into()],
        &["destroyer".into(), "battleship".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect_err("an undeclared slot anchor must be rejected");
    assert_eq!(
        err,
        DuelError::UndeclaredSlotAnchor {
            slot: "side_b_2".to_string()
        }
    );
    // The message names the slot and how to fix it.
    let msg = err.to_string();
    assert!(msg.contains("side_b_2"), "got {msg:?}");
}

/// A literal-path ship name carrying backslashes (a Windows path handed
/// straight to `--side-b`) must emit a valid Rhai string, not an invalid
/// escape that fails the build-time script gate.
#[test]
fn a_backslashed_template_path_is_escaped_in_the_generated_driver() {
    let resolve = |name: &str| Ok(name.to_string());
    let world = apply_duel_sides(
        fixture(),
        &["cruiser".into()],
        &[r"assets\entities\x.toml".into()],
        &resolve,
        &no_templates(),
    )
    .expect("applies");
    assert!(
        generated(&world).contains(r"assets\\entities\\x.toml"),
        "got:\n{}",
        generated(&world)
    );
}

/// Quotes and raw control characters get the same treatment: a path
/// carrying either would otherwise close the generated literal early or
/// break the line, and the run would die in the build-time script gate with
/// a Rhai parse error pointing at source the user never wrote.
#[test]
fn rhai_str_escapes_quotes_and_control_characters() {
    assert_eq!(rhai_str(r#"a"b"#), r#"a\"b"#);
    assert_eq!(rhai_str("a\nb"), r"a\nb");
    assert_eq!(rhai_str("a\r\nb"), r"a\r\nb");
    assert_eq!(rhai_str("a\tb"), r"a\tb");
    // The backslash pass runs FIRST, so an authored backslash-n stays two
    // characters and does not collide with the newline escape above.
    assert_eq!(rhai_str(r"a\nb"), r"a\\nb");
}

/// A newline in a ship name reaches the generated driver as `\n`, not as a
/// raw line break that would split the driver in half.
#[test]
fn a_newline_in_a_template_path_is_escaped_in_the_generated_driver() {
    let resolve = |name: &str| Ok(name.to_string());
    let world = apply_duel_sides(
        fixture(),
        &["cruiser".into()],
        &["a\nb.toml".into()],
        &resolve,
        &no_templates(),
    )
    .expect("applies");
    let gen = generated(&world);
    assert!(gen.contains(r"a\nb.toml"), "got:\n{gen}");
    assert_eq!(
        gen.lines().filter(|l| l.starts_with("fn spawn_")).count(),
        1,
        "the driver must stay on one line:\n{gen}"
    );
}

// ── The marked world's half of the contract ──────────────────────────────

/// A world that marks a `[script]` block but authors no `spawn_slot`
/// compiles CLEAN — the generated registrations are valid Rhai — and then
/// discards every call at t=0, producing an empty arena scored as a draw.
/// It is rejected up front instead, naming the fn.
#[test]
fn a_marked_world_without_the_spawn_slot_body_is_rejected() {
    let stripped = DUEL_FIXTURE.replace("fn spawn_slot(", "fn spawn_slot_renamed(");
    let err = apply_duel_sides(
        toml::from_str(&stripped).expect("fixture parses"),
        &["cruiser".into()],
        &["destroyer".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect_err("a world without spawn_slot must be rejected");
    assert_eq!(
        err,
        DuelError::MissingSlotFn {
            name: "spawn_slot",
            needed_for: "every generated slot driver delegates to it",
        }
    );
    let msg = err.to_string();
    assert!(msg.contains("fn spawn_slot("), "got {msg:?}");
}

/// The victory handler is only part of the contract when side B has anyone
/// on it — that is the only case the registration naming it is emitted.
#[test]
fn the_victory_handler_is_required_exactly_when_side_b_is_filled() {
    let stripped =
        DUEL_FIXTURE.replace("fn on_side_b_destroyed(", "fn on_side_b_destroyed_renamed(");
    let world = || toml::from_str::<toml::Value>(&stripped).expect("fixture parses");

    let err = apply_duel_sides(
        world(),
        &["cruiser".into()],
        &["destroyer".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect_err("a filled side B needs the victory handler");
    assert_eq!(
        err,
        DuelError::MissingSlotFn {
            name: "on_side_b_destroyed",
            needed_for: "a non-empty --side-b registers it as the victory handler",
        }
    );

    // Empty side B emits no victory registration, so nothing names it.
    apply_duel_sides(
        world(),
        &["cruiser".into()],
        &[],
        &fake_resolve,
        &no_templates(),
    )
    .expect("an empty side B needs no victory handler");
}

/// Two marker lines make the truncation point a coin toss — and whichever
/// loses, the roster under it survives into the output as authored text the
/// harness did not generate. Rejected.
#[test]
fn a_source_with_two_markers_is_rejected() {
    let doubled = DUEL_FIXTURE.replace(
        "// duel:slots\n",
        "// duel:slots\non_timer(0, \"spawn_nothing\");\n// duel:slots\n",
    );
    let err = apply_duel_sides(
        toml::from_str(&doubled).expect("fixture parses"),
        &["cruiser".into()],
        &["destroyer".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect_err("two markers must be rejected");
    assert_eq!(
        err,
        DuelError::DuplicateSlotMarker {
            key: "setup".to_string(),
            count: 2,
        }
    );
}

/// The marker is a whole line. A line that merely NAMES it — the commentary
/// above the seam in `duel.toml` does exactly this — must not become the
/// truncation point, or that world's real roster would be eaten silently.
#[test]
fn a_line_merely_mentioning_the_marker_is_not_a_seam() {
    let prose = DUEL_FIXTURE.replace(
        "// duel:slots\n",
        "// LOAD-BEARING: the // duel:slots line below is the seam.\n",
    );
    assert_eq!(
        apply_duel_sides(
            toml::from_str(&prose).expect("fixture parses"),
            &["cruiser".into()],
            &["destroyer".into()],
            &fake_resolve,
            &no_templates(),
        )
        .unwrap_err(),
        DuelError::NoDuelSlots
    );
}

/// A marker line at end-of-source carries no newline of its own. The kept
/// prelude gets one, or the first registration would land ON the marker
/// line — behind its `//`, commented out along with everything the
/// generator emitted after it on that line.
#[test]
fn a_marker_line_without_a_trailing_newline_still_separates_the_roster() {
    const AT_EOF: &str = r#"
[anchors]
side_b_1 = [0.0, 0.0, 0.0]

[script]
setup = "fn spawn_slot(ctx, n, t, f, g) {}\nfn on_side_b_destroyed(ctx) {}\n// duel:slots"
"#;
    let world = apply_duel_sides(
        toml::from_str(AT_EOF).expect("fixture parses"),
        &["cruiser".into()],
        &["destroyer".into()],
        &fake_resolve,
        &no_templates(),
    )
    .expect("applies");
    assert!(
        source(&world).ends_with(
            "// duel:slots\n\
                 on_timer(0, \"spawn_side_b_1\");\n\
                 on_all_destroyed(\"side_b\", \"on_side_b_destroyed\");\n\
                 \n\
                 fn spawn_side_b_1(ctx) { spawn_slot(ctx, \"side_b_1\", \
                 \"assets/entities/destroyer.toml\", \
                 \"cccccccc-3333-4333-8333-cccccccccccc\", \"side_b\"); }\n"
        ),
        "got:\n{}",
        source(&world)
    );
}

// ── The #888 doctrine-anchor guard, harness side ─────────────────────────

/// A hull carrying a patrol route named for the scenario it normally fights
/// in: `--side-b` can field it anywhere, and the route comes along.
const ROUTED_HULL: &[(&str, &str)] = &[(
    "assets/entities/warhawk.toml",
    r#"
[[doctrine]]
id = "patrol-warhawk"
directive_kind = "Patrol"
directive_anchors = ["ghost_route_a"]
base_priority = 20.0
"#,
)];

/// The #888 guard, re-run at the harness because the load-time validator
/// cannot see a script-authored slot: a fielded hull steering to an anchor
/// this arena never staged is a hard error naming the anchor, not a ship
/// that arrives with a goal resolving to nothing.
#[test]
fn a_fielded_hull_whose_route_the_arena_never_staged_is_rejected() {
    let err = apply_duel_sides(
        fixture(),
        &["cruiser".into()],
        &["warhawk".into()],
        &fake_resolve,
        &fake_templates(ROUTED_HULL),
    )
    .expect_err("an undeclared doctrine anchor must be rejected");
    assert_eq!(
        err,
        DuelError::UndeclaredDoctrineAnchor {
            slot: "side_b_1".to_string(),
            template: "assets/entities/warhawk.toml".to_string(),
            anchor: "ghost_route_a".to_string(),
            kind: "Patrol",
        }
    );
    // The message names the anchor to declare and the slot that wanted it.
    let msg = err.to_string();
    assert!(msg.contains("ghost_route_a"), "got {msg:?}");
    assert!(msg.contains("side_b_1"), "got {msg:?}");
}

/// …and the same hull is accepted once the arena declares the route, which
/// is exactly what `duel.toml`'s `warhawk_patrol_*` anchors are for.
#[test]
fn a_fielded_hull_is_accepted_once_the_arena_declares_its_route() {
    let staged = DUEL_FIXTURE.replace(
        "[player_spawn]",
        "ghost_route_a = [55.0, 0.0, 30.0]\n\n[player_spawn]",
    );
    apply_duel_sides(
        toml::from_str(&staged).expect("fixture parses"),
        &["cruiser".into()],
        &["warhawk".into()],
        &fake_resolve,
        &fake_templates(ROUTED_HULL),
    )
    .expect("a declared route applies");
}

// ── resolve_template: the documented order, filesystem-backed ────────────

#[test]
fn resolve_prefers_alliance_prefixed_then_bare_then_literal() {
    // Alliance-prefixed wins: `cruiser` → alliance_cruiser.toml.
    assert_eq!(
        resolve_template("cruiser").unwrap(),
        "assets/entities/alliance_cruiser.toml"
    );
    // Bare template: `ship_harrow_warhawk` has no alliance_ prefix on disk.
    assert_eq!(
        resolve_template("ship_harrow_warhawk").unwrap(),
        "assets/entities/ship_harrow_warhawk.toml"
    );
    // Literal path: a full path that exists is taken verbatim.
    assert_eq!(
        resolve_template("assets/entities/ship_harrow_patrol.toml").unwrap(),
        "assets/entities/ship_harrow_patrol.toml"
    );
}

#[test]
fn resolve_lists_every_tried_path_on_failure() {
    let err = resolve_template("nonesuch").unwrap_err();
    let DuelError::Unresolved { tried, .. } = &err else {
        panic!("expected Unresolved, got {err:?}");
    };
    assert_eq!(
        tried,
        &[
            "assets/entities/alliance_nonesuch.toml".to_string(),
            "assets/entities/nonesuch.toml".to_string(),
            "nonesuch".to_string(),
        ]
    );
    // The Display form names all three, in order.
    let msg = err.to_string();
    assert!(msg.contains("alliance_nonesuch.toml"), "{msg}");
    assert!(msg.contains("\"nonesuch\""), "{msg}");
}
