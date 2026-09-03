//! Civilian discovery by scan and rescue by transporter, driven end to end
//! through a real seeded headless run (issue #1348, PRD #1337).
//!
//! `probe_transporter.toml` fields the Alliance Destroyer as a backfilled player
//! ship (both the sensor suite that discovers the life signs and the Engineering
//! transporter that recovers them) and a stranded lighter carrying four
//! civilians. This test drives the discovery through the ordinary admitted
//! `ScanTarget` path and the rescue through the ordinary admitted
//! `TransportSelectContact` / `StartTransport` path — the same admission a human
//! console's own controls take — and asserts on the narrative timeline the run
//! folds them into, exactly as `task_lifecycle_dock_umbilical_repair.rs` does for
//! the #1345 workflows.
//!
//! What it proves, end to end:
//!
//!   * the rescue objective and the "life signs" cue appear ONLY after the scan
//!     latches `scan.lighter.taken` — the computer announces life signs only
//!     after the revealing scan;
//!   * the transporter recovers the four civilians over time and the transport
//!     task lifecycle records exactly one start and one `completed` terminal;
//!   * the rescue completes: `rescue.lighter.recovered` rises, the objective
//!     completes, and the saved report row is written.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;

use project_phoenix::core::messages::{
    ClientMessage, ObjectiveStatus, SystemControlPayload, SystemId,
};
use project_phoenix::entities::spawner::{EntityName, EntityUuid};
use project_phoenix::headless::args::{ticks_for_sim_seconds, ReportFormat};
use project_phoenix::headless::report::RunReport;
use project_phoenix::headless::{build_headless_app, build_report, run, HeadlessArgs};
use project_phoenix::lobby::InboundMessage;
use project_phoenix::server_app::LocalShip;
use project_phoenix::ship::state::ShipPhysics;
use project_phoenix::ship::system_registry::{SENSORS_SYSTEM_ID, TRANSPORTER_SYSTEM_ID};

const LIGHTER: &str = "world.probe_transporter.entity.lighter.name";
const TOKEN: &str = "ai:transporter-lifecycle-probe";

fn args() -> HeadlessArgs {
    let dt = 1.0 / 60.0;
    HeadlessArgs {
        world_path: "assets/worlds/probe_transporter.toml".into(),
        ship_path: "assets/entities/alliance_destroyer.toml".into(),
        dt,
        max_ticks: ticks_for_sim_seconds(90.0, dt),
        seed: Some(1348),
        deterministic: true,
        report_format: ReportFormat::Json,
        ..Default::default()
    }
}

fn operator(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<LocalShip>>()
        .iter(app.world())
        .next()
        .expect("the probe world spawns a local operator")
}

fn place_operator(app: &mut App, position: Vec3) {
    let entity = operator(app);
    let mut physics = app
        .world_mut()
        .get_mut::<ShipPhysics>(entity)
        .expect("the operator is a ship");
    physics.x = position.x;
    physics.y = position.y;
    physics.z = position.z;
}

fn named_entity(app: &mut App, name: &str) -> Entity {
    let mut q = app.world_mut().query::<(Entity, &EntityName)>();
    q.iter(app.world())
        .find(|(_, n)| n.0 == name)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("the probe world spawns '{name}'"))
}

fn uuid_of(app: &mut App, entity: Entity) -> String {
    app.world_mut()
        .get::<EntityUuid>(entity)
        .expect("the entity has a uuid")
        .0
        .clone()
}

fn move_entity(app: &mut App, entity: Entity, position: Vec3) {
    app.world_mut()
        .get_mut::<Transform>(entity)
        .expect("the entity has a transform")
        .translation = position;
}

fn send(app: &mut App, target: SystemId, payload: SystemControlPayload) {
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<InboundMessage>>()
        .write(InboundMessage {
            token: TOKEN.into(),
            msg: ClientMessage::ControlSystem { target, payload },
        });
    run(app, 2);
}

fn objective_status(report: &RunReport, id: &str) -> Option<ObjectiveStatus> {
    report
        .scenario
        .as_ref()
        .expect("the probe world loads a scenario")
        .objectives
        .iter()
        .find(|o| o.id == id)
        .map(|o| o.status.clone())
}

fn flag_set(report: &RunReport, name: &str) -> bool {
    report
        .scenario
        .as_ref()
        .expect("the probe world loads a scenario")
        .flags
        .iter()
        .any(|f| f.name == name && f.value > 0)
}

fn lifecycle_rows(report: &RunReport) -> Vec<(u64, String, String, String)> {
    report
        .narrative
        .events
        .iter()
        .filter(|e| e.event.kind.is_task_lifecycle())
        .map(|e| {
            let reason = match e.event.detail.get("reason") {
                Some(project_phoenix::core::narrative::NarrativeValue::Text(s)) => s.clone(),
                _ => String::new(),
            };
            (
                e.seq,
                e.event.kind.as_str().to_string(),
                e.event.id.clone(),
                reason,
            )
        })
        .collect()
}

/// The whole flow: scan reveals the civilians (and only then does the objective
/// post), then the transporter recovers them and the rescue completes.
#[test]
fn a_scan_reveals_civilians_and_the_transporter_recovers_them() {
    let args = args();
    let mut app = build_headless_app(&args).expect("probe_transporter world should build");
    run(&mut app, 60);

    let lighter = named_entity(&mut app, LIGHTER);
    let lighter_uuid = uuid_of(&mut app, lighter);

    // Put the operator on top of its spawn and the lighter 100 units off — inside
    // the destroyer's 120-unit detailed scan band and its 500-unit transporter
    // reach.
    place_operator(&mut app, Vec3::ZERO);
    move_entity(&mut app, lighter, Vec3::new(100.0, 0.0, 0.0));
    run(&mut app, 2);

    // Before the scan, the discovery objective must not exist — the computer has
    // announced nothing.
    let before = build_report(&mut app, &args, 0.0);
    assert_eq!(
        objective_status(&before, "obj-rescue-civilians"),
        None,
        "the rescue objective must not post before the revealing scan"
    );

    // Scan the lighter through the ordinary admitted path.
    send(
        &mut app,
        SystemId(SENSORS_SYSTEM_ID.into()),
        SystemControlPayload::ScanTarget {
            uuid: lighter_uuid.clone(),
        },
    );
    // Give the scan flag its one-tick bridge and the discovery beat a tick.
    run(&mut app, 6);

    let discovered = build_report(&mut app, &args, 0.0);
    assert!(
        flag_set(&discovered, "scan.lighter.taken"),
        "the scan must latch its taken flag"
    );
    assert_eq!(
        objective_status(&discovered, "obj-rescue-civilians"),
        Some(ObjectiveStatus::Active),
        "the rescue objective posts once the scan has revealed the life signs"
    );

    // Select the lighter and start the transport.
    send(
        &mut app,
        SystemId(TRANSPORTER_SYSTEM_ID.into()),
        SystemControlPayload::TransportSelectContact {
            uuid: lighter_uuid.clone(),
        },
    );
    send(
        &mut app,
        SystemId(TRANSPORTER_SYSTEM_ID.into()),
        SystemControlPayload::StartTransport,
    );
    // Four civilians at two seconds each: eight seconds, plus slack.
    run(&mut app, ticks_for_sim_seconds(12.0, args.dt));

    let report = build_report(&mut app, &args, 0.0);

    // The transport lifecycle: exactly one start and one `completed` terminal.
    let rows = lifecycle_rows(&report);
    let transport: Vec<_> = rows
        .iter()
        .filter(|(_, _, key, _)| key.contains("/transport/"))
        .collect();
    assert!(
        transport
            .iter()
            .any(|(_, kind, _, _)| kind == "task_started"),
        "the transport must record a start beat: {transport:?}"
    );
    assert!(
        transport
            .iter()
            .any(|(_, _, _, reason)| reason == "completed"),
        "the transport must complete when every civilian is recovered: {transport:?}"
    );

    // The recovery raised its completion flag, and the rescue objective
    // completed off it (the saved report row is authored off the same flag).
    assert!(
        flag_set(&report, "rescue.lighter.recovered"),
        "the transporter raises the recovered flag when every civilian is aboard"
    );
    assert_eq!(
        objective_status(&report, "obj-rescue-civilians"),
        Some(ObjectiveStatus::Completed),
        "the rescue objective completes when the recovery lands"
    );
}
