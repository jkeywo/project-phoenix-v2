//! Loading a world into a **running** native host (issue #1326).
//!
//! # The problem this solves
//!
//! Until now a native host had to be told its world on the command line, and
//! the world was ingested by [`crate::boot::build`] before the `App` existed.
//! The browser host gets away with the same ordering — `wasm_init` throws to
//! unwind the JS stack, so it has to run last, after `wasm_load_world` — because
//! its pre-scenario lobby is an HTML panel over a canvas Bevy has not started
//! drawing to yet. A native host has no such panel: its lobby is the viewscreen,
//! and the viewscreen is the running `App`. So the world has to arrive *after*
//! the `App` does.
//!
//! # What arrives, and how
//!
//! Exactly what arrives on the browser: a
//! [`SelectScenario`](crate::core::messages::ClientMessage::SelectScenario) and a
//! [`SelectPlayerShip`](crate::core::messages::ClientMessage::SelectPlayerShip)
//! from any participant — a phone, a local Station pane, or (slice #1328) the
//! host's own on-screen picker — arbitrated first-valid-wins against the
//! published catalogue. The rule is [`crate::lobby::scenario_arbiter`], which is
//! a transcription of `gui/scenario-arbiter.js` rather than a second design, and
//! the catalogue is
//! [`ManifestSource::merged_catalog`](crate::delivery::serve::ManifestSource::merged_catalog),
//! which is the same [`build_merged_catalog`](crate::world::manifest::build_merged_catalog)
//! call `wasm_get_scenario_catalog` makes.
//!
//! # Why the load is the boot load
//!
//! The determinism claim rests on reuse, not on prose:
//!
//! * The ingest is [`crate::boot::ingest_world`] — literally the function
//!   [`crate::boot::build`] calls, over a [`BootPlan`](crate::boot::BootPlan)
//!   built by the same [`app::boot_plan`](crate::native_host::app::boot_plan).
//!   Ledger reset → read → validate → compile → abort-on-broken → native
//!   template gate → apply → eager record → freeze → insert `WorldConfig` +
//!   `PreCompiledScripts`, in that order, once, in one place.
//! * The hull, the seed precedence, the hull's own cache gate and the two ship
//!   resources come from
//!   [`app::install_world_selection`](crate::native_host::app::install_world_selection),
//!   which `build_native_host_app` also calls.
//! * The spawn pass is [`RuntimeWorldLoad`], whose chain is the `Startup`
//!   topological order stated explicitly — including the
//!   `compile_world_scripts < setup_world < spawn_world_entities` pin
//!   `server_app::registration` expresses with `.after`/`.before` edges, and
//!   whose flip once moved the authoritative digest. Because that chain is a
//!   DUPLICATE of a list that lives elsewhere, it is guarded structurally:
//!   `the_runtime_spawn_pass_cannot_silently_fall_behind_the_startup_chain`
//!   (`tests/native_host_lobby.rs`) reads both schedules out of a live app and
//!   asserts the runtime set covers `WorldPlugin`'s `Startup` chain, so a system
//!   added there cannot silently not-run here.
//! * The ids those spawns mint are minted from a [`WorldIdMint`] parked at tick
//!   0, then the live mint is restored — see [`park_mint`]. Without that, every
//!   world entity's id would carry the tick the operator happened to press the
//!   button on, and two hosts of one mission could not agree on a single uuid.
//!
//! # What the participants are told
//!
//! A world landing is not a private event. A phone that identified before the
//! pick was welcomed with the world-less lobby's FALLBACK roster and client
//! config, so [`apply_pending_world_load`] re-publishes a fresh `Welcome` (and
//! the `ShipManual` that always accompanies one) to everyone the moment the load
//! succeeds — see [`republish_loaded_world`], which also explains why the seats
//! are cleared first and why a second `Welcome` is safe. A load that is REFUSED
//! publishes the catalogue again instead, over a lobby put back to genuinely
//! world-less by [`unwind_failed_load`].
//!
//! What is *not* claimed: that a runtime-loaded host reaches `InProgress` on the
//! same tick a `--solo --world` host does. It does not, and it never could — the
//! mission starts when the crew ready up, which is already true of every crewed
//! boot today. `spawn_game_start_entities` mints on the phase-transition tick on
//! both paths.

use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;

use crate::core::messages::{ClientMessage, GamePhase, ServerMessage};
use crate::lobby::handler::Target;
use crate::lobby::scenario_arbiter::{self, ScenarioSelection, SelectionOutcome};
use crate::lobby::stations_config::ShipStations;
use crate::lobby::{InboundMessage, LobbyOutbox};
use crate::logging::{LogCat, LogFilterConfig};
use crate::native_host::app::{self, HullChoice, NativeHostError};
use crate::world::manifest::ScenarioCatalog;
use crate::world_id::{WorldIdMint, WorldIdMintState};

/// Everything this module does in `FixedUpdate`, as one ordering anchor.
///
/// `native_host::app` hangs `solo_auto_start` off it so a `--solo` host that
/// boots world-less starts the mission on the tick its world lands.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeWorldLoadSet;

/// The spawn pass a runtime world load runs, in place of `Startup`.
///
/// A schedule rather than a hand-rolled sequence of `run_system_once` calls
/// because `.chain()` is what inserts the deferred-command flush between each
/// pair — the same flush `Startup` gives them — and because the order is then
/// declared in one readable place instead of being implied by call order.
#[derive(ScheduleLabel, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeWorldLoad;

/// The catalogue a world-less host offers, published to every participant that
/// identifies and re-published whenever the selection moves.
#[derive(Resource, Clone, Debug, Default)]
pub struct LobbyScenarioCatalog(pub ScenarioCatalog);

/// The command-line answers a world-less host still has to honour when its world
/// finally arrives, plus the boot inputs the runtime ingest rebuilds its
/// [`BootPlan`](crate::boot::BootPlan) from.
///
/// `--ship` and `--seed` are accepted without `--world` precisely so a scripted
/// run can pin the hull and the seed and let the scenario be chosen from the
/// lobby; `--ship` still outranks whatever `SelectPlayerShip` locked, the way it
/// outranks the world's default hull today.
#[derive(Resource, Clone, Debug)]
pub struct LobbyBootSettings {
    /// `--ship`, if one was given. Wins over the arbitrated hull.
    ///
    /// And therefore **satisfies the hull half of the selection**: with one
    /// pinned, a scenario lock alone completes the pick and the world loads. The
    /// alternative was asking a participant for a `SelectPlayerShip` this host
    /// would then discard — a scripted `--lobby --ship X` run would hang waiting
    /// for a message whose content cannot matter. The catalogue phones fold
    /// reports it as the locked hull for the same reason: a picker offering a
    /// choice the host has already overruled is a lie the phone acts on.
    pub ship_path: Option<String>,
    /// `--seed`, which outranks the world's `[global] seed`.
    pub seed: Option<u64>,
    /// The raw `--log` spec, so the runtime plan's `log_filter` is the boot
    /// plan's.
    pub log_spec: String,
    /// `NativeHostConfig::deterministic`.
    pub deterministic: bool,
    /// `NativeHostConfig::surface`.
    pub surface: crate::boot::NativeRenderSurface,
}

/// What the arbiter has locked so far. Present only on a world-less host.
#[derive(Resource, Clone, Debug, Default)]
pub struct LobbySelection(pub ScenarioSelection);

/// A complete selection waiting to be ingested, written by
/// [`drain_scenario_selection`] and consumed by [`apply_pending_world_load`] in
/// the same tick.
#[derive(Resource, Clone, Debug)]
struct PendingWorldLoad {
    world_path: String,
    ship_path: Option<String>,
    curated_ships: Vec<String>,
}

/// Installed on every native host and inert on one that already has a world.
pub struct NativeWorldLoadPlugin;

impl Plugin for NativeWorldLoadPlugin {
    fn build(&self, app: &mut App) {
        // ── The spawn pass ────────────────────────────────────────────────
        //
        // Registered unconditionally so `run_schedule` never meets an absent
        // schedule, and never RUN unless a runtime load asks for it — which is
        // what keeps a `--world` host byte-for-byte unchanged.
        //
        // This is the one place the runtime order could drift from `Startup`'s,
        // so it is written as the **topological order `Startup` actually
        // produces**, not as a fresh design:
        //
        //  * `world::server`'s own `.chain()` — `insert_world_config_resource`,
        //    `insert_raw_world_source_resource`, `compile_world_scripts`,
        //    `spawn_world_entities`, `init_world_runtime`, `load_extra_worlds`.
        //  * `server_app::registration`'s three edges on `setup_world`:
        //    `.after(insert_world_config_resource)`,
        //    `.after(compile_world_scripts)`, `.before(spawn_world_entities)`.
        //    That last one is a determinism pin, not tidiness — `setup_world`
        //    (anonymous stars and planets) and `spawn_world_entities` (named and
        //    asteroid entities) both mint from the shared `WorldIdMint`, and
        //    letting a scheduling tie-break decide their order moved the
        //    authoritative digest once already.
        //  * `lobby::server::update_session_with_config`,
        //    `server::reference_grid::resolve_reference_grid_config` and
        //    `server::radar::spawn_viewscreen_radar_widgets` are unordered
        //    against the world chain at `Startup` and mint nothing, so their
        //    position here is free; they sit last because each reads the hull
        //    `install_world_selection` has just selected. The radar pass
        //    despawns first — `Startup` already ran the spawn once, against
        //    whatever hull the world-less lobby defaulted to, and re-running it
        //    bare would stack two sets of widgets on the viewscreen.
        //
        // The first two systems are native no-ops (`get_world_config` and
        // `BridgeWorldSource` are browser-side) and are included anyway: leaving
        // a system out because *today* it does nothing on this target is how the
        // two orders start to drift.
        app.add_systems(
            RuntimeWorldLoad,
            (
                crate::world::server::insert_world_config_resource,
                crate::world::server::insert_raw_world_source_resource,
                crate::world::server::compile_world_scripts,
                crate::server_app::setup_world,
                crate::world::server::spawn_world_entities,
                crate::world::server::init_world_runtime,
                crate::world::server::load_extra_worlds,
                crate::lobby::server::update_session_with_config,
                crate::server::reference_grid::resolve_reference_grid_config,
                crate::server::radar::despawn_viewscreen_radar_widgets,
                crate::server::radar::spawn_viewscreen_radar_widgets,
            )
                .chain(),
        );

        app.init_resource::<LobbySelection>().add_systems(
            FixedUpdate,
            (drain_scenario_selection, apply_pending_world_load)
                .chain()
                .in_set(NativeWorldLoadSet)
                // After the lobby, so a participant who identifies and picks in
                // the same tick has a session by the time the catalogue is
                // addressed to their token — `Target::Token` resolves through
                // `SessionManager`, and answering a token that does not exist
                // yet would drop the one message that participant is waiting
                // for.
                .after(crate::lobby::LobbySystemSet)
                // Before the simulation reads anything: a world that lands this
                // tick must be visible to `SimSet::Input` on the same tick, the
                // way a `Startup`-ingested one is visible to the first tick.
                // (`SimSet::Input` already orders itself after
                // `LobbySystemSet`, so this pair of edges cannot cycle.)
                .before(crate::sim_sets::SimSet::Input)
                .run_if(awaiting_world),
        );
    }
}

/// Whether this host is a world-less one that has not yet loaded a world.
///
/// A `--world` host never runs either system (it has a `WorldConfig` before the
/// first fixed step and no `LobbyScenarioCatalog` at all), and a host that has
/// loaded one stops running them the moment it does — a second `SelectScenario`
/// from a late phone is then the deliberate no-op `lobby::handler` already
/// documents.
fn awaiting_world(
    catalog: Option<Res<LobbyScenarioCatalog>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
) -> bool {
    catalog.is_some() && world_config.is_none()
}

/// Arbitrate inbound selection requests and publish the catalogue.
///
/// Runs only while the host has no world. Everything it decides goes through
/// [`crate::lobby::scenario_arbiter`]; this system owns only the plumbing —
/// which messages to look at, who to answer, and when the selection is complete
/// enough to load.
fn drain_scenario_selection(
    mut inbound: MessageReader<InboundMessage>,
    catalog: Res<LobbyScenarioCatalog>,
    settings: Option<Res<LobbyBootSettings>>,
    mut selection: ResMut<LobbySelection>,
    mut outbox: ResMut<LobbyOutbox>,
    mut commands: Commands,
    log: Option<Res<LogFilterConfig>>,
) {
    // `--ship` given without `--world` (issue #1326 makes that combination
    // legal) already outranks whatever `SelectPlayerShip` locks, so asking a
    // participant to send one anyway would be asking for a message the host
    // discards. A pinned hull therefore SATISFIES the hull half of the
    // selection: the scenario lock alone completes it, and the catalogue the
    // phones fold reports the pinned hull as the locked one so their picker
    // shows the decision that has actually been made rather than a live choice.
    let pinned_ship: Option<String> = settings.as_ref().and_then(|s| s.ship_path.clone());
    let mut changed = false;
    let mut greet: Vec<String> = Vec::new();

    for message in inbound.read() {
        match &message.msg {
            // A participant arriving before a world exists needs the catalogue
            // to pick from, exactly as `server.html`'s `sendCatalogTo` hands it
            // to a fresh datachannel.
            ClientMessage::Identify { token, .. } => greet.push(token.clone()),
            ClientMessage::SelectScenario { scenario_id } => {
                let (outcome, next) =
                    scenario_arbiter::select_scenario(&selection.0, &catalog.0, scenario_id);
                log_outcome(&log, outcome, "scenario", scenario_id);
                if outcome == SelectionOutcome::Accepted {
                    selection.0 = next;
                    changed = true;
                }
            }
            ClientMessage::SelectPlayerShip { template_path } => {
                let (outcome, next) =
                    scenario_arbiter::select_player_ship(&selection.0, &catalog.0, template_path);
                log_outcome(&log, outcome, "hull", template_path);
                if outcome == SelectionOutcome::Accepted {
                    selection.0 = next;
                    changed = true;
                }
            }
            _ => {}
        }
    }

    for token in greet {
        outbox.0.push((
            Target::Token(token),
            catalog_message(&catalog.0, &selection.0, pinned_ship.as_deref()),
        ));
    }
    if changed {
        outbox.0.push((
            Target::All,
            catalog_message(&catalog.0, &selection.0, pinned_ship.as_deref()),
        ));
    }

    // Complete when the arbiter locked both halves, OR when it locked the
    // scenario and `--ship` already supplied the other.
    let complete =
        selection.0.is_complete() || (selection.0.scenario().is_some() && pinned_ship.is_some());
    if !changed || !complete {
        return;
    }
    let Some(world_path) = scenario_arbiter::world_path_for(&catalog.0, &selection.0) else {
        return;
    };
    commands.insert_resource(PendingWorldLoad {
        world_path: world_path.to_string(),
        // `--ship` still outranks the lobby's pick, the way it outranks the
        // world's default hull on the `--world` path.
        ship_path: pinned_ship.or_else(|| selection.0.ship().map(str::to_string)),
        curated_ships: scenario_arbiter::curated_ships_for(&catalog.0, &selection.0),
    });
}

/// The catalogue message a phone folds through `gui/lobby-state.js`, identical
/// in shape to the one `server.html` synthesises before its own world load.
///
/// `pinned_ship` is `--ship`. It is reported as the locked hull because it *is*
/// the hull this host will fly whatever a phone picks, and a picker offering a
/// choice the host has already overruled is a lie the phone acts on.
fn catalog_message(
    catalog: &ScenarioCatalog,
    selection: &ScenarioSelection,
    pinned_ship: Option<&str>,
) -> ServerMessage {
    ServerMessage::ScenarioCatalog {
        scenarios: scenario_arbiter::catalog_wire(catalog),
        locked_scenario: selection.scenario().map(str::to_string),
        locked_ship: selection
            .ship()
            .map(str::to_string)
            .or_else(|| pinned_ship.map(str::to_string)),
    }
}

fn log_outcome(
    log: &Option<Res<LogFilterConfig>>,
    outcome: SelectionOutcome,
    kind: &str,
    value: &str,
) {
    match outcome {
        SelectionOutcome::Accepted => {
            crate::pinfo!(log, LogCat::Lobby, "{kind} locked: {value}")
        }
        SelectionOutcome::Ignored => {
            crate::pdebug!(
                log,
                LogCat::Lobby,
                "{kind} already locked, ignoring {value}"
            )
        }
        SelectionOutcome::Rejected => crate::pwarn!(
            log,
            LogCat::Lobby,
            "{kind} {value} is not in this host's catalogue, refused"
        ),
    }
}

/// Ingest the selected world into the running `World`.
///
/// Exclusive because it inserts resources the whole simulation reads and then
/// runs a schedule; both need `&mut World`, and both must happen inside one
/// fixed step so that nothing observes a half-loaded world.
fn apply_pending_world_load(world: &mut World) {
    let Some(pending) = world.remove_resource::<PendingWorldLoad>() else {
        return;
    };
    match load_selected_world(world, &pending) {
        Ok(()) => republish_loaded_world(world),
        Err(error) => {
            // Refuse the selection, do not refuse the host. An operator who
            // picked a scenario whose content is broken should be able to pick
            // another one — the boot-time equivalent (`--world` naming a broken
            // world) fails at the prompt because there is a prompt to fail at;
            // here there is a lobby to go back to.
            let log = world.get_resource::<LogFilterConfig>().cloned();
            crate::perror!(
                log,
                LogCat::World,
                "loading {} failed, staying in the lobby: {error}",
                pending.world_path
            );
            unwind_failed_load(world);
            if let Some(mut selection) = world.get_resource_mut::<LobbySelection>() {
                selection.0 = ScenarioSelection::default();
            }
            let pinned = world
                .get_resource::<LobbyBootSettings>()
                .and_then(|s| s.ship_path.clone());
            if let (Some(catalog), Some(selection)) = (
                world.get_resource::<LobbyScenarioCatalog>().cloned(),
                world.get_resource::<LobbySelection>().cloned(),
            ) {
                let message = catalog_message(&catalog.0, &selection.0, pinned.as_deref());
                if let Some(mut outbox) = world.get_resource_mut::<LobbyOutbox>() {
                    outbox.0.push((Target::All, message));
                }
            }
        }
    }
}

/// Put the lobby back to genuinely world-less after a refused selection.
///
/// Three things have to go, and the third is the one that is easy to miss:
///
///  * `WorldConfig` and `PreCompiledScripts`. `install_world_selection` can fail
///    AFTER [`crate::boot::ingest_world`] has already inserted them (an uncached
///    hull, a hull with no `[[station]]` blocks), and leaving them behind would
///    take [`awaiting_world`] false — a host holding a world it never spawned,
///    unable to accept another pick.
///  * The **content ledger**. `ingest_world` froze it over the refused world's
///    file set, and `install_world_selection` froze it again after re-recording
///    the hull — both before either failure point. A frozen ledger is the input
///    to [`crate::content_ledger::frozen_or_live`], which is what
///    `snapshot::versions` answers a fleet peer's content check with and what a
///    save is bound to; left alone it would go on answering for a world this
///    host does not have and never spawned. `reset` is the whole undo rather
///    than a restore because `ingest_world` opens every attempt — including the
///    next, successful one — with exactly this `reset`, so an emptied, unfrozen
///    ledger is precisely the state the next pick starts from. What it discards
///    is the template preload's records, which the next load's own eager record
///    re-reads from disk regardless.
///
/// Nothing has spawned at either failure point, so there are no entities to
/// unwind alongside them.
fn unwind_failed_load(world: &mut World) {
    world.remove_resource::<crate::world::config::WorldConfig>();
    world.remove_resource::<crate::world::server::PreCompiledScripts>();
    crate::content_ledger::reset();
}

/// Tell every connected participant about the world that just loaded.
///
/// # Why a second `Welcome` rather than nothing
///
/// A phone that identified BEFORE the pick was welcomed by a host that had no
/// world, so `update_session_with_config` answered from its two hard-coded
/// fallbacks: `ShipStations` from `load_ship_config_from_disk`'s **battleship**,
/// and the client config from `alliance_cruiser` (the literal path it uses when
/// no `SelectedShipResource` has been installed). Those are the numbers and the
/// seat list that phone mounted its consoles against, and they belong to two
/// hulls, neither of them the one about to fly. The load has just replaced both
/// with the chosen hull's, and nothing else on this path would ever say so —
/// `gui/lobby-state.js`'s fully-locked-catalogue branch resumes the lobby on the
/// *stale* `Welcome`, because that branch was written for issue #756's round-2
/// world REUSE, where the roster genuinely has not moved. A native runtime load
/// is round one, and the browser's round one re-welcomes for free: its phones
/// are welcomed by a Bevy app that does not exist until `wasm_init`, i.e. until
/// after the world is loaded.
///
/// The consequence of staying silent is not cosmetic. Consoles are mounted for
/// the wrong hull with the wrong authored numbers, and
/// `handler::handle_select_station` validates a claim against the REAL roster —
/// so a phone tapping a seat it can see is silently ignored.
///
/// # Why the seats are cleared first
///
/// The roster is being replaced wholesale, so any seat claimed against the
/// pre-load fallback was claimed on a ship this host is not flying. That is the
/// same situation issue #756's `handle_return_to_lobby` is in when a new round
/// picks a new hull, and it gets the same three-line answer: drop the seats, the
/// lobby-chosen ratings and the per-token eligibility reports, and let every
/// client re-report against the hull that is actually loaded. No separate
/// `StationAssigned` broadcast is needed — the `Welcome` below carries the whole
/// cleared roster to everyone at once.
///
/// # Idempotence
///
/// A second `Welcome` is exactly what an already-connected client is built to
/// take: `gui/lobby-state.js`'s `replaceFrom` REPLACES phase, roster, players
/// and ship config rather than merging, and its `Welcome` arm pushes
/// `MOUNT_CONSOLES` unconditionally — "Emit it every time, even when the values
/// equal the previous Welcome" — followed by `REBUILD_STATIONS`. The `ShipManual`
/// that follows is the message `handle_identify_system` already pairs with every
/// `Welcome` it produces (issue #772), and it is re-sent for this one's reason:
/// it is built from the selected hull and the selected hull has just changed.
fn republish_loaded_world(world: &mut World) {
    {
        let Some(mut sessions) = world.get_resource_mut::<crate::lobby::Sessions>() else {
            return;
        };
        sessions.0.clear_all_stations();
        sessions.0.clear_all_pending_ratings();
        sessions.0.clear_all_eligibility();
    }

    // The ratings the `Welcome` reports, resolved the way
    // `handle_identify_system` resolves them: the live ship's if it has spawned,
    // else whatever the lobby has pending.
    let mut live_ratings = world.query_filtered::<
        &crate::ship_plugin::ActiveStationRatings,
        With<crate::server_app::LocalShip>,
    >();
    let station_ratings = match live_ratings.single(world) {
        Ok(ratings) => ratings.0.clone(),
        Err(_) => world
            .resource::<crate::lobby::Sessions>()
            .0
            .pending_ratings()
            .clone(),
    };

    let phase = world.resource::<State<GamePhase>>().get().clone();
    let world_data = world
        .get_resource::<crate::lobby::WorldResource>()
        .map(|w| w.0.clone());
    let stations = world.resource::<ShipStations>().clone();
    let ship_config = world
        .resource::<crate::lobby::server::ShipClientConfigResource>()
        .0
        .clone();
    let manual = world
        .resource::<crate::lobby::server::ShipManualResource>()
        .0
        .clone();
    let welcome = crate::lobby::handler::welcome_message(
        &world.resource::<crate::lobby::Sessions>().0,
        &phase,
        world_data.as_ref(),
        &stations,
        &ship_config,
        &station_ratings,
    );

    let mut outbox = world.resource_mut::<LobbyOutbox>();
    outbox.0.push((Target::All, welcome));
    outbox
        .0
        .push((Target::All, ServerMessage::ShipManual { manual }));
}

/// The whole runtime load, as one fallible step.
fn load_selected_world(
    world: &mut World,
    pending: &PendingWorldLoad,
) -> Result<(), NativeHostError> {
    let settings = world
        .get_resource::<LobbyBootSettings>()
        .cloned()
        .unwrap_or(LobbyBootSettings {
            ship_path: None,
            seed: None,
            log_spec: String::new(),
            deterministic: false,
            surface: crate::boot::NativeRenderSurface::Contract,
        });

    // Step one: the boot ingest, unchanged and unforked.
    let plan = app::boot_plan(
        Some(&pending.world_path),
        &settings.log_spec,
        settings.deterministic,
        settings.surface,
    );
    crate::boot::ingest_world(world, &plan).map_err(NativeHostError::Boot)?;

    // Step two: the hull, the seed and the two ship resources — again the same
    // function `build_native_host_app` calls.
    let world_config = world
        .resource::<crate::world::config::WorldConfig>()
        .clone();
    let sim_rng = app::install_world_selection(
        world,
        &world_config,
        &HullChoice {
            world_label: &pending.world_path,
            ship_path: pending.ship_path.as_deref(),
            curated_ships: &pending.curated_ships,
            seed: settings.seed,
        },
    )?;
    // The seed the world authored (or `--seed` overrode) replaces the OS draw
    // `add_simulation_plugins_with` left behind. Nothing has consumed the
    // stream: the simulation sets are gated on `GamePhase::InProgress` and this
    // host has been sitting in `Lobby`.
    world.insert_resource(sim_rng);

    // `update_session_with_config` recomputes the station roster only while it
    // is empty — deliberately, so a running mission's roster is never rewritten
    // under it. A world-less lobby has already filled it from
    // `load_ship_config_from_disk`'s fallback, so clear it here and let the
    // chain below fill it from the hull that was actually chosen.
    world.resource_mut::<ShipStations>().stations.clear();

    // Step three: the spawn pass, with the mint parked at tick 0 so the ids it
    // hands out are the ids a `Startup` ingest would have handed out.
    let restore = park_mint(world);
    world.run_schedule(RuntimeWorldLoad);
    restore_mint(world, restore);
    Ok(())
}

/// Park the [`WorldIdMint`] at tick 0, returning the state to put back.
///
/// A boot ingest spawns the world from `Startup`, where the mint is still at its
/// `Default` — tick 0, every sequence 0 — so every world entity's id renders as
/// `…-0-<seq>`. A runtime ingest happens at tick N, and `begin_tick` only resets
/// the sequences when the tick *changes*, so without this the same authored
/// world would produce a different set of uuids depending on how long the
/// operator spent choosing it. Those uuids are folded into the authoritative
/// digest by name (`sim_digest::fold_entity_namespace` folds the rendered id),
/// they key a snapshot's entity matching, and in a fleet they have to agree
/// across hosts — none of which can depend on human reaction time.
///
/// The live state is restored afterwards rather than left at zero, so anything
/// that already minted on tick N keeps its sequence and cannot collide with an
/// id minted after the load.
fn park_mint(world: &mut World) -> Option<WorldIdMintState> {
    let live = world.get_resource::<WorldIdMint>()?.state();
    world.insert_resource(WorldIdMint::default());
    Some(live)
}

/// Put back what [`park_mint`] took.
fn restore_mint(world: &mut World, saved: Option<WorldIdMintState>) {
    if let Some(state) = saved {
        world.insert_resource(WorldIdMint::from_state(state));
    }
}
