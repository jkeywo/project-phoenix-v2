// Runtime state and event evaluators for world content.
//
// Pure Rust module — no Bevy. Pure config types (`TriggerCondition`,
// `TriggerAction`, `Trigger`, `CommsTemplate`, `CommsDialogueNode`,
// `CommsResponse`) live in `world::config` and are re-exported here so
// existing imports continue to resolve. This module owns:
//
//   * `WorldEvent` — events triggers / comms templates react to.
//   * `TriggerState` / `CommsTemplateState` — per-trigger fired flag.
//   * `evaluate_triggers` / `evaluate_comms_templates` — single-shot evaluators.
//   * `ActiveDialogue` — dialogue state machine.
//   * `trigger_states_from_world` / `comms_template_states_from_world` —
//     factories that derive runtime states from a parsed `WorldConfig`.
//
// PRD #342: the legacy multi-world layering machinery was deleted in slice 5.
// One world is loaded per session; runtime state is flat.

use std::collections::HashMap;

// Re-export pure config types so legacy import paths continue to resolve.
pub use crate::world::config::{
    CommsDialogueNode, CommsResponse, CommsTemplate, Trigger, TriggerAction, TriggerCondition,
};
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
    FlagSet { name: String, origin_layer: Option<String> },
    /// A world flag transitioned true→false. See `FlagSet`.
    FlagCleared { name: String, origin_layer: Option<String> },
    /// The containing world finished loading (base-world `Startup` or
    /// sub-world `LoadWorld`). Emitted once per load cycle.
    WorldLoaded,
    /// The player ship entered a region (by UUID).
    EnteredRegion { uuid: String },
    /// The player ship exited a region (by UUID), either by crossing the
    /// boundary or because the region entity was despawned while the
    /// ship was inside.
    ExitedRegion { uuid: String },
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
}

/// Runtime state for one comms template — tracks whether it has already fired.
#[derive(Clone, Debug)]
pub struct CommsTemplateState {
    pub template: CommsTemplate,
    pub fired: bool,
}

/// Result of evaluating triggers against a batch of world events.
#[derive(Clone, Debug, PartialEq)]
pub struct FiredTrigger {
    pub actions: Vec<TriggerAction>,
    /// Origin sub-world layer path (or `None` for base-world triggers).
    /// Used by `spawn_entity` action dispatch (issue #417) to attach the
    /// new entity to the right `WorldLayerMap` entry.
    pub origin_layer: Option<String>,
}

/// A comms template that fired in response to world events.
#[derive(Clone, Debug, PartialEq)]
pub struct FiredCommsTemplate {
    /// The sender entity name from the template.
    pub from: String,
    /// The root dialogue node to inject into the inbox.
    pub node: CommsDialogueNode,
}

/// Runtime state for one active dialogue conversation.
#[derive(Clone, Debug)]
pub struct ActiveDialogue {
    /// The current dialogue node being presented.
    pub current_node: CommsDialogueNode,
    /// Thread identifier shared by all messages in this dialogue tree.
    /// Set when the first message is injected; follow-ups inherit the same id.
    pub thread_id: String,
}

// ── Evaluators ────────────────────────────────────────────────────────────

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
    evaluate_triggers_with_flags(states, events, name_to_uuid, &[])
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
) -> Vec<FiredTrigger> {
    let mut results = Vec::new();
    for state in states.iter_mut() {
        if state.fired {
            continue;
        }
        // Default layer chain = just the trigger's own layer (no parents
        // known to this entry point). `OnFlagSet`/`OnFlagCleared` with a
        // `parent:` prefix on the condition name will therefore resolve
        // past-root and never match — matches the previous behaviour
        // before fix 1.
        let layer_chain: [Option<String>; 1] = [state.origin_layer.clone()];
        let fires = events.iter().any(|event| {
            condition_matches(&state.trigger.condition, event, name_to_uuid, &layer_chain)
        });
        if !fires {
            continue;
        }
        if let Some(pred) = &state.trigger.when {
            if !pred.evaluate(flag_chain) {
                continue;
            }
        }
        state.fired = true;
        results.push(FiredTrigger {
            actions: state.trigger.actions.clone(),
            origin_layer: state.origin_layer.clone(),
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
) -> Option<FiredTrigger> {
    if state.fired {
        return None;
    }
    let fires = events.iter().any(|event| {
        condition_matches(&state.trigger.condition, event, name_to_uuid, layer_chain)
    });
    if !fires {
        return None;
    }
    if let Some(pred) = &state.trigger.when {
        if !pred.evaluate(flag_chain) {
            return None;
        }
    }
    state.fired = true;
    Some(FiredTrigger {
        actions: state.trigger.actions.clone(),
        origin_layer: state.origin_layer.clone(),
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

/// Evaluate all comms templates in `states` against the given `events`.
///
/// Each template fires at most once (single-shot).
#[allow(clippy::ptr_arg)]
pub fn evaluate_comms_templates(
    states: &mut Vec<CommsTemplateState>,
    events: &[WorldEvent],
    name_to_uuid: &HashMap<String, String>,
) -> Vec<FiredCommsTemplate> {
    let mut results = Vec::new();
    for state in states.iter_mut() {
        if state.fired {
            continue;
        }
        let fires = events.iter().any(|event| {
            // Comms templates don't currently support `parent:` flag
            // conditions; pass a single-element base-only chain so any
            // `parent:` prefix in an OnFlagSet condition resolves past
            // root and never matches (back-compat).
            condition_matches(&state.template.trigger, event, name_to_uuid, &[None])
        });
        if fires {
            state.fired = true;
            results.push(FiredCommsTemplate {
                from: state.template.from.clone(),
                node: state.template.node.clone(),
            });
        }
    }
    results
}

/// Returns true if `condition` matches `event`, using `name_to_uuid` to
/// resolve entity names to runtime UUIDs and `layer_chain` to resolve
/// `parent:` prefixes on `OnFlagSet` / `OnFlagCleared` condition names
/// (innermost layer first; `None` = base world).
fn condition_matches(
    condition: &TriggerCondition,
    event: &WorldEvent,
    name_to_uuid: &HashMap<String, String>,
    layer_chain: &[Option<String>],
) -> bool {
    match (condition, event) {
        (TriggerCondition::OnDestroyed { entity_name }, WorldEvent::Destroyed { uuid }) => {
            name_to_uuid.get(entity_name).map(|u| u == uuid).unwrap_or(false)
        }
        (TriggerCondition::OnAttacked { entity_name }, WorldEvent::Attacked { uuid, .. }) => {
            name_to_uuid.get(entity_name).map(|u| u == uuid).unwrap_or(false)
        }
        (TriggerCondition::OnTimer { after_secs }, WorldEvent::TimerElapsed { elapsed_secs }) => {
            elapsed_secs >= after_secs
        }
        (TriggerCondition::OnHailed { entity_name }, WorldEvent::Hailed { target_uuid }) => {
            name_to_uuid.get(entity_name).map(|u| u == target_uuid).unwrap_or(false)
        }
        (
            TriggerCondition::OnFlagSet { name },
            WorldEvent::FlagSet { name: ev_name, origin_layer: ev_layer },
        ) => match resolve_layer_prefix(name, layer_chain) {
            Some((stripped, target_layer)) => &stripped == ev_name && &target_layer == ev_layer,
            None => false,
        },
        (
            TriggerCondition::OnFlagCleared { name },
            WorldEvent::FlagCleared { name: ev_name, origin_layer: ev_layer },
        ) => match resolve_layer_prefix(name, layer_chain) {
            Some((stripped, target_layer)) => &stripped == ev_name && &target_layer == ev_layer,
            None => false,
        },
        (TriggerCondition::OnWorldLoaded, WorldEvent::WorldLoaded) => true,
        (TriggerCondition::OnEnteredRegion { entity_name }, WorldEvent::EnteredRegion { uuid }) => {
            name_to_uuid.get(entity_name).map(|u| u == uuid).unwrap_or(false)
        }
        (TriggerCondition::OnExitedRegion { entity_name }, WorldEvent::ExitedRegion { uuid }) => {
            name_to_uuid.get(entity_name).map(|u| u == uuid).unwrap_or(false)
        }
        _ => false,
    }
}

// ── Factories from WorldConfig ────────────────────────────────────────────

/// Create a `Vec<TriggerState>` from a parsed `WorldConfig` (PRD #341).
///
/// All triggers start unfired.
pub fn trigger_states_from_world(
    world: &crate::world::config::WorldConfig,
) -> Vec<TriggerState> {
    world
        .triggers
        .iter()
        .map(|t| TriggerState {
            trigger: t.clone(),
            fired: false,
            origin_layer: None,
        })
        .collect()
}

/// Create a `Vec<CommsTemplateState>` from a parsed `WorldConfig` (PRD #341).
pub fn comms_template_states_from_world(
    world: &crate::world::config::WorldConfig,
) -> Vec<CommsTemplateState> {
    world
        .comms
        .iter()
        .map(|t| CommsTemplateState {
            template: t.clone(),
            fired: false,
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
            condition: TriggerCondition::OnDestroyed { entity_name: name.into() },
            actions: vec![action],
            when: None,
        }
    }

    fn add_obj(id: &str) -> TriggerAction {
        TriggerAction::AddObjective {
            id: id.into(),
            text: format!("Objective {id}"),
            mandatory: false,
        }
    }

    // ── evaluate_triggers ─────────────────────────────────────────────────

    #[test]
    fn on_destroyed_fires_when_entity_destroyed() {
        let mut states = vec![TriggerState {
            trigger: dest_trigger("raider", add_obj("obj-1")),
            fired: false,
            origin_layer: None,
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-1".into());
        let events = vec![WorldEvent::Destroyed { uuid: "uuid-1".into() }];
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
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-1".into());
        name_to_uuid.insert("station".into(), "uuid-2".into());
        let events = vec![WorldEvent::Destroyed { uuid: "uuid-2".into() }];
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
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-1".into());
        let events = vec![WorldEvent::Destroyed { uuid: "uuid-1".into() }];
        let fired1 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        let fired2 = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired1.len(), 1);
        assert!(fired2.is_empty());
    }

    #[test]
    fn on_timer_fires_when_elapsed_exceeds_threshold() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnTimer { after_secs: 30.0 },
                actions: vec![add_obj("obj-timer")],
                when: None,
            },
            fired: false,
            origin_layer: None,
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
                condition: TriggerCondition::OnAttacked { entity_name: "raider".into() },
                actions: vec![add_obj("obj-atk")],
                when: None,
            },
            fired: false,
            origin_layer: None,
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
                condition: TriggerCondition::OnHailed { entity_name: "starbase".into() },
                actions: vec![add_obj("obj-hail")],
                when: None,
            },
            fired: false,
            origin_layer: None,
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("starbase".into(), "uuid-sb".into());
        let events = vec![WorldEvent::Hailed { target_uuid: "uuid-sb".into() }];
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
            },
            TriggerState {
                trigger: dest_trigger("station", add_obj("obj-s")),
                fired: false,
                origin_layer: None,
            },
        ];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-r".into());
        name_to_uuid.insert("station".into(), "uuid-s".into());
        let events = vec![WorldEvent::Destroyed { uuid: "uuid-r".into() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert!(states[0].fired);
        assert!(!states[1].fired);
    }

    #[test]
    fn on_destroyed_does_not_fire_if_entity_name_unknown() {
        let mut states = vec![TriggerState {
            trigger: dest_trigger("ghost", add_obj("obj-ghost")),
            fired: false,
            origin_layer: None,
        }];
        let name_to_uuid = HashMap::new();
        let events = vec![WorldEvent::Destroyed { uuid: "uuid-x".into() }];
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

    #[test]
    fn comms_template_states_from_world_creates_unfired_states_for_every_template() {
        let mut world = WorldConfig::default();
        world.comms.push(CommsTemplate {
            from: "starbase".into(),
            trigger: TriggerCondition::OnHailed { entity_name: "starbase".into() },
            node: CommsDialogueNode { body: "hello".into(), responses: vec![] },
        });
        let states = comms_template_states_from_world(&world);
        assert_eq!(states.len(), 1);
        assert!(!states[0].fired);
    }

    // ── evaluate_comms_templates ──────────────────────────────────────────

    #[test]
    fn evaluate_comms_templates_fires_on_attacked() {
        let mut states = vec![CommsTemplateState {
            template: CommsTemplate {
                from: "raider".into(),
                trigger: TriggerCondition::OnAttacked { entity_name: "raider".into() },
                node: CommsDialogueNode { body: "MAYDAY".into(), responses: vec![] },
            },
            fired: false,
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-r".into());
        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-r".into(),
            attacker_uuid: "uuid-p".into(),
        }];
        let fired = evaluate_comms_templates(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].from, "raider");
    }

    #[test]
    fn evaluate_comms_templates_fires_at_most_once() {
        let mut states = vec![CommsTemplateState {
            template: CommsTemplate {
                from: "raider".into(),
                trigger: TriggerCondition::OnAttacked { entity_name: "raider".into() },
                node: CommsDialogueNode { body: "MAYDAY".into(), responses: vec![] },
            },
            fired: false,
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-r".into());
        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-r".into(),
            attacker_uuid: "uuid-p".into(),
        }];
        let first = evaluate_comms_templates(&mut states, &events, &name_to_uuid);
        let second = evaluate_comms_templates(&mut states, &events, &name_to_uuid);
        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
    }

    #[test]
    fn evaluate_comms_templates_does_not_fire_for_unrelated_entity() {
        let mut states = vec![CommsTemplateState {
            template: CommsTemplate {
                from: "raider".into(),
                trigger: TriggerCondition::OnAttacked { entity_name: "raider".into() },
                node: CommsDialogueNode { body: "MAYDAY".into(), responses: vec![] },
            },
            fired: false,
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider".into(), "uuid-r".into());
        name_to_uuid.insert("station".into(), "uuid-s".into());
        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-s".into(),
            attacker_uuid: "uuid-p".into(),
        }];
        let fired = evaluate_comms_templates(&mut states, &events, &name_to_uuid);
        assert!(fired.is_empty());
    }

    // ── Shipped-world integration ─────────────────────────────────────────

    #[test]
    fn default_world_on_attacked_fires_comms_template() {
        let toml = include_str!("../../assets/worlds/default.toml");
        let world = crate::world::config::parse_world(toml).expect("default.toml must parse");
        let mut states = comms_template_states_from_world(&world);
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider_alpha".into(), "uuid-r".into());
        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-r".into(),
            attacker_uuid: "uuid-p".into(),
        }];
        let fired = evaluate_comms_templates(&mut states, &events, &name_to_uuid);
        assert!(!fired.is_empty(), "raider_alpha on_attacked comms must fire");
        assert!(fired.iter().any(|f| f.from == "raider_alpha"));
    }

    #[test]
    fn patrol_world_on_destroyed_trigger_fires_add_objective() {
        let toml = include_str!("../../assets/worlds/patrol.toml");
        let world = crate::world::config::parse_world(toml).expect("patrol.toml must parse");
        let mut states = trigger_states_from_world(&world);
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("raider_alpha".into(), "uuid-r".into());
        let events = vec![WorldEvent::Destroyed { uuid: "uuid-r".into() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert!(fired[0].actions.iter().any(|a| matches!(a, TriggerAction::AddObjective { .. })));
    }

    // ── on_world_loaded (issue #415) ──────────────────────────────────────

    #[test]
    fn on_world_loaded_matches_world_loaded_event() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnWorldLoaded,
                actions: vec![add_obj("obj-loaded")],
                when: None,
            },
            fired: false,
            origin_layer: None,
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
            },
            fired: false,
            origin_layer: None,
        }];
        let name_to_uuid = HashMap::new();
        let events = vec![
            WorldEvent::Destroyed { uuid: "x".into() },
            WorldEvent::TimerElapsed { elapsed_secs: 5.0 },
            WorldEvent::FlagSet { name: "f".into(), origin_layer: None },
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
            },
            fired: false,
            origin_layer: None,
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
                condition: TriggerCondition::OnEnteredRegion { entity_name: "nebula".into() },
                actions: vec![add_obj("obj-nebula")],
                when: None,
            },
            fired: false,
            origin_layer: None,
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
        let events = vec![WorldEvent::EnteredRegion { uuid: "uuid-nebula".into() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
        assert!(states[0].fired);
    }

    #[test]
    fn on_entered_region_does_not_match_different_uuid() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnEnteredRegion { entity_name: "nebula".into() },
                actions: vec![add_obj("obj-nebula")],
                when: None,
            },
            fired: false,
            origin_layer: None,
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
        let events = vec![WorldEvent::EnteredRegion { uuid: "uuid-other".into() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert!(fired.is_empty());
        assert!(!states[0].fired);
    }

    #[test]
    fn on_entered_region_does_not_match_exited_region_event() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnEnteredRegion { entity_name: "nebula".into() },
                actions: vec![add_obj("x")],
                when: None,
            },
            fired: false,
            origin_layer: None,
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
        let events = vec![WorldEvent::ExitedRegion { uuid: "uuid-nebula".into() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert!(fired.is_empty());
    }

    #[test]
    fn on_exited_region_matches_exited_region_event_with_resolved_uuid() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnExitedRegion { entity_name: "nebula".into() },
                actions: vec![add_obj("obj-left-nebula")],
                when: None,
            },
            fired: false,
            origin_layer: None,
        }];
        let mut name_to_uuid = HashMap::new();
        name_to_uuid.insert("nebula".into(), "uuid-nebula".into());
        let events = vec![WorldEvent::ExitedRegion { uuid: "uuid-nebula".into() }];
        let fired = evaluate_triggers(&mut states, &events, &name_to_uuid);
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn on_entered_region_does_not_match_unresolved_name() {
        let mut states = vec![TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnEnteredRegion { entity_name: "unknown".into() },
                actions: vec![add_obj("x")],
                when: None,
            },
            fired: false,
            origin_layer: None,
        }];
        let name_to_uuid = HashMap::new(); // empty — name does not resolve
        let events = vec![WorldEvent::EnteredRegion { uuid: "any-uuid".into() }];
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
                condition: TriggerCondition::OnFlagSet { name: "parent:armed".into() },
                actions: vec![add_obj("obj-parent")],
                when: None,
            },
            fired: false,
            origin_layer: Some("child.toml".into()),
        };
        let layer_chain = vec![Some("child.toml".into()), None];
        let events = vec![WorldEvent::FlagSet {
            name: "armed".into(),
            origin_layer: None,
        }];
        let name_to_uuid = HashMap::new();
        let fired = evaluate_single_trigger(
            &mut state, &events, &name_to_uuid, &[], &layer_chain,
        );
        assert!(fired.is_some(), "parent:armed must match a base-layer FlagSet");
    }

    #[test]
    fn on_flag_set_does_not_match_same_name_in_different_layer() {
        // Per-layer scoping: same flag name in another sub-world must not
        // fire a trigger watching `armed` in its own layer.
        let mut state = TriggerState {
            trigger: Trigger {
                condition: TriggerCondition::OnFlagSet { name: "armed".into() },
                actions: vec![add_obj("obj-self")],
                when: None,
            },
            fired: false,
            origin_layer: Some("child.toml".into()),
        };
        let layer_chain = vec![Some("child.toml".into()), None];
        // Event originated in a DIFFERENT layer (the base / None layer).
        let events = vec![WorldEvent::FlagSet {
            name: "armed".into(),
            origin_layer: None,
        }];
        let name_to_uuid = HashMap::new();
        let fired = evaluate_single_trigger(
            &mut state, &events, &name_to_uuid, &[], &layer_chain,
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
                condition: TriggerCondition::OnFlagSet { name: "parent:armed".into() },
                actions: vec![add_obj("x")],
                when: None,
            },
            fired: false,
            origin_layer: None,
        };
        let layer_chain = vec![None];
        let events = vec![WorldEvent::FlagSet {
            name: "armed".into(),
            origin_layer: None,
        }];
        let name_to_uuid = HashMap::new();
        let fired = evaluate_single_trigger(
            &mut state, &events, &name_to_uuid, &[], &layer_chain,
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
