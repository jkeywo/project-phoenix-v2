//! Optional read-only build-graph evidence, using the same ScheduleBuildPass
//! seam as the existing admitted-producer proofs. Never adds/removes an edge.
use bevy::{
    ecs::schedule::{
        graph::{Dag, DiGraph, DiGraphToposortError},
        NodeId, ScheduleBuildError, ScheduleBuildPass, ScheduleGraph, SystemKey, SystemSetKey,
    },
    platform::hash::FixedHasher,
    prelude::*,
};
use serde_json::json;

#[derive(Debug)]
struct Observe(std::path::PathBuf);
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
        graph: &mut ScheduleGraph,
        flattened: &mut Dag<SystemKey>,
    ) -> Result<(), ScheduleBuildError> {
        let name = |key| graph.systems.get(key).unwrap().system.name().as_string();
        let components = match flattened.graph().toposort(Vec::new()) {
            Ok(_) => Vec::new(),
            Err(DiGraphToposortError::Loop(key)) => vec![vec![key]],
            Err(DiGraphToposortError::Cycle(components)) => components,
        };
        let cycles: Vec<_> = components.iter().map(|component| {
            let mut edges: Vec<_> = flattened.graph().all_edges()
                .filter(|(a,b)| component.contains(a) && component.contains(b))
                .map(|(a,b)| [name(a),name(b)]).collect();
            edges.sort();
            json!({"members":component.iter().copied().map(name).collect::<Vec<_>>(),"edges":edges})
        }).collect();
        let evidence = json!({"cycles":cycles,"nodes":graph.systems.len(),
            "edges":flattened.graph().all_edges().count(),
            "named_edges":flattened.graph().all_edges().map(|(a,b)| json!({
                "from":format!("{a:?}"),"from_name":name(a),
                "to":format!("{b:?}"),"to_name":name(b)
            })).collect::<Vec<_>>()});
        std::fs::write(&self.0, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
        Ok(())
    }
}
pub fn install(app: &mut App) {
    if let Some(path) = std::env::var_os("PHOENIX_PDC_GRAPH_DIAGNOSTICS") {
        app.world_mut().schedule_scope(FixedUpdate, |_, schedule| {
            schedule.add_build_pass(Observe(path.into()));
        });
    }
}
