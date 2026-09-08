//! Instance IDs are local to one capture, never ledger keys or cross-build identities.
use bevy::{
    ecs::schedule::{
        graph::{Dag, DiGraph},
        ApplyDeferred, NodeId, ScheduleBuildError, ScheduleBuildPass, ScheduleGraph, SystemKey,
        SystemSetKey,
    },
    platform::hash::FixedHasher,
    prelude::*,
};
use serde::Serialize;
use std::{
    any::TypeId,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Serialize)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub exclusive: bool,
    pub has_deferred: bool,
    pub apply_deferred: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Conflict {
    pub instances: [String; 2],
    pub access: Vec<String>,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Capture {
    pub nodes: Vec<Node>,
    /// Parent set -> member (system or nested set).
    pub hierarchy: Vec<[String; 2]>,
    /// Authored system/set dependency edges, before set expansion.
    pub dependency: Vec<[String; 2]>,
    /// After existing build passes, before redundant-edge removal. Same reachability
    /// as the executor graph; includes actual inserted ApplyDeferred instances.
    pub effective_dependency: Vec<[String; 2]>,
    pub conflicts: Vec<Conflict>,
}
fn id(node: NodeId) -> String {
    format!("{node:?}")
}

#[derive(Debug)]
struct Observe(Arc<Mutex<Option<Vec<[String; 2]>>>>);
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
        _: &mut World,
        _: &mut ScheduleGraph,
        graph: &mut Dag<SystemKey>,
    ) -> Result<(), ScheduleBuildError> {
        let mut edges: Vec<_> = graph
            .graph()
            .all_edges()
            .map(|(a, b)| [id(NodeId::System(a)), id(NodeId::System(b))])
            .collect();
        edges.sort();
        *self.0.lock().expect("diagnostic observer lock") = Some(edges);
        Ok(())
    }
}

/// Capture a fresh inspection App by ordinary initialization, without running it.
/// Bevy keeps executable edges private. An appended read-only build pass observes
/// the graph after its existing passes (including automatic deferred barriers).
/// Refuse an already initialized schedule rather than perturb it to force a rebuild.
/// No edge, system, condition, ambiguity policy or executor setting is changed.
pub fn fixed_update_graph(app: &mut App) -> Result<Capture, String> {
    app.world_mut()
        .try_schedule_scope(FixedUpdate, |world, schedule| {
            if schedule.systems().is_ok() {
                return Err("graph capture requires a fresh, uninitialized inspection App".into());
            }
            let observed = Arc::new(Mutex::new(None));
            schedule.add_build_pass(Observe(observed.clone()));
            let result = schedule.initialize(world).map_err(|e| format!("{e:?}"));
            schedule.remove_build_pass::<Observe>();
            result?;
            let effective_dependency = observed
                .lock()
                .map_err(|e| e.to_string())?
                .take()
                .ok_or("observer did not run")?;
            let mut nodes = Vec::new();
            for (key, system) in schedule.systems().map_err(|e| format!("{e:?}"))? {
                nodes.push(Node {
                    id: id(NodeId::System(key)),
                    name: system.name().as_string(),
                    kind: "system".into(),
                    exclusive: system.is_exclusive(),
                    has_deferred: system.has_deferred(),
                    apply_deferred: system.type_id() == TypeId::of::<ApplyDeferred>(),
                });
            }
            let graph = schedule.graph();
            for (key, set, _) in graph.system_sets.iter() {
                nodes.push(Node {
                    id: id(NodeId::Set(key)),
                    name: format!("{set:?}"),
                    kind: "set".into(),
                    exclusive: false,
                    has_deferred: false,
                    apply_deferred: false,
                });
            }
            nodes.sort_by(|a, b| a.id.cmp(&b.id));
            let mut hierarchy: Vec<_> = graph
                .hierarchy()
                .graph()
                .all_edges()
                .map(|(a, b)| [id(a), id(b)])
                .collect();
            let mut dependency: Vec<_> = graph
                .dependency()
                .graph()
                .all_edges()
                .map(|(a, b)| [id(a), id(b)])
                .collect();
            hierarchy.sort();
            dependency.sort();
            let mut conflicts = Vec::new();
            for (a, b, components) in graph.conflicting_systems().iter() {
                let mut instances = [id(NodeId::System(*a)), id(NodeId::System(*b))];
                instances.sort();
                let mut access = components
                    .iter()
                    .map(|key| {
                        world
                            .components()
                            .get_name(*key)
                            .map(|name| name.as_string())
                            .ok_or_else(|| format!("missing access {key:?}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if access.is_empty() {
                    access.push("<exclusive World access>".into());
                }
                access.sort();
                conflicts.push(Conflict { instances, access });
            }
            conflicts.sort_by(|a, b| a.instances.cmp(&b.instances).then(a.access.cmp(&b.access)));
            Ok(Capture {
                nodes,
                hierarchy,
                dependency,
                effective_dependency,
                conflicts,
            })
        })
        .map_err(|e| format!("{e:?}"))?
}
