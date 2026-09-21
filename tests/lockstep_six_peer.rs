//! Six simulation peers under representative crew and GM traffic (issue #1519).
//!
//! Four complete ship hosts each know only their own three Station clients.
//! Two more complete, stationless simulations carry the two GM identities. All
//! six independently derive the ordinary NPC's commands; no NPC output crosses
//! the mesh. The short test is ordinary-CI coverage. The ignored test runs the
//! same workload at wall-clock pace for the separately-recorded one-hour gate.

#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bevy::prelude::*;
use serde::Serialize;

use project_phoenix::command_admission::log::{HostSlot, LoggedCommand};
use project_phoenix::core::messages::{
    AdmittedCommands, ClientMessage, GamePhase, PowerGroupId, StationId, SystemControlPayload,
    SystemId,
};
use project_phoenix::entities::spawner::{EntityName, EntityUuid};
use project_phoenix::gm_action::{
    sequence_owner_proposal, GmAction, GmActionFrame, GmActionGrant, GmActionId, GmActionJournal,
    GmActionProposal,
};
use project_phoenix::headless::{build_headless_app, run, world_digest, HeadlessArgs};
use project_phoenix::lobby::{InboundMessage, Sessions};
use project_phoenix::lockstep::{
    join_fleet, FleetGm, FleetLockstep, FleetRoster, FleetShip, FleetSlotOf, MeshAgreement,
    MeshFrame, MeshInbox, MeshOutbox,
};
use project_phoenix::sim_tick::SimTick;

const WORLD: &str = "assets/worlds/probe_fleet_six_peer.toml";
const SHIP: &str = "assets/entities/alliance_cruiser.toml";
const SEED: u64 = 1_519_006;
const OWNER: HostSlot = HostSlot(1);
const SHIP_SLOTS: [HostSlot; 4] = [HostSlot(1), HostSlot(2), HostSlot(3), HostSlot(4)];
const GM_SLOTS: [HostSlot; 2] = [HostSlot(5), HostSlot(6)];
const ALL_SLOTS: [HostSlot; 6] = [
    HostSlot(1),
    HostSlot(2),
    HostSlot(3),
    HostSlot(4),
    HostSlot(5),
    HostSlot(6),
];
const CREW: [(&str, &str); 3] = [("captain", "Std"), ("helm", "Std"), ("engineering", "Std")];

fn args() -> HeadlessArgs {
    HeadlessArgs {
        world_path: WORLD.into(),
        ship_path: SHIP.into(),
        max_ticks: 10_000_000,
        seed: Some(SEED),
        deterministic: true,
        ..Default::default()
    }
}

fn roster(local: HostSlot) -> FleetRoster {
    let ships = SHIP_SLOTS
        .into_iter()
        .map(|host| FleetShip {
            host,
            authored_slot_id: None,
            ship_path: Some(SHIP.into()),
            crew: CREW
                .into_iter()
                .map(|(station, rating)| (StationId(station.into()), rating.into()))
                .collect(),
        })
        .collect();
    FleetRoster::with_participants_and_gms(
        ships,
        ALL_SLOTS.to_vec(),
        vec![
            FleetGm {
                host: GM_SLOTS[0],
                operator_id: "gm-1".into(),
            },
            FleetGm {
                host: GM_SLOTS[1],
                operator_id: "gm-2".into(),
            },
        ],
        local,
        OWNER,
    )
    .expect("four ship slots and two GM slots form one valid roster")
}

struct Host {
    app: App,
    slot: HostSlot,
    tokens: BTreeMap<&'static str, String>,
}

impl Host {
    fn new(slot: HostSlot) -> Self {
        let mut app = build_headless_app(&args()).expect("six-peer probe app should build");
        let mut tokens = BTreeMap::new();
        if SHIP_SLOTS.contains(&slot) {
            let mut sessions = app.world_mut().resource_mut::<Sessions>();
            for (station, _) in CREW {
                let token = format!("slot-{}-{station}", slot.0);
                sessions
                    .0
                    .register(token.clone(), format!("Slot {} {station}", slot.0))
                    .expect("each representative client has a unique token");
                sessions
                    .0
                    .set_station(&token, Some(StationId(station.into())));
                tokens.insert(station, token);
            }
        }
        let delay = project_phoenix::lockstep::authored_delay(app.world());
        assert!(delay > 0, "the endurance world must exercise delayed input");
        assert!(join_fleet(app.world_mut(), roster(slot), delay));
        app.insert_resource(MeshAgreement::new(30));
        Self { app, slot, tokens }
    }

    fn tick(&self) -> u64 {
        self.app.world().resource::<SimTick>().0
    }

    fn digest(&self) -> u64 {
        world_digest(self.app.world())
    }

    fn command(&mut self, station: &'static str, target: &str, payload: SystemControlPayload) {
        let token = self
            .tokens
            .get(station)
            .expect("this ship host owns that client");
        self.app
            .world_mut()
            .resource_mut::<Messages<InboundMessage>>()
            .write(InboundMessage {
                token: token.clone(),
                msg: ClientMessage::ControlSystem {
                    target: SystemId(target.into()),
                    payload,
                },
            });
    }

    fn drain(&mut self) -> Vec<MeshFrame> {
        self.app.world_mut().resource_mut::<MeshOutbox>().drain()
    }

    fn deliver(&mut self, frames: &[MeshFrame]) {
        let mut inbox = self.app.world_mut().resource_mut::<MeshInbox>();
        for frame in frames {
            inbox.push(frame.clone());
        }
    }

    /// Commands emitted by entities which no fleet slot flies. This observes
    /// NPC decisions separately from the whole-world digest without inventing
    /// an owner for the NPC or putting those commands on the mesh.
    fn npc_output(&mut self) -> Vec<NpcOutput> {
        let mut query = self.app.world_mut().query_filtered::<
            (&EntityUuid, Option<&EntityName>, &AdmittedCommands),
            Without<FleetSlotOf>,
        >();
        let mut output: Vec<_> = query
            .iter(self.app.world())
            .filter(|(_, _, commands)| !commands.0.is_empty())
            .map(|(uuid, name, commands)| NpcOutput {
                uuid: uuid.0.clone(),
                name: name.map(|name| name.0.clone()),
                commands: commands
                    .0
                    .iter()
                    .map(|command| format!("{}:{:?}", command.target.0, command.payload))
                    .collect(),
            })
            .collect();
        output.sort_by(|left, right| left.uuid.cmp(&right.uuid));
        output
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct NpcOutput {
    uuid: String,
    name: Option<String>,
    commands: Vec<String>,
}

fn ferry(hosts: &mut [Host]) {
    let outgoing: Vec<Vec<MeshFrame>> = hosts.iter_mut().map(Host::drain).collect();
    for (receiver, host) in hosts.iter_mut().enumerate() {
        for (sender, frames) in outgoing.iter().enumerate() {
            if receiver != sender {
                host.deliver(frames);
            }
        }
    }
}

fn step(hosts: &mut [Host]) {
    ferry(hosts);
    for host in hosts {
        run(&mut host.app, 1);
    }
}

fn crew_wave(hosts: &mut [Host], wave: u64) {
    for (index, slot) in SHIP_SLOTS.into_iter().enumerate() {
        let host = hosts.iter_mut().find(|host| host.slot == slot).unwrap();
        let sign = if (wave + index as u64).is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        host.command(
            "helm",
            "helm-thrust",
            SystemControlPayload::SetThrust {
                value: 0.45 + index as f32 * 0.1,
            },
        );
        host.command(
            "captain",
            "red-alert",
            SystemControlPayload::SetRedAlert {
                active: wave.is_multiple_of(2),
            },
        );
        host.command(
            "engineering",
            "power-reactor",
            SystemControlPayload::SetPowerGroupAllocation {
                group: PowerGroupId("weapons".into()),
                level: if sign > 0.0 { 3 } else { 2 },
            },
        );
        host.command(
            "helm",
            "helm-steering",
            SystemControlPayload::SetSteering { value: sign * 0.25 },
        );
    }
}

fn gm_wave(hosts: &mut [Host], gm: HostSlot, hostile: bool, wave: u64) {
    let owner = &mut hosts[0];
    let now = owner.tick();
    let ready_through = owner
        .app
        .world()
        .resource::<FleetLockstep>()
        .0
        .ready_through(now);
    let proposal = GmActionProposal {
        from: gm,
        operator_id: if gm == GM_SLOTS[0] { "gm-1" } else { "gm-2" }.into(),
        correlation: GmActionId::new(format!("six-peer-{wave}-{}", gm.0)).unwrap(),
        action: GmAction::SetFactionHostility {
            faction: "Harrow".into(),
            enemy: "Alliance".into(),
            hostile,
        },
    };
    let grant = sequence_owner_proposal(
        &mut owner.app.world_mut().resource_mut::<GmActionJournal>(),
        &proposal,
        OWNER,
        now,
        ready_through,
        false,
        false,
        false,
    )
    .expect("the bound GM may submit an ordinary canonical operation");
    owner
        .app
        .world_mut()
        .resource_mut::<MeshOutbox>()
        .push(MeshFrame::GmAction(GmActionFrame::Granted(grant)));
}

#[derive(Serialize)]
struct DivergenceArtifact {
    format: &'static str,
    revision: String,
    world: &'static str,
    world_content_hash: String,
    ship: &'static str,
    ship_content_hash: String,
    seed: u64,
    peer_composition: Vec<String>,
    client_composition: Vec<String>,
    first_divergent_tick: u64,
    digests: Vec<(u32, u64)>,
    npc_outputs: Vec<(u32, Vec<NpcOutput>)>,
    command_replay: Vec<LoggedCommand>,
    gm_replay: Vec<GmActionGrant>,
}

fn revision() -> String {
    std::process::Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unavailable".into())
}

fn content_hash(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|error| {
        panic!("the divergence artifact could not pin content {path}: {error}")
    });
    format!("fnv1a64:{:016x}", vellum_digest::fnv1a(&bytes))
}

fn artifact(hosts: &mut [Host], tick: u64, outputs: Vec<Vec<NpcOutput>>) -> DivergenceArtifact {
    DivergenceArtifact {
        format: "phoenix-six-peer-divergence-v1",
        revision: revision(),
        world: WORLD,
        world_content_hash: content_hash(WORLD),
        ship: SHIP,
        ship_content_hash: content_hash(SHIP),
        seed: SEED,
        peer_composition: ALL_SLOTS
            .into_iter()
            .map(|slot| {
                if SHIP_SLOTS.contains(&slot) {
                    format!("slot-{}:ship-host", slot.0)
                } else {
                    format!("slot-{}:gm-peer", slot.0)
                }
            })
            .collect(),
        client_composition: SHIP_SLOTS
            .into_iter()
            .flat_map(|slot| {
                CREW.into_iter()
                    .map(move |(station, _)| format!("slot-{}:{station}", slot.0))
            })
            .collect(),
        first_divergent_tick: tick,
        digests: hosts
            .iter()
            .map(|host| (host.slot.0, host.digest()))
            .collect(),
        npc_outputs: hosts
            .iter()
            .zip(outputs)
            .map(|(host, output)| (host.slot.0, output))
            .collect(),
        command_replay: hosts[0]
            .app
            .world()
            .resource::<project_phoenix::command_admission::CommandLog>()
            .entries()
            .to_vec(),
        gm_replay: hosts[0]
            .app
            .world()
            .resource::<GmActionJournal>()
            .grants()
            .to_vec(),
    }
}

fn fail_with_artifact(hosts: &mut [Host], tick: u64, outputs: Vec<Vec<NpcOutput>>) -> ! {
    let artifact = artifact(hosts, tick, outputs);
    let json = serde_json::to_string_pretty(&artifact).expect("artifact is JSON serialisable");
    let directory = std::env::var_os("PHOENIX_T5_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/t5-artifacts"));
    let path = directory.join("six-peer-divergence.json");
    let retained = std::fs::create_dir_all(&directory)
        .and_then(|()| std::fs::write(&path, &json))
        .map(|()| path.display().to_string())
        .unwrap_or_else(|error| format!("artifact write failed: {error}"));
    panic!("six-peer divergence artifact retained at {retained}\n{json}");
}

fn assert_equivalent(hosts: &mut [Host]) -> usize {
    let tick = hosts[0].tick();
    let outputs: Vec<_> = hosts.iter_mut().map(Host::npc_output).collect();
    let npc_commands = outputs[0].iter().map(|output| output.commands.len()).sum();
    let digest = hosts[0].digest();
    if hosts
        .iter()
        .any(|host| host.tick() != tick || host.digest() != digest)
        || outputs.iter().any(|output| output != &outputs[0])
    {
        fail_with_artifact(hosts, tick, outputs);
    }
    npc_commands
}

fn build_fleet() -> Vec<Host> {
    ALL_SLOTS.into_iter().map(Host::new).collect()
}

fn run_workload(hosts: &mut [Host], ticks: u64, wall_clock: bool) -> usize {
    let frame = Duration::from_nanos(16_666_667);
    let mut deadline = Instant::now();
    let mut npc_commands = 0;
    for tick in 0..ticks {
        // Traffic starts after the first fixed step: the mission boundary
        // deliberately clears the prior run's logs, so a tick-zero injection
        // is setup noise rather than endurance workload evidence.
        if tick % 120 == 30 {
            crew_wave(hosts, tick / 120);
        }
        if tick % 90 == 30 {
            let wave = tick / 90;
            gm_wave(
                hosts,
                GM_SLOTS[(wave as usize) % GM_SLOTS.len()],
                !wave.is_multiple_of(2),
                wave,
            );
        }
        step(hosts);
        npc_commands += assert_equivalent(hosts);
        if wall_clock {
            deadline += frame;
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
        }
    }
    npc_commands
}

#[test]
fn six_peers_keep_digest_and_npc_output_equal_under_twelve_clients_and_two_gms() {
    let mut hosts = build_fleet();
    let npc_commands = run_workload(&mut hosts, 240, false);
    assert!(
        npc_commands > 0,
        "the test must compare real NPC decisions, not six empty outputs"
    );

    let command_log = hosts[0]
        .app
        .world()
        .resource::<project_phoenix::command_admission::CommandLog>();
    assert!(command_log.ticks_are_monotonic());
    assert!(
        command_log.len() >= 32,
        "two waves from twelve active clients"
    );
    let gm = hosts[0].app.world().resource::<GmActionJournal>();
    assert!(gm.grants().iter().any(|grant| grant.from == GM_SLOTS[0]));
    assert!(gm.grants().iter().any(|grant| grant.from == GM_SLOTS[1]));
    assert!(hosts.iter().all(|host| {
        *host.app.world().resource::<State<GamePhase>>().get() == GamePhase::InProgress
    }));

    let sample =
        serde_json::to_value(artifact(&mut hosts, 240, vec![Vec::new(); ALL_SLOTS.len()])).unwrap();
    assert_eq!(sample["peer_composition"].as_array().unwrap().len(), 6);
    assert_eq!(sample["client_composition"].as_array().unwrap().len(), 12);
    assert!(!sample["revision"].as_str().unwrap().is_empty());
    assert!(sample["world_content_hash"]
        .as_str()
        .unwrap()
        .starts_with("fnv1a64:"));
    assert!(sample["ship_content_hash"]
        .as_str()
        .unwrap()
        .starts_with("fnv1a64:"));
    assert!(!sample["command_replay"].as_array().unwrap().is_empty());
    assert!(!sample["gm_replay"].as_array().unwrap().is_empty());
}

/// Manual acceptance workload. Default: one hour at a 60 Hz wall-clock pace.
/// Set `PHOENIX_T5_ENDURANCE_SECONDS` for a shorter rehearsal; failure retains
/// the first divergent tick and replay-rich JSON under `target/t5-artifacts/`.
#[test]
#[ignore = "one-hour T5 acceptance workload; run explicitly"]
fn six_peer_one_hour_endurance_workload() {
    let seconds = std::env::var("PHOENIX_T5_ENDURANCE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(3_600);
    assert!(seconds > 0, "the endurance duration must be positive");
    let ticks = seconds.checked_mul(60).expect("duration fits a tick count");
    let mut hosts = build_fleet();
    let npc_commands = run_workload(&mut hosts, ticks, true);
    assert!(npc_commands > 0, "the endurance run observed no NPC output");
}
