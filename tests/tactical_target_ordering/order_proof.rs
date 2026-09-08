//! Declared-order stability of the existing Tactical instances across registrations.
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

const SYSTEMS: [&str; 4] = [
    "project_phoenix::console::weapons::server::ai_target_selection",
    "project_phoenix::console::weapons::beam::handle_set_target",
    "project_phoenix::console::weapons::beam::ai_phaser_auto_fire",
    "project_phoenix::console::weapons::blaster::tick_blaster_auto_fire",
];
const PAIRS: [(usize, usize); 5] = [(0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];

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
        assert!(
            reachable(&edges, ids[0], ids[1]),
            "selection must precede its sole applier"
        );
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
                panic!("new unbounded overlap invalidates the commutativity proof");
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
            let mut expected = vec![std::any::type_name::<
                project_phoenix::core::messages::AdmittedCommands,
            >()
            .to_owned()];
            if a == 0 {
                expected.push(
                    std::any::type_name::<project_phoenix::server_app::ShipSystemBlackboards>()
                        .to_owned(),
                );
            }
            expected.sort();
            assert_eq!(
                access, expected,
                "new overlap requires a new logical proof: {} / {}",
                SYSTEMS[a], SYSTEMS[b]
            );
            let forward = reachable(&edges, ids[a], ids[b]);
            let reverse = reachable(&edges, ids[b], ids[a]);
            let expected = crate::declared_order::before(SYSTEMS[a], SYSTEMS[b]);
            assert_eq!(
                (forward, reverse),
                (expected, !expected),
                "the actual declared path follows the frozen owner order"
            );
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
        *self.result.lock().unwrap() = Some(
            json!({"order":self.order,"unique_instances":4,"pairs":pairs,"selection_before_applier":true,"external_conflicts":external,"raw_external":raw_external,"graph":crate::declared_order::capture(graph,dag,&self.order)}),
        );
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
        assert_eq!(sets.len(), SYSTEMS.len());
        // Registration changes are selected before constructing the real App.
        // This observer adds no edges, systems, access or run conditions.
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
                // These physical conflicts have declared paths and are absent from the census.
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
                        "the five annotated pairs must remain absent from the ambiguity census"
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

pub fn per_target(report: &Value) -> Value {
    json!(report["orders"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let mut commands = std::collections::BTreeMap::<String, Vec<Value>>::new();
            for command in row["commands"].as_array().unwrap() {
                commands
                    .entry(command["target"].as_str().unwrap().to_owned())
                    .or_default()
                    .push(command.clone());
            }
            json!({"tick":row["tick"],"commands":commands})
        })
        .collect::<Vec<_>>())
}

#[test]
fn declared_tactical_order_preserves_every_target_subsequence_and_gameplay_tick() {
    for case in ["retarget", "clear", "death-reacquire"] {
        let mut reference: Option<Value> = None;
        let mut registration_roles = std::collections::BTreeMap::new();
        for order in ["ordinary", "shuffle-a", "shuffle-b"] {
            for role in ["default-1", "default-2", "pinned"] {
                let output = std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "tactical_target_baseline_child",
                        "--ignored",
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env("PHOENIX_TACTICAL_BASELINE_ROLE", role)
                    .env("PHOENIX_TACTICAL_BASELINE_CASE", case)
                    .env("PHOENIX_TACTICAL_TEST_ORDER", order)
                    .current_dir(env!("CARGO_MANIFEST_DIR"))
                    .output()
                    .unwrap();
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert!(
                    output.status.success(),
                    "{case}/{order}/{role}: {}\n{stdout}\n{}",
                    output.status,
                    String::from_utf8_lossy(&output.stderr)
                );
                let rows: Vec<_> = stdout
                    .lines()
                    .filter_map(|line| line.strip_prefix(super::PREFIX))
                    .collect();
                assert_eq!(rows.len(), 1);
                let report: Value = serde_json::from_str(rows[0]).unwrap();
                assert_eq!(report["role"], role);
                assert_eq!(report["case"], case);
                assert_eq!(report["order_proof"]["order"], order);
                println!("\nPHOENIX_TACTICAL_REGISTRATION={}", rows[0]);
                assert_eq!(report["seed"], 140013);
                assert_ne!(report["process"], std::process::id());
                assert!(report["process"].as_u64().unwrap() > 0);
                if role == "pinned" {
                    assert_eq!(report["compute_threads"], 1);
                    assert_eq!(report["fixed_update_executor"], "SingleThreaded");
                } else {
                    assert!(report["compute_threads"].as_u64().unwrap() > 1);
                    assert_eq!(report["fixed_update_executor"], "MultiThreaded");
                }
                crate::declared_order::observe_role(
                    &mut registration_roles,
                    &report["order_proof"]["graph"],
                );
                if let Some(first) = &reference {
                    crate::declared_order::assert_graph(
                        &first["order_proof"]["graph"],
                        &report["order_proof"]["graph"],
                    );
                    assert_eq!(
                        first["order_proof"]["raw_external"], report["order_proof"]["raw_external"],
                        "all raw external conflicts and multiplicities remain visible"
                    );
                    assert_eq!(
                        first["ticks"], report["ticks"],
                        "{case}/{order}/{role}: actual gameplay differs"
                    );
                    assert_eq!(
                        per_target(first),
                        per_target(&report),
                        "{case}/{order}/{role}: target subsequence differs"
                    );
                } else {
                    reference = Some(report);
                }
            }
        }
    }
}
