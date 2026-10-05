use super::*;
use crate::core::broadcast::lobby::LobbyBroadcaster;
use crate::core::broadcast::sim::SimBroadcaster;
use crate::core::broadcast::{Lobby, Sim};
use crate::core::messages::{GamePhase, ServerMessage};
use crate::lobby::{LobbyPlugin, OutboundMessage, Sessions};
use std::time::Duration;

// ── cadence_timer unit tests ──────────────────────────────────────────

#[test]
fn cadence_timer_hz_builds_repeating_timer_with_inverse_period() {
    let t = cadence_timer(&Cadence::Hz(10.0)).expect("Hz > 0 should build a timer");
    assert_eq!(t.mode(), TimerMode::Repeating);
    assert!((t.duration().as_secs_f32() - 0.1).abs() < 1e-6);
}

#[test]
fn cadence_timer_non_positive_hz_yields_none() {
    assert!(cadence_timer(&Cadence::Hz(0.0)).is_none());
    assert!(cadence_timer(&Cadence::Hz(-1.0)).is_none());
}

#[test]
fn cadence_timer_period_builds_repeating_timer() {
    let d = Duration::from_millis(250);
    let t = cadence_timer(&Cadence::Period(d)).expect("Period should build a timer");
    assert_eq!(t.mode(), TimerMode::Repeating);
    assert_eq!(t.duration(), d);
}

#[test]
fn cadence_timer_on_event_yields_none() {
    assert!(cadence_timer(&Cadence::OnEvent).is_none());
}

#[test]
fn cadence_timer_once_builds_zero_duration_one_shot() {
    let t = cadence_timer(&Cadence::Once).expect("Once should build a timer");
    assert_eq!(t.mode(), TimerMode::Once);
    assert_eq!(t.duration(), Duration::ZERO);
}

// ── Coexistence: both parameterisations in one App ────────────────────

#[derive(Resource, Default)]
struct Outbox(Vec<OutboundMessage>);

fn collect(mut reader: MessageReader<OutboundMessage>, mut box_: ResMut<Outbox>) {
    for m in reader.read() {
        box_.0.push(m.clone());
    }
}

/// Sim and Lobby broadcasters must coexist in one `App` with fully
/// independent registries: a registration on one must never fire through
/// the other's dispatch (which would show up as a duplicate message or a
/// message with the wrong `DeliveryClass`).
#[test]
fn sim_and_lobby_registries_coexist_independently() {
    let sim = SimBroadcaster::new().register(Audience::All, Cadence::Once, |_: &mut World| {
        vec![ServerMessage::GameStarted]
    });
    let lobby = LobbyBroadcaster::new().register(Audience::All, Cadence::Once, |_: &mut World| {
        vec![ServerMessage::PlayerLeft { token: "t1".into() }]
    });

    let mut app = App::new();
    app.add_plugins(LobbyPlugin);
    app.add_plugins(bevy::time::TimePlugin);
    app.add_plugins(sim);
    app.add_plugins(lobby);

    // Lobby phase so the lobby gate passes (sim dispatch is internally
    // ungated in this harness — its gate lives on the SimSet chain).
    app.world_mut()
        .insert_resource(State::new(GamePhase::Lobby));
    {
        let mut sm = app.world_mut().resource_mut::<Sessions>();
        sm.0.register("alice".to_string(), "Alice".to_string())
            .unwrap();
    }
    app.init_resource::<Outbox>();
    app.add_systems(PostUpdate, collect);
    // One fixed step per update (issue #895): both dispatchers run on
    // the logical tick.
    crate::ship::test_support::drive_one_fixed_step_per_update(
        &mut app,
        std::time::Duration::from_millis(1),
    );

    // Distinct Resource identities, one registration each.
    assert_eq!(
        app.world()
            .resource::<BroadcastRegistry<Sim>>()
            .registrations
            .len(),
        1
    );
    assert_eq!(
        app.world()
            .resource::<BroadcastRegistry<Lobby>>()
            .registrations
            .len(),
        1
    );

    app.update();
    let msgs = app.world().resource::<Outbox>().0.clone();

    // Sim's registration fired exactly once, stamped Snapshot.
    let game_started: Vec<_> = msgs
        .iter()
        .filter(|m| matches!(m.msg, ServerMessage::GameStarted))
        .collect();
    assert_eq!(
        game_started.len(),
        1,
        "sim registration must fire exactly once (no cross-dispatch)"
    );
    assert_eq!(game_started[0].delivery, DeliveryClass::Snapshot);

    // Lobby's registration fired exactly once, stamped Reliable.
    let player_left: Vec<_> = msgs
        .iter()
        .filter(|m| matches!(m.msg, ServerMessage::PlayerLeft { .. }))
        .collect();
    assert_eq!(
        player_left.len(),
        1,
        "lobby registration must fire exactly once (no cross-dispatch)"
    );
    assert_eq!(player_left[0].delivery, DeliveryClass::Reliable);
}
