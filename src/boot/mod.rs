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
//! # Additive — nothing calls [`build`] yet
//!
//! This issue introduces the module and its guard test only. The headless and
//! `wasm_init` adapters adopt it in later issues (#1218/#1219), each behind its own
//! evidence gate (a digest A/B for headless; a Playwright smoke for `wasm_init`).
//! No existing boot or world-ingestion call site changes here.
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
    /// The `EnvFilter` string handed to [`LogPlugin`] (already `warn`-prefixed by
    /// the caller, matching both existing boot paths).
    pub log_filter: String,
    /// Authored path of the root world TOML (its content-ledger / snapshot key).
    pub world_path: String,
    /// The world-TOML reader for this target.
    pub reader: Box<dyn WorldReader>,
    /// The sibling-`.rhai` script resolver for this target
    /// ([`crate::config_cache::production_script_resolver`] in production).
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
// is present only for BrowserHost. Inserted into a boot-composed `App` only —
// nothing composes via `boot::build` yet.

/// Marks that [`render_surrogate`] ran (Headless / BrowserAutomation).
#[derive(Resource, Debug, Default, Clone, Copy)]
struct RenderSurrogateApplied;

/// Marks that [`render_stack`] ran (BrowserHost).
#[derive(Resource, Debug, Default, Clone, Copy)]
struct RenderStackApplied;

// ── build ────────────────────────────────────────────────────────────────────

/// Compose an `App` for `plan`'s [`BootProfile`].
///
/// The shape is the same for all three: the shared [`core_plugins`], then the
/// renderer axis ([`render_stack`] for BrowserHost, else [`render_surrogate`]),
/// then [`ingest_world`]. The simulation, lobby and world plugins the two existing
/// boot paths add around this seam are the adopting adapters' to attach (#1218/
/// #1219) — this function owns only what actually differs per profile plus the
/// world-ingestion order.
pub fn build(plan: BootPlan) -> Result<App, BootError> {
    let mut app = App::new();

    core_plugins(
        &mut app,
        plan.profile,
        &plan.log_filter,
        plan.single_threaded,
    );

    if plan.profile.has_render_stack() {
        render_stack(&mut app);
    } else {
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
/// The wgpu-backed render plugins are instantiated **only on the browser target**.
/// A native build (the target the parity test runs on) cannot stand up the render
/// stack at all — Bevy's `RenderPlugin` requests a GPU adapter and panics with none,
/// which is the very reason the [`BrowserAutomation`](BootProfile::BrowserAutomation)
/// inventory exists. So on native this registers the renderer's *contract* with the
/// simulation explicitly (the same floor [`render_surrogate`] provides), which is
/// what lets `build(BrowserHost)` compose on native and the parity test assert the
/// shared four-asset/three-message floor.
///
/// The Bevy render stack itself (`DefaultPlugins`' `RenderPlugin`, which
/// `init_asset`s the four types on the browser) is the wasm adapter's to attach in
/// #1219; `add_message`/`init_asset` are idempotent, so the explicit registration
/// here is a safe belt-and-braces even once that lands.
fn render_stack(app: &mut App) {
    app.insert_resource(RenderStackApplied);

    #[cfg(target_arch = "wasm32")]
    {
        // The real renderer: RendererPlugin plus the viewscreen border/lobby
        // pushes. ViewscreenBorderPlugin already registers HudStateChanged,
        // LobbyStateChanged and `push_lobby_state`, so only the asset types and
        // AiChatterEvent are added here (the latter also rides in on ShipPlugin in
        // the full app; add_message is idempotent).
        register_render_assets(app);
        app.add_message::<AiChatterEvent>();
        app.add_plugins(crate::server::renderer::RendererPlugin)
            .add_plugins(crate::server::viewscreen_border::ViewscreenBorderPlugin);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
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
        .add_message::<AiChatterEvent>()
        // The one system the missing ViewscreenBorderPlugin still owes the HTML
        // lobby overlay. Its parameters are all `Option<Res<_>>` bar the `GamePhase`
        // state, so a boot that never inits that state simply never runs it.
        .add_systems(Update, crate::server::viewscreen_border::push_lobby_state);
}

// ── ingest_world ─────────────────────────────────────────────────────────────

/// The sole caller of [`crate::world::load::load`], and the sole owner of the
/// content-ledger and Rhai-seed order a boot must run in.
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
/// 4. The composition + script gate, applied per the profile's
///    [abort-vs-block](BootProfile::broken_world_aborts) policy.
/// 5. Apply the ledger records the load gathered, eager-record the declared entity
///    templates (native only — the browser's JS preload is its equivalent), and
///    [`freeze`](crate::content_ledger::freeze) — so the content digest a save is
///    checked against does not drift as the world streams in.
/// 6. Hand the parsed world and once-compiled scripts to the `App` as resources for
///    `WorldPlugin`'s `Startup` to consume; a broken-but-not-aborted (browser) world
///    carries its findings through so the downstream gate blocks activation.
fn ingest_world(app: &mut App, plan: &BootPlan) -> Result<(), BootError> {
    crate::world::script::init_hashing_seed();
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
    // own findings can carry errors; a broken world aborts the build only where the
    // profile says so.
    let mut invalid: Vec<String> = Vec::new();
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
        crate::content_ledger::eager_record_world_entities(&loaded.config);
        for child in &loaded.children {
            crate::content_ledger::eager_record_world_entities(&child.config);
        }
    }
    crate::content_ledger::freeze();

    app.insert_resource(loaded.config);
    app.insert_resource(crate::world::server::PreCompiledScripts(loaded.scripts));
    Ok(())
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
