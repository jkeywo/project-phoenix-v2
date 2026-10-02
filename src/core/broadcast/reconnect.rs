//! Coherent typed reconnect capture and canonical transport delivery.
//!
//! Every owner contributes concrete read-only parameters. One safely built
//! Bevy pipeline holds the complete collection/capture/delivery access union,
//! so no writer can interleave the request or owner observations.
use super::lifecycle::{ReplicationLifecycleAdapter, ReplicationLifecycleRegistry};
use crate::{core::messages::ServerMessage, lobby::Target, server_app::SimOutbox};
use bevy::{
    ecs::system::{
        DynParamBuilder, DynSystemParam, IntoSystem, ParamBuilder, PipeSystem, ReadOnlySystemParam,
        System, SystemParamBuilder,
    },
    prelude::*,
};
use std::any::TypeId;

#[derive(Resource, Default)]
pub struct ReconnectRequests(pub Vec<String>);
pub type ReconnectBatch = Vec<Vec<ServerMessage>>;
type Captured = Vec<(usize, &'static str, Vec<ServerMessage>)>;
type Project = for<'w, 's> fn(DynSystemParam<'w, 's>) -> ReconnectBatch;

#[derive(Clone, Copy)]
pub(super) struct Projection {
    // This type identity is bound at registration beside the only builder.
    param_type: TypeId,
    owner_type: TypeId,
    builder: fn() -> DynParamBuilder<'static>,
    project: Project,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReconnectBoundary;

struct ReconnectPlugin;
impl Plugin for ReconnectPlugin {
    fn build(&self, _: &mut App) {}
    fn finish(&self, app: &mut App) {
        finalize_reconnect_projections(app);
    }
}

/// Register an owner-local projection with concrete read-only parameters.
/// The caller cannot supply an unconstrained builder. Its dynamic parameter
/// can only downcast to P; a wrong downcast cannot obtain additional access.
/// Normal App::finish finalizes the cohort after all plugin builders finish.
pub fn register_reconnect_projection<O, P>(app: &mut App, key: &'static str, project: Project)
where
    O: Send + Sync + 'static,
    P: ReadOnlySystemParam + 'static,
{
    use crate::authoritative::{DeclareState, StateClass};
    app.init_resource::<ReplicationLifecycleRegistry>();
    let first = {
        let mut registry = app
            .world_mut()
            .resource_mut::<ReplicationLifecycleRegistry>();
        assert!(
            !registry.finalized,
            "reconnect registration after finalization"
        );
        assert!(
            !key.trim().is_empty(),
            "replication lifecycle key must not be empty"
        );
        assert!(
            !registry.projections.contains_key(key),
            "duplicate reconnect lifecycle key '{key}'"
        );
        assert!(
            registry.projector_types.insert(TypeId::of::<O>()),
            "duplicate reconnect owner marker"
        );
        let entry = registry
            .adapters
            .entry(key)
            .or_insert_with(|| ReplicationLifecycleAdapter::new(key));
        assert!(
            !entry.reconnect,
            "duplicate reconnect lifecycle key '{key}'"
        );
        entry.reconnect = true;
        let first = registry.projections.is_empty();
        registry.projections.insert(
            key,
            Projection {
                param_type: TypeId::of::<P>(),
                owner_type: TypeId::of::<O>(),
                builder: || DynParamBuilder::new::<P>(ParamBuilder),
                project,
            },
        );
        first
    };
    app.init_resource::<ReconnectRequests>()
        .declare_state::<ReplicationLifecycleRegistry>(
            StateClass::Cache,
            "digest-exclusion-classes",
        )
        .declare_state_alias::<ReconnectRequests, ReplicationLifecycleRegistry>();
    if first {
        app.add_plugins(ReconnectPlugin);
    }
}

fn owners(world: &World) -> Vec<(&'static str, Projection)> {
    world
        .get_resource::<ReplicationLifecycleRegistry>()
        .map(|registry| {
            registry
                .projections
                .iter()
                .map(|(key, p)| (*key, *p))
                .collect()
        })
        .unwrap_or_default()
}

fn build_capture(
    world: &mut World,
    owners: Vec<(&'static str, Projection)>,
) -> impl System<In = (), Out = Captured> {
    let builders: Vec<_> = owners.iter().map(|(_, p)| (p.builder)()).collect();
    (builders, ParamBuilder::resource::<ReconnectRequests>()).build_state(world)
        .build_any_system(move |params: Vec<DynSystemParam>, requests: Res<ReconnectRequests>| {
            assert_eq!(params.len(), owners.len(), "one concrete parameter per registered owner");
            let mut captured = Vec::new();
            if requests.0.is_empty() { return captured; }
            for (param, (key, owner)) in params.into_iter().zip(&owners) {
                let rows = (owner.project)(param);
                assert_eq!(rows.len(), requests.0.len(), "projector must answer each request ordinal, including empty replies: {key} ({:?})", owner.param_type);
                for (ordinal, messages) in rows.into_iter().enumerate() {
                    captured.push((ordinal, *key, messages));
                }
            }
            captured
        })
}

/// Automatic from the private plugin's finish hook. Bare App harnesses may
/// call this explicitly before their first schedule run; later owners refuse.
pub fn finalize_reconnect_projections(app: &mut App) {
    let owners = owners(app.world());
    if owners.is_empty() {
        return;
    }
    {
        let mut registry = app
            .world_mut()
            .resource_mut::<ReplicationLifecycleRegistry>();
        if registry.finalized {
            return;
        }
        assert_eq!(
            registry.projector_types.len(),
            owners.len(),
            "one marker per owner"
        );
        for (_, owner) in &owners {
            assert!(
                registry.projector_types.contains(&owner.owner_type),
                "owner marker binding must survive finalization"
            );
        }
        registry.finalized = true;
    }
    let capture = build_capture(app.world_mut(), owners);
    let collector = IntoSystem::into_system(crate::server_app::refresh_caches_on_midgame_reconnect);
    let name = collector.name();
    let capture_delivery = IntoSystem::into_system(capture.pipe(deliver));
    // One schedule node preserves the old callback body's indivisible request
    // collection, source observation and delivery. PipeSystem safely unions
    // the actual inner access sets; retaining the collector's logical name is
    // not a diagnostic alias or permission to ignore a different system.
    app.add_systems(
        FixedUpdate,
        PipeSystem::new(collector, capture_delivery, name)
            .in_set(ReconnectBoundary)
            .in_set(crate::sim_sets::FixedStep::RefreshCachesOnMidgameReconnect),
    );
}

fn deliver(
    In(mut captured): In<Captured>,
    mut requests: ResMut<ReconnectRequests>,
    mut outbox: Option<ResMut<SimOutbox>>,
) {
    captured.sort_by_key(|(ordinal, key, _)| (*ordinal, *key));
    let mut previous = None;
    for (ordinal, key, messages) in captured {
        assert_ne!(
            previous,
            Some((ordinal, key)),
            "duplicate reconnect output owner/ordinal"
        );
        previous = Some((ordinal, key));
        let token = requests
            .0
            .get(ordinal)
            .expect("projection ordinal belongs to this request batch");
        if !messages.is_empty() {
            outbox
                .as_mut()
                .expect("reconnect delivery requires SimOutbox")
                .extend_snapshot(
                    messages
                        .into_iter()
                        .map(|message| (Target::Token(token.clone()), message)),
                );
        }
    }
    requests.0.clear();
}

/// Unit-only adapter over the same coherent capture builder; no StoredSystem
/// entities, arbitrary World callback or live-cache reset is installed.
#[cfg(test)]
pub fn reconnect_registered_replication(world: &mut World, token: &str) -> Vec<ServerMessage> {
    let owners = owners(world);
    world.init_resource::<ReconnectRequests>();
    world.resource_mut::<ReconnectRequests>().0 = vec![token.into()];
    let mut capture = build_capture(world, owners);
    let _access = capture.initialize(world);
    let mut captured = capture.run((), world).expect("typed reconnect capture");
    captured.sort_by_key(|(ordinal, key, _)| (*ordinal, *key));
    world.resource_mut::<ReconnectRequests>().0.clear();
    captured
        .into_iter()
        .flat_map(|(_, _, messages)| messages)
        .collect()
}
#[cfg(test)]
pub fn resync_registered_replication_for_token(world: &mut World, token: &str) {
    let messages = reconnect_registered_replication(world, token);
    if messages.is_empty() {
        return;
    }
    world.resource_mut::<SimOutbox>().extend_snapshot(
        messages
            .into_iter()
            .map(|m| (Target::Token(token.into()), m)),
    );
}

/// Schedule fixtures enter through the collector's real inputs, not scratch.
#[cfg(test)]
pub(crate) fn test_reconnect_welcomes(app: &mut App, tokens: &[&str]) {
    use crate::core::messages::{GamePhase, GameState};
    use crate::lobby::server::LobbyOutbox;
    app.insert_resource(State::new(GamePhase::InProgress));
    app.init_resource::<LobbyOutbox>();
    app.world_mut().resource_mut::<LobbyOutbox>().0 = tokens
        .iter()
        .map(|token| {
            (
                Target::Token((*token).into()),
                ServerMessage::Welcome {
                    state: GameState {
                        phase: GamePhase::InProgress,
                        players: vec![],
                        world: None,
                    },
                    ship_stations: crate::lobby::stations_config::ShipStations { stations: vec![] },
                    ship_config: Default::default(),
                    station_ratings: Default::default(),
                    gms: vec![],
                    string_catalogues: vec![],
                },
            )
        })
        .collect();
}
#[cfg(test)]
#[path = "reconnect_tests.rs"]
mod tests;
