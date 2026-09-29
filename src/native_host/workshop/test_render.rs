//! Presentation of the ordinary native Test in the parent's docked document.
//! One offscreen composite, one outstanding GPU capture, one queued encode.
use super::test_frames::{self, HEIGHT, WIDTH};
use crate::console_bridge::*;
use crate::core::codec;
use crate::workshop::test_protocol::TestPresentation;
use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use std::{
    io::Cursor,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Resource)]
struct TestRender {
    image: Option<Handle<Image>>,
    pending: Option<Instant>,
    next: Instant,
    encode: mpsc::SyncSender<Image>,
}
#[derive(Resource, Default)]
struct PresentationFeed {
    payload: TestPresentation,
    last: Option<String>,
}

pub fn install(app: &mut App) -> Result<(), String> {
    let (encode, input) = mpsc::sync_channel::<Image>(1);
    let failure = test_frames::PresentationFailure::default();
    let worker_failure = failure.clone();
    std::thread::Builder::new()
        .name("phoenix-test-frame-encode".into())
        .spawn(move || {
            while let Ok(image) = input.recv() {
                let result = (|| {
                    let image = image.try_into_dynamic().map_err(|e| e.to_string())?;
                    let mut bytes = Cursor::new(Vec::new());
                    image
                        .write_to(&mut bytes, image::ImageFormat::Png)
                        .map_err(|e| e.to_string())?;
                    test_frames::write_frame(&mut std::io::stdout().lock(), bytes.get_ref())
                        .map_err(|e| e.to_string())
                })();
                if let Err(error) = result {
                    worker_failure.fail(error);
                    break;
                }
            }
        })
        .map_err(|e| e.to_string())?;
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<TestRender>(StateClass::Presentation, "gm-milestone-integrated-workshop")
        .declare_state::<PresentationFeed>(
            StateClass::Presentation,
            "gm-milestone-integrated-workshop",
        )
        .declare_state::<test_frames::PresentationFailure>(
            StateClass::Presentation,
            "gm-milestone-integrated-workshop",
        )
        .insert_resource(failure)
        .insert_resource(TestRender {
            image: None,
            pending: None,
            next: Instant::now(),
            encode,
        })
        .init_resource::<PresentationFeed>()
        .add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
            Duration::from_millis(16),
        ))
        .add_systems(PostStartup, attach_target)
        .add_systems(Last, (capture, feed_presentation));
    // Native Test has no lobby/GM surface, so its ordinary projection plugins
    // have not been installed by NativeGmPlugin. Add only their read half.
    app.add_plugins((
        crate::gm_projection::GmProjectionPlugin,
        crate::gm_activity::GmActivityPlugin,
        crate::gm_attention::GmAttentionPlugin,
        crate::gm_health::GmHealthPlugin,
        crate::gm_workload::GmWorkloadPlugin,
    ));
    Ok(())
}

fn attach_target(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    cameras: Query<Entity, With<Camera>>,
    mut state: ResMut<TestRender>,
) {
    let mut image = Image::new_fill(
        Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage |= TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC;
    let image = images.add(image);
    // The game's HDR-matched 3D and UI cameras must share one target. Replacing
    // either renderer or retargeting only one camera breaks the composite.
    for camera in &cameras {
        commands
            .entity(camera)
            .insert(RenderTarget::from(image.clone()));
    }
    state.image = Some(image);
}

fn capture(
    mut commands: Commands,
    mut state: ResMut<TestRender>,
    failure: Res<test_frames::PresentationFailure>,
) {
    let now = Instant::now();
    if let Some(started) = state.pending {
        if now.saturating_duration_since(started) >= Duration::from_secs(15) {
            failure
                .fail("The native Test renderer did not return a frame within 15 seconds".into());
        }
        return;
    }
    if now < state.next || failure.error().is_some() {
        return;
    }
    let Some(image) = state.image.clone() else {
        return;
    };
    state.pending = Some(now);
    state.next = Instant::now() + Duration::from_millis(100);
    commands.spawn(Screenshot::image(image)).observe(
        |shot: On<ScreenshotCaptured>,
         mut state: ResMut<TestRender>,
         failure: Res<test_frames::PresentationFailure>| {
            state.pending = None;
            match state.encode.try_send(shot.image.clone()) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => {} // Drop, never queue frames behind rendering.
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    failure.fail("Test frame encoder stopped".into())
                }
            }
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn feed_presentation(
    mut feed: ResMut<PresentationFeed>,
    config: Res<crate::world::config::WorldConfig>,
    failure: Res<test_frames::PresentationFailure>,
    mut hud: MessageReader<HudStateChanged>,
    mut entity: MessageReader<GmEntityProjectionChanged>,
    mut activity: MessageReader<GmActivityFeedChanged>,
    mut station: MessageReader<GmStationProjectionChanged>,
    mut session: MessageReader<GmSessionChanged>,
    mut mission: MessageReader<GmMissionChanged>,
    mut spawn: MessageReader<GmSpawnChanged>,
    mut comms: MessageReader<GmCommsChanged>,
    mut attention: MessageReader<GmAttentionChanged>,
    mut health: MessageReader<GmHealthChanged>,
    mut workload: MessageReader<GmWorkloadChanged>,
) {
    if failure.error().is_some() {
        return;
    }
    macro_rules! feed_channel {
        ($reader:ident, $name:literal, $encode:ident) => {
            if let Some(message) = $reader.read().last() {
                match codec::$encode(&message.payload) {
                    Ok(json) => {
                        feed.payload.channels.insert($name.into(), json);
                    }
                    Err(error) => failure.fail(error.to_string()),
                }
            }
        };
    }
    if let Some(message) = hud.read().last() {
        feed.payload
            .channels
            .insert("hud".into(), message.json.clone());
    }
    feed_channel!(entity, "gm_entity", encode_gm_entity_projection);
    feed_channel!(activity, "gm_activity", encode_gm_activity_feed);
    feed_channel!(station, "gm_station", encode_gm_station_projection);
    feed_channel!(session, "gm_session", encode_gm_session_projection);
    feed_channel!(mission, "gm_mission", encode_gm_mission_projection);
    feed_channel!(spawn, "gm_spawn", encode_gm_spawn_projection);
    feed_channel!(comms, "gm_comms", encode_gm_comms_projection);
    feed_channel!(attention, "gm_attention", encode_gm_attention_projection);
    feed_channel!(health, "gm_health", encode_gm_health_projection);
    feed_channel!(workload, "gm_workload", encode_gm_workload_projection);
    feed.payload.role_presets = codec::encode_gm_role_presets(&config.gm_role_presets);
    // Compare at the previous sequence; only changed absolute state emits.
    match codec::encode_workshop_test_presentation(&feed.payload) {
        Ok(json) if feed.last.as_ref() == Some(&json) => {}
        Ok(_) => {
            feed.payload.sequence = feed.payload.sequence.saturating_add(1);
            match codec::encode_workshop_test_presentation(&feed.payload) {
                Ok(json) => {
                    if let Err(error) =
                        test_frames::write_presentation(&mut std::io::stdout().lock(), &json)
                    {
                        failure.fail(error.to_string());
                    } else {
                        feed.last = Some(json);
                    }
                }
                Err(error) => failure.fail(error),
            }
        }
        Err(error) => failure.fail(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    #[test]
    fn missing_gpu_reply_fails_the_run_instead_of_remaining_healthy_with_no_frame() {
        let (encode, _receive) = mpsc::sync_channel(1);
        let mut world = World::new();
        world.insert_resource(test_frames::PresentationFailure::default());
        world.insert_resource(TestRender {
            image: None,
            pending: Some(Instant::now() - Duration::from_secs(16)),
            next: Instant::now(),
            encode,
        });
        world.run_system_once(capture).unwrap();
        assert!(world
            .resource::<test_frames::PresentationFailure>()
            .error()
            .unwrap()
            .contains("15 seconds"));
    }
}
