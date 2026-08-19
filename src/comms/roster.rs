//! Pure hail-roster derivation (issue #985).
//!
//! The Comms roster — `CommsRuntime::contacts`, the list of endpoints a Comms
//! officer (human or Backfill AI) may hail — had exactly ONE source until this
//! issue: the declarative `[[comms]]` templates in the world TOML. Every contact
//! was a template's `from` reference id resolved through `name_to_uuid`. The
//! Rhai conversion (issue #982, milestone M7) deleted `[[comms]]` parsing
//! outright, which would have emptied the roster and taken `resolve_hail_target`,
//! `candidate_fact(source_comms_contact)` and every hail candidate with it.
//!
//! This module is the replacement, and now the only source: contacts derived
//! from the ENTITIES themselves. An entity opts in with `[comms] hailable = true`
//! in its template (the same block that already declares `range`), and the live
//! ECS set of such entities is unioned into the roster every tick.
//!
//! ## Merge rule
//!
//! The roster is `already-seated ∪ entity-derived`, de-duplicated on the entity
//! UUID:
//!
//! * The **seated** entry WINS on a UUID collision — it keeps its display
//!   metadata and the runtime `in_range`/`is_urgent` stamps.
//! * Entity-derived entries for UUIDs the seated list did not already hold are
//!   APPENDED, sorted by `(name, uuid)`, so the roster order never depends on
//!   Bevy archetype iteration order.
//!
//! The rule was written for coexistence with the declarative `[[comms]]` roster,
//! where "seated" meant "authored as a template" and the collision rule was
//! "declarative wins". Issue #985 deleted that source; what the same rule now
//! buys is idempotency, because `update_comms_range_flags` re-merges the roster
//! every tick and a contact must not lose its label or its stamps to its own
//! re-derivation.
//!
//! A world's sender label therefore lives on the entity: `[comms] display_name
//! = "…"`, or the entity's `name` reference id, exactly as the fallback below
//! does.
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
/// Precedence mirrors the rule the deleted declarative front-end used
/// (`display_name` over the `from` reference id, issue #751):
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

/// Union entity-derived contacts into the roster.
///
/// De-duplication is keyed on `uuid`, and the entry ALREADY IN `roster` wins: a
/// seated contact keeps its name and its live `in_range`/`is_urgent` stamps
/// untouched. Only UUIDs absent from the roster are appended. The caller merges
/// every tick, so that rule is what makes the pass idempotent.
///
/// New entries are appended in `(name, uuid)` order. The caller hands over an
/// unsorted `derived` (ECS query order is archetype order, which is not a
/// stable contract), so sorting HERE is what makes the player-visible contact
/// order deterministic.
///
/// New contacts are pushed with `in_range: true` / `is_urgent: false`; both are
/// re-stamped downstream from the authoritative `range_flags` map and the
/// inbox.
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

    fn seated(uuid: &str, name: &str) -> CommsContact {
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
    fn a_seated_entry_wins_the_uuid_collision() {
        let mut roster = vec![seated("u1", "Starbase Alpha")];
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
        let mut roster = vec![seated("u1", "Starbase Alpha")];
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
    fn an_empty_derivation_leaves_the_seated_roster_alone() {
        let mut roster = vec![seated("u1", "Starbase Alpha"), seated("u2", "Raider")];
        let before = roster.clone();
        let mut derived_in = Vec::new();
        assert!(!merge_entity_contacts(&mut roster, &mut derived_in));
        assert_eq!(roster, before);
    }
}
