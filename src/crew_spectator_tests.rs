use super::*;
use crate::command_admission::log::HostSlot;
use crate::core::messages::{StationId, SystemId, ViewMode};
use crate::lockstep::{FleetRoster, FleetShip};
use crate::ship::{damage::SystemHull, state::ShipViewMode};

fn hull(alive: bool) -> EntitySystemHull {
    let id = SystemId("hull".into());
    let mut hull = SystemHull::from_config(&[(id.clone(), 10.0)]);
    if !alive {
        hull.set_hp(&id, 0.0);
    }
    EntitySystemHull(hull)
}

fn fixture() -> (App, Entity, Entity, Entity) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::state::app::StatesPlugin));
    app.insert_state(GamePhase::InProgress);
    app.add_message::<InboundMessage>()
        .add_message::<OutboundMessage>();
    app.add_plugins(CrewSpectatorPlugin);
    app.insert_resource(FleetRoster::new(
        vec![FleetShip::new(HostSlot(0)), FleetShip::new(HostSlot(1))],
        HostSlot(0),
    ));
    let mut sessions = crate::lobby::session::SessionManager::new();
    for token in ["a", "b", "viewer"] {
        sessions.register(token.into(), token.into()).unwrap();
    }
    sessions.set_station("a", Some(StationId("helm".into())));
    sessions.set_spectator("viewer", true);
    sessions.disconnect("b");
    app.insert_resource(Sessions(sessions));
    let own = app
        .world_mut()
        .spawn((
            LocalShip,
            Ship,
            EntityUuid("own".into()),
            hull(false),
            ShipViewMode::default(),
        ))
        .id();
    let ally = app
        .world_mut()
        .spawn((
            Ship,
            EntityUuid("ally".into()),
            EntityName("Ally".into()),
            hull(true),
        ))
        .id();
    let enemy = app
        .world_mut()
        .spawn((
            Ship,
            EntityUuid("enemy".into()),
            EntityName("Opponent".into()),
            hull(true),
        ))
        .id();
    (app, own, ally, enemy)
}

fn choose(app: &mut App, token: &str, uuid: &str) {
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: token.into(),
            msg: ClientMessage::SelectCrewSpectatorTarget { uuid: uuid.into() },
        });
}

#[test]
fn crew_spectator_latest_original_crew_choice_preserves_identity_and_reconnect() {
    let (mut app, own, _, _) = fixture();
    app.update();
    assert_eq!(
        app.world().resource::<CrewSpectator>().target.as_deref(),
        Some("ally")
    );
    assert_eq!(
        app.world().get::<ShipViewMode>(own).unwrap().view_mode,
        ViewMode::Cinematic
    );
    choose(&mut app, "viewer", "enemy");
    choose(&mut app, "b", "enemy"); // disconnected original crew cannot send
    app.update();
    assert_eq!(
        app.world().resource::<CrewSpectator>().target.as_deref(),
        Some("ally")
    );
    app.world_mut().resource_mut::<Sessions>().0.reconnect("b");
    choose(&mut app, "a", "enemy");
    choose(&mut app, "b", "ally");
    app.update();
    assert_eq!(
        app.world().resource::<CrewSpectator>().target.as_deref(),
        Some("ally")
    );
    choose(&mut app, "b", "enemy");
    app.update();
    assert_eq!(
        app.world().resource::<CrewSpectator>().target.as_deref(),
        Some("enemy")
    );
    assert!(app.world().get::<LocalShip>(own).is_some());
    assert_eq!(
        app.world().resource::<Sessions>().0.players()[0].station,
        Some(StationId("helm".into()))
    );
    app.world_mut()
        .resource_mut::<Messages<OutboundMessage>>()
        .clear();
    app.world_mut()
        .resource_mut::<Messages<InboundMessage>>()
        .write(InboundMessage {
            token: "b".into(),
            msg: ClientMessage::Identify {
                token: "b".into(),
                name: "B".into(),
            },
        });
    app.update();
    let messages = app.world().resource::<Messages<OutboundMessage>>();
    let mut cursor = messages.get_cursor();
    assert!(cursor.read(messages).any(|ev| ev.target == Target::Token("b".into())
            && matches!(&ev.msg, ServerMessage::CrewSpectatorState(p) if p.can_select && p.target.as_deref() == Some("enemy"))));
}

#[test]
fn crew_spectator_target_loss_new_join_and_other_host_are_independent() {
    let (mut app, _, ally, enemy) = fixture();
    let (mut other, other_own, _, _) = fixture();
    other.world_mut().entity_mut(other_own).insert(hull(true));
    app.update();
    other.update();
    {
        let messages = app.world().resource::<Messages<OutboundMessage>>();
        let mut cursor = messages.get_cursor();
        assert!(cursor.read(messages).any(|ev| ev.target == Target::Token("viewer".into())
                && matches!(&ev.msg, ServerMessage::CrewSpectatorState(p) if !p.active && !p.can_select && p.target.is_none() && p.ships.is_empty())));
    }
    assert!(!other.world().resource::<CrewSpectator>().active);
    app.world_mut()
        .resource_mut::<Sessions>()
        .0
        .register("late".into(), "Late".into())
        .unwrap();
    choose(&mut app, "late", "enemy");
    app.update();
    assert_eq!(
        app.world().resource::<CrewSpectator>().target.as_deref(),
        Some("ally")
    );
    app.world_mut().entity_mut(ally).insert(hull(false));
    app.update();
    assert_eq!(
        app.world().resource::<CrewSpectator>().target.as_deref(),
        Some("enemy")
    );
    app.world_mut().entity_mut(enemy).despawn();
    choose(&mut app, "a", "missing");
    app.update();
    assert_eq!(app.world().resource::<CrewSpectator>().target, None);
    assert_eq!(
        *app.world().resource::<State<GamePhase>>().get(),
        GamePhase::InProgress
    );
    assert!(!other.world().resource::<CrewSpectator>().active);
}
