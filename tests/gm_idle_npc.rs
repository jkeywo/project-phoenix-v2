//! The GM idle-NPC advisory over real worlds, real doctrine and real GM actions
//! (issue #1435, PRD #1419 M4).
//!
//! Every hull here is an ordinary authored `[[entity]]`; every order is either
//! the template's own doctrine or a shipped `GmAction`; and every boundary is
//! counted in fixed simulation steps rather than in `update()` calls, because
//! the whole claim of this feature is that the grace is SIMULATION time.
//!
//! Its own binary for `snapshot_resume.rs`'s reason: `--deterministic` pins a
//! one-thread task pool, and Bevy's pools are process-global.
#![cfg(all(feature = "headless", not(target_arch = "wasm32")))]

use bevy::prelude::*;
use phoenix::command_admission::HostSlot;
use phoenix::core::messages::GamePhase;
use phoenix::entities::spawner::{EntityName, EntityUuid};
use phoenix::gm_action::{
    GmAction, GmActionGrant, GmActionId, GmActionJournal, GmActionOrder, GmAffectedField,
    SimulationPaused,
};
use phoenix::gm_attention::*;
use phoenix::lockstep::{FleetRoster, FleetShip};
use phoenix::sim_tick::SimTick;
use phoenix::snapshot::{capture, restore};
use project_phoenix as phoenix;

/// The shipped default: thirty simulation seconds. Authored nowhere in
/// `probe_gm_idle_npc.toml`, which is the point of that world.
const DEFAULT_WORLD: &str = "assets/worlds/probe_gm_idle_npc.toml";
/// A two-second grace in the `urgent` band, plus the doctrine palette that
/// gives the drifting hull an order.
const AUTHORED_WORLD: &str = "assets/worlds/probe_gm_idle_npc_authored.toml";
/// The advisory switched off, alongside a live pending-Comms occurrence.
const QUIET_WORLD: &str = "assets/worlds/probe_gm_idle_npc_quiet.toml";

const PICKET: &str = "world.probe_gm_idle_npc.picket";
const DRIFTER: &str = "world.probe_gm_idle_npc.drifter";

/// Both probe worlds run on the 30 Hz floor, so one frame is one fixed step and
/// a tick count is directly readable as simulation seconds.
const HZ: f64 = 30.0;

fn seeded(world: &str) -> App {
    let mut app = phoenix::headless::build_headless_app(&phoenix::headless::HeadlessArgs {
        world_path: world.into(),
        ship_path: "assets/entities/alliance_cruiser.toml".into(),
        dt: 1.0 / HZ,
        seed: Some(1435),
        deterministic: true,
        max_ticks: 5000,
        ..Default::default()
    })
    .expect("the probe world builds through the ordinary headless boot");
    app.insert_resource(FleetRoster::new(
        vec![FleetShip::new(HostSlot(1))],
        HostSlot(1),
    ));
    // The advisory only runs on a peer actually presenting a GM desk, exactly
    // like the rest of the attention queue.
    app.insert_resource(phoenix::gm_projection::BrowserGameMaster);
    app.add_plugins(GmAttentionPlugin);
    app.finish();
    app.cleanup();
    for _ in 0..600 {
        app.update();
        if *app.world().resource::<State<GamePhase>>().get() == GamePhase::InProgress {
            break;
        }
    }
    assert_eq!(
        *app.world().resource::<State<GamePhase>>().get(),
        GamePhase::InProgress,
        "the probe world reaches a running mission"
    );
    app
}

fn tick(app: &App) -> u64 {
    app.world().resource::<SimTick>().0
}

/// Advance exactly `steps` FIXED steps. Frames are only the vehicle; the count
/// that matters is the simulation's.
fn step(app: &mut App, steps: u64) {
    let target = tick(app) + steps;
    let mut frames = 0;
    while tick(app) < target {
        app.update();
        frames += 1;
        assert!(frames < steps * 8 + 64, "the fixed clock is advancing");
    }
    assert_eq!(tick(app), target, "landed on the exact tick asked for");
}

fn uuid_of(app: &mut App, name: &str) -> String {
    let mut query = app.world_mut().query::<(&EntityUuid, &EntityName)>();
    query
        .iter(app.world())
        .find(|(_, entity)| entity.0 == name)
        .unwrap_or_else(|| panic!("the world authors {name}"))
        .0
         .0
        .clone()
}

fn fleet_uuid(app: &mut App) -> String {
    let mut query = app
        .world_mut()
        .query::<(&EntityUuid, &phoenix::lockstep::FleetSlotOf)>();
    query.iter(app.world()).next().unwrap().0 .0.clone()
}

fn watch(app: &App) -> GmIdleNpcWatch {
    app.world().resource::<GmIdleNpcWatch>().clone()
}

fn spell_ticks(app: &App, ship: &str) -> Option<u64> {
    watch(app).spell(ship).map(|spell| spell.ticks)
}

/// The published queue, through the real publisher.
fn queue(app: &mut App) -> Vec<GmAttentionOccurrence> {
    use bevy::ecs::system::RunSystemOnce;
    app.world_mut()
        .run_system_once(publish_attention_projection)
        .unwrap();
    app.world()
        .resource::<GmAttentionState>()
        .last()
        .cloned()
        .unwrap_or_default()
        .occurrences
}

fn idle_rows_of(app: &mut App) -> Vec<GmAttentionOccurrence> {
    queue(app)
        .into_iter()
        .filter(|row| row.category == GmAttentionCategory::IdleNpc)
        .collect()
}

fn grant(sequence: u64, tick: u64, action: GmAction) -> GmActionGrant {
    GmActionGrant {
        from: HostSlot(4),
        sequenced_by: HostSlot(1),
        operator_id: "gm-idle".into(),
        correlation: GmActionId::new(format!("idle-{sequence}")).unwrap(),
        recovery_generation: 0,
        apply_tick: tick,
        order: GmActionOrder::new(HostSlot(4), sequence),
        action,
    }
}

/// Enqueue one GM action on the ordinary journal and let it apply.
fn enqueue(app: &mut App, action: GmAction) -> u64 {
    let at = tick(app);
    let sequence = app.world().resource::<GmActionJournal>().next_sequence();
    app.world_mut()
        .resource_mut::<GmActionJournal>()
        .insert(grant(sequence, at, action))
        .unwrap();
    step(app, 8);
    sequence
}

// ── The boundary ─────────────────────────────────────────────────────────────

/// The shipped thirty-second grace, walked to its exact edge and one step past
/// it, with a patrolling hull and the crew's own cruiser alongside as the two
/// things that must never be mistaken for idleness.
#[test]
fn an_npc_with_no_orders_reaches_the_queue_on_the_exact_grace_boundary() {
    let mut app = seeded(DEFAULT_WORLD);
    let drifter = uuid_of(&mut app, DRIFTER);
    let picket = uuid_of(&mut app, PICKET);
    let fleet = fleet_uuid(&mut app);

    // A standing patrol IS an order: this hull is flying its template's own
    // `patrol-ironveil` Patrol entry, and it is never watched.
    step(&mut app, 4);
    assert_eq!(
        spell_ticks(&app, &picket),
        None,
        "a patrolling hull is busy"
    );
    assert_eq!(
        spell_ticks(&app, &fleet),
        None,
        "a fleet ship is a player's, never an NPC the GM is advised about"
    );
    let started = spell_ticks(&app, &drifter).expect("the drifting hull is watched at once");

    // The default grace is thirty SIMULATION seconds; this world runs at 30 Hz,
    // so that is 900 fixed steps.
    let grace = 30 * 30;
    assert!(started < grace);
    step(&mut app, grace - started - 1);
    assert_eq!(spell_ticks(&app, &drifter), Some(grace - 1));
    assert!(
        idle_rows_of(&mut app).is_empty(),
        "one step short of the grace says nothing"
    );

    step(&mut app, 1);
    assert_eq!(spell_ticks(&app, &drifter), Some(grace));
    let rows = idle_rows_of(&mut app);
    assert_eq!(rows.len(), 1, "exactly one occurrence, not one per step");
    let row = &rows[0];

    // Background by default: an idle hull is worth noticing, not worth dropping
    // a conversation for.
    assert_eq!(row.band, GmAttentionBand::Background);
    assert_eq!(row.category, GmAttentionCategory::IdleNpc);
    // The reason names the observed condition and its age, in simulation time.
    assert_eq!(row.reason.id, IDLE_NPC_REASON);
    assert_eq!(
        row.reason.params.get("ship").map(String::as_str),
        Some(DRIFTER)
    );
    assert_eq!(
        row.reason.params.get("idle").map(String::as_str),
        Some("0:30")
    );
    // Opening it is a navigation to that hull and nothing else — no route, no
    // conversation, and therefore no order chosen on the operator's behalf.
    assert_eq!(
        row.target.ship.as_ref().map(|ship| ship.entity_id.as_str()),
        Some(drifter.as_str())
    );
    assert!(row.target.route.is_none());
    assert!(row.target.conversation.is_none());
    assert!(row.target.sender.is_none());

    // Neither of the two hulls that are not idle ever appears, however long the
    // probe runs; the age keeps climbing for the one that is.
    step(&mut app, 30);
    let rows = idle_rows_of(&mut app);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].reason.params.get("idle").map(String::as_str),
        Some("0:31")
    );
    assert_eq!(
        rows[0].id, row.id,
        "the identity holds while the spell does"
    );
    assert!(queue(&mut app).iter().all(|row| row
        .target
        .ship
        .as_ref()
        .is_none_or(|ship| ship.entity_id != picket && ship.entity_id != fleet)));
}

/// A paused world does not age. The seam is the one pause actually uses —
/// `Time<Virtual>` stops, which starves the fixed accumulator — so no fixed step
/// starts and the stopwatch has nothing to count.
#[test]
fn a_paused_simulation_does_not_advance_the_grace() {
    let mut app = seeded(DEFAULT_WORLD);
    let drifter = uuid_of(&mut app, DRIFTER);
    step(&mut app, 60);
    let before_tick = tick(&app);
    let before = spell_ticks(&app, &drifter).expect("the drifting hull is watched");

    app.world_mut().insert_resource(SimulationPaused(true));
    app.world_mut().resource_mut::<Time<Virtual>>().pause();
    for _ in 0..400 {
        app.update();
    }
    assert_eq!(
        tick(&app),
        before_tick,
        "a paused world takes no fixed step"
    );
    assert_eq!(
        spell_ticks(&app, &drifter),
        Some(before),
        "and therefore ages nothing"
    );
    // Four hundred paused frames is more than a thirty-second grace's worth of
    // wall clock; the row is still not there.
    assert!(idle_rows_of(&mut app).is_empty());

    app.world_mut().insert_resource(SimulationPaused(false));
    app.world_mut().resource_mut::<Time<Virtual>>().unpause();
    step(&mut app, 10);
    assert_eq!(
        spell_ticks(&app, &drifter),
        Some(before + 10),
        "and resumes from exactly where it stopped"
    );
}

// ── Authored settings ────────────────────────────────────────────────────────

/// The authored grace and band, end to end from the world file.
#[test]
fn an_authored_grace_and_band_are_honoured_from_the_world_file() {
    let mut app = seeded(AUTHORED_WORLD);
    let drifter = uuid_of(&mut app, DRIFTER);
    let settings = app
        .world()
        .resource::<phoenix::world::config::WorldConfig>()
        .gm_attention
        .clone();
    assert_eq!(settings.idle_npc_grace_secs, 2.0);
    assert_eq!(settings.idle_npc_band(), GmAttentionBand::Urgent);
    assert_eq!(settings.idle_grace_ticks(30.0), 60);

    let started = spell_ticks(&app, &drifter).expect("watched");
    assert!(started < 60);
    step(&mut app, 59 - started);
    assert_eq!(spell_ticks(&app, &drifter), Some(59));
    assert!(
        idle_rows_of(&mut app).is_empty(),
        "the authored boundary is two seconds, not thirty"
    );
    step(&mut app, 1);
    let rows = idle_rows_of(&mut app);
    assert_eq!(rows.len(), 1);
    // The authored band, not the system default.
    assert_eq!(rows[0].band, GmAttentionBand::Urgent);
    assert_eq!(
        rows[0].reason.params.get("idle").map(String::as_str),
        Some("0:02")
    );
}

/// An unusable authored value fails the world load naming the section and key,
/// rather than being quietly clamped onto a threshold nobody chose.
#[test]
fn an_unusable_authored_setting_fails_the_world_load_by_name() {
    let base = std::fs::read_to_string(AUTHORED_WORLD).unwrap();

    let negative = base.replace("idle_npc_grace_secs = 2.0", "idle_npc_grace_secs = -1.0");
    let error = phoenix::world::config::parse_world(&negative).unwrap_err();
    assert!(
        error.contains("[gm_attention]") && error.contains("idle_npc_grace_secs"),
        "{error}"
    );

    let zero = base.replace("idle_npc_grace_secs = 2.0", "idle_npc_grace_secs = 0.0");
    let error = phoenix::world::config::parse_world(&zero).unwrap_err();
    assert!(error.contains("idle_npc_grace_secs"), "{error}");

    let band = base.replace("idle_npc_band = \"urgent\"", "idle_npc_band = \"critical\"");
    let error = phoenix::world::config::parse_world(&band).unwrap_err();
    assert!(
        error.contains("[gm_attention]")
            && error.contains("idle_npc_band")
            && error.contains("'background'"),
        "{error}"
    );

    // And the shipped default when the table is absent altogether.
    let plain = std::fs::read_to_string(DEFAULT_WORLD).unwrap();
    let parsed = phoenix::world::config::parse_world(&plain).unwrap();
    assert_eq!(
        parsed.gm_attention.idle_npc_grace_secs,
        DEFAULT_IDLE_NPC_GRACE_SECS
    );
    assert_eq!(
        parsed.gm_attention.idle_npc_band(),
        GmAttentionBand::Background
    );
    assert!(!parsed.gm_attention.idle_npc_disabled);
}

/// The off switch silences this advisory and only this advisory: the world's
/// pending conversation still reaches the same queue.
#[test]
fn the_disable_flag_silences_idle_rows_without_touching_the_rest_of_the_queue() {
    let mut app = seeded(QUIET_WORLD);
    let drifter = uuid_of(&mut app, DRIFTER);
    // Well past the one-second grace this world authors, so an empty idle list
    // can only be the off switch.
    step(&mut app, 120);
    assert!(
        spell_ticks(&app, &drifter).is_some_and(|ticks| ticks > 30),
        "the stopwatch still runs; only the advisory is silent"
    );
    let rows = queue(&mut app);
    assert!(
        rows.iter()
            .all(|row| row.category != GmAttentionCategory::IdleNpc),
        "{rows:?}"
    );
    assert!(
        rows.iter()
            .any(|row| row.category == GmAttentionCategory::PendingComms),
        "the world's own conversation is still waiting: {rows:?}"
    );
}

// ── Lifecycle ────────────────────────────────────────────────────────────────

/// Giving the hull an order resolves its row; taking the order away again opens
/// a NEW spell with a new identity, so no snooze against the first can hide it.
/// Both halves go through shipped GM actions.
#[test]
fn an_order_resolves_the_row_and_a_later_idle_spell_is_a_fresh_occurrence() {
    let mut app = seeded(AUTHORED_WORLD);
    let drifter = uuid_of(&mut app, DRIFTER);
    step(&mut app, 70);
    let first = idle_rows_of(&mut app);
    assert_eq!(first.len(), 1);
    let first_id = first[0].id.clone();

    let sequence = enqueue(
        &mut app,
        GmAction::SetNpcDoctrine {
            target: drifter.clone(),
            doctrine: "hold-station".into(),
        },
    );
    assert_eq!(
        spell_ticks(&app, &drifter),
        None,
        "an order ends the spell outright"
    );
    assert!(
        idle_rows_of(&mut app).is_empty(),
        "and takes the occurrence with it"
    );

    // Reverse the order through the ordinary inverse, and the hull is back to
    // having nothing to do.
    enqueue(
        &mut app,
        GmAction::UndoGmAction {
            original: GmActionId::new(format!("idle-{sequence}")).unwrap(),
            original_operator: "gm-idle".into(),
            original_sequence: sequence,
            expected: GmAffectedField::NpcDoctrine {
                entity: drifter.clone(),
                before: None,
                after: Some("hold-station".into()),
            },
        },
    );
    let restarted = spell_ticks(&app, &drifter).expect("the hull falls idle again");
    assert!(
        restarted <= 10,
        "the grace restarts from zero, not from where it left off: {restarted}"
    );
    assert!(
        idle_rows_of(&mut app).is_empty(),
        "and it must serve the whole grace again"
    );

    step(&mut app, 70);
    let second = idle_rows_of(&mut app);
    assert_eq!(second.len(), 1);
    assert_ne!(
        second[0].id, first_id,
        "a later spell is a different occurrence"
    );
    assert_eq!(
        second[0].reason.params.get("idle").map(String::as_str),
        Some("0:02")
    );
}

/// Removal resolves the occurrence, and the watch forgets the hull rather than
/// keeping a wait nobody can act on.
#[test]
fn removing_the_ship_resolves_the_occurrence_and_forgets_its_wait() {
    let mut app = seeded(AUTHORED_WORLD);
    let drifter = uuid_of(&mut app, DRIFTER);
    step(&mut app, 70);
    assert_eq!(idle_rows_of(&mut app).len(), 1);

    enqueue(
        &mut app,
        GmAction::DespawnEntity {
            target: drifter.clone(),
        },
    );
    assert_eq!(
        spell_ticks(&app, &drifter),
        None,
        "a hull that left the world takes its wait with it"
    );
    assert!(idle_rows_of(&mut app).is_empty());
    assert!(watch(&app).is_empty());
}

// ── Saved and restored timing ────────────────────────────────────────────────

/// The stopwatch is captured, not re-seeded. A resume of a save taken one step
/// short of the boundary crosses it on its very next step; the run that was
/// never interrupted crosses it on the same step, which is what "deterministic"
/// means here.
#[test]
fn a_restored_save_resumes_the_grace_exactly_where_it_was_captured() {
    let mut app = seeded(AUTHORED_WORLD);
    let drifter = uuid_of(&mut app, DRIFTER);
    let started = spell_ticks(&app, &drifter).expect("watched");
    step(&mut app, 59 - started);
    assert_eq!(spell_ticks(&app, &drifter), Some(59));
    assert!(idle_rows_of(&mut app).is_empty(), "one step short");

    let saved = capture(app.world());
    assert_eq!(
        saved.gm_idle_npcs.spell(&drifter).map(|spell| spell.ticks),
        Some(59),
        "the save carries the wait, not a promise to start again"
    );

    // A fresh boot of the same world is a hull that has only just started
    // drifting; restoring the save is what puts it back one step from its
    // boundary.
    let mut resumed = seeded(AUTHORED_WORLD);
    assert!(
        spell_ticks(&resumed, &drifter).is_some_and(|ticks| ticks < 59),
        "a fresh boot has not served the grace"
    );
    restore(resumed.world_mut(), &saved);
    assert_eq!(spell_ticks(&resumed, &drifter), Some(59));
    assert!(
        idle_rows_of(&mut resumed).is_empty(),
        "a restore does not skip the last step either"
    );

    step(&mut resumed, 1);
    let rows = idle_rows_of(&mut resumed);
    assert_eq!(rows.len(), 1, "the very next step crosses the boundary");
    assert_eq!(
        rows[0].reason.params.get("idle").map(String::as_str),
        Some("0:02")
    );

    // The uninterrupted run crosses it on the same step, from the same wait.
    step(&mut app, 1);
    let straight = idle_rows_of(&mut app);
    assert_eq!(straight.len(), 1);
    assert_eq!(straight[0].id, rows[0].id);
    assert_eq!(straight[0].reason, rows[0].reason);
}
