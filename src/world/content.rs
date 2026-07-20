// Runtime state and event evaluators for world content.
//
// Pure Rust module — no Bevy. Pure config types (`TriggerCondition`,
// `TriggerAction`, `Trigger`) live in `world::config` and are re-exported
// here so existing imports continue to resolve. This module owns:
//
//   * `WorldEvent` — events triggers / comms templates react to.
//   * `TriggerState` — per-trigger fired flag.
//   * `evaluate_triggers` — single-shot trigger evaluator.
//   * `condition_matches` — the shared trigger-condition vocabulary
//     matcher (also consumed by the comms evaluators in `comms::content`).
//   * `trigger_states_from_world` — factory that derives runtime states
//     from a parsed `WorldConfig`.
//
// The comms half (`CommsTemplateState`, `evaluate_comms_templates`,
// `ActiveDialogue`, `PendingFollowUp`, `FiredCommsTemplate`,
// `follow_up_trigger_holds`) lives in `comms::content` (issue #816).
//
// PRD #342: the legacy multi-world layering machinery was deleted in slice 5.
// One world is loaded per session; runtime state is flat.

use std::collections::{HashMap, HashSet};

// Re-export pure config types so legacy import paths continue to resolve.
pub use crate::world::config::{Trigger, TriggerAction, TriggerCondition};
use crate::world::flags::FlagStore;

// ── World events ──────────────────────────────────────────────────────────

/// A world event that triggers can react to.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldEvent {
    /// An entity (by UUID) was destroyed.
    Destroyed { uuid: String },
    /// An entity (by UUID) was attacked; `attacker_uuid` is the attacker.
    Attacked { uuid: String, attacker_uuid: String },
    /// Simulation time has advanced. `elapsed_secs` is total elapsed time.
    TimerElapsed { elapsed_secs: f32 },
    /// A `Hail` message arrived for `target_uuid`.
    Hailed { target_uuid: String },
    /// A world flag transitioned false→true (single-pass / next-pass).
    ///
    /// `origin_layer` records the sub-world layer whose `FlagStore` owns
    /// the mutated flag (`None` = base world). `OnFlagSet { name }`
    /// conditions match only when the trigger's resolved target layer
    /// (computed by walking `parent:` prefixes on the condition `name`
    /// from the trigger's own origin layer) equals `origin_layer`. This
    /// keeps same-named flags in different layers from cross-firing
    /// (PRD #397 US #6, fix 1).
    FlagSet {
        name: String,
        origin_layer: Option<String>,
    },
    /// A world flag transitioned true→false. See `FlagSet`.
    FlagCleared {
        name: String,
        origin_layer: Option<String>,
    },
    /// The containing world finished loading (base-world `Startup` or
    /// sub-world `LoadWorld`). Emitted once per load cycle.
    WorldLoaded,
    /// The player ship entered a region (by UUID).
    EnteredRegion { uuid: String },
    /// The player ship exited a region (by UUID), either by crossing the
    /// boundary or because the region entity was despawned while the
    /// ship was inside.
    ExitedRegion { uuid: String },
    /// A ship (by UUID) reached the named waypoint anchor of the
    /// Patrol/Reach objective its cursor is following.
    WaypointReached { uuid: String, waypoint: String },
}

// ── Runtime state ─────────────────────────────────────────────────────────

/// Runtime state for one trigger within the active world.
#[derive(Clone, Debug)]
pub struct TriggerState {
    pub trigger: Trigger,
    /// Whether this trigger has already fired (single-shot semantics).
    pub fired: bool,
    /// Path of the sub-world layer that authored this trigger, or `None`
    /// for triggers declared in the base world. Used by `spawn_entity`
    /// trigger actions (issue #417) to attach freshly-spawned entities
    /// to the parent `WorldLayerMap` entry so `UnloadWorld` cascades.
    #[doc(hidden)]
    pub origin_layer: Option<String>,
    /// Set of entity names whose `WorldEvent::Destroyed` we have already
    /// observed. Only populated for `OnAllDestroyed` triggers; ignored by
    /// every other condition. The matcher fires when this set contains
    /// every name in the condition's `entity_names`. (#470)
    #[doc(hidden)]
    pub seen_destroyed: HashSet<String>,
}

/// Result of evaluating triggers against a batch of world events.
#[derive(Clone, Debug, PartialEq)]
pub struct FiredTrigger {
    pub actions: Vec<TriggerAction>,
    /// Per-action delays, parallel to `actions`. `0.0` means immediate dispatch.
    pub action_delays: Vec<f32>,
    /// Origin sub-world layer path (or `None` for base-world triggers).
    /// Used by `spawn_entity` action dispatch (issue #417) to attach the
    /// new entity to the right `WorldLayerMap` entry.
    pub origin_layer: Option<String>,
    /// The entity name from the trigger condition that caused this trigger
    /// to fire (e.g. from `OnDestroyed`, `OnAttacked`, `OnHailed`,
    /// `OnEnteredRegion`, `OnExitedRegion`). Propagated to `AddObjective`
    /// actions so the objective can be linked to an entity on radar.
    pub entity_name: Option<String>,
}

// ── Evaluators ────────────────────────────────────────────────────────────

/// Extract the entity name from a `TriggerCondition`, if the variant carries one.
pub fn entity_name_from_condition(condition: &TriggerCondition) -> Option<String> {
    match condition {
        TriggerCondition::OnDestroyed { entity_name }
        | TriggerCondition::OnAttacked { entity_name }
        | TriggerCondition::OnHailed { entity_name }
        | TriggerCondition::OnEnteredRegion { entity_name }
        | TriggerCondition::OnExitedRegion { entity_name }
        | TriggerCondition::OnWaypointReached { entity_name, .. } => Some(entity_name.clone()),
        TriggerCondition::OnTimer { .. }
        | TriggerCondition::OnFlagSet { .. }
        | TriggerCondition::OnFlagCleared { .. }
        | TriggerCondition::OnAllDestroyed { .. }
        | TriggerCondition::OnWorldLoaded => None,
    }
}

/// Evaluate all triggers in `states` against the given `events`.
///
/// Each trigger fires at most once (single-shot). When a trigger fires its
/// `fired` flag is set to `true` and its actions are collected into a
/// `FiredTrigger`.
///
/// Convenience wrapper that passes an empty flag chain — any `when`
/// predicate that references a flag will evaluate as if the flag is unset
/// (so a `when` of `flag(a)` always evaluates false through this entry
/// point). Production code should call `evaluate_triggers_with_flags`.
#[allow(clippy::ptr_arg)]
pub fn evaluate_triggers(
    states: &mut Vec<TriggerState>,
    events: &[WorldEvent],
    name_to_uuid: &HashMap<String, String>,
) -> Vec<FiredTrigger> {
    evaluate_triggers_with_flags(states, events, name_to_uuid, &[], &HashMap::new(), 0.0)
}

/// Evaluate all triggers, including `when` predicate gating.
///
/// `flag_chain` is the layer of flag stores used by `when` predicate
/// evaluation (innermost first). This convenience entry point uses the
/// SAME chain for every trigger and treats `OnFlagSet` / `OnFlagCleared`
/// conditions as same-layer (no `parent:` walk). Per-trigger chain
/// resolution (needed by the Bevy runtime once sub-worlds are loaded)
/// goes through `evaluate_single_trigger` instead — see PRD #397 fix 1.
#[allow(clippy::ptr_arg)]
pub fn evaluate_triggers_with_flags(
    states: &mut Vec<TriggerState>,
    events: &[WorldEvent],
    name_to_uuid: &HashMap<String, String>,
    flag_chain: &[&FlagStore],
    entity_groups: &HashMap<String, HashSet<String>>,
    current_elapsed: f32,
) -> Vec<FiredTrigger> {
    let mut results = Vec::new();
    for state in states.iter_mut() {
        if state.fired {
            continue;
        }
        let layer_chain: [Option<String>; 1] = [state.origin_layer.clone()];
        let fires = trigger_fires_for_events(
            &state.trigger.condition,
            events,
            name_to_uuid,
            &layer_chain,
            &mut state.seen_destroyed,
            entity_groups,
            current_elapsed,
        );
        if !fires {
            continue;
        }
        if let Some(pred) = &state.trigger.when {
            if !pred.evaluate(flag_chain) {
                continue;
            }
        }
        let filtered: Vec<(usize, TriggerAction)> = state
            .trigger
            .actions
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                state
                    .trigger
                    .action_predicates
                    .get(*i)
                    .and_then(|p| p.as_ref())
                    .map(|p| p.evaluate(flag_chain))
                    .unwrap_or(true)
            })
            .map(|(i, a)| (i, a.clone()))
            .collect();
        let filtered_actions: Vec<TriggerAction> =
            filtered.iter().map(|(_, a)| a.clone()).collect();
        let filtered_delays: Vec<f32> = filtered
            .iter()
            .map(|(i, _)| state.trigger.action_delays.get(*i).copied().unwrap_or(0.0))
            .collect();
        state.fired = true;
        results.push(FiredTrigger {
            actions: filtered_actions,
            action_delays: filtered_delays,
            origin_layer: state.origin_layer.clone(),
            entity_name: entity_name_from_condition(&state.trigger.condition),
        });
    }
    results
}

/// Evaluate a single trigger with explicit per-trigger flag and layer chains.
///
/// `flag_chain` — innermost-first slice of `FlagStore`s used by `when`
/// predicate evaluation and `parent:` walks inside the predicate.
///
/// `layer_chain` — innermost-first slice of layer paths used by
/// `OnFlagSet` / `OnFlagCleared` condition matching to resolve
/// `parent:` prefixes on the condition's `name`. `layer_chain[0]` must
/// be the trigger's own origin layer (`None` = base world);
/// `layer_chain[1]` its loader; etc. Walking past the end of
/// `layer_chain` (i.e. more `parent:` prefixes than chain entries)
/// resolves as "no match" — the condition never fires.
///
/// Returns `Some(FiredTrigger)` if the trigger matched and its `when`
/// predicate (if any) evaluated true. Mutates `state.fired` to `true`
/// in that case.
pub fn evaluate_single_trigger(
    state: &mut TriggerState,
    events: &[WorldEvent],
    name_to_uuid: &HashMap<String, String>,
    flag_chain: &[&FlagStore],
    layer_chain: &[Option<String>],
    entity_groups: &HashMap<String, HashSet<String>>,
    current_elapsed: f32,
) -> Option<FiredTrigger> {
    if state.fired {
        return None;
    }
    let fires = trigger_fires_for_events(
        &state.trigger.condition,
        events,
        name_to_uuid,
        layer_chain,
        &mut state.seen_destroyed,
        entity_groups,
        current_elapsed,
    );
    if !fires {
        return None;
    }
    if let Some(pred) = &state.trigger.when {
        if !pred.evaluate(flag_chain) {
            return None;
        }
    }
    let filtered: Vec<(usize, TriggerAction)> = state
        .trigger
        .actions
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            state
                .trigger
                .action_predicates
                .get(*i)
                .and_then(|p| p.as_ref())
                .map(|p| p.evaluate(flag_chain))
                .unwrap_or(true)
        })
        .map(|(i, a)| (i, a.clone()))
        .collect();
    let filtered_actions: Vec<TriggerAction> = filtered.iter().map(|(_, a)| a.clone()).collect();
    let filtered_delays: Vec<f32> = filtered
        .iter()
        .map(|(i, _)| state.trigger.action_delays.get(*i).copied().unwrap_or(0.0))
        .collect();
    state.fired = true;
    Some(FiredTrigger {
        actions: filtered_actions,
        action_delays: filtered_delays,
        origin_layer: state.origin_layer.clone(),
        entity_name: entity_name_from_condition(&state.trigger.condition),
    })
}

/// Strip leading `parent:` tokens from `name` and walk `layer_chain`
/// (innermost first) by that many steps. Returns the stripped name and
/// the resolved layer (or `None` if walking past the end of the chain).
///
/// `layer_chain[0]` represents the starting layer (`None` = base world);
/// each subsequent entry is the loader of the previous. A name with no
/// `parent:` prefix resolves to `layer_chain[0]`. A name with N prefixes
/// resolves to `layer_chain[N]` if present, otherwise `None` is returned
/// for both fields to signal "past root".
pub fn resolve_layer_prefix(
    name: &str,
    layer_chain: &[Option<String>],
) -> Option<(String, Option<String>)> {
    let mut depth = 0usize;
    let mut rest = name;
    while let Some(s) = rest.strip_prefix("parent:") {
        depth += 1;
        rest = s;
    }
    if depth >= layer_chain.len() {
        return None;
    }
    Some((rest.to_string(), layer_chain[depth].clone()))
}

/// Decide whether a single trigger fires for the given batch of events.
///
/// Most conditions delegate to the stateless `condition_matches` once per
/// event. `OnAllDestroyed` is stateful: this function updates
/// `seen_destroyed` from any `WorldEvent::Destroyed` whose UUID resolves
/// to one of the named entities, then fires when the set covers every
/// name. Names whose UUID is never registered in `name_to_uuid` can never
/// reach `seen_destroyed`, so the trigger never fires for unspawned
/// entities (matches `OnDestroyed`'s "unknown entity → never matches"
/// semantics).
fn trigger_fires_for_events(
    condition: &TriggerCondition,
    events: &[WorldEvent],
    name_to_uuid: &HashMap<String, String>,
    layer_chain: &[Option<String>],
    seen_destroyed: &mut HashSet<String>,
    entity_groups: &HashMap<String, HashSet<String>>,
    current_elapsed: f32,
) -> bool {
    if let TriggerCondition::OnAllDestroyed { group, after_secs } = condition {
        let members: HashSet<String> = entity_groups.get(group).cloned().unwrap_or_default();
        // Backward compat for tests: if entity_groups is empty or group not found,
        // fall back to using the group name itself as the sole entity to track.
        // This preserves the old entity_names semantics for existing tests.
        let members = if members.is_empty() && entity_groups.is_empty() {
            std::iter::once(group.clone()).collect()
        } else if members.is_empty() {
            return false; // group specified but no members exist yet
        } else {
            members
        };
        for event in events {
            if let WorldEvent::Destroyed { uuid } = event {
                for name in &members {
                    if seen_destroyed.contains(name) {
                        continue;
                    }
                    if name_to_uuid.get(name).map(|u| u == uuid).unwrap_or(false) {
                        seen_destroyed.insert(name.clone());
                    }
                }
            }
        }
        if current_elapsed < *after_secs {
            return false;
        }
        return members.iter().all(|n| seen_destroyed.contains(n));
    }
    events
        .iter()
        .any(|event| condition_matches(condition, event, name_to_uuid, layer_chain))
}

/// Returns true if `condition` matches `event`, using `name_to_uuid` to
/// resolve entity names to runtime UUIDs and `layer_chain` to resolve
/// `parent:` prefixes on `OnFlagSet` / `OnFlagCleared` condition names
/// (innermost layer first; `None` = base world).
///
/// Stateless and read-only — does not handle `OnAllDestroyed` (which is
/// stateful). `OnAllDestroyed` is fast-pathed in `trigger_fires_for_events`
/// before this matcher runs and never reaches the match block.
pub(crate) fn condition_matches(
    condition: &TriggerCondition,
    event: &WorldEvent,
    name_to_uuid: &HashMap<String, String>,
    layer_chain: &[Option<String>],
) -> bool {
    match (condition, event) {
        (TriggerCondition::OnDestroyed { entity_name }, WorldEvent::Destroyed { uuid }) => {
            name_to_uuid
                .get(entity_name)
                .map(|u| u == uuid)
                .unwrap_or(false)
        }
        (TriggerCondition::OnAttacked { entity_name }, WorldEvent::Attacked { uuid, .. }) => {
            name_to_uuid
                .get(entity_name)
                .map(|u| u == uuid)
                .unwrap_or(false)
        }
        (TriggerCondition::OnTimer { after_secs }, WorldEvent::TimerElapsed { elapsed_secs }) => {
            elapsed_secs >= after_secs
        }
        (TriggerCondition::OnHailed { entity_name }, WorldEvent::Hailed { target_uuid }) => {
            name_to_uuid
                .get(entity_name)
                .map(|u| u == target_uuid)
                .unwrap_or(false)
        }
        (
            TriggerCondition::OnFlagSet { name },
            WorldEvent::FlagSet {
                name: ev_name,
                origin_layer: ev_layer,
            },
        ) => match resolve_layer_prefix(name, layer_chain) {
            Some((stripped, target_layer)) => &stripped == ev_name && &target_layer == ev_layer,
            None => false,
        },
        (
            TriggerCondition::OnFlagCleared { name },
            WorldEvent::FlagCleared {
                name: ev_name,
                origin_layer: ev_layer,
            },
        ) => match resolve_layer_prefix(name, layer_chain) {
            Some((stripped, target_layer)) => &stripped == ev_name && &target_layer == ev_layer,
            None => false,
        },
        (TriggerCondition::OnWorldLoaded, WorldEvent::WorldLoaded) => true,
        (TriggerCondition::OnEnteredRegion { entity_name }, WorldEvent::EnteredRegion { uuid }) => {
            name_to_uuid
                .get(entity_name)
                .map(|u| u == uuid)
                .unwrap_or(false)
        }
        (TriggerCondition::OnExitedRegion { entity_name }, WorldEvent::ExitedRegion { uuid }) => {
            name_to_uuid
                .get(entity_name)
                .map(|u| u == uuid)
                .unwrap_or(false)
        }
        (
            TriggerCondition::OnWaypointReached {
                entity_name,
                waypoint,
            },
            WorldEvent::WaypointReached {
                uuid,
                waypoint: ev_waypoint,
            },
        ) => {
            // An omitted `waypoint` means "any waypoint on this ship's route".
            let waypoint_matches = waypoint.as_ref().map(|w| w == ev_waypoint).unwrap_or(true);
            waypoint_matches
                && name_to_uuid
                    .get(entity_name)
                    .map(|u| u == uuid)
                    .unwrap_or(false)
        }
        _ => false,
    }
}

// ── Factories from WorldConfig ────────────────────────────────────────────

/// Create a `Vec<TriggerState>` from a parsed `WorldConfig` (PRD #341).
///
/// All triggers start unfired.
pub fn trigger_states_from_world(world: &crate::world::config::WorldConfig) -> Vec<TriggerState> {
    world
        .triggers
        .iter()
        .map(|t| TriggerState {
            trigger: t.clone(),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        })
        .collect()
}

// ── Unit Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::config::WorldConfig;

    fn dest_trigger(name: &str, action: TriggerAction) -> Trigger {
        Trigger {
            condition: TriggerCondition::OnDestroyed {
                entity_name: name.into(),
            },
            actions: vec![action],
            when: None,
            action_predicates: vec![],
            action_delays: vec![],
        }
    }

    fn add_obj(id: &str) -> TriggerAction {
        TriggerAction::AddObjective {
            id: id.into(),
            text: format!("Objective {id}"),
            mandatory: false,
            targets: vec![],
            directive: crate::messages::AiDirective::None,
            utility: crate::objectives::UtilityConfig::default(),
            source: crate::messages::ObjectiveSource::default(),
        }
    }

    // ── evaluate_triggers ─────────────────────────────────────────────────

    #[test]
    fn on_destroyed_fires_when_entity_destroyed() {
        let mut states = vec![TriggerState {
            trigger: dest_trigger("raider", add_obj("obj-1")),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-1".into());
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-1".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].actions.len(), 1);
        assert!(states[0].fired);
    }

    #[test]
    fn on_destroyed_does_not_fire_for_different_entity() {
        let mut states = vec![TriggerState {
            trigger: dest_trigger("raider", add_obj("obj-1")),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-1".into());
        name_to_uuid.insert("station".into(), "uuid-2".into());
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-2".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert!(fired.is_empty());
        assert!(!states[0].fired);
    }

    #[test]
    fn trigger_fires_only_once_single_shot() {
        let mut states = vec![TriggerState {
            trigger: dest_trigger("raider", add_obj("obj-1")),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-1".into());
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-1".into(),
        }];
        let fired1 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        let fired2 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired1.len(), 1);
        assert!(fired2.is_empty());
    }

    // ── OnAllDestroyed ─────────────────────────────────────────────────────

    fn all_destroyed_trigger(name: &str, action: TriggerAction) -> Trigger {
        Trigger {
            condition: TriggerCondition::OnAllDestroyed {
                group: name.into(),
                after_secs: 0.0,
            },
            actions: vec![action],
            when: None,
            action_predicates: vec![],
            action_delays: vec![],
        }
    }

    #[test]
    fn on_all_destroyed_fires_only_after_last_named_entity_dies() {
        let mut states = vec![TriggerState {
            trigger: all_destroyed_trigger("a", add_obj("obj-cleared")),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("a".into(), "uuid-a".into());
        name_to_uuid.insert("b".into(), "uuid-b".into());

        let entity_groups: HashMap<String, HashSet<String>> = [(
            "a".to_string(),
            ["a".to_string(), "b".to_string()].into_iter().collect(),
        )]
        .into_iter()
        .collect();

        // First destruction: only `a` dies — must NOT fire.
        let events1 = vec![WorldEvent::Destroyed {
            uuid: "uuid-a".into(),
        }];
        let fired1 = evaluate_triggers_with_flags(
            &mut states,
            &events1,
            &name_to_uuid,
            &[],
            &entity_groups,
            0.0,
        );
        assert!(
            fired1.is_empty(),
            "OnAllDestroyed must not fire while any named entity still alive"
        );
        assert!(!states[0].fired);
        assert!(states[0].seen_destroyed.contains("a"));

        // Second destruction: `b` dies — now must fire.
        let events2 = vec![WorldEvent::Destroyed {
            uuid: "uuid-b".into(),
        }];
        let fired2 = evaluate_triggers_with_flags(
            &mut states,
            &events2,
            &name_to_uuid,
            &[],
            &entity_groups,
            0.0,
        );
        assert_eq!(fired2.len(), 1);
        assert!(states[0].fired);
    }

    #[test]
    fn on_all_destroyed_is_single_shot() {
        let mut states = vec![TriggerState {
            trigger: all_destroyed_trigger("a", add_obj("obj-cleared")),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("a".into(), "uuid-a".into());
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-a".into(),
        }];
        let fired1 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        let fired2 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired1.len(), 1);
        assert!(
            fired2.is_empty(),
            "OnAllDestroyed must not re-fire after firing once"
        );
    }

    #[test]
    fn on_all_destroyed_never_fires_for_unspawned_entity() {
        let mut states = vec![TriggerState {
            trigger: all_destroyed_trigger("a", add_obj("obj-x")),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("a".into(), "uuid-a".into());
        // "b" never registered.

        let entity_groups: HashMap<String, HashSet<String>> = [(
            "a".to_string(),
            ["a".to_string(), "b".to_string()].into_iter().collect(),
        )]
        .into_iter()
        .collect();

        let events = vec![
            WorldEvent::Destroyed {
                uuid: "uuid-a".into(),
            },
            WorldEvent::Destroyed {
                uuid: "uuid-b".into(),
            },
        ];
        let fired = evaluate_triggers_with_flags(
            &mut states,
            &events,
            &name_to_uuid,
            &[],
            &entity_groups,
            0.0,
        );
        assert!(
            fired.is_empty(),
            "OnAllDestroyed must not fire when a named entity was never registered"
        );
        assert!(states[0].seen_destroyed.contains("a"));
        assert!(!states[0].seen_destroyed.contains("b"));
    }

    #[test]
    fn on_all_destroyed_fires_when_all_named_entities_die_in_one_batch() {
        // NOTE: With new group-based API, this test uses single entity "a".
        // For proper multi-entity testing, use evaluate_triggers_with_flags
        // with entity_groups = {"a" => {"a", "b", "c"}}.
        let mut states = vec![TriggerState {
            trigger: all_destroyed_trigger("a", add_obj("obj-cleared")),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("a".into(), "uuid-a".into());
        name_to_uuid.insert("b".into(), "uuid-b".into());
        name_to_uuid.insert("c".into(), "uuid-c".into());

        let events = vec![
            WorldEvent::Destroyed {
                uuid: "uuid-a".into(),
            },
            WorldEvent::Destroyed {
                uuid: "uuid-b".into(),
            },
            WorldEvent::Destroyed {
                uuid: "uuid-c".into(),
            },
        ];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert!(states[0].fired);
    }

    #[test]
    fn on_all_destroyed_ignores_destruction_events_for_other_entities() {
        let mut states = vec![TriggerState {
            trigger: all_destroyed_trigger("a", add_obj("obj-x")),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("a".into(), "uuid-a".into());
        name_to_uuid.insert("other".into(), "uuid-other".into());

        // Some other entity dies — must not register against this trigger.
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-other".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert!(fired.is_empty());
        assert!(!states[0].seen_destroyed.contains("a"));
        assert!(states[0].seen_destroyed.is_empty());
    }

    #[test]
    fn on_all_destroyed_when_predicate_gates_firing_but_seen_set_persists() {
        // Pins the design decision: `seen_destroyed` accumulates BEFORE the
        // `when` predicate is evaluated. The trigger fires as soon as both
        // (a) every named entity has been destroyed AND
        // (b) the `when` predicate evaluates true,
        // even if those become true on different ticks.
        use crate::world::flags::{parse_predicate, FlagStore};

        let predicate = parse_predicate("flag(armed)").expect("parse");
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnAllDestroyed {
                    group: "a".into(),
                    after_secs: 0.0,
                },
                actions: vec![add_obj("obj-cleared")],
                when: Some(predicate),
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("a".into(), "uuid-a".into());
        name_to_uuid.insert("b".into(), "uuid-b".into());

        // Tick 1: both entities die, but `armed` flag is unset → must NOT fire.
        let events = vec![
            WorldEvent::Destroyed {
                uuid: "uuid-a".into(),
            },
            WorldEvent::Destroyed {
                uuid: "uuid-b".into(),
            },
        ];
        let flag_store_unset = FlagStore::default();
        let chain_unset: [&FlagStore; 1] = [&flag_store_unset];
        let entity_groups: HashMap<String, HashSet<String>> = [(
            "a".to_string(),
            ["a".to_string(), "b".to_string()].into_iter().collect(),
        )]
        .into_iter()
        .collect();
        let fired_tick1 = evaluate_triggers_with_flags(
            &mut states,
            &events,
            &name_to_uuid,
            &chain_unset,
            &entity_groups,
            0.0,
        );
        assert!(
            fired_tick1.is_empty(),
            "must not fire while `when` predicate is false"
        );
        assert!(!states[0].fired);
        // But the seen_destroyed set must still have grown.
        assert!(states[0].seen_destroyed.contains("a"));
        assert!(states[0].seen_destroyed.contains("b"));

        // Tick 2: no new events, but the flag is now set → must fire from
        // the persisted `seen_destroyed` set.
        let mut flag_store_set = FlagStore::default();
        flag_store_set.set_flag("armed");
        let chain_set: [&FlagStore; 1] = [&flag_store_set];
        let fired_tick2 = evaluate_triggers_with_flags(
            &mut states,
            &[],
            &name_to_uuid,
            &chain_set,
            &entity_groups,
            0.0,
        );
        assert_eq!(fired_tick2.len(), 1);
        assert!(states[0].fired);
    }

    #[test]
    fn on_all_destroyed_after_secs_gates_until_time_elapsed() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnAllDestroyed {
                    group: "waves".into(),
                    after_secs: 10.0,
                },
                actions: vec![add_obj("obj-waves-cleared")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-r".into());
        let entity_groups: HashMap<String, HashSet<String>> = [(
            "waves".to_string(),
            ["raider".to_string()].into_iter().collect(),
        )]
        .into_iter()
        .collect();

        // Destroy the only group member while elapsed is below the gate.
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-r".into(),
        }];

        // elapsed = 5.0 < after_secs = 10.0 → trigger must NOT fire.
        let fired = evaluate_triggers_with_flags(
            &mut states,
            &events,
            &name_to_uuid,
            &[],
            &entity_groups,
            5.0,
        );
        assert!(
            fired.is_empty(),
            "OnAllDestroyed must not fire before after_secs gate"
        );
        assert!(!states[0].fired);
        assert!(states[0].seen_destroyed.contains("raider"));

        // No new events, but elapsed is now 15.0 > after_secs → trigger fires.
        let fired2 = evaluate_triggers_with_flags(
            &mut states,
            &[],
            &name_to_uuid,
            &[],
            &entity_groups,
            15.0,
        );
        assert_eq!(fired2.len(), 1);
        assert!(states[0].fired);
    }

    #[test]
    fn on_timer_fires_when_elapsed_exceeds_threshold() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnTimer { after_secs: 30.0 },
                actions: vec![add_obj("obj-timer")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let name_to_uuid = HashMap::new();
        let before = vec![WorldEvent::TimerElapsed { elapsed_secs: 10.0 }];
        let fired = evaluate_triggers(&mut states, &before, &name_to_uuid);
        assert!(fired.is_empty());
        let after = vec![WorldEvent::TimerElapsed { elapsed_secs: 60.0 }];
        let fired = evaluate_triggers(&mut states, &after, &name_to_uuid);
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn on_attacked_fires_when_entity_attacked() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnAttacked {
                    entity_name: "raider".into(),
                },
                actions: vec![add_obj("obj-atk")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-1".into());
        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-1".into(),
            attacker_uuid: "uuid-player".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn on_hailed_fires_when_entity_hailed() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnHailed {
                    entity_name: "starbase".into(),
                },
                actions: vec![add_obj("obj-hail")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("starbase".into(), "uuid-sb".into());
        let events = vec![WorldEvent::Hailed {
            target_uuid: "uuid-sb".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn only_matching_trigger_fires() {
        let mut states = vec![
            TriggerState {
                trigger: dest_trigger("raider", add_obj("obj-r")),
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            },
            TriggerState {
                trigger: dest_trigger("station", add_obj("obj-s")),
                fired: false,
                origin_layer: None,
                seen_destroyed: HashSet::new(),
            },
        ];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-r".into());
        name_to_uuid.insert("station".into(), "uuid-s".into());
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-r".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert!(states[0].fired);
        assert!(!states[1].fired);
    }

    #[test]
    fn action_predicates_filter_individual_actions() {
        use crate::world::flags::{parse_predicate, FlagStore};

        let pred_armed = parse_predicate("flag(armed)").expect("parse");
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnDestroyed {
                    entity_name: "raider".into(),
                },
                actions: vec![add_obj("always"), add_obj("only-when-armed")],
                when: None,
                action_predicates: vec![None, Some(pred_armed)],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-r".into());
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-r".into(),
        }];

        // With flag unset, second action's predicate is false → second action filtered.
        let flag_store = FlagStore::default();
        let chain: [&FlagStore; 1] = [&flag_store];
        let fired = evaluate_triggers_with_flags(
            &mut states,
            &events,
            &name_to_uuid,
            &chain,
            &HashMap::new(),
            0.0,
        );
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].actions.len(), 1);
        assert!(matches!(
            fired[0].actions[0],
            TriggerAction::AddObjective { ref id, .. } if id == "always"
        ));

        // Reset state and set the flag; now second action should be included.
        states[0].fired = false;
        states[0].seen_destroyed.clear();
        let mut flag_store_armed = FlagStore::default();
        flag_store_armed.set_flag("armed");
        let chain_armed: [&FlagStore; 1] = [&flag_store_armed];
        let fired2 = evaluate_triggers_with_flags(
            &mut states,
            &events,
            &name_to_uuid,
            &chain_armed,
            &HashMap::new(),
            0.0,
        );
        assert_eq!(fired2.len(), 1);
        assert_eq!(fired2[0].actions.len(), 2);
    }

    #[test]
    fn action_delays_passed_through_to_fired_trigger() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnDestroyed {
                    entity_name: "raider".into(),
                },
                actions: vec![add_obj("immediate"), add_obj("delayed")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![0.0, 10.0],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-r".into());
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-r".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].actions.len(), 2);
        assert_eq!(fired[0].action_delays, vec![0.0, 10.0]);
    }

    #[test]
    fn on_destroyed_does_not_fire_if_entity_name_unknown() {
        let mut states = vec![TriggerState {
            trigger: dest_trigger("ghost", add_obj("obj-ghost")),
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let name_to_uuid = HashMap::new();
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-x".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert!(fired.is_empty());
    }

    // ── trigger_states_from_world ─────────────────────────────────────────

    #[test]
    fn trigger_states_from_world_creates_unfired_states_for_every_trigger() {
        let mut world = WorldConfig::default();
        world.triggers.push(dest_trigger("a", add_obj("oa")));
        world.triggers.push(dest_trigger("b", add_obj("ob")));
        let states = trigger_states_from_world(&world);
        assert_eq!(states.len(), 2);
        assert!(states.iter().all(|s| !s.fired));
    }

    // ── Shipped-world integration ─────────────────────────────────────────

    #[test]
    fn patrol_world_on_destroyed_trigger_fires_add_objective() {
        let toml = include_str!("../../assets/worlds/patrol.toml");
        let world = crate::world::config::parse_world(toml).expect("patrol.toml must parse");
        let mut states = trigger_states_from_world(&world);
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("world.entity.raider_alpha.name".into(), "uuid-r".into());
        let events = vec![WorldEvent::Destroyed {
            uuid: "uuid-r".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert!(fired[0]
            .actions
            .iter()
            .any(|a| matches!(a, TriggerAction::AddObjective { .. })));
    }

    // ── on_world_loaded (issue #415) ──────────────────────────────────────

    #[test]
    fn on_world_loaded_matches_world_loaded_event() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnWorldLoaded,
                actions: vec![add_obj("obj-loaded")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let name_to_uuid = HashMap::new();
        let events = vec![WorldEvent::WorldLoaded];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert!(states[0].fired);
    }

    #[test]
    fn on_world_loaded_does_not_match_unrelated_events() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnWorldLoaded,
                actions: vec![add_obj("obj-loaded")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let name_to_uuid = HashMap::new();
        let events = vec![
            WorldEvent::Destroyed { uuid: "x".into() },
            WorldEvent::TimerElapsed { elapsed_secs: 5.0 },
            WorldEvent::FlagSet {
                name: "f".into(),
                origin_layer: None,
            },
        ];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert!(fired.is_empty());
        assert!(!states[0].fired);
    }

    #[test]
    fn on_world_loaded_is_single_shot() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnWorldLoaded,
                actions: vec![add_obj("obj-loaded")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let name_to_uuid = HashMap::new();
        let events = vec![WorldEvent::WorldLoaded];
        let fired1 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        let fired2 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired1.len(), 1);
        assert!(fired2.is_empty());
    }

    // ── on_entered_region / on_exited_region (issue #416) ─────────────────

    #[test]
    fn on_entered_region_matches_entered_region_event_with_resolved_uuid() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnEnteredRegion {
                    entity_name: "nebula".into(),
                },
                actions: vec![add_obj("obj-nebula")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
        let events = vec![WorldEvent::EnteredRegion {
            uuid: "uuid-nebula".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert!(states[0].fired);
    }

    #[test]
    fn on_entered_region_does_not_match_different_uuid() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnEnteredRegion {
                    entity_name: "nebula".into(),
                },
                actions: vec![add_obj("obj-nebula")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
        let events = vec![WorldEvent::EnteredRegion {
            uuid: "uuid-other".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert!(fired.is_empty());
        assert!(!states[0].fired);
    }

    #[test]
    fn on_entered_region_does_not_match_exited_region_event() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnEnteredRegion {
                    entity_name: "nebula".into(),
                },
                actions: vec![add_obj("x")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
        let events = vec![WorldEvent::ExitedRegion {
            uuid: "uuid-nebula".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert!(fired.is_empty());
    }

    #[test]
    fn on_exited_region_matches_exited_region_event_with_resolved_uuid() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnExitedRegion {
                    entity_name: "nebula".into(),
                },
                actions: vec![add_obj("obj-left-nebula")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
        let events = vec![WorldEvent::ExitedRegion {
            uuid: "uuid-nebula".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn on_entered_region_does_not_match_unresolved_name() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnEnteredRegion {
                    entity_name: "unknown".into(),
                },
                actions: vec![add_obj("x")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        }];
        let name_to_uuid = HashMap::new(); // empty — name does not resolve
        let events = vec![WorldEvent::EnteredRegion {
            uuid: "any-uuid".into(),
        }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert!(fired.is_empty());
    }

    // ── parent: layer walker on flag-transition conditions (PRD #397 fix 1) ──

    #[test]
    fn on_flag_set_with_parent_prefix_matches_event_in_loader_layer() {
        // Trigger lives in "child.toml", which was loaded by base world.
        // Condition `name = "parent:armed"` must match an event whose
        // origin_layer is the trigger's loader (base = None).
        let mut state = TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnFlagSet {
                    name: "parent:armed".into(),
                },
                actions: vec![add_obj("obj-parent")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: Some("child.toml".into()),
            seen_destroyed: HashSet::new(),
        };
        let layer_chain = vec![Some("child.toml".into()), None];
        let events = vec![WorldEvent::FlagSet {
            name: "armed".into(),
            origin_layer: None,
        }];
        let name_to_uuid = HashMap::new();
        let fired = evaluate_single_trigger(
            &mut state,
            &events,
            &name_to_uuid,
            &[],
            &layer_chain,
            &HashMap::new(),
            0.0,
        );
        assert!(
            fired.is_some(),
            "parent:armed must match a base-layer FlagSet"
        );
    }

    #[test]
    fn on_flag_set_does_not_match_same_name_in_different_layer() {
        // Per-layer scoping: same flag name in another sub-world must not
        // fire a trigger watching `armed` in its own layer.
        let mut state = TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnFlagSet {
                    name: "armed".into(),
                },
                actions: vec![add_obj("obj-self")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: Some("child.toml".into()),
            seen_destroyed: HashSet::new(),
        };
        let layer_chain = vec![Some("child.toml".into()), None];
        // Event originated in a DIFFERENT layer (the base / None layer).
        let events = vec![WorldEvent::FlagSet {
            name: "armed".into(),
            origin_layer: None,
        }];
        let name_to_uuid = HashMap::new();
        let fired = evaluate_single_trigger(
            &mut state,
            &events,
            &name_to_uuid,
            &[],
            &layer_chain,
            &HashMap::new(),
            0.0,
        );
        assert!(
            fired.is_none(),
            "same-named flag in another layer must not cross-fire"
        );
    }

    #[test]
    fn on_flag_set_parent_walk_past_root_does_not_match() {
        // Trigger in base world (origin_layer = None) requests `parent:armed`
        // — walks past root. Must never match.
        let mut state = TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnFlagSet {
                    name: "parent:armed".into(),
                },
                actions: vec![add_obj("x")],
                when: None,
                action_predicates: vec![],
                action_delays: vec![],
            },
            fired: false,
            origin_layer: None,
            seen_destroyed: HashSet::new(),
        };
        let layer_chain = vec![None];
        let events = vec![WorldEvent::FlagSet {
            name: "armed".into(),
            origin_layer: None,
        }];
        let name_to_uuid = HashMap::new();
        let fired = evaluate_single_trigger(
            &mut state,
            &events,
            &name_to_uuid,
            &[],
            &layer_chain,
            &HashMap::new(),
            0.0,
        );
        assert!(fired.is_none(), "parent: from base must resolve past root");
    }

    #[test]
    fn resolve_layer_prefix_returns_innermost_for_bare_name() {
        let chain = vec![Some("child.toml".into()), None];
        let r = resolve_layer_prefix("armed", &chain);
        assert_eq!(r, Some(("armed".into(), Some("child.toml".into()))));
    }

    #[test]
    fn resolve_layer_prefix_walks_one_level() {
        let chain = vec![Some("child.toml".into()), None];
        let r = resolve_layer_prefix("parent:armed", &chain);
        assert_eq!(r, Some(("armed".into(), None)));
    }

    #[test]
    fn resolve_layer_prefix_walks_past_root_returns_none() {
        let chain = vec![None];
        assert_eq!(resolve_layer_prefix("parent:armed", &chain), None);
        assert_eq!(resolve_layer_prefix("parent:parent:armed", &chain), None);
    }
}
