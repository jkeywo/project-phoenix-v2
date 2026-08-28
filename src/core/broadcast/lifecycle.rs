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

use crate::core::messages::ServerMessage;

/// Reset one producer's replication bookkeeping without changing source state.
pub type ResetReplication = fn(&mut World);

/// Build one producer's current permitted projection for `token`.
///
/// The caller supplies routing and delivery after the generic runner has
/// collected every adapter; an adapter owns only its source-state query and
/// payload shape.
pub type ReconnectProjection = fn(&mut World, &str) -> Vec<ServerMessage>;

/// One owner's lifecycle hooks, identified by a stable semantic key.
#[derive(Clone, Copy)]
pub struct ReplicationLifecycleAdapter {
    key: &'static str,
    reset: Option<ResetReplication>,
    reconnect: Option<ReconnectProjection>,
}

impl ReplicationLifecycleAdapter {
    /// Start an adapter declaration.  Add whichever hooks this owner needs.
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            reset: None,
            reconnect: None,
        }
    }

    /// Attach this owner's run-boundary cache reset.
    #[must_use]
    pub fn with_reset(mut self, reset: ResetReplication) -> Self {
        self.reset = Some(reset);
        self
    }

    /// Attach this owner's targeted reconnect projection.
    #[must_use]
    pub fn with_reconnect(mut self, reconnect: ReconnectProjection) -> Self {
        self.reconnect = Some(reconnect);
        self
    }
}

/// Build-time lifecycle catalogue, ordered by stable owner key.
#[derive(Resource, Default)]
pub struct ReplicationLifecycleRegistry {
    adapters: BTreeMap<&'static str, ReplicationLifecycleAdapter>,
}

impl ReplicationLifecycleRegistry {
    fn register(&mut self, adapter: ReplicationLifecycleAdapter) {
        assert!(
            !adapter.key.trim().is_empty(),
            "replication lifecycle key must not be empty"
        );
        assert!(
            adapter.reset.is_some() || adapter.reconnect.is_some(),
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

/// Build every registered reconnect projection in stable key order.
pub fn reconnect_registered_replication(world: &mut World, token: &str) -> Vec<ServerMessage> {
    let projectors: Vec<ReconnectProjection> = world
        .get_resource::<ReplicationLifecycleRegistry>()
        .map(|registry| {
            registry
                .adapters
                .values()
                .filter_map(|adapter| adapter.reconnect)
                .collect()
        })
        .unwrap_or_default();

    projectors
        .into_iter()
        .flat_map(|project| project(world, token))
        .collect()
}

/// Route every registered reconnect projection to one session as snapshots.
///
/// This is the only generic reconnect delivery seam. It knows the target token
/// and delivery class, but no owner cache types, source components, audience
/// policies, or `ServerMessage` variants.
pub fn resync_registered_replication_for_token(world: &mut World, token: &str) {
    let messages = reconnect_registered_replication(world, token);
    if messages.is_empty() {
        return;
    }

    let target = crate::lobby::Target::Token(token.to_string());
    world
        .resource_mut::<crate::server_app::SimOutbox>()
        .extend_snapshot(
            messages
                .into_iter()
                .map(|message| (target.clone(), message)),
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default, Debug, PartialEq, Eq)]
    struct Trace(Vec<String>);

    fn reset_alpha(world: &mut World) {
        world.resource_mut::<Trace>().0.push("reset:alpha".into());
    }

    fn reset_zulu(world: &mut World) {
        world.resource_mut::<Trace>().0.push("reset:zulu".into());
    }

    fn reconnect_alpha(world: &mut World, token: &str) -> Vec<ServerMessage> {
        world
            .resource_mut::<Trace>()
            .0
            .push(format!("reconnect:alpha:{token}"));
        vec![ServerMessage::GameStarted]
    }

    fn reconnect_zulu(world: &mut World, token: &str) -> Vec<ServerMessage> {
        world
            .resource_mut::<Trace>()
            .0
            .push(format!("reconnect:zulu:{token}"));
        vec![ServerMessage::GameStarted]
    }

    #[test]
    fn runners_use_key_order_not_registration_order() {
        let mut app = App::new();
        app.init_resource::<Trace>();
        app.register_replication_lifecycle(
            ReplicationLifecycleAdapter::new("zulu")
                .with_reset(reset_zulu)
                .with_reconnect(reconnect_zulu),
        );
        app.register_replication_lifecycle(
            ReplicationLifecycleAdapter::new("alpha")
                .with_reset(reset_alpha)
                .with_reconnect(reconnect_alpha),
        );

        assert_eq!(
            app.world()
                .resource::<ReplicationLifecycleRegistry>()
                .keys()
                .collect::<Vec<_>>(),
            vec!["alpha", "zulu"]
        );

        reset_registered_replication(app.world_mut());
        let projections = reconnect_registered_replication(app.world_mut(), "crew-token");

        assert_eq!(projections.len(), 2);
        assert_eq!(
            app.world().resource::<Trace>().0,
            vec![
                "reset:alpha",
                "reset:zulu",
                "reconnect:alpha:crew-token",
                "reconnect:zulu:crew-token",
            ]
        );
    }

    #[test]
    fn owners_may_register_only_the_lifecycle_operations_they_have() {
        let mut app = App::new();
        app.init_resource::<Trace>();
        app.register_replication_lifecycle(
            ReplicationLifecycleAdapter::new("reset-only").with_reset(reset_alpha),
        );
        app.register_replication_lifecycle(
            ReplicationLifecycleAdapter::new("reconnect-only").with_reconnect(reconnect_zulu),
        );

        reset_registered_replication(app.world_mut());
        let projections = reconnect_registered_replication(app.world_mut(), "one");

        assert_eq!(projections.len(), 1);
        assert_eq!(
            app.world().resource::<Trace>().0,
            vec!["reset:alpha", "reconnect:zulu:one"]
        );
    }

    #[test]
    #[should_panic(expected = "duplicate replication lifecycle key 'same'")]
    fn duplicate_owner_keys_are_rejected() {
        let mut app = App::new();
        app.register_replication_lifecycle(
            ReplicationLifecycleAdapter::new("same").with_reset(reset_alpha),
        );
        app.register_replication_lifecycle(
            ReplicationLifecycleAdapter::new("same").with_reset(reset_zulu),
        );
    }
}
