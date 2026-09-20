//! Explicit offline native Workshop. Shares the boot core and pane renderer,
//! but owns no live world, crew transport, join code or Game Master identity.
pub mod billboard_capture;
pub mod bridge;
pub mod document;
pub mod keyboard;
pub mod lod_generation;
pub mod preview;
pub mod test_clock;
pub mod test_process;

use bevy::prelude::*;

#[derive(Resource, Clone)]
pub struct WorkshopSurface {
    pub bridge: bridge::WorkshopBridge,
    pub url: String,
}

pub fn build_shell(
    surface: crate::boot::NativeRenderSurface,
) -> Result<App, crate::boot::BootError> {
    let mut app = crate::boot::build(crate::boot::BootPlan {
        profile: crate::boot::BootProfile::NativeWorkshop,
        world_ingest: crate::boot::WorldIngest::Deferred,
        log_filter: "warn".into(),
        world_path: String::new(),
        reader: Box::new(crate::world::load::MemoryReader::new(std::iter::empty::<(
            String,
            String,
        )>())),
        script_resolver: Box::new(crate::entities::config_cache::production_script_resolver()),
        single_threaded: false,
        raw_transform: None,
        native_surface: surface,
    })?;
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<WorkshopSurface>(StateClass::Timer, "gm-milestone-integrated-workshop");
    app.add_plugins(crate::native_host::panes::upload::PaneUploadPlugin);
    if surface == crate::boot::NativeRenderSurface::Window {
        app.add_systems(Startup, |mut commands: Commands| {
            commands.spawn(Camera2d);
        });
        #[cfg(feature = "ultralight")]
        app.add_plugins(crate::native_host::panes::ultralight::PaneDisplayPlugin);
    }
    Ok(app)
}

/// One executable, explicit offline mode. Delivery is loopback-only static
/// content; it never exposes a request route to the selected-root provider.
#[cfg(feature = "host")]
#[allow(clippy::disallowed_methods)] // Private document nonce, never simulation identity.
pub fn run(args: &crate::delivery::args::HostArgs) -> Result<(), String> {
    #[cfg(not(feature = "ultralight"))]
    {
        let _ = args;
        Err("Native Workshop needs a build with --features host,ultralight".into())
    }
    #[cfg(feature = "ultralight")]
    {
        use crate::delivery::{
            args::ClientSource,
            serve::{HostServer, ShutdownSignal},
        };
        use crate::workshop::provider::{NativeWorkshopProvider, WorkspaceKind};
        let selected = args.workshop.as_ref().ok_or("No Workshop root selected")?;
        let ClientSource::Bundled { dir } = &args.client else {
            return Err("Workshop needs a built bundle".into());
        };
        let root = std::fs::canonicalize(&selected.root).map_err(|e| e.to_string())?;
        let bundle = std::fs::canonicalize(dir).map_err(|e| e.to_string())?;
        let content = std::fs::canonicalize(&args.content_dir).map_err(|e| e.to_string())?;
        let html = std::fs::read_to_string(bundle.join("workshop.html"))
            .map_err(|e| format!("Cannot read built Workshop page: {e}"))?;
        let html = document::build_document(&html)?;
        let private = directories::BaseDirs::new()
            .ok_or("Workshop private storage unavailable")?
            .data_local_dir()
            .join("ProjectPhoenix/workshop");
        let dependencies = if selected.project {
            Default::default()
        } else {
            read_dependencies(&content)?
        };
        let provider = NativeWorkshopProvider::open(
            if selected.project {
                WorkspaceKind::Project
            } else {
                WorkspaceKind::Mod
            },
            root,
            private,
            dependencies,
        )?;
        let mut delivery = args.clone();
        delivery.addr = "127.0.0.1:0".into();
        delivery.client = ClientSource::Bundled {
            dir: bundle.to_string_lossy().into_owned(),
        };
        delivery.content_dir = content.to_string_lossy().into_owned();
        let server = HostServer::bind(&delivery)?;
        let path = document::document_path(&uuid::Uuid::new_v4().to_string());
        server.hosted_documents().publish(path.clone(), html);
        let worker = bridge::WorkshopWorker::spawn_hosted(
            provider,
            server.hosted_documents(),
            format!("http://{}", server.local_addr()),
        )?;
        let suffix = selected
            .open
            .as_deref()
            .map(|query| format!("#{query}"))
            .unwrap_or_default();
        let surface = WorkshopSurface {
            bridge: worker.bridge(),
            url: format!("http://{}{}{}", server.local_addr(), path, suffix),
        };
        let mut app =
            build_shell(crate::boot::NativeRenderSurface::Window).map_err(|e| e.to_string())?;
        app.insert_resource(surface);
        let shutdown = ShutdownSignal::new();
        let stop = shutdown.clone();
        let delivery = std::thread::Builder::new()
            .name("phoenix-workshop-delivery".into())
            .spawn(move || server.serve_until(stop, |_| {}))
            .map_err(|e| e.to_string())?;
        app.run();
        shutdown.stop();
        let _ = delivery.join();
        drop(worker);
        Ok(())
    }
}

/// Snapshot the explicitly selected read-only base using current source bytes,
/// not a stale bundle's source cache and not a process-global runtime overlay.
#[cfg(all(feature = "host", feature = "ultralight"))]
fn read_dependencies(
    root: &std::path::Path,
) -> Result<crate::workshop::WorkshopDependencies, String> {
    fn walk(
        root: &std::path::Path,
        directory: &std::path::Path,
        files: &mut std::collections::BTreeMap<String, String>,
        binary: &mut std::collections::BTreeMap<String, Vec<u8>>,
        total: &mut usize,
    ) -> Result<(), String> {
        for entry in std::fs::read_dir(directory).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err("Read-only Workshop dependencies cannot contain linked paths".into());
            }
            let path = entry.path();
            if kind.is_dir() {
                walk(root, &path, files, binary, total)?;
            } else if kind.is_file() {
                let name = path
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_string_lossy()
                    .replace('\\', "/");
                let text_source = name.ends_with(".toml") || name.ends_with(".rhai");
                if !text_source
                    && !crate::workshop::provider::assets::binary_path(&name)
                    && !crate::workshop::provider::test_snapshot::runtime_support_path(&name)
                {
                    continue;
                }
                use std::io::Read;
                let remaining = (512 * 1024 * 1024usize).saturating_sub(*total);
                let mut bytes = Vec::new();
                std::fs::File::open(&path)
                    .map_err(|e| e.to_string())?
                    .take(remaining as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                *total = total.saturating_add(bytes.len());
                if *total > 512 * 1024 * 1024 || files.len() + binary.len() >= 16384 {
                    return Err("Workshop dependencies are too large".into());
                }
                if text_source {
                    files.insert(name, String::from_utf8(bytes).map_err(|e| e.to_string())?);
                } else {
                    binary.insert(name, bytes);
                }
            }
        }
        Ok(())
    }
    let mut dependencies = crate::workshop::WorkshopDependencies::default();
    let assets = root.join("assets");
    if std::fs::symlink_metadata(&assets)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Read-only Workshop dependencies cannot contain linked paths".into());
    }
    let mut total = 0;
    walk(
        root,
        &assets,
        &mut dependencies.base_files,
        &mut dependencies.base_assets,
        &mut total,
    )?;
    Ok(dependencies)
}

#[cfg(test)]
mod tests {
    #[test]
    fn offline_workshop_shell_installs_the_actual_pane_upload_consumer() {
        let mut app = super::build_shell(crate::boot::NativeRenderSurface::Contract).unwrap();
        assert!(app.is_plugin_added::<crate::native_host::panes::upload::PaneUploadPlugin>());
        assert!(app
            .world()
            .contains_resource::<crate::native_host::panes::upload::PanePendingUploads>());
        app.update();
    }
}
