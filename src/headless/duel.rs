//! `--side-a` / `--side-b` class-matchup transform for `duel.toml` (issue #844;
//! retyped onto the Rhai front-end by issue #984, M6).
//!
//! `duel.toml` authors its NPC combatants in one inline `[script]` block: a
//! parameterised spawn body (`spawn_slot`) written ONCE, plus a roster of
//! one-line *drivers* that call it — one per filled slot — under a
//! [`SLOT_MARKER`] line. [`apply_duel_sides`] takes the **raw world TOML** and
//! the two CLI ship lists, truncates that script source at the marker, and
//! regenerates the roster below it from the lists. It is still a pure transform,
//! now over `toml::Value` rather than `WorldConfig`, and it still runs once
//! before the world boots.
//!
//! ## Why the raw source and not the parsed config
//!
//! A converted world's spawn slots are no longer `[[trigger]]` blocks —
//! `parse_world` never sees them, so `WorldConfig::triggers` is empty of slots
//! and there is nothing there to fill or delete. Registration happens when the
//! script loader runs the unit's TOP LEVEL (`compile_scripts`), which is also why
//! the transform cannot be a runtime decision: a script cannot conditionally skip
//! registering the side-B victory trigger from data that only exists at runtime.
//! Editing the source string before the loader reads it is the one seam that
//! keeps both properties.
//!
//! The whole mechanism lives inside ONE script unit because it has to: ASTs are
//! keyed per source path and each unit's top level runs separately, so a second
//! `[script.<key>]` block cannot call a helper defined in `[script.setup]`.
//!
//! ## The authored body and the generated drivers
//!
//! The transform generates only the drivers, never the spawn body:
//!
//! ```rhai
//! on_timer(0, "spawn_side_b_1");
//! on_all_destroyed("side_b", "on_side_b_destroyed");
//!
//! fn spawn_side_b_1(ctx) { spawn_slot(ctx, "side_b_1", "assets/entities/alliance_destroyer.toml", "<harrow-uuid>", "side_b"); }
//! ```
//!
//! `spawn_slot` — the doctrine, the anchor convention, the override shape — is
//! authored in `duel.toml` and is the same body the un-harnessed default roster
//! calls. So `--side-a`/`--side-b` cannot drift from the arena's own doctrine the
//! way a Rust-side generator of whole spawn bodies would. A helper fn's effects
//! land in its CALLER's buffer (`EffectSink` is an `Arc<Mutex<_>>` shared through
//! the copied `ctx`), which is what makes the delegation work at all; pinned by
//! `world::script::effects`'s `a_helper_fn_shares_the_callers_effect_buffer`.
//!
//! Registration order below the marker mirrors the declarative file exactly —
//! side-A escorts, then side-B, then the victory `on_all_destroyed` — so the
//! trigger indices, and with them the spawn order the world digest folds, are
//! unchanged by the conversion. Rhai function definitions are position
//! independent, so the drivers may follow their own registrations.
//!
//! ## The player decides side A
//!
//! Side-A slot 1 is the player's own ship (the `LocalShip`), set from
//! `side_a[0]` via `--ship`/`PendingShipConfig`, **not** a spawn slot. The run
//! ends in DEFEAT when that ship dies — the engine's built-in player-death
//! latch (`GamePhase::GameOver`, `Outcome::Defeat`), never an
//! `on_all_destroyed group = "side_a"` trigger. Authoring one would be wrong
//! two ways: an empty `side_a` group (the common 1vN case, side A player-only)
//! fires `on_all_destroyed` *immediately* (empty-group = already-destroyed),
//! and even with escorts the player — not in the group — may still be alive.
//! So `duel.toml` never authors a side-A victory/defeat trigger; escort deaths
//! don't end the run, the player's death does.
//!
//! VICTORY is `on_all_destroyed group = "side_b"` → `game_over` victory. The
//! transform simply DOES NOT EMIT that registration when `side_b` is empty, so a
//! degenerate `--side-b` can't instant-fire a false victory off an empty group.
//! (Declaratively this was a deletion; generating the roster makes it an
//! omission, which is the same guard stated the other way round.)

use crate::logging::LogCat;

/// The marker the harness truncates `duel.toml`'s script source at.
///
/// Everything from the START of the line carrying this marker is kept; every
/// byte after that line is discarded and regenerated from `--side-a`/`--side-b`.
/// A world whose `[script]` blocks carry no such marker is REJECTED
/// ([`DuelError::NoDuelSlots`]) rather than silently run with the ship lists
/// ignored, so the mechanism can never fail quietly.
///
/// Keeping the marker line in the output makes the transform idempotent.
pub const SLOT_MARKER: &str = "// duel:slots";

/// Federation faction UUID — side A's own faction (the player's). Side-A NPC
/// escorts are forced to it so they bucket as the player side in the report.
/// A faction *identity* reference, matching `assets/factions/federation.toml`
/// (mirrors how `probe_duel.toml` embeds the Harrow UUID inline) — not a
/// tunable gameplay value.
pub const FEDERATION_FACTION: &str = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";

/// Harrow faction UUID — side B's faction. Matches
/// `assets/factions/harrow.toml`. Federation<->Harrow mutual hostility is
/// armed by the pre-authored `add_faction_enemy` calls in `duel.toml`'s
/// `on_world_loaded` handler, above the marker and so never regenerated.
pub const HARROW_FACTION: &str = "cccccccc-3333-4333-8333-cccccccccccc";

/// Maximum ships per side. Side A is one player (slot 1) plus up to four NPC
/// escort slots (`side_a_2`..`side_a_5`); side B is up to five NPC slots
/// (`side_b_1`..`side_b_5`).
pub const MAX_SIDE: usize = 5;

/// Anything that stopped a duel side being applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DuelError {
    /// A side listed more than [`MAX_SIDE`] ships.
    TooManyShips { side: &'static str, count: usize },
    /// A ship name resolved to none of the candidate paths.
    Unresolved { name: String, tried: Vec<String> },
    /// No `[script]` block in the world carries the [`SLOT_MARKER`], so there is
    /// nowhere to generate the slot drivers the ship lists ask for.
    ///
    /// Without this the transform was a silent no-op: `--side-a cruiser
    /// --side-b destroyer --world <some non-duel world>` loaded that world
    /// untouched and produced a combat-free draw that reads like a balance
    /// finding. Naming the missing marker is the whole point of the error.
    NoDuelSlots,
    /// A generated slot names an anchor the world's `[anchors]` table never
    /// declares.
    ///
    /// The declarative slots carried their own `anchor = "…"` and so could only
    /// name what the arena had staged; a generated driver names the anchor
    /// itself, and `dispatch_spawn_entity` answers an unresolvable one with a
    /// warning and no spawn — the ship would simply not be there, and the run
    /// would read as a lopsided balance result. Rejecting up front says which
    /// staging coordinate is missing.
    UndeclaredSlotAnchor { slot: String },
}

impl std::fmt::Display for DuelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DuelError::TooManyShips { side, count } => write!(
                f,
                "--side-{side} lists {count} ships; the maximum is {MAX_SIDE} per side"
            ),
            DuelError::Unresolved { name, tried } => write!(
                f,
                "could not resolve ship {name:?}; tried, in order: {}",
                tried
                    .iter()
                    .map(|p| format!("{p:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            DuelError::NoDuelSlots => write!(
                f,
                "--side-a/--side-b need a duel-shaped world: no [script] block in \
                 this one carries the `{SLOT_MARKER}` marker the harness \
                 regenerates the side_a_*/side_b_* slot drivers below. Drop \
                 --world to use the duel harness (assets/worlds/duel.toml), or \
                 author the marker in a [script] block of your own"
            ),
            DuelError::UndeclaredSlotAnchor { slot } => write!(
                f,
                "slot {slot:?} has no [anchors] entry in this world; a slot spawns \
                 on the anchor of its own name, and an undeclared one would leave \
                 that ship out of the fight. Declare {slot} = [x, y, z]"
            ),
        }
    }
}

impl std::error::Error for DuelError {}

/// One generated slot driver: which arena slot, which hull, whose side.
struct Slot {
    /// Slot name — also the `[anchors]` key it stages on and the driver fn's
    /// suffix (`side_b_1` → `spawn_side_b_1`).
    name: String,
    /// Resolved template path for the hull filling it.
    template: String,
    /// Faction UUID forced onto the hull, so the report buckets it on the right
    /// side regardless of the template's own faction.
    faction: &'static str,
    /// `on_all_destroyed` group the spawn joins.
    group: &'static str,
}

/// Render `s` as the body of a Rhai string literal.
///
/// The generated drivers embed resolved template paths, and `resolve_template`'s
/// third candidate is the name *as given* — so a literal Windows path
/// (`assets\entities\x.toml`) would otherwise emit `\e`, an invalid Rhai escape,
/// and fail the build-time script gate with a parse error rather than running.
fn rhai_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// The `[script]` table key whose source carries [`SLOT_MARKER`].
///
/// `None` for a world with no `script` key, a sibling `script = "file.rhai"`
/// (the harness edits an inline block; a sibling file is not the world's to
/// rewrite), or a `[script]` table in which no entry carries the marker. Table
/// iteration is sorted (toml maps are `BTreeMap`s), so a world that marked two
/// blocks resolves to the same one every run.
fn marked_script_key(raw: &toml::Value) -> Option<String> {
    raw.get("script")?
        .as_table()?
        .iter()
        .find(|(_, v)| v.as_str().is_some_and(|s| s.contains(SLOT_MARKER)))
        .map(|(k, _)| k.clone())
}

/// Whether the world's `[anchors]` table declares `slot`.
fn declares_anchor(raw: &toml::Value, slot: &str) -> bool {
    raw.get("anchors")
        .and_then(|a| a.as_table())
        .is_some_and(|t| t.contains_key(slot))
}

/// The slots the two ship lists fill, in the declarative file's authored order:
/// side-A escorts (`side_a[1..]` → `side_a_2`..), then side B (`side_b[..]` →
/// `side_b_1`..). `side_a[0]` is the player's own hull and fills no slot.
fn slots_for(
    side_a: &[String],
    side_b: &[String],
    resolve: &impl Fn(&str) -> Result<String, DuelError>,
) -> Result<Vec<Slot>, DuelError> {
    let mut slots = Vec::with_capacity(side_a.len().saturating_sub(1) + side_b.len());
    for (i, ship) in side_a.iter().enumerate().skip(1) {
        slots.push(Slot {
            name: format!("side_a_{}", i + 1),
            template: resolve(ship)?,
            faction: FEDERATION_FACTION,
            group: "side_a",
        });
    }
    for (i, ship) in side_b.iter().enumerate() {
        slots.push(Slot {
            name: format!("side_b_{}", i + 1),
            template: resolve(ship)?,
            faction: HARROW_FACTION,
            group: "side_b",
        });
    }
    Ok(slots)
}

/// Render the Rhai the transform appends below the marker: one `on_timer(0, …)`
/// registration per filled slot, the side-B victory registration when side B is
/// non-empty, then the one-line drivers that delegate to the authored
/// `spawn_slot`.
///
/// Registrations come first only for readability — Rhai resolves function
/// definitions independently of position — but their ORDER is load-bearing: it
/// is the trigger order the declarative file authored, and therefore the spawn
/// order the world digest folds.
fn render_drivers(slots: &[Slot], side_b_filled: bool) -> String {
    let mut out = String::from("\n");
    for slot in slots {
        out.push_str(&format!("on_timer(0, \"spawn_{}\");\n", slot.name));
    }
    if side_b_filled {
        out.push_str("on_all_destroyed(\"side_b\", \"on_side_b_destroyed\");\n");
    }
    if !slots.is_empty() {
        out.push('\n');
    }
    for slot in slots {
        out.push_str(&format!(
            "fn spawn_{}(ctx) {{ spawn_slot(ctx, \"{}\", \"{}\", \"{}\", \"{}\"); }}\n",
            slot.name,
            rhai_str(&slot.name),
            rhai_str(&slot.template),
            slot.faction,
            slot.group,
        ));
    }
    out
}

/// Regenerate `duel.toml`'s slot drivers from the CLI ship lists — a pure
/// transform over the raw world [`toml::Value`].
///
/// - `side_a[0]` is the player ship (applied elsewhere via `--ship`); only
///   `side_a[1..]` fill escort slots.
/// - `side_b[..]` fill side-B slots.
/// - Slots the lists do not reach are simply not generated.
/// - The side-B victory `on_all_destroyed` is emitted only when side B has at
///   least one ship, so an empty side never instant-fires.
/// - Rejects either side longer than [`MAX_SIDE`], a world with no
///   [`SLOT_MARKER`], and a slot whose anchor the world does not declare.
///
/// `resolve` turns a ship name into a template path; inject a fake in tests,
/// [`resolve_template`] in production.
pub fn apply_duel_sides(
    mut raw: toml::Value,
    side_a: &[String],
    side_b: &[String],
    resolve: &impl Fn(&str) -> Result<String, DuelError>,
) -> Result<toml::Value, DuelError> {
    if side_a.len() > MAX_SIDE {
        return Err(DuelError::TooManyShips {
            side: "a",
            count: side_a.len(),
        });
    }
    if side_b.len() > MAX_SIDE {
        return Err(DuelError::TooManyShips {
            side: "b",
            count: side_b.len(),
        });
    }

    // Reject a world the harness has no seam in before touching it, rather than
    // running it untransformed with the ship lists silently ignored.
    let key = marked_script_key(&raw).ok_or(DuelError::NoDuelSlots)?;

    let slots = slots_for(side_a, side_b, resolve)?;
    // A generated slot names its own anchor, so the arena has to have staged it.
    for slot in &slots {
        if !declares_anchor(&raw, &slot.name) {
            return Err(DuelError::UndeclaredSlotAnchor {
                slot: slot.name.clone(),
            });
        }
    }
    let drivers = render_drivers(&slots, !side_b.is_empty());

    let table = raw
        .get_mut("script")
        .and_then(|s| s.as_table_mut())
        .expect("the marked key came from this table");
    let source = table
        .get(&key)
        .and_then(|v| v.as_str())
        .expect("the marked entry is a string");
    // Keep the marker line itself, so the output is still marked and a second
    // pass over it is a no-op.
    let cut = source.find(SLOT_MARKER).expect("the marker was just found");
    let prelude_end = source[cut..]
        .find('\n')
        .map_or(source.len(), |i| cut + i + 1);
    let composed = format!("{}{drivers}", &source[..prelude_end]);

    bevy::log::debug!(
        target: LogCat::Config.target(),
        "duel harness: regenerated [script.{key}] slot drivers:{drivers}"
    );
    table.insert(key, toml::Value::String(composed));
    Ok(raw)
}

/// Resolve a ship name to a template path, filesystem-backed.
///
/// Documented order (issue #844):
/// 1. alliance-prefixed template — `assets/entities/alliance_<name>.toml`
/// 2. bare template — `assets/entities/<name>.toml`
/// 3. literal path — `<name>` as given, if it exists
///
/// The first candidate that exists on disk wins. When none do, the error lists
/// all three paths that were tried, in order.
pub fn resolve_template(name: &str) -> Result<String, DuelError> {
    let candidates = [
        format!("assets/entities/alliance_{name}.toml"),
        format!("assets/entities/{name}.toml"),
        name.to_string(),
    ];
    for candidate in &candidates {
        if std::path::Path::new(candidate).exists() {
            return Ok(candidate.clone());
        }
    }
    Err(DuelError::Unresolved {
        name: name.to_string(),
        tried: candidates.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
    ctx.effects.add_faction_enemy("Federation", "Harrow");
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
    /// marker line.
    fn generated(world: &toml::Value) -> String {
        let src = source(world);
        let cut = src.find(SLOT_MARKER).expect("the marker survives");
        let after = src[cut..].find('\n').map_or(src.len(), |i| cut + i + 1);
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
fn on_load(ctx) { ctx.effects.add_faction_enemy("Federation", "Harrow"); }
"""
"#;
        let err = apply_duel_sides(
            toml::from_str(NO_SLOTS).expect("fixture parses"),
            &["cruiser".into()],
            &["destroyer".into()],
            &fake_resolve,
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
            apply_duel_sides(world, &["cruiser".into()], &[], &fake_resolve).unwrap_err(),
            DuelError::NoDuelSlots
        );
    }

    /// A sibling `script = "file.rhai"` is not an inline block, so there is
    /// nothing here to rewrite — reject rather than edit a file beside the world.
    #[test]
    fn a_sibling_script_file_is_rejected() {
        let world = toml::from_str("script = \"duel.rhai\"\n").expect("parses");
        assert_eq!(
            apply_duel_sides(world, &["cruiser".into()], &[], &fake_resolve).unwrap_err(),
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
        )
        .expect("applies");

        assert_eq!(
            drivers(&world),
            vec![
                // side_a_2 from side_a[1] = courier, Federation faction.
                (
                    "side_a_2".to_string(),
                    "assets/entities/courier.toml".to_string(),
                    FEDERATION_FACTION.to_string(),
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
        )
        .expect("applies");
        assert_eq!(
            generated(&world),
            "\n\
             on_timer(0, \"spawn_side_b_1\");\n\
             on_all_destroyed(\"side_b\", \"on_side_b_destroyed\");\n\
             \n\
             fn spawn_side_b_1(ctx) { spawn_slot(ctx, \"side_b_1\", \
             \"assets/entities/destroyer.toml\", \
             \"cccccccc-3333-4333-8333-cccccccccccc\", \"side_b\"); }\n"
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
        let world = apply_duel_sides(fixture(), &five, &five, &fake_resolve).expect("5v5 applies");
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
        let world =
            apply_duel_sides(fixture(), &["cruiser".into()], &[], &fake_resolve).expect("applies");
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
        let world = apply_duel_sides(fixture(), &five, &five, &fake_resolve).expect("5v5 applies");
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
        )
        .expect("applies");
        let twice = apply_duel_sides(
            once.clone(),
            &["cruiser".into()],
            &["destroyer".into()],
            &fake_resolve,
        )
        .expect("applies again");
        assert_eq!(once, twice);
    }

    #[test]
    fn rejects_a_side_longer_than_five() {
        let six: Vec<String> = (0..6).map(|i| i.to_string()).collect();
        assert_eq!(
            apply_duel_sides(fixture(), &six, &[], &fake_resolve).unwrap_err(),
            DuelError::TooManyShips {
                side: "a",
                count: 6
            }
        );
        assert_eq!(
            apply_duel_sides(fixture(), &["x".into()], &six, &fake_resolve).unwrap_err(),
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
        )
        .expect("applies");
        assert!(
            generated(&world).contains(r"assets\\entities\\x.toml"),
            "got:\n{}",
            generated(&world)
        );
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
}
