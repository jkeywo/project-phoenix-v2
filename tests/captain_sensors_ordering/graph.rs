//! Production graph capture plus one read-only, pre-extraction raw-access observer.
use bevy::{
    ecs::{
        query::AccessConflicts,
        schedule::{
            graph::{Dag, DiGraph},
            ApplyDeferred, InternedSystemSet, NodeId, ScheduleBuildError, ScheduleBuildPass,
            ScheduleGraph, SystemKey, SystemSetKey,
        },
    },
    platform::hash::FixedHasher,
    prelude::*,
};
use project_phoenix::headless::determinism_audit::{graph::fixed_update_graph, Ambiguity};
use serde_json::{json, Value};
use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
pub const PRODUCERS: [&str; 2] = [
    "project_phoenix::console::captain::server::operate_captain_ai",
    "project_phoenix::ship::sensors::operate_sensors_ai",
];
const CONSUMERS: [(usize, &str); 3] = [
    (
        0,
        "project_phoenix::console::captain::server::handle_set_red_alert",
    ),
    (1, "project_phoenix::ship::sensors::handle_sensors_messages"),
    (1, "project_phoenix::science::server::tick_scans"),
];
pub fn key(graph: &ScheduleGraph, name: &str) -> SystemKey {
    let keys: Vec<_> = graph
        .systems
        .iter()
        .filter(|(_, s, _)| s.name().as_string() == name)
        .map(|(k, _, _)| k)
        .collect();
    assert_eq!(keys.len(), 1, "one actual instance: {name}");
    keys[0]
}
pub fn type_set(graph: &ScheduleGraph, id: SystemKey) -> InternedSystemSet {
    let sets: Vec<_> = graph
        .system_sets
        .iter()
        .filter(|(k, s, _)| {
            s.system_type().is_some()
                && graph
                    .hierarchy()
                    .graph()
                    .all_edges()
                    .any(|(a, b)| a == NodeId::Set(*k) && b == NodeId::System(id))
        })
        .collect();
    assert_eq!(sets.len(), 1);
    let set_key = sets[0].0;
    let members: Vec<_> = graph
        .hierarchy()
        .graph()
        .all_edges()
        .filter(|(a, _)| *a == NodeId::Set(set_key))
        .map(|(_, b)| b)
        .collect();
    assert_eq!(members, [NodeId::System(id)]);
    let actual: Vec<_> = graph
        .systems
        .get(id)
        .unwrap()
        .system
        .default_system_sets()
        .into_iter()
        .filter(|s| graph.system_sets.get_key(*s) == Some(set_key))
        .collect();
    assert_eq!(actual.len(), 1);
    actual[0]
}
// Conditions retain their actual evaluation order. Type IDs are meaningful only
// within these nine processes of this one executable, never across builds.
fn system_metadata(graph: &ScheduleGraph, id: SystemKey) -> Value {
    let system = &graph.systems.get(id).unwrap().system;
    let conditions: Vec<_> = graph
        .systems
        .get_conditions(id)
        .unwrap()
        .iter()
        .map(|c| c.condition.name().as_string())
        .collect();
    json!({"name":system.name().as_string(),"type":format!("{:?}",system.type_id()),"exclusive":system.is_exclusive(),"has_deferred":system.has_deferred(),"conditions":conditions})
}
fn hierarchy_members(graph: &ScheduleGraph, start: NodeId, ancestors: bool) -> Vec<NodeId> {
    let mut pending = vec![start];
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    while let Some(node) = pending.pop() {
        for (parent, child) in graph.hierarchy().graph().all_edges() {
            let next = if ancestors && child == node {
                Some(parent)
            } else if !ancestors && parent == node {
                Some(child)
            } else {
                None
            };
            if let Some(next) = next {
                if seen.insert(format!("{next:?}")) {
                    pending.push(next);
                    result.push(next);
                }
            }
        }
    }
    result
}
fn instance_identities(graph: &ScheduleGraph) -> Vec<Value> {
    let sets: BTreeMap<_,_> = graph.system_sets.iter().map(|(key,set,conditions)| {
        let mut descendants: Vec<_> = hierarchy_members(graph,NodeId::Set(key),false).into_iter().filter_map(|node| node.as_system()).map(|id|system_metadata(graph,id)).collect();
        descendants.sort_by_key(Value::to_string);
        let conditions: Vec<_> = conditions.iter().map(|c|c.condition.name().as_string()).collect();
        let descriptor = if set.is_anonymous() {
            // AnonymousSet allocation numbers change under registration shuffle.
            // Its actual membership multiset and conditions remain meaningful.
            json!({"kind":"anonymous","descendants":descendants,"conditions":conditions})
        } else {
            json!({"kind":"typed","type":format!("{:?}",set.type_id()),"name":format!("{set:?}"),"conditions":conditions})
        };
        (format!("{:?}",NodeId::Set(key)),json!({"id":format!("{:?}",NodeId::Set(key)),"name":format!("{set:?}"),"anonymous":set.is_anonymous(),"descriptor":descriptor}))
    }).collect();
    graph.systems.iter().map(|(id,_,_)| {
        let mut ancestors: Vec<_> = hierarchy_members(graph,NodeId::System(id),true).into_iter().map(|k|sets[&format!("{k:?}")].clone()).collect();
        ancestors.sort_by_key(Value::to_string);
        let mut semantic: Vec<_> = ancestors.iter().map(|a|a["descriptor"].clone()).collect();
        semantic.sort_by_key(Value::to_string);
        json!({"id":format!("{:?}",NodeId::System(id)),"ancestors":ancestors,"descriptor":{"system":system_metadata(graph,id),"ancestors":semantic}})
    }).collect()
}
#[derive(Debug)]
struct RawCapture {
    access: Vec<Ambiguity>,
    identities: Vec<Value>,
}
#[derive(Debug)]
struct RawAccess(Arc<Mutex<Option<RawCapture>>>);
impl ScheduleBuildPass for RawAccess {
    type EdgeOptions = ();
    fn add_dependency(&mut self, _: NodeId, _: NodeId, _: Option<&()>) {}
    fn collapse_set(
        &mut self,
        _: SystemSetKey,
        _: &indexmap::IndexSet<SystemKey, FixedHasher>,
        _: &DiGraph<NodeId>,
    ) -> impl Iterator<Item = (NodeId, NodeId)> {
        std::iter::empty()
    }
    fn build(
        &mut self,
        world: &mut World,
        graph: &mut ScheduleGraph,
        _: &mut Dag<SystemKey>,
    ) -> Result<(), ScheduleBuildError> {
        // Access is initialized here, before Bevy extracts systems into its executable.
        // Read the executor's complete access, never its suppressed ambiguity list.
        let all: Vec<_> = graph.systems.iter().map(|(k, _, _)| k).collect();
        let selected = PRODUCERS.map(|n| key(graph, n));
        let mut raw = Vec::new();
        for (i, &a) in all.iter().enumerate() {
            for &b in &all[i + 1..] {
                if !selected.contains(&a) && !selected.contains(&b) {
                    continue;
                }
                let a = graph.systems.get(a).unwrap();
                let b = graph.systems.get(b).unwrap();
                let mut access = if a.system.is_exclusive() || b.system.is_exclusive() {
                    vec!["<exclusive World access>".into()]
                } else {
                    if a.access.is_compatible(&b.access) {
                        continue;
                    }
                    match a.access.get_conflicts(&b.access) {
                        AccessConflicts::Individual(c) => c
                            .ones()
                            .map(|id| {
                                world
                                    .components()
                                    .get_name(bevy::ecs::component::ComponentId::new(id))
                                    .unwrap()
                                    .as_string()
                            })
                            .collect::<Vec<_>>(),
                        AccessConflicts::All => vec!["<exclusive World access>".into()],
                    }
                };
                if access.is_empty() {
                    continue;
                }
                access.sort();
                let mut systems = [a.system.name().as_string(), b.system.name().as_string()];
                systems.sort();
                raw.push(Ambiguity { systems, access });
            }
        }
        raw.sort();
        assert!(
            self.0
                .lock()
                .unwrap()
                .replace(RawCapture {
                    access: raw,
                    identities: instance_identities(graph)
                })
                .is_none(),
            "one initialized raw capture"
        );
        Ok(())
    }
}
pub fn capture(app: &mut App, order: &str) -> Value {
    assert!(matches!(order, "canonical" | "shuffle-17" | "shuffle-991"));
    let observed = Arc::new(Mutex::new(None));
    let registration = app.world_mut().schedule_scope(FixedUpdate, |_, s| {
        assert!(s.systems().is_err());
        let probes: BTreeSet<_> = super::probes::names()
            .into_iter()
            .flat_map(|(a, b)| [a, b])
            .collect();
        // This is the actual slot iteration before initialization, not a predicted
        // permutation derived from a seed. Parent requires all three to differ.
        let registration: Vec<_> = s
            .graph()
            .systems
            .iter()
            .filter(|(_, system, _)| {
                system.type_id() != TypeId::of::<ApplyDeferred>()
                    && !probes.contains(system.name().as_string().as_str())
            })
            .map(|(_, system, _)| system.name().as_string())
            .collect();
        s.add_build_pass(RawAccess(observed.clone()));
        registration
    });
    let captured_result = fixed_update_graph(app);
    app.world_mut().schedule_scope(FixedUpdate, |_, s| {
        s.remove_build_pass::<RawAccess>();
    });
    let captured = captured_result.expect("ordinary finalized graph capture");
    let raw_capture = observed
        .lock()
        .unwrap()
        .take()
        .expect("initialized raw-access observer ran");
    let raw = raw_capture.access;
    let nodes: Vec<_> = captured
        .nodes
        .iter()
        .filter(|n| n.kind == "system")
        .collect();
    let lookup = |name: &str| {
        let rows: Vec<_> = nodes.iter().filter(|n| n.name == name).collect();
        assert_eq!(rows.len(), 1, "unique {name}");
        rows[0].id.clone()
    };
    let ids = PRODUCERS.map(lookup);
    for id in &ids {
        let node = nodes.iter().find(|n| n.id == *id).unwrap();
        assert!(!node.exclusive && !node.has_deferred && !node.apply_deferred);
    }
    let mut closure = BTreeSet::new();
    for start in &nodes {
        let mut pending = vec![start.id.clone()];
        let mut seen = BTreeSet::new();
        while let Some(a) = pending.pop() {
            for edge in &captured.effective_dependency {
                if edge[0] == a && seen.insert(edge[1].clone()) {
                    pending.push(edge[1].clone());
                }
            }
        }
        for b in seen {
            closure.insert([start.id.clone(), b]);
        }
    }
    let reaches = |a: &str, b: &str| closure.contains(&[a.to_owned(), b.to_owned()]);
    let directions = (reaches(&ids[0], &ids[1]), reaches(&ids[1], &ids[0]));
    assert_eq!(
        directions,
        (true, false),
        "declared Captain -> Sensors in every registration"
    );
    for (producer, consumer) in CONSUMERS {
        assert!(
            reaches(&ids[producer], &lookup(consumer)),
            "actual consumer {consumer}"
        );
    }
    let mut probe_ids = BTreeSet::new();
    let mut boundaries = Vec::new();
    for (i, (before, after)) in super::probes::names().into_iter().enumerate() {
        let before = lookup(before);
        let after = lookup(after);
        for id in [&before, &after] {
            assert!(probe_ids.insert(id.clone()));
            let n = nodes.iter().find(|n| n.id == *id).unwrap();
            assert!(!n.exclusive && !n.has_deferred && !n.apply_deferred);
        }
        let admission = [
            lookup("project_phoenix::command_admission::admit_system_commands"),
            lookup("project_phoenix::command_admission::clear_inter_system_queue"),
        ];
        assert!(admission.iter().all(|a| reaches(a, &before)));
        assert!(reaches(&before, &ids[i]) && reaches(&ids[i], &after));
        boundaries
            .push(json!({"admission":admission,"before":before,"producer":ids[i],"after":after}));
    }
    assert_eq!(probe_ids.len(), 4);
    let production: Vec<_> = nodes
        .iter()
        .filter(|n| !n.apply_deferred && !probe_ids.contains(&n.id))
        .collect();
    let production_ids: BTreeSet<_> = production.iter().map(|n| n.id.as_str()).collect();
    let identities: Vec<_> = raw_capture
        .identities
        .into_iter()
        .filter(|n| production_ids.contains(n["id"].as_str().unwrap()))
        .collect();
    assert_eq!(
        identities.len(),
        production.len(),
        "every actual production instance is mapped"
    );
    let mut logical_instances: Vec<_> =
        identities.iter().map(|n| n["descriptor"].clone()).collect();
    logical_instances.sort_by_key(Value::to_string);
    let unique: BTreeSet<_> = logical_instances.iter().map(Value::to_string).collect();
    assert_eq!(
        unique.len(),
        production.len(),
        "actual ancestry must disambiguate every instance; never collapse remaining duplicates"
    );
    let logical_ids: BTreeMap<_, _> = identities
        .iter()
        .map(|n| {
            (
                n["id"].as_str().unwrap().to_owned(),
                logical_instances
                    .iter()
                    .position(|d| d == &n["descriptor"])
                    .unwrap(),
            )
        })
        .collect();
    let mut paths = Vec::new();
    let mut deferred = Vec::new();
    for a in &production {
        for b in &production {
            if reaches(&a.id, &b.id) {
                paths.push([logical_ids[&a.id], logical_ids[&b.id]]);
            }
            if a.has_deferred
                && nodes.iter().any(|barrier| {
                    barrier.apply_deferred
                        && reaches(&a.id, &barrier.id)
                        && reaches(&barrier.id, &b.id)
                })
            {
                deferred.push([logical_ids[&a.id], logical_ids[&b.id]]);
            }
        }
    }
    assert!(!paths.is_empty() && !deferred.is_empty());
    paths.sort();
    deferred.sort();
    let selected: Vec<_> = raw
        .iter()
        .filter(|r| r.systems == PRODUCERS.map(str::to_owned))
        .collect();
    assert_eq!(selected.len(), 1);
    assert_eq!(
        selected[0].access,
        [std::any::type_name::<
            project_phoenix::core::messages::AdmittedCommands,
        >()]
    );
    let raw_external: Vec<_> = raw
        .iter()
        .filter(|r| r.systems != PRODUCERS.map(str::to_owned))
        .collect();
    assert!(!raw_external.is_empty());
    let reported = captured
        .conflicts
        .iter()
        .filter(|r| r.instances.iter().all(|i| ids.contains(i)))
        .count();
    assert_eq!(
        reported, 0,
        "declared path resolves this one reported pair, raw access remains"
    );
    json!({"order":order,"registration":registration,"logical_instances":logical_instances,"instance_identities":identities,"logical_ids":logical_ids,"pair":selected,"raw_external":raw_external,"capture":captured,"production_instances":production,"production_paths":paths,"deferred_visibility":deferred,"probe_boundaries":boundaries})
}
pub fn assert_preserved(a: &Value, b: &Value) {
    assert_eq!(
        a["logical_instances"], b["logical_instances"],
        "complete ancestry-qualified instance inventory before mapped path comparison"
    );
    if a["registration"] == b["registration"] {
        assert_eq!(
            a["production_instances"], b["production_instances"],
            "same actual registration retains exact physical IDs and metadata"
        );
        assert_eq!(
            a["logical_ids"], b["logical_ids"],
            "same registration retains exact instance mapping"
        );
        assert_eq!(
            a["instance_identities"], b["instance_identities"],
            "same registration retains raw ancestors too"
        );
    }
    assert_eq!(
        a["raw_external"], b["raw_external"],
        "complete external vectors and multiplicity"
    );
    for field in ["production_paths", "deferred_visibility"] {
        let expected = a[field].as_array().unwrap();
        assert!(!expected.is_empty());
        let actual: BTreeSet<_> = b[field]
            .as_array()
            .unwrap()
            .iter()
            .map(Value::to_string)
            .collect();
        for row in expected {
            assert!(actual.contains(&row.to_string()), "lost {field}: {row}");
        }
    }
}
