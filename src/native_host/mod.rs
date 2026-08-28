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
//!   rather than duplicated: `--world` turns the delivery host into an
//!   authoritative one, and every existing flag keeps its exact meaning.
//!
//! # Content roots
//!
//! A native process resolves content through two independent mechanisms, and
//! they must be made to agree — see [`pin_content_root`].

pub mod app;
pub mod panes;
/// The real WebSocket behind [`relay_transport`]. Behind the `host` feature
/// because it is the only thing here that needs `tungstenite`; the protocol it
/// carries, and that protocol's tests, stay on the default feature set.
#[cfg(feature = "host")]
pub mod relay_socket;
pub mod relay_transport;
pub mod transport;

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
