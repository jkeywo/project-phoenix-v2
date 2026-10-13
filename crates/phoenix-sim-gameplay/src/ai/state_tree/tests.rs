use super::*;
use std::collections::BTreeMap;
fn config() -> AiTreeConfig {
    serde_json::from_value(serde_json::json!({"root":"main","tree":[{"id":"main","initial":"root","memory":{"owned":false},"state":[{"id":"root","task":[{"id":"claim","action":{"kind":"remember","key":"owned","value":{"source":"literal","value":true}}},{"id":"write","action":{"kind":"write","channel":"enabled","value":{"source":"memory","name":"owned"}}}]}]}]})).unwrap()
}
fn bindings() -> SupportedBindings {
    SupportedBindings {
        facts: Default::default(),
        channels: ["enabled".into()].into(),
    }
}
#[test]
fn writes_observe_prior_memory_and_roundtrip_runtime() {
    let p = compile(&config(), &bindings()).unwrap();
    let mut r = TreeRuntime::default();
    let channels = BTreeMap::from([("enabled".into(), false)]);
    let result = step(&p, &BTreeMap::new(), &channels, &mut r, 1).unwrap();
    assert!(result.intents["enabled"]);
    assert_eq!(r.memory("owned"), Some(true));
    let mut restored: TreeRuntime =
        serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(
        step(&p, &BTreeMap::new(), &channels, &mut restored, 2).unwrap(),
        step(&p, &BTreeMap::new(), &channels, &mut r, 2).unwrap()
    );
    assert_eq!(r, restored);
}
#[test]
fn errors_are_transactional_and_unwritten_channels_are_not_intents() {
    let mut c = config();
    c.tree[0].state[0].task.clear();
    let p = compile(&c, &bindings()).unwrap();
    let mut r = TreeRuntime::default();
    let channels = BTreeMap::from([("enabled".into(), false)]);
    assert!(step(&p, &BTreeMap::new(), &channels, &mut r, 1)
        .unwrap()
        .intents
        .is_empty());
    let before = r.clone();
    assert!(step(&p, &BTreeMap::new(), &channels, &mut r, 1).is_err());
    assert_eq!(r, before);
    assert!(step(&p, &BTreeMap::new(), &BTreeMap::new(), &mut r, 2).is_err());
    assert_eq!(r, before);
}
#[test]
fn inheritance_replaces_in_place_and_appends() {
    let mut c = config();
    let mut child = c.tree[0].clone();
    child.id = "derived".into();
    child.extends = Some("main".into());
    child.initial = None;
    child.state[0].task.remove(0);
    child.state[0].task[0].action = TaskAction::Write {
        channel: "enabled".into(),
        value: BoolSource::Literal { value: false },
    };
    c.root = "derived".into();
    c.tree.push(child);
    let resolved = compile(&c, &bindings()).unwrap().resolved();
    let tree = resolved.tree.iter().find(|t| t.id == "derived").unwrap();
    assert_eq!(
        tree.state[0]
            .task
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        vec!["claim", "write"]
    );
}
#[test]
fn closed_schema_and_cycles_fail() {
    assert!(serde_json::from_value::<AiTreeConfig>(
        serde_json::json!({"root":"x","tree":[],"script":"code"})
    )
    .is_err());
    let mut c = config();
    c.tree[0].extends = Some("main".into());
    assert!(compile(&c, &bindings()).is_err());
    c.tree[0].extends = None;
    c.tree[0].state[0].task.push(TaskDef {
        id: "recurse".into(),
        when: vec![],
        action: TaskAction::Call {
            tree: "main".into(),
            parameters: BTreeMap::new(),
        },
    });
    assert!(compile(&c, &bindings()).is_err());
}
#[test]
fn references_are_inline_and_parent_transition_interrupts() {
    let mut c = config();
    let mut child = c.tree[0].clone();
    child.id = "child".into();
    child.state[0].task = vec![TaskDef {
        id: "write".into(),
        when: vec![],
        action: TaskAction::Write {
            channel: "enabled".into(),
            value: BoolSource::Literal { value: false },
        },
    }];
    c.tree[0].state[0].task.insert(
        1,
        TaskDef {
            id: "call".into(),
            when: vec![],
            action: TaskAction::Call {
                tree: "child".into(),
                parameters: BTreeMap::new(),
            },
        },
    );
    c.tree[0].state[0].transition.push(TransitionDef {
        id: "leave".into(),
        when: vec![BoolCondition {
            value: BoolSource::Memory {
                name: "owned".into(),
            },
            equals: true,
        }],
        target: TransitionTarget::State {
            state: "done".into(),
        },
    });
    c.tree[0].state.push(StateDef {
        id: "done".into(),
        parent: None,
        initial_child: None,
        task: vec![],
        transition: vec![],
    });
    c.tree.push(child);
    let p = compile(&c, &bindings()).unwrap();
    let mut r = TreeRuntime::default();
    let channels = BTreeMap::from([("enabled".into(), false)]);
    assert!(
        step(&p, &BTreeMap::new(), &channels, &mut r, 1)
            .unwrap()
            .intents["enabled"]
    );
    assert_eq!(r.root.as_ref().unwrap().calls.len(), 1);
    assert!(step(&p, &BTreeMap::new(), &channels, &mut r, 2)
        .unwrap()
        .intents
        .is_empty());
    assert!(r.root.unwrap().calls.is_empty());
}

#[test]
fn child_calls_have_independent_memory_and_completion() {
    let mut c = config();
    let mut child = c.tree[0].clone();
    child.id = "child".into();
    child.state[0].task.push(TaskDef {
        id: "finish".into(),
        when: vec![],
        action: TaskAction::Complete {
            result: Completion::Success,
        },
    });
    c.tree[0].state[0].task = vec![
        TaskDef {
            id: "one".into(),
            when: vec![],
            action: TaskAction::Call {
                tree: "child".into(),
                parameters: BTreeMap::new(),
            },
        },
        TaskDef {
            id: "two".into(),
            when: vec![BoolCondition {
                value: BoolSource::CallResult {
                    task: "root/one".into(),
                    result: Completion::Success,
                },
                equals: true,
            }],
            action: TaskAction::Call {
                tree: "child".into(),
                parameters: BTreeMap::new(),
            },
        },
    ];
    c.tree.push(child);
    let p = compile(&c, &bindings()).unwrap();
    let mut r = TreeRuntime::default();
    step(
        &p,
        &BTreeMap::new(),
        &BTreeMap::from([("enabled".into(), false)]),
        &mut r,
        1,
    )
    .unwrap();
    let root = r.root.as_ref().unwrap();
    assert!(!root.memory["owned"]);
    assert_eq!(root.calls.len(), 2);
    for child in root.calls.values() {
        assert!(child.memory["owned"]);
        assert_eq!(child.result, Some(Completion::Success));
    }
}
#[test]
fn pending_values_are_visible_and_facts_remain_frozen() {
    let mut c = config();
    c.tree[0].state[0].task[0].action = TaskAction::Write {
        channel: "enabled".into(),
        value: BoolSource::Fact {
            name: "ready".into(),
        },
    };
    c.tree[0].state[0].task[1].action = TaskAction::Remember {
        key: "owned".into(),
        value: BoolSource::Pending {
            channel: "enabled".into(),
        },
    };
    let mut b = bindings();
    b.facts.insert("ready".into());
    let p = compile(&c, &b).unwrap();
    let facts = BTreeMap::from([("ready".into(), true)]);
    let mut r = TreeRuntime::default();
    step(
        &p,
        &facts,
        &BTreeMap::from([("enabled".into(), false)]),
        &mut r,
        1,
    )
    .unwrap();
    assert_eq!(r.memory("owned"), Some(true));
    assert!(facts["ready"]);
}
#[test]
fn invalid_restored_runtime_is_transactionally_rejected() {
    let p = compile(&config(), &bindings()).unwrap();
    let mut r = TreeRuntime::default();
    let channels = BTreeMap::from([("enabled".into(), false)]);
    step(&p, &BTreeMap::new(), &channels, &mut r, 1).unwrap();
    r.root
        .as_mut()
        .unwrap()
        .memory
        .insert("unexpected".into(), false);
    let before = r.clone();
    assert!(step(&p, &BTreeMap::new(), &channels, &mut r, 2).is_err());
    assert_eq!(r, before);
}

#[test]
fn shared_host_gate_preserves_private_continuation() {
    use crate::ai::host::{decide_tree, TreeHostOutcome};
    use crate::core::messages::SystemId;
    use crate::ship::control_source::{ControlSource, ControlSourceResolver};
    let p = compile(&config(), &bindings()).unwrap();
    let mut r = TreeRuntime::default();
    let mut sources = ControlSourceResolver::new();
    let id = SystemId("dock".into());
    let channels = BTreeMap::from([("enabled".into(), false)]);
    for source in [ControlSource::Human, ControlSource::Offline] {
        sources.set(id.clone(), source);
        assert_eq!(
            decide_tree(
                &sources,
                &id,
                Some(&p),
                &BTreeMap::new(),
                &channels,
                &mut r,
                1
            )
            .unwrap(),
            TreeHostOutcome::NotAiOperated
        );
        assert_eq!(r, TreeRuntime::default());
    }
    sources.set(id.clone(), ControlSource::Ai);
    assert_eq!(
        decide_tree(&sources, &id, None, &BTreeMap::new(), &channels, &mut r, 1).unwrap(),
        TreeHostOutcome::Undeclared
    );
    assert_eq!(r, TreeRuntime::default());
    assert!(matches!(
        decide_tree(
            &sources,
            &id,
            Some(&p),
            &BTreeMap::new(),
            &channels,
            &mut r,
            1
        )
        .unwrap(),
        TreeHostOutcome::Evaluated(_)
    ));
    let saved = r.clone();
    sources.set(id.clone(), ControlSource::Human);
    decide_tree(
        &sources,
        &id,
        Some(&p),
        &BTreeMap::new(),
        &channels,
        &mut r,
        2,
    )
    .unwrap();
    assert_eq!(r, saved);
}

#[test]
fn hierarchy_runs_parent_before_child_and_child_can_override() {
    let mut c = config();
    c.tree[0].state[0].initial_child = Some("leaf".into());
    c.tree[0].state.push(StateDef {
        id: "leaf".into(),
        parent: Some("root".into()),
        initial_child: None,
        transition: vec![],
        task: vec![TaskDef {
            id: "override".into(),
            when: vec![BoolCondition {
                value: BoolSource::Memory {
                    name: "owned".into(),
                },
                equals: true,
            }],
            action: TaskAction::Write {
                channel: "enabled".into(),
                value: BoolSource::Literal { value: false },
            },
        }],
    });
    let p = compile(&c, &bindings()).unwrap();
    let result = step(
        &p,
        &BTreeMap::new(),
        &BTreeMap::from([("enabled".into(), false)]),
        &mut TreeRuntime::default(),
        1,
    )
    .unwrap();
    assert!(!result.intents["enabled"]);
    let writes: Vec<_> = result
        .trace
        .iter()
        .filter_map(|event| {
            if let TraceEvent::Write { value, .. } = event {
                Some(*value)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(writes, vec![true, false]);
}
#[test]
fn cached_ancestry_cannot_bypass_depth_bound() {
    let mut c = config();
    for i in 0..MAX_DEPTH {
        let mut child = c.tree[0].clone();
        child.id = format!("z{i:02}");
        child.extends = Some(if i == 0 {
            "main".into()
        } else {
            format!("z{:02}", i - 1)
        });
        child.state.clear();
        child.initial = None;
        c.tree.push(child);
    }
    assert!(compile(&c, &bindings()).is_err());
}

#[test]
fn restored_internal_state_cannot_skip_required_child() {
    let mut c = config();
    c.tree[0].state[0].initial_child = Some("leaf".into());
    c.tree[0].state.push(StateDef {
        id: "leaf".into(),
        parent: Some("root".into()),
        initial_child: None,
        task: vec![],
        transition: vec![],
    });
    let p = compile(&c, &bindings()).unwrap();
    let channels = BTreeMap::from([("enabled".into(), false)]);
    let mut runtime = TreeRuntime::default();
    step(&p, &BTreeMap::new(), &channels, &mut runtime, 1).unwrap();
    runtime.root.as_mut().unwrap().active_leaf = "root".into();
    let before = runtime.clone();
    assert!(step(&p, &BTreeMap::new(), &channels, &mut runtime, 2).is_err());
    assert_eq!(runtime, before);
}
