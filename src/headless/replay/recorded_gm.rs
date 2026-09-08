//! Replay GM-only input between two ordinary StoredRun captures (#1316).
//!
//! This is an evidence tool, not an attended-session replay format. The browser
//! collector must attest that no ordinary crew commands or seat/rating changes
//! occurred: ordinary manual saves deliberately omit that history. A successful
//! proof means the saved origin plus these GM requests reproduces the recorded
//! end, including its actual terminal results. It cannot prove absent history.

use super::PhoenixSim;
use bevy::prelude::*;
use serde::Serialize;

use crate::gm_action::{GmActionFrame, GmActionJournal};
use crate::lockstep::{FleetRoster, FleetSlotOf};
use crate::snapshot::{self, BootIdentity, StoredRun};

const BOOT_FRAME_LIMIT: u64 = 600;
const MAX_CONTINUATION_TICKS: u64 = 1_000_000;

#[derive(Debug, Serialize)]
pub struct RecordedGmReplayError {
    pub stage: &'static str,
    pub detail: String,
}

impl std::fmt::Display for RecordedGmReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.stage, self.detail)
    }
}
impl std::error::Error for RecordedGmReplayError {}

fn refused(stage: &'static str, detail: impl ToString) -> RecordedGmReplayError {
    RecordedGmReplayError {
        stage,
        detail: detail.to_string(),
    }
}

/// Digest strings retain all 64 bits when consumed by the Node evidence runner.
#[derive(Debug, Serialize)]
pub struct RecordedGmReplayReport {
    pub scope: &'static str,
    pub initial_tick: u64,
    pub final_tick: u64,
    pub initial_digest: String,
    pub expected_final_digest: String,
    pub actual_final_digest: String,
    pub initial_applied_actions: usize,
    pub final_applied_actions: usize,
    pub results_match: bool,
    pub frozen_crew: usize,
}

pub struct RecordedGmReplay {
    pub report: RecordedGmReplayReport,
    pub simulation: PhoenixSim,
}

fn parse(text: &str) -> Result<StoredRun, RecordedGmReplayError> {
    let run = StoredRun::from_ron(text).map_err(|e| refused("parse", e))?;
    // Gate format/rules before interpreting current payload semantics; actual
    // content is checked against the ordinary boot ledger below.
    let compatible = vellum_save::Versions::new(
        snapshot::SNAPSHOT_FORMAT,
        snapshot::SIMULATION_RULES,
        run.versions.content,
    );
    run.versions
        .check(&compatible)
        .map_err(|e| refused("versions", e))?;
    snapshot::required_boot_identity(&run).map_err(|e| refused("boot identity", e))?;
    let saved = run
        .snapshot
        .as_ref()
        .expect("required_boot_identity requires a snapshot");
    if saved.tick != saved.state.tick
        || run.ledger.final_tick != saved.tick
        || run.ledger.final_digest != saved.digest
        || !run.ledger.samples.is_empty()
    {
        return Err(refused(
            "capture",
            "snapshot and save ledger must describe one exact boundary",
        ));
    }
    if !run.commands.is_empty() {
        return Err(refused(
            "scope",
            "ordinary command history is outside GM-only continuation",
        ));
    }
    if saved
        .state
        .rng
        .as_ref()
        .is_none_or(|rng| rng.seed != run.seed)
    {
        return Err(refused(
            "seed",
            "snapshot RNG and envelope seed differ or are absent",
        ));
    }
    Ok(run)
}

fn validate_pair(initial: &StoredRun, final_run: &StoredRun) -> Result<(), RecordedGmReplayError> {
    let first = initial.snapshot.as_ref().unwrap();
    let last = final_run.snapshot.as_ref().unwrap();
    if initial.versions != final_run.versions
        || initial.scenario != final_run.scenario
        || initial.seed != final_run.seed
    {
        return Err(refused("pair", "versions, scenario and seed must match"));
    }
    if first.state.boot_identity != last.state.boot_identity {
        return Err(refused(
            "scope",
            "fleet topology, hulls, authored UUIDs or frozen crew changed",
        ));
    }
    if first.state.phase != Some(crate::core::messages::GamePhase::InProgress)
        || !matches!(
            last.state.phase,
            Some(
                crate::core::messages::GamePhase::InProgress
                    | crate::core::messages::GamePhase::GameOver
            )
        )
        || last.tick < first.tick
        || last.tick - first.tick > MAX_CONTINUATION_TICKS
    {
        return Err(refused(
            "boundary",
            "requires a bounded forward continuation from InProgress",
        ));
    }
    let source = &first.state.gm_actions;
    let target = &last.state.gm_actions;
    if source.initial_paused() != target.initial_paused()
        || source.recovery_generations() != target.recovery_generations()
    {
        return Err(refused(
            "scope",
            "pause baseline or recovery-generation history changed",
        ));
    }
    if !target.grants().starts_with(source.grants())
        || !target.applied_prefix().starts_with(source.applied_prefix())
        || !target
            .applied_results()
            .starts_with(source.applied_results())
    {
        return Err(refused(
            "journal",
            "initial grants and actual outcomes must be an exact prefix",
        ));
    }
    if target.grants()[source.len()..]
        .iter()
        .any(|grant| grant.apply_tick < first.tick)
    {
        return Err(refused("journal", "new grants precede the restored origin"));
    }
    let roster = &first.state.boot_identity.as_ref().unwrap().fleet;
    for grant in target.grants() {
        crate::gm_action::validate_fleet_frame(&GmActionFrame::Granted(grant.clone()), roster)
            .map_err(|reason| refused("grant identity", format!("{reason:?}")))?;
    }
    Ok(())
}

fn validate_roster(boot: &BootIdentity) -> Result<(), RecordedGmReplayError> {
    let roster = &boot.fleet;
    let rebuilt = FleetRoster::with_participants_and_gms(
        roster.ships().to_vec(),
        roster.participants(),
        roster.gms().to_vec(),
        roster.local(),
        roster.owner(),
    )
    .ok_or_else(|| refused("topology", "invalid frozen roster"))?;
    if &rebuilt != roster
        || !roster
            .ships()
            .iter()
            .any(|ship| ship.host == roster.local())
    {
        return Err(refused(
            "topology",
            "requires canonical topology exported by a ship host",
        ));
    }
    for ship in roster.ships() {
        let mut stations = std::collections::BTreeSet::new();
        if ship
            .crew
            .iter()
            .any(|(station, rating)| !stations.insert(&station.0) || rating.is_empty())
        {
            return Err(refused(
                "crew",
                "duplicate Station or empty rating in frozen crew",
            ));
        }
    }
    Ok(())
}

fn seed_local_sessions(
    world: &mut World,
    roster: &FleetRoster,
) -> Result<(), RecordedGmReplayError> {
    let crew = &roster
        .ships()
        .iter()
        .find(|ship| ship.host == roster.local())
        .unwrap()
        .crew;
    let mut sessions = crate::lobby::session::SessionManager::new();
    for (index, (station, rating)) in crew.iter().enumerate() {
        let token = format!("recorded-gm-crew-{index}");
        sessions
            .register(token.clone(), format!("Recorded crew {index}"))
            .map_err(|e| refused("crew", format!("{e:?}")))?;
        sessions.set_station(&token, Some(station.clone()));
        sessions.set_pending_rating(station, rating.clone());
    }
    world.insert_resource(crate::lobby::Sessions(sessions));
    Ok(())
}

/// Assert the ordinary boot/control owner still represents the immutable crew.
/// GM takeover changes System sources, so compare Station ratings, not those
/// transient sources. The proof's native regression inspects actual sources too.
fn check_frozen_crew(world: &World, roster: &FleetRoster) -> Result<(), RecordedGmReplayError> {
    if world.get_resource::<FleetRoster>() != Some(roster) {
        return Err(refused(
            "scope",
            "fleet crew/topology changed while continuing",
        ));
    }
    if !world
        .resource::<crate::command_admission::CommandLog>()
        .is_empty()
    {
        return Err(refused(
            "scope",
            "ordinary commands entered the continuation",
        ));
    }
    let sessions = world.resource::<crate::lobby::Sessions>();
    let local = roster
        .ships()
        .iter()
        .find(|ship| ship.host == roster.local())
        .unwrap();
    let mut seats: Vec<_> = sessions
        .0
        .players()
        .iter()
        .filter(|player| player.connected && !player.afk && !player.spectator)
        .filter_map(|player| player.station.clone())
        .collect();
    let mut expected: Vec<_> = local
        .crew
        .iter()
        .map(|(station, _)| station.clone())
        .collect();
    seats.sort_by(|a, b| a.0.cmp(&b.0));
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    if seats != expected {
        return Err(refused(
            "scope",
            "local crew seats changed while continuing",
        ));
    }
    let mut query = world
        .try_query::<(
            &FleetSlotOf,
            &crate::ship::components::ShipConfigComponent,
            &crate::ship::components::ActiveStationRatings,
        )>()
        .ok_or_else(|| refused("crew", "Fleet ship ratings are unavailable"))?;
    let mut observed = std::collections::BTreeSet::new();
    for (slot, config, ratings) in query.iter(world) {
        if !observed.insert(slot.0) {
            return Err(refused("crew", "duplicate live Fleet ship slot"));
        }
        let ship = roster
            .ships()
            .iter()
            .find(|ship| ship.host == slot.0)
            .ok_or_else(|| refused("crew", "ship has no frozen Fleet row"))?;
        for (station, rating) in &ship.crew {
            if crate::ship::rating::resolve_automated_systems(&config.0, station, rating).is_none()
            {
                return Err(refused(
                    "crew",
                    "recorded Station/rating is not supported by its real hull",
                ));
            }
        }
        for station in &config.0.stations {
            let expected = ship
                .rating_at(&station.id)
                .unwrap_or(crate::ship::rating::BACKFILL_RATING);
            if ratings.0.get(&station.id).map(String::as_str) != Some(expected) {
                return Err(refused(
                    "scope",
                    format!(
                        "Station {} changed from recorded rating {expected}",
                        station.id.0
                    ),
                ));
            }
        }
    }
    if observed.len() != roster.ships().len() {
        return Err(refused("crew", "a frozen Fleet ship has no live ratings"));
    }
    Ok(())
}

/// Recreate real simulation state from the initial capture, then derive the
/// final capture by applying only the recorded GM request suffix. Neither
/// final world state nor final terminal results are installed into the App.
pub fn replay_exports(
    initial_text: &str,
    final_text: &str,
) -> Result<RecordedGmReplay, RecordedGmReplayError> {
    let initial = parse(initial_text)?;
    let final_run = parse(final_text)?;
    validate_pair(&initial, &final_run)?;
    let first = initial.snapshot.as_ref().unwrap();
    let last = final_run.snapshot.as_ref().unwrap();
    let boot = first.state.boot_identity.as_ref().unwrap();
    validate_roster(boot)?;
    let args = crate::headless::HeadlessArgs {
        world_path: initial.scenario.clone(),
        ship_path: boot.selected_ship.clone(),
        seed: Some(initial.seed),
        deterministic: true,
        max_ticks: BOOT_FRAME_LIMIT
            + (last.tick - first.tick).saturating_mul(4)
            + (last.state.gm_actions.len() as u64).saturating_mul(4)
            + 120,
        ..Default::default()
    };
    let mut simulation = PhoenixSim::new(&args, 0, 0).map_err(|e| refused("boot", e))?;
    let world = simulation.app.world_mut();
    initial
        .versions
        .check(&snapshot::versions(&crate::content_ledger::frozen_or_live()))
        .map_err(|e| refused("content", e))?;
    snapshot::validate_boot_identity_for_world(
        boot,
        world.resource::<crate::world::config::WorldConfig>(),
    )
    .map_err(|e| refused("boot identity", e))?;
    world.insert_resource(boot.fleet.clone());
    seed_local_sessions(world, &boot.fleet)?;
    crate::server_app::stage_resume_game_start_entity_uuids(world, boot);
    // A frame advances at most one ordinary fixed step, regardless of browser
    // render pacing. Preserve the saved fixed overstep when restore writes it.
    let step = crate::sim_tick::sim_tick_period(
        world
            .resource::<crate::world::config::WorldConfig>()
            .global
            .sim_tick_hz,
    );
    world.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(step));
    let mut ready = false;
    for frame in 0..BOOT_FRAME_LIMIT {
        simulation.step();
        if !simulation
            .app
            .world()
            .contains_resource::<crate::server_app::GameStartEntityUuids>()
        {
            continue;
        }
        match snapshot::reconcile_world_layers(simulation.app.world_mut(), &first.state) {
            snapshot::LayerReconcileStatus::Ready => {
                ready = snapshot::ready_to_restore(simulation.app.world(), &first.state)
                    || (frame + 1 == BOOT_FRAME_LIMIT
                        && snapshot::ready_to_rebuild(simulation.app.world(), &first.state));
            }
            snapshot::LayerReconcileStatus::Waiting => {}
            snapshot::LayerReconcileStatus::Failed(path) => {
                return Err(refused(
                    "restore",
                    format!("could not reconstruct layer {path}"),
                ))
            }
        }
        if ready {
            break;
        }
    }
    if !ready {
        return Err(refused(
            "restore",
            "bootstrap did not become restore-ready within its bound",
        ));
    }
    let restore = snapshot::restore(simulation.app.world_mut(), &first.state);
    if !restore.is_complete() {
        return Err(refused(
            "restore",
            format!("incomplete: {:?}", restore.gaps),
        ));
    }
    let restored = crate::sim_digest::world_digest(simulation.app.world());
    if restored != first.digest {
        return Err(refused(
            "initial digest",
            format!("recorded {}, restored {restored}", first.digest),
        ));
    }
    check_frozen_crew(simulation.app.world(), &boot.fleet)?;
    {
        let mut journal = simulation.app.world_mut().resource_mut::<GmActionJournal>();
        for grant in last.state.gm_actions.grants().iter().skip(journal.len()) {
            journal
                .insert(grant.clone())
                .map_err(|e| refused("journal", format!("{e:?}")))?;
        }
    }
    // Future requests are replay input. Prove that installing them did not
    // rewrite the exact origin's applied frontier or authoritative digest.
    if crate::sim_digest::world_digest(simulation.app.world()) != first.digest {
        return Err(refused(
            "journal",
            "appending future requests changed the origin digest",
        ));
    }
    simulation.gm_actions = last.state.gm_actions.clone();
    simulation.final_tick = Some(last.tick);
    simulation.frames = 0;
    while !simulation.reached_replay_end() && simulation.frames_left() {
        if super::is_game_over(&simulation.app) {
            return Err(refused(
                "continuation",
                "scenario ended before the recorded boundary",
            ));
        }
        simulation.step();
        check_frozen_crew(simulation.app.world(), &boot.fleet)?;
    }
    if !simulation.reached_replay_end() || simulation.tick() != last.tick {
        return Err(refused(
            "continuation",
            "did not reach the exact final tick and applied frontier",
        ));
    }
    let actual = simulation.app.world().resource::<GmActionJournal>();
    if actual != &last.state.gm_actions {
        return Err(refused(
            "results",
            "re-derived GM journal/outcomes differ from the final capture",
        ));
    }
    let digest = crate::sim_digest::world_digest(simulation.app.world());
    if digest != last.digest {
        return Err(refused(
            "final digest",
            format!("recorded {}, replayed {digest}", last.digest),
        ));
    }
    Ok(RecordedGmReplay {
        report: RecordedGmReplayReport {
            scope: "GM-only requests; immutable recorded crew; absence of omitted human input requires collector evidence",
            initial_tick: first.tick, final_tick: last.tick,
            initial_digest: restored.to_string(), expected_final_digest: last.digest.to_string(),
            actual_final_digest: digest.to_string(), initial_applied_actions: first.state.gm_actions.applied_grants(),
            final_applied_actions: actual.applied_grants(), results_match: true,
            frozen_crew: boot.fleet.ships().iter().map(|ship| ship.crew.len()).sum(),
        },
        simulation,
    })
}
