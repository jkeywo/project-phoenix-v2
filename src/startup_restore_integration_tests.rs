//! Ordinary-App coverage for the shared startup driver after the T2 merge.

use super::*;
use crate::command_admission::log::{stamp_accepted_command, PendingCommands, ShipKey};
use crate::console::command::server::ShipStationStances;
use crate::console::weapons::LastShipAttacker;
use crate::core::messages::{AdmittedCommand, GamePhase, StationId, SystemControlPayload};
use crate::entities::spawner::EntityUuid;
use crate::headless::{build_headless_app, HeadlessArgs};
use crate::server_app::LocalShip;
use crate::ship::control_source::ControlSource;
use crate::ship::state::ShipRedAlert;
use crate::ship_plugin::{ShipConfigComponent, ShipSystemControlSources};
use crate::sim_digest::world_digest;
use crate::sim_tick::SimTick;
use crate::world::server::WorldContentRuntime;

const WORLD: &str = "tests/fixtures/world_materialization_root.toml";
const HULL: &str = "assets/entities/alliance_destroyer.toml";
const SEED: u64 = 42;

fn fresh_app() -> App {
    let mut app = build_headless_app(&HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: HULL.into(),
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    })
    .unwrap();
    let period = crate::sim_tick::sim_tick_period(
        app.world()
            .resource::<crate::world::config::WorldConfig>()
            .global
            .sim_tick_hz,
    );
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(period));
    app.finish();
    app.cleanup();
    app
}

fn local_ship(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .single(app.world())
        .unwrap()
}

fn keep_human_controls(app: &mut App, ship: Entity) {
    let ids: Vec<_> = app
        .world()
        .get::<ShipConfigComponent>(ship)
        .unwrap()
        .0
        .systems
        .iter()
        .map(|system| system.id.clone())
        .collect();
    let mut sources = app
        .world_mut()
        .get_mut::<ShipSystemControlSources>(ship)
        .unwrap();
    for id in ids {
        sources.0.set(id, ControlSource::Human);
    }
}

fn captain_alert(app: &mut App, ship: Entity, active: bool) {
    let tick = app.world().resource::<SimTick>().0;
    let key = ShipKey(app.world().get::<EntityUuid>(ship).unwrap().0.clone());
    stamp_accepted_command(
        &mut app.world_mut().resource_mut::<PendingCommands>(),
        tick,
        None,
        ship,
        key,
        AdmittedCommand {
            target: crate::ship::system_registry::red_alert_system_id(),
            payload: SystemControlPayload::SetRedAlert { active },
            response_token: None,
            feedback_correlation: None,
        },
    );
    app.update();
    assert_eq!(app.world().resource::<SimTick>().0, tick + 1);
    assert_eq!(app.world().get::<ShipRedAlert>(ship).unwrap().0, active);
}

fn stance(app: &App, ship: Entity) -> Option<&str> {
    app.world()
        .get::<ShipStationStances>(ship)
        .unwrap()
        .0
        .get(&StationId("tactical".into()))
        .map(String::as_str)
}

#[test]
fn shared_startup_restore_preserves_alert_frontier_then_accepts_live_captain_commands() {
    // Both mismatch directions matter: saved false must not clear attribution,
    // and saved true must not overwrite a deliberately selected normal stance.
    for captured_alert in [false, true] {
        let mut source = fresh_app();
        for _ in 0..12 {
            source.update();
        }
        assert_eq!(
            source.world().resource::<State<GamePhase>>().get(),
            &GamePhase::InProgress
        );
        let source_ship = local_ship(&mut source);
        keep_human_controls(&mut source, source_ship);
        captain_alert(&mut source, source_ship, captured_alert);
        // A real stable authored entity, present in both bootstraps, keeps the
        // ordinary dead-attacker cleanup from masking the alert-edge contract.
        let attacker = source
            .world()
            .resource::<WorldContentRuntime>()
            .name_to_uuid["test.materialization.root"]
            .clone();
        source
            .world_mut()
            .get_mut::<LastShipAttacker>(source_ship)
            .unwrap()
            .0 = Some(attacker.clone());
        source.update();
        assert_eq!(
            source
                .world()
                .get::<LastShipAttacker>(source_ship)
                .unwrap()
                .0
                .as_deref(),
            Some(attacker.as_str())
        );
        let captured_stance = if captured_alert {
            "tactical-normal"
        } else {
            "tactical-high"
        };
        let station = StationId("tactical".into());
        assert!(source
            .world()
            .get::<ShipConfigComponent>(source_ship)
            .unwrap()
            .0
            .station(&station)
            .unwrap()
            .stances
            .iter()
            .any(|row| row.id == captured_stance));
        source
            .world_mut()
            .get_mut::<ShipStationStances>(source_ship)
            .unwrap()
            .0
            .insert(station, captured_stance.into());
        let checkpoint = snapshot::capture(source.world());
        let digest = world_digest(source.world());
        let uuid = source
            .world()
            .get::<EntityUuid>(source_ship)
            .unwrap()
            .0
            .clone();
        let controls = checkpoint
            .entities
            .iter()
            .find(|row| row.uuid == uuid)
            .unwrap()
            .control
            .clone()
            .expect("the saved hull carries its control frontier");
        let run = snapshot::run_for(
            checkpoint.clone(),
            digest,
            SEED,
            WORLD,
            snapshot::versions(&crate::content_ledger::frozen_or_live()),
        );

        let mut target = fresh_app();
        assert!(!target
            .world()
            .contains_resource::<crate::server_app::GameStartEntityUuids>());
        stage(target.world_mut(), run);
        assert_eq!(
            advance(target.world_mut()),
            None,
            "the shared driver waits for real bootstrap"
        );
        assert!(is_pending(target.world()));
        assert!(save_slots_lifecycle::startup_restore_pending(
            target.world()
        ));
        let mut applied = None;
        for _ in 0..30 {
            target.update();
            let ready = snapshot::ready_to_restore(target.world(), &checkpoint);
            if ready {
                let ship = local_ship(&mut target);
                keep_human_controls(&mut target, ship);
                // Insert immediately before the actual restore, without an
                // update in between: the bootstrap change has no consumed edge.
                target.world_mut().entity_mut(ship).remove::<ShipRedAlert>();
                target
                    .world_mut()
                    .entity_mut(ship)
                    .insert(ShipRedAlert(!captured_alert));
                target
                    .world_mut()
                    .get_mut::<LastShipAttacker>(ship)
                    .unwrap()
                    .0 = None;
            }
            if let Some(outcome) = advance(target.world_mut()) {
                assert!(ready, "the real bootstrap prerequisites were satisfied");
                assert_eq!(
                    outcome,
                    RestoreOutcome::Applied {
                        tick: checkpoint.tick
                    }
                );
                applied = Some(outcome);
                break;
            }
        }
        assert_eq!(
            applied,
            Some(RestoreOutcome::Applied {
                tick: checkpoint.tick
            })
        );
        assert_eq!(world_digest(target.world()), digest);
        assert!(!is_pending(target.world()));
        assert!(!save_slots_lifecycle::startup_restore_pending(
            target.world()
        ));
        assert_eq!(
            advance(target.world_mut()),
            None,
            "Applied is emitted exactly once"
        );
        let ship = local_ship(&mut target);
        assert_eq!(
            crate::ship::continuation::ControlState::capture_from(target.world().entity(ship)),
            Some(controls)
        );
        for _ in 0..2 {
            target.update();
            assert_eq!(advance(target.world_mut()), None);
            assert_eq!(
                target.world().get::<ShipRedAlert>(ship).unwrap().0,
                captured_alert
            );
            assert_eq!(
                target
                    .world()
                    .get::<LastShipAttacker>(ship)
                    .unwrap()
                    .0
                    .as_deref(),
                Some(attacker.as_str())
            );
            assert_eq!(stance(&target, ship), Some(captured_stance));
        }

        // An accepted Captain command still uses the ordinary ingress drain
        // and owner consumer. A same-value request does not fabricate an edge.
        captain_alert(&mut target, ship, captured_alert);
        assert_eq!(stance(&target, ship), Some(captured_stance));
        assert_eq!(
            target
                .world()
                .get::<LastShipAttacker>(ship)
                .unwrap()
                .0
                .as_deref(),
            Some(attacker.as_str())
        );
        if !captured_alert {
            captain_alert(&mut target, ship, true);
            assert_eq!(stance(&target, ship), Some("tactical-high"));
            assert_eq!(
                target
                    .world()
                    .get::<LastShipAttacker>(ship)
                    .unwrap()
                    .0
                    .as_deref(),
                Some(attacker.as_str())
            );
        }
        captain_alert(&mut target, ship, false);
        assert_eq!(stance(&target, ship), Some("tactical-normal"));
        assert_eq!(
            target.world().get::<LastShipAttacker>(ship).unwrap().0,
            None
        );
    }
}
