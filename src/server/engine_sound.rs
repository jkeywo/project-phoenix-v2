use bevy::audio::{AudioSink, AudioSource, PlaybackSettings, AudioPlayer, Volume};
use bevy::prelude::*;

use crate::gui::setup_ui_sounds;
use crate::messages::GamePhase;
use crate::server_app::ShipImpulse;
use crate::ship_state::ShipState;
use crate::ship_plugin::LastHelmInput;

#[derive(Resource)]
pub struct EngineSoundHandle(pub Handle<AudioSource>);

#[derive(Resource)]
pub struct BgHumHandle(pub Handle<AudioSource>);

#[derive(Resource)]
pub struct RedAlertSirenHandle(pub Handle<AudioSource>);

#[derive(Resource, Default)]
struct PrevRedAlert(bool);

#[derive(Component)]
pub struct EngineSoundPlayer;

#[derive(Component)]
pub struct BgHumPlayer;

pub struct EngineSoundPlugin;

impl Plugin for EngineSoundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PrevRedAlert>()
            .add_systems(Startup, (setup_engine_sound, setup_ui_sounds))
            .add_systems(
                OnEnter(GamePhase::InProgress),
                (spawn_engine_sound, spawn_bg_hum, reset_prev_red_alert),
            )
            .add_systems(
                OnExit(GamePhase::InProgress),
                (despawn_engine_sound, despawn_bg_hum),
            )
            .add_systems(
                Update,
                (
                    update_engine_volume.after(crate::sim_sets::SimSet::Physics),
                    play_siren_on_red_alert,
                )
                    .run_if(in_state(GamePhase::InProgress)),
            );
    }
}

fn setup_engine_sound(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(EngineSoundHandle(
        asset_server.load("sounds/ship_engine.wav"),
    ));
    commands.insert_resource(BgHumHandle(
        asset_server.load("sounds/background_hum.ogg"),
    ));
    commands.insert_resource(RedAlertSirenHandle(
        asset_server.load("sounds/red_alert_siren.ogg"),
    ));
}

fn reset_prev_red_alert(mut prev: ResMut<PrevRedAlert>) {
    prev.0 = false;
}

fn spawn_engine_sound(mut commands: Commands, handle: Res<EngineSoundHandle>) {
    commands.spawn((
        AudioPlayer::<AudioSource>(handle.0.clone()),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)),
        EngineSoundPlayer,
    ));
}

fn spawn_bg_hum(mut commands: Commands, handle: Res<BgHumHandle>) {
    commands.spawn((
        AudioPlayer::<AudioSource>(handle.0.clone()),
        PlaybackSettings::LOOP.with_volume(Volume::Linear(0.25)),
        BgHumPlayer,
    ));
}

fn despawn_engine_sound(mut commands: Commands, query: Query<Entity, With<EngineSoundPlayer>>) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

fn despawn_bg_hum(mut commands: Commands, query: Query<Entity, With<BgHumPlayer>>) {
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

fn play_siren_on_red_alert(
    mut commands: Commands,
    ship: Res<ShipState>,
    mut prev: ResMut<PrevRedAlert>,
    handle: Res<RedAlertSirenHandle>,
) {
    let current = ship.red_alert();
    if current && !prev.0 {
        commands.spawn((
            AudioPlayer::<AudioSource>(handle.0.clone()),
            PlaybackSettings::ONCE,
        ));
    }
    prev.0 = current;
}
