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
//! [`BootProfile`] and composes the core once, so the three inventories cannot
//! drift apart unnoticed — a drift the [three-profile parity test](self#tests)
//! guards permanently.
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

/// Which of the three inventories to compose.
///
/// The two axes the three profiles vary along are read off this enum by the
/// private predicates below rather than matched inline, so a fourth profile (or a
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
    fn broken_world_aborts(self) -> bool {
        matches!(self, BootProfile::Headless)
    }

    /// Whether this profile drives the real renderer ([`render_stack`]) rather
    /// than the [`render_surrogate`].
    fn has_render_stack(self) -> bool {
        matches!(self, BootProfile::BrowserHost)
    }

    /// Whether this profile runs inside a browser window and so needs the
    /// input/window/winit shell on top of the shared core.
    fn is_browser(self) -> bool {
        matches!(
            self,
            BootProfile::BrowserHost | BootProfile::BrowserAutomation
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
    /// inserts; it owns only the two order-critical calls the host cannot place
    /// itself at the right moment: the Rhai hashing-seed pin (before any engine)
    /// and the content-ledger freeze (after the preload, before anything spawns).
    ///
    /// The [`BootPlan`]'s `world_path`, `reader`, `script_resolver` and
    /// `raw_transform` are unused in this mode — a `HostPreloaded` plan still
    /// carries the target-correct values (the browser's `WasmReader` and script
    /// resolver) for shape and future use, but [`build`] consults none of them.
    HostPreloaded,
}

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
    /// Only a headless `--deterministic`/`--seed` run asks for this — reproducing
    /// a byte-identical digest needs the system execution order fixed, not just
    /// the timestep. The browser profiles always leave it `false` (a rendered
    /// host is not reproduced tick-for-tick, and wasm has its own pool policy).
    pub single_threaded: bool,
    /// Optional transform applied to the raw world `toml::Value` **before** its
    /// scripts compile — the seam `headless::duel::apply_duel_sides` rewrites the
    /// `--side-a`/`--side-b` slot roster through (issue #844). `None` for a plain
    /// run and for both browser profiles; when present it is attached to the
    /// [`LoadRequest`](crate::world::load::LoadRequest) [`ingest_world`] builds, so
    /// the load owns the one transform hook exactly as it owns the load itself.
    pub raw_transform: Option<Box<dyn Fn(toml::Value) -> Result<toml::Value, String>>>,
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
}

impl fmt::Display for BootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BootError::WorldLoad(e) => write!(f, "world load failed: {e}"),
            BootError::WorldInvalid(msg) => write!(f, "world activation blocked: {msg}"),
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
        render_stack(&mut app, &plan.log_filter);
    } else {
        core_plugins(
            &mut app,
            plan.profile,
            &plan.log_filter,
            plan.single_threaded,
        );
        render_surrogate(&mut app);
    }

    ingest_world(&mut app, &plan)?;

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
    // Both arms are a `TaskPoolPlugin`, so the tuple below stays one type; a
    // deterministic run needs a fixed system execution order, which a
    // single-threaded pool gives and the multithreaded default does not.
    let task_pool = if single_threaded {
        TaskPoolPlugin {
            task_pool_options: bevy::app::TaskPoolOptions::with_num_threads(1),
        }
    } else {
        TaskPoolPlugin::default()
    };
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

/// The real viewscreen renderer, for BrowserHost only.
///
/// Owns the whole plugin stack for this profile (see [`build`]'s note): on the
/// browser that is Bevy's `DefaultPlugins` — the shared core **and** the wgpu
/// render plugins in one group — so [`core_plugins`] is *not* also called for
/// BrowserHost.
///
/// The wgpu-backed render plugins are instantiated **only on the browser target**.
/// A native build (the target the parity test runs on) cannot stand up the render
/// stack at all — Bevy's `RenderPlugin` requests a GPU adapter and panics with none,
/// which is the very reason the [`BrowserAutomation`](BootProfile::BrowserAutomation)
/// inventory exists. So on native this composes the shared core ([`core_plugins`])
/// plus the renderer's *contract* with the simulation (the same floor
/// [`render_surrogate`] provides), which is what lets `build(BrowserHost)` compose on
/// native and the parity test assert the shared four-asset/three-message floor.
fn render_stack(app: &mut App, log_filter: &str) {
    app.insert_resource(RenderStackApplied);

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
        // browser run never reaches this arm.
        core_plugins(app, BootProfile::BrowserHost, log_filter, false);
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
/// freeze from step 5 — it does not reset, read, or insert. The order below is the
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
fn ingest_world(app: &mut App, plan: &BootPlan) -> Result<(), BootError> {
    // Step 1 for both modes: the Rhai hashing-seed pin. Genuinely first, before any
    // script engine — `set_hashing_seed` no-ops once a hash is taken. Idempotent
    // across boots and across the browser's own earlier calls.
    crate::world::script::init_hashing_seed();

    // The host already ingested the world by another route (the browser's JS
    // preload + `WorldPlugin`'s Startup systems — see [`WorldIngest::HostPreloaded`]).
    // Boot does not read, reset, or insert anything; it owns only the freeze that
    // seals the content digest after the preload and before anything spawns. The
    // host reset the ledger and streamed its records in at world-selection time, so
    // a reset here would wipe them.
    if matches!(plan.world_ingest, WorldIngest::HostPreloaded) {
        crate::content_ledger::freeze();
        return Ok(());
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

    app.insert_resource(loaded.config);
    app.insert_resource(crate::world::server::PreCompiledScripts(loaded.scripts));
    Ok(())
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
