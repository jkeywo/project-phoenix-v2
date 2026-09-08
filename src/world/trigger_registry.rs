//! Ordered World triggers, their script handlers, and continuation ownership.
//!
//! A handler and the state it describes enter and leave together. The registry
//! also owns the observer generation, so an equal-length layer replacement or
//! a continuation restore cannot preserve history belonging to earlier rows.
//! ASTs and execution budgets stay with the script runtime.

use std::collections::{HashMap, HashSet};
use std::ops::Index;

use serde::{Deserialize, Serialize};

use super::content::{evaluate_single_trigger, FiredTrigger, TriggerState, WorldEvent};
use super::flags::FlagStore;
use super::script::engine::ScriptTrigger;

/// The retained script unit and function supplying one trigger's effects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptHandlerRef {
    pub script_path: String,
    pub fn_name: String,
}

/// The existing snapshot row, in registry order. Authored identity and handlers
/// are rebuilt from the same content and layer activation order, not serialized.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TriggerRuntimeState {
    pub index: u32,
    pub fired: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub seen_destroyed: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_elapsed: Option<f32>,
}

/// A refused continuation leaves every live row and its generation untouched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerRestoreError {
    Cardinality {
        saved: usize,
        found: usize,
    },
    /// An index is out of range or repeated, so the rows cannot cover the table.
    InvalidIndex {
        index: u32,
    },
}

#[derive(Clone, Debug)]
struct TriggerEntry {
    state: TriggerState,
    handler: Option<ScriptHandlerRef>,
}

/// One completed condition evaluation with the handler paired to that row.
pub struct FiredWorldTrigger {
    pub trigger_id: Option<String>,
    pub handler: Option<ScriptHandlerRef>,
    pub context: FiredTrigger,
}

#[derive(Default)]
pub struct WorldTriggerRegistry {
    entries: Vec<TriggerEntry>,
    generation: u64,
}

/// Immutable ordered access without exposing mutable entries or handlers.
pub struct TriggerStates<'a>(std::slice::Iter<'a, TriggerEntry>);

impl<'a> Iterator for TriggerStates<'a> {
    type Item = &'a TriggerState;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|entry| &entry.state)
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}
impl ExactSizeIterator for TriggerStates<'_> {}
impl<'a> IntoIterator for &'a WorldTriggerRegistry {
    type Item = &'a TriggerState;
    type IntoIter = TriggerStates<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<Vec<TriggerState>> for WorldTriggerRegistry {
    fn from(states: Vec<TriggerState>) -> Self {
        let mut registry = Self::default();
        registry.replace_declarative(states);
        registry
    }
}

impl WorldTriggerRegistry {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A cache token, deliberately absent from snapshots and authoritative digests.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Ordered, immutable state for observers and the authoritative digest walk.
    pub fn iter(&self) -> TriggerStates<'_> {
        TriggerStates(self.entries.iter())
    }

    pub fn handler(&self, index: usize) -> Option<&ScriptHandlerRef> {
        self.entries
            .get(index)
            .and_then(|entry| entry.handler.as_ref())
    }

    /// Add an already-constructed declarative trigger, including pure fixtures
    /// that have no script runtime. Its absent handler is part of the entry.
    pub fn push(&mut self, state: TriggerState) {
        self.entries.push(TriggerEntry {
            state,
            handler: None,
        });
        self.changed();
    }

    /// Replace a declarative table without exposing independently mutable rows.
    pub fn replace_declarative(&mut self, states: Vec<TriggerState>) {
        if self.entries.is_empty() && states.is_empty() {
            return;
        }
        self.entries = states
            .into_iter()
            .map(|state| TriggerEntry {
                state,
                handler: None,
            })
            .collect();
        self.changed();
    }

    pub fn clear(&mut self) {
        if !self.entries.is_empty() {
            self.entries.clear();
            self.changed();
        }
    }

    /// Consume one layer's registrations in authored order. Each new state
    /// carries its owner beside its handler from the moment it becomes visible.
    pub fn append_scripted(
        &mut self,
        triggers: impl IntoIterator<Item = ScriptTrigger>,
        origin: Option<&str>,
    ) {
        let before = self.entries.len();
        self.entries
            .extend(triggers.into_iter().map(|trigger| TriggerEntry {
                state: TriggerState {
                    trigger: trigger.trigger,
                    fired: false,
                    origin_layer: origin.map(str::to_owned),
                    seen_destroyed: HashSet::new(),
                    last_fired_elapsed: None,
                },
                handler: Some(ScriptHandlerRef {
                    script_path: trigger.source_path,
                    fn_name: trigger.handler,
                }),
            }));
        if self.entries.len() != before {
            self.changed();
        }
    }

    pub fn remove_layer(&mut self, path: &str) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|entry| entry.state.origin_layer.as_deref() != Some(path));
        let removed = before - self.entries.len();
        if removed > 0 {
            self.changed();
        }
        removed
    }

    /// Evaluate the entire ordered table before any caller applies effects.
    /// The adapter supplies each owner's live flag/path chains; no table borrow
    /// or index lookup survives into effect application.
    pub fn evaluate<'a>(
        &mut self,
        events: &[WorldEvent],
        name_to_uuid: &HashMap<String, String>,
        entity_groups: &HashMap<String, HashSet<String>>,
        elapsed: f32,
        chains: impl FnMut(Option<&str>) -> (Vec<&'a FlagStore>, Vec<Option<String>>),
    ) -> Vec<FiredWorldTrigger> {
        self.evaluate_filtered(
            events,
            name_to_uuid,
            entity_groups,
            elapsed,
            chains,
            (|_| true, |_| true),
        )
    }

    /// Pause is checked before any condition state changes; dispatch filtering
    /// runs only after an occurrence has advanced its ordinary latch/cooldown.
    pub fn evaluate_filtered<'a>(
        &mut self,
        events: &[WorldEvent],
        name_to_uuid: &HashMap<String, String>,
        entity_groups: &HashMap<String, HashSet<String>>,
        elapsed: f32,
        mut chains: impl FnMut(Option<&str>) -> (Vec<&'a FlagStore>, Vec<Option<String>>),
        filters: (
            impl FnMut(&TriggerState) -> bool,
            impl FnMut(&TriggerState) -> bool,
        ),
    ) -> Vec<FiredWorldTrigger> {
        let (mut should_evaluate, mut should_dispatch) = filters;
        let mut fired = Vec::new();
        for entry in &mut self.entries {
            if !should_evaluate(&entry.state) {
                continue;
            }
            let (flags, layers) = chains(entry.state.origin_layer.as_deref());
            if let Some(context) = evaluate_single_trigger(
                &mut entry.state,
                events,
                name_to_uuid,
                &flags,
                &layers,
                entity_groups,
                elapsed,
            ) {
                if !should_dispatch(&entry.state) {
                    continue;
                }
                fired.push(FiredWorldTrigger {
                    trigger_id: entry.state.trigger.id.clone(),
                    handler: entry.handler.clone(),
                    context,
                });
            }
        }
        fired
    }

    /// Attempt selected manual occurrences in authored order. Neither automatic
    /// Pause nor Skip applies here. Suppressed predicates/cooldowns retain arms;
    /// spent and missing identities are removed. Returned handlers stay paired.
    pub fn fire_manual<'a>(
        &mut self,
        pending: &mut std::collections::BTreeSet<String>,
        elapsed: f32,
        mut identity: impl FnMut(&TriggerState) -> Option<String>,
        mut chains: impl FnMut(Option<&str>) -> Vec<&'a FlagStore>,
    ) -> Vec<FiredWorldTrigger> {
        let mut live = std::collections::BTreeSet::new();
        let mut fired = Vec::new();
        for entry in &mut self.entries {
            let Some(id) = identity(&entry.state) else {
                continue;
            };
            live.insert(id.clone());
            if !pending.contains(&id) {
                continue;
            }
            if !super::content::manual_fire_is_still_live(&entry.state) {
                pending.remove(&id);
                continue;
            }
            let flags = chains(entry.state.origin_layer.as_deref());
            if let Some(context) =
                super::content::fire_manual_trigger(&mut entry.state, &flags, elapsed)
            {
                pending.remove(&id);
                fired.push(FiredWorldTrigger {
                    trigger_id: entry.state.trigger.id.clone(),
                    handler: entry.handler.clone(),
                    context,
                });
            }
        }
        pending.retain(|id| live.contains(id));
        fired
    }

    pub fn reset_by_id(&mut self, id: &str) -> usize {
        let mut count = 0;
        for entry in &mut self.entries {
            count +=
                super::content::reset_triggers_by_id(std::slice::from_mut(&mut entry.state), id);
        }
        count
    }

    pub fn capture(&self) -> Vec<TriggerRuntimeState> {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let mut seen_destroyed: Vec<_> =
                    entry.state.seen_destroyed.iter().cloned().collect();
                seen_destroyed.sort();
                TriggerRuntimeState {
                    index: index as u32,
                    fired: entry.state.fired,
                    seen_destroyed,
                    last_fired_elapsed: entry.state.last_fired_elapsed,
                }
            })
            .collect()
    }

    /// Validate the complete stored index set before replacing any continuation.
    /// Restoring latches retains every authored row/handler and invalidates old
    /// observer baselines even when the table's shape is unchanged.
    pub fn restore(&mut self, rows: &[TriggerRuntimeState]) -> Result<(), TriggerRestoreError> {
        if rows.len() != self.entries.len() {
            return Err(TriggerRestoreError::Cardinality {
                saved: rows.len(),
                found: self.entries.len(),
            });
        }
        let mut seen = vec![false; self.entries.len()];
        for row in rows {
            let Some(present) = seen.get_mut(row.index as usize) else {
                return Err(TriggerRestoreError::InvalidIndex { index: row.index });
            };
            if *present {
                return Err(TriggerRestoreError::InvalidIndex { index: row.index });
            }
            *present = true;
        }
        for row in rows {
            let state = &mut self.entries[row.index as usize].state;
            state.fired = row.fired;
            state.seen_destroyed = row.seen_destroyed.iter().cloned().collect();
            state.last_fired_elapsed = row.last_fired_elapsed;
        }
        self.changed();
        Ok(())
    }

    fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

impl Index<usize> for WorldTriggerRegistry {
    type Output = TriggerState;

    fn index(&self, index: usize) -> &Self::Output {
        &self.entries[index].state
    }
}

#[cfg(test)]
#[path = "trigger_registry_tests.rs"]
mod tests;
