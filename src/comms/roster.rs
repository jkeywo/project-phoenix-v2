//! Pure hail-roster derivation (issue #985).
//!
//! The Comms roster — `CommsRuntime::contacts`, the list of endpoints a Comms
//! officer (human or Backfill AI) may hail — has historically had exactly ONE
//! source: the declarative `[[comms]]` templates in the world TOML. Every
//! contact was a template's `from` reference id resolved through
//! `name_to_uuid`. The Rhai conversion (issue #982, milestone M7) deletes
//! `[[comms]]` parsing outright, which would empty the roster and take
//! `resolve_hail_target`, `candidate_fact(source_comms_contact)` and every hail
//! candidate with it.
//!
//! This module is the replacement source: contacts derived from the ENTITIES
//! themselves. An entity opts in with `[comms] hailable = true` in its template
//! (the same block that already declares `range`), and the live ECS set of such
//! entities is unioned into the roster every tick.
//!
//! ## Dual-source rule (coexistence)
//!
//! While `[[comms]]` still exists, the roster is
//! `declarative ∪ entity-derived`, de-duplicated on the entity UUID:
//!
//! * The **declarative** entry WINS on a UUID collision — it keeps its authored
//!   display metadata (`display_name`, and the runtime `in_range`/`is_urgent`
//!   stamps), so shipped behaviour is unchanged for every world that authors
//!   `[[comms]]`.
//! * Entity-derived entries for UUIDs the declarative pass did not produce are
//!   APPENDED, sorted by `(name, uuid)`, so the roster order never depends on
//!   Bevy archetype iteration order.
//!
//! At M7 the declarative half is gone and the entity-derived half is the only
//! source; a world converting to scripted comms moves the `[[comms]]`
//! `display_name` onto the entity (`[comms] display_name = "…"`) or relies on
//! the entity's `name` reference id, exactly as the fallback below does.
//!
//! Bevy-free so it can be unit-tested directly; the applier lives in
//! `comms::server::update_comms_range_flags`.

use crate::messages::CommsContact;

/// A hailable endpoint derived from a live entity (issue #985).
///
/// Produced by the Bevy applier from the ECS — one per live entity that
/// carries `CommsRange` **and** opted in with `[comms] hailable = true`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntityContact {
    /// Player-facing label. Sorted on FIRST so the roster orders by the name
    /// the officer actually reads, with the UUID only breaking ties.
    pub name: String,
    /// The entity UUID — the roster's de-duplication key.
    pub uuid: String,
}

/// Resolve the player-facing label for an entity-derived contact.
///
/// Precedence mirrors the declarative rule (`display_name` over the `from`
/// reference id, issue #751):
///
/// 1. `[comms] display_name` on the entity template — authored player-facing text.
/// 2. The entity's `EntityName`, which for a world-declared `[[entity]]` is its
///    `name` reference id: the SAME string a `[[comms]] from` would have named,
///    so a converted world's label is unchanged.
/// 3. The raw UUID, matching the weapons console's own no-`EntityName` fallback.
///    A hailable entity is never silently dropped from the roster.
pub fn entity_contact_label(
    display_name: Option<&str>,
    entity_name: Option<&str>,
    uuid: &str,
) -> String {
    display_name.or(entity_name).unwrap_or(uuid).to_string()
}

/// Union entity-derived contacts into a roster that already holds the
/// declarative (`[[comms]]`-derived) entries.
///
/// De-duplication is keyed on `uuid`, and the entry ALREADY IN `roster` wins:
/// a declarative contact keeps its authored name and its live
/// `in_range`/`is_urgent` stamps untouched. Only UUIDs absent from the roster
/// are appended.
///
/// New entries are appended in `(name, uuid)` order. The caller hands over an
/// unsorted `derived` (ECS query order is archetype order, which is not a
/// stable contract), so sorting HERE is what makes the player-visible contact
/// order deterministic.
///
/// New contacts are pushed with `in_range: true` / `is_urgent: false`; both are
/// re-stamped downstream from the authoritative `range_flags` map and the
/// inbox, exactly as they are for a declarative contact.
///
/// Returns `true` when at least one contact was appended (the caller's cue to
/// set `needs_broadcast`).
pub fn merge_entity_contacts(
    roster: &mut Vec<CommsContact>,
    derived: &mut Vec<EntityContact>,
) -> bool {
    derived.sort();
    derived.dedup_by(|a, b| a.uuid == b.uuid);

    let mut added = false;
    for candidate in derived.iter() {
        if roster.iter().any(|c| c.uuid == candidate.uuid) {
            // Declarative entry wins its display metadata while `[[comms]]`
            // still exists — this is what keeps shipped rosters byte-identical.
            continue;
        }
        roster.push(CommsContact {
            uuid: candidate.uuid.clone(),
            name: candidate.name.clone(),
            in_range: true,
            is_urgent: false,
        });
        added = true;
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declarative(uuid: &str, name: &str) -> CommsContact {
        CommsContact {
            uuid: uuid.into(),
            name: name.into(),
            in_range: false,
            is_urgent: true,
        }
    }

    fn derived(name: &str, uuid: &str) -> EntityContact {
        EntityContact {
            name: name.into(),
            uuid: uuid.into(),
        }
    }

    // ── Label precedence ──────────────────────────────────────────────────

    #[test]
    fn label_prefers_authored_display_name() {
        assert_eq!(
            entity_contact_label(
                Some("Axiom Station"),
                Some("world.entity.starbase_alpha.name"),
                "u1"
            ),
            "Axiom Station"
        );
    }

    #[test]
    fn label_falls_back_to_the_entity_reference_id() {
        assert_eq!(
            entity_contact_label(None, Some("world.entity.starbase_alpha.name"), "u1"),
            "world.entity.starbase_alpha.name"
        );
    }

    #[test]
    fn label_falls_back_to_the_uuid_when_the_entity_is_unnamed() {
        assert_eq!(entity_contact_label(None, None, "u1"), "u1");
    }

    // ── Merge semantics ───────────────────────────────────────────────────

    #[test]
    fn entity_contacts_are_appended_to_an_empty_roster() {
        let mut roster = Vec::new();
        let mut derived_in = vec![derived("Bravo", "u2"), derived("Alpha", "u1")];
        assert!(merge_entity_contacts(&mut roster, &mut derived_in));
        assert_eq!(
            roster.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["Alpha", "Bravo"]
        );
        assert!(roster.iter().all(|c| c.in_range && !c.is_urgent));
    }

    #[test]
    fn a_declarative_entry_wins_the_uuid_collision() {
        let mut roster = vec![declarative("u1", "Starbase Alpha")];
        let mut derived_in = vec![derived("world.entity.starbase_alpha.name", "u1")];
        assert!(
            !merge_entity_contacts(&mut roster, &mut derived_in),
            "a colliding entity-derived contact adds nothing"
        );
        assert_eq!(roster.len(), 1);
        // Name AND the live range/urgency stamps survive untouched.
        assert_eq!(roster[0].name, "Starbase Alpha");
        assert!(!roster[0].in_range);
        assert!(roster[0].is_urgent);
    }

    #[test]
    fn only_the_uncollided_entity_contacts_are_appended() {
        let mut roster = vec![declarative("u1", "Starbase Alpha")];
        let mut derived_in = vec![derived("Alpha", "u1"), derived("Courier", "u2")];
        assert!(merge_entity_contacts(&mut roster, &mut derived_in));
        assert_eq!(
            roster
                .iter()
                .map(|c| (c.uuid.as_str(), c.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("u1", "Starbase Alpha"), ("u2", "Courier")]
        );
    }

    #[test]
    fn merging_is_idempotent_across_ticks() {
        let mut roster = Vec::new();
        let mut first = vec![derived("Alpha", "u1")];
        assert!(merge_entity_contacts(&mut roster, &mut first));
        let mut second = vec![derived("Alpha", "u1")];
        assert!(
            !merge_entity_contacts(&mut roster, &mut second),
            "the same live entity must not be re-added on the next tick"
        );
        assert_eq!(roster.len(), 1);
    }

    #[test]
    fn append_order_is_independent_of_input_order() {
        let names = [
            ("Delta", "u4"),
            ("Alpha", "u1"),
            ("Charlie", "u3"),
            ("Bravo", "u2"),
        ];
        let mut forward: Vec<CommsContact> = Vec::new();
        let mut a: Vec<EntityContact> = names.iter().map(|(n, u)| derived(n, u)).collect();
        merge_entity_contacts(&mut forward, &mut a);

        let mut reversed: Vec<CommsContact> = Vec::new();
        let mut b: Vec<EntityContact> = names.iter().rev().map(|(n, u)| derived(n, u)).collect();
        merge_entity_contacts(&mut reversed, &mut b);

        assert_eq!(
            forward, reversed,
            "roster order must not depend on query order"
        );
        assert_eq!(
            forward.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["Alpha", "Bravo", "Charlie", "Delta"]
        );
    }

    #[test]
    fn duplicate_names_tie_break_on_uuid() {
        let mut roster = Vec::new();
        let mut derived_in = vec![derived("Harrow", "u9"), derived("Harrow", "u2")];
        merge_entity_contacts(&mut roster, &mut derived_in);
        assert_eq!(
            roster.iter().map(|c| c.uuid.as_str()).collect::<Vec<_>>(),
            vec!["u2", "u9"]
        );
    }

    #[test]
    fn a_repeated_uuid_in_one_batch_is_collapsed() {
        let mut roster = Vec::new();
        let mut derived_in = vec![derived("Alpha", "u1"), derived("Alpha", "u1")];
        merge_entity_contacts(&mut roster, &mut derived_in);
        assert_eq!(roster.len(), 1);
    }

    #[test]
    fn an_empty_derivation_leaves_the_declarative_roster_alone() {
        let mut roster = vec![
            declarative("u1", "Starbase Alpha"),
            declarative("u2", "Raider"),
        ];
        let before = roster.clone();
        let mut derived_in = Vec::new();
        assert!(!merge_entity_contacts(&mut roster, &mut derived_in));
        assert_eq!(roster, before);
    }
}

/// Shipped-content regression guard for the dual-source roster (issue #985).
///
/// The acceptance bar for adding the entity-derived source was
/// BEHAVIOUR-PRESERVING: no shipped world's hail roster may change. Two
/// invariants together deliver that, and this module asserts both.
#[cfg(test)]
mod shipped_world_rosters {
    use std::path::{Path, PathBuf};

    fn manifest(sub: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(sub)
    }

    /// Every `.toml` under `assets/entities`, recursively (templates AND the
    /// composition fragments they include).
    fn entity_templates() -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![manifest("assets/entities")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("assets/entities must be readable") {
                let path = entry.expect("readable dir entry").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                    out.push(path);
                }
            }
        }
        out.sort();
        out
    }

    /// The declarative roster a world produces today: the `[[comms]]` `from`
    /// reference ids in authored order, de-duplicated the way
    /// `init_comms_runtime` de-duplicates them (first entry per sender wins;
    /// resolution is `from` → UUID, and distinct names resolve to distinct
    /// UUIDs, so ordered-unique `from` IS the roster identity).
    fn declarative_roster(world: &crate::world::config::WorldConfig) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for tmpl in &world.comms {
            if !out.iter().any(|f| f == &tmpl.from) {
                out.push(tmpl.from.clone());
            }
        }
        out
    }

    /// INVARIANT 1 — the entity-derived source is authored on NOTHING.
    ///
    /// `[comms] hailable = true` is opt-in and no shipped template sets it, so
    /// the entity-derived half of the union contributes zero contacts to every
    /// shipped world and every roster below is produced by the declarative half
    /// alone, exactly as before this commit.
    ///
    /// The census that forced the opt-in: 13 shipped entity templates declare
    /// `[comms] range` (every Alliance hull, every Harrow hull, the Requiem
    /// courier, all three stations, and the RNG-coverage lancer). Deriving the
    /// roster from `range` alone would have made `combat_test`'s roster jump
    /// from ONE contact (Starbase Alpha) to that plus every hostile wave ship,
    /// and would have given `patrol`, `reinforcements` and `rng_coverage` —
    /// which author no `[[comms]]` at all — a non-empty roster where they have
    /// none today.
    ///
    /// The M7-era world conversions are what turn this on, per world. When one
    /// does, THIS test is the thing that must be updated deliberately.
    #[test]
    fn no_shipped_entity_opts_in_to_the_hail_roster() {
        let mut opted_in: Vec<String> = Vec::new();
        for path in entity_templates() {
            let text = std::fs::read_to_string(&path).expect("entity template must be readable");
            let value: toml::Value = match toml::from_str(&text) {
                Ok(v) => v,
                Err(e) => panic!("{} must be valid TOML: {e}", path.display()),
            };
            let hailable = value
                .get("comms")
                .and_then(|c| c.get("hailable"))
                .and_then(|h| h.as_bool())
                .unwrap_or(false);
            if hailable {
                opted_in.push(path.display().to_string());
            }
        }
        assert!(
            opted_in.is_empty(),
            "issue #985 landed `[comms] hailable` authored on NOTHING so shipped rosters are \
             unchanged. These templates now opt in, which ADDS contacts to every world that \
             spawns them — update the roster snapshots below in the same commit: {opted_in:?}"
        );
    }

    /// INVARIANT 2 — the declarative rosters themselves, snapshotted.
    ///
    /// One entry per shipped world; the value is the ordered-unique `[[comms]]`
    /// `from` list. A world absent from this table must have an EMPTY roster.
    /// `combat_test` is the demo scenario: twelve `[[comms]]` templates, all
    /// from Starbase Alpha, collapsing to a single contact.
    #[test]
    fn shipped_world_declarative_rosters_are_unchanged() {
        const EXPECTED: &[(&str, &[&str])] = &[
            ("combat_test.toml", &["world.entity.starbase_alpha.name"]),
            (
                "default.toml",
                &[
                    "world.entity.raider_alpha.name",
                    "world.entity.starbase_alpha.name",
                ],
            ),
        ];

        let dir = manifest("assets/worlds");
        let mut worlds: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("assets/worlds must be readable")
            .map(|e| e.expect("readable dir entry").path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("toml"))
            .collect();
        worlds.sort();
        assert!(!worlds.is_empty(), "assets/worlds must ship worlds");

        for path in worlds {
            let file = path
                .file_name()
                .and_then(|f| f.to_str())
                .expect("world file name")
                .to_string();
            let text = std::fs::read_to_string(&path).expect("world must be readable");
            let world = crate::world::config::parse_world(&text)
                .unwrap_or_else(|e| panic!("{file} must parse: {e}"));
            let expected: Vec<String> = EXPECTED
                .iter()
                .find(|(name, _)| *name == file)
                .map(|(_, senders)| senders.iter().map(|s| s.to_string()).collect())
                .unwrap_or_default();
            assert_eq!(
                declarative_roster(&world),
                expected,
                "{file}: hail roster changed. Entity-derived contacts (#985) are a UNION on top \
                 of this list, so any diff here is a behaviour change"
            );
            assert!(
                world.scripted_comms.is_empty(),
                "{file}: a scripted `[[comms]]` thread contributes NO contact today — the \
                 entity-derived source is what will carry it at M7. Converting this world \
                 needs `[comms] hailable = true` on its sender in the same commit"
            );
        }
    }

    /// The demo scenario's twelve templates really are twelve, collapsing to
    /// one contact — the specific number the M7 teardown would have zeroed.
    #[test]
    fn combat_test_collapses_twelve_templates_onto_one_contact() {
        let text = include_str!("../../assets/worlds/combat_test.toml");
        let world = crate::world::config::parse_world(text).expect("combat_test.toml must parse");
        assert_eq!(
            world.comms.len(),
            12,
            "combat_test authors 12 comms templates"
        );
        assert_eq!(declarative_roster(&world).len(), 1);
    }
}
