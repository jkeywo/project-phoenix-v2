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
        PipeSystem::new(collector, capture_delivery, name).in_set(ReconnectBoundary),
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
                },
            )
        })
        .collect();
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::DeliveryClass;
    struct Alpha;
    struct Zulu;
    fn alpha(requests: Res<ReconnectRequests>) -> ReconnectBatch {
        requests
            .0
            .iter()
            .map(|token| {
                if token == "hidden" {
                    vec![]
                } else {
                    vec![
                        ServerMessage::GameStartCountdown { remaining_secs: 1 },
                        ServerMessage::GameStarted,
                    ]
                }
            })
            .collect()
    }
    fn zulu(requests: Res<ReconnectRequests>) -> ReconnectBatch {
        requests
            .0
            .iter()
            .map(|_| vec![ServerMessage::GameStartCountdown { remaining_secs: 9 }])
            .collect()
    }
    fn app(reverse: bool) -> App {
        let mut app = App::new();
        app.init_resource::<SimOutbox>();
        test_reconnect_welcomes(&mut app, &[]);
        if reverse {
            register_reconnect_projection::<Zulu, Res<'static, ReconnectRequests>>(
                &mut app,
                "zulu",
                |p| zulu(p.downcast::<Res<ReconnectRequests>>().unwrap()),
            );
            register_reconnect_projection::<Alpha, Res<'static, ReconnectRequests>>(
                &mut app,
                "alpha",
                |p| alpha(p.downcast::<Res<ReconnectRequests>>().unwrap()),
            );
        } else {
            register_reconnect_projection::<Alpha, Res<'static, ReconnectRequests>>(
                &mut app,
                "alpha",
                |p| alpha(p.downcast::<Res<ReconnectRequests>>().unwrap()),
            );
            register_reconnect_projection::<Zulu, Res<'static, ReconnectRequests>>(
                &mut app,
                "zulu",
                |p| zulu(p.downcast::<Res<ReconnectRequests>>().unwrap()),
            );
        }
        app
    }
    #[test]
    fn scheduled_delivery_retains_occurrences_owner_privacy_and_payload_order() {
        let mut observations = Vec::new();
        for reverse in [false, true] {
            let mut app = app(reverse);
            app.finish();
            test_reconnect_welcomes(&mut app, &["second", "hidden", "second", "first"]);
            // Neither a broadcast Welcome nor a different targeted message is
            // a reconnect request. Keep them adjacent to real occurrences.
            let welcome = app
                .world()
                .resource::<crate::lobby::server::LobbyOutbox>()
                .0[0]
                .1
                .clone();
            app.world_mut()
                .resource_mut::<crate::lobby::server::LobbyOutbox>()
                .0
                .extend([
                    (Target::All, welcome),
                    (Target::Token("ignored".into()), ServerMessage::GameStarted),
                ]);
            app.world_mut().run_schedule(FixedUpdate);
            let output = app.world_mut().resource_mut::<SimOutbox>().drain();
            let mut rows = Vec::new();
            for row in output {
                assert_eq!(row.delivery, DeliveryClass::Snapshot);
                let Target::Token(token) = row.target else {
                    panic!("private target required")
                };
                let value = match row.message {
                    ServerMessage::GameStartCountdown { remaining_secs } => remaining_secs,
                    ServerMessage::GameStarted => 2,
                    _ => panic!("unexpected owner payload"),
                };
                rows.push((token, value));
            }
            assert_eq!(
                rows,
                vec![
                    ("second".into(), 1),
                    ("second".into(), 2),
                    ("second".into(), 9),
                    ("hidden".into(), 9),
                    ("second".into(), 1),
                    ("second".into(), 2),
                    ("second".into(), 9),
                    ("first".into(), 1),
                    ("first".into(), 2),
                    ("first".into(), 9)
                ]
            );
            assert!(app.world().resource::<ReconnectRequests>().0.is_empty());
            app.world_mut()
                .resource_mut::<crate::lobby::server::LobbyOutbox>()
                .0
                .clear();
            app.world_mut().run_schedule(FixedUpdate);
            assert!(
                app.world_mut()
                    .resource_mut::<SimOutbox>()
                    .drain()
                    .is_empty(),
                "scratch must not replay stale requests"
            );
            observations.push(rows);
        }
        assert_eq!(observations[0], observations[1]);
    }
    #[test]
    fn initialized_reconnect_pipeline_contains_no_exclusive_system() {
        let mut app = app(false);
        app.finish();
        app.world_mut().run_schedule(FixedUpdate);
        let schedules = app.world().resource::<bevy::ecs::schedule::Schedules>();
        let systems: Vec<_> = schedules
            .get(FixedUpdate)
            .unwrap()
            .systems()
            .unwrap()
            .collect();
        assert_eq!(systems.len(), 1, "one collector/capture/delivery node");
        for (_, system) in systems {
            assert_eq!(system.name().to_string(),
                "project_phoenix::server_app::broadcast_publish::refresh_caches_on_midgame_reconnect",
                "the single production node retains the exact old full logical name");
            assert!(!system.is_exclusive(), "{} hides its access", system.name());
        }
    }
    #[test]
    #[should_panic(expected = "duplicate reconnect lifecycle key")]
    fn duplicate_semantic_owner_cannot_overwrite_output() {
        let mut app = app(false);
        struct Other;
        register_reconnect_projection::<Other, Res<'static, ReconnectRequests>>(
            &mut app,
            "alpha",
            |p| zulu(p.downcast::<Res<ReconnectRequests>>().unwrap()),
        );
    }
    #[test]
    #[should_panic(expected = "duplicate reconnect owner marker")]
    fn distinct_keys_cannot_share_mutable_owner_scratch() {
        let mut app = app(false);
        register_reconnect_projection::<Alpha, Res<'static, ReconnectRequests>>(
            &mut app,
            "other",
            |p| alpha(p.downcast::<Res<ReconnectRequests>>().unwrap()),
        );
    }
    #[test]
    #[should_panic(expected = "projector must answer each request ordinal")]
    fn missing_occurrence_is_refused_instead_of_shifting_private_output() {
        fn broken(_: Res<ReconnectRequests>) -> ReconnectBatch {
            vec![]
        }
        let mut app = App::new();
        test_reconnect_welcomes(&mut app, &["owner"]);
        register_reconnect_projection::<Alpha, Res<'static, ReconnectRequests>>(
            &mut app,
            "broken",
            |p| broken(p.downcast::<Res<ReconnectRequests>>().unwrap()),
        );
        app.finish();
        app.world_mut().run_schedule(FixedUpdate);
    }
    #[test]
    fn physical_transport_scratch_inherits_the_existing_canonical_owner() {
        use crate::authoritative::StateCensus;
        let app = app(false);
        let census = app.world().resource::<StateCensus>();
        let owner = std::any::type_name::<ReplicationLifecycleRegistry>();
        assert_eq!(
            census.alias_owner(std::any::type_name::<ReconnectRequests>()),
            Some(owner)
        );
    }
    #[derive(Resource, Default)]
    struct SourceA(u32);
    #[derive(Component, Default)]
    struct SourceB(u32);
    fn advance(mut a: ResMut<SourceA>, mut b: Query<&mut SourceB>) {
        a.0 += 1;
        b.single_mut().unwrap().0 = a.0;
    }
    fn coherent_app() -> App {
        let mut app = App::new();
        app.init_resource::<SourceA>().init_resource::<SimOutbox>();
        test_reconnect_welcomes(&mut app, &[]);
        app.world_mut().spawn(SourceB::default());
        register_reconnect_projection::<
            Alpha,
            (Res<'static, ReconnectRequests>, Res<'static, SourceA>),
        >(&mut app, "alpha", |p| {
            let (requests, source) = p
                .downcast::<(Res<ReconnectRequests>, Res<SourceA>)>()
                .unwrap();
            requests
                .0
                .iter()
                .map(|_| {
                    vec![ServerMessage::GameStartCountdown {
                        remaining_secs: source.0,
                    }]
                })
                .collect()
        });
        register_reconnect_projection::<
            Zulu,
            (
                Res<'static, ReconnectRequests>,
                Query<'static, 'static, &'static SourceB>,
            ),
        >(&mut app, "zulu", |p| {
            let (requests, source) = p
                .downcast::<(Res<ReconnectRequests>, Query<&SourceB>)>()
                .unwrap();
            requests
                .0
                .iter()
                .map(|_| {
                    vec![ServerMessage::GameStartCountdown {
                        remaining_secs: source.single().unwrap().0,
                    }]
                })
                .collect()
        });
        app
    }
    #[test]
    fn capture_holds_the_exact_concrete_read_union_against_a_cross_owner_writer() {
        let mut app = coherent_app();
        let registered = owners(app.world());
        let mut capture = build_capture(app.world_mut(), registered);
        let capture_access = capture.initialize(app.world_mut());
        let access = capture_access.combined_access();
        let components = app.world().components();
        let a = components.resource_id::<SourceA>().unwrap();
        let b = components.component_id::<SourceB>().unwrap();
        assert!(access.has_resource_read(a));
        assert!(access.has_component_read(b));
        assert!(!access.has_any_write());
        assert!(!access.has_read_all_resources() && !access.has_read_all_components());
        assert!(!capture.is_exclusive());
        let mut writer = IntoSystem::into_system(advance);
        let writer_access = writer.initialize(app.world_mut());
        assert!(
            !capture_access.is_compatible(&writer_access),
            "the scheduler must hold both owners' reads across the whole capture"
        );
    }
    #[test]
    fn one_cohort_cannot_straddle_a_writer_between_owners_or_duplicate_requests() {
        let mut app = coherent_app();
        app.add_systems(FixedUpdate, advance);
        app.finish();
        let mut values = std::collections::BTreeSet::new();
        for _ in 0..16 {
            test_reconnect_welcomes(&mut app, &["one", "two", "one"]);
            app.world_mut().run_schedule(FixedUpdate);
            let rows = app.world_mut().resource_mut::<SimOutbox>().drain();
            assert_eq!(rows.len(), 6);
            let captured: Vec<_> = rows
                .iter()
                .map(|r| {
                    let ServerMessage::GameStartCountdown { remaining_secs } = r.message else {
                        panic!("typed observation")
                    };
                    remaining_secs
                })
                .collect();
            assert!(
                captured.iter().all(|n| *n == captured[0]),
                "all owners/occurrences share one source observation"
            );
            values.insert(captured[0]);
        }
        assert!(values.len() >= 15, "the conflicting writer really advances");
    }
    #[test]
    #[should_panic(expected = "reconnect registration after finalization")]
    fn finalized_cohort_refuses_a_late_owner() {
        let mut app = app(false);
        app.finish();
        struct Later;
        register_reconnect_projection::<Later, Res<'static, ReconnectRequests>>(
            &mut app,
            "late",
            |p| alpha(p.downcast::<Res<ReconnectRequests>>().unwrap()),
        );
    }
    #[test]
    fn empty_requests_do_not_invoke_owner_projection() {
        struct Never;
        let mut app = App::new();
        test_reconnect_welcomes(&mut app, &[]);
        register_reconnect_projection::<Never, ()>(&mut app, "never", |_| {
            panic!("empty capture invoked an owner")
        });
        app.finish();
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().resource::<ReconnectRequests>().0.is_empty());
        // A real targeted Welcome outside InProgress must likewise not invoke
        // a projector; the collector clears the cohort at the same boundary.
        test_reconnect_welcomes(&mut app, &["lobby-only"]);
        app.insert_resource(State::new(crate::core::messages::GamePhase::Lobby));
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().resource::<ReconnectRequests>().0.is_empty());
    }
}
