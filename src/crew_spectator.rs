//! A destroyed crew keeps its ship identity; only its camera follows another hull.
use crate::core::messages::{
    ClientMessage, CrewSpectatorPayload, CrewSpectatorShip, DeliveryClass, GamePhase, ServerMessage,
};
use crate::entities::spawner::{EntityName, EntitySystemHull, EntityUuid};
use crate::lobby::{InboundMessage, OutboundMessage, Sessions, Target};
use crate::server_app::{LocalShip, Ship};
use bevy::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

/// Host-local presentation state. The original LocalShip and its private
/// projections never change when the shared camera follows another ship.
#[derive(Resource, Default)]
pub struct CrewSpectator {
    pub active: bool,
    pub target: Option<String>,
    crew_ship: Option<String>,
    original_crew: BTreeSet<String>,
    last_sent: BTreeMap<String, CrewSpectatorPayload>,
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CrewSpectatorPresentation;

pub struct CrewSpectatorPlugin;
impl Plugin for CrewSpectatorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CrewSpectator>().add_systems(
            Update,
            update_crew_spectator
                .in_set(CrewSpectatorPresentation)
                // GM view expiry first clears its own force. A destroyed crew
                // then installs the cinematic force for this frame; without
                // this order the GM sync can clear it back to Camera.
                .after(crate::gm_presentation::sync_views)
                .run_if(resource_exists::<Sessions>),
        );
    }
}

/// Runs outside the authoritative tick: this is the local crew's camera and
/// reliable Console projection, shared by native and browser hosts.
fn update_crew_spectator(
    policy: CrewMissionPolicy,
    phase: Res<State<GamePhase>>,
    sessions: Res<Sessions>,
    local: Query<(&EntityUuid, &EntitySystemHull), With<LocalShip>>,
    ships: Query<(&EntityUuid, Option<&EntityName>, &EntitySystemHull), With<Ship>>,
    mut modes: Query<&mut crate::ship::state::ShipViewMode, With<LocalShip>>,
    mut state: ResMut<CrewSpectator>,
    mut inbound: MessageReader<InboundMessage>,
    mut outbound: MessageWriter<OutboundMessage>,
) {
    let messages: Vec<_> = inbound.read().collect();
    let dead = local
        .single()
        .ok()
        .filter(|(_, hull)| hull.0.is_destroyed());
    let active = *phase.get() == GamePhase::InProgress
        && policy.continues_after_ship_loss()
        && dead.is_some();
    if *phase.get() == GamePhase::Lobby {
        state.crew_ship = None;
        state.original_crew.clear();
    }
    if active {
        let uuid = &dead.unwrap().0 .0;
        if state.crew_ship.as_ref() != Some(uuid) {
            state.crew_ship = Some(uuid.clone());
            state.original_crew = sessions
                .0
                .players()
                .iter()
                .filter(|player| !player.spectator)
                .map(|player| player.token.clone())
                .collect();
            state.target = None;
        }
    }
    let was_active = state.active;
    state.active = active;
    let mut candidates: Vec<_> = if active {
        ships
            .iter()
            .filter(|(_, _, hull)| !hull.0.is_destroyed())
            .map(|(uuid, name, _)| CrewSpectatorShip {
                uuid: uuid.0.clone(),
                name: name.map(|n| n.0.clone()).unwrap_or_else(|| uuid.0.clone()),
            })
            .collect()
    } else {
        Vec::new()
    };
    candidates.sort_by(|a, b| a.uuid.cmp(&b.uuid));
    if !state
        .target
        .as_ref()
        .is_some_and(|uuid| candidates.iter().any(|ship| &ship.uuid == uuid))
    {
        state.target = candidates.first().map(|ship| ship.uuid.clone());
    }
    for ev in &messages {
        let ClientMessage::SelectCrewSpectatorTarget { uuid } = &ev.msg else {
            continue;
        };
        if active
            && state.original_crew.contains(&ev.token)
            && sessions
                .0
                .players()
                .iter()
                .any(|p| p.token == ev.token && p.connected)
            && candidates.iter().any(|ship| ship.uuid == *uuid)
        {
            state.target = Some(uuid.clone());
        }
    }
    if active || was_active {
        if let Ok(mut mode) = modes.single_mut() {
            mode.force_view_mode(active.then_some(crate::core::messages::ViewMode::Cinematic));
        }
    }
    for player in sessions.0.players().iter().filter(|p| p.connected) {
        let crew_active = active && state.original_crew.contains(&player.token);
        let payload = CrewSpectatorPayload {
            active: crew_active,
            can_select: crew_active,
            target: crew_active.then(|| state.target.clone()).flatten(),
            ships: if crew_active {
                candidates.clone()
            } else {
                Vec::new()
            },
        };
        let reconnect = messages
            .iter()
            .any(|ev| ev.token == player.token && matches!(ev.msg, ClientMessage::Identify { .. }));
        if reconnect || state.last_sent.get(&player.token) != Some(&payload) {
            state
                .last_sent
                .insert(player.token.clone(), payload.clone());
            outbound.write(OutboundMessage {
                target: Target::Token(player.token.clone()),
                msg: ServerMessage::CrewSpectatorState(payload),
                delivery: DeliveryClass::Reliable,
            });
        }
    }
}

/// Mission topology, independent of which crew this host projects.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct CrewMissionPolicy<'w> {
    world: Option<Res<'w, crate::world::config::WorldConfig>>,
    fleet: Option<Res<'w, crate::lockstep::FleetRoster>>,
}

impl CrewMissionPolicy<'_> {
    pub(crate) fn continues_after_ship_loss(&self) -> bool {
        self.world
            .as_ref()
            .is_some_and(|world| world.ship_slots.len() > 1)
            || self
                .fleet
                .as_ref()
                .is_some_and(|fleet| fleet.ships().len() > 1)
    }
}

#[cfg(test)]
mod tests {
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
}
