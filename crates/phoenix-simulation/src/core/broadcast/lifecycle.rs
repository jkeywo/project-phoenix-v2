//! Stable-keyed replication lifecycle registration.
//!
//! Live snapshot producers own the shape and visibility rules of their
//! replicas.  The same owner therefore registers the two lifecycle operations
//! that accompany a producer:
//!
//! - resetting any delta cache at the start of a run; and
//! - projecting current state for one reconnecting session.
//!
//! The runners below know neither cache resource types nor `ServerMessage`
//! variants.  They invoke adapters in lexical key order, so plugin insertion
//! order cannot change observable reconnect ordering.

use std::collections::BTreeMap;

use bevy::prelude::*;

/// Reset one producer's replication bookkeeping without changing source state.
pub type ResetReplication = fn(&mut World);

/// One owner's lifecycle hooks, identified by a stable semantic key.
#[derive(Clone, Copy)]
pub struct ReplicationLifecycleAdapter {
    key: &'static str,
    reset: Option<ResetReplication>,
    pub(super) reconnect: bool,
}

impl ReplicationLifecycleAdapter {
    /// Start an adapter declaration.  Add whichever hooks this owner needs.
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            reset: None,
            reconnect: false,
        }
    }

    /// Attach this owner's run-boundary cache reset.
    #[must_use]
    pub fn with_reset(mut self, reset: ResetReplication) -> Self {
        self.reset = Some(reset);
        self
    }
}

/// Build-time lifecycle catalogue, ordered by stable owner key.
#[derive(Resource, Default)]
pub struct ReplicationLifecycleRegistry {
    pub(super) adapters: BTreeMap<&'static str, ReplicationLifecycleAdapter>,
    pub(super) projector_types: std::collections::HashSet<std::any::TypeId>,
    pub(super) projections: BTreeMap<&'static str, super::reconnect::Projection>,
    pub(super) finalized: bool,
}

impl ReplicationLifecycleRegistry {
    fn register(&mut self, adapter: ReplicationLifecycleAdapter) {
        assert!(
            !self.finalized,
            "replication registration after reconnect finalization"
        );
        assert!(
            !adapter.key.trim().is_empty(),
            "replication lifecycle key must not be empty"
        );
        assert!(
            adapter.reset.is_some() || adapter.reconnect,
            "replication lifecycle '{}' has no reset or reconnect adapter",
            adapter.key
        );
        assert!(
            !self.adapters.contains_key(adapter.key),
            "duplicate replication lifecycle key '{}'",
            adapter.key
        );
        self.adapters.insert(adapter.key, adapter);
    }

    /// Registered owner keys in invocation order.
    pub fn keys(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.adapters.keys().copied()
    }

    /// Number of registered owners.
    pub fn len(&self) -> usize {
        self.adapters.len()
    }

    /// Whether no owner has registered yet.
    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }
}

/// App-build registration API used beside each live replication producer.
pub trait RegisterReplicationLifecycle {
    /// Register one stable-keyed lifecycle adapter.
    fn register_replication_lifecycle(&mut self, adapter: ReplicationLifecycleAdapter)
        -> &mut Self;
}

impl RegisterReplicationLifecycle for App {
    fn register_replication_lifecycle(
        &mut self,
        adapter: ReplicationLifecycleAdapter,
    ) -> &mut Self {
        if !self
            .world()
            .contains_resource::<ReplicationLifecycleRegistry>()
        {
            self.init_resource::<ReplicationLifecycleRegistry>();
        }
        // The registry is transport bookkeeping populated at app build,
        // analogous to `BroadcastRegistry<M>`; it never becomes a second copy
        // of authoritative simulation state. `declare_state` is idempotent, so
        // this remains correct if a test initialised the resource first.
        use crate::authoritative::{DeclareState, StateClass};
        self.declare_state::<ReplicationLifecycleRegistry>(
            StateClass::Cache,
            "digest-exclusion-classes",
        );
        self.world_mut()
            .resource_mut::<ReplicationLifecycleRegistry>()
            .register(adapter);
        self
    }
}

/// Invoke every registered reset adapter in stable key order.
pub fn reset_registered_replication(world: &mut World) {
    let resets: Vec<ResetReplication> = world
        .get_resource::<ReplicationLifecycleRegistry>()
        .map(|registry| {
            registry
                .adapters
                .values()
                .filter_map(|adapter| adapter.reset)
                .collect()
        })
        .unwrap_or_default();

    for reset in resets {
        reset(world);
    }
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
