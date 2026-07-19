//! Building star and planet visuals from their config sections.
//!
//! These are the only entities using hand-written WGSL
//! (`assets/shaders/star_surface.wgsl`, `star_halo.wgsl`,
//! `planet_surface.wgsl`, `planet_clouds.wgsl`), so keeping the construction
//! in one place means the standalone model viewer exercises exactly the
//! material setup the game does.

use bevy::prelude::*;

use crate::entity_config::{PlanetConfig, StarConfig};
use crate::entity_planet::{PlanetCloudMaterial, PlanetSurfaceMaterial};
use crate::entity_star::{StarHalo, StarHaloMaterial, StarSurfaceMaterial};

/// Attach a star's surface sphere to `entity` plus a billboarded halo child.
pub fn insert_star_visual(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    surface_materials: &mut Assets<StarSurfaceMaterial>,
    halo_materials: &mut Assets<StarHaloMaterial>,
    entity: Entity,
    cfg: &StarConfig,
) {
    let surface_mesh = meshes.add(crate::entity_star::uv_sphere_mesh(
        cfg.radius,
        cfg.longitude_segments,
        cfg.latitude_segments,
    ));
    let surface_mat = surface_materials.add(crate::entity_star::surface_material_from_config(cfg));
    let halo_radius = cfg.radius * cfg.halo_radius_multiplier.max(1.0);
    let halo_mesh = meshes.add(crate::entity_star::halo_quad_mesh(halo_radius));
    let halo_mat = halo_materials.add(crate::entity_star::halo_material_from_config(cfg));
    let mut ec = commands.entity(entity);
    ec.insert((Mesh3d(surface_mesh), MeshMaterial3d(surface_mat)));
    ec.with_children(|parent| {
        parent.spawn((
            Mesh3d(halo_mesh),
            MeshMaterial3d(halo_mat),
            Transform::default(),
            StarHalo {
                radius: halo_radius,
            },
        ));
    });
}

/// Attach a planet's textured surface sphere to `entity`, plus an
/// alpha-blended cloud shell child when the config declares `[planet.clouds]`.
pub fn insert_planet_visual(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    surface_materials: &mut Assets<PlanetSurfaceMaterial>,
    cloud_materials: &mut Assets<PlanetCloudMaterial>,
    asset_server: &AssetServer,
    entity: Entity,
    cfg: &PlanetConfig,
) {
    let surface_mesh = meshes.add(crate::entity_star::uv_sphere_mesh(
        cfg.radius,
        cfg.longitude_segments,
        cfg.latitude_segments,
    ));
    let surface_mat = surface_materials.add(crate::entity_planet::surface_material_from_config(
        cfg,
        asset_server,
    ));
    let mut ec = commands.entity(entity);
    ec.insert((Mesh3d(surface_mesh), MeshMaterial3d(surface_mat)));
    if let Some(cloud_mat) = crate::entity_planet::cloud_material_from_config(cfg, asset_server) {
        let shell_scale = cfg
            .clouds
            .as_ref()
            .map(|c| c.scale.max(1.001))
            .unwrap_or(1.03);
        let cloud_mesh = meshes.add(crate::entity_star::uv_sphere_mesh(
            cfg.radius * shell_scale,
            cfg.longitude_segments,
            cfg.latitude_segments,
        ));
        let cloud_mat = cloud_materials.add(cloud_mat);
        ec.with_children(|parent| {
            parent.spawn((
                Mesh3d(cloud_mesh),
                MeshMaterial3d(cloud_mat),
                Transform::default(),
            ));
        });
    }
}
