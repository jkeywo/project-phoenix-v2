//! Frozen observed owner ranks and registration perturbations; no test ordering edges.
use bevy::{
    ecs::schedule::{graph::Dag, NodeId, ScheduleGraph, SystemKey},
    prelude::*,
};
use project_phoenix::{headless::HeadlessArgs, server_app::RegistrationOrder};
use serde_json::{json, Value};
use std::{
    any::TypeId,
    collections::{BTreeMap, HashMap, HashSet},
};

pub fn build(args: HeadlessArgs, role: &str) -> App {
    let fixture = super::common::SimFixture::new(args);
    match role {
        "ordinary" => fixture.build(),
        "shuffle-a" => fixture
            .registration_order(RegistrationOrder::Shuffled(17))
            .build(),
        "shuffle-b" => fixture
            .registration_order(RegistrationOrder::Shuffled(991))
            .build(),
        "physics-last" => fixture.physics_last(true).build(),
        _ => panic!("unknown registration role {role}"),
    }
}
pub fn before(a: &str, b: &str) -> bool {
    let inventory: Value = serde_json::from_str(include_str!(
        "../fixtures/determinism/declared-owner-order.json"
    ))
    .unwrap();
    let rank = |name: &str| {
        let matches: Vec<_> = inventory["owners"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["name"] == name)
            .collect();
        assert_eq!(matches.len(), 1, "unique frozen owner {name}");
        matches[0]["index"].as_u64().unwrap()
    };
    let (a, b) = (rank(a), rank(b));
    assert_ne!(a, b);
    a < b
}
/// Actual ancestor-set semantic membership distinguishes repeated function
/// registrations without assigning identity from insertion rank or displayed name.
pub fn membership(graph: &ScheduleGraph, id: SystemKey) -> Vec<String> {
    let edges: Vec<_> = graph.hierarchy().graph().all_edges().collect();
    let sets: HashMap<_, _> = graph.system_sets.iter().map(|(key,set,conditions)| {
        let conditions: Vec<_> = conditions.iter().map(|c|c.condition.name().as_string()).collect();
        let descriptor = if set.is_anonymous() {
            // AnonymousSet's numeric Debug suffix is allocation, not semantic
            // identity. Retain actual descendant multiplicity and condition order.
            let mut pending = vec![NodeId::Set(key)];
            let mut seen = HashSet::new();
            let mut members = Vec::new();
            while let Some(parent) = pending.pop() {
                for &(source, member) in &edges {
                    if source == parent && seen.insert(member) {
                        match member {
                            NodeId::Set(_) => pending.push(member),
                            NodeId::System(id) => {
                                let s = &graph.systems.get(id).unwrap().system;
                                members.push(json!({"name":s.name().as_string(),"exclusive":s.is_exclusive(),"has_deferred":s.has_deferred()}));
                            }
                        }
                    }
                }
            }
            members.sort_by_key(Value::to_string);
            json!({"anonymous":true,"conditions":conditions,"members":members})
        } else {
            json!({"anonymous":false,"name":format!("{set:?}"),"conditions":conditions})
        };
        (key, descriptor.to_string())
    }).collect();
    let mut pending = vec![NodeId::System(id)];
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    while let Some(child) = pending.pop() {
        for &(parent, member) in &edges {
            if member == child && seen.insert(parent) {
                let NodeId::Set(key) = parent else {
                    panic!("hierarchy parent must be a set")
                };
                names.push(sets[&key].clone());
                pending.push(parent);
            }
        }
    }
    names.sort(); // Preserve equal-descriptor distinct set instances, not a set.
    names
}
pub fn hierarchy(graph: &ScheduleGraph) -> Value {
    let id = |node| match node {
        NodeId::System(key) => format!("{key:?}"),
        NodeId::Set(key) => format!("{key:?}"),
    };
    json!({"sets":graph.system_sets.iter().map(|(key,set,conditions)|json!({"id":format!("{key:?}"),"name":format!("{set:?}"),"anonymous":set.is_anonymous(),"conditions":conditions.iter().map(|c|c.condition.name().as_string()).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "edges":graph.hierarchy().graph().all_edges().map(|(a,b)|[id(a),id(b)]).collect::<Vec<_>>()})
}
/// Retain every raw instance and edge. Automatic barriers are compared by actual
/// writer/sink visibility obligations, never by incidental allocation identity.
#[allow(dead_code)] // The producer binary retains its own probe-aware capture.
pub fn capture(graph: &ScheduleGraph, dag: &Dag<SystemKey>, role: &str) -> Value {
    let all: Vec<_> = graph.systems.iter().map(|(id, _, _)| id).collect();
    let edges: Vec<_> = dag.graph().all_edges().collect();
    let closure: HashMap<_, HashSet<_>> = all
        .iter()
        .map(|&a| {
            let mut seen = HashSet::new();
            let mut pending: Vec<_> = edges
                .iter()
                .filter(|(x, _)| *x == a)
                .map(|(_, y)| *y)
                .collect();
            while let Some(n) = pending.pop() {
                if seen.insert(n) {
                    pending.extend(edges.iter().filter(|(x, _)| *x == n).map(|(_, y)| *y));
                }
            }
            (a, seen)
        })
        .collect();
    let deferred: HashSet<_> = all
        .iter()
        .copied()
        .filter(|&id| {
            graph.systems.get(id).unwrap().system.type_id()
                == TypeId::of::<bevy::ecs::schedule::ApplyDeferred>()
        })
        .collect();
    let mut instances:Vec<_>=all.iter().map(|&id|{let s=&graph.systems.get(id).unwrap().system;json!({"id":format!("{id:?}"),"name":s.name().as_string(),"exclusive":s.is_exclusive(),"has_deferred":s.has_deferred(),"apply_deferred":deferred.contains(&id),"probe":false,"membership":membership(graph,id)})}).collect();
    instances.sort_by_key(|v| v["id"].as_str().unwrap().to_owned());
    let production: Vec<_> = all
        .iter()
        .copied()
        .filter(|id| !deferred.contains(id))
        .collect();
    let mut paths = Vec::new();
    let mut visibility = Vec::new();
    for &a in &production {
        for &b in &production {
            if closure[&a].contains(&b) {
                paths.push([format!("{a:?}"), format!("{b:?}")]);
            }
            if graph.systems.get(a).unwrap().system.has_deferred()
                && deferred
                    .iter()
                    .any(|d| closure[&a].contains(d) && closure[d].contains(&b))
            {
                visibility.push([format!("{a:?}"), format!("{b:?}")]);
            }
        }
    }
    paths.sort();
    visibility.sort();
    json!({"order":role,"hierarchy":hierarchy(graph),"production_instances":instances.iter().filter(|v|v["apply_deferred"]==false).collect::<Vec<_>>(),"instances":instances,"effective_edges":edges.iter().map(|(a,b)|[format!("{a:?}"),format!("{b:?}")]).collect::<Vec<_>>(),"production_paths":paths,"deferred_visibility":visibility})
}
/// Only a unique complete concrete-instance metadata correspondence permits ID
/// remapping. Repeated ambiguous concrete identities fail rather than collapse.
pub fn assert_graph(base: &Value, actual: &Value) {
    fn inventory(v: &Value) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for row in v["production_instances"].as_array().unwrap() {
            let mut metadata = row.clone();
            let id = metadata
                .as_object_mut()
                .unwrap()
                .remove("id")
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned();
            assert!(
                out.insert(metadata.to_string(), id).is_none(),
                "ambiguous repeated concrete identity"
            );
        }
        assert!(!out.is_empty());
        out
    }
    let b = inventory(base);
    let a = inventory(actual);
    assert_eq!(
        b.keys().collect::<Vec<_>>(),
        a.keys().collect::<Vec<_>>(),
        "complete concrete inventory"
    );
    if base["order"] != actual["order"] {
        assert_ne!(
            a, b,
            "registration perturbation must alter actual concrete instance registration identities"
        );
    }
    let mapping: HashMap<_, _> = a
        .iter()
        .map(|(meta, id)| (id.as_str(), b[meta].as_str()))
        .collect();
    for field in ["production_paths", "deferred_visibility"] {
        let expected = base[field].as_array().unwrap();
        assert!(!expected.is_empty(), "nonvacuous {field}");
        let mut counts = BTreeMap::<[String; 2], usize>::new();
        for pair in actual[field].as_array().unwrap() {
            let p = [
                mapping[pair[0].as_str().unwrap()].to_owned(),
                mapping[pair[1].as_str().unwrap()].to_owned(),
            ];
            *counts.entry(p).or_default() += 1;
        }
        for pair in expected {
            let p = [
                pair[0].as_str().unwrap().to_owned(),
                pair[1].as_str().unwrap().to_owned(),
            ];
            let n = counts
                .get_mut(&p)
                .expect("existing concrete dependency/flush obligation disappeared");
            assert!(*n > 0);
            *n -= 1;
        }
    }
}

/// Require all requested registration roles to be genuinely distinct while
/// repeated fresh pool modes of one role retain the same registration inventory.
pub fn observe_role(seen: &mut BTreeMap<String, Value>, graph: &Value) {
    let role = graph["order"].as_str().unwrap();
    let inventory = &graph["production_instances"];
    if let Some(previous) = seen.get(role) {
        assert_eq!(
            previous, inventory,
            "same registration role across fresh pools"
        );
    } else {
        for (other, previous) in seen.iter() {
            assert_ne!(
                previous, inventory,
                "distinct registration roles: {role}/{other}"
            );
        }
        seen.insert(role.to_owned(), inventory.clone());
    }
}
