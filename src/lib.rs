// Structural lints we allow at the crate level because the
// affected functions consume a fixed set of Bevy system parameters
// common in game-development patterns.
#![forbid(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

// Declared early: the `plog!` family is `#[macro_export]`ed, and the helper
// macros they expand to must be defined before any module that uses them.
pub mod logging;

// Headless runner. Native only: it drives the app with a manual fixed-timestep
// loop, which has no meaning under requestAnimationFrame.
#[cfg(all(feature = "headless", not(target_arch = "wasm32")))]
pub mod headless;

// Performance measurement (issue #868). Not behind the headless feature: the
// asset inventory needs no simulation, and the browser collector runs in the
// shipped wasm build.
pub mod perf;

// The boot seam (issue #1217, Track 2 step B5). One place that composes an
// `App` for each of the three inventories — Headless, BrowserHost,
// BrowserAutomation — that `headless::app::build_headless_app` and
// `server::bridge::wasm_init` today spell out by hand. Additive: nothing calls
// `boot::build` yet (the headless and wasm adapters adopt it in #1218/#1219).
// Not behind the headless feature — two of its three profiles are browser
// (wasm) inventories.
pub mod boot;

pub mod ai;
pub mod asteroids;
/// Re-exported from the `phoenix-math` workspace crate (issue #1184), so
/// `crate::audio_config::…` still resolves after the extraction.
pub use phoenix_math::audio_config;
/// The authoritative-state declaration registry (issue #1220, Track 3 step C8):
/// `App::declare_state::<T>(class, pasm)` for an owning plugin to record, at the
/// site that owns a type, how it relates to the #894 digest boundary. Adds only
/// the mechanism — no plugin declares yet, and nothing in the digest/snapshot
/// reads the census, so the registry is inert to `sim_digest::world_digest`.
pub mod authoritative;
/// Fixed-capacity history window (issue #788). Pure, Bevy-free, domain-neutral.
/// Re-exported from the `phoenix-math` workspace crate (issue #1184).
pub use phoenix_math::bounded_history;
/// Compile-time build flags readable at runtime (issue #939) — currently just
/// `PHOENIX_DEMO_BUILD`, which gates the host settings menu's Debug/Cheat tab.
pub mod build_flags;
/// Campaign continuity (issue #867): the pure fold from a finished mission's
/// snapshot to the facts the next mission may open on. Reads state, registers
/// none.
pub mod campaign;
/// Civilian traffic (issue #1028): the pure route / order / compliance
/// vocabulary and the adapter that installs it as an ordinary NPC doctrine
/// directive.
pub mod civilian;
pub mod command_admission;
pub mod comms;
/// Composite-key deterministic value derivation (issue #788). Pure, Bevy-free,
/// domain-neutral. Re-exported from the `phoenix-math` workspace crate (issue #1184).
pub use phoenix_math::composite_rng;
pub mod console_ai;
pub mod console_bridge;
/// The loaded-content ledger (issue #935): every authored file the world/entity
/// loader actually reads, folded into `snapshot::content_digest` so an edit to
/// a hull, fragment, or sidecar moves a save's content version exactly as
/// reliably as an edit to the scenario TOML does. Compiles on both targets —
/// native and wasm converge on the same recording shape so the digest is
/// target-independent for identical bytes.
pub mod content_ledger;
pub mod core;
/// The seeded cross-target determinism probe (issue #904): one minimal sim
/// world both a native test and the browser drive, under deliberately
/// different frame pacing, folding the canonical digest at shared ticks.
pub mod cross_target_probe;
/// Delivery (PRD #855): how a host — browser tab or native `phoenix-host`
/// process — publishes its client bundle, its content manifest, its scenario
/// catalogue and its version pin. Compiles on both targets on purpose: the
/// catalogue field list and the pin are shared code, and only the socket loop
/// (`delivery::serve`) is native-only.
pub mod delivery;
/// Helm docking (issue #1159): the pure marker-mating module and its Bevy
/// adapter. A hull with a `[dock]` table and dock markers in its rig sidecar can
/// dock with, or be docked by, another hull carrying dock markers; the docked
/// relationship is a folded, published state the umbilical (#1160) gates on.
pub mod dock;
/// Dossiers (issue #1030): the pure per-subject projection of what a crew
/// knows, and the adapter that publishes it on the local ship's intelligence
/// channel. Holds no state of its own — every fact is a fold of something
/// another subsystem already owns.
pub mod dossier;
/// `EffectQueue<T>`: per-owner transient per-tick effect queues (issue #1223),
/// extracted from the `WorldContentRuntime` god-resource.
pub mod effect_queue;
pub mod entities;
/// Infrastructure condition + capacity on authored world furniture (issue
/// #1025): the pure degradation/repair track and its Bevy adapter.
pub mod infrastructure;
pub mod lobby;
pub mod modifiers;
pub mod objectives;
pub mod radar;
pub mod radar_config;
/// The viewscreen's reference grid: the authored `[reference_grid]` table, its
/// validation, and the world-lattice maths `assets/shaders/reference_grid.wgsl`
/// mirrors. Bevy-free and unit-tested here; the render half is
/// `server::reference_grid` and exists only under `SimPluginOptions::render`.
pub mod reference_grid;
pub mod regions;
/// The science scan (issue #1032): the pure derivation that turns a structure's
/// live condition track into a fidelity-banded sensor reading, and its Bevy
/// adapter. There is no authored scan text anywhere behind it — see
/// `pasm/spec/design/simulation-differentiation.yaml`.
pub mod science;
pub mod server_app;
/// The render half lifted out of `server_app` (issue #1195): the Bevy mesh
/// cache, material factory, LOD swapper, and light spawners. Registered from
/// `server_app` under `SimPluginOptions::render`; presentation-only, outside
/// the authoritative digest.
pub mod server_app_render;
pub mod ship;
pub mod ship_plugin;
/// The canonical authoritative-state digest (issue #901). At the crate root
/// rather than under `headless` since issue #904: a digest that only compiles
/// on native cannot make a native↔wasm claim. `headless::digest` aliases it.
pub mod sim_digest;
pub mod sim_rng;
pub mod sim_sets;
pub mod sim_tick;
/// Shared pure-Rust libm wrappers — the only sanctioned transcendental float
/// math in simulation code (issue #908; enforced via clippy.toml).
/// Re-exported from the `phoenix-math` workspace crate (issue #1184).
pub use phoenix_math::simmath;
/// Cross-target vector battery proving native and wasm agree, bit for bit,
/// on every `simmath` function (issue #909).
pub mod simmath_vectors;
/// The authoritative world snapshot (issue #862): phoenix's save *payload*,
/// stored inside `vellum-save`'s envelope. Compiles on both targets — a browser
/// host is the thing that saves.
pub mod snapshot;
/// Host-derived per-Station importance projection (issue #1101): a pure,
/// Bevy-free attention stream (one-off unread events vs continuing critical
/// conditions), held strictly apart from health.
pub mod station_importance;
/// The tractor beam (issue #1156), linchpin of PRD #1143's coupling family: the
/// pure, Bevy-free coupling-position module and refusal vocabulary, and its Bevy
/// adapter — the engineering-owned `[[system]]` that couples the ship to
/// Tactical's current lock and holds the derelict on the operator's rig. The
/// umbilical, dock and external repair-dispatch slices copy this shape.
pub mod tractor;
/// The transfer umbilical (issue #1160), third slice of PRD #1143's coupling
/// family: the pure, Bevy-free flow-arithmetic module and refusal vocabulary, and
/// its Bevy adapter — the engineering-owned `[[system]]` that moves an authored
/// capacity per second between two DOCKED hulls' capacity ledgers, gating on the
/// dock slice (#1159) so a flow runs only while docked.
pub mod umbilical;
pub mod weapons;
pub mod world;
/// Deterministic tick-scoped world-id minting (issue #907) — the single
/// chokepoint every simulation entity, message and projectile id comes from.
pub mod world_id;

// ── Console module ─────────────────────────────────────────────────────────

pub mod console;

// Server-only grouped module (bridge, renderer, viewscreen_border, debug_overlay).
#[cfg(feature = "server")]
pub mod server;

pub mod debug_overlay;

/// Structured debug observability (PRD #1144): one read-only projection pipeline
/// off authoritative state, carried as `serde` JSON to the dock, the headless
/// report, and (later) the GM Live Inspector. The first slice (issue #1145) is
/// the always-on station-activity tracker + its dock chart. The schema and
/// transport conventions the later slices reuse live in `crate::debug::payload`.
pub mod debug;

/// Shared 3D render setup (skybox, camera optics, ambient fill) — used by both
/// the game renderer and the standalone model viewer.
pub mod render_setup;

/// Shared native headless-render core (offscreen RGBA target + render-graph
/// readback + framing maths) for the `capture-billboard` and `tune-lods` tools.
/// Native + `capture` only: it pulls the render-graph readback plumbing and the
/// `crossbeam-channel` bridge, neither of which the shipped wasm build wants.
#[cfg(all(feature = "capture", not(target_arch = "wasm32")))]
pub mod render_capture;

/// Pure math behind the automatic LOD switch-range tuner (`tune-lods`): the
/// alpha-aware image difference and the knee-of-curve rule. Re-exported from the
/// `phoenix-math` workspace crate (issue #1184).
pub use phoenix_math::lod_tune;

/// Standalone model/shader viewer (`viewer.html`), a dev tool built as its own
/// Trunk target. Not part of the game binary.
#[cfg(feature = "viewer")]
pub mod viewer;

// Generic GUI widget library — needed by the server viewscreen radar
// (ServerViewscreenRadarPlugin).
#[cfg(feature = "server")]
pub mod gui;
