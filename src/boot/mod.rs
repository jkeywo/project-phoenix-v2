//! The boot seam: composing an `App` for each of the three inventories (issue
//! #1217, Track 2 step B5).
//!
//! Today three hand-maintained functions spell out "what plugins and
//! registrations does the simulation need" once each:
//!
//! * [`crate::headless::app::build_headless_app`] — native, no window, no GPU.
//! * `server::bridge::wasm_init`'s `is_automation` branch — the browser under
//!   WebDriver, where the wgpu `RenderPlugin` panics with no GPU, so the render
//!   stack is skipped and the assets/messages it would have registered are added
//!   by hand.
//! * `server::bridge::wasm_init`'s real branch — the browser host, `DefaultPlugins`
//!   plus the viewscreen renderer.
//!
//! The three agree on a core (panic/log/task-pool/time/transform/diagnostics/
//! asset/scene/states) and differ in two axes: whether a real renderer is present,
//! and whether they run inside a browser window. This module names those axes as a
//! [`BootProfile`] and composes the core once, so the inventories cannot
//! drift apart unnoticed — a drift the [profile parity test](self#tests)
//! guards permanently.
//!
//! Issue #1121 added the fourth: [`BootProfile::NativeHost`], a real renderer
//! that is *not* a browser — the combination the original two predicates could
//! express but no profile occupied. It varies two more things a browser profile
//! cannot: [`NativeRenderSurface`] (a winit window, an offscreen wgpu device, or
//! no wgpu at all, decided at runtime because native is both the shipped target
//! and the test target) and the native entity-template cache it refuses to boot
//! without ([`BootProfile::requires_native_templates`]).
//!
//! # The adapters
//!
//! Both production boot paths are now thin [`build`] adapters, each adopted behind
//! its own evidence gate: [`crate::headless::app::build_headless_app`] (#1218, a
//! digest A/B) fills a [`BootPlan`] with [`BootProfile::Headless`] +
//! [`WorldIngest::FromReader`]; `server::bridge::wasm_init` (#1219, a Playwright
//! smoke) fills one with [`BootProfile::BrowserHost`] or
//! [`BootProfile::BrowserAutomation`] + [`WorldIngest::HostPreloaded`], its two
//! branches now differing only by that profile. Each adapter attaches the
//! simulation/lobby/world plugins around this seam and keeps only its target-only
//! wiring; `build` owns the shared core, the renderer axis, and the world-ingestion
//! order.
//!
//! # The render surrogate vs the render stack
//!
//! A renderer owes the simulation four things it names even when nothing is drawn
//! — the [`Shader`], [`Image`], [`Mesh`] and [`StandardMaterial`] asset types — plus
//! three host-page bridge messages ([`HudStateChanged`], [`LobbyStateChanged`],
//! [`AiChatterEvent`]) and the lobby-state push system. [`render_surrogate`]
//! registers exactly that contract for the two profiles with no renderer (Headless,
//! BrowserAutomation); [`render_stack`] is the real renderer, for BrowserHost only.
//! See [`render_stack`] for why its wgpu-backed plugins are instantiated only on the
//! browser target.
//!
//! # World ingestion, in one documented order
//!
//! [`ingest_world`] is the sole caller of [`crate::world::load::load`] and the sole
//! owner of the [`content_ledger`](crate::content_ledger) reset→apply→freeze order
//! and the Rhai [`init_hashing_seed`](crate::world::script::init_hashing_seed) that
//! must precede any script engine. Whether a broken world *aborts* the build or
//! merely *blocks activation* downstream is a [`BootProfile`] property.

use bevy::app::{PanicHandlerPlugin, TaskPoolPlugin};
use bevy::asset::{AssetApp, AssetPlugin};
use bevy::diagnostic::{DiagnosticsPlugin, FrameCountPlugin};
use bevy::image::Image;
use bevy::log::LogPlugin;
use bevy::mesh::Mesh;
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::scene::ScenePlugin;
use bevy::shader::{Shader, ShaderLoader};
use bevy::state::app::StatesPlugin;
use bevy::time::TimePlugin;
use bevy::transform::TransformPlugin;

use std::fmt;

use crate::console_bridge::{AiChatterEvent, HudStateChanged, LobbyStateChanged};
use crate::world::load::{load, LoadPolicy, LoadRequest, WorldReader};
use crate::world::script::load::ScriptResolver;

// ── Profile ──────────────────────────────────────────────────────────────────

/// Which of the five inventories to compose.
///
/// The axes the profiles vary along are read off this enum by the private
/// predicates below rather than matched inline, so a fifth profile (or a
/// changed policy) has one place to change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootProfile {
    /// Native, no window, no GPU — the `phoenix-headless` inventory. Aborts the
    /// build on a broken world (an authoritative batch run must not proceed).
    Headless,
    /// The browser host: a real viewscreen renderer inside a browser window.
    BrowserHost,
    /// The browser under WebDriver automation: a browser window but no renderer
    /// (wgpu has no GPU in headless CI), so the render surrogate stands in.
    BrowserAutomation,
    /// The production browser Game Master peer: the ordinary browser shell and
    /// authoritative simulation, deliberately without a render stack or local
    /// player ship. Unlike [`BrowserAutomation`](Self::BrowserAutomation), this
    /// profile is selected explicitly by the GM page and is shipped behavior.
    BrowserGameMaster,
    /// The native windowed authoritative host (issue #1121): the same
    /// simulation/plugin graph the browser host runs, with the viewscreen drawn
    /// by native Bevy/wgpu through winit instead of onto a `<canvas>`.
    ///
    /// It is a render-stack profile that is **not** a browser profile — the one
    /// combination the original three could not express, and the reason this
    /// enum's two predicates were written as separate questions rather than one
    /// match. What it varies that the browser host does not is
    /// [`BootPlan::native_surface`] (window / offscreen / no wgpu at all) and
    /// the native template cache it insists on (see
    /// [`BootProfile::requires_native_templates`]).
    NativeHost,
}

impl BootProfile {
    /// Whether a broken world **aborts** the build (Headless) rather than merely
    /// **blocking activation** downstream (the browser profiles).
    ///
    /// Headless is an authoritative batch run: a world whose composition or
    /// scripts do not validate must stop the build so it activates zero content.
    /// A browser host instead keeps booting — into a lobby that never leaves the
    /// gate — because a player mis-typing a `?scenario=` URL should see an error,
    /// not a dead page.
    ///
    /// [`NativeHost`](BootProfile::NativeHost) takes headless's side: it is
    /// launched from a command line naming its world, so a broken world is a
    /// mistyped flag or bad content, and there is no URL bar to correct it in.
    /// Failing at the prompt beats opening a window onto a lobby that can never
    /// start.
    fn broken_world_aborts(self) -> bool {
        matches!(self, BootProfile::Headless | BootProfile::NativeHost)
    }

    /// Whether this profile drives the real renderer ([`render_stack`]) rather
    /// than the [`render_surrogate`].
    fn has_render_stack(self) -> bool {
        matches!(self, BootProfile::BrowserHost | BootProfile::NativeHost)
    }

    /// Whether this profile runs inside a browser window and so needs the
    /// input/window/winit shell on top of the shared core.
    fn is_browser(self) -> bool {
        matches!(
            self,
            BootProfile::BrowserHost
                | BootProfile::BrowserAutomation
                | BootProfile::BrowserGameMaster
        )
    }

    /// Whether this profile's world must already be in the **native** entity
    /// template cache before it composes (issue #1121).
    ///
    /// Only [`NativeHost`](BootProfile::NativeHost). The browser profiles read a
    /// different cache — the JS preload's `thread_local!`, filled before Bevy
    /// starts — and headless populates the native one itself, one step earlier,
    /// because its model-marker gate must abort before `App::new()`.
    ///
    /// This exists because `boot::build` calls no preload of its own and every
    /// cache-only reader reads it with **no filesystem fallback**:
    /// `asteroids::lifecycle`, `lobby::server`, `server::radar`,
    /// `server::reference_grid`, `server::asset_preload`,
    /// `server_app::world_setup` and `world::server`. (Deliberately named
    /// rather than counted: the population grows as call sites are added, and a
    /// number in prose drifts from the code the moment one does.) An
    /// unpopulated cache does not fail there; it answers `Default` — default
    /// helm radar range, default impulse-charge duration, default hostile-arc
    /// colour — and a native host would run a plausible-looking mission with
    /// the wrong numbers and nothing in the log.
    /// See [`check_native_templates`]. Native-only: the cache it names does not
    /// exist on wasm, where the browser's JS preload is the equivalent.
    #[cfg(not(target_arch = "wasm32"))]
    fn requires_native_templates(self) -> bool {
        matches!(self, BootProfile::NativeHost)
    }
}

/// How a [`NativeHost`](BootProfile::NativeHost) boot puts pixels somewhere
/// (issue #1121).
///
/// This is the axis a native host varies that no browser profile can: on the
/// browser, "is there a renderer" is answered by the target itself
/// (`#[cfg(target_arch = "wasm32")]`), whereas native is simultaneously the
/// shipped host target AND the target `cargo test` runs on, and the machine
/// running the tests has no GPU. So the choice has to be made at runtime, from
/// the plan, rather than at compile time from a `cfg`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeRenderSurface {
    /// No wgpu at all: compose the shared core plus the renderer's *contract*,
    /// exactly as [`BrowserHost`](BootProfile::BrowserHost) does on the native
    /// parity-test target.
    ///
    /// The default, and the only value a GPU-less CI runner can build, so it is
    /// what the four-profile parity test uses. It is also the value the
    /// native↔headless digest-equivalence test uses: the claim there is about
    /// the *simulation* plugin graph, and a real surface would only add a GPU
    /// requirement to a determinism check.
    #[default]
    Contract,
    /// A real winit window driven by `DefaultPlugins` — the shipped
    /// `phoenix-host --world …` viewscreen.
    Window,
    /// A real wgpu device with **no** window: `WinitPlugin` disabled and no
    /// primary window, as `capture-billboard` and `tune-lods` already run. The
    /// automated native render proof draws into an offscreen target through
    /// this and reads the pixels back.
    Offscreen,
}

impl NativeRenderSurface {
    /// Whether this surface instantiates Bevy's real wgpu render plugins.
    /// Native-only: no browser profile consults this axis, because the browser
    /// answers the same question with a `cfg`.
    #[cfg(not(target_arch = "wasm32"))]
    fn is_wgpu(self) -> bool {
        matches!(
            self,
            NativeRenderSurface::Window | NativeRenderSurface::Offscreen
        )
    }
}

// ── Plan / error ─────────────────────────────────────────────────────────────

/// How this profile's world reaches the ECS.
///
/// Orthogonal to [`BootProfile`], exactly as [`BootPlan::single_threaded`] is: the
/// three-profile parity tests build **every** profile — the browser ones
/// included — through [`FromReader`](WorldIngest::FromReader) over a
/// `MemoryReader`, while the production browser boots
/// [`HostPreloaded`](WorldIngest::HostPreloaded) because its world genuinely
/// arrives by a different route (a JS preload) than the filesystem read headless
/// performs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldIngest {
    /// Boot owns the whole load: reset the ledger, read the root and its
    /// `extra_worlds` children through the [`BootPlan`] reader, validate the
    /// composition, compile each world's scripts for the pre-freeze declared
    /// set, apply the ledger records (+ native eager-record), freeze, and insert
    /// the root `WorldConfig` and root `PreCompiledScripts`. Static children are
    /// compiled again when their runtime layers activate. Headless and the
    /// parity tests.
    FromReader,
    /// The host already ingested the world by another route, so boot must not run
    /// the reader-based load at all. The browser's JS preload parses the
    /// `WorldConfig` into a thread-local and streams the entity-template records
    /// into the content ledger — resetting it at world-*selection* time, not here
    /// — and `WorldPlugin`'s `Startup` systems insert the `WorldConfig`, the
    /// `RawWorldSource`, and compile the scripts. Boot neither reads, resets, nor
    /// inserts the world; it owns the two order-critical steps the host cannot place
    /// itself at the right moment: the Rhai hashing-seed pin (before any engine)
    /// and a pending content-ledger freeze, completed at Startup after the root
    /// scripts have been compiled and recorded, before anything spawns.
    ///
    /// The [`BootPlan`]'s `world_path`, `reader`, `script_resolver` and
    /// `raw_transform` are unused in this mode — a `HostPreloaded` plan still
    /// carries the target-correct values (the browser's `WasmReader` and script
    /// resolver) for shape and future use, but [`build`] consults none of them.
    HostPreloaded,
    /// There is **no world yet** (issue #1326): compose the `App` and leave the
    /// whole ingestion — reset, read, validate, compile, apply, freeze, insert —
    /// to a later runtime load through [`ingest_world`] on the running `World`.
    ///
    /// The native host boots this way under `--lobby`: it opens its window on an
    /// empty `GamePhase::Lobby`, publishes the scenario catalogue, and ingests a
    /// world only once a `SelectScenario` + `SelectPlayerShip` pair has been
    /// arbitrated.
    ///
    /// Boot runs step 1 of [`ingest_world`]'s order and nothing else. It must
    /// NOT freeze: freezing is what seals the content digest for the world that
    /// is being loaded, and there is none — a freeze here would publish an empty
    /// content digest that the runtime load then has to reset out from under
    /// anything that read it. Nothing may spawn before that load, which is what
    /// makes "no frozen digest yet" safe rather than merely tolerable.
    Deferred,
}

/// A preloaded host still owes the root-script half of its content identity.
/// Consumed once by WorldPlugin's Startup chain after script compilation and
/// before either spawn pass. Reader-based and deferred boots never insert it.
#[derive(Resource)]
pub(crate) struct PendingHostContentFreeze;

/// Everything [`build`] needs that is not implied by the [`BootProfile`].
///
/// The world is supplied as a [`WorldReader`] plus a [`ScriptResolver`] rather than
/// baked in, so the same `build` serves the filesystem (headless: [`FsReader`]), the
/// JS fetch queue (browser: [`WasmReader`]) and an in-memory fixture (tests:
/// [`MemoryReader`]) without a target branch of its own.
///
/// [`FsReader`]: crate::world::load::FsReader
/// [`WasmReader`]: crate::world::load::WasmReader
/// [`MemoryReader`]: crate::world::load::MemoryReader
pub struct BootPlan {
    /// Which inventory to compose.
    pub profile: BootProfile,
    /// How this profile's world reaches the ECS — see [`WorldIngest`]. Headless
    /// and the parity tests use [`WorldIngest::FromReader`]; the production
    /// browser uses [`WorldIngest::HostPreloaded`].
    pub world_ingest: WorldIngest,
    /// The `EnvFilter` string handed to [`LogPlugin`] (already `warn`-prefixed by
    /// the caller, matching both existing boot paths).
    pub log_filter: String,
    /// Authored path of the root world TOML (its content-ledger / snapshot key).
    pub world_path: String,
    /// The world-TOML reader for this target.
    pub reader: Box<dyn WorldReader>,
    /// The sibling-`.rhai` script resolver for this target
    /// ([`crate::entities::config_cache::production_script_resolver`] in production).
    pub script_resolver: Box<dyn ScriptResolver>,
    /// Pin Bevy's [`TaskPoolPlugin`] to a single thread, so the executor runs
    /// systems in a fixed order run to run.
    ///
    /// A headless `--deterministic`/`--seed` run asks for this — reproducing a
    /// byte-identical digest needs the system execution order fixed, not just
    /// the timestep — and so does a
    /// [`NativeHost`](BootProfile::NativeHost) built for a digest comparison
    /// ([`crate::native_host::NativeHostConfig::deterministic`]). The browser
    /// profiles always leave it `false` (a rendered host is not reproduced
    /// tick-for-tick, and wasm has its own pool policy).
    ///
    /// **Honoured on every path that builds a task pool**, not only the
    /// surrogate one: [`render_stack`]'s native fallbacks and the real
    /// wgpu-backed [`native_render_stack`] all take it, the latter by
    /// `.set(TaskPoolPlugin { .. })` on `DefaultPlugins`. A silently-ignored
    /// determinism flag is worse than an absent one — it makes a digest
    /// comparison look pinned when it is not, which is exactly the trap the
    /// native↔headless equivalence test fell into before issue #1121's fix
    /// round.
    pub single_threaded: bool,
    /// Optional transform applied to the raw world `toml::Value` **before** its
    /// scripts compile — the seam `headless::duel::apply_duel_sides` rewrites the
    /// `--side-a`/`--side-b` slot roster through (issue #844). `None` for a plain
    /// run and for both browser profiles; when present it is attached to the
    /// [`LoadRequest`](crate::world::load::LoadRequest) [`ingest_world`] builds, so
    /// the load owns the one transform hook exactly as it owns the load itself.
    pub raw_transform: Option<Box<dyn Fn(toml::Value) -> Result<toml::Value, String>>>,
    /// How a [`NativeHost`](BootProfile::NativeHost) boot presents — see
    /// [`NativeRenderSurface`]. Inert for every other profile: the browser's
    /// renderer is chosen by the target, and the two renderer-less profiles have
    /// none to choose. Defaults to [`NativeRenderSurface::Contract`], which is
    /// the only value a GPU-less machine can build.
    pub native_surface: NativeRenderSurface,
}

/// Why [`build`] could not produce an `App`.
#[derive(Debug)]
pub enum BootError {
    /// The world could not be read, parsed or transformed —
    /// [`crate::world::load::load`] failed outright.
    WorldLoad(crate::world::load::LoadError),
    /// The world loaded, but its composition or scripts contain errors and this
    /// profile [aborts](BootProfile::broken_world_aborts) on a broken world. The
    /// string names the erroring findings.
    WorldInvalid(String),
    /// This profile [requires](BootProfile::requires_native_templates) the
    /// native entity-template cache to hold every template its world declares,
    /// and these are missing (issue #1121). Nothing downstream would have
    /// *failed* on them — the cache-only readers answer `Default` — which is
    /// exactly why the boot refuses here instead.
    NativeTemplatesMissing(Vec<String>),
}

impl fmt::Display for BootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootError::WorldLoad(e) => write!(f, "world load failed: {e}"),
            BootError::WorldInvalid(msg) => write!(f, "world activation blocked: {msg}"),
            BootError::NativeTemplatesMissing(missing) => write!(
                f,
                "native entity-template cache is missing {} template(s) this world \
                 declares, so the host would read Default hull, radar and asteroid \
                 configuration instead of the authored ones: {}",
                missing.len(),
                missing.join(", ")
            ),
        }
    }
}

impl std::error::Error for BootError {}

// ── Seam markers ─────────────────────────────────────────────────────────────
//
// Both `render_surrogate` and `render_stack`'s native fallback register the SAME
// asset/message contract (that is what "the surrogate is what a missing renderer
// owes" means), so the observable that tells which path a profile took is a
// dedicated zero-sized marker rather than the contract itself. The three-profile
// parity test asserts the contract holds for all three AND that the stack marker
// is present only for BrowserHost.

/// Marks that [`render_surrogate`] ran (Headless / BrowserAutomation).
#[derive(Resource, Debug, Default, Clone, Copy)]
struct RenderSurrogateApplied;

/// Marks that [`render_stack`] ran (BrowserHost).
#[derive(Resource, Debug, Default, Clone, Copy)]
struct RenderStackApplied;

// ── build ────────────────────────────────────────────────────────────────────

/// Compose an `App` for `plan`'s [`BootProfile`].
///
/// The renderer-less profiles take the shared [`core_plugins`] then
/// [`render_surrogate`]; BrowserHost takes [`render_stack`], which owns the whole
/// plugin stack itself because on the browser its renderer is `DefaultPlugins`, a
/// superset of `core_plugins` (see the note below). Every profile then runs
/// [`ingest_world`]. The simulation, lobby and world plugins the two boot paths add
/// around this seam are the adopting adapters' to attach (#1218 headless / #1219
/// `wasm_init`) — this function owns only what actually differs per profile plus
/// the world-ingestion order.
pub fn build(plan: BootPlan) -> Result<App, BootError> {
    let mut app = App::new();

    // Command/system errors WARN rather than abort the process (Bevy 0.18's
    // `DefaultErrorHandler`, set once here so every target — browser via
    // `wasm_init`, native, headless — shares it). Bevy 0.18 made a class of
    // command fatal that older Bevy silently ignored: a command applied to an
    // entity another system despawned the same frame. The game shipped and
    // played for years with those ignored, so panicking on them is a
    // regression, not a new safety net — most visibly a native host crashing a
    // few seconds into a mission on a combat despawn↔command race (the entity
    // varies per run), which drops every joined phone. `warn` restores the
    // intended semantics and, unlike `ignore`, LOGS each occurrence (with the
    // caller under `track_location`), so a genuine logic error stays visible
    // and fixable rather than hidden.
    app.insert_resource(bevy::ecs::error::DefaultErrorHandler(
        bevy::ecs::error::warn,
    ));

    // Shared artifact metadata for the peer-local save lifecycle. This comes
    // from the same BootPlan on browser, native, and headless profiles; target
    // adapters therefore cannot disagree about which scenario a capture names.
    app.insert_resource(crate::save_slots_lifecycle::SaveScenario(
        plan.world_path.clone(),
    ));

    // The render-stack profile's shared core rides in *with* its renderer: on the
    // browser that renderer is `DefaultPlugins`, which is a superset of
    // [`core_plugins`] (it carries `PanicHandlerPlugin`, `LogPlugin`, the task
    // pool and the rest itself), so adding both would double-add those plugins and
    // Bevy panics on a duplicate. [`render_stack`] therefore owns the whole plugin
    // stack for BrowserHost — and calls [`core_plugins`] itself on the native
    // parity-test target, which cannot stand up the real wgpu renderer. The two
    // renderer-less profiles keep the original shape: the shared core, then the
    // surrogate that stands in for a missing renderer.
    if plan.profile.has_render_stack() {
        render_stack(
            &mut app,
            &plan.log_filter,
            plan.profile,
            plan.native_surface,
            plan.single_threaded,
        );
    } else {
        core_plugins(
            &mut app,
            plan.profile,
            &plan.log_filter,
            plan.single_threaded,
        );
        render_surrogate(&mut app);
    }

    ingest_world(app.world_mut(), &plan)?;

    Ok(app)
}

// ── core_plugins ─────────────────────────────────────────────────────────────

/// The Bevy core the three inventories agree on, plus the browser window shell
/// where the profile needs it.
///
/// The core list is the intersection of `build_headless_app`'s and both
/// `wasm_init` branches' plugin sets: panic handling, logging, the task pool,
/// frame counting, time, transforms, diagnostics, assets, scenes and states. The
/// browser profiles add input, a canvas window and accessibility on top; the
/// winit event loop is added only on the browser target (see [`browser_shell`]).
///
/// `single_threaded` pins the task pool to one thread, for a headless
/// deterministic run — see [`BootPlan::single_threaded`].
fn core_plugins(app: &mut App, profile: BootProfile, log_filter: &str, single_threaded: bool) {
    let task_pool = task_pool_plugin(single_threaded);
    app.add_plugins((
        PanicHandlerPlugin,
        LogPlugin {
            // Our own `LogCat`s gate the `plog!` call sites; this filter governs
            // only bevy-internal events. The caller has already `warn`-prefixed it.
            filter: log_filter.to_string(),
            ..default()
        },
        task_pool,
        FrameCountPlugin,
        TimePlugin,
        TransformPlugin,
        DiagnosticsPlugin,
        asset_plugin(profile),
        ScenePlugin,
        StatesPlugin,
    ));

    if profile.is_browser() {
        browser_shell(app);
    }
}

/// The [`TaskPoolPlugin`] every composition path takes, so
/// [`BootPlan::single_threaded`] has exactly one implementation.
///
/// Both arms are a `TaskPoolPlugin`, so a caller can drop it into a plugin
/// tuple or into `DefaultPlugins::set` without a type dance. A deterministic run
/// needs a fixed system execution order, which a one-thread pool gives and the
/// multithreaded default does not.
fn task_pool_plugin(single_threaded: bool) -> TaskPoolPlugin {
    if single_threaded {
        TaskPoolPlugin {
            task_pool_options: bevy::app::TaskPoolOptions::with_num_threads(1),
        }
    } else {
        TaskPoolPlugin::default()
    }
}

/// The asset plugin for `profile`. The browser profiles never request `.meta`
/// sidecars — none ship, and Cloudflare Pages (the demo host) answers a missing
/// one with its SPA `index.html` at HTTP 200, which the default `AssetMetaCheck`
/// reads as a corrupt sidecar and dies on (see both `wasm_init` branches). Native
/// headless keeps the default: it reads real files off disk.
fn asset_plugin(profile: BootProfile) -> AssetPlugin {
    if profile.is_browser() {
        AssetPlugin {
            meta_check: bevy::asset::AssetMetaCheck::Never,
            ..default()
        }
    } else {
        AssetPlugin::default()
    }
}

/// The browser window shell: input, a `#canvas` window and accessibility.
///
/// [`WinitPlugin`](bevy::winit::WinitPlugin) owns the browser event loop and is
/// added only under `target_arch = "wasm32"`. On the browser that loop attaches to
/// the canvas; on the native parity-test target it has no display to open, so it is
/// left out — the identical target split `wasm_init` already lives under (the
/// automation branch adds `WinitPlugin`, but only ever executes in a real browser).
fn browser_shell(app: &mut App) {
    use bevy::a11y::AccessibilityPlugin;
    use bevy::input::InputPlugin;
    app.add_plugins((
        InputPlugin,
        bevy::window::WindowPlugin {
            primary_window: Some(bevy::window::Window {
                canvas: Some("#canvas".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        },
        AccessibilityPlugin,
    ));
    #[cfg(target_arch = "wasm32")]
    app.add_plugins(bevy::winit::WinitPlugin::default());
}

// ── render surrogate / stack ─────────────────────────────────────────────────

/// What a missing renderer owes the simulation, for the two profiles that have no
/// renderer (Headless, BrowserAutomation).
///
/// Both existing boot paths register exactly this by hand when they skip the
/// render stack (`build_headless_app` after its core plugins; `wasm_init`'s
/// automation branch). Consolidated here so the surrogate cannot drift from the
/// real stack's contract.
fn render_surrogate(app: &mut App) {
    register_render_contract(app);
    app.insert_resource(RenderSurrogateApplied);
}

/// The real viewscreen renderer, for the two render-stack profiles.
///
/// Owns the whole plugin stack for those profiles (see [`build`]'s note): the
/// renderer is Bevy's `DefaultPlugins` — the shared core **and** the wgpu render
/// plugins in one group — so [`core_plugins`] is *not* also called when the real
/// stack goes in.
///
/// Two targets, two different questions:
///
/// * **Browser** ([`BrowserHost`](BootProfile::BrowserHost)): the wgpu-backed
///   plugins are instantiated only under `target_arch = "wasm32"`. A native
///   build (the target the parity test runs on) cannot stand up that stack —
///   Bevy's `RenderPlugin` requests a GPU adapter and panics with none, which is
///   the very reason the [`BrowserAutomation`](BootProfile::BrowserAutomation)
///   inventory exists. So on native this composes the shared core plus the
///   renderer's *contract* (the same floor [`render_surrogate`] provides), which
///   is what lets `build(BrowserHost)` compose on native at all.
/// * **Native** ([`NativeHost`](BootProfile::NativeHost)): native is *both* the
///   shipped host target and the test target, so the same question is answered
///   at runtime from [`BootPlan::native_surface`] instead — see
///   [`native_render_stack`].
///
/// Either way [`RenderStackApplied`] is inserted, because the profile named the
/// real renderer; whether wgpu could actually be stood up on this target is a
/// separate fact.
fn render_stack(
    app: &mut App,
    log_filter: &str,
    profile: BootProfile,
    surface: NativeRenderSurface,
    single_threaded: bool,
) {
    app.insert_resource(RenderStackApplied);

    #[cfg(not(target_arch = "wasm32"))]
    if profile == BootProfile::NativeHost {
        native_render_stack(app, log_filter, surface, single_threaded);
        return;
    }
    // Consumed only by the native arm above / the wasm arm below; naming them
    // here keeps every target's build free of "unused variable" noise without a
    // second `cfg` block per parameter.
    let _ = (profile, surface, single_threaded);

    // `feature = "server"` as well as `wasm32` (issue #1194): this branch names the
    // presentation `crate::server::{renderer,viewscreen_border}` plugins, so the
    // always-compiled boot module must not reference them with the feature off. The
    // browser host always builds with the default `server` feature, so `all(wasm32,
    // server)` is exactly the real BrowserHost build — no behaviour change — while
    // keeping this simulation-side module free of any ungated `crate::server` name.
    #[cfg(all(target_arch = "wasm32", feature = "server"))]
    {
        // The full Bevy stack the browser host runs on, customised exactly as the
        // pre-#1219 `wasm_init` real branch did (issue #1219): the `#canvas`
        // window, the page's log filter, and `AssetMetaCheck::Never` — no `.meta`
        // sidecars ship, and Cloudflare Pages (the demo host) answers a missing one
        // with its SPA `index.html` at HTTP 200, which the default check reads as a
        // corrupt sidecar and dies on. `DefaultPlugins` carries every plugin
        // [`core_plugins`] would add, so this REPLACES it for BrowserHost.
        app.add_plugins(
            bevy::DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: Some(bevy::window::Window {
                        canvas: Some("#canvas".into()),
                        fit_canvas_to_parent: true,
                        ..default()
                    }),
                    ..default()
                })
                .set(LogPlugin {
                    filter: log_filter.to_string(),
                    ..default()
                })
                .set(AssetPlugin {
                    meta_check: bevy::asset::AssetMetaCheck::Never,
                    ..default()
                }),
        );
        // Do NOT re-register the four asset types here. `DefaultPlugins`' render
        // stack already `init_asset`s Shader/Image/Mesh/StandardMaterial AND
        // installs the `ShaderLoader`; calling `register_render_assets` on top of
        // it registers a SECOND `ShaderLoader` for the same extensions (Bevy warns
        // "Duplicate AssetLoader registered for … Shader") and leaves the shader
        // `Assets` storage and its index allocator out of step, which panics in
        // `DenseAssetStorage::insert` ("index out of bounds") the moment the
        // pipeline loads a shader — trapping the wasm instance so the sim loop, and
        // with it the Welcome handshake, never runs (issue #1219 regressed this;
        // the pre-#1219 real branch added ONLY `DefaultPlugins` here). The
        // renderer-less profiles still need the manual registration — that is what
        // `render_surrogate`/`register_render_contract` are for — but the
        // BrowserHost render path must leave it entirely to `DefaultPlugins`.
        // `AiChatterEvent` is NOT part of that render-owned set (it rides in on
        // `ShipPlugin` in the full app), so it is added explicitly; `add_message`
        // is idempotent.
        app.add_message::<AiChatterEvent>();
        app.add_plugins(crate::server::renderer::RendererPlugin)
            .add_plugins(crate::server::viewscreen_border::ViewscreenBorderPlugin);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // The parity-test target. `DefaultPlugins`' wgpu renderer would panic here,
        // so stand up the shared core and the renderer's contract instead — a
        // browser run never reaches this arm. `single_threaded` still travels:
        // a production BrowserHost plan always leaves it `false`, and forwarding
        // it rather than hardcoding one keeps "the plan is honoured" true on
        // every branch instead of on most of them.
        core_plugins(app, BootProfile::BrowserHost, log_filter, single_threaded);
        register_render_contract(app);
    }
}

/// The native windowed host's renderer (issue #1121).
///
/// The browser arm above and this one compose the *same* three things —
/// `DefaultPlugins`, the [`AiChatterEvent`] message, and the two presentation
/// plugins — differing only in what the window is and where assets come from:
///
/// * **Window.** No `canvas`; a real winit window with the viewscreen title.
///   [`NativeRenderSurface::Offscreen`] instead disables `WinitPlugin` and asks
///   for no primary window at all, which is how `capture-billboard` and
///   `tune-lods` already drive native wgpu in this repo — the shape the
///   automated native render proof uses, because CI has no display.
/// * **Assets.** The browser sets `AssetMetaCheck::Never` because no `.meta`
///   sidecars ship and Cloudflare Pages answers a missing one with its SPA
///   `index.html` at HTTP 200, which the default check reads as a corrupt
///   sidecar and dies on. A native host reads real files off a real disk, so it
///   keeps `AssetPlugin::default()` — and resolves it against `BEVY_ASSET_ROOT`,
///   which `native_host::pin_content_root` sets from `--content-dir` so that
///   Bevy's asset root and the CWD that `std::fs`-based world/template/sidecar
///   reads resolve against cannot disagree.
///
/// Note what is deliberately absent: [`register_render_contract`]. With the real
/// stack in, `DefaultPlugins` already `init_asset`s Shader/Image/Mesh/
/// StandardMaterial and installs the `ShaderLoader`, and registering a second
/// one panics in `DenseAssetStorage::insert` the moment a shader loads (the
/// regression issue #1219 shipped). `AiChatterEvent` is not part of that
/// render-owned set, so it is added explicitly; `add_message` is idempotent.
///
/// With [`NativeRenderSurface::Contract`] no wgpu is stood up at all and this
/// falls back to the shared core plus the renderer's contract — the same
/// treatment `BrowserHost` gets on native, and what makes the four-profile
/// parity test and the native↔headless digest comparison runnable on a
/// GPU-less CI runner.
///
/// [`BootPlan::single_threaded`] is honoured on **all three** of those paths,
/// the real wgpu one included: `DefaultPlugins` carries its own
/// [`TaskPoolPlugin`], so the plan's answer is `.set` over it rather than
/// dropped. That is what lets the `#[ignore]`d `Offscreen` digest companion in
/// `tests/native_headless_digest.rs` be a pinned comparison rather than a race
/// between two task pools.
#[cfg(not(target_arch = "wasm32"))]
fn native_render_stack(
    app: &mut App,
    log_filter: &str,
    surface: NativeRenderSurface,
    single_threaded: bool,
) {
    if !surface.is_wgpu() {
        core_plugins(app, BootProfile::NativeHost, log_filter, single_threaded);
        register_render_contract(app);
        return;
    }

    #[cfg(feature = "server")]
    {
        use bevy::window::{ExitCondition, Window, WindowPlugin};
        let window_plugin = match surface {
            NativeRenderSurface::Offscreen => WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            },
            _ => WindowPlugin {
                primary_window: Some(Window {
                    title: crate::native_host::WINDOW_TITLE.to_string(),
                    ..default()
                }),
                ..default()
            },
        };
        let plugins = bevy::DefaultPlugins
            .set(window_plugin)
            .set(LogPlugin {
                filter: log_filter.to_string(),
                ..default()
            })
            .set(AssetPlugin::default())
            // `DefaultPlugins` brings its own `TaskPoolPlugin`, so the plan's
            // determinism answer has to REPLACE it rather than ride alongside
            // it — Bevy panics on a duplicate plugin.
            .set(task_pool_plugin(single_threaded));
        if surface == NativeRenderSurface::Offscreen {
            app.add_plugins(plugins.disable::<bevy::winit::WinitPlugin>());
        } else {
            app.add_plugins(plugins);
        }
        app.add_message::<AiChatterEvent>();
        app.add_plugins(crate::server::renderer::RendererPlugin)
            .add_plugins(crate::server::viewscreen_border::ViewscreenBorderPlugin);
    }
    // A `--no-default-features` build has no presentation half to render with,
    // so there is no native viewscreen to stand up — take the contract, exactly
    // as `Contract` does. Nothing ships this combination; the boundary job
    // compiles it.
    #[cfg(not(feature = "server"))]
    {
        core_plugins(app, BootProfile::NativeHost, log_filter, single_threaded);
        register_render_contract(app);
    }
}

/// The four asset types a renderer registers that simulation systems name even
/// when nothing is drawn.
fn register_render_assets(app: &mut App) {
    app.init_asset::<Shader>()
        .init_asset_loader::<ShaderLoader>()
        .init_asset::<Image>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>();
}

/// The full "what a missing renderer owes the simulation" contract: the four asset
/// types, the three host-page bridge messages, and the lobby-state push system that
/// [`ViewscreenBorderPlugin`](crate::server::viewscreen_border::ViewscreenBorderPlugin)
/// would otherwise own. Registered by [`render_surrogate`] and by [`render_stack`]'s
/// native fallback.
fn register_render_contract(app: &mut App) {
    register_render_assets(app);
    app.add_message::<HudStateChanged>()
        .add_message::<LobbyStateChanged>()
        .add_message::<AiChatterEvent>();
    // The one system the missing ViewscreenBorderPlugin still owes the HTML lobby
    // overlay. Its parameters are all `Option<Res<_>>` bar the `GamePhase` state,
    // so a boot that never inits that state simply never runs it. `server`-gated
    // (issue #1194): `push_lobby_state` lives in the presentation
    // `crate::server::viewscreen_border`, which the feature-off build has no module
    // for — and the HTML lobby overlay it feeds is a browser/host (server) surface,
    // so there is nothing for it to push when the feature is absent.
    #[cfg(feature = "server")]
    app.add_systems(Update, crate::server::viewscreen_border::push_lobby_state);
}

// ── ingest_world ─────────────────────────────────────────────────────────────

/// The sole caller of [`crate::world::load::load`], and the sole owner of the
/// content-ledger and Rhai-seed order a boot must run in.
///
/// Two modes, per the plan's [`WorldIngest`]. Under
/// [`HostPreloaded`](WorldIngest::HostPreloaded) the host loaded the world by
/// another route (the browser's JS preload), so boot runs step 1 and then only the
/// pending freeze from step 5 — it does not reset, read, or insert the world.
/// Startup completes that freeze after compiling the preloaded root scripts.
/// The order below is the
/// [`FromReader`](WorldIngest::FromReader) path (headless and the parity tests).
///
/// The order, and why it is this order:
///
/// 1. [`init_hashing_seed`](crate::world::script::init_hashing_seed) — before any
///    script engine can be built. `set_hashing_seed` silently no-ops once a hash
///    has been taken, so it must be genuinely first; idempotent across boots.
/// 2. [`content_ledger::reset`](crate::content_ledger::reset) — a new boot is a new
///    world load; clear the ledger before the load records into it so a second
///    `build` in one process never inherits the previous world's files.
/// 3. [`load`](crate::world::load::load) under [`LoadPolicy::Activate`] — the one
///    read/parse/validate/compile of the root and its `extra_worlds` children.
/// 4. The composition + root-script gate, applied per the profile's
///    [abort-vs-block](BootProfile::broken_world_aborts) policy. A broken static
///    child's script set always aborts: unlike the root set, it is not inserted
///    as `PreCompiledScripts` for a browser-side downstream gate to retain.
/// 5. Apply the ledger records the load gathered, eager-record the declared entity
///    templates (native only — the browser's JS preload is its equivalent), and
///    [`freeze`](crate::content_ledger::freeze) — so the content digest a save is
///    checked against does not drift as the world streams in.
/// 6. Hand the parsed root world and its once-compiled root scripts to the `App`
///    as resources for `WorldPlugin`'s `Startup` to consume; a broken-but-not-
///    aborted browser root carries its findings through so the downstream gate
///    blocks activation. Static-child compiled sets do not cross this boundary.
///
/// # Called twice, deliberately
///
/// [`build`] calls this on the `World` of the `App` it is composing. The native
/// host's runtime world load (issue #1326) calls it on the `World` of an `App`
/// that is **already running** — a host that booted into an empty lobby and has
/// since had a scenario chosen. That is why it takes a `&mut World` rather than
/// a `&mut App`: there is no `App` to hand it at the second call site, and there
/// must not be a second implementation of this order. Everything the runtime
/// path needs to be the boot path — the reset/apply/eager-record/freeze
/// sequence, the abort-vs-block policy, the native template gate, and which two
/// resources are inserted — is therefore stated once, here.
pub(crate) fn ingest_world(world: &mut World, plan: &BootPlan) -> Result<(), BootError> {
    // Step 1 for both modes: the Rhai hashing-seed pin. Genuinely first, before any
    // script engine — `set_hashing_seed` no-ops once a hash is taken. Idempotent
    // across boots and across the browser's own earlier calls.
    crate::world::script::init_hashing_seed();

    match plan.world_ingest {
        // The host already ingested the world by another route (the browser's JS
        // preload + `WorldPlugin`'s Startup systems — see
        // [`WorldIngest::HostPreloaded`]). Boot does not read, reset, or insert
        // the world; it requests the freeze that seals the content digest after
        // preload AND script compilation, before anything spawns. Freezing here
        // omitted the root's compiled script record from browser saves (#1316).
        // The host reset the ledger and
        // streamed its records in at world-selection time, so a reset here would
        // wipe them.
        WorldIngest::HostPreloaded => {
            world.insert_resource(PendingHostContentFreeze);
            return Ok(());
        }
        // No world yet (issue #1326) — the seed pin above is the whole of boot's
        // job, and a later call to this same function on the running `World` owns
        // everything below. Deliberately no freeze: see [`WorldIngest::Deferred`].
        WorldIngest::Deferred => return Ok(()),
        WorldIngest::FromReader => {}
    }

    crate::content_ledger::reset();

    let mut request = LoadRequest::new(
        plan.world_path.clone(),
        plan.reader.as_ref(),
        plan.script_resolver.as_ref(),
        LoadPolicy::Activate,
    );
    // The one raw-value transform hook (headless's `--side-a`/`--side-b` duel
    // seam). Borrowed from `plan`, which outlives this load call.
    if let Some(transform) = &plan.raw_transform {
        request = request.with_transform(&**transform);
    }
    let loaded = load(request).map_err(BootError::WorldLoad)?;

    // The activation gate. Both the composition findings and the compiled scripts'
    // own findings can carry errors. Root errors follow the profile's abort-vs-
    // block policy because the root `CompiledScripts` is retained below for the
    // browser's downstream gate. Static-child compiled sets are pre-freeze inputs,
    // not runtime resources; no downstream owner receives them, so a broken child
    // must be rejected here on every profile rather than silently dropped.
    let mut invalid: Vec<String> = Vec::new();
    // Non-blocking findings first, and they have to be LOGGED rather than
    // counted (issue #1046). `LoadedWorld::findings` had exactly one production
    // consumer — the `has_error` gate below — and `describe_findings` filters to
    // errors, so every warning a validator produced was dropped unread. That is
    // tolerable for a check whose warning is decoration, and fatal for one whose
    // warning IS the report: `validate_doctrine_anchors_in` softens to a warning
    // exactly where it cannot prove the defect, and a warning nobody prints is
    // indistinguishable from no check at all.
    log_non_error_findings("composition", &loaded.findings);
    if let Some(scripts) = &loaded.scripts {
        log_non_error_findings("scripts", &scripts.findings);
    }
    let mut invalid_child_scripts = Vec::new();
    for (index, child) in loaded.children.iter().enumerate() {
        let label = format!("extra_world[{index}] scripts");
        if let Some(scripts) = &child.scripts {
            log_non_error_findings(&label, &scripts.findings);
            if crate::world::validate::has_error(&scripts.findings) {
                invalid_child_scripts.push(describe_findings(&label, &scripts.findings));
            }
        }
    }
    if !invalid_child_scripts.is_empty() {
        return Err(BootError::WorldInvalid(invalid_child_scripts.join("; ")));
    }
    if crate::world::validate::has_error(&loaded.findings) {
        invalid.push(describe_findings("composition", &loaded.findings));
    }
    if let Some(scripts) = &loaded.scripts {
        if crate::world::validate::has_error(&scripts.findings) {
            invalid.push(describe_findings("scripts", &scripts.findings));
        }
    }
    if !invalid.is_empty() && plan.profile.broken_world_aborts() {
        return Err(BootError::WorldInvalid(invalid.join("; ")));
    }

    // The native content gate (issue #1121), before anything is inserted into
    // the `App`: a profile that reads the native template cache with no
    // filesystem fallback must find that cache already carrying this world's
    // declared templates. See [`check_native_templates`].
    #[cfg(not(target_arch = "wasm32"))]
    if plan.profile.requires_native_templates() {
        // The COMPOSED world — root plus every `extra_worlds` child — because
        // that is the set the eager record twenty lines below walks, and a
        // template declared only by a static child is just as cache-only to the
        // readers as one declared by the root.
        check_native_templates(
            std::iter::once(&loaded.config).chain(loaded.children.iter().map(|c| &c.config)),
        )?;
    }

    loaded.ledger.apply();
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::content_ledger::eager_record_world_entities_with_scripts(
            &loaded.config,
            loaded.scripts.as_ref(),
        );
        for child in &loaded.children {
            crate::content_ledger::eager_record_world_entities_with_scripts(
                &child.config,
                child.scripts.as_ref(),
            );
        }
    }
    crate::content_ledger::freeze();

    world.insert_resource(loaded.config);
    world.insert_resource(crate::world::server::PreCompiledScripts(loaded.scripts));
    Ok(())
}

/// Refuse to compose a native host whose world declares templates the native
/// entity-template cache does not hold (issue #1121).
///
/// The set checked is [`crate::world::config::entity_template_paths`] with no
/// curation — every static `[[entity]]`, every `available_ships[*]` hull, and
/// every literal `template_path` a compiled script spawns — over the **composed
/// world**: the root and every `extra_worlds` child. That is exactly the set the
/// browser's JS preload fetches before Bevy starts and exactly the set
/// [`ingest_world`]'s eager record walks, so this is the native statement of the
/// same precondition rather than a new rule. Checking the root alone would let a
/// hull declared only by a static child through the gate and straight into the
/// silent-`Default` failure below.
///
/// It is a *refusal*, not a warning, because the failure it guards is silent:
/// `lobby::server::update_session_with_config` reads the selected hull straight
/// out of this cache with no fallback, and on a miss keeps a `Default`
/// `ShipClientConfig` — default helm radar range, default impulse-charge
/// duration, default hostile-arc colour — while every log line stays clean and
/// the mission still runs. `server::radar`, `server::reference_grid`,
/// `server::asset_preload` and `asteroids::lifecycle` fail the same way.
///
/// The populate itself is
/// [`crate::entities::template_preload::preload_entity_templates`], which the
/// native host adapter runs before calling [`build`]. Boot does not run it
/// here: the preload's model-marker gate has to abort *before* an `App` exists,
/// and its findings belong to the adapter that will log them once a subscriber
/// is installed.
#[cfg(not(target_arch = "wasm32"))]
fn check_native_templates<'a>(
    worlds: impl IntoIterator<Item = &'a crate::world::config::WorldConfig>,
) -> Result<(), BootError> {
    let cache = crate::entities::config_cache::get_config_cache();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut missing: Vec<String> = Vec::new();
    for world in worlds {
        for path in crate::world::config::entity_template_paths(world, &[]) {
            let key = crate::entities::include_resolve::canonical_template_path(&path);
            // Root and child may name the same hull; report it once, in the
            // order the composed walk first met it.
            if cache.contains_key(&key) || !seen.insert(key.clone()) {
                continue;
            }
            missing.push(key);
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(BootError::NativeTemplatesMissing(missing))
    }
}

/// Log every NON-error finding of one gate at warn level (issue #1046).
///
/// The sibling of [`describe_findings`], and the reason both exist: an error
/// rides into [`BootError::WorldInvalid`] and stops the boot, so it is seen
/// whatever the log level; a warning has nowhere else to go. Each line carries
/// the category, the source file and — when the validator could resolve one —
/// the LINE, because the findings that land here are the ones asking an author
/// to go and look at a specific spawn.
fn log_non_error_findings(kind: &str, findings: &[crate::world::validate::WorldFinding]) {
    for finding in findings.iter().filter(|f| !f.is_error()) {
        let at = match finding.source.line {
            Some(line) => format!("{}:{line}", finding.source.file),
            None => finding.source.file.clone(),
        };
        bevy::log::warn!(
            "world {kind} [{}] {at}: {}",
            finding.category,
            finding.message
        );
    }
}

/// Render the erroring findings of one gate (`composition` or `scripts`) into the
/// message a [`BootError::WorldInvalid`] carries.
fn describe_findings(kind: &str, findings: &[crate::world::validate::WorldFinding]) -> String {
    let errors: Vec<String> = findings
        .iter()
        .filter(|f| f.is_error())
        .map(|f| format!("[{}] {}", f.category, f.message))
        .collect();
    format!(
        "{kind} invalid ({} error(s)): {}",
        errors.len(),
        errors.join("; ")
    )
}

#[cfg(test)]
mod tests;
