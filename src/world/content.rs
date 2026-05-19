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
//   * `ActiveDialogue` / `process_response` — dialogue state machine.
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
}

// ── Runtime state ─────────────────────────────────────────────────────────

/// Runtime state for one trigger within the active world.
#[derive(Clone, Debug)]
pub struct TriggerState {
    pub trigger: Trigger,
    /// Whether this trigger has already fired (single-shot semantics).
    pub fired: bool,
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
    /// ID of the `CommsMessage` that was sent to the Comms inbox for this dialogue.
    pub message_id: String,
    /// The current dialogue node being presented.
    pub current_node: CommsDialogueNode,
}

// ── Evaluators ────────────────────────────────────────────────────────────

/// Evaluate all triggers in `states` against the given `events`.
///
/// Each trigger fires at most once (single-shot). When a trigger fires its
/// `fired` flag is set to `true` and its actions are collected into a
/// `FiredTrigger`.
pub fn evaluate_triggers(
    states: &mut Vec<TriggerState>,
    events: &[WorldEvent],
    name_to_uuid: &HashMap<String, String>,
) -> Vec<FiredTrigger> {
    let mut results = Vec::new();
    for state in states.iter_mut() {
        if state.fired {
            continue;
        }
        let fires = events.iter().any(|event| {
            condition_matches(&state.trigger.condition, event, name_to_uuid)
        });
        if fires {
            state.fired = true;
            results.push(FiredTrigger {
                actions: state.trigger.actions.clone(),
            });
        }
    }
    results
}

/// Evaluate all comms templates in `states` against the given `events`.
///
/// Each template fires at most once (single-shot).
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
            condition_matches(&state.template.trigger, event, name_to_uuid)
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
/// resolve entity names to runtime UUIDs.
fn condition_matches(
    condition: &TriggerCondition,
    event: &WorldEvent,
    name_to_uuid: &HashMap<String, String>,
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

// ── Dialogue state machine ────────────────────────────────────────────────

/// Result returned by `process_response`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessResponseResult {
    /// Actions from the chosen response to execute.
    pub actions: Vec<TriggerAction>,
    /// Follow-up dialogue node to present next, if any.
    pub follow_up: Option<CommsDialogueNode>,
}

/// Process a player's response to an active dialogue.
///
/// Looks up `message_id` in `dialogues`. If found and `response_index` is
/// valid, returns the response's actions and optional follow-up node.
pub fn process_response(
    dialogues: &[ActiveDialogue],
    message_id: &str,
    response_index: usize,
) -> Option<ProcessResponseResult> {
    let dialogue = dialogues.iter().find(|d| d.message_id == message_id)?;
    let response = dialogue.current_node.responses.get(response_index)?;
    Some(ProcessResponseResult {
        actions: response.actions.clone(),
        follow_up: response.follow_up.clone(),
    })
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
            },
            fired: false,
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
            },
            fired: false,
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
            },
            fired: false,
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
            },
            TriggerState {
                trigger: dest_trigger("station", add_obj("obj-s")),
                fired: false,
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

    // ── process_response ──────────────────────────────────────────────────

    #[test]
    fn process_response_executes_response_actions() {
        let dialogues = vec![ActiveDialogue {
            message_id: "msg-1".into(),
            current_node: CommsDialogueNode {
                body: "hi".into(),
                responses: vec![CommsResponse {
                    text: "yes".into(),
                    actions: vec![add_obj("obj-yes")],
                    follow_up: None,
                }],
            },
        }];
        let result = process_response(&dialogues, "msg-1", 0).unwrap();
        assert_eq!(result.actions.len(), 1);
        assert!(result.follow_up.is_none());
    }

    #[test]
    fn process_response_returns_follow_up_when_present() {
        let dialogues = vec![ActiveDialogue {
            message_id: "msg-1".into(),
            current_node: CommsDialogueNode {
                body: "hi".into(),
                responses: vec![CommsResponse {
                    text: "more".into(),
                    actions: vec![],
                    follow_up: Some(CommsDialogueNode {
                        body: "follow up body".into(),
                        responses: vec![],
                    }),
                }],
            },
        }];
        let result = process_response(&dialogues, "msg-1", 0).unwrap();
        assert!(result.follow_up.is_some());
        assert_eq!(result.follow_up.unwrap().body, "follow up body");
    }

    #[test]
    fn process_response_returns_none_for_unknown_message_id() {
        let dialogues: Vec<ActiveDialogue> = vec![];
        assert!(process_response(&dialogues, "ghost-id", 0).is_none());
    }

    #[test]
    fn process_response_returns_none_for_out_of_bounds_index() {
        let dialogues = vec![ActiveDialogue {
            message_id: "msg-1".into(),
            current_node: CommsDialogueNode {
                body: "hi".into(),
                responses: vec![CommsResponse { text: "a".into(), actions: vec![], follow_up: None }],
            },
        }];
        assert!(process_response(&dialogues, "msg-1", 99).is_none());
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
}
