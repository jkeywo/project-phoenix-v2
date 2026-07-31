use bevy::{
    asset::RenderAssetUsages,
    mesh::Indices,
    prelude::*,
    reflect::TypePath,
    render::{render_resource::AsBindGroup, render_resource::PrimitiveTopology},
    shader::ShaderRef,
};

use crate::entity_config::StarConfig;

const STAR_SURFACE_SHADER: &str = "shaders/star_surface.wgsl";
const STAR_HALO_SHADER: &str = "shaders/star_halo.wgsl";

#[derive(Component, Clone, Copy, Debug)]
pub struct StarHalo {
    pub radius: f32,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct StarSurfaceMaterial {
    #[uniform(0)]
    pub surface_r: f32,
    #[uniform(0)]
    pub surface_g: f32,
    #[uniform(0)]
    pub surface_b: f32,
    #[uniform(0)]
    pub _pad0: f32,
    #[uniform(0)]
    pub hot_r: f32,
    #[uniform(0)]
    pub hot_g: f32,
    #[uniform(0)]
    pub hot_b: f32,
    #[uniform(0)]
    pub time: f32,
    #[uniform(0)]
    pub cell_r: f32,
    #[uniform(0)]
    pub cell_g: f32,
    #[uniform(0)]
    pub cell_b: f32,
    #[uniform(0)]
    pub animation_speed: f32,
}

impl Material for StarSurfaceMaterial {
    fn fragment_shader() -> ShaderRef {
        STAR_SURFACE_SHADER.into()
    }
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct StarHaloMaterial {
    #[uniform(0)]
    pub color_r: f32,
    #[uniform(0)]
    pub color_g: f32,
    #[uniform(0)]
    pub color_b: f32,
    #[uniform(0)]
    pub alpha: f32,
    #[uniform(0)]
    pub time: f32,
    #[uniform(0)]
    pub animation_speed: f32,
    #[uniform(0)]
    pub _pad0: f32,
    #[uniform(0)]
    pub _pad1: f32,
}

impl Material for StarHaloMaterial {
    fn fragment_shader() -> ShaderRef {
        STAR_HALO_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

pub struct StarRenderPlugin;

impl Plugin for StarRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<StarSurfaceMaterial>::default())
            .add_plugins(MaterialPlugin::<StarHaloMaterial>::default())
            .add_systems(Update, (tick_star_materials, billboard_star_halos));
    }
}

pub fn surface_material_from_config(config: &StarConfig) -> StarSurfaceMaterial {
    StarSurfaceMaterial {
        surface_r: config.surface_colour[0],
        surface_g: config.surface_colour[1],
        surface_b: config.surface_colour[2],
        _pad0: 0.0,
        hot_r: config.hot_colour[0],
        hot_g: config.hot_colour[1],
        hot_b: config.hot_colour[2],
        time: 0.0,
        cell_r: config.cell_colour[0],
        cell_g: config.cell_colour[1],
        cell_b: config.cell_colour[2],
        animation_speed: config.animation_speed,
    }
}

pub fn halo_material_from_config(config: &StarConfig) -> StarHaloMaterial {
    StarHaloMaterial {
        color_r: config.halo_colour[0],
        color_g: config.halo_colour[1],
        color_b: config.halo_colour[2],
        alpha: 0.55,
        time: 0.0,
        animation_speed: config.animation_speed,
        _pad0: 0.0,
        _pad1: 0.0,
    }
}

// Presentation-only render mesh generation: vertex positions never feed
// simulation state, so std transcendentals are fine (issue #908, simmath.rs).
#[allow(clippy::disallowed_methods)]
pub fn uv_sphere_mesh(radius: f32, longitude_segments: u32, latitude_segments: u32) -> Mesh {
    let radius = radius.max(0.1);
    let longitudes = longitude_segments.max(8);
    let latitudes = latitude_segments.max(4);

    let mut positions = Vec::with_capacity(((longitudes + 1) * (latitudes + 1)) as usize);
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut uvs = Vec::with_capacity(positions.capacity());

    for lat in 0..=latitudes {
        let v = lat as f32 / latitudes as f32;
        let theta = v * std::f32::consts::PI;
        let sin_theta = theta.sin();
        let cos_theta = theta.cos();

        for lon in 0..=longitudes {
            let u = lon as f32 / longitudes as f32;
            let phi = u * std::f32::consts::TAU;
            let normal = Vec3::new(phi.cos() * sin_theta, cos_theta, phi.sin() * sin_theta);

            positions.push((normal * radius).to_array());
            normals.push(normal.to_array());
            uvs.push([u, v]);
        }
    }

    let stride = longitudes + 1;
    let mut indices = Vec::with_capacity((longitudes * latitudes * 6) as usize);
    for lat in 0..latitudes {
        for lon in 0..longitudes {
            let a = lat * stride + lon;
            let b = a + stride;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

pub fn halo_quad_mesh(radius: f32) -> Mesh {
    let r = radius.max(0.1);
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-r, -r, 0.0], [r, -r, 0.0], [r, r, 0.0], [-r, r, 0.0]],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
}

fn tick_star_materials(
    time: Res<Time>,
    mut surface_materials: ResMut<Assets<StarSurfaceMaterial>>,
    mut halo_materials: ResMut<Assets<StarHaloMaterial>>,
) {
    let elapsed = time.elapsed_secs();
    for (_, material) in surface_materials.iter_mut() {
        material.time = elapsed;
    }
    for (_, material) in halo_materials.iter_mut() {
        material.time = elapsed;
    }
}

fn billboard_star_halos(
    camera: Query<&GlobalTransform, (With<Camera3d>, Without<StarHalo>)>,
    mut halos: Query<(&GlobalTransform, &mut Transform), With<StarHalo>>,
) {
    let Some(camera_transform) = camera.iter().next() else {
        return;
    };
    let camera_position = camera_transform.translation();

    for (global_transform, mut transform) in &mut halos {
        let direction = camera_position - global_transform.translation();
        if direction.length_squared() > 0.0001 {
            transform.look_to(direction.normalize(), Vec3::Y);
        }
    }
}
