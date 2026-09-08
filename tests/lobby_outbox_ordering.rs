//! Source-bound before/after evidence for the narrow #1400 outbox declaration.
//! This fixture is run unchanged on both production revisions. Its output is
//! evidence to compare, never a replacement allowance or a blessed digest.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::{ecs::message::MessageCursor, prelude::*};
use project_phoenix::core::messages::{ClientMessage, DeliveryClass, ServerMessage};
use project_phoenix::headless::{build_headless_app, HeadlessArgs};
use project_phoenix::lobby::server::{InboundMessage, OutboundMessage};
use project_phoenix::sim_digest::world_digest;
use project_phoenix::sim_tick::SimTick;

#[path = "common/default_pool.rs"]
mod default_pool;

const WORLD: &str = "assets/worlds/probe_fleet_duel.toml";
const SEED: u64 = 1_400_249;

#[test]
fn default_pool_preserves_lobby_start_and_reconnect_order() {
    default_pool::run_guards(&[("lobby_start_and_reconnect_trace_is_repeatable", 2)]);
}

#[test]
fn lobby_start_and_reconnect_trace_is_repeatable() {
    let args = HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        seed: Some(SEED),
        deterministic: default_pool::deterministic(),
        ..Default::default()
    };
    let mut apps = [
        build_headless_app(&args).expect("first ordinary host"),
        build_headless_app(&args).expect("second ordinary host"),
    ];
    let workers = bevy::tasks::ComputeTaskPool::get().thread_num();
    let expected_executor = if default_pool::deterministic() {
        assert_eq!(workers, 1, "pinned process must use one compute worker");
        bevy::ecs::schedule::ExecutorKind::SingleThreaded
    } else {
        assert!(
            workers > 1,
            "default process must exercise multiple workers"
        );
        bevy::ecs::schedule::ExecutorKind::MultiThreaded
    };
    for app in &apps {
        assert_eq!(
            app.get_schedule(FixedUpdate).unwrap().get_executor_kind(),
            expected_executor
        );
        assert_eq!(
            app.world()
                .resource::<project_phoenix::sim_rng::SimRng>()
                .seed(),
            SEED
        );
    }
    println!(
        "\nPHOENIX_OUTBOX_EXECUTION={}",
        serde_json::json!({
            "compute_threads": workers, "executor": format!("{expected_executor:?}"),
            "seed": SEED, "process": std::process::id(),
        })
    );
    let mut cursors = [
        MessageCursor::<OutboundMessage>::default(),
        MessageCursor::default(),
    ];
    let mut boundaries = Vec::new();
    let mut lifecycle = [Vec::new(), Vec::new()];
    let mut starts = [0, 0];
    let mut welcomes = [0, 0];
    for frame in 0..90 {
        for (index, app) in apps.iter_mut().enumerate() {
            // Ordinary Identify, followed by a same-token reconnect. No direct
            // LobbyOutbox write or production registration change in this proof.
            if frame == 20 || frame == 40 {
                app.world_mut().write_message(InboundMessage {
                    token: "outbox-observer".into(),
                    msg: ClientMessage::Identify {
                        token: "outbox-observer".into(),
                        name: "Observer".into(),
                    },
                });
            }
            app.update();
            let tick = app.world().resource::<SimTick>().0;
            for message in cursors[index].read(app.world().resource::<Messages<OutboundMessage>>())
            {
                match &message.msg {
                    ServerMessage::GameStarted => starts[index] += 1,
                    ServerMessage::Welcome { .. } => {
                        welcomes[index] += 1;
                        assert_eq!(
                            message.target,
                            project_phoenix::lobby::Target::Token("outbox-observer".into())
                        );
                    }
                    ServerMessage::GameStartCountdown { .. } => {}
                    _ => continue,
                }
                assert_eq!(message.delivery, DeliveryClass::Reliable);
                lifecycle[index].push(serde_json::json!({
                    "frame": frame, "tick": tick,
                    "target": format!("{:?}", message.target),
                    "message": message.msg,
                }));
            }
        }
        let left = (
            apps[0].world().resource::<SimTick>().0,
            world_digest(apps[0].world()),
        );
        let right = (
            apps[1].world().resource::<SimTick>().0,
            world_digest(apps[1].world()),
        );
        assert_eq!(left, right, "authoritative boundary at frame {frame}");
        assert_eq!(
            lifecycle[0], lifecycle[1],
            "ordered lifecycle output at frame {frame}"
        );
        boundaries.push(left);
    }
    assert_eq!(starts, [1, 1], "one ordinary mission-start event per host");
    assert_eq!(
        welcomes,
        [2, 2],
        "join and reconnect both traverse the real handler"
    );
    assert!(
        boundaries
            .windows(2)
            .filter(|pair| pair[1].0 > pair[0].0)
            .count()
            > 80
    );
    println!(
        "\nPHOENIX_OUTBOX_TRACE={}",
        serde_json::json!({
            "boundaries": boundaries, "lifecycle": lifecycle[0],
        })
    );
    for app in &apps {
        default_pool::observe(app, WORLD, SEED);
    }
}
