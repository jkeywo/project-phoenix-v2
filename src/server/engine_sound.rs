use bevy::audio::{AudioSink, AudioSource, PlaybackSettings, AudioPlayer, Volume};
use bevy::prelude::*;

use crate::messages::GamePhase;
use crate::server_app::ShipImpulse;
use crate::ship_plugin::LastHelmInput;

#[derive(Resource)]
pub struct EngineSoundHandle(pub Handle<AudioSource>);

#[derive(Component)]
pub struct EngineSoundPlayer;

pub struct EngineSoundPlugin;

impl Plugin for EngineSoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_engine_sound)
            .add_systems(OnEnter(GamePhase::InProgress), spawn_engine_sound)
            .add_systems(OnExit(GamePhase::InProgress), despawn_engine_sound)
            .add_systems(
                Update,
                update_engine_volume
                    .after(crate::sim_sets::SimSet::Physics)
                    .run_if(in_state(GamePhase::InProgress)),
            );
    }
}

fn setup_engine_sound(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(EngineSoundHandle(
        asset_server.load("sounds/ship_engine.wav"),
    ));
}

fn spawn_engine_sound(mut commands: Commands, handle: Res<EngineSoundHandle>) {
    commands.spawn((
        AudioPlayer::<AudioSource>(handle.0.clone()),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
        EngineSoundPlayer,
    ));
}

fn despawn_engine_sound(mut commands: Commands, query: Query<Entity, With<EngineSoundPlayer>>) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

fn update_engine_volume(
    mut query: Query<&mut AudioSink, With<EngineSoundPlayer>>,
    last_input: Res<LastHelmInput>,
    impulse: Res<ShipImpulse>,
) {
    let Ok(mut sink) = query.single_mut() else {
        return;
    };
    let volume = if impulse.0.is_active() {
        1.0
    } else {
        last_input.thrust.abs()
    };
    sink.set_volume(Volume::Linear(volume));
}
