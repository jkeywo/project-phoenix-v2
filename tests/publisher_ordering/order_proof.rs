//! Declared-order stability of existing production instances across registration changes.
use bevy::{
    ecs::{
        component::ComponentId,
        query::AccessConflicts,
        schedule::{
            graph::{Dag, DiGraph},
            InternedSystemSet, NodeId, ScheduleBuildError, ScheduleBuildPass, ScheduleGraph,
            SystemKey, SystemSetKey,
        },
    },
    platform::hash::FixedHasher,
    prelude::*,
};
use project_phoenix::headless::determinism_audit::Ambiguity;
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

const SYSTEMS: [&str; 3] = [
    "project_phoenix::ship::power::publish_power_blackboard",
    "project_phoenix::ship::shields::publish_shields_blackboard",
    "project_phoenix::console::repair::server::publish_repair_blackboard",
];
const PAIRS: [(usize, usize); 3] = [(0, 1), (0, 2), (1, 2)];

fn keys(graph: &ScheduleGraph) -> Vec<SystemKey> {
    SYSTEMS
        .iter()
        .map(|name| {
            let matches: Vec<_> = graph
                .systems
                .iter()
                .filter(|(_, system, _)| system.name().as_string() == *name)
                .map(|(key, _, _)| key)
                .collect();
            assert_eq!(
                matches.len(),
                1,
                "annotation must own one actual instance: {name}"
            );
            matches[0]
        })
        .collect()
}

fn reachable(edges: &[(SystemKey, SystemKey)], from: SystemKey, to: SystemKey) -> bool {
    let mut pending = vec![from];
    let mut seen = HashSet::new();
    while let Some(node) = pending.pop() {
        if node == to {
            return true;
        }
        if seen.insert(node) {
            pending.extend(edges.iter().filter(|(a, _)| *a == node).map(|(_, b)| *b));
        }
    }
    false
}

fn debt(
    world: &World,
    mut systems: [String; 2],
    ids: impl Iterator<Item = ComponentId>,
) -> Ambiguity {
    systems.sort();
    let mut access: Vec<_> = ids
        .map(|id| world.components().get_name(id).unwrap().as_string())
        .collect();
    if access.is_empty() {
        access.push("<exclusive World access>".into());
    }
    access.sort();
    Ambiguity { systems, access }
}

#[derive(Debug)]
struct Observe {
    order: String,
    result: Arc<Mutex<Option<Value>>>,
}
impl ScheduleBuildPass for Observe {
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
        dag: &mut Dag<SystemKey>,
    ) -> Result<(), ScheduleBuildError> {
        let ids = keys(graph);
        let edges: Vec<_> = dag.graph().all_edges().collect();
        let mut pairs = Vec::new();
        for (a, b) in PAIRS {
            let first = graph.systems.get(ids[a]).unwrap();
            let second = graph.systems.get(ids[b]).unwrap();
            assert!(!first.system.is_exclusive() && !second.system.is_exclusive());
            // This is the SAME access compatibility used by Bevy's executor,
            // independent of its ignored-ambiguity graph. Containers stay serialized.
            assert!(
                !first.access.is_compatible(&second.access),
                "do not hide actual access"
            );
            let AccessConflicts::Individual(indices) = first.access.get_conflicts(&second.access)
            else {
                panic!("new unbounded overlap invalidates the declared-order proof");
            };
            let mut access: Vec<_> = indices
                .ones()
                .map(|id| {
                    world
                        .components()
                        .get_name(ComponentId::new(id))
                        .unwrap()
                        .as_string()
                })
                .collect();
            access.sort();
            let expected = vec![std::any::type_name::<
                project_phoenix::server_app::ShipSystemBlackboards,
            >()
            .to_owned()];
            assert_eq!(
                access, expected,
                "new overlap requires a new logical proof: {} / {}",
                SYSTEMS[a], SYSTEMS[b]
            );
            let forward = reachable(&edges, ids[a], ids[b]);
            let reverse = reachable(&edges, ids[b], ids[a]);
            let expected = crate::declared_order::before(SYSTEMS[a], SYSTEMS[b]);
            assert_eq!((forward, reverse), (expected, !expected));
            pairs.push(json!({"systems":[SYSTEMS[a],SYSTEMS[b]],"access":access,"incompatible":true,"forward_path":forward,"reverse_path":reverse}));
        }
        // Retain complete physical external vectors even when production order
        // resolves every reported row; separately check the unordered census subset.
        let mut external = Vec::new();
        let mut raw_external = Vec::new();
        for &a in &ids {
            for (b, system, _) in graph.systems.iter() {
                if ids.contains(&b) {
                    continue;
                }
                let first = graph.systems.get(a).unwrap();
                let second = graph.systems.get(b).unwrap();
                let names = [first.system.name().as_string(), system.name().as_string()];
                let previous = raw_external.len();
                if first.system.is_exclusive() || second.system.is_exclusive() {
                    raw_external.push(debt(world, names, std::iter::empty()));
                } else if !first.access.is_compatible(&second.access) {
                    let access = match first.access.get_conflicts(&second.access) {
                        AccessConflicts::All => Vec::new(),
                        AccessConflicts::Individual(indices) => {
                            indices.ones().map(ComponentId::new).collect()
                        }
                    };
                    raw_external.push(debt(world, names, access.into_iter()));
                }
                if raw_external.len() > previous
                    && !reachable(&edges, a, b)
                    && !reachable(&edges, b, a)
                {
                    external.push(raw_external.last().unwrap().clone());
                }
            }
        }
        raw_external.sort();
        external.sort();
        assert!(
            !raw_external.is_empty(),
            "external-conflict preservation must not be vacuous"
        );
        let mut external_edges: Vec<_> = edges
            .iter()
            .filter(|(a, b)| !(ids.contains(a) && ids.contains(b)))
            .map(|(a, b)| [format!("{a:?}"), format!("{b:?}")])
            .collect();
        external_edges.sort();
        *self.result.lock().unwrap() = Some(
            json!({"order":self.order,"unique_instances":3,"pairs":pairs,
                "external_conflicts":external,"raw_external":raw_external,"external_edges":external_edges,"graph":crate::declared_order::capture(graph,dag,&self.order)}),
        );
        Ok(())
    }
}

pub struct Proof(Arc<Mutex<Option<Value>>>);
pub fn install(app: &mut App, order: &str) -> Proof {
    assert!(matches!(
        order,
        "ordinary" | "shuffle-a" | "shuffle-b" | "physics-last"
    ));
    let result = Arc::new(Mutex::new(None));
    app.world_mut().schedule_scope(FixedUpdate, |_, schedule| {
        assert!(
            schedule.systems().is_err(),
            "proof must precede the first real run"
        );
        let graph = schedule.graph();
        let ids = keys(graph);
        // Resolve each existing direct SystemTypeSet, checking multiplicity so
        // a future repeated registration cannot silently broaden the selected owners.
        let sets: Vec<InternedSystemSet> =
            ids.iter()
                .map(|id| {
                    let matches: Vec<_> = graph
                        .system_sets
                        .iter()
                        .filter(|(key, set, _)| {
                            set.system_type().is_some()
                                && graph.hierarchy().graph().all_edges().any(|(a, b)| {
                                    a == NodeId::Set(*key) && b == NodeId::System(*id)
                                })
                        })
                        .collect();
                    assert_eq!(matches.len(), 1, "unique direct type-set membership");
                    let (key, _, _) = matches[0];
                    let members: Vec<_> = graph
                        .hierarchy()
                        .graph()
                        .all_edges()
                        .filter(|(a, _)| *a == NodeId::Set(key))
                        .map(|(_, b)| b)
                        .collect();
                    assert_eq!(
                        members,
                        [NodeId::System(*id)],
                        "type-set must own only this one instance"
                    );
                    // The trait-object set cannot call Sized-only intern().
                    // Obtain this actual system's already-interned defaults,
                    // then require the same uniquely verified graph set key.
                    let interned: Vec<_> = graph
                        .systems
                        .get(*id)
                        .unwrap()
                        .system
                        .default_system_sets()
                        .into_iter()
                        .filter(|set| graph.system_sets.get_key(*set) == Some(key))
                        .collect();
                    assert_eq!(interned.len(), 1, "actual default type-set identity");
                    interned[0]
                })
                .collect();
        assert_eq!(sets.len(), 3);
        schedule.add_build_pass(Observe {
            order: order.to_owned(),
            result: result.clone(),
        });
    });
    Proof(result)
}
impl Proof {
    pub fn finish(self, app: &mut App) -> Value {
        let result = self
            .0
            .lock()
            .unwrap()
            .take()
            .expect("actual build pass must run");
        app.world_mut()
            .schedule_scope(FixedUpdate, |world, schedule| {
                // Test-local orders retain physical conflicts; only reachability changes.
                // Do not inspect graph.systems here: initialization moved instances
                // into the executable. Their stable names are exposed by systems().
                let names: std::collections::HashMap<_, _> = schedule
                    .systems()
                    .unwrap()
                    .map(|(id, system)| (id, system.name().as_string()))
                    .collect();
                let mut external = Vec::new();
                for (a, b, access) in schedule.graph().conflicting_systems().iter() {
                    assert!(
                        !PAIRS.iter().any(|(x, y)| (names[a] == SYSTEMS[*x]
                            && names[b] == SYSTEMS[*y])
                            || (names[b] == SYSTEMS[*x] && names[a] == SYSTEMS[*y])),
                        "the three annotated pairs must be absent even in ordinary order"
                    );
                    if SYSTEMS.contains(&names[a].as_str()) != SYSTEMS.contains(&names[b].as_str())
                    {
                        external.push(debt(
                            world,
                            [names[a].clone(), names[b].clone()],
                            access.iter().copied(),
                        ));
                    }
                }
                external.sort();
                assert_eq!(
                    serde_json::to_value(external).unwrap(),
                    result["external_conflicts"],
                    "all other incident access vectors and multiplicities remain visible"
                );
                schedule.remove_build_pass::<Observe>();
            });
        result
    }
}
