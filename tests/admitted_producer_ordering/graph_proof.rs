//! Reuse actual production instances/access metadata; add only empty test ordering sets.
use bevy::{
    ecs::{
        component::ComponentId,
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
use project_phoenix::headless::determinism_audit::Ambiguity;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    any::TypeId,
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

pub const PRODUCERS: [&str; 4] = [
    "project_phoenix::console_ai::server::ai_power_allocation",
    "project_phoenix::console_ai::server::ai_shield_focus",
    "project_phoenix::console::navigation::server::operate_navigation_ai",
    "project_phoenix::console::repair::server::operate_repair_ai",
];
pub const CONSUMERS: [&str; 4] = [
    "project_phoenix::ship::power::handle_power_messages",
    "project_phoenix::ship::shields::handle_shields_messages",
    "project_phoenix::console::navigation::server::handle_navigation_waypoint",
    "project_phoenix::console::repair::dispatch::handle_dispatch_repair_team",
];
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Candidate {
    pub systems: [String; 2],
    pub access: Vec<String>,
}
#[derive(Deserialize)]
struct Inventory {
    schema: u32,
    pairs: Vec<Candidate>,
}
pub fn candidates() -> Vec<Candidate> {
    let inventory: Inventory = serde_json::from_str(include_str!(
        "../fixtures/determinism/admitted-producer-six.json"
    ))
    .unwrap();
    assert_eq!(inventory.schema, 1);
    let mut expected = Vec::new();
    for (i, a) in PRODUCERS.iter().enumerate() {
        for b in &PRODUCERS[i + 1..] {
            let mut systems = [a.to_string(), b.to_string()];
            systems.sort();
            expected.push(Candidate {
                systems,
                access: vec![std::any::type_name::<
                    project_phoenix::core::messages::AdmittedCommands,
                >()
                .to_owned()],
            });
        }
    }
    expected.sort();
    let mut actual = inventory.pairs;
    actual.sort();
    assert_eq!(
        actual, expected,
        "six exact pairs including multiplicity and full access"
    );
    actual
}

pub fn key(graph: &ScheduleGraph, name: &str) -> SystemKey {
    let matches: Vec<_> = graph
        .systems
        .iter()
        .filter(|(_, system, _)| system.name().as_string() == name)
        .map(|(key, _, _)| key)
        .collect();
    assert_eq!(matches.len(), 1, "exact actual instance: {name}");
    matches[0]
}

pub fn type_set(graph: &ScheduleGraph, id: SystemKey) -> InternedSystemSet {
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

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
struct Between(usize);

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
        // Identify only the eight actual observation systems; never classify
        // production by a module/name prefix. Their declared function names
        // come from the probe module that installed them.
        let admission = [
            key(
                graph,
                "project_phoenix::command_admission::admit_system_commands",
            ),
            key(
                graph,
                "project_phoenix::command_admission::clear_inter_system_queue",
            ),
        ];
        let mut probe_keys = HashSet::new();
        let mut probe_boundaries = Vec::new();
        for (i, (before_name, after_name)) in super::probes::names().into_iter().enumerate() {
            let before = key(graph, before_name);
            let producer = key(graph, PRODUCERS[i]);
            let after = key(graph, after_name);
            for id in [before, after] {
                assert!(
                    probe_keys.insert(id),
                    "eight distinct actual probe instances"
                );
                let system = &graph.systems.get(id).unwrap().system;
                assert!(!system.is_exclusive() && !system.has_deferred());
                assert_ne!(system.type_id(), TypeId::of::<ApplyDeferred>());
            }
            assert!(
                admission.iter().all(|&id| reaches(id, before)),
                "actual Admission seam must precede every before-probe"
            );
            assert!(
                reaches(before, producer) && reaches(producer, after),
                "actual before/producer/after boundary"
            );
            probe_boundaries.push(json!({"producer":PRODUCERS[i],
                "admission":admission.map(|id|format!("{id:?}")),
                "before":format!("{before:?}"),"producer_id":format!("{producer:?}"),
                "after":format!("{after:?}")}));
        }
        assert_eq!(probe_keys.len(), 8);
        let deferred: HashSet<_> = all
            .iter()
            .copied()
            .filter(|&id| {
                graph.systems.get(id).unwrap().system.type_id() == TypeId::of::<ApplyDeferred>()
            })
            .collect();
        let production: Vec<_> = all
            .iter()
            .copied()
            .filter(|id| !probe_keys.contains(id) && !deferred.contains(id))
            .collect();
        assert!(!production.is_empty() && !deferred.is_empty());
        let mut instances: Vec<_> = all
            .iter()
            .map(|&id| {
                let system = &graph.systems.get(id).unwrap().system;
                json!({"id":format!("{id:?}"),"name":system.name().as_string(),
                "exclusive":system.is_exclusive(),"has_deferred":system.has_deferred(),
                "apply_deferred":deferred.contains(&id),"probe":probe_keys.contains(&id)})
            })
            .collect();
        instances.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        let production_instances: Vec<_> = instances
            .iter()
            .filter(|row| row["probe"] == false && row["apply_deferred"] == false)
            .cloned()
            .collect();
        assert_eq!(production_instances.len(), production.len());
        let mut effective_edges: Vec<_> = dag
            .graph()
            .all_edges()
            .map(|(a, b)| [format!("{a:?}"), format!("{b:?}")])
            .collect();
        effective_edges.sort();
        let mut production_paths = Vec::new();
        let mut deferred_visibility = Vec::new();
        for &a in &production {
            for &b in &production {
                if reaches(a, b) {
                    production_paths.push([format!("{a:?}"), format!("{b:?}")]);
                }
                if graph.systems.get(a).unwrap().system.has_deferred()
                    && deferred
                        .iter()
                        .any(|&barrier| reaches(a, barrier) && reaches(barrier, b))
                {
                    // Existential publication obligation: the actual deferred
                    // writer reaches a barrier that reaches this actual sink.
                    // Which automatic barrier provides it may legitimately vary.
                    deferred_visibility.push([format!("{a:?}"), format!("{b:?}")]);
                }
            }
        }
        production_paths.sort();
        deferred_visibility.sort();
        assert!(
            !production_paths.is_empty(),
            "nonempty production path coverage"
        );
        assert!(
            !deferred_visibility.is_empty(),
            "nonempty deferred writer/sink coverage"
        );
        let pairs = candidates();
        let mut coverage = Vec::new();
        for pair in &pairs {
            let a = key(graph, &pair.systems[0]);
            let b = key(graph, &pair.systems[1]);
            let raw =
                conflict(world, graph, a, b).expect("actual executor incompatibility remains");
            assert_eq!(raw.systems, pair.systems);
            assert_eq!(
                raw.access, pair.access,
                "new shared access invalidates this bounded premise"
            );
            let (forward, reverse) = (reaches(a, b), reaches(b, a));
            let ai = PRODUCERS
                .iter()
                .position(|n| *n == pair.systems[0])
                .unwrap();
            let bi = PRODUCERS
                .iter()
                .position(|n| *n == pair.systems[1])
                .unwrap();
            match self.order.as_str() {
                "ordinary" => assert!(!forward && !reverse, "new baseline edge: {pair:?}"),
                "forward" => assert_eq!((forward, reverse), (ai < bi, ai > bi)),
                "reverse" => assert_eq!((forward, reverse), (ai > bi, ai < bi)),
                _ => unreachable!(),
            }
            coverage.push(json!({"pair":pair,"raw":raw,"incompatible":true,"forward":forward,"reverse":reverse}));
        }
        assert_eq!(coverage.len(), 6);
        for (producer, consumer) in PRODUCERS.iter().zip(CONSUMERS) {
            assert!(
                reaches(key(graph, producer), key(graph, consumer)),
                "ordinary producer/consumer path: {producer} -> {consumer}"
            );
        }
        let selected_keys: HashSet<_> = PRODUCERS.iter().map(|name| key(graph, name)).collect();
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
            "authorized_candidates":6,"behavioral_tranche":coverage,
            "raw_external":raw_external,"existing_paths":paths,
            "instances":instances,"effective_edges":effective_edges,
            "production_instances":production_instances,"production_paths":production_paths,
            "deferred_visibility":deferred_visibility,"probe_boundaries":probe_boundaries}));
        Ok(())
    }
}

pub struct Proof(Arc<Mutex<Option<Value>>>);
pub fn install(app: &mut App, order: &str) -> Proof {
    assert!(matches!(order, "ordinary" | "forward" | "reverse"));
    let result = Arc::new(Mutex::new(None));
    app.world_mut().schedule_scope(FixedUpdate, |_, schedule| {
        assert!(
            schedule.systems().is_err(),
            "install before first execution"
        );
        let graph = schedule.graph();
        let mut producers: Vec<_> = PRODUCERS
            .iter()
            .map(|name| type_set(graph, key(graph, name)))
            .collect();
        if order == "reverse" {
            producers.reverse();
        }
        if order != "ordinary" {
            for (i, pair) in producers.windows(2).enumerate() {
                schedule.configure_sets(Between(i).after(pair[0]).before(pair[1]));
            }
        }
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
                        count,
                        usize::from(result["order"] == "ordinary"),
                        "no production annotation is added by this baseline fixture"
                    );
                }
                result["reported_debt"] = json!(debt);
                schedule.remove_build_pass::<Observe>();
            });
        result
    }
}

pub fn assert_preserved(ordinary: &Value, forced: &Value) -> Value {
    assert_eq!(
        ordinary["raw_external"], forced["raw_external"],
        "every external raw vector and duplicate remains declared"
    );
    fn multiset(value: &Value) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for row in value.as_array().unwrap() {
            *counts.entry(row.to_string()).or_default() += 1;
        }
        counts
    }
    // IDs are used only after the complete same-source production inventory,
    // including names and flags, has matched. This is not a cross-build ID law.
    assert_eq!(
        ordinary["production_instances"], forced["production_instances"],
        "same-source production instance IDs/names/access flags must match before comparing paths"
    );
    assert!(!ordinary["production_instances"]
        .as_array()
        .unwrap()
        .is_empty());
    for field in ["production_paths", "deferred_visibility"] {
        let baseline = multiset(&ordinary[field]);
        assert!(!baseline.is_empty(), "nonempty {field} obligations");
        let actual = multiset(&forced[field]);
        for (path, count) in baseline {
            assert!(
                actual.get(&path).copied().unwrap_or_default() >= count,
                "existing {field} obligation disappeared: {path}"
            );
        }
    }
    // Keep and expose all displaced raw name paths, including their original
    // multiplicity, without claiming an incidental probe/barrier identity law.
    let paths = multiset(&forced["existing_paths"]);
    let displaced: Vec<_> = multiset(&ordinary["existing_paths"])
        .into_iter()
        .filter_map(|(path, count)| {
            let actual = paths.get(&path).copied().unwrap_or_default();
            (actual < count).then(|| {
                json!({
                "path":serde_json::from_str::<Value>(&path).unwrap(),
                "lost_multiplicity":count-actual})
            })
        })
        .collect();
    let inspection = json!({"ordinary_order":ordinary["order"],"compared_order":forced["order"],
        "displaced_raw_name_paths":displaced});
    eprintln!("PHOENIX_PRODUCER_DISPLACED_PATHS={inspection}");
    inspection
}
