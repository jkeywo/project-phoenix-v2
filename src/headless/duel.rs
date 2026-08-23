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
//! ## What the harness refuses, and why it has to
//!
//! Every guard here answers the same failure shape: a run that produces
//! *numbers* rather than an error, and reads as a balance finding. A world with
//! no marker (or two), a marked world that authors none of the bodies the
//! generated roster calls, a slot the arena never staged, a hull whose own
//! doctrine steers to an anchor this arena never staged — each of those
//! otherwise ends in a warning nobody reads and a ship that is not in the fight.
//!
//! The last of those is issue #888's load-time guard, re-run here because it can
//! no longer reach these hulls: `world::validate`'s
//! `validate_doctrine_anchors_in` walks the *declarative* config, and a
//! script-authored slot is not in it. The harness checks each hull it fields
//! through the same `world::validate::template_doctrine_anchors` table
//! (`DuelError::UndeclaredDoctrineAnchor`).
//!
//! ## Issue #1046 closed the general gap, and this world is the exception
//!
//! `collect_spawned_instances` now walks scripted `spawn_entity` calls too, so
//! a hull a script spawns is template-resolved and anchor-checked like a
//! declarative one — across the shipped worlds, 41 references the gate could not
//! previously see.
//!
//! `duel.toml` contributes ZERO of them, harnessed or not, and its authored
//! default roster is NOT covered despite being ordinary authored content. Every
//! slot in this world — the ones below the marker as much as the ones the CLI
//! generates — reaches the same one `spawn_slot` body, so the only
//! `template_path:` in the file reads `template_path: template`. The scan sees a
//! computed path and correctly declines to guess.
//!
//! The reason it cannot be fixed by a stronger scan is structural. That gate
//! reads a literal `template_path:`
//! out of a `spawn_entity` MAP. The generated drivers do not carry one: they
//! pass the hull POSITIONALLY to `duel.toml`'s own `spawn_slot(ctx, name,
//! template, faction, group)`, which is the whole point of the design above —
//! the harness generates drivers, never spawn bodies, so `--side-a`/`--side-b`
//! cannot drift from the arena's doctrine. Inside `spawn_slot` the map reads
//! `template_path: template`, a computed path no load-time scan can resolve.
//! Following the literal from the call site to the parameter is interprocedural
//! constant propagation over a Rhai AST this build cannot even walk (Rhai's
//! `internals` feature is off — see `world::validate::collect_spawned_instances`).
//!
//! So the per-slot check stays, and stays the only thing standing between a
//! `--side-b ship_harrow_warhawk` and a ship pursuing a goal that resolves to
//! nothing. It shares `template_doctrine_anchors` with the load-time gate, so
//! the two cannot disagree about which fields are anchors; what differs is only
//! which spawns each can see.
//!
//! VICTORY is `on_all_destroyed group = "side_b"` → `game_over` victory. The
//! transform simply DOES NOT EMIT that registration when `side_b` is empty, so a
//! degenerate `--side-b` can't instant-fire a false victory off an empty group.
//! (Declaratively this was a deletion; generating the roster makes it an
//! omission, which is the same guard stated the other way round.)

use crate::logging::LogCat;

/// The marker the harness truncates `duel.toml`'s script source at.
///
/// The source is kept up to and including the marker line; every byte after it
/// is discarded and regenerated from `--side-a`/`--side-b`. Keeping the marker
/// line itself is what makes the transform idempotent.
///
/// A marker is a WHOLE trimmed line (`// duel:slots` and nothing else), so
/// prose that merely names the marker — this module's own docs, `duel.toml`'s
/// commentary above the seam — cannot become one by accident. A world whose
/// `[script]` blocks carry no such line is REJECTED
/// ([`DuelError::NoDuelSlots`]) rather than silently run with the ship lists
/// ignored, and a source carrying two is rejected as well
/// ([`DuelError::DuplicateSlotMarker`]): with two seams the truncation point is
/// a coin toss, and the roster below the *first* one would survive into the
/// output as unreachable authored text.
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
    /// A hull fielded into a slot carries a doctrine directive steering to an
    /// anchor the world's `[anchors]` table never declares (issue #888's guard,
    /// applied to harnessed runs — issue #984 review).
    ///
    /// `validate_doctrine_anchors_in` fails the LOAD over exactly this, but it
    /// walks the declarative config, where a script-authored slot does not
    /// appear — so a CLI-fielded hull escaped it once the duel slots became
    /// script. The check is re-run here, over the same
    /// `world::validate::template_doctrine_anchors` table, so `--side-b
    /// ship_harrow_warhawk` in an arena that never staged its patrol route is
    /// still a hard error naming the anchor rather than a ship pursuing a goal
    /// that resolves to nothing.
    UndeclaredDoctrineAnchor {
        /// The slot the hull was fielded into.
        slot: String,
        /// The hull's resolved template path.
        template: String,
        /// The anchor its doctrine steers to.
        anchor: String,
        /// The directive kind that reads the anchor (`Patrol`, `Reach`,
        /// `Retreat`).
        kind: &'static str,
    },
    /// The marked source does not author a fn the generated roster calls.
    ///
    /// The generated drivers delegate to the world's own `spawn_slot`, and the
    /// side-B victory registration names `on_side_b_destroyed`; neither is
    /// generated, both are the marked world's to author. A world that marks a
    /// `[script]` block but authors no `spawn_slot` compiles CLEAN — the
    /// registrations are valid Rhai — and then `RuntimeHost` warns and discards
    /// each call at t=0, producing an EMPTY ARENA that reads as a combat-free
    /// draw. That is the same silent-nothing failure [`DuelError::NoDuelSlots`]
    /// exists to prevent, one level in.
    MissingSlotFn {
        /// The fn the source has to author (`spawn_slot`,
        /// `on_side_b_destroyed`).
        name: &'static str,
        /// Why the roster needs it, for the message.
        needed_for: &'static str,
    },
    /// The marked source carries the [`SLOT_MARKER`] line more than once, so
    /// which of them is the seam is ambiguous.
    DuplicateSlotMarker {
        /// The `[script]` table key whose source carries them.
        key: String,
        /// How many marker lines it carries.
        count: usize,
    },
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
                 this one carries a `{SLOT_MARKER}` line (on its own) for the \
                 harness to regenerate the side_a_*/side_b_* slot drivers below. \
                 Drop --world to use the duel harness (assets/worlds/duel.toml), \
                 or author the full contract in a [script] block of your own: the \
                 marker line, a `fn spawn_slot(ctx, name, template, faction, \
                 group)` body for the generated drivers to delegate to, and a `fn \
                 on_side_b_destroyed(ctx)` for the victory registration a \
                 non-empty --side-b emits"
            ),
            DuelError::UndeclaredSlotAnchor { slot } => write!(
                f,
                "slot {slot:?} has no [anchors] entry in this world; a slot spawns \
                 on the anchor of its own name, and an undeclared one would leave \
                 that ship out of the fight. Declare {slot} = [x, y, z]"
            ),
            DuelError::UndeclaredDoctrineAnchor {
                slot,
                template,
                anchor,
                kind,
            } => write!(
                f,
                "the hull fielded into slot {slot:?} ({template:?}) carries a \
                 {kind} doctrine directive steering to anchor {anchor:?}, which \
                 this world's [anchors] table does not declare; the ship would \
                 arrive with a goal that resolves to nothing. Declare {anchor} = \
                 [x, y, z] in the world, or stand that doctrine entry down"
            ),
            DuelError::MissingSlotFn { name, needed_for } => write!(
                f,
                "the marked [script] block authors no `fn {name}(`, which {needed_for}; \
                 the harness generates only the one-line drivers, never the bodies \
                 they call, so this world would compile clean and then discard every \
                 call at runtime. Author it beside the `{SLOT_MARKER}` marker"
            ),
            DuelError::DuplicateSlotMarker { key, count } => write!(
                f,
                "[script.{key}] carries {count} `{SLOT_MARKER}` lines; the harness \
                 truncates at THE marker and cannot tell which of them is the seam. \
                 Keep exactly one"
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
///
/// Backslash first (so the escapes introduced below are not re-escaped), then
/// the quote, then the line/tab control characters. A raw newline inside a Rhai
/// string literal is a parse error, and a path carrying one — pathological, but
/// `--side-b "$(printf 'a\nb')"` is a shell away — would otherwise fail the
/// build-time script gate with an opaque Rhai diagnostic pointing at generated
/// source the user never wrote. Escaped, it stays a well-formed literal and the
/// complaint lands where it belongs: an unresolvable template path.
fn rhai_str(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// The end offsets (one past the newline) of every line in `source` that IS the
/// [`SLOT_MARKER`] — the whole line, trimmed, and nothing else.
///
/// Whole-line matching rather than `contains`: the marker names itself in prose
/// all over this repo (`duel.toml`'s own commentary above the seam, these docs),
/// and a substring match would let such a line become the truncation point and
/// silently eat the roster below it. The offsets are line ENDS because the
/// marker line is kept in the output.
fn marker_line_ends(source: &str) -> Vec<usize> {
    let mut ends = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if line.trim() == SLOT_MARKER {
            ends.push(offset + line.len());
        }
        offset += line.len();
    }
    ends
}

/// The `[script]` table key whose source carries a [`SLOT_MARKER`] line.
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
        .find(|(_, v)| v.as_str().is_some_and(|s| !marker_line_ends(s).is_empty()))
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
///
/// The block starts flush against the marker line, with no separating blank
/// line: that is how `duel.toml` authors its own default roster, and
/// `the_generated_default_roster_is_byte_identical_to_the_authored_one` pins
/// generated ≡ authored on exactly that text. (Callers append this to a prelude
/// that is guaranteed to end in a newline.)
fn render_drivers(slots: &[Slot], side_b_filled: bool) -> String {
    let mut out = String::new();
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
/// - Rejects either side longer than [`MAX_SIDE`]; a world with no
///   [`SLOT_MARKER`] line, with two of them, or without the `spawn_slot` /
///   `on_side_b_destroyed` bodies the generated roster calls; a slot whose
///   anchor the world does not declare; and a fielded hull whose own doctrine
///   steers to an anchor the world does not declare.
///
/// `resolve` turns a ship name into a template path; inject a fake in tests,
/// [`resolve_template`] in production. `templates` loads a resolved template so
/// its doctrine anchors can be checked against this world's `[anchors]`;
/// [`DuelTemplateLoader`] in production, a fake (or a loader that finds nothing,
/// which simply skips the check) in tests.
pub fn apply_duel_sides(
    mut raw: toml::Value,
    side_a: &[String],
    side_b: &[String],
    resolve: &impl Fn(&str) -> Result<String, DuelError>,
    templates: &dyn crate::entities::loader::TemplateLoader,
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
    let source = raw["script"][key.as_str()]
        .as_str()
        .expect("the marked entry is a string")
        .to_string();

    // The rest of the marked world's half of the contract. The harness
    // generates only registrations and one-line drivers; every body they name
    // is the world's, and a missing one fails at RUNTIME as a discarded call —
    // an empty arena, scored as a draw.
    let marker_ends = marker_line_ends(&source);
    if marker_ends.len() > 1 {
        return Err(DuelError::DuplicateSlotMarker {
            key,
            count: marker_ends.len(),
        });
    }
    if !source.contains("fn spawn_slot(") {
        return Err(DuelError::MissingSlotFn {
            name: "spawn_slot",
            needed_for: "every generated slot driver delegates to it",
        });
    }
    if !side_b.is_empty() && !source.contains("fn on_side_b_destroyed(") {
        return Err(DuelError::MissingSlotFn {
            name: "on_side_b_destroyed",
            needed_for: "a non-empty --side-b registers it as the victory handler",
        });
    }

    let slots = slots_for(side_a, side_b, resolve)?;
    for slot in &slots {
        // A generated slot names its own anchor, so the arena has to have
        // staged it.
        if !declares_anchor(&raw, &slot.name) {
            return Err(DuelError::UndeclaredSlotAnchor {
                slot: slot.name.clone(),
            });
        }
        // And the hull fielded into it arrives carrying its own doctrine, whose
        // route this arena has to have staged too (issue #888) — the check the
        // declarative validator can no longer make for a script-authored slot.
        for (anchor, kind) in
            crate::world::validate::template_doctrine_anchors(&slot.template, templates)
        {
            if !declares_anchor(&raw, &anchor) {
                return Err(DuelError::UndeclaredDoctrineAnchor {
                    slot: slot.name.clone(),
                    template: slot.template.clone(),
                    anchor,
                    kind,
                });
            }
        }
    }
    let drivers = render_drivers(&slots, !side_b.is_empty());

    // Keep the marker line itself, so the output is still marked and a second
    // pass over it is a no-op.
    let prelude_end = marker_ends.first().copied().unwrap_or(source.len());
    let mut composed = source[..prelude_end].to_string();
    // A marker line at EOF with no trailing newline would otherwise splice the
    // first registration onto the end of it, commenting the whole line out.
    if !composed.ends_with('\n') {
        composed.push('\n');
    }
    composed.push_str(&drivers);

    let table = raw
        .get_mut("script")
        .and_then(|s| s.as_table_mut())
        .expect("the marked key came from this table");

    bevy::log::debug!(
        target: LogCat::Config.target(),
        "duel harness: regenerated [script.{key}] slot drivers:\n{drivers}"
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

/// The template loader [`apply_duel_sides`] reads fielded hulls with in
/// production: the filesystem, including each template's `includes` closure.
///
/// Not `entity_loader::FsTemplateLoader`, which resolves the same document but
/// also RECORDS it in the content ledger (issue #935). This read is a pre-boot
/// *validation*, not a spawn, and the ledger is the record of what a run
/// consumed — the hulls inspected here are recorded properly when their slots
/// actually spawn, and recording them earlier would quietly move the frozen
/// content digest of every harnessed run.
#[derive(Debug, Default, Clone, Copy)]
pub struct DuelTemplateLoader;

impl crate::entities::loader::TemplateLoader for DuelTemplateLoader {
    fn load_template(&self, path: &str) -> Option<crate::entities::config::EntityConfig> {
        crate::entities::include_resolve::resolve_from_disk(path)
            .ok()?
            .parse()
            .ok()
    }

    /// The filesystem is authoritative, exactly as it is for `FsTemplateLoader`.
    /// Unused by the anchor check — which is silent about a template it cannot
    /// load either way — but the trait has no default, by design.
    fn absence_is_final(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
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
fn on_load(ctx) { ctx.effects.add_faction_enemy("Federation", "Harrow"); }
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
}
