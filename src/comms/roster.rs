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

    /// INVARIANT 1 — the entity-derived source is authored on exactly the
    /// templates a converted world needs, and nothing else.
    ///
    /// `[comms] hailable = true` is the opt-in that replaces a deleted
    /// `[[comms]] from` as a roster source. It is TEMPLATE-level, so switching
    /// it on adds a contact to every world that spawns the hull — which is why
    /// the set is pinned here rather than left to grow.
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
    /// The world conversions are what turn this on, per world. When one does,
    /// THIS test is the thing that must be updated deliberately.
    ///
    /// `station_axiom` is the first and, now that every shipped world is
    /// converted, the ONLY entry (issue #984). It is safe at template level
    /// because the two worlds that field the station — `default` and
    /// `combat_test` — both listed it as their one hailable contact before they
    /// converted, so the template-level opt-in reproduces exactly the roster
    /// they had. The conversions' other sender, `ship_harrow_patrol`, is
    /// deliberately NOT here: it also flies in four worlds that never listed it
    /// — including `combat_test`'s own wave 8 — so its opt-in is a per-instance
    /// override in `default.toml` (see INVARIANT 3).
    #[test]
    fn only_the_converted_worlds_senders_opt_in_to_the_hail_roster() {
        const EXPECTED: &[&str] = &["station_axiom.toml"];

        let mut opted_in: Vec<String> = Vec::new();
        for path in entity_templates() {
            let text = std::fs::read_to_string(&path).expect("entity template must be readable");
            let value: toml::Value = match toml::from_str(&text) {
                Ok(v) => v,
                Err(e) => panic!("{} must be valid TOML: {e}", path.display()),
            };
            if template_hailable(&value) {
                opted_in.push(
                    path.file_name()
                        .and_then(|f| f.to_str())
                        .expect("template file name")
                        .to_string(),
                );
            }
        }
        assert_eq!(
            opted_in, EXPECTED,
            "`[comms] hailable` ADDS a contact to every world that spawns the template. Changing \
             this set changes shipped rosters — update the snapshots below in the same commit"
        );
    }

    /// Read `[comms] hailable` out of an entity document (template or merged).
    fn template_hailable(value: &toml::Value) -> bool {
        value
            .get("comms")
            .and_then(|c| c.get("hailable"))
            .and_then(|h| h.as_bool())
            .unwrap_or(false)
    }

    /// INVARIANT 2 — the declarative rosters themselves, snapshotted.
    ///
    /// One entry per shipped world; the value is the ordered-unique `[[comms]]`
    /// `from` list. A world absent from this table must have an EMPTY roster.
    ///
    /// The table is now EMPTY, and that is the finished state rather than a
    /// gap: issue #984 converted `default.toml` and then `combat_test.toml` —
    /// the last shipped world authoring `[[comms]]` — so no shipped world
    /// produces a declarative contact any more and every roster is entirely
    /// entity-derived. INVARIANT 3 is what holds the converted rosters to the
    /// senders this table used to carry. The assertion stays because the
    /// declarative source is still LIVE (a mod pack or hand-authored world may
    /// use it): a shipped world that starts authoring `[[comms]]` again is a
    /// roster change and has to be seen.
    #[test]
    fn shipped_world_declarative_rosters_are_unchanged() {
        const EXPECTED: &[(&str, &[&str])] = &[];

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

    /// The entity-derived roster a world produces: the label of every NAMED
    /// `[[entity]]` whose merged `[comms]` block opts in, in the `(name, uuid)`
    /// order [`merge_entity_contacts`] appends. Names are distinct reference
    /// ids, so ordering on the name alone reproduces that order.
    ///
    /// The merge is the `comms` sub-table only — enough for the opt-in, and
    /// deliberately not a re-implementation of `merge_entity_config_toml`. It
    /// reads the template file directly, so a `[comms]` block arriving through
    /// an include fragment would be missed; no shipped template does that, and
    /// [`only_the_converted_worlds_senders_opt_in_to_the_hail_roster`] is what
    /// notices if one starts.
    fn entity_derived_roster(text: &str) -> Vec<String> {
        let world: toml::Value = toml::from_str(text).expect("world must be valid TOML");
        let mut out: Vec<String> = Vec::new();
        let Some(entities) = world.get("entity").and_then(|e| e.as_array()) else {
            return out;
        };
        for entity in entities {
            let Some(name) = entity.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let template_path = entity
                .get("template_path")
                .and_then(|p| p.as_str())
                .expect("a world entity must name a template");
            let template: toml::Value = toml::from_str(
                &std::fs::read_to_string(manifest(template_path))
                    .expect("entity template must be readable"),
            )
            .expect("entity template must be valid TOML");
            let base = template
                .get("comms")
                .cloned()
                .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()));
            let merged = match entity.get("overrides").and_then(|o| o.get("comms")) {
                Some(over) => crate::entity_override::merge_toml(&base, over),
                None => base,
            };
            let mut wrapper = toml::map::Map::new();
            wrapper.insert("comms".to_string(), merged);
            if template_hailable(&toml::Value::Table(wrapper)) {
                out.push(name.to_string());
            }
        }
        out.sort();
        out
    }

    /// INVARIANT 3 — the converted worlds' rosters survived their conversions
    /// (#984).
    ///
    /// Both worlds' comms are `[script]` now, so the declarative source that
    /// produced their contacts is gone and the entity-derived source has to
    /// produce the SAME senders, in the same order.
    ///
    /// `default.toml`: the declarative roster was `[raider_alpha,
    /// starbase_alpha]` in authored order, and the entity-derived one appends in
    /// `(name, uuid)` order — which for these two reference ids is the same
    /// sequence. That coincidence is load-bearing: `tests/smoke/comms.spec.js`
    /// hails `contacts[0]`.
    ///
    /// `combat_test.toml`: twelve `[[comms]]` templates, all from Starbase
    /// Alpha, collapsed to a single contact; the entity-derived source produces
    /// that same one contact from `station_axiom.toml`'s template-level opt-in.
    /// Its wave 8 flies `ship_harrow_patrol`, whose opt-in is deliberately
    /// per-INSTANCE in `default.toml` — so the raid adds nobody to the roster,
    /// which is the whole reason that opt-in is not template-level.
    #[test]
    fn the_converted_worlds_rosters_match_the_declarative_ones_they_replaced() {
        let default_text = include_str!("../../assets/worlds/default.toml");
        let world = crate::world::config::parse_world(default_text).expect("default.toml parses");
        assert!(
            world.comms.is_empty(),
            "default.toml's comms are [script]-authored (#984)"
        );
        assert_eq!(
            entity_derived_roster(default_text),
            vec![
                "world.entity.raider_alpha.name".to_string(),
                "world.entity.starbase_alpha.name".to_string(),
            ],
            "the conversion must preserve default.toml's two contacts, in order"
        );

        // The layer the converted world still loads authors no contacts, and
        // must not start doing so through the shared hull.
        let reinforcements = include_str!("../../assets/worlds/reinforcements.toml");
        assert!(
            entity_derived_roster(reinforcements).is_empty(),
            "reinforcements.toml has no hail contacts and must not gain any"
        );

        // The demo scenario: twelve templates onto one contact, and after the
        // conversion that one contact has to come from the entity instead.
        let combat_test = include_str!("../../assets/worlds/combat_test.toml");
        let world =
            crate::world::config::parse_world(combat_test).expect("combat_test.toml must parse");
        assert!(
            world.comms.is_empty(),
            "combat_test's twelve comms templates are [script]-authored (#984)"
        );
        assert_eq!(
            entity_derived_roster(combat_test),
            vec!["world.entity.starbase_alpha.name".to_string()],
            "combat_test's one entity-derived candidate is the contact it already had"
        );
    }
}
