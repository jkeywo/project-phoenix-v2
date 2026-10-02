//! Execution-only dependencies are added after Bevy has materialized the
//! existing deferred-application contract. Sending these edges through the
//! automatic flush pass can suppress an existing sync edge or move a flush.
use super::FixedStep;
use bevy::{
    ecs::schedule::{
        graph::{Dag, DiGraph},
        InternedSystemSet, NodeId, ScheduleBuildError, ScheduleBuildPass, ScheduleGraph, SystemKey,
        SystemSetKey,
    },
    platform::{collections::HashSet, hash::FixedHasher},
    prelude::*,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct Endpoint {
    set: InternedSystemSet,
    singleton: bool,
    optional: bool,
}

impl From<FixedStep> for Endpoint {
    fn from(owner: FixedStep) -> Self {
        Self {
            set: owner.intern(),
            singleton: true,
            optional: owner == FixedStep::HeadlessAutoStart,
        }
    }
}

macro_rules! boundary {
    ($($ty:ty),+ $(,)?) => {$(
        impl From<$ty> for Endpoint {
            fn from(set: $ty) -> Self {
                Self { set: set.intern(), singleton: false, optional: false }
            }
        }
    )+};
}
boundary!(
    crate::sim_sets::SimSet,
    crate::lobby::LobbySystemSet,
    crate::command_admission::AdmissionSet,
    crate::core::broadcast::ReconnectBoundary,
);

#[derive(Debug, Default)]
pub(super) struct DeclaredOrder {
    edges: Vec<(Endpoint, Endpoint)>,
}

impl DeclaredOrder {
    pub(super) fn before(&mut self, from: impl Into<Endpoint>, to: impl Into<Endpoint>) {
        self.edges.push((from.into(), to.into()));
    }
}

fn members(graph: &ScheduleGraph, endpoint: Endpoint) -> Vec<SystemKey> {
    let Some(key) = graph.system_sets.get_key(endpoint.set) else {
        assert!(
            endpoint.optional,
            "missing fixed-order owner {:?}",
            endpoint.set
        );
        return Vec::new();
    };
    // systems_in_set() intentionally refuses the still-dirty build graph.
    // Resolve typed membership directly, including nested phase sets.
    let hierarchy = graph.hierarchy().graph();
    let mut pending = vec![NodeId::Set(key)];
    let mut seen = HashSet::<NodeId>::default();
    let mut systems = Vec::new();
    while let Some(node) = pending.pop() {
        if !seen.insert(node) {
            continue;
        }
        match node {
            NodeId::System(system) => systems.push(system),
            NodeId::Set(_) => pending.extend(
                hierarchy
                    .all_edges()
                    .filter_map(|(parent, member)| (parent == node).then_some(member)),
            ),
        }
    }
    if endpoint.singleton {
        assert_eq!(systems.len(), 1, "fixed-order owner {:?}", endpoint.set);
    } else {
        assert!(
            !systems.is_empty(),
            "empty fixed-order boundary {:?}",
            endpoint.set
        );
    }
    systems
}

impl ScheduleBuildPass for DeclaredOrder {
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
        for &(from, to) in &self.edges {
            let before = members(graph, from);
            let after = members(graph, to);
            for &a in &before {
                for &b in &after {
                    assert_ne!(a, b, "overlapping fixed-order endpoints: {from:?}, {to:?}");
                    flattened.graph_mut().add_edge(a, b);
                }
            }
        }
        // Bevy performs its ordinary cycle analysis and ambiguity detection
        // after every build pass. No system, access or ambiguity is replaced.
        Ok(())
    }
}

#[cfg(test)]
#[path = "pass_tests.rs"]
mod tests;
