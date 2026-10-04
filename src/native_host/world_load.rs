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
//! [`SelectShipSlot`](crate::core::messages::ClientMessage::SelectShipSlot) and
//! [`SelectPlayerShip`](crate::core::messages::ClientMessage::SelectPlayerShip)
//! from any participant — a phone, a local Station pane, or (slice #1328) the
//! host's own on-screen picker — arbitrated first-valid-wins against the
//! published catalogue. A compatibility-synthesized singleton slot is claimed
//! by the scenario winner without an extra message. The rule is
//! [`crate::lobby::scenario_arbiter`], which is a transcription of
//! `gui/scenario-arbiter.js` rather than a second design, and the catalogue is
//! [`ManifestSource::merged_catalog`](crate::delivery::serve::ManifestSource::merged_catalog),
//! which is the same [`build_merged_catalog`](crate::world::manifest::build_merged_catalog)
//! call `wasm_get_scenario_catalog` makes.
//!
//! # Why the load is the boot load
//!
//! The determinism claim rests on reuse, not on prose:
//!
//! * Reader ingestion shares [`crate::boot::prepare_world_ingest`] and its
//!   installation with the boot compatibility wrapper. Preparation retains parsed
//!   config, compiled scripts, sound catalogue and content-ledger inputs.
//! * Hull and seed preparation shares [`app::prepare_world_selection`] with
//!   boot. Slot claims and optional standalone GM binding are validated before
//!   any live resources are installed. Commitment then consumes these values.
//! * The spawn pass is [`RuntimeWorldLoad`], registered by the same
//!   [`world::materialization::register`](crate::world::materialization::register)
//!   that `WorldPlugin` uses for `Startup`. Both schedules therefore contain the
//!   same compile → anonymous spawn → named spawn → init → layer-load chain,
//!   as ordinary systems preserving cross-plugin ordering edges. Only the
//!   native hull-dependent roster/reference-grid/radar tail is registered here.
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
//! publishes the catalogue again instead, while preparation leaves the live World unchanged.
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
use crate::lobby::{InboundMessage, LobbyOutbox, PlayerDisconnected};
use crate::logging::{LogCat, LogFilterConfig};
use crate::native_host::app::{self, HullChoice, NativeHostError};
use crate::sim_rng::{SeedSource, SimRng};
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

/// Claimant holding the selected authored slot while the native host remains
/// in its pre-world lobby. Kept beside (rather than inside) `ScenarioSelection`
/// so the published lock never leaks a reconnect credential.
#[derive(Resource, Clone, Debug, Default)]
struct LobbySlotClaim(Option<String>);

/// A complete selection waiting to be ingested, written by
/// [`drain_scenario_selection`] and consumed by [`apply_pending_world_load`] in
/// the same tick.
#[derive(Resource, Clone, Debug)]
struct PendingWorldLoad {
    world_path: String,
    ship_path: Option<String>,
    slot_id: Option<String>,
    claimant: Option<String>,
    curated_ships: Vec<String>,
}

/// A validated roster waiting for the Lobby exit that makes it immutable.
/// World ingestion is deliberately earlier than launch: keeping this separate
/// prevents a loaded-but-still-waiting lobby from claiming its roster has
/// already crossed the mission freeze boundary.
#[derive(Resource)]
struct PendingSlotFreeze(crate::ship_slots::FrozenShipSlots);

/// Installed on every native host. Selection is inert once a world has loaded;
/// returning crew still receive that retained world's complete lobby projection.
pub struct NativeWorldLoadPlugin;

impl Plugin for NativeWorldLoadPlugin {
    fn build(&self, app: &mut App) {
        // ReturnToLobby already cleared seats/readies and queued its reliable
        // acknowledgement. Native hosts retain their selected world, so there
        // is no subsequent scenario pick to release the client's waiting
        // overlay. Re-Welcome on the accepted phase edge, after those clears,
        // without resetting the world or changing the browser's selection flow.
        for exited in [GamePhase::GameOver, GamePhase::InProgress] {
            app.add_systems(
                OnTransition {
                    exited,
                    entered: GamePhase::Lobby,
                },
                publish_world_welcome.run_if(resource_exists::<crate::world::config::WorldConfig>),
            );
        }
        app.add_systems(OnExit(GamePhase::Lobby), freeze_selected_ship_slots);
        // The same authoritative pass as Startup, in this schedule rather
        // than behind a nested schedule that would hide cross-plugin edges.
        crate::world::materialization::register(app, RuntimeWorldLoad);
        // Startup already ran these hull-dependent systems against the empty
        // lobby's fallback hull. Selection replaces that input, so rerun them
        // after materialization and replace the old radar widgets first.
        app.add_systems(
            RuntimeWorldLoad,
            (
                crate::lobby::server::update_session_with_config,
                crate::server::reference_grid::resolve_reference_grid_config,
                crate::server::radar::despawn_viewscreen_radar_widgets,
                crate::server::radar::spawn_viewscreen_radar_widgets,
            )
                .chain()
                .after(crate::world::materialization::WorldMaterialization),
        );

        app.init_resource::<LobbySelection>()
            .init_resource::<LobbySlotClaim>()
            .add_systems(
                FixedUpdate,
                (
                    drain_scenario_selection.run_if(awaiting_world),
                    apply_pending_world_load.run_if(awaiting_world),
                    greet_catalogue,
                )
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
                    // And before the outbox drain, so the re-`Welcome`
                    // [`republish_loaded_world`] queues reaches the wire on the tick
                    // the world lands rather than on whichever tick the executor's
                    // ambiguity resolution happens to put the drain after. This is
                    // the same edge `server_app::registration` gives
                    // `refresh_caches_on_midgame_reconnect`, and for the same
                    // reason. It cannot cycle: `drain_lobby_outbox` orders itself
                    // only `.after(tick_countdown)`, which is *inside*
                    // `LobbySystemSet` — already upstream of this set.
                    .before(crate::lobby::server::drain_lobby_outbox)
                    .run_if(resource_exists::<LobbyScenarioCatalog>),
            );
    }
}

/// Commit the already-validated authored roster at the phase boundary all
/// launch paths share: ordinary crew readiness, the viewscreen control, and a
/// GM/start grant all eventually leave `Lobby` through this schedule.
fn freeze_selected_ship_slots(world: &mut World) {
    if let Some(pending) = world.remove_resource::<PendingSlotFreeze>() {
        world.insert_resource(pending.0);
    }
}

/// Whether this host is a world-less one that has not yet loaded a world.
///
/// A `--world` host never runs either system (it has a `WorldConfig` before the
/// first fixed step and no `LobbyScenarioCatalog` at all), and a host that has
/// loaded one stops running the selection/load pair the moment it does — a second `SelectScenario`
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
    mut disconnected: MessageReader<PlayerDisconnected>,
    catalog: Res<LobbyScenarioCatalog>,
    settings: Option<Res<LobbyBootSettings>>,
    mut selection: ResMut<LobbySelection>,
    mut slot_claim: ResMut<LobbySlotClaim>,
    mut outbox: ResMut<LobbyOutbox>,
    mut commands: Commands,
    log: Option<Res<LogFilterConfig>>,
    native_role: Option<Res<crate::native_host::session_role::NativeSessionRoleState>>,
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

    for departure in disconnected.read() {
        if slot_claim.0.as_deref() == Some(departure.token.as_str()) {
            selection.0.slot_id = None;
            selection.0.template_path = None;
            slot_claim.0 = None;
            changed = true;
        }
    }

    for message in inbound.read() {
        match &message.msg {
            ClientMessage::SelectScenario { scenario_id } => {
                let (outcome, next) =
                    scenario_arbiter::select_scenario(&selection.0, &catalog.0, scenario_id);
                log_outcome(&log, outcome, "scenario", scenario_id);
                if outcome == SelectionOutcome::Accepted {
                    selection.0 = next;
                    // Every pre-#1518 world is represented by one synthesized
                    // slot in the catalogue. That compatibility stage is not a
                    // new question for the operator: reserve it for the sender
                    // that won the scenario race, so the historical
                    // SelectScenario -> SelectPlayerShip pair remains complete.
                    // A genuinely multi-slot scenario still stops here and
                    // requires an explicit, claimant-bound slot request.
                    if let Some(slot_id) = scenario_arbiter::find_scenario(&catalog.0, scenario_id)
                        .and_then(|entry| {
                            (entry.slots.len() == 1).then(|| entry.slots[0].id.clone())
                        })
                    {
                        let (slot_outcome, next) =
                            scenario_arbiter::select_ship_slot(&selection.0, &catalog.0, &slot_id);
                        debug_assert_eq!(slot_outcome, SelectionOutcome::Accepted);
                        if slot_outcome == SelectionOutcome::Accepted {
                            selection.0 = next;
                            slot_claim.0 = Some(message.token.clone());
                        }
                    }
                    changed = true;
                }
            }
            ClientMessage::SelectPlayerShip { template_path } => {
                if selection.0.slot().is_some()
                    && slot_claim.0.as_deref() != Some(message.token.as_str())
                {
                    log_outcome(&log, SelectionOutcome::Rejected, "hull", template_path);
                    continue;
                }
                let (outcome, next) =
                    scenario_arbiter::select_player_ship(&selection.0, &catalog.0, template_path);
                log_outcome(&log, outcome, "hull", template_path);
                if outcome == SelectionOutcome::Accepted {
                    selection.0 = next;
                    changed = true;
                }
            }
            ClientMessage::SelectShipSlot { slot_id } => {
                if slot_claim
                    .0
                    .as_deref()
                    .is_some_and(|held| held != message.token)
                {
                    log_outcome(&log, SelectionOutcome::Rejected, "slot", slot_id);
                    continue;
                }
                let (outcome, next) =
                    scenario_arbiter::select_ship_slot(&selection.0, &catalog.0, slot_id);
                log_outcome(&log, outcome, "slot", slot_id);
                if outcome == SelectionOutcome::Accepted {
                    selection.0 = next;
                    slot_claim.0 = Some(message.token.clone());
                    changed = true;
                }
            }
            ClientMessage::ReleaseShipSlot
                if slot_claim.0.as_deref() == Some(message.token.as_str()) =>
            {
                selection.0.slot_id = None;
                selection.0.template_path = None;
                slot_claim.0 = None;
                changed = true;
            }
            _ => {}
        }
    }

    if changed {
        outbox.0.push((
            Target::All,
            catalog_message(&catalog.0, &selection.0, pinned_ship.as_deref()),
        ));
    }

    // Complete when the arbiter locked both halves, OR when it locked the
    // scenario and `--ship` already supplied the other.
    let fleet_gm = native_role.as_ref().is_some_and(|state| {
        matches!(
            state.role(),
            crate::native_host::session_role::NativeSessionRole::FleetGameMaster
                | crate::native_host::session_role::NativeSessionRole::StandaloneGameMaster
        )
    });
    let scenario_needs_slot = !fleet_gm
        && selection
            .0
            .scenario()
            .and_then(|id| scenario_arbiter::find_scenario(&catalog.0, id))
            .is_some_and(|entry| !entry.slots.is_empty());
    let slot_complete = !scenario_needs_slot || selection.0.slot().is_some();
    let complete = fleet_gm && selection.0.scenario().is_some()
        || (selection.0.is_complete() && slot_complete)
        || (selection.0.scenario().is_some() && slot_complete && pinned_ship.is_some());
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
        ship_path: (!fleet_gm)
            .then(|| pinned_ship.or_else(|| selection.0.ship().map(str::to_string)))
            .flatten(),
        slot_id: (!fleet_gm)
            .then(|| selection.0.slot().map(str::to_string))
            .flatten(),
        claimant: (!fleet_gm).then(|| slot_claim.0.clone()).flatten(),
        curated_ships: scenario_arbiter::curated_ships_for(&catalog.0, &selection.0),
    });
}

/// Rehydrate a fresh/reconnecting phone before or after world selection. The
/// selection systems stop at world load; the catalogue's pack roster does not.
fn greet_catalogue(
    mut inbound: MessageReader<InboundMessage>,
    catalog: Res<LobbyScenarioCatalog>,
    selection: Res<LobbySelection>,
    settings: Option<Res<LobbyBootSettings>>,
    mut outbox: ResMut<LobbyOutbox>,
) {
    let pinned = settings.as_ref().and_then(|s| s.ship_path.as_deref());
    for message in inbound.read() {
        if let ClientMessage::Identify { token, .. } = &message.msg {
            outbox.0.push((
                Target::Token(token.clone()),
                catalog_message(&catalog.0, &selection.0, pinned),
            ));
        }
    }
}

/// What this host is publishing about its own selection, in the one place both
/// audiences read it from.
///
/// There are two audiences and they must never be told different things: every
/// phone in the room, through
/// [`ServerMessage::ScenarioCatalog`](crate::core::messages::ServerMessage::ScenarioCatalog),
/// and the viewscreen's own picker, through
/// [`ScenarioPanelPayload`](crate::native_host::host_lobby::ScenarioPanelPayload)
/// (issue #1328). Both carry the same typed snapshot, built once — so
/// "the viewscreen shows a different catalogue from the phones" is not a
/// question this host can be asked.
pub(crate) struct PublishedCatalog(crate::core::messages::ScenarioCatalogPayload);

/// Render what this host is publishing.
///
/// `pinned_ship` is `--ship`. It is reported as the locked hull because it *is*
/// the hull this host will fly whatever a phone picks, and a picker offering a
/// choice the host has already overruled is a lie whoever is looking at it acts
/// on.
///
/// Which makes the precedence load-bearing rather than cosmetic: `pinned_ship`
/// FIRST, exactly as [`drain_scenario_selection`] resolves the hull it actually
/// loads (`pinned_ship.or_else(|| selection.ship())`). Reported the other way
/// round, `--lobby --ship B` with a phone picking A would fly B while telling
/// every phone the locked hull is A — the precise lie this field exists to
/// prevent, on the one combination where the two answers differ.
pub(crate) fn published_catalog(
    catalog: &ScenarioCatalog,
    selection: &ScenarioSelection,
    pinned_ship: Option<&str>,
) -> PublishedCatalog {
    PublishedCatalog(crate::delivery::payload::catalogue_snapshot(
        crate::delivery::payload::catalog_payload(catalog),
        &crate::entities::config_cache::active_packs(),
        selection.scenario().map(str::to_string),
        selection.slot().map(str::to_string),
        pinned_ship
            .map(str::to_string)
            .or_else(|| selection.ship().map(str::to_string)),
    ))
}

impl PublishedCatalog {
    /// The catalogue message a phone folds through `gui/lobby-state.js`,
    /// identical in shape to the one `server.html` synthesises before its own
    /// world load.
    ///
    /// `pub(crate)` since issue #1366: an accepted mod pack widens the catalogue
    /// from inside `native_host::host_lobby`, and every phone in the room has to
    /// be told through the SAME derivation the viewscreen's picker reads. A
    /// second `ScenarioCatalog` assembled at that call site would be exactly the
    /// "the viewscreen and the phones are looking at two catalogues" split this
    /// type exists to close.
    pub(crate) fn wire(self) -> ServerMessage {
        ServerMessage::ScenarioCatalog(self.0)
    }

    /// The same catalogue as the viewscreen picker's snapshot (issue
    /// #1328), plus the one thing a phone has no use for: whether a world has
    /// landed and closed the picker for good.
    pub(crate) fn surface(
        self,
        locked: bool,
    ) -> crate::native_host::host_lobby::ScenarioPanelPayload {
        crate::native_host::host_lobby::ScenarioPanelPayload {
            catalog: self.0,
            ship_required: true,
            locked,
        }
    }
}

/// [`published_catalog`], as the wire message.
fn catalog_message(
    catalog: &ScenarioCatalog,
    selection: &ScenarioSelection,
    pinned_ship: Option<&str>,
) -> ServerMessage {
    published_catalog(catalog, selection, pinned_ship).wire()
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
            if let Some(mut selection) = world.get_resource_mut::<LobbySelection>() {
                selection.0 = ScenarioSelection::default();
            }
            if let Some(mut claim) = world.get_resource_mut::<LobbySlotClaim>() {
                claim.0 = None;
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
/// picks a new hull, and it gets the same four-line answer: drop the ready
/// flags, the seats, the lobby-chosen ratings and the per-token eligibility
/// reports, and let every client re-report against the hull that is actually
/// loaded.
///
/// The ready flags are the one of the four that is easy to leave out and the one
/// with teeth. `SessionManager::all_ready` ignores seats entirely — it asks only
/// whether every connected non-spectator is ready — so a ready flag set against
/// the pre-load fallback roster survives the seat wipe and can start a mission
/// with a crew holding no stations at all. Today the shipped phone client cannot
/// reach that state (it readies from a console it has already claimed, and a
/// runtime load is round one), but "cannot reach it through this client" is not
/// the claim this function makes: it claims parity with `handle_return_to_lobby`,
/// and parity means all four.
///
/// So the per-player `ReadyChanged { ready: false }` broadcasts come with them,
/// exactly as `handle_return_to_lobby` emits them. No separate `StationAssigned`
/// broadcast is needed and none is sent — the `Welcome` below carries the whole
/// cleared roster, ready flags included, to everyone at once. The `ReadyChanged`
/// pair is not carrying the state (the `Welcome` already does): it is the
/// `REDUCER_EFFECTS.READY_CHANGED` edge `gui/lobby-state.js` raises only from
/// that arm, which is what a console's own ready control redraws off.
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
    let roster: Vec<String> = {
        let Some(mut sessions) = world.get_resource_mut::<crate::lobby::Sessions>() else {
            return;
        };
        // Captured before the wipe, because the `ReadyChanged` broadcasts below
        // are per player and `handle_return_to_lobby` sends one for everybody on
        // the roster, not only for whoever happened to be ready.
        let roster = sessions
            .0
            .players()
            .iter()
            .map(|p| p.token.clone())
            .collect();
        sessions.0.reset_ready();
        sessions.0.clear_all_stations();
        sessions.0.clear_all_pending_ratings();
        sessions.0.clear_all_eligibility();
        roster
    };

    let mut outbox = world.resource_mut::<LobbyOutbox>();
    for token in roster {
        outbox.0.push((
            Target::All,
            ServerMessage::ReadyChanged {
                token,
                ready: false,
            },
        ));
    }
    publish_world_welcome(world);
}

/// Publish the selected world's complete client projection after materializing
/// it or returning to its retained lobby. This is presentation only: the caller
/// owns any Session reset, and no entities, scripts, clocks or identities reset.
/// Welcome clears the client's scenario wait and rebuilds the cleared roster;
/// ShipManual restores its paired hull-specific client state.
fn publish_world_welcome(world: &mut World) {
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
        world.resource::<crate::gm_roster::GmRoster>().operators(),
    );

    let mut outbox = world.resource_mut::<LobbyOutbox>();
    outbox.0.push((Target::All, welcome));
    outbox
        .0
        .push((Target::All, ServerMessage::ShipManual { manual }));
}

/// All recoverable decisions for a native load, before touching the live World.
struct PreparedNativeWorldLoad {
    ingest: crate::boot::PreparedWorldIngest,
    hull: Option<app::PreparedWorldSelection>,
    rng: Option<SimRng>,
    frozen_slots: Option<crate::ship_slots::FrozenShipSlots>,
    pending_slots: Option<PendingSlotFreeze>,
    gm: Option<crate::gm_solo::PreparedStandaloneGameMaster>,
}

fn prepare_selected_world(
    world: &World,
    pending: &PendingWorldLoad,
) -> Result<PreparedNativeWorldLoad, NativeHostError> {
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
    let mut ingest = crate::boot::prepare_world_ingest(&plan).map_err(NativeHostError::Boot)?;

    if !pending.curated_ships.is_empty() {
        let curated = crate::ship_slots::curate_ship_slots(
            &ingest.config().ship_slots,
            &pending.curated_ships,
        )
        .map_err(NativeHostError::Ship)?;
        ingest.config_mut().ship_slots = curated;
    }

    let native_role = world
        .get_resource::<crate::native_host::session_role::NativeSessionRoleState>()
        .map(|state| state.role())
        .unwrap_or(crate::native_host::session_role::NativeSessionRole::ShipHost);
    let fleet_gm = matches!(
        native_role,
        crate::native_host::session_role::NativeSessionRole::FleetGameMaster
            | crate::native_host::session_role::NativeSessionRole::StandaloneGameMaster
    );

    // Step two: only ship hosts install a chosen local hull. A GM owns no
    // ship: standalone slots or the adopted fleet topology supply the ships.
    let world_config = ingest.config().clone();
    let mut frozen_slots = None;
    let mut pending_slots = None;
    if native_role == crate::native_host::session_role::NativeSessionRole::StandaloneGameMaster {
        frozen_slots = Some(
            crate::ship_slots::ShipSlotReservations::default()
                .freeze(&world_config.effective_ship_slots())
                .expect("empty reservations are confirmed"),
        );
    } else if fleet_gm && !world_config.ship_slots.is_empty() {
        if let Some(roster) = world.get_resource::<crate::lockstep::FleetRoster>() {
            let frozen = crate::ship_slots::FrozenShipSlots::from_fleet_roster(
                &world_config.ship_slots,
                roster,
            )
            .map_err(NativeHostError::Ship)?;
            frozen_slots = Some(frozen);
        }
    }
    let hull = if fleet_gm {
        None
    } else {
        Some(app::prepare_world_selection(
            world,
            &world_config,
            &HullChoice {
                world_label: &pending.world_path,
                ship_path: pending.ship_path.as_deref(),
                curated_ships: &pending.curated_ships,
                seed: settings.seed,
            },
        )?)
    };
    let rng = if fleet_gm {
        Some(match (settings.seed, world_config.global.seed) {
            (Some(seed), _) => SimRng::new(seed, SeedSource::Cli),
            (None, Some(seed)) => SimRng::new(seed, SeedSource::World),
            (None, None) => SimRng::random(),
        })
    } else {
        None
    };
    if !fleet_gm && !world_config.ship_slots.is_empty() {
        let mut reservations = crate::ship_slots::ShipSlotReservations::default();
        let (Some(slot_id), Some(claimant), Some(hull)) = (
            pending.slot_id.as_deref(),
            pending.claimant.as_deref(),
            pending.ship_path.as_deref(),
        ) else {
            return Err(NativeHostError::Ship(
                "authored ship slots require a claimant and confirmed hull before launch".into(),
            ));
        };
        if crate::ship_slots::ClaimOutcome::Claimed
            != reservations.claim(&world_config.ship_slots, slot_id, claimant)
            || crate::ship_slots::HullOutcome::Confirmed
                != reservations.confirm_hull(&world_config.ship_slots, slot_id, claimant, hull)
        {
            return Err(NativeHostError::Ship(format!(
                "hull {hull:?} is not allowed for claimed ship slot {slot_id:?}"
            )));
        }
        let frozen = reservations
            .freeze(&world_config.ship_slots)
            .ok_or_else(|| {
                NativeHostError::Ship(
                    "every claimed ship slot must confirm a hull before launch".into(),
                )
            })?;
        pending_slots = Some(PendingSlotFreeze(frozen));
    }
    let gm = if native_role
        == crate::native_host::session_role::NativeSessionRole::StandaloneGameMaster
    {
        Some(
            crate::gm_solo::prepare_standalone_game_master(world, true).ok_or_else(|| {
                NativeHostError::Ship("standalone GM identity could not be admitted".into())
            })?,
        )
    } else {
        None
    };
    Ok(PreparedNativeWorldLoad {
        ingest,
        hull,
        rng,
        frozen_slots,
        pending_slots,
        gm,
    })
}

/// Commit is infallible: materialization consumes already validated inputs.
impl PreparedNativeWorldLoad {
    fn install(self, world: &mut World) {
        self.ingest.install(world);
        let rng = if let Some(hull) = self.hull {
            hull.install(world)
        } else {
            world.insert_resource(crate::gm_projection::GameMasterPeer);
            self.rng.expect("prepared GM load carries its RNG")
        };
        if let Some(slots) = self.frozen_slots {
            world.insert_resource(slots);
        }
        if let Some(slots) = self.pending_slots {
            world.insert_resource(slots);
        }
        crate::sim_rng::install(world, rng);
        world.resource_mut::<ShipStations>().stations.clear();
        let restore = park_mint(world);
        world.run_schedule(RuntimeWorldLoad);
        restore_mint(world, restore);
        if let Some(gm) = self.gm {
            gm.install(world);
            if let Some(bridge) =
                world.get_resource::<crate::native_host::host_lobby::HostLobbyBridgeResource>()
            {
                bridge
                    .0
                    .push_join(crate::native_host::host_lobby::join::JoinInvite::Off.to_json());
            }
        }
        if let Some(mut role) =
            world.get_resource_mut::<crate::native_host::session_role::NativeSessionRoleState>()
        {
            role.commit();
        }
    }
}

fn load_selected_world(
    world: &mut World,
    pending: &PendingWorldLoad,
) -> Result<(), NativeHostError> {
    crate::content_ledger::reset();
    let prepared =
        prepare_selected_world(world, pending).inspect_err(|_| crate::content_ledger::reset())?;
    prepared.install(world);
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
    crate::world_id::install(world, WorldIdMint::default());
    Some(live)
}

/// Put back what [`park_mint`] took.
fn restore_mint(world: &mut World, saved: Option<WorldIdMintState>) {
    if let Some(state) = saved {
        crate::world_id::install(world, WorldIdMint::from_state(state));
    }
}

#[cfg(test)]
#[path = "world_load_tests.rs"]
mod tests;
