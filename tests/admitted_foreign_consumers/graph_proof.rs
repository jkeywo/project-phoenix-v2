//! Observe actual production instances and declared order without adding edges.
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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

pub const PRODUCER: &str =
    "project_phoenix::console::captain::server::backfill_captain_prefers_cinematic_view";
pub const CONSUMERS: [&str; 5] = [
    "project_phoenix::console::captain::server::handle_set_red_alert",
    "project_phoenix::console::weapons::beam::handle_set_target",
    "project_phoenix::console::weapons::torpedo::handle_set_torpedo_volley_target",
    "project_phoenix::console::weapons::beam::handle_set_phaser_mode",
    "project_phoenix::console::weapons::beam::handle_set_phaser_frequency",
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Candidate {
    pub producer: String,
    pub consumer: String,
}

#[derive(Deserialize)]
struct Inventory {
    schema: u32,
    source_census_sha256: String,
    pairs: Vec<Candidate>,
}

pub fn candidates() -> Vec<Candidate> {
    let inventory: Inventory = serde_json::from_str(include_str!(
        "../fixtures/determinism/admitted-foreign-candidates.json"
    ))
    .unwrap();
    assert_eq!(inventory.schema, 1);
    assert_eq!(
        inventory.source_census_sha256,
        "AFB8E64538203DAC8A8C64130F97925AAC8B074C6B2C7686807CD7D9AEDFA7A9"
    );
    assert_eq!(
        inventory.pairs.len(),
        275,
        "exact authorized audit boundary"
    );
    let mut unique = inventory.pairs.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 275, "candidate multiplicity is explicit");
    inventory.pairs
}

fn selected(pair: &Candidate) -> bool {
    pair.producer == PRODUCER && CONSUMERS.contains(&pair.consumer.as_str())
}

fn key(graph: &ScheduleGraph, name: &str) -> SystemKey {
    let matches: Vec<_> = graph
        .systems
        .iter()
        .filter(|(_, system, _)| system.name().as_string() == name)
        .map(|(key, _, _)| key)
        .collect();
    assert_eq!(matches.len(), 1, "exact actual instance: {name}");
    matches[0]
}

fn type_set(graph: &ScheduleGraph, id: SystemKey) -> InternedSystemSet {
    let matches: Vec<_> = graph
        .system_sets
        .iter()
        .filter(|(key, set, _)| {
            set.system_type().is_some()
                && graph
                    .hierarchy()
                    .graph()
                    .all_edges()
                    .any(|(a, b)| a == NodeId::Set(*key) && b == NodeId::System(id))
        })
        .collect();
    assert_eq!(matches.len(), 1, "unique direct type-set");
    let (set_key, _, _) = matches[0];
    let members: Vec<_> = graph
        .hierarchy()
        .graph()
        .all_edges()
        .filter(|(a, _)| *a == NodeId::Set(set_key))
        .map(|(_, b)| b)
        .collect();
    assert_eq!(members, [NodeId::System(id)], "no repeated registration");
    let sets: Vec<_> = graph
        .systems
        .get(id)
        .unwrap()
        .system
        .default_system_sets()
        .into_iter()
        .filter(|set| graph.system_sets.get_key(*set) == Some(set_key))
        .collect();
    assert_eq!(sets.len(), 1, "existing interned identity");
    sets[0]
}

fn conflict(world: &World, graph: &ScheduleGraph, a: SystemKey, b: SystemKey) -> Option<Ambiguity> {
    let first = graph.systems.get(a).unwrap();
    let second = graph.systems.get(b).unwrap();
    let mut names = [
        first.system.name().as_string(),
        second.system.name().as_string(),
    ];
    names.sort();
    let mut access = if first.system.is_exclusive() || second.system.is_exclusive() {
        vec!["<exclusive World access>".to_owned()]
    } else {
        if first.access.is_compatible(&second.access) {
            return None;
        }
        match first.access.get_conflicts(&second.access) {
            AccessConflicts::All => vec!["<exclusive World access>".to_owned()],
            AccessConflicts::Individual(indices) => indices
                .ones()
                .map(|id| {
                    world
                        .components()
                        .get_name(ComponentId::new(id))
                        .unwrap()
                        .as_string()
                })
                .collect(),
        }
    };
    access.sort();
    Some(Ambiguity {
        systems: names,
        access,
    })
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
        let all: Vec<_> = graph.systems.iter().map(|(id, _, _)| id).collect();
        let mut adjacency = HashMap::<SystemKey, Vec<SystemKey>>::new();
        for (a, b) in dag.graph().all_edges() {
            adjacency.entry(a).or_default().push(b);
        }
        let closure: HashMap<SystemKey, HashSet<SystemKey>> = all
            .iter()
            .map(|&start| {
                let mut seen = HashSet::new();
                let mut pending = adjacency.get(&start).cloned().unwrap_or_default();
                while let Some(id) = pending.pop() {
                    if seen.insert(id) {
                        pending.extend(adjacency.get(&id).into_iter().flatten().copied());
                    }
                }
                (start, seen)
            })
            .collect();
        let reaches = |a, b| closure.get(&a).is_some_and(|set| set.contains(&b));
        let mut paths = Vec::new();
        for (&a, targets) in &closure {
            for &b in targets {
                paths.push([
                    graph.systems.get(a).unwrap().system.name().as_string(),
                    graph.systems.get(b).unwrap().system.name().as_string(),
                ]);
            }
        }
        // Do not deduplicate equal-name paths: repeated instances carry multiplicity.
        paths.sort();
        let pairs = candidates();
        let mut coverage = Vec::new();
        let mut ordered = Vec::new();
        for pair in &pairs {
            let a = key(graph, &pair.producer);
            let b = key(graph, &pair.consumer);
            let raw =
                conflict(world, graph, a, b).expect("actual executor incompatibility remains");
            assert_eq!(
                raw.access,
                [
                    std::any::type_name::<project_phoenix::core::messages::AdmittedCommands>()
                        .to_owned()
                ],
                "a new access invalidates the exact family premise: {pair:?}"
            );
            let expected = crate::declared_order::before(&pair.producer, &pair.consumer);
            let (forward, reverse) = (reaches(a, b), reaches(b, a));
            assert_eq!(
                (forward, reverse),
                (expected, !expected),
                "frozen declared direction: {pair:?}"
            );
            if selected(pair) {
                coverage.push(json!({"pair":pair,"raw":raw,"incompatible":true,"producer_before":forward,"consumer_before":reverse}));
            } else {
                ordered.push(json!({"pair":pair,"raw":raw,"producer_before":forward,"consumer_before":reverse}));
            }
        }
        assert_eq!(coverage.len(), 5);
        assert_eq!(
            ordered.len(),
            270,
            "graph coverage does not claim behavioral coverage"
        );
        let selected_keys: HashSet<_> = std::iter::once(PRODUCER)
            .chain(CONSUMERS)
            .map(|name| key(graph, name))
            .collect();
        let mut raw_external = Vec::new();
        for (index, &a) in all.iter().enumerate() {
            for &b in &all[index + 1..] {
                if !selected_keys.contains(&a) && !selected_keys.contains(&b) {
                    continue;
                }
                if let Some(raw) = conflict(world, graph, a, b) {
                    if coverage
                        .iter()
                        .any(|row| row["raw"]["systems"] == json!(raw.systems))
                    {
                        continue;
                    }
                    raw_external.push(raw);
                }
            }
        }
        raw_external.sort();
        assert!(
            !raw_external.is_empty(),
            "external preservation is nonvacuous"
        );
        *self.result.lock().unwrap() = Some(json!({"order":self.order,
            "authorized_candidates":275,"behavioral_tranche":coverage,"uncovered":pairs.into_iter().filter(|p|!selected(p)).collect::<Vec<_>>(),
            "ordered_candidates":ordered,"raw_external":raw_external,"existing_paths":paths,"graph":crate::declared_order::capture(graph,dag,&self.order)}));
        Ok(())
    }
}

pub struct Proof(Arc<Mutex<Option<Value>>>);
pub fn install(app: &mut App, order: &str) -> Proof {
    assert!(matches!(order, "ordinary" | "shuffle-a" | "shuffle-b"));
    let result = Arc::new(Mutex::new(None));
    app.world_mut().schedule_scope(FixedUpdate, |_, schedule| {
        assert!(
            schedule.systems().is_err(),
            "install before first execution"
        );
        let graph = schedule.graph();
        let producer = type_set(graph, key(graph, PRODUCER));
        let consumers: Vec<_> = CONSUMERS
            .iter()
            .map(|name| type_set(graph, key(graph, name)))
            .collect();
        assert_eq!(consumers.len(), 5);
        let _ = producer;
        schedule.add_build_pass(Observe {
            order: order.to_owned(),
            result: result.clone(),
        });
    });
    Proof(result)
}

impl Proof {
    pub fn finish(self, app: &mut App) -> Value {
        let mut result = self
            .0
            .lock()
            .unwrap()
            .take()
            .expect("actual build pass ran");
        app.world_mut()
            .schedule_scope(FixedUpdate, |world, schedule| {
                let names: HashMap<_, _> = schedule
                    .systems()
                    .unwrap()
                    .map(|(id, system)| (id, system.name().as_string()))
                    .collect();
                let mut debt = Vec::new();
                for (a, b, access) in schedule.graph().conflicting_systems().iter() {
                    let mut systems = [names[a].clone(), names[b].clone()];
                    systems.sort();
                    let mut access: Vec<_> = access
                        .iter()
                        .map(|id| world.components().get_name(*id).unwrap().as_string())
                        .collect();
                    if access.is_empty() {
                        access.push("<exclusive World access>".into());
                    }
                    access.sort();
                    debt.push(Ambiguity { systems, access });
                }
                debt.sort();
                for row in result["behavioral_tranche"].as_array().unwrap() {
                    let count = debt.iter().filter(|d| json!(d) == row["raw"]).count();
                    assert_eq!(
                        count, 0,
                        "declared production order resolves this exact raw conflict"
                    );
                }
                result["reported_debt"] = json!(debt);
                schedule.remove_build_pass::<Observe>();
            });
        result
    }
}

pub fn assert_preserved(ordinary: &Value, forced: &Value) {
    assert_eq!(
        ordinary["raw_external"], forced["raw_external"],
        "every external raw vector and duplicate remains declared"
    );
    crate::declared_order::assert_graph(&ordinary["graph"], &forced["graph"]);
}
