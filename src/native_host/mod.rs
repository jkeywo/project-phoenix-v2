//! The native windowed authoritative host (issue #1121).
//!
//! One Windows executable that loads ordinary Phoenix content and runs the
//! *same* authoritative simulation and plugin graph the browser host runs, with
//! the shared viewscreen drawn by native Bevy/wgpu through winit instead of
//! onto a `<canvas>`.
//!
//! # Where the pieces live
//!
//! * [`app`] — the builder. Thin on purpose: the plugin core, the render stack
//!   and the whole world-ingestion order come from [`crate::boot`] under
//!   [`BootProfile::NativeHost`](crate::boot::BootProfile::NativeHost), and the
//!   simulation is [`crate::server_app::add_simulation_plugins_with`] in its
//!   unchanged registration order.
//! * [`world_load`] — issue #1326's runtime half: a host given no world boots
//!   into an empty lobby and takes one from an arbitrated `SelectScenario` +
//!   `SelectPlayerShip` pair, through the same [`crate::boot::ingest_world`] a
//!   `--world` boot runs.
//! * [`transport`] — the seam a transport plugs into: two systems over the
//!   three `lobby::server` messages, plus the reserved-token gate the browser
//!   applies at its own ingress.
//! * [`panes`] — issue #1122's local Stations: one isolated Ultralight view per
//!   pane, each an ordinary logical client entering through that same seam with
//!   its own minted session token. Everything about *what a pane may say and
//!   hear* compiles and is tested with the SDK feature off; only the drawing and
//!   the input translation are behind `--features ultralight`.
//! * [`relay_transport`] — what else plugs into it (issue #1113): the rendezvous
//!   service's WebSocket game relay, which is how a browser client joins a
//!   native host. #1121 deferred that acceptance criterion to a transport that
//!   did not exist yet; this is it.
//! * `phoenix-host` (`src/bin/phoenix_host.rs`) — the process. Issue #1121
//!   **evolves** that binary rather than adding a parallel one, so PRD #855's
//!   bundle serving, catalogue restriction and startup version pin are shared
//!   rather than duplicated: `--world` (or, since #1326, `--lobby`) turns the
//!   delivery host into an authoritative one, and every existing flag keeps its
//!   exact meaning.
//!
//! # Content roots
//!
//! A native process resolves content through two independent mechanisms, and
//! they must be made to agree — see [`pin_content_root`].

pub mod app;
/// The winit/Bevy adapter (issue #1123) that reads real monitors and opens one
/// borderless-fullscreen surface per configured monitor from a resolved
/// [`bridge_profile`]. Provable only under the ignored integration test.
pub mod bridge_display;
/// The bridge **layout law** (issue #1327) — pure, Bevy-free. One viewscreen, no
/// console over it, at most two consoles per screen: the typed assign/move/
/// unassign transitions every path shares (lobby buttons, saved per-ship-class
/// layouts, CLI profiles), their typed refusals, the per-monitor occupancy and
/// per-station eligibility a greyed button row is a `map` over, and the
/// conversion to and from a [`bridge_profile::ValidatedProfile`]. Where
/// [`bridge_profile`] judges a *file*, this judges a *transition*.
pub mod bridge_layout;
/// The bridge-media profile model (issue #1126) — pure, Bevy-free. Media-device
/// kinds and stable identities, the per-surface camera/microphone/output
/// assignment, the parse/validate failure taxonomy (wrong-kind, duplicate,
/// unconsented share), the deterministic default and the missing/denied-device
/// resolution all live here and are tested by the ordinary `cargo test` CI runs.
/// Its assignments ride in the same [`bridge_profile::BridgeProfile`] TOML. The
/// real OS enumeration/preview/test backend is not yet in the tree — see
/// [`bridge_media::enumerate_note`].
pub mod bridge_media;
/// The bridge-display profile model (issue #1123) — pure, Bevy-free. Stable
/// monitor identities, the one/two-pane density rule, pane geometry, the TOML
/// round-trip and the missing-display resolution all live here and are tested by
/// the ordinary `cargo test` CI runs. The winit adapter that opens real
/// borderless-fullscreen windows from a resolved profile is [`bridge_display`].
pub mod bridge_profile;
/// The host as its own rendezvous (issue #1353): the in-process, single-game
/// subset of the rendezvous service, so a phone on the LAN joins over the
/// delivery port with no external service anywhere. Behind the `host` feature
/// with [`relay_socket`], because it is the other thing here that needs
/// `tungstenite`; the protocol logic it drives is [`relay_transport`]'s,
/// unchanged.
#[cfg(feature = "host")]
pub mod direct_join;
/// The native host's own lobby surface (issue #1325) — the crew lobby the
/// browser host shows, composited onto the viewscreen window from an embedded
/// web view over the SAME `gui/host-lobby-view.js` + `gui/host-lobby-render.js`
/// pair `server.html` renders with. The document assembly, the bridge and the
/// reveal state machine are pure and CI-tested; only the compositing lives
/// behind `--features ultralight`, in [`panes::ultralight`].
pub mod host_lobby;
/// The pure input-routing model (issue #1124) — the coordinate transforms, the
/// pane-boundary hit test, the keyboard-focus order and the touch
/// contact-capture map. Bevy-free and CI-tested; the winit/Ultralight adapter
/// that feeds it real events is [`panes::ultralight`] behind `--features
/// ultralight`.
pub mod input_routing;
/// The authored join-code table, read by a host that ISSUES codes
/// (issue #1353). Pure and CI-tested: minting, canonicalisation and the typed
/// lookup a direct-accept host answers a joiner's code with, all read out of
/// `assets/join/join-codes.toml` rather than written down a second time.
pub mod join_codes;
/// The **saved bridge layouts** (issue #1334) — pure, Bevy-free, and its
/// location injected. One TOML file per ship class under the operator's own
/// settings directory (`%APPDATA%\ProjectPhoenix\bridge-layouts` on Windows),
/// holding the arrangement they built in the lobby the last time they flew that
/// hull: the class key, the atomic temp-then-rename write, and the
/// load-parse-revalidate that hands [`bridge_layout`] a profile as untrusted as
/// a hand-authored one. Its Bevy adapter — the pre-apply at the hull-known
/// moment and the write on every accepted lobby change — is
/// [`layout_store_systems`].
pub mod layout_store;
/// The Bevy adapter for [`layout_store`] (issue #1334): the two systems that
/// pre-apply a class's remembered bridge once the hull is known and file every
/// accepted lobby change back to it, both gated off for a run an operator gave
/// an explicit `--profile`.
pub mod layout_store_systems;
pub mod panes;
/// The real WebSocket behind [`relay_transport`]. Behind the `host` feature
/// because it is the only thing here that needs `tungstenite`; the protocol it
/// carries, and that protocol's tests, stay on the default feature set.
#[cfg(feature = "host")]
pub mod relay_socket;
pub mod relay_transport;
/// The setup/layout accessibility model (issue #1128) — pure, Bevy-free. The
/// reflow-headroom check that keeps one- and two-pane layouts operable at the
/// supported text-scale extremes, the keyboard-focus order across monitors and
/// split panes, and the setup-action reachability invariant all live here and are
/// CI-tested.
pub mod setup_accessibility;
pub mod transport;
/// Loading a world into a **running** host (issue #1326): the native half of the
/// pre-scenario flow. Boots into an empty `GamePhase::Lobby` holding the merged
/// scenario catalogue, arbitrates `SelectScenario` + `SelectPlayerShip` through
/// [`crate::lobby::scenario_arbiter`], and then runs the *same*
/// [`crate::boot::ingest_world`] a `--world` boot runs — on the `World` of the
/// app that is already drawing the lobby.
pub mod world_load;

pub use app::{
    build_native_host_app, curated_hulls_for_world, preload_content_templates, run,
    NativeHostConfig, NativeHostError, WINDOW_TITLE,
};

/// Make Bevy's asset root and the process working directory name the same
/// content tree.
///
/// A native host reads content two ways, and they resolve differently:
///
/// 1. **Bevy's `AssetServer`**, rooted at `AssetPlugin.file_path` (default
///    `"assets"`), which Bevy resolves against `BEVY_ASSET_ROOT` if set, else
///    `CARGO_MANIFEST_DIR` if cargo set it, else **the executable's own
///    directory**. For a double-clicked `phoenix-host.exe` that last one is the
///    release directory, not the content tree. This governs GLB models,
///    shaders, textures and audio.
/// 2. **Raw `std::fs`** against the **process working directory**, for the full
///    authored path a world TOML writes. This governs the world TOML itself
///    (`world::load::FsReader`), entity templates (`FsTemplateLoader`), Rhai
///    scripts (`FsScriptFallback`) and model-rig sidecars
///    (`entities::glb_visual`).
///
/// Pin one and not the other and the host silently half-loads: a ship with no
/// mesh, or a mesh with no ship, and no single clear error. So this sets the
/// working directory to `content_dir` **and** exports `BEVY_ASSET_ROOT` as its
/// absolute path, from the one `--content-dir` value the operator gave.
/// `capture-billboard` sets the same variable for the same reason; `perf::mesh`
/// works around the same split by making its asset root absolute.
///
/// An existing `BEVY_ASSET_ROOT` is left alone, so an operator can still point
/// the asset root somewhere else deliberately.
pub fn pin_content_root(content_dir: &str) -> Result<std::path::PathBuf, String> {
    let root = std::path::Path::new(content_dir);
    let absolute = std::fs::canonicalize(root)
        .map_err(|e| format!("cannot resolve content directory {content_dir:?}: {e}"))?;
    std::env::set_current_dir(&absolute)
        .map_err(|e| format!("cannot enter content directory {content_dir:?}: {e}"))?;
    if std::env::var_os("BEVY_ASSET_ROOT").is_none() {
        std::env::set_var("BEVY_ASSET_ROOT", &absolute);
    }
    Ok(absolute)
}
