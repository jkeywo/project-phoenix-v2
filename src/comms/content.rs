// Pure comms evaluators and runtime state.
//
// Pure Rust module — no Bevy. This is the evaluation half of the Comms
// concept (issue #816), mirroring the world-trigger pattern
// (`world/dispatch.rs` / `world/content.rs` / `world/flags.rs`): resolution
// happens here, the Bevy applier (`comms::server`) stays dumb.
//
// Pure config types (`CommsTemplate`, `CommsDialogueNode`, `CommsResponse`)
// live in `world::config` — they are shared TOML vocabulary with
// `TriggerCondition` / `TriggerAction` — and are re-exported here so comms
// consumers get the whole comms surface from one path. This module owns:
//
//   * `CommsTemplateState` — per-template fired flag.
//   * `FiredCommsTemplate` — evaluation result for one fired template.
//   * `ActiveDialogue` — dialogue state machine entry.
//   * `PendingFollowUp` — a queued follow-up message awaiting its trigger.
//   * `evaluate_comms_templates` — single-shot template evaluator.
//   * `follow_up_trigger_holds` — pure follow-up trigger evaluator.
//   * `comms_template_states_from_world` — factory from a parsed `WorldConfig`.

use std::collections::{HashMap, HashSet};

// Re-export the comms TOML vocabulary so comms consumers import template
// types alongside the runtime state types defined here.
use crate::messages::CommsResponseView;
use crate::world::config::TriggerCondition;
pub use crate::world::config::{CommsDialogueNode, CommsResponse, CommsTemplate};
use crate::world::content::{condition_matches, WorldEvent};
use crate::world::flags::FlagStore;

/// Project a node's authored responses onto the wire `CommsResponseView`
/// vector (issue #761). Each view carries the authored `text`, the authored
/// `important` flag, and the current `available` (sender-in-range) flag. All
/// responses in a message share the same availability — a response is
/// "unavailable" exactly when its message's sender is out of comms range, the
/// same authoritative reachability that stamps `CommsMessage::sender_in_range`.
pub fn response_views(responses: &[CommsResponse], available: bool) -> Vec<CommsResponseView> {
    responses
        .iter()
        .map(|r| CommsResponseView {
            text: r.text.clone(),
            important: r.important,
            available,
        })
        .collect()
}

// ── Runtime state ─────────────────────────────────────────────────────────

/// Runtime state for one comms template — tracks whether it has already fired.
#[derive(Clone, Debug)]
pub struct CommsTemplateState {
    pub template: CommsTemplate,
    pub fired: bool,
}

/// A comms template that fired in response to world events.
#[derive(Clone, Debug, PartialEq)]
pub struct FiredCommsTemplate {
    /// The sender entity **reference id** from the template (resolved to the
    /// sender UUID; used for hailing/range/contact lookup).
    pub from: String,
    /// Optional player-facing sender display text, independent of `from`
    /// (issue #751). `None` falls back to `from` at injection time.
    pub display_name: Option<String>,
    /// The root dialogue node to inject into the inbox.
    pub node: CommsDialogueNode,
    /// Thread_id from the template, if set. When absent a UUID is generated
    /// at injection time.
    pub thread_id: Option<String>,
    /// When true, the injected `CommsMessage` should be flagged as urgent.
    pub urgent: bool,
    /// Optional chained follow-up node that should be scheduled at inject
    /// time. The server queues this onto `pending_follow_ups` so the
    /// chained message arrives without any player response click required
    /// (one-way "Stand by..." broadcasts). If the chained node carries a
    /// `trigger`, the follow-up waits for that trigger to fire; otherwise
    /// it fires on the next tick.
    pub root_follow_up: Option<CommsDialogueNode>,
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

/// A comms message that has been queued and is waiting to be injected into
/// the inbox.
///
/// A follow-up sits in the queue until its trigger condition is met. If the
/// follow-up has no trigger, it fires on the next tick. If the follow-up
/// has a trigger, it fires when:
///   - the trigger condition is observed in a `WorldEvent` after queueing, OR
///   - the trigger condition is "already-true" at evaluation time (e.g.
///     the ship is currently inside the named region for `OnEnteredRegion`;
///     the flag is already set for `OnFlagSet`; the world has already
///     loaded for `OnWorldLoaded`), OR
///   - for `OnTimer`, the `elapsed_secs` field reaches `after_secs`. The
///     `elapsed_secs` clock is queue-relative, NOT world-relative, so a
///     3-second response follow-up fires three seconds after the player
///     picks the response.
#[derive(Clone, Debug)]
pub struct PendingFollowUp {
    /// The dialogue node to inject once the trigger condition is met.
    pub node: CommsDialogueNode,
    /// UUID of the entity sending this message.
    pub sender_uuid: String,
    /// Display name of the sender (already resolved to the per-node override
    /// or the parent template's `from`).
    pub sender_name: String,
    /// Shared thread identifier for this conversation.
    pub thread_id: String,
    /// Seconds elapsed since this follow-up was queued. Used for
    /// `OnTimer` trigger evaluation (queue-relative, not world-relative).
    pub elapsed_secs: f32,
    /// The id of the `...` placeholder message currently shown in the inbox,
    /// if the follow-up is an in-thread response follow-up. Chained roots
    /// stay silent until the real message is ready.
    pub placeholder_id: Option<String>,
    /// Whether the real message should be flagged as urgent.
    pub urgent: bool,
}

// ── Evaluators ────────────────────────────────────────────────────────────

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
                display_name: state.template.display_name.clone(),
                node: state.template.node.clone(),
                thread_id: state.template.thread_id.clone(),
                urgent: state.template.urgent,
                root_follow_up: state.template.root_follow_up.clone(),
            });
        }
    }
    results
}

/// Pure evaluator: returns true when a follow-up trigger condition is met
/// for the given snapshot of world state and observed events.
///
/// State-based conditions check current world state and fire immediately
/// when "already true" — `OnEnteredRegion` fires while the ship is inside
/// the region, `OnFlagSet` fires while the flag holds a non-zero counter,
/// `OnDestroyed` fires once the named entity's UUID is absent from the
/// live ECS set, `OnWorldLoaded` always fires (the world is, by
/// construction, loaded once a follow-up is queued).
///
/// Event-based conditions (`OnAttacked`, `OnHailed`) require a matching
/// `WorldEvent` in `events`. `OnTimer` is queue-relative: it compares
/// `elapsed_secs` against the configured `after_secs`.
///
/// A `None` trigger means "fire immediately" — the follow-up arrives on
/// the next tick after being queued.
#[allow(clippy::too_many_arguments)]
pub fn follow_up_trigger_holds(
    trigger: Option<&TriggerCondition>,
    elapsed_secs: f32,
    events: &[WorldEvent],
    name_to_uuid: &HashMap<String, String>,
    flags: &FlagStore,
    inside_region_uuids: &HashSet<String>,
    live_uuids: &HashSet<String>,
    entity_groups: &HashMap<String, HashSet<String>>,
) -> bool {
    let Some(condition) = trigger else {
        return true;
    };
    match condition {
        TriggerCondition::OnTimer { after_secs } => elapsed_secs >= *after_secs,
        TriggerCondition::OnWorldLoaded => true,
        TriggerCondition::OnEnteredRegion { entity_name } => name_to_uuid
            .get(entity_name)
            .map(|u| inside_region_uuids.contains(u))
            .unwrap_or(false),
        TriggerCondition::OnExitedRegion { entity_name } => name_to_uuid
            .get(entity_name)
            .map(|u| !inside_region_uuids.contains(u))
            .unwrap_or(false),
        TriggerCondition::OnFlagSet { name } => {
            // Follow-ups don't currently participate in sub-world layer
            // chains; strip any `parent:` prefix to keep the predicate
            // resolving against the base store. (Matches the comms-template
            // evaluator, which passes a base-only chain.)
            let key = strip_parent_prefix(name);
            flags.flag(key)
        }
        TriggerCondition::OnFlagCleared { name } => {
            let key = strip_parent_prefix(name);
            !flags.flag(key)
        }
        TriggerCondition::OnDestroyed { entity_name } => {
            // "Already destroyed" — the entity was registered in
            // `name_to_uuid` but its UUID is no longer in the live ECS set.
            // Also fires on a fresh `Destroyed` event observed this tick.
            name_to_uuid
                .get(entity_name)
                .map(|u| {
                    !live_uuids.contains(u)
                        || events
                            .iter()
                            .any(|e| matches!(e, WorldEvent::Destroyed { uuid } if uuid == u))
                })
                .unwrap_or(false)
        }
        TriggerCondition::OnAllDestroyed { group, after_secs } => {
            if elapsed_secs < *after_secs {
                return false;
            }
            let members: HashSet<String> = entity_groups
                .get(group)
                .cloned()
                .unwrap_or_else(|| std::iter::once(group.clone()).collect());
            if members.is_empty() {
                return false;
            }
            members.iter().all(|name| {
                name_to_uuid
                    .get(name)
                    .map(|u| !live_uuids.contains(u))
                    .unwrap_or(false)
            })
        }
        TriggerCondition::OnAttacked { entity_name } => name_to_uuid
            .get(entity_name)
            .map(|u| {
                events
                    .iter()
                    .any(|e| matches!(e, WorldEvent::Attacked { uuid, .. } if uuid == u))
            })
            .unwrap_or(false),
        TriggerCondition::OnHullBelow {
            entity_name,
            threshold,
        } => name_to_uuid
            .get(entity_name)
            .map(|u| {
                events.iter().any(|e| matches!(
                    e,
                    WorldEvent::HullDroppedBelow {
                        uuid,
                        previous_fraction,
                        current_fraction,
                    } if uuid == u && previous_fraction >= threshold && current_fraction < threshold
                ))
            })
            .unwrap_or(false),
        TriggerCondition::OnHailed { entity_name } => name_to_uuid
            .get(entity_name)
            .map(|u| {
                events
                    .iter()
                    .any(|e| matches!(e, WorldEvent::Hailed { target_uuid } if target_uuid == u))
            })
            .unwrap_or(false),
        // Event-based, like `OnAttacked` / `OnHailed`: there is no
        // "already arrived" state to inspect, so the follow-up waits for a
        // fresh arrival. An omitted `waypoint` matches any waypoint.
        TriggerCondition::OnWaypointReached {
            entity_name,
            waypoint,
        } => name_to_uuid
            .get(entity_name)
            .map(|u| {
                events.iter().any(|e| match e {
                    WorldEvent::WaypointReached {
                        uuid,
                        waypoint: ev_waypoint,
                    } => uuid == u && waypoint.as_ref().map(|w| w == ev_waypoint).unwrap_or(true),
                    _ => false,
                })
            })
            .unwrap_or(false),
    }
}

fn strip_parent_prefix(name: &str) -> &str {
    let mut rest = name;
    while let Some(s) = rest.strip_prefix("parent:") {
        rest = s;
    }
    rest
}

// ── Factories from WorldConfig ────────────────────────────────────────────

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

    // ── comms_template_states_from_world ──────────────────────────────────

    #[test]
    fn comms_template_states_from_world_creates_unfired_states_for_every_template() {
        let mut world = WorldConfig::default();
        world.comms.push(CommsTemplate {
            from: "starbase".into(),
            trigger: TriggerCondition::OnHailed {
                entity_name: "starbase".into(),
            },
            node: CommsDialogueNode {
                body: "hello".into(),
                responses: vec![],
                speaker: None,
                trigger: None,
            },
            thread_id: None,
            urgent: false,
            root_follow_up: None,
            display_name: None,
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
                trigger: TriggerCondition::OnAttacked {
                    entity_name: "raider".into(),
                },
                node: CommsDialogueNode {
                    body: "MAYDAY".into(),
                    responses: vec![],
                    speaker: None,
                    trigger: None,
                },
                thread_id: None,
                urgent: false,
                root_follow_up: None,
                display_name: None,
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
                trigger: TriggerCondition::OnAttacked {
                    entity_name: "raider".into(),
                },
                node: CommsDialogueNode {
                    body: "MAYDAY".into(),
                    responses: vec![],
                    speaker: None,
                    trigger: None,
                },
                thread_id: None,
                urgent: false,
                root_follow_up: None,
                display_name: None,
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
    fn hull_threshold_templates_are_strict_single_shot_and_share_a_large_hit() {
        let mut states = [0.75, 0.50, 0.10]
            .into_iter()
            .map(|threshold| CommsTemplateState {
                template: CommsTemplate {
                    from: "station".into(),
                    trigger: TriggerCondition::OnHullBelow {
                        entity_name: "station".into(),
                        threshold,
                    },
                    node: CommsDialogueNode {
                        body: format!("{threshold}"),
                        responses: vec![],
                        speaker: None,
                        trigger: None,
                    },
                    thread_id: None,
                    urgent: true,
                    root_follow_up: None,
                    display_name: None,
                },
                fired: false,
            })
            .collect::<Vec<_>>();
        let name_to_uuid = HashMap::from([("station".into(), "station-uuid".into())]);

        let exactly_at_threshold = vec![WorldEvent::HullDroppedBelow {
            uuid: "station-uuid".into(),
            previous_fraction: 1.0,
            current_fraction: 0.75,
        }];
        assert!(
            evaluate_comms_templates(&mut states, &exactly_at_threshold, &name_to_uuid).is_empty()
        );

        let large_hit = vec![WorldEvent::HullDroppedBelow {
            uuid: "station-uuid".into(),
            previous_fraction: 0.75,
            current_fraction: 0.05,
        }];
        let fired = evaluate_comms_templates(&mut states, &large_hit, &name_to_uuid);
        assert_eq!(
            fired
                .iter()
                .map(|message| message.node.body.as_str())
                .collect::<Vec<_>>(),
            vec!["0.75", "0.5", "0.1"]
        );

        let repaired_then_hit_again = vec![WorldEvent::HullDroppedBelow {
            uuid: "station-uuid".into(),
            previous_fraction: 1.0,
            current_fraction: 0.01,
        }];
        assert!(
            evaluate_comms_templates(&mut states, &repaired_then_hit_again, &name_to_uuid)
                .is_empty()
        );
    }

    #[test]
    fn evaluate_comms_templates_does_not_fire_for_unrelated_entity() {
        let mut states = vec![CommsTemplateState {
            template: CommsTemplate {
                from: "raider".into(),
                trigger: TriggerCondition::OnAttacked {
                    entity_name: "raider".into(),
                },
                node: CommsDialogueNode {
                    body: "MAYDAY".into(),
                    responses: vec![],
                    speaker: None,
                    trigger: None,
                },
                thread_id: None,
                urgent: false,
                root_follow_up: None,
                display_name: None,
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
        name_to_uuid.insert("world.entity.raider_alpha.name".into(), "uuid-r".into());
        let events = vec![WorldEvent::Attacked {
            uuid: "uuid-r".into(),
            attacker_uuid: "uuid-p".into(),
        }];
        let fired = evaluate_comms_templates(&mut states, &events, &name_to_uuid);
        assert!(
            !fired.is_empty(),
            "raider_alpha on_attacked comms must fire"
        );
        assert!(fired
            .iter()
            .any(|f| f.from == "world.entity.raider_alpha.name"));
    }

    // ── follow_up_trigger_holds (pure evaluator) ──────────────────────────

    fn name_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(n, u)| (n.to_string(), u.to_string()))
            .collect()
    }

    #[test]
    fn follow_up_trigger_holds_fires_immediately_when_trigger_is_none() {
        let n2u = HashMap::new();
        let flags = FlagStore::new();
        assert!(follow_up_trigger_holds(
            None,
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_world_loaded_always_fires() {
        let n2u = HashMap::new();
        let flags = FlagStore::new();
        assert!(follow_up_trigger_holds(
            Some(&TriggerCondition::OnWorldLoaded),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_timer_uses_elapsed_secs_not_world_events() {
        let n2u = HashMap::new();
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnTimer { after_secs: 3.0 };

        // Below threshold: does not fire.
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            2.9,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
        // At/above threshold: fires.
        assert!(follow_up_trigger_holds(
            Some(&cond),
            3.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
        assert!(follow_up_trigger_holds(
            Some(&cond),
            10.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_entered_region_fires_when_ship_inside_region() {
        let n2u = name_map(&[("Axiom Dock", "axiom-dock-uuid")]);
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnEnteredRegion {
            entity_name: "Axiom Dock".into(),
        };

        // Ship not inside: does NOT fire.
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
        // Ship inside: fires.
        let mut inside = HashSet::new();
        inside.insert("axiom-dock-uuid".to_string());
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &inside,
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_entered_region_unknown_entity_does_not_fire() {
        // Even if the ship is inside some region, an unmapped entity name
        // never resolves and the trigger never fires.
        let n2u = HashMap::new();
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnEnteredRegion {
            entity_name: "Nowhere".into(),
        };
        let mut inside = HashSet::new();
        inside.insert("some-other-uuid".to_string());
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &inside,
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_exited_region_fires_when_ship_outside() {
        // "Already-true" semantics: a follow-up that needs the player to
        // be OUTSIDE the region fires immediately if they are already
        // outside.
        let n2u = name_map(&[("Trap Zone", "trap-uuid")]);
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnExitedRegion {
            entity_name: "Trap Zone".into(),
        };

        // Ship inside: does NOT fire.
        let mut inside = HashSet::new();
        inside.insert("trap-uuid".to_string());
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &inside,
            &HashSet::new(),
            &HashMap::new(),
        ));
        // Ship outside: fires.
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_flag_set_fires_when_flag_already_set() {
        let n2u = HashMap::new();
        let mut flags = FlagStore::new();
        flags.set_flag("aphelion_armed");
        let cond = TriggerCondition::OnFlagSet {
            name: "aphelion_armed".into(),
        };
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_flag_set_does_not_fire_when_flag_unset() {
        let n2u = HashMap::new();
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnFlagSet {
            name: "aphelion_armed".into(),
        };
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_flag_set_strips_parent_prefix() {
        // Follow-ups don't participate in sub-world layer chains; the
        // evaluator strips any `parent:` prefix so the predicate resolves
        // against the base flag store.
        let n2u = HashMap::new();
        let mut flags = FlagStore::new();
        flags.set_flag("aphelion_armed");
        let cond = TriggerCondition::OnFlagSet {
            name: "parent:aphelion_armed".into(),
        };
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_flag_cleared_fires_when_flag_already_unset() {
        let n2u = HashMap::new();
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnFlagCleared {
            name: "shields_offline".into(),
        };
        // Unset flag is treated as "cleared" fires immediately.
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_destroyed_fires_when_entity_already_destroyed() {
        let n2u = name_map(&[("Ironveil", "ironveil-uuid")]);
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnDestroyed {
            entity_name: "Ironveil".into(),
        };

        // Ironveil's UUID is registered but NOT in the live set fires.
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_destroyed_does_not_fire_when_entity_alive() {
        let n2u = name_map(&[("Ironveil", "ironveil-uuid")]);
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnDestroyed {
            entity_name: "Ironveil".into(),
        };
        let mut live = HashSet::new();
        live.insert("ironveil-uuid".to_string());
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &live,
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_attacked_requires_event() {
        let n2u = name_map(&[("Ironveil", "ironveil-uuid")]);
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnAttacked {
            entity_name: "Ironveil".into(),
        };

        // No event: does NOT fire (event-only condition; no "already
        // attacked" state to short-circuit on).
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
        // Matching event in the snapshot: fires.
        let events = vec![WorldEvent::Attacked {
            uuid: "ironveil-uuid".into(),
            attacker_uuid: "phoenix-uuid".into(),
        }];
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &events,
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_hailed_requires_event() {
        let n2u = name_map(&[("Axiom", "axiom-uuid")]);
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnHailed {
            entity_name: "Axiom".into(),
        };
        let events = vec![WorldEvent::Hailed {
            target_uuid: "axiom-uuid".into(),
        }];
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &events,
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &HashMap::new(),
        ));
    }

    #[test]
    fn follow_up_trigger_holds_on_all_destroyed_fires_when_all_uuids_absent() {
        let n2u = name_map(&[("A", "a-uuid"), ("B", "b-uuid"), ("C", "c-uuid")]);
        let flags = FlagStore::new();
        let cond = TriggerCondition::OnAllDestroyed {
            group: "test".into(),
            after_secs: 0.0,
        };
        let entity_groups: HashMap<String, HashSet<String>> = [(
            "test".to_string(),
            ["A".to_string(), "B".to_string(), "C".to_string()]
                .into_iter()
                .collect(),
        )]
        .into_iter()
        .collect();

        // All three live: does NOT fire.
        let mut live = HashSet::new();
        live.insert("a-uuid".to_string());
        live.insert("b-uuid".to_string());
        live.insert("c-uuid".to_string());
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &live,
            &entity_groups,
        ));

        // A destroyed, B+C still alive: does NOT fire.
        let mut live = HashSet::new();
        live.insert("b-uuid".to_string());
        live.insert("c-uuid".to_string());
        assert!(!follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &live,
            &entity_groups,
        ));

        // All three destroyed: fires.
        assert!(follow_up_trigger_holds(
            Some(&cond),
            0.0,
            &[],
            &n2u,
            &flags,
            &HashSet::new(),
            &HashSet::new(),
            &entity_groups,
        ));
    }
}
