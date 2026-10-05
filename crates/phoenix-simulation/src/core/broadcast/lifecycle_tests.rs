use super::*;

#[derive(Resource, Default, Debug, PartialEq, Eq)]
struct Trace(Vec<String>);

fn reset_alpha(world: &mut World) {
    world.resource_mut::<Trace>().0.push("reset:alpha".into());
}

fn reset_zulu(world: &mut World) {
    world.resource_mut::<Trace>().0.push("reset:zulu".into());
}

#[test]
fn reset_uses_key_order_not_registration_order() {
    let mut app = App::new();
    app.init_resource::<Trace>();
    app.register_replication_lifecycle(
        ReplicationLifecycleAdapter::new("zulu").with_reset(reset_zulu),
    );
    app.register_replication_lifecycle(
        ReplicationLifecycleAdapter::new("alpha").with_reset(reset_alpha),
    );
    assert_eq!(
        app.world()
            .resource::<ReplicationLifecycleRegistry>()
            .keys()
            .collect::<Vec<_>>(),
        vec!["alpha", "zulu"]
    );
    reset_registered_replication(app.world_mut());
    assert_eq!(
        app.world().resource::<Trace>().0,
        vec!["reset:alpha", "reset:zulu"]
    );
}

#[test]
fn reset_is_optional_for_a_projection_owner() {
    use super::super::reconnect::{
        register_reconnect_projection, ReconnectBatch, ReconnectRequests,
    };
    struct ProjectionOwner;
    fn project(requests: Res<ReconnectRequests>) -> ReconnectBatch {
        requests.0.iter().map(|_| vec![]).collect()
    }
    let mut app = App::new();
    app.init_resource::<Trace>();
    app.register_replication_lifecycle(
        ReplicationLifecycleAdapter::new("reset-only").with_reset(reset_alpha),
    );
    register_reconnect_projection::<ProjectionOwner, Res<'static, ReconnectRequests>>(
        &mut app,
        "projection-only",
        |p| project(p.downcast::<Res<ReconnectRequests>>().unwrap()),
    );
    reset_registered_replication(app.world_mut());
    assert_eq!(app.world().resource::<Trace>().0, vec!["reset:alpha"]);
    assert_eq!(
        app.world().resource::<ReplicationLifecycleRegistry>().len(),
        2
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
