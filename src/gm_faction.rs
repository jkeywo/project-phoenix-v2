//! The narrowly typed, attributed GM adapter over faction hostility (#1442).
//!
//! Faction relations were already mutable at runtime — `add_faction_enemy` and
//! `remove_faction_enemy` are authored trigger actions, and
//! [`crate::ai::faction::FactionRegistry::add_enemy`]/`remove_enemy` are the
//! vocabulary they run through. What did not exist was a way for a GM to make
//! one of those changes *as a GM*: attributed to an operator, ordered by the
//! canonical journal, and therefore reversible. This module adds exactly that
//! and nothing else. There is no raw faction editor here: a GM names two
//! authored factions and a hostility value, the same two registry calls run,
//! and the AI target re-validation the trigger path already performs runs too.
//!
//! # Why the overrides are recorded separately
//!
//! [`GmFactionOverrides`] is not a second registry. It is the record of *which
//! pairs a GM has moved and what they held before*, which three different
//! consumers need and none of which can recover from the registry itself:
//!
//!  - the inverse path (#1442) revalidates the affected pair at the canonical
//!    apply tick, and needs the value the original action left, not merely the
//!    value the pair holds now;
//!  - a snapshot restore has to put GM-attributed relations back. The registry
//!    is rebuilt from `assets/factions/*.toml` on load, so without this record a
//!    save would restore a hostility the journal still reports as Applied —
//!    silently wrong, and indistinguishable from correct;
//!  - the deterministic digest folds it, so two peers that disagree about a GM
//!    faction decision are caught by the ordinary mesh comparator.
//!
//! Trigger-driven changes are deliberately NOT recorded here. They are authored
//! content, replayed by the trigger pipeline, and folding them would move the
//! digest of every existing world that calls `add_faction_enemy` — `duel.toml`
//! among them — for no gain. The fold below is skipped entirely while no GM has
//! touched a relation, so a run that never uses this surface keeps the digest it
//! had before this module existed.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::entities::spawner::{BehaviourSection, EntityUuid, FactionComponent};

/// One GM-attributed hostility override for an ordered pair of factions.
///
/// Keyed by the authored faction `name` — the identity world TOML, Rhai and the
/// `add_faction_enemy` trigger action all use — rather than by UUID, because
/// that is the vocabulary a GM reads and the one a save can still resolve after
/// a content reload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmFactionRelationOverride {
    /// The faction whose own enemies list changed. Hostility is asymmetric by
    /// construction, exactly as [`crate::ai::faction::is_enemy`] documents.
    pub faction: String,
    /// The faction added to or removed from that list.
    pub enemy: String,
    /// What the pair held immediately BEFORE the first GM change of this run.
    ///
    /// This is the value a restore reverts to when a save predates the change,
    /// so it is captured once and never overwritten by later GM work on the
    /// same pair.
    pub before: bool,
    /// What the canonical journal has most recently made true.
    pub current: bool,
}

/// Every GM-attributed faction relation change in this run, ordered.
///
/// Sorted by `(faction, enemy)` and held as a `Vec` rather than a map keyed by
/// a tuple: the same value is serialized into the snapshot, folded into the
/// digest and projected to the GM page, and a tuple map key is not expressible
/// in every one of those encodings.
#[derive(Resource, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmFactionOverrides {
    #[serde(default)]
    entries: Vec<GmFactionRelationOverride>,
    /// A GM withdrawal has cleared a hostility that AI tactical locks may still
    /// be holding, and [`revalidate_gm_faction_locks`] has not run yet.
    ///
    /// It rides the durable resource rather than a transient local, for
    /// `PendingGmStationCommands`' reason: the canonical reducer runs in
    /// `PreUpdate` and the re-validation in `FixedUpdate`, so a capture taken
    /// between the two would otherwise restore a world whose NPCs are still
    /// locked onto a ship the GM has just made friendly.
    #[serde(default, skip_serializing_if = "is_false")]
    pending_revalidation: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl GmFactionOverrides {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[GmFactionRelationOverride] {
        &self.entries
    }

    fn position(&self, faction: &str, enemy: &str) -> Result<usize, usize> {
        self.entries.binary_search_by(|entry| {
            (entry.faction.as_str(), entry.enemy.as_str()).cmp(&(faction, enemy))
        })
    }

    /// The value the canonical journal has made true for this pair, if a GM has
    /// ever moved it.
    pub fn current(&self, faction: &str, enemy: &str) -> Option<bool> {
        self.position(faction, enemy)
            .ok()
            .map(|index| self.entries[index].current)
    }

    /// Whether an AI tactical-lock re-validation is still owed.
    pub fn revalidation_pending(&self) -> bool {
        self.pending_revalidation
    }

    /// Record one applied GM change.
    ///
    /// `before` is honoured only the first time a pair is recorded — it is the
    /// pre-GM baseline a restore reverts to, not the previous value.
    pub fn record(&mut self, faction: &str, enemy: &str, before: bool, current: bool) {
        // Withdrawing a hostility can strand a retained tactical lock; adding
        // one cannot, because the tiers below retention consult the registry
        // organically on the next decision.
        self.pending_revalidation |= !current;
        match self.position(faction, enemy) {
            Ok(index) => self.entries[index].current = current,
            Err(index) => self.entries.insert(
                index,
                GmFactionRelationOverride {
                    faction: faction.to_string(),
                    enemy: enemy.to_string(),
                    before,
                    current,
                },
            ),
        }
    }
}

/// Read/write access to live faction hostility for the canonical GM reducer.
///
/// A [`SystemParam`] rather than four more parameters on
/// [`crate::gm_action::apply_due_actions`], which is already at Bevy's
/// parameter ceiling. Every field is `Option`/`Query` for the reducer's
/// standing reason: the pure journal fixtures and the replay harness run that
/// exact production system without a world to hold factions.
#[derive(bevy::ecs::system::SystemParam)]
pub struct GmFactionControl<'w> {
    pub registry: Option<ResMut<'w, crate::entities::config_cache::FactionRegistryResource>>,
    pub overrides: Option<ResMut<'w, GmFactionOverrides>>,
}

/// What a requested hostility change resolves to at the canonical apply tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GmFactionOutcome {
    /// The relation moved. `before` is what it held a moment ago.
    Applied { before: bool },
    /// The relation already held the requested value.
    NoOp { current: bool },
    /// One of the two names does not resolve to a live faction.
    UnknownFaction,
    /// This build has no faction registry at all.
    Unavailable,
}

impl GmFactionControl<'_> {
    /// Whether `faction` currently lists `enemy` as hostile, or `None` when
    /// either name does not resolve.
    pub fn hostile(&self, faction: &str, enemy: &str) -> Option<bool> {
        let registry = &self.registry.as_deref()?.0;
        let faction_uuid = registry.uuid_by_name(faction)?;
        let enemy_uuid = registry.uuid_by_name(enemy)?;
        Some(
            registry
                .get(&faction_uuid)
                .is_some_and(|config| config.enemies.contains(&enemy_uuid)),
        )
    }

    /// Set `faction`'s hostility toward `enemy` to `hostile`.
    ///
    /// The same two registry calls the authored trigger action makes, followed
    /// by the same AI target re-validation on a withdrawal — a lock retained by
    /// `ai_target_selection` never re-asks whether its target is still an
    /// enemy, so skipping this would leave an NPC shooting a ship a GM has just
    /// made friendly.
    pub fn set_hostile(&mut self, faction: &str, enemy: &str, hostile: bool) -> GmFactionOutcome {
        let Some(registry) = self.registry.as_deref_mut() else {
            return GmFactionOutcome::Unavailable;
        };
        let (Some(faction_uuid), Some(enemy_uuid)) = (
            registry.0.uuid_by_name(faction),
            registry.0.uuid_by_name(enemy),
        ) else {
            return GmFactionOutcome::UnknownFaction;
        };
        let changed = if hostile {
            registry.0.add_enemy(faction_uuid, enemy_uuid)
        } else {
            registry.0.remove_enemy(faction_uuid, enemy_uuid)
        };
        if !changed {
            return GmFactionOutcome::NoOp { current: hostile };
        }
        if let Some(overrides) = self.overrides.as_deref_mut() {
            overrides.record(faction, enemy, !hostile, hostile);
        }
        GmFactionOutcome::Applied { before: !hostile }
    }
}

/// Drop AI tactical locks a GM withdrawal has just made friendly.
///
/// The `remove_faction_enemy` trigger action performs exactly this sweep inline;
/// the canonical GM reducer cannot, because it runs in `PreUpdate` and already
/// borrows components that conflict with the lock query. So the reducer arms
/// [`GmFactionOverrides::revalidation_pending`] and this system — ordered
/// immediately before `ai_target_selection`, the other writer of that lock —
/// consumes it. `ai_target_selection`'s retention tier deliberately keeps an
/// established lock without re-asking whether the target is still an enemy, so
/// without this an NPC would go on shooting a ship the GM just made friendly.
pub fn revalidate_gm_faction_locks(
    registry: Option<Res<crate::entities::config_cache::FactionRegistryResource>>,
    mut overrides: Option<ResMut<GmFactionOverrides>>,
    non_ai_factions: Query<(&EntityUuid, &FactionComponent), Without<BehaviourSection>>,
    mut ai_locks: Query<
        (
            &EntityUuid,
            Option<&mut crate::console::weapons::TacticalRadarSelection>,
            Option<&FactionComponent>,
        ),
        With<BehaviourSection>,
    >,
) {
    let Some(overrides) = overrides.as_deref_mut() else {
        return;
    };
    if !overrides.pending_revalidation {
        return;
    }
    overrides.pending_revalidation = false;
    let Some(registry) = registry.as_deref() else {
        return;
    };
    let ai_factions: Vec<(uuid::Uuid, uuid::Uuid)> = ai_locks
        .iter()
        .filter_map(|(uid, _, faction)| Some((uuid::Uuid::parse_str(&uid.0).ok()?, faction?.0)))
        .collect();
    let uuid_to_faction =
        crate::world::server::build_uuid_to_faction(&non_ai_factions, &ai_factions);
    crate::world::server::revalidate_ai_targets_after_faction_change(
        &mut ai_locks,
        &registry.0,
        &uuid_to_faction,
    );
}

/// Re-apply a restored save's GM faction overrides to the live registry.
///
/// Called from [`crate::snapshot::restore`]. A pair the live run has moved but
/// the save never did is reverted to its recorded pre-GM baseline, so a restore
/// really rewinds to the saved world rather than leaving a later hostility
/// standing under an older journal.
pub fn restore_overrides(world: &mut World, restored: &GmFactionOverrides) {
    let live = world
        .get_resource::<GmFactionOverrides>()
        .cloned()
        .unwrap_or_default();
    let mut wanted: Vec<(String, String, bool)> = restored
        .entries()
        .iter()
        .map(|entry| (entry.faction.clone(), entry.enemy.clone(), entry.current))
        .collect();
    for entry in live.entries() {
        if restored.current(&entry.faction, &entry.enemy).is_none() {
            wanted.push((entry.faction.clone(), entry.enemy.clone(), entry.before));
        }
    }
    if let Some(mut registry) =
        world.get_resource_mut::<crate::entities::config_cache::FactionRegistryResource>()
    {
        for (faction, enemy, hostile) in &wanted {
            let (Some(faction_uuid), Some(enemy_uuid)) = (
                registry.0.uuid_by_name(faction),
                registry.0.uuid_by_name(enemy),
            ) else {
                continue;
            };
            if *hostile {
                registry.0.add_enemy(faction_uuid, enemy_uuid);
            } else {
                registry.0.remove_enemy(faction_uuid, enemy_uuid);
            }
        }
    }
    world.insert_resource(restored.clone());
}

/// One faction and the factions it currently treats as enemies.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmFactionRow {
    /// The authored reference `name`, which is what the typed action carries.
    pub name: String,
    /// The `strings.csv` id for a crew-facing label, when the setting authored
    /// one. Absent means this faction has no display name, and the GM page
    /// shows the reference name rather than putting English on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The reference names this faction considers enemies, sorted.
    pub enemies: Vec<String>,
}

/// Every live faction and its current hostilities, for the GM faction control.
///
/// Deterministically ordered by name: the registry is a `HashMap`, so an
/// unsorted walk would publish a different payload on every process.
pub fn faction_rows(registry: Option<&crate::ai::faction::FactionRegistry>) -> Vec<GmFactionRow> {
    let Some(registry) = registry else {
        return Vec::new();
    };
    let name_of: std::collections::BTreeMap<uuid::Uuid, &str> = registry
        .iter()
        .map(|config| (config.uuid, config.name.as_str()))
        .collect();
    let mut rows: Vec<GmFactionRow> = registry
        .iter()
        .map(|config| {
            let mut enemies: Vec<String> = config
                .enemies
                .iter()
                .filter_map(|uuid| name_of.get(uuid).map(|name| (*name).to_string()))
                .collect();
            enemies.sort();
            enemies.dedup();
            GmFactionRow {
                name: config.name.clone(),
                label: config.display_name.clone(),
                enemies,
            }
        })
        .collect();
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    rows.dedup_by(|left, right| left.name == right.name);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overrides() -> GmFactionOverrides {
        let mut value = GmFactionOverrides::default();
        value.record("Harrow", "Alliance", false, true);
        value
    }

    #[test]
    fn keeps_the_pre_gm_baseline_through_later_changes_to_the_same_pair() {
        let mut value = overrides();
        // Adding a hostility owes no lock re-validation; withdrawing one does.
        assert!(!value.revalidation_pending());
        value.record("Harrow", "Alliance", true, false);
        assert!(value.revalidation_pending());
        assert_eq!(value.entries().len(), 1);
        // `before` is the value the pair held before ANY GM touched it, so a
        // restore of a save that predates the first change reverts to `false`,
        // not to the value the second change happened to see.
        assert!(!value.entries()[0].before);
        assert!(!value.entries()[0].current);
        assert_eq!(value.current("Harrow", "Alliance"), Some(false));
    }

    #[test]
    fn stays_sorted_so_the_digest_and_snapshot_are_order_independent() {
        let mut value = GmFactionOverrides::default();
        value.record("Zephyr", "Alliance", false, true);
        value.record("Alliance", "Harrow", false, true);
        value.record("Alliance", "Alliance", false, true);
        assert_eq!(
            value
                .entries()
                .iter()
                .map(|entry| (entry.faction.as_str(), entry.enemy.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("Alliance", "Alliance"),
                ("Alliance", "Harrow"),
                ("Zephyr", "Alliance")
            ]
        );
        assert_eq!(value.current("Alliance", "Missing"), None);
    }

    #[test]
    fn projects_faction_rows_by_name_in_a_stable_order() {
        use crate::ai::faction::{FactionConfig, FactionRegistry};
        let alliance = uuid::Uuid::from_u128(1);
        let harrow = uuid::Uuid::from_u128(2);
        let mut registry = FactionRegistry::new();
        registry.insert(FactionConfig {
            uuid: harrow,
            name: "Harrow".into(),
            display_name: Some("dossier.faction.harrow".into()),
            enemies: vec![alliance],
            compliance: None,
        });
        registry.insert(FactionConfig {
            uuid: alliance,
            name: "Alliance".into(),
            display_name: None,
            enemies: vec![],
            compliance: None,
        });
        let rows = faction_rows(Some(&registry));
        assert_eq!(
            rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
            vec!["Alliance", "Harrow"]
        );
        assert_eq!(rows[0].enemies, Vec::<String>::new());
        assert_eq!(rows[1].enemies, vec!["Alliance".to_string()]);
        assert_eq!(rows[1].label.as_deref(), Some("dossier.faction.harrow"));
        assert!(faction_rows(None).is_empty());
    }
}
