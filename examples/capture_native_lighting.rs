//! Bounded GPU smoke of the actual native mission renderer.
//! cargo run --features host --example capture_native_lighting -- target/native-lighting.png [flare intensity] [msaa samples]
use bevy::{
    app::AppExit,
    prelude::*,
    render::view::screenshot::{save_to_disk, Screenshot},
    transform::TransformSystems,
};
use project_phoenix::{
    boot::NativeRenderSurface,
    native_host::{build_native_host_app, preload_content_templates, NativeHostConfig},
    render_setup::GameCamera,
    world::config::{RenderConfig, WorldConfig},
};
#[derive(Resource)]
struct Capture {
    output: String,
    started: std::time::Instant,
    taken: bool,
    msaa: Msaa,
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let output = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "target/native-lighting.png".into());
    let intensity = args
        .get(2)
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(3.0);
    let msaa = if args.get(3).is_some_and(|s| s == "1") {
        Msaa::Off
    } else {
        Msaa::Sample4
    };
    std::env::set_var("BEVY_ASSET_ROOT", std::env::current_dir().unwrap());
    let preload = preload_content_templates(".").expect("content preloads");
    let mut cfg = NativeHostConfig::new("assets/worlds/combat_test.toml");
    cfg.solo = true;
    cfg.surface = NativeRenderSurface::Window;
    let mut app = build_native_host_app(&cfg, &preload).expect("native host assembles");
    app.world_mut()
        .resource_mut::<WorldConfig>()
        .render
        .get_or_insert_with(RenderConfig::default)
        .native
        .flare_intensity = intensity;
    app.insert_resource(Capture {
        output,
        started: std::time::Instant::now(),
        taken: false,
        msaa,
    })
    .add_systems(PostUpdate, pose.before(TransformSystems::Propagate))
    .add_systems(Update, capture)
    .run();
}
fn pose(mut cameras: Query<(&mut Transform, &mut Msaa), With<GameCamera>>, capture: Res<Capture>) {
    for (mut transform, mut msaa) in &mut cameras {
        *transform = Transform::from_xyz(400.0, 15.0, 200.0).looking_at(Vec3::ZERO, Vec3::Y);
        *msaa = capture.msaa;
    }
}
fn capture(mut commands: Commands, mut capture: ResMut<Capture>, mut exit: MessageWriter<AppExit>) {
    let elapsed = capture.started.elapsed().as_secs_f32();
    if elapsed > 15.0 && !capture.taken {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(capture.output.clone()));
        capture.taken = true;
    }
    if elapsed > 19.0 {
        exit.write(AppExit::Success);
    }
}
