//! `--side-a` / `--side-b` class-matchup transform for `duel.toml` (issue #844).
//!
//! `duel.toml` pre-authors up to five NPC spawn slots per side (as
//! `spawn_entity` triggers, so `on_all_destroyed` group membership registers
//! through the normal `SpawnEntity` dispatch — a static `[[entity]]` never
//! joins a group), plus one static `player-ship` entity for side-A slot 1.
//! [`apply_duel_sides`] takes the parsed [`WorldConfig`] and the two CLI ship
//! lists and, as a **pure** transform over the config, fills the slots the
//! lists name and deletes the rest.
//!
//! The core (fill / delete / reject / empty-group guard) is filesystem-free:
//! template-name resolution is injected as a closure so the transform is
//! unit-testable with a fake resolver, and the production wiring passes
//! [`resolve_template`] (which does touch the filesystem).
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
//! transform DELETES that trigger when `side_b` ends up with zero filled slots,
//! so a degenerate `--side-b` (empty) can't instant-fire a false victory off an
//! empty group. The guard runs for both groups symmetrically.

use crate::world::config::{TriggerAction, TriggerCondition, WorldConfig};

/// Federation faction UUID — side A's own faction (the player's). Side-A NPC
/// escorts are forced to it so they bucket as the player side in the report.
/// A faction *identity* reference, matching `assets/factions/federation.toml`
/// (mirrors how `probe_duel.toml` embeds the Harrow UUID inline) — not a
/// tunable gameplay value.
pub const FEDERATION_FACTION: &str = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";

/// Harrow faction UUID — side B's faction. Matches
/// `assets/factions/harrow.toml`. Federation<->Harrow mutual hostility is
/// armed by the pre-authored `add_faction_enemy` actions in `duel.toml`.
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
    /// The world carries no `side_a_*`/`side_b_*` spawn slots, so there is
    /// nothing for the ship lists to fill.
    ///
    /// Without this the transform was a silent no-op: `--side-a cruiser
    /// --side-b destroyer --world <some non-duel world>` loaded that world
    /// untouched and produced a combat-free draw that reads like a balance
    /// finding. Naming the missing slots is the whole point of the error.
    NoDuelSlots,
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
                "--side-a/--side-b need a duel-shaped world: this one authors no \
                 side_a_*/side_b_* spawn_entity slots to fill. Drop --world to use \
                 the duel harness (assets/worlds/duel.toml), or author matching slots"
            ),
        }
    }
}

impl std::error::Error for DuelError {}

/// Which side a slot belongs to, for faction assignment and empty-group guards.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    A,
    B,
}

impl Side {
    fn faction(self) -> &'static str {
        match self {
            Side::A => FEDERATION_FACTION,
            Side::B => HARROW_FACTION,
        }
    }
}

/// Map a slot spawn name to `(side, array index)`.
///
/// `side_a_2`..`side_a_5` are the side-A *escort* slots: `side_a_1` is the
/// player (a static entity, not a spawn slot), so `side_a_2` is `side_a[1]`.
/// `side_b_1`..`side_b_5` are `side_b[0]`..`side_b[4]`.
fn slot_of(name: &str) -> Option<(Side, usize)> {
    if let Some(n) = name.strip_prefix("side_a_") {
        let slot: usize = n.parse().ok()?;
        // side_a_1 is the player; escort slots start at 2.
        (slot >= 2).then(|| (Side::A, slot - 1))
    } else if let Some(n) = name.strip_prefix("side_b_") {
        let slot: usize = n.parse().ok()?;
        (slot >= 1).then(|| (Side::B, slot - 1))
    } else {
        None
    }
}

/// The `SpawnEntity` name in a trigger's action list, if it has exactly one.
fn spawn_name(actions: &[TriggerAction]) -> Option<&str> {
    actions.iter().find_map(|a| match a {
        TriggerAction::SpawnEntity { name, .. } => Some(name.as_str()),
        _ => None,
    })
}

/// How many escort slots a side ends up with filled, for the empty-group guard.
///
/// Side A's fillable escort slots are `side_a[1..]` (slot 1 is the player), so
/// at most [`MAX_SIDE`]-1. Side B fills `side_b[..]`, at most [`MAX_SIDE`].
fn filled_slots(side_a: &[String], side_b: &[String], group: &str) -> Option<usize> {
    match group {
        "side_a" => Some(side_a.len().saturating_sub(1).min(MAX_SIDE - 1)),
        "side_b" => Some(side_b.len().min(MAX_SIDE)),
        _ => None,
    }
}

/// Force `faction` into a `spawn_entity` action's inline `overrides` table,
/// creating the table if the slot authored none. Side-A escorts get the
/// player's faction, side-B ships the enemy's, so the report buckets each ship
/// on the correct side regardless of what the template's own faction was.
fn set_override_faction(overrides: &mut Option<toml::Value>, faction: &str) {
    let table = overrides.get_or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if let toml::Value::Table(t) = table {
        t.insert("faction".into(), toml::Value::String(faction.to_string()));
    }
}

/// Fill `duel.toml`'s NPC slots from the CLI ship lists and delete the rest —
/// a pure transform over [`WorldConfig`].
///
/// - `side_a[0]` is the player ship (applied elsewhere via `--ship`); only
///   `side_a[1..]` fill escort slots.
/// - `side_b[..]` fill side-B slots.
/// - Unused slot triggers are removed.
/// - An `on_all_destroyed` trigger (and its `game_over`) whose group ends up
///   with zero filled slots is removed, so an empty side never instant-fires.
/// - Rejects either side longer than [`MAX_SIDE`].
///
/// `resolve` turns a ship name into a template path; inject a fake in tests,
/// [`resolve_template`] in production.
pub fn apply_duel_sides(
    mut world: WorldConfig,
    side_a: &[String],
    side_b: &[String],
    resolve: &impl Fn(&str) -> Result<String, DuelError>,
) -> Result<WorldConfig, DuelError> {
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

    // Reject a world with no slots at all before touching it. The fill loop
    // below is a no-op on such a world, which used to mean the run proceeded
    // with the CLI's ship lists silently ignored.
    if !world
        .triggers
        .iter()
        .any(|t| spawn_name(&t.actions).and_then(slot_of).is_some())
    {
        return Err(DuelError::NoDuelSlots);
    }

    let mut kept = Vec::with_capacity(world.triggers.len());
    for mut trigger in world.triggers.into_iter() {
        // Empty-group guard: drop a side's victory `on_all_destroyed` when that
        // side has no filled slots (an empty group would fire immediately).
        if let TriggerCondition::OnAllDestroyed { group, .. } = &trigger.condition {
            if filled_slots(side_a, side_b, group) == Some(0) {
                continue;
            }
        }

        // Slot spawns: fill the ones the lists reach, delete the rest.
        if let Some((side, idx)) = spawn_name(&trigger.actions).and_then(slot_of) {
            let list = match side {
                Side::A => side_a,
                Side::B => side_b,
            };
            let Some(name) = list.get(idx) else {
                continue; // slot beyond the list length → delete
            };
            let template = resolve(name)?;
            for action in &mut trigger.actions {
                if let TriggerAction::SpawnEntity {
                    template_path,
                    overrides,
                    ..
                } = action
                {
                    *template_path = template.clone();
                    set_override_faction(overrides, side.faction());
                }
            }
        }

        kept.push(trigger);
    }
    world.triggers = kept;
    Ok(world)
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
    use crate::world::config::parse_world;

    /// A minimal `duel.toml`-shaped world: two side-A escort slots, two side-B
    /// slots, and a side-B victory `on_all_destroyed`. Placeholder templates
    /// are overwritten on fill and irrelevant on delete.
    const DUEL_FIXTURE: &str = r#"
[global]
seed = 1

[player_spawn]
position = [0.0, 0.0, 0.0]

[[entity]]
template_path = "assets/entities/alliance_cruiser.toml"
id = "player-ship"
spawn_on = "game_start"

[[trigger]]
condition = "on_world_loaded"
  [[trigger.action]]
  type = "add_faction_enemy"
  faction = "Federation"
  enemy = "Harrow"

[[trigger]]
condition = "on_timer"
after_secs = 0.0
  [[trigger.action]]
  type = "spawn_entity"
  template_path = "assets/entities/placeholder.toml"
  name = "side_a_2"
  position = [0.0, 0.0, 40.0]
  groups = ["side_a"]
  overrides = { behaviour = { doctrine = [] } }

[[trigger]]
condition = "on_timer"
after_secs = 0.0
  [[trigger.action]]
  type = "spawn_entity"
  template_path = "assets/entities/placeholder.toml"
  name = "side_a_3"
  position = [0.0, 0.0, -40.0]
  groups = ["side_a"]
  overrides = { behaviour = { doctrine = [] } }

[[trigger]]
condition = "on_timer"
after_secs = 0.0
  [[trigger.action]]
  type = "spawn_entity"
  template_path = "assets/entities/placeholder.toml"
  name = "side_b_1"
  position = [300.0, 0.0, 0.0]
  groups = ["side_b"]
  overrides = { behaviour = { doctrine = [] } }

[[trigger]]
condition = "on_timer"
after_secs = 0.0
  [[trigger.action]]
  type = "spawn_entity"
  template_path = "assets/entities/placeholder.toml"
  name = "side_b_2"
  position = [300.0, 0.0, 40.0]
  groups = ["side_b"]
  overrides = { behaviour = { doctrine = [] } }

[[trigger]]
condition = "on_all_destroyed"
group = "side_b"
  [[trigger.action]]
  type = "game_over"
  outcome = "victory"
"#;

    fn fixture() -> WorldConfig {
        parse_world(DUEL_FIXTURE).expect("fixture parses")
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

    /// Every `spawn_entity` action in the world, as `(name, template, faction)`.
    fn spawns(world: &WorldConfig) -> Vec<(String, String, Option<String>)> {
        world
            .triggers
            .iter()
            .flat_map(|t| &t.actions)
            .filter_map(|a| match a {
                TriggerAction::SpawnEntity {
                    name,
                    template_path,
                    overrides,
                    ..
                } => {
                    let faction = overrides
                        .as_ref()
                        .and_then(|o| o.get("faction").and_then(|v| v.as_str()).map(String::from));
                    Some((name.clone(), template_path.clone(), faction))
                }
                _ => None,
            })
            .collect()
    }

    fn has_victory(world: &WorldConfig) -> bool {
        world.triggers.iter().any(|t| {
            matches!(&t.condition, TriggerCondition::OnAllDestroyed { group, .. } if group == "side_b")
        })
    }

    /// A world that authors no duel slots must be rejected, not silently run
    /// with the ship lists ignored — that produced a combat-free draw that
    /// looked like a balance result.
    #[test]
    fn a_world_without_duel_slots_is_rejected() {
        const NO_SLOTS: &str = r#"
[global]
seed = 1

[player_spawn]
position = [0.0, 0.0, 0.0]

[[trigger]]
condition = "on_world_loaded"
  [[trigger.action]]
  type = "add_faction_enemy"
  faction = "Federation"
  enemy = "Harrow"
"#;
        let err = apply_duel_sides(
            parse_world(NO_SLOTS).expect("fixture parses"),
            &["cruiser".into()],
            &["destroyer".into()],
            &fake_resolve,
        )
        .expect_err("a slot-free world must be rejected");
        assert_eq!(err, DuelError::NoDuelSlots);
        // The message has to name what is missing, or the user cannot act on it.
        let msg = err.to_string();
        assert!(msg.contains("side_a_*/side_b_*"), "got {msg:?}");
        assert!(msg.contains("assets/worlds/duel.toml"), "got {msg:?}");
    }

    #[test]
    fn fills_named_slots_with_resolved_template_and_side_faction() {
        let world = apply_duel_sides(
            fixture(),
            &["cruiser".into(), "courier".into()], // player + 1 escort
            &["destroyer".into(), "battleship".into()],
            &fake_resolve,
        )
        .expect("applies");

        let s = spawns(&world);
        // side_a_2 filled from side_a[1] = courier, Federation faction.
        let a2 = s
            .iter()
            .find(|(n, ..)| n == "side_a_2")
            .expect("side_a_2 kept");
        assert_eq!(a2.1, "assets/entities/courier.toml");
        assert_eq!(a2.2.as_deref(), Some(FEDERATION_FACTION));
        // side_b_1 / side_b_2 filled from side_b[0] / side_b[1], Harrow faction.
        let b1 = s
            .iter()
            .find(|(n, ..)| n == "side_b_1")
            .expect("side_b_1 kept");
        assert_eq!(b1.1, "assets/entities/destroyer.toml");
        assert_eq!(b1.2.as_deref(), Some(HARROW_FACTION));
        let b2 = s
            .iter()
            .find(|(n, ..)| n == "side_b_2")
            .expect("side_b_2 kept");
        assert_eq!(b2.1, "assets/entities/battleship.toml");
    }

    #[test]
    fn deletes_unfilled_slots() {
        // Player only on side A (no escorts), one ship on side B.
        let world = apply_duel_sides(
            fixture(),
            &["cruiser".into()],
            &["destroyer".into()],
            &fake_resolve,
        )
        .expect("applies");
        let names: Vec<String> = spawns(&world).into_iter().map(|(n, ..)| n).collect();
        // side_a_2 / side_a_3 unfilled → gone; side_b_2 unfilled → gone.
        assert_eq!(names, vec!["side_b_1".to_string()]);
        // side_b still has a filled slot → victory trigger survives.
        assert!(has_victory(&world));
    }

    #[test]
    fn empty_side_b_deletes_the_victory_trigger() {
        // Degenerate: nobody on side B. The victory `on_all_destroyed` group
        // would be empty and fire immediately — the transform removes it.
        let world =
            apply_duel_sides(fixture(), &["cruiser".into()], &[], &fake_resolve).expect("applies");
        assert!(
            !has_victory(&world),
            "empty side_b must drop the victory trigger"
        );
        // And no side-B slots remain.
        let names: Vec<String> = spawns(&world).into_iter().map(|(n, ..)| n).collect();
        assert!(names.is_empty(), "no slots should remain, got {names:?}");
    }

    #[test]
    fn full_five_v_five_fills_every_slot() {
        // The fixture only authors two slots per side, but the transform must
        // accept a full roster without rejecting it.
        let five = vec![
            "a".to_string(),
            "b".into(),
            "c".into(),
            "d".into(),
            "e".into(),
        ];
        let world = apply_duel_sides(fixture(), &five, &five, &fake_resolve).expect("5v5 applies");
        // Both authored side-B slots are filled (side_b[0], side_b[1]).
        let s = spawns(&world);
        assert!(s
            .iter()
            .any(|(n, t, _)| n == "side_b_1" && t == "assets/entities/a.toml"));
        assert!(s
            .iter()
            .any(|(n, t, _)| n == "side_b_2" && t == "assets/entities/b.toml"));
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
