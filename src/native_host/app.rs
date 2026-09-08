//! Assembling the native windowed authoritative host (issue #1121).
//!
//! This is the fourth app builder in the repo, and it is a *thin* one on
//! purpose: everything shared with the other three comes from
//! [`crate::boot::build`] under [`BootProfile::NativeHost`], and the simulation
//! itself is [`add_simulation_plugins_with`], unchanged and in its unchanged
//! registration order (that order is a digest input — see
//! `server_app::registration`'s module note).
//!
//! Read it beside [`crate::headless::app::build_headless_app_with`]: the two
//! run the *same* sequence, and the differences are the whole content of this
//! issue.
//!
//! | headless | native host |
//! |---|---|
//! | `BootProfile::Headless`, render surrogate | `BootProfile::NativeHost`, real Bevy/wgpu through winit |
//! | `add_simulation_plugins_with(render: false)` | `render: true` — the viewscreen radar, reference grid, star/planet renderers, LOD swap and asset preloader |
//! | `TimeUpdateStrategy::ManualDuration` + a hand-rolled `while` loop | `App::run()` under winit, `WinitSettings` pinned to `Continuous` so an unfocused bridge machine keeps simulating |
//! | ship chosen by `--ship`/`--side-a` | ship chosen by `--ship`, defaulting to the world's first `available_ships` entry |
//! | template preload derived from the ship's directory | template preload over the whole content tree, done by the caller and required by [`crate::boot`] |
//! | nobody can ever connect | a [`NativeTransportPlugin`] seam a transport plugs into |
//!
//! # The deferred acceptance criterion
//!
//! Issue #1121's third acceptance criterion — browser clients joining the
//! native host through the same protocol, admission and projection contracts —
//! is **deferred to issue #1112**, the Phoenix WebRTC transport that replaces
//! PeerJS. It has to be: PeerJS is browser JavaScript, so it cannot run in a
//! native process at all, and #1121 is filed as blocked by #1112 for exactly
//! this reason. What is built here instead is the seam it plugs into — see
//! [`crate::native_host::transport`] — which is wired, ordered and gated
//! already, and exercised end to end by a loopback participant in
//! `tests/native_host_sim.rs`.

use bevy::prelude::*;

use crate::asteroids::lifecycle::AsteroidLifecyclePlugin;
use crate::boot::{BootError, BootPlan, BootProfile, NativeRenderSurface, WorldIngest};
use crate::core::messages::{GamePhase, ServerMessage};
use crate::entities::loader::TemplateLoader;
use crate::entities::template_preload::{preload_entity_templates, TemplatePreload};
use crate::lobby::{LobbyOutbox, LobbyPlugin, SelectedShipResource, Target};
use crate::logging::{LogFilterConfig, LoggingPlugin};
use crate::modifiers::coordination::ModifierCoordinationPlugin;
use crate::native_host::transport::NativeTransportPlugin;
use crate::server_app::{add_simulation_plugins_with, SimPluginOptions};
use crate::ship_plugin::PendingShipConfig;
use crate::sim_rng::{SeedSource, SimRng};
use crate::world::WorldPlugin;

/// The OS window caption.
///
/// Not a `strings.csv` id, and deliberately so: there is no Rust-side string
/// table in this repo (ids are resolved client-side by `gui/strings.js`), and
/// this is the window caption of an operator-launched process, on the same
/// footing as `phoenix-host`'s `HELP` text and its operator log. It is also a
/// proper noun rather than prose.
pub const WINDOW_TITLE: &str = "Project Phoenix";

/// Anything that stopped a native host being assembled.
#[derive(Debug)]
pub enum NativeHostError {
    /// The content tree could not be preloaded — see
    /// [`preload_entity_templates`]. Includes the deliberately loud "loaded
    /// nothing" case.
    Content(String),
    /// The model-marker contract is violated somewhere in the content tree
    /// (issue #758), so nothing may spawn.
    Markers(String),
    /// [`crate::boot::build`] refused.
    Boot(BootError),
    /// The world names no playable hull and none was given.
    NoShip(String),
    /// The chosen hull could not be read, parsed, or carries no `[[station]]`
    /// blocks.
    Ship(String),
    /// One or more participant pane names — from `--pane <NAME>` or from a
    /// `--profile`'s station-less `[[display.pane]]` slots — are also **station
    /// ids** on the hull this host is about to fly (issue #1331). See
    /// [`pane_labels_shadowing_stations`] and [`AuthoredPaneLabels`].
    PaneShadowsStation(Vec<String>),
}

impl std::fmt::Display for NativeHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NativeHostError::Content(m) => write!(f, "content preload: {m}"),
            NativeHostError::Markers(m) => write!(f, "{m}"),
            NativeHostError::Boot(e) => write!(f, "{e}"),
            NativeHostError::NoShip(m) => write!(f, "no playable hull: {m}"),
            NativeHostError::Ship(m) => write!(f, "ship: {m}"),
            // The noun is "the pane name" rather than "--pane" because the same
            // collision arrives by two routes and the refusal covers both: a
            // `--pane <NAME>` flag, and a `--profile` `[[display.pane]]` slot
            // with a `label` and no `station`. Everything the sentence asserts,
            // and the remedy it gives, is unchanged.
            NativeHostError::PaneShadowsStation(labels) => write!(
                f,
                "the pane name {} names a station this hull has, and a pane name and a station \
                 id are one namespace on the pane bus (issue #1331): the lobby's screen rows \
                 open a station's console under its own id, so closing that station's console \
                 would close this person's instead. Rename the pane — `--pane {name}-crew`, or \
                 `label = \"{name}-crew\"` in the `--profile` — or drop it and open that \
                 station's console from the lobby's screen row",
                labels
                    .iter()
                    .map(|l| format!("{l:?}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                name = labels.first().map(String::as_str).unwrap_or("name"),
            ),
        }
    }
}

impl std::error::Error for NativeHostError {}

/// What to assemble.
///
/// Filled from `delivery::args::HostArgs` by the `phoenix-host` binary and by
/// hand in tests. Every path is authored the way a world TOML authors one —
/// repo-relative with forward slashes — and resolved against the process
/// working directory, which [`crate::native_host::pin_content_root`] has
/// already pinned to `--content-dir`.
pub struct NativeHostConfig {
    /// The root world TOML, or `None` to boot into an **empty lobby** and take
    /// the world from a runtime scenario selection instead (issue #1326).
    ///
    /// `Some` is `--world`: the world is ingested by [`crate::boot::build`]
    /// before the `App` exists, exactly as it always was. `None` composes the
    /// same `App` with [`WorldIngest::Deferred`], publishes [`Self::catalog`] to
    /// the lobby, and ingests through
    /// [`world_load`](crate::native_host::world_load) once a `SelectScenario` +
    /// `SelectPlayerShip` pair has been arbitrated — the same two messages, and
    /// the same first-valid-wins rule, the browser host arbitrates in
    /// `gui/scenario-arbiter.js`.
    pub world_path: Option<String>,
    /// The scenario catalogue a world-less host offers, from
    /// [`ManifestSource::merged_catalog`](crate::delivery::serve::ManifestSource::merged_catalog).
    /// Empty (and unread) when [`Self::world_path`] is `Some`.
    pub catalog: crate::world::manifest::ScenarioCatalog,
    /// The player's hull. `None` takes the world's first `available_ships`
    /// entry, which is what the browser's ship picker defaults to.
    pub ship_path: Option<String>,
    /// Overrides the world's `[global] seed`; both lose to nothing, and an
    /// absent pair draws from the OS.
    pub seed: Option<u64>,
    /// `plog!` filtering, already parsed.
    pub log: LogFilterConfig,
    /// The raw `--log` spec, forwarded to `LogPlugin`'s own `EnvFilter` so
    /// bevy-internal events roughly agree with our categories.
    pub log_spec: String,
    /// How this host presents — a winit window in production; see
    /// [`NativeRenderSurface`].
    pub surface: NativeRenderSurface,
    /// Start the mission with nobody connected, every station on `Backfill`.
    ///
    /// The native twin of headless's auto-start and of the host page's
    /// force-start button. Without it the host boots to the lobby and waits for
    /// participants to ready up, which is what a crewed session wants — but
    /// **no participant can arrive yet**: the transport is issue #1112's, and
    /// there is no native force-start. [`build_native_host_app`] therefore warns
    /// loudly when this is `false`, rather than refusing a mode that becomes
    /// correct the moment #1112 lands.
    pub solo: bool,
    /// The hulls a restricting `--manifest` publishes for this world (issue
    /// #917's curated allowlist), or empty when the catalogue is unrestricted.
    ///
    /// Only the DEFAULT hull consults it: an explicit `--ship` still wins, the
    /// way an explicit `?ship=` does in the browser. Its job is to stop one
    /// process publishing a curated catalogue over HTTP and simultaneously
    /// flying something that catalogue excludes.
    pub curated_ships: Vec<String>,
    /// Select serial fixed executors (including shared StateTransition) and a
    /// one-thread task pool — [`BootPlan::single_threaded`].
    ///
    /// `false` for a shipped host: a rendered viewscreen is not reproduced
    /// tick-for-tick and the multithreaded pool is worth having. `true` is for a
    /// digest comparison, where an unpinned executor makes the claim a race
    /// rather than a measurement (see `tests/native_headless_digest.rs`).
    pub deterministic: bool,
    /// Log a one-line per-second frame-cost breakdown (`--frame-stats`) — see
    /// [`panes::frame_stats`](crate::native_host::panes::frame_stats). Inert
    /// on the Contract and Offscreen surfaces: there is no frame to account
    /// for, and the test compositions must not read a clock they did not ask
    /// for.
    pub frame_stats: bool,
    /// The diagnostic A/B toggles read from `PHOENIX_FRAME_EXPERIMENTS`, or
    /// none. Applied only to a windowed host, like [`frame_stats`](Self::frame_stats);
    /// the one that reaches the pane document (`raf33`) is applied by the
    /// caller when it publishes the panes, because that happens before this
    /// builder runs.
    pub experiments: crate::native_host::panes::frame_stats::PaneExperiments,
    /// The pane bus, and the Station panes `--pane` opened on it (issue #1122).
    ///
    /// `opened` may be empty (issue #1331): a host with a client bundle carries
    /// a bus whether or not it was given a `--pane`, because the lobby's screen
    /// rows open consoles on it while the host runs.
    ///
    /// Opened by the caller rather than here, because a pane's document is
    /// published at the delivery listener's own address and a `:0` bind does not
    /// know its port until it has bound. What this builder does with them is
    /// install the transport — a pane is a participant, so its traffic enters
    /// through the same `NativeTransportPlugin` seam issue #1121 built and is
    /// subject to the same reserved-token refusal.
    pub panes: Option<crate::native_host::panes::LocalPanes>,
    /// A validated bridge-display profile (issue #1123), or `None` for the
    /// single-window #1121 behaviour.
    ///
    /// Already **validated** by the caller — a bad profile (an unknown role, a
    /// Station with three panes) fails at the prompt, so by the time it reaches
    /// here its roles and density are sound and the density rule need not be
    /// re-checked. When present, [`build_native_host_app`] installs
    /// [`BridgeDisplayPlugin`](crate::native_host::bridge_display::BridgeDisplayPlugin),
    /// which opens one borderless-fullscreen surface per configured monitor:
    /// the viewscreen on the primary window, each Station on its own window.
    pub bridge_profile: Option<crate::native_host::bridge_profile::ValidatedProfile>,
    /// The host's own lobby surface (issue #1325), already opened and published.
    ///
    /// Opened by the caller for the same reason panes are: the document is
    /// published at the delivery listener's own address, and a `:0` bind does
    /// not know its port until it has bound. What this builder does with it is
    /// install [`HostLobbyPlugin`](crate::native_host::host_lobby::HostLobbyPlugin),
    /// which carries the lobby state the browser host's own
    /// `viewscreen_border::push_lobby_state` already emits across the bridge,
    /// and (under `--features ultralight`) tell the pane display host to
    /// composite it onto the viewscreen window.
    ///
    /// `None` leaves the host byte-for-byte as it was: no plugin, no resource,
    /// no surface.
    pub host_lobby: Option<crate::native_host::host_lobby::LocalHostLobby>,
    /// The mod-pack shelf this host offers on its landing (issue #1366), already
    /// scanned, or `None` for a host given no `--mod-pack-dir`.
    ///
    /// `None` is the whole of "this host offers no shelf": with no resource the
    /// surface is never told it can answer the Load-mod-pack row, so that row
    /// stays exactly the inert one issue #1360 shipped. The alternative — a
    /// resource pointing at some default folder — would put an empty shelf on
    /// every native host in the world and make the row lie.
    ///
    /// Scanned by the CALLER, for the same reason `--content-dir` is resolved
    /// there: the folder is named relative to the launch directory, and
    /// [`crate::native_host::pin_content_root`] has moved the process out of it
    /// by the time anything in this builder runs.
    pub mod_pack_shelf: Option<crate::native_host::host_lobby::ModPackShelfResource>,
}

impl NativeHostConfig {
    /// A configuration for `world_path` with everything else defaulted: no
    /// hull override, no seed override, warn-level logging, no window, and a
    /// lobby that waits. The shape tests start from.
    pub fn new(world_path: impl Into<String>) -> Self {
        Self {
            world_path: Some(world_path.into()),
            catalog: crate::world::manifest::ScenarioCatalog::default(),
            ship_path: None,
            seed: None,
            log: LogFilterConfig::default(),
            log_spec: String::new(),
            surface: NativeRenderSurface::Contract,
            solo: false,
            curated_ships: Vec::new(),
            deterministic: false,
            frame_stats: false,
            experiments: Default::default(),
            panes: None,
            bridge_profile: None,
            host_lobby: None,
            mod_pack_shelf: None,
        }
    }

    /// A world-less configuration (issue #1326): the same host, booted into an
    /// empty [`GamePhase::Lobby`] offering `catalog`, with no world ingested
    /// until a scenario and hull are selected at runtime.
    ///
    /// What `phoenix-host --lobby` builds, and the shape the runtime-load tests
    /// start from.
    pub fn lobby(catalog: crate::world::manifest::ScenarioCatalog) -> Self {
        Self {
            world_path: None,
            catalog,
            ..Self::new(String::new())
        }
    }
}

/// Populate the native entity-template cache for a content tree.
///
/// The strict walk — [`preload_entity_templates`] over `<content_dir>/assets/
/// entities` — not `delivery::serve::preload_templates`. Both write the same
/// process-global cache and a host must call **one** of them, once; the strict
/// one is the one a simulation needs (sorted, marker-validating, and it refuses
/// to succeed having cached nothing), and its side effect of enriching the
/// published catalogue is exactly what the delivery walk was there for.
///
/// `content_dir` is expected to be the process working directory — in practice
/// `"."`, because [`crate::native_host::pin_content_root`] has already made it
/// so. That is not incidental: the cache is keyed by the path a *world*
/// authors (`assets/entities/…`), so a root spelled any other way would key
/// entries nothing ever looks up.
///
/// **Process-global.** Callers belong in a binary or an integration test.
pub fn preload_content_templates(content_dir: &str) -> Result<TemplatePreload, NativeHostError> {
    let root = content_dir.trim_end_matches(['/', '\\']);
    let dir = if root.is_empty() || root == "." {
        "assets/entities".to_string()
    } else {
        format!("{root}/assets/entities")
    };
    preload_entity_templates(&dir).map_err(NativeHostError::Content)
}

/// The hulls a scenario manifest publishes for `world_path` — issue #917's
/// curated allowlist, read for the process's OWN default hull.
///
/// `phoenix-host --manifest assets/scenarios.demo.toml` restricts what this
/// process publishes over HTTP; without this it did not restrict what the same
/// process flies, so one host could serve a curated catalogue and simultaneously
/// run a hull that catalogue excludes. The browser has no such gap: its picker
/// is built from the curated list.
///
/// Empty means **unrestricted**, and it is the answer for all three of "the
/// manifest curates nothing", "the manifest does not publish this world at all"
/// and "the manifest does not parse". That is deliberate: this is a narrowing
/// of the default hull, not a second gate on the world — refusing a `--world`
/// the catalogue happens not to list would break the ordinary dev invocation,
/// where the world is named directly and the manifest is beside the point.
pub fn curated_hulls_for_world(manifest_toml: &str, world_path: &str) -> Vec<String> {
    let wanted = crate::entities::include_resolve::canonical_template_path(world_path);
    let Ok(manifest) = crate::world::manifest::parse_manifest(manifest_toml) else {
        return Vec::new();
    };
    manifest
        .scenarios
        .iter()
        .find(|s| crate::entities::include_resolve::canonical_template_path(&s.world) == wanted)
        .map(|s| s.ships.clone())
        .unwrap_or_default()
}

/// The [`BootPlan`] a native host composes, for `world_path` (or for no world
/// at all).
///
/// Shared by [`build_native_host_app`] and by
/// [`world_load`](crate::native_host::world_load)'s runtime ingest, so the two
/// cannot disagree about the reader, the script resolver, the abort policy or
/// the log filter — the runtime load is the boot load, differing only in *when*
/// it runs. The render/surface/task-pool fields no longer choose anything once
/// an `App` exists, but the runtime call still fills them from the settings the
/// host booted with
/// ([`LobbyBootSettings`](crate::native_host::world_load::LobbyBootSettings)) so
/// there is one plan rather than a plan and an approximation of it.
pub(crate) fn boot_plan(
    world_path: Option<&str>,
    log_spec: &str,
    deterministic: bool,
    surface: NativeRenderSurface,
) -> BootPlan {
    BootPlan {
        profile: BootProfile::NativeHost,
        world_ingest: match world_path {
            Some(_) => WorldIngest::FromReader,
            None => WorldIngest::Deferred,
        },
        // Keep bevy-internal events quiet by default; a `--log` spec is folded
        // in after the `warn` floor, exactly as headless does it.
        log_filter: if log_spec.is_empty() {
            "warn".to_string()
        } else {
            format!("warn,{log_spec}")
        },
        world_path: world_path.unwrap_or_default().to_string(),
        // Overlay-first (issue #1366). A native host can now have mod packs
        // installed, and a pack's worlds live only in the overlay — so a bare
        // `FsReader` would offer a pack's scenario in the lobby and then fail to
        // read the world behind it. With no pack installed this IS `FsReader`.
        reader: Box::new(crate::world::load::OverlayFsReader),
        script_resolver: Box::new(crate::entities::config_cache::production_script_resolver()),
        // A shipped rendered host is not reproduced tick-for-tick, so it keeps
        // the multithreaded pool — the same answer both browser profiles give.
        // A digest comparison asks for the pinned executor instead; boot
        // honours it on every native composition path, wgpu included.
        single_threaded: deterministic,
        raw_transform: None,
        native_surface: surface,
    }
}

/// Which hull to fly, and under what seed — the inputs
/// [`install_world_selection`] resolves against a loaded world.
pub(crate) struct HullChoice<'a> {
    /// The world's authored path, for error messages only.
    pub world_label: &'a str,
    /// An explicit hull (`--ship`, or the lobby's `SelectPlayerShip`). `None`
    /// takes the world's first curated `available_ships` entry.
    pub ship_path: Option<&'a str>,
    /// The manifest's curated hull allowlist for this world (issue #917); empty
    /// means unrestricted.
    pub curated_ships: &'a [String],
    /// `--seed`, which outranks the world's own `[global] seed`.
    pub seed: Option<u64>,
}

/// Every **participant pane name** this host was launched with (issue #1331).
///
/// Inserted by [`build_native_host_app`], so that [`install_world_selection`] can
/// check them against the hull's station ids — on the `--world` path and on the
/// runtime `--lobby` world load alike, which are the only two places a roster is
/// ever chosen. Empty for a host given a `--client-dir` but no `--pane` and no
/// `--profile` participant slot, which is the ordinary console host.
///
/// # Two sources, one list
///
/// A pane name reaches the bus by two routes and the collision is identical
/// down both, so they are gathered into one list rather than guarded in two
/// places:
///
///  * `--pane <NAME>` (issue #1122), whose pane is opened at boot;
///  * a `--profile`'s **participant** pane slots — a `[[display.pane]]` with a
///    `label` and no `station` key
///    ([`ValidatedProfile::participant_pane_labels`](crate::native_host::bridge_profile::ValidatedProfile::participant_pane_labels)).
///    Those are the slots `assigned_surfaces` keeps in the runtime watcher's
///    `pane_labels`, and the ones the adapter lays out as a rectangle on a
///    Station window that a `--pane` of the same name is then seated into.
///
/// Only the FIRST of the two was checked when the guard landed, which left the
/// whole collision reachable through a file: an authored `label = "helm"` on a
/// hull with a `helm` station passes the `--pane` check (there is no `--pane`),
/// keeps "helm" in the watcher's `pane_labels`, and then has the *station's*
/// console — opened under the same name by the lobby's screen row — closed by an
/// unplug of a monitor the law never unseated anything on, minting a fresh token
/// in its place.
#[derive(Resource, Clone, Debug, Default)]
pub struct AuthoredPaneLabels(pub Vec<String>);

/// The participant pane names that are also station ids on `stations`
/// (issue #1331) — see [`AuthoredPaneLabels`] for where `labels` comes from.
///
/// **A pane name and a station id are one namespace.** `PaneBus` resolves a pane
/// by participant name (`open_pane_for_name`), and since #1331 the lobby's
/// screen rows open a station's console under the *station id* as its name —
/// deliberately, because that is what lets the layout law and the pane bus talk
/// about the same console by the same key. A hand-authored `--pane helm` — or a
/// `--profile` pane slot whose `label` is `"helm"` — on a hull that has a `helm`
/// station therefore collides: unassigning `helm` from a screen row resolves the
/// name to the human's pane and closes **their** console, seating `helm` finds a
/// pane already open and never builds one, and an unplug of the monitor the
/// profile named closes whichever of the two the bus answers with.
///
/// Comparison is exact and case-sensitive, matching `open_pane_for_name`'s own
/// `==` — a guard that judged by a different rule than the lookup it protects
/// would pass a name the lookup then confuses.
pub(crate) fn pane_labels_shadowing_stations(
    labels: &[String],
    stations: &[crate::core::messages::StationId],
) -> Vec<String> {
    labels
        .iter()
        .filter(|label| stations.iter().any(|s| &s.0 == *label))
        .cloned()
        .collect()
}

/// Resolve the hull, gate it against the native template cache, and insert the
/// two ship resources `LobbyPlugin` reads — returning the [`SimRng`] this run
/// should adopt.
///
/// Extracted from [`build_native_host_app`] so the runtime world load
/// ([`world_load`](crate::native_host::world_load)) performs the *same* steps in
/// the same order rather than a second version of them: seed precedence, hull
/// choice, the cache gate whose failure is otherwise silent, the #935 hull
/// re-record + re-freeze, `PendingShipConfig`, and the canonical
/// `SelectedShipResource`.
pub(crate) fn install_world_selection(
    world: &mut World,
    world_config: &crate::world::config::WorldConfig,
    choice: &HullChoice<'_>,
) -> Result<SimRng, NativeHostError> {
    // Seed precedence: `--seed`, then the world's `[global] seed`, then the OS.
    let sim_rng = match (choice.seed, world_config.global.seed) {
        (Some(seed), _) => SimRng::new(seed, SeedSource::Cli),
        (None, Some(seed)) => SimRng::new(seed, SeedSource::World),
        (None, None) => SimRng::random(),
    };

    // The hull. `--ship` wins; otherwise the world's first `available_ships`
    // entry the published catalogue still offers, which is what the browser's
    // ship picker pre-selects — `entity_template_paths` filters that same list
    // by the same allowlist, in the world's own authored order.
    //
    // Consulting `curated_ships` here is what stops one process publishing a
    // curated catalogue over HTTP (`--manifest assets/scenarios.demo.toml`,
    // issue #917's native half) and simultaneously flying a hull that catalogue
    // excludes.
    let ship_path = match choice.ship_path {
        Some(path) => path.to_string(),
        None => world_config
            .available_ships
            .iter()
            .find(|s| {
                choice.curated_ships.is_empty()
                    || choice.curated_ships.iter().any(|c| c == &s.template_path)
            })
            .map(|s| s.template_path.clone())
            .ok_or_else(|| {
                NativeHostError::NoShip(if choice.curated_ships.is_empty() {
                    format!(
                        "{} authors no [[available_ships]] and no --ship was given",
                        choice.world_label
                    )
                } else {
                    format!(
                        "{} authors no [[available_ships]] the manifest's curated hull list \
                         admits ({}), and no --ship was given",
                        choice.world_label,
                        choice.curated_ships.join(", ")
                    )
                })
            })?,
    };

    // The hull's own half of boot's template-cache gate (issue #1121).
    //
    // `check_native_templates` sees the world's DECLARED set, and an explicit
    // `--ship` need not be in it: issue #935 made the player's hull authored
    // content that may sit outside `available_ships` entirely. So a `--ship`
    // pointing at a template the preload never cached would walk straight past
    // that gate, and `lobby::server::update_session_with_config` — which looks
    // this exact path up in the cache with NO filesystem fallback — would keep
    // a DEFAULT `ShipClientConfig`: default helm radar range, default
    // impulse-charge duration, default hostile-arc colour, a plausible mission,
    // a clean log. The refusal below is the same refusal boot makes, sited
    // where the resolved hull is finally known.
    let ship_key = crate::entities::include_resolve::canonical_template_path(&ship_path);
    if crate::entities::config_cache::get_cached_entity_config(&ship_key).is_none() {
        return Err(NativeHostError::Ship(format!(
            "{ship_key} is not in the native entity-template cache, so the host would \
             read a Default hull configuration — helm radar range, impulse-charge \
             duration and hostile-arc colour — instead of this hull's authored one. \
             Check --content-dir and that the hull lives under <content-dir>/assets/entities"
        )));
    }

    // Ship config, BEFORE `LobbyPlugin`: the native twin of
    // `wasm_validate_stations`. Without it `update_session_with_config` falls
    // back to `load_ship_config_from_disk`, which returns the *battleship*
    // roster regardless of the hull chosen — so every station, and therefore
    // every backfilled AI system, would belong to the wrong ship.
    let ship_entity_config = crate::entities::include_resolve::load_entity_config(&ship_path)
        .map_err(|e| NativeHostError::Ship(format!("{ship_path:?} failed to parse: {e}")))?;
    // Issue #935: the player's own hull is authored content too and need not be
    // among the world's declared entities, so re-record and re-freeze. The
    // ledger fold is path-sorted and order-independent, so the frozen digest is
    // byte-identical whichever route the hull rode in on.
    let _ = crate::entities::loader::FsTemplateLoader.load_template(&ship_path);
    crate::content_ledger::freeze();
    let ship_config = ship_entity_config
        .ship_config
        .ok_or_else(|| NativeHostError::Ship(format!("{ship_path:?} has no [[station]] blocks")))?;

    // The one place a participant pane name and a station id can be compared:
    // the names were fixed at the prompt (a `--pane` flag, or a `--profile` pane
    // slot that names no station), and this is where the roster is finally
    // known — at boot for a `--world` host, and at the pick for a `--lobby` one,
    // which is why the guard lives here rather than in `phoenix_host`'s main.
    // Refused rather than warned: the two names resolve to one pane on the bus,
    // so whichever of the two the operator meant, one of them is going to close
    // the other's console (see `pane_labels_shadowing_stations`). Refusing
    // BEFORE `PendingShipConfig` lands leaves the world-less lobby's unwind
    // nothing extra to undo.
    let stations: Vec<crate::core::messages::StationId> =
        ship_config.stations.iter().map(|s| s.id.clone()).collect();
    let shadowed = pane_labels_shadowing_stations(
        &world
            .get_resource::<AuthoredPaneLabels>()
            .map(|l| l.0.clone())
            .unwrap_or_default(),
        &stations,
    );
    if !shadowed.is_empty() {
        return Err(NativeHostError::PaneShadowsStation(shadowed));
    }
    world.insert_resource(PendingShipConfig(ship_config));
    // Store the CANONICAL key, not the raw `--ship` string: every downstream
    // reader (`lobby::server::update_session_with_config`, `server::radar`,
    // `server::reference_grid`, `server_app::world_setup`) looks this path up
    // in the native template cache with NO filesystem fallback, and that cache
    // is keyed canonically. The gate two lines above already canonicalises
    // before checking, so a raw string here would let a `--ship` spelled with
    // `./` or Windows backslashes pass the gate and then miss every one of
    // those lookups, silently keeping a Default `ShipClientConfig`.
    world.insert_resource(SelectedShipResource(ship_key));

    Ok(sim_rng)
}

/// Assemble the native host's `App`. Does not run it — see [`run`].
///
/// `preload` is the receipt from [`preload_content_templates`], taken by
/// reference rather than performed here so that the caller (`phoenix-host`)
/// can populate the cache *before* it binds its HTTP listener and builds the
/// published catalogue from the same templates. It cannot be forged: boot
/// re-checks the cache against the world's declared set regardless, and refuses
/// with [`BootError::NativeTemplatesMissing`] on a miss.
pub fn build_native_host_app(
    cfg: &NativeHostConfig,
    preload: &TemplatePreload,
) -> Result<App, NativeHostError> {
    // The model-marker contract gate (issue #758), before an `App` exists. An
    // unresolved marker attaches a beam, exhaust plume or camera to the ship's
    // centre and produces a plausible-looking mission with the wrong numbers —
    // exactly the class of silent failure a windowed host is worst at showing.
    preload.marker_gate().map_err(NativeHostError::Markers)?;

    // The boot seam (issue #1217, fourth profile #1121). It composes the plugin
    // core and the real render stack, and owns the whole world-ingestion order:
    // Rhai hashing seed → content-ledger reset → read/validate/compile →
    // abort on a broken world (NativeHost is authoritative, like headless) →
    // the native template-cache check → ledger apply + eager record → freeze →
    // insert the `WorldConfig` and its `PreCompiledScripts`.
    //
    // With no `--world` (issue #1326) the SAME plan is composed with
    // [`WorldIngest::Deferred`] instead: identical profile, identical render
    // surface, identical task-pool answer — boot simply stops after the Rhai
    // hashing-seed pin, and `world_load::load_selected_world` runs the rest of
    // that order later, on the running `World`, through this same function.
    let mut app = crate::boot::build(boot_plan(
        cfg.world_path.as_deref(),
        &cfg.log_spec,
        cfg.deterministic,
        cfg.surface,
    ))
    .map_err(NativeHostError::Boot)?;

    // Everything the preload gathered before a `tracing` subscriber existed.
    // Emitted HERE, after boot's `LogPlugin::build` installed one — before it,
    // every line goes nowhere.
    preload.report();

    // Crate-side `plog!` filtering, separate from boot's bevy `LogPlugin`.
    app.insert_resource(cfg.log.clone())
        .add_plugins(LoggingPlugin);

    // The participant pane names, BEFORE the world selection below — that is
    // where they are checked against the hull's station ids (issue #1331), and
    // the runtime `--lobby` load reaches the same check through the same resource
    // on a running `World`.
    //
    // BOTH sources, because both put a name on the pane bus and the collision is
    // identical down either: the `--pane <NAME>` flags, and the `--profile`'s own
    // station-less pane slots. See [`AuthoredPaneLabels`].
    let mut authored_pane_labels: Vec<String> = cfg
        .panes
        .as_ref()
        .map(|p| {
            p.opened
                .iter()
                .map(|pane| pane.identity.name().to_string())
                .collect()
        })
        .unwrap_or_default();
    if let Some(profile) = &cfg.bridge_profile {
        authored_pane_labels.extend(profile.participant_pane_labels());
    }
    app.insert_resource(AuthoredPaneLabels(authored_pane_labels));

    // The world half — everything below is skipped for a world-less host, which
    // does it at runtime instead. `install_world_selection` is the shared body:
    // seed precedence, hull resolution, the hull's own template-cache gate, and
    // the two ship resources `LobbyPlugin` reads.
    let sim_rng = match cfg.world_path.as_deref() {
        Some(world_path) => {
            let world_config = app
                .world()
                .resource::<crate::world::config::WorldConfig>()
                .clone();
            Some(install_world_selection(
                app.world_mut(),
                &world_config,
                &HullChoice {
                    world_label: world_path,
                    ship_path: cfg.ship_path.as_deref(),
                    curated_ships: &cfg.curated_ships,
                    seed: cfg.seed,
                },
            )?)
        }
        None => None,
    };

    // `ConfigCachePlugin` is wasm-only; its two jobs are the template cache
    // (done by the preload) and the faction registry, which
    // `add_simulation_plugins` already inserts from the `include_str!`ed native
    // registry.
    app.add_plugins(AsteroidLifecyclePlugin)
        .add_plugins(ModifierCoordinationPlugin)
        .add_plugins(LobbyPlugin)
        .add_plugins(crate::lobby::lobby_outbox_broadcaster());

    // `render: true` — the same option the browser host takes, and the reason
    // this host draws anything: the star and planet renderers, the procedural
    // mesh cache and LOD swap, the viewscreen radar, the reference grid and the
    // asset preloader that gates `Loading → InProgress`. Every type it adds is
    // declared `Presentation`/`DeferredFold` in the authoritative census, which
    // is the claim `tests/native_host_sim.rs` pins by digest against a headless
    // run of the same world and seed.
    add_simulation_plugins_with(
        &mut app,
        SimPluginOptions {
            render: true,
            ..Default::default()
        },
    );
    // After the plugins, so it overrides their OS-seeded `init_resource`. A
    // world-less host has no `[global] seed` to read yet and keeps the
    // OS-seeded default until `world_load` applies the same precedence to the
    // world it ingests.
    if let Some(sim_rng) = sim_rng {
        crate::sim_rng::install(app.world_mut(), sim_rng);
    }
    app.add_plugins(WorldPlugin);

    // The runtime world load (issue #1326). Selection runs only while there is
    // no WorldConfig. Both native entry paths also re-Welcome existing crew
    // when returning to the retained world's Lobby, without another ingest.
    app.add_plugins(crate::native_host::world_load::NativeWorldLoadPlugin);
    if cfg.world_path.is_none() {
        app.insert_resource(crate::native_host::world_load::LobbyScenarioCatalog(
            cfg.catalog.clone(),
        ));
        app.insert_resource(crate::native_host::world_load::LobbyBootSettings {
            ship_path: cfg.ship_path.clone(),
            seed: cfg.seed,
            log_spec: cfg.log_spec.clone(),
            deterministic: cfg.deterministic,
            surface: cfg.surface,
        });
    }

    // The host-authoritative force start (issue #1328).
    //
    // `apply_force_start` is `server::bridge`'s own, de-wasm-gated rather than
    // reimplemented: the rule it applies — Lobby only, wait for the asset
    // preload, announce `GameStarted`, refuse with no world — is host policy,
    // and a second native copy of it is a second policy that drifts. What stays
    // browser-side is only `drain_force_start_input`, which reads a thread-local
    // JavaScript set.
    //
    // The two ordering edges are `wasm_init`'s and `solo_auto_start`'s:
    // `.before(SimSet::Input)` puts the `NextState<GamePhase>` write on the
    // tick-scoped transition site (issue #907), and
    // `.after(NativeWorldLoadSet)` means a press on the tick a runtime world
    // lands sees that world rather than being refused by the guard.
    //
    // Installed unconditionally. Nothing sets the latch on a host with no lobby
    // surface, so a `--pane`-only, `--solo` or headless-shaped host is a
    // resource and an inert system heavier and behaves identically.
    app.init_resource::<crate::server::bridge::PendingForceStart>()
        .add_systems(
            FixedUpdate,
            crate::server::bridge::apply_force_start
                .before(crate::sim_sets::SimSet::Input)
                .after(crate::native_host::world_load::NativeWorldLoadSet),
        );

    // The transport seam. Installed unconditionally and inert until something
    // inserts a `NativeTransportLink` — see the module docs for why it is here
    // before the transport is.
    app.add_plugins(NativeTransportPlugin);

    // Local Station panes (issue #1122). They are participants, so the transport
    // they get is the ORDINARY seam above — not a side door. Everything a pane
    // says enters through `Messages<InboundMessage>` and past the reserved-token
    // refusal; everything it hears is a `Target` the broadcaster resolved
    // through `SessionManager::holder_for_station`.
    //
    // A pane host with panes is therefore also a host that HAS a transport,
    // which is why the `--solo` warning below asks whether one is installed
    // rather than assuming nobody can ever connect.
    //
    // `panes.opened` may be EMPTY (issue #1331): a host with a client bundle
    // carries a bus whether or not it was given a `--pane`, because the lobby's
    // per-station screen rows open consoles on it while the host runs. The bus,
    // the transport and the (empty) display config are installed all the same —
    // a console opened three minutes into a lobby is the same kind of
    // participant as one a flag opened at boot, and it must not need a second
    // seam to reach the simulation.
    if let Some(panes) = &cfg.panes {
        app.insert_resource(crate::native_host::transport::NativeTransportLink::new(
            panes.bus.transport(),
        ));
        app.insert_resource(crate::native_host::panes::PaneBusResource(
            panes.bus.clone(),
        ));
        #[cfg(feature = "ultralight")]
        {
            use crate::native_host::panes::ultralight::{PaneDisplayConfig, PaneDisplayEntry};
            app.insert_resource(PaneDisplayConfig {
                panes: panes
                    .display_entries()
                    .into_iter()
                    .map(|(id, url, label)| PaneDisplayEntry { id, url, label })
                    .collect(),
            });
        }
    }

    // The host's own lobby surface (issue #1325). Not a participant and not on
    // the pane bus: it renders the lobby state this process is ALREADY
    // broadcasting to every phone in the room, over the same
    // `gui/host-lobby-view.js` + `gui/host-lobby-render.js` pair `server.html`
    // renders with. The plugin is what carries that state across the bridge; the
    // display config below is what composites the surface, and only a build with
    // the SDK has anything to composite onto.
    if let Some(lobby) = &cfg.host_lobby {
        app.insert_resource(crate::native_host::host_lobby::HostLobbyBridgeResource(
            lobby.bridge.clone(),
        ));
        app.add_plugins(crate::native_host::host_lobby::HostLobbyPlugin);
        // The mod-pack shelf (issue #1366), inserted only when the operator
        // named a folder. Beside the bridge rather than outside this block on
        // purpose: the shelf is a stage of the LANDING, and the landing only
        // exists on a host with a lobby surface — a shelf without one would be a
        // scanned folder nothing could ever show.
        if let Some(shelf) = &cfg.mod_pack_shelf {
            app.insert_resource(shelf.clone());
        }
        #[cfg(feature = "ultralight")]
        {
            app.insert_resource(
                crate::native_host::panes::ultralight::HostLobbyDisplayConfig { url: lobby.url() },
            );
            // The viewscreen HUD overlay (issue #422's `#hud-overlay`, ported to
            // the native path): a transparent Ultralight surface on the viewscreen
            // window that frames the 3-D scene the same way `server.html` frames
            // its canvas. Composited by the same `PaneHost`, shown in-game only.
            app.insert_resource(
                crate::native_host::panes::ultralight::ViewscreenHudDisplayConfig {
                    url: lobby.viewscreen_hud_url(),
                },
            );
        }
    }

    // One display host for both kinds of surface — Ultralight allows one
    // `Renderer` per process, and `PaneDisplayPlugin` owns it. Added once,
    // after both configs, because Bevy panics on a duplicate plugin and either
    // block above can be the one that wanted it.
    #[cfg(feature = "ultralight")]
    if cfg.panes.is_some() || cfg.host_lobby.is_some() {
        app.add_plugins(crate::native_host::panes::ultralight::PaneDisplayPlugin);
    }

    // The pane-frame upload path (issue #1404). UNCONDITIONAL, and deliberately
    // outside the `ultralight` block above: it is the render-world half of the
    // pane pipeline, it compiles and is tested with the feature off, and on a
    // renderer-less Contract host it is what drains a queue nothing can upload.
    app.add_plugins(crate::native_host::panes::upload::PaneUploadPlugin);

    // Bridge-display profile (issue #1123). Installed unconditionally, and the
    // config is inserted only when a validated profile was given — an
    // AUTHORED one, which is what makes the adapter place the windows it names:
    // the viewscreen on the primary window in borderless fullscreen, each
    // Station on its own window.
    //
    // Without `--profile` the adapter now synthesises its own config from the
    // monitors it finds (issue #1330), so the monitor watcher and the lobby's
    // monitor row run on every windowed host rather than only on a configured
    // one. That config is a description rather than an instruction and places
    // nothing, so a host launched with no display arguments and no lobby press
    // still opens exactly the single #1121 window it always did.
    app.add_plugins(crate::native_host::bridge_display::BridgeDisplayPlugin);
    if let Some(profile) = &cfg.bridge_profile {
        app.insert_resource(crate::native_host::bridge_display::BridgeDisplayConfig {
            profile: profile.clone(),
            authored: true,
        });
    }

    // The per-ship-class saved layouts (issue #1334). The plugin is installed
    // unconditionally and is inert without the resource below, exactly as the
    // display plugin above is inert without monitors.
    //
    // The resource is inserted only for a host whose bridge is the LOBBY's to
    // remember, and the `else` arm is the whole of issue #1334's precedence
    // rule: a run given an explicit `--profile` uses that profile verbatim, has
    // nothing pre-applied over it, and files nothing back. That is stated here
    // AND as a run condition on the systems, because it is also a data-loss
    // guard — a saved `--profile`-seeded layout would drop the operator's
    // `--pane` participant slots out of the file (`to_validated_profile` emits
    // seats and only seats) and the next boot would move the viewscreen onto a
    // live console. See `layout_store_systems`' module note.
    app.add_plugins(crate::native_host::layout_store_systems::BridgeLayoutStorePlugin);
    if cfg.bridge_profile.is_none() {
        match crate::native_host::layout_store::LayoutStore::user() {
            Some(store) => {
                // Opening the operator's real store is the one moment to clear
                // debris out of it. A `*.tmp` sibling can only be there because
                // a previous host was HARD-killed between the create and the
                // rename — an ordinary failed save removes its own — and this is
                // a directory people browse, copy between machines and delete
                // single entries from. Debug rather than info: a tidy-up nobody
                // asked for is not news, and it says nothing on the run after.
                for stale in store.sweep_temporaries() {
                    crate::pdebug!(
                        cfg.log,
                        crate::logging::LogCat::Lobby,
                        "bridge layouts: swept the stale temporary {} left behind by a host that \
                         did not finish a write",
                        stale.display()
                    );
                }
                app.insert_resource(
                    crate::native_host::layout_store_systems::BridgeLayoutStore {
                        store,
                        remembered: None,
                    },
                );
            }
            None => crate::pwarn!(
                cfg.log,
                crate::logging::LogCat::Lobby,
                "no home directory could be resolved, so this host cannot remember the bridge \
                 arrangement per ship class; every session starts from the displays as found. \
                 Pass --profile to run a fixed arrangement instead."
            ),
        }
    }

    // An unfocused bridge machine must keep simulating: the browser host
    // inserts exactly this, and a native window's default is to throttle when
    // it loses focus.
    if cfg.surface == NativeRenderSurface::Window {
        app.insert_resource(bevy::winit::WinitSettings {
            focused_mode: bevy::winit::UpdateMode::Continuous,
            unfocused_mode: bevy::winit::UpdateMode::Continuous,
        });
    }

    // The frame-cost measurement and the A/B toggles it is read against (see
    // `panes::frame_stats`). Both need a real frame to account for, so the
    // Contract and Offscreen surfaces — the test compositions — get neither,
    // whatever the config says.
    if cfg.surface == NativeRenderSurface::Window {
        if !cfg.experiments.is_empty() {
            crate::pinfo!(
                cfg.log,
                crate::logging::LogCat::Lobby,
                "frame experiments on ({}): {}",
                crate::native_host::panes::frame_stats::EXPERIMENTS_ENV,
                cfg.experiments
            );
            app.insert_resource(cfg.experiments);
        }
        if cfg.frame_stats {
            app.add_plugins(crate::native_host::panes::frame_stats::PaneFrameStatsPlugin);
        }
    }

    if cfg.solo {
        app.add_systems(
            FixedUpdate,
            solo_auto_start
                .before(crate::sim_sets::SimSet::Input)
                // A world-less host loads its world in this set (issue #1326);
                // the edge is what makes `--solo` start the mission on the SAME
                // tick the world lands, exactly as a `--world` host starts it on
                // the first tick after `Startup` ingested one. With a world
                // already ingested the set is empty and the edge is inert.
                .after(crate::native_host::world_load::NativeWorldLoadSet),
        );
    } else if cfg.panes.as_ref().is_none_or(|p| p.opened.is_empty()) && cfg.host_lobby.is_none() {
        // The mode is correct and the flag is not refused — but with nothing
        // able to connect it opens a window onto a lobby nothing can start.
        // Every route to `InProgress` needs a session (collective `SetReady`
        // auto-start) or a force-start, and browser participants are still
        // deferred to issue #1112 — so say so at the top of the log rather than
        // leaving an operator watching a lobby that will never move.
        //
        // Two arrangements do NOT take this arm. A host with local Station panes
        // (issue #1122): its panes are participants, they ready up like any
        // other, and the mission starts when they do. And a host with a lobby
        // SURFACE (issue #1328): the AI-launch control on that surface is a
        // native force-start, so the operator standing in front of the
        // viewscreen can launch it on Backfill without `--solo`.
        crate::pwarn!(
            cfg.log,
            crate::logging::LogCat::Lobby,
            "no --solo, no --pane and no lobby surface: this host is waiting in \
             the lobby for participants, but browser clients cannot join a \
             native host yet (issue #1112 — PeerJS is browser JavaScript). \
             Nothing can ready up and there is no control to force-start from, \
             so the mission will not start. Re-run with --solo to fly it on \
             Backfill, --pane <NAME> to crew it locally, or --client-dir <DIR> \
             so the viewscreen carries the lobby's own launch control."
        );
    }

    Ok(app)
}

/// Start the mission with nobody connected.
///
/// Byte-for-byte the policy `headless::app::headless_auto_start` applies, and
/// for the same reason: going straight to `InProgress` with an empty `Sessions`
/// is what makes the player ship AI-driven, because
/// `spawn_game_start_entities` assigns `BACKFILL_RATING` to every station not
/// in the manned set and with no sessions that set is empty.
///
/// **Registered in `FixedUpdate`, not `PreUpdate` (issue #907).** A
/// `NextState<GamePhase>` write from `PreUpdate` applies at the FRAME-level
/// `StateTransition`, so `OnEnter(GamePhase::InProgress)` — and the player-ship
/// mint inside it — would fire at a point whose relationship to `SimTick`
/// depends on frame pacing. `FixedUpdate` puts the write on the same
/// tick-scoped transition site every other phase writer uses.
///
/// It deliberately does NOT wait for the asset preloader the way the host
/// page's force-start does. A solo run is the AI flying the mission; the
/// viewscreen catches up as models stream in, and gating the simulation on a
/// GPU upload would make the run's tick sequence depend on disk speed.
///
/// It DOES wait for a world (issue #1326). A `--world` host has the
/// `WorldConfig` before the first fixed step, so that arm is byte-for-byte the
/// behaviour it always had; a host that boots into an empty lobby would
/// otherwise start a mission with no world at all on its very first tick,
/// before anyone could pick one.
fn solo_auto_start(
    state: Res<State<GamePhase>>,
    world_config: Option<Res<crate::world::config::WorldConfig>>,
    mut next_state: ResMut<NextState<GamePhase>>,
    mut outbox: ResMut<LobbyOutbox>,
    mut started: Local<bool>,
) {
    if *started || state.get() != &GamePhase::Lobby || world_config.is_none() {
        return;
    }
    next_state.set(GamePhase::InProgress);
    outbox.0.push((Target::All, ServerMessage::GameStarted));
    *started = true;
}

/// Run the host until the window closes.
///
/// `App::run()` blocks on native, unlike in the browser where it hands control
/// back to `requestAnimationFrame` — and winit owns the main thread on Windows.
/// Everything else a native host does (the delivery HTTP listener, a transport)
/// therefore lives on other threads, and this call is the last thing the
/// binary's `main` does.
pub fn run(mut app: App) {
    let log = app.world().get_resource::<LogFilterConfig>().cloned();
    let frames =
        match crate::perf::native_frames::NativeFrameCapture::install_from_environment(&mut app) {
            Ok(capture) => capture,
            Err(error) => {
                crate::perror!(
                    log,
                    crate::logging::LogCat::Config,
                    "native frame capture refused: {error}"
                );
                return;
            }
        };
    let origin = frames
        .as_ref()
        .map(|capture| (capture.clock_origin(), capture.started_unix_ms()));
    let surfaces =
        match crate::native_host::panes::surface_stats::SurfaceCapture::install_from_environment(
            &mut app, origin,
        ) {
            Ok(capture) => capture,
            Err(error) => {
                crate::perror!(
                    log,
                    crate::logging::LogCat::Config,
                    "surface capture refused: {error}"
                );
                return;
            }
        };
    let exit = app.run();
    // Close the live worker observer before serializing frame samples, which
    // were already frozen by the final First schedule.
    if let Some(capture) = surfaces {
        if let Err(error) = capture.finish(&exit) {
            crate::perror!(
                log,
                crate::logging::LogCat::Config,
                "surface capture failed: {error}"
            );
        }
    }
    if let Some(capture) = frames {
        if let Err(error) = capture.finish(&exit) {
            crate::perror!(
                log,
                crate::logging::LogCat::Config,
                "native frame capture failed: {error}"
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::messages::StationId;

    fn stations(ids: &[&str]) -> Vec<StationId> {
        ids.iter().map(|id| StationId(id.to_string())).collect()
    }

    #[test]
    fn a_pane_named_for_a_station_on_this_hull_is_refused_at_the_prompt() {
        // Issue #1331 opened a collision that could not exist before it: the
        // lobby's screen rows open a station's console under the STATION ID as
        // its pane name, and `PaneBus::open_pane_for_name` resolves by exactly
        // that name. So a hand-authored `--pane helm` on a hull with a `helm`
        // station is two participants under one key — and the screen row's off
        // button, resolving "helm", would close the person's console instead of
        // the station's.
        let shadowed = pane_labels_shadowing_stations(
            &["Ada".to_string(), "helm".to_string()],
            &stations(&["helm", "weapons"]),
        );
        assert_eq!(shadowed, vec!["helm".to_string()], "only the collision");

        // And it is REFUSED, in a sentence that says what to do instead — a
        // warning would leave the operator's own console to be closed by
        // somebody pressing a button about a station.
        let refusal = NativeHostError::PaneShadowsStation(shadowed).to_string();
        assert!(refusal.contains("\"helm\""), "{refusal}");
        assert!(refusal.contains("one namespace"), "{refusal}");
        assert!(refusal.contains("--pane helm-crew"), "{refusal}");
    }

    #[test]
    fn a_pane_that_names_nobody_on_the_roster_is_left_alone() {
        // The ordinary `--pane <NAME>` this must not disturb: a crew member's
        // own name, on a hull whose stations are named for jobs. It is also
        // exact and case-sensitive, because `open_pane_for_name` is — a guard
        // that judged by a looser rule than the lookup it protects would refuse
        // a name the lookup never confuses.
        assert!(pane_labels_shadowing_stations(
            &["Ada".to_string(), "Grace".to_string()],
            &stations(&["helm", "weapons"]),
        )
        .is_empty());
        assert!(
            pane_labels_shadowing_stations(&["Helm".to_string()], &stations(&["helm"]),).is_empty()
        );
        assert!(pane_labels_shadowing_stations(&[], &stations(&["helm"])).is_empty());
    }
}
