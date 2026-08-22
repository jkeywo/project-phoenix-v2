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
    /// An entity's aggregate hull fraction crossed downward. Both samples are
    /// retained so several authored thresholds can match one large hit.
    HullDroppedBelow {
        uuid: String,
        previous_fraction: f32,
        current_fraction: f32,
    },
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
    /// World-elapsed seconds at which this trigger last fired, or `None` if it
    /// has never fired (issue #751). Used to gate `repeat` re-fires by
    /// `Trigger::cooldown_secs`. Reset to `None` by a `ResetTrigger` action.
    pub last_fired_elapsed: Option<f32>,
}

/// Result of evaluating triggers against a batch of world events.
#[derive(Clone, Debug, PartialEq)]
pub struct FiredTrigger {
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

/// Whether a trigger's cooldown has elapsed and it may fire at
/// `current_elapsed` (issue #751).
///
/// Returns `true` (fire allowed) when the trigger has no cooldown, has never
/// fired, or enough world-elapsed seconds have passed since its last fire.
/// A never-fired trigger (`last_fired_elapsed == None`) is always allowed, so
/// the very first fire is never blocked.
fn cooldown_elapsed(state: &TriggerState, current_elapsed: f32) -> bool {
    match (state.trigger.cooldown_secs, state.last_fired_elapsed) {
        (Some(cd), Some(last)) => current_elapsed - last >= cd,
        _ => true,
    }
}

/// Re-arm every trigger in `states` whose authored `id` equals `id`
/// (issue #751): clear `fired`, the `seen_destroyed` accumulation, and the
/// cooldown clock so the trigger behaves as freshly loaded. Returns the number
/// of trigger states re-armed (0 = unknown id, a no-op).
pub fn reset_triggers_by_id(states: &mut [TriggerState], id: &str) -> usize {
    let mut count = 0;
    for state in states.iter_mut() {
        if state.trigger.id.as_deref() == Some(id) {
            state.fired = false;
            state.seen_destroyed.clear();
            state.last_fired_elapsed = None;
            count += 1;
        }
    }
    count
}

/// Extract the entity name from a `TriggerCondition`, if the variant carries one.
pub fn entity_name_from_condition(condition: &TriggerCondition) -> Option<String> {
    match condition {
        TriggerCondition::OnDestroyed { entity_name }
        | TriggerCondition::OnAttacked { entity_name }
        | TriggerCondition::OnHullBelow { entity_name, .. }
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
        // Once-only triggers (the default) never fire twice. Repeatable
        // triggers fall through and re-evaluate their condition each tick
        // (issue #751).
        if state.fired && !state.trigger.repeat {
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
        // Cooldown gate (issue #751): a repeatable trigger cannot re-fire
        // until at least `cooldown_secs` have elapsed since its last fire.
        // The first fire has `last_fired_elapsed == None`, so the gate never
        // blocks it. Determinism: compares against the injected
        // `current_elapsed`, never a wall clock.
        if !cooldown_elapsed(state, current_elapsed) {
            continue;
        }
        state.fired = true;
        state.last_fired_elapsed = Some(current_elapsed);
        results.push(FiredTrigger {
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
    // Once-only triggers (the default) never fire twice; repeatable triggers
    // re-evaluate each tick (issue #751).
    if state.fired && !state.trigger.repeat {
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
    // Cooldown gate (issue #751) — see `evaluate_triggers_with_flags`.
    if !cooldown_elapsed(state, current_elapsed) {
        return None;
    }
    state.fired = true;
    state.last_fired_elapsed = Some(current_elapsed);
    Some(FiredTrigger {
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
        (
            TriggerCondition::OnHullBelow {
                entity_name,
                threshold,
            },
            WorldEvent::HullDroppedBelow {
                uuid,
                previous_fraction,
                current_fraction,
            },
        ) => {
            previous_fraction >= threshold
                && current_fraction < threshold
                && name_to_uuid
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

// ── Unit Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "content_tests.rs"]
mod tests;
