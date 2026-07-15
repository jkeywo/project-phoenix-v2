//! Server audio: config push, forcefield envelope, and positional cues.
//!
//! Playback itself lives in the host page's JS — Bevy audio was tried and
//! reverted in-browser. What this plugin does is feed that JS:
//!
//! - **Config push** — [`push_audio_config`] merges the local ship's `[audio]`
//!   block with the world's `[audio.red_alert]` and sends it once on game
//!   start, so JS can build its `<audio>` elements from TOML rather than
//!   hardcoded markup.
//! - **Forcefield envelope** — [`process_forcefield_damage`] spikes an
//!   intensity on damage and [`drive_forcefield_level`] decays it, pushing the
//!   resulting volume as a bare float. Modelled directly on
//!   `viewscreen_border`'s `process_hull_shake` / `apply_camera_shake` pair.
//!   The envelope lives here rather than in JS because its five tuning knobs
//!   are in the ship TOML, which only Rust parses.
//! - **Positional cues** — [`push_blaster_cues`] rotates each blaster report
//!   into the listener's frame so JS can hand it straight to a `PannerNode`.
//!
//! Both damage and blaster fire are observed by reading [`OutboundMessage`]
//! after `SimSet::Broadcast`, the same sanctioned route
//! `process_shield_flash` / `process_hull_shake` already use. That keeps the
//! whole feature inside `src/server/` — the shared weapons plugin needs no
//! changes, and NPC blasters are picked up for free.
//!
//! Server-only — gated by the `server` feature in `lib.rs`.

use bevy::prelude::*;

use crate::audio_config::{
    build_audio_payload, forcefield_decay, forcefield_spike, forcefield_volume, AudioCue,
    ForcefieldSource,
};
use crate::codec;
use crate::console_bridge::{AudioConfigChanged, AudioCueEvent};
use crate::entity_spawner::ShipAudioSection;
use crate::lobby::OutboundMessage;
use crate::messages::{GamePhase, ServerMessage};
use crate::ship_state::ShipPhysics;
use crate::sim_sets::SimSet;
use crate::simulation::LocalShip;
use crate::world::config::WorldConfig;

/// Current forcefield SFX intensity, 0.0 (idle bed) to 1.0 (full hit).
///
/// Spiked by [`process_forcefield_damage`], decayed by
/// [`drive_forcefield_level`]. Mirrors `viewscreen_border::ShakeState` /
/// `ShieldFlashState`.
#[derive(Resource, Default, Debug)]
pub struct ForcefieldAudioState {
    pub intensity: f32,
}

/// Whether [`push_audio_config`] has already sent this game's config. Reset on
/// entering `InProgress` so a second playthrough re-pushes.
#[derive(Resource, Default, Debug)]
struct AudioConfigSent(bool);

pub struct ServerAudioPlugin;

impl Plugin for ServerAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<AudioConfigChanged>()
            .add_message::<AudioCueEvent>()
            .init_resource::<ForcefieldAudioState>()
            .init_resource::<AudioConfigSent>()
            .add_systems(OnEnter(GamePhase::InProgress), reset_audio_config_sent)
            .add_systems(
                Update,
                (
                    // Runs every frame until the ship exists, then latches. An
                    // OnEnter system would be simpler, but it must observe the
                    // ship spawned by `spawn_game_start_entities` in the same
                    // OnEnter chain — and if it lost that ordering race it
                    // would never get a second chance.
                    push_audio_config.run_if(in_state(GamePhase::InProgress)),
                    process_forcefield_damage.after(SimSet::Broadcast),
                    // Deliberately not gated on InProgress: if a hit spikes
                    // the level just as the ship dies, gating here would
                    // freeze the decay and leave the bed looping at full
                    // volume forever. With no LocalShip (lobby) it early-returns.
                    drive_forcefield_level.after(process_forcefield_damage),
                    push_blaster_cues.after(SimSet::Broadcast),
                ),
            );
    }
}

fn reset_audio_config_sent(mut sent: ResMut<AudioConfigSent>) {
    sent.0 = false;
}

/// Sends the merged ship + world audio config to JS, once, on game start.
///
/// Reads [`ShipAudioSection`] off the `LocalShip` rather than
/// `SelectedShipResource`: that resource is snapshotted at `wasm_init`, so
/// lobby ship-picker changes never reach it. The spawned component is the only
/// source that reflects what the player actually chose.
fn push_audio_config(
    ship_q: Query<&ShipAudioSection, With<LocalShip>>,
    world_config: Option<Res<WorldConfig>>,
    mut writer: MessageWriter<AudioConfigChanged>,
    mut sent: ResMut<AudioConfigSent>,
) {
    if sent.0 {
        return;
    }
    let ship = ship_q.single().ok().map(|s| &s.0);
    let world = world_config.as_ref().and_then(|wc| wc.audio.as_ref());
    if ship.is_none() && world.is_none() {
        // Nothing configured anywhere yet — the ship may still be spawning, so
        // leave the latch open and try again next frame.
        return;
    }
    let payload = build_audio_payload(ship, world);
    match codec::encode_audio_config(&payload) {
        Ok(json) => {
            writer.write(AudioConfigChanged { json });
            sent.0 = true;
        }
        Err(e) => warn!("failed to encode audio config: {e}"),
    }
}

/// Spikes [`ForcefieldAudioState`] on player-facing damage.
///
/// `DamageTaken` is only ever emitted for the `LocalShip` (see `server_app`),
/// so no entity filter is needed here.
fn process_forcefield_damage(
    mut outbound: MessageReader<OutboundMessage>,
    mut state: ResMut<ForcefieldAudioState>,
    ship_q: Query<&ShipAudioSection, With<LocalShip>>,
) {
    let Ok(section) = ship_q.single() else { return };
    let Some(cfg) = section.0.forcefield.as_ref() else {
        return;
    };
    for msg in outbound.read() {
        let ServerMessage::DamageTaken { hull, shield } = &msg.msg else {
            continue;
        };
        let damage = match cfg.source {
            ForcefieldSource::Shield => *shield,
            ForcefieldSource::Hull => *hull,
            ForcefieldSource::Total => *hull + *shield,
        };
        if let Some(spike) = forcefield_spike(damage, cfg.damage_threshold, cfg.damage_full_spike) {
            // Take the louder of the decaying tail and the new hit, so a big
            // hit is never quietened by an in-flight decay.
            state.intensity = state.intensity.max(spike);
        }
    }
}

/// Decays the intensity and pushes the resulting volume to JS.
fn drive_forcefield_level(
    mut state: ResMut<ForcefieldAudioState>,
    ship_q: Query<&ShipAudioSection, With<LocalShip>>,
    time: Res<Time>,
) {
    let Ok(section) = ship_q.single() else { return };
    let Some(cfg) = section.0.forcefield.as_ref() else {
        return;
    };
    state.intensity = forcefield_decay(state.intensity, time.delta_secs(), cfg.decay_rate_per_sec);
    let level = forcefield_volume(state.intensity, cfg.base_volume, cfg.spike_volume);
    push_forcefield_level(level);
}

/// WASM forwards the level to JS; native has nowhere to send it. Mirrors the
/// cfg split in `viewscreen_border::apply_camera_shake`.
#[cfg(target_arch = "wasm32")]
fn push_forcefield_level(level: f32) {
    crate::server::bridge::set_forcefield_level(level);
}

#[cfg(not(target_arch = "wasm32"))]
fn push_forcefield_level(_level: f32) {}

/// Emits a positional [`AudioCue`] for every blaster shot — the player's and
/// every NPC's, since all of them are broadcast.
fn push_blaster_cues(
    mut outbound: MessageReader<OutboundMessage>,
    ship_q: Query<(&ShipAudioSection, &ShipPhysics), With<LocalShip>>,
    mut writer: MessageWriter<AudioCueEvent>,
) {
    let Ok((section, physics)) = ship_q.single() else {
        return;
    };
    let Some(cfg) = section.0.blaster.as_ref() else {
        return;
    };
    for msg in outbound.read() {
        let ServerMessage::BlasterFired { x, z, .. } = &msg.msg else {
            continue;
        };
        let pos = crate::audio_config::listener_relative(physics.x, physics.z, physics.yaw, *x, *z);
        // Cull shots beyond the configured falloff: they'd be inaudible, but
        // each one still costs an AudioBufferSourceNode allocation in JS.
        let dist_sq = pos[0] * pos[0] + pos[2] * pos[2];
        if dist_sq > cfg.max_distance * cfg.max_distance {
            continue;
        }
        match codec::encode_audio_cue(&AudioCue::blaster(pos)) {
            Ok(json) => {
                writer.write(AudioCueEvent { json });
            }
            Err(e) => warn!("failed to encode blaster cue: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_config::{ForcefieldAudio, ShipAudioConfig};
    use crate::lobby::Target;
    use crate::messages::DeliveryClass;

    fn forcefield_cfg() -> ForcefieldAudio {
        ForcefieldAudio {
            file: "assets/sounds/ForcefieldHit.mp3".into(),
            base_volume: 0.06,
            spike_volume: 0.8,
            damage_threshold: 1.0,
            damage_full_spike: 30.0,
            decay_rate_per_sec: 1.5,
            source: ForcefieldSource::Shield,
        }
    }

    /// Minimal app: the systems under test plus a LocalShip carrying an
    /// audio section. No rendering, no full sim.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_message::<OutboundMessage>()
            .add_message::<AudioCueEvent>()
            .init_resource::<ForcefieldAudioState>()
            .init_resource::<Time>();
        app.add_systems(
            Update,
            (
                process_forcefield_damage,
                drive_forcefield_level.after(process_forcefield_damage),
                push_blaster_cues,
            ),
        );
        app.world_mut().spawn((
            LocalShip,
            ShipPhysics::default(),
            ShipAudioSection(ShipAudioConfig {
                forcefield: Some(forcefield_cfg()),
                ..Default::default()
            }),
        ));
        app
    }

    /// Advance the clock, then run one update.
    ///
    /// A bare `App` with `init_resource::<Time>()` never advances on its own,
    /// so `delta_secs()` would stay 0 and the decay step would be a silent
    /// no-op — the test would pass while testing nothing.
    fn tick(app: &mut App, dt_secs: f32) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(dt_secs));
        app.update();
    }

    fn send_damage(app: &mut App, hull: f32, shield: f32) {
        app.world_mut()
            .resource_mut::<Messages<OutboundMessage>>()
            .write(OutboundMessage {
                target: Target::All,
                msg: ServerMessage::DamageTaken { hull, shield },
                // Matches `drain_lobby_outbox`, which marks every drained
                // message Reliable.
                delivery: DeliveryClass::Reliable,
            });
    }

    fn intensity(app: &App) -> f32 {
        app.world().resource::<ForcefieldAudioState>().intensity
    }

    #[test]
    fn full_shield_hit_spikes_intensity_to_one() {
        let mut app = test_app();
        send_damage(&mut app, 0.0, 30.0);
        // A tiny dt so the same update's decay step barely bites.
        tick(&mut app, 0.001);
        assert!(intensity(&app) > 0.99, "got {}", intensity(&app));
    }

    #[test]
    fn damage_below_threshold_leaves_intensity_untouched() {
        let mut app = test_app();
        send_damage(&mut app, 0.0, 0.5);
        tick(&mut app, 0.016);
        assert_eq!(intensity(&app), 0.0);
    }

    #[test]
    fn hull_damage_ignored_when_source_is_shield() {
        let mut app = test_app();
        send_damage(&mut app, 50.0, 0.0);
        tick(&mut app, 0.016);
        assert_eq!(intensity(&app), 0.0);
    }

    #[test]
    fn intensity_decays_to_zero_and_stops() {
        let mut app = test_app();
        send_damage(&mut app, 0.0, 30.0);
        tick(&mut app, 0.001);
        let peak = intensity(&app);

        // decay_rate is 1.5/sec, so ~0.67s of silence should reach zero.
        let mut prev = peak;
        for _ in 0..10 {
            tick(&mut app, 0.1);
            let now = intensity(&app);
            assert!(
                now <= prev,
                "intensity rose without damage: {prev} -> {now}"
            );
            assert!(now >= 0.0, "intensity went negative: {now}");
            prev = now;
        }
        assert_eq!(prev, 0.0, "should have fully decayed to the bed");
        assert!(peak > prev, "never decayed at all");
    }

    #[test]
    fn a_bigger_hit_overrides_a_decaying_tail() {
        let mut app = test_app();
        send_damage(&mut app, 0.0, 5.0);
        tick(&mut app, 0.001);
        let small = intensity(&app);

        send_damage(&mut app, 0.0, 30.0);
        tick(&mut app, 0.001);
        let big = intensity(&app);
        assert!(
            big > small,
            "big hit did not override tail: {small} -> {big}"
        );
    }

    #[test]
    fn a_glancing_hit_does_not_quieten_a_loud_tail() {
        // `.max()` rather than assignment: a 2 HP scratch mid-decay must not
        // cut the tail of a 30 HP hit short.
        let mut app = test_app();
        send_damage(&mut app, 0.0, 30.0);
        tick(&mut app, 0.001);
        let loud = intensity(&app);

        send_damage(&mut app, 0.0, 2.0);
        tick(&mut app, 0.001);
        let after = intensity(&app);
        assert!(
            after > loud - 0.01,
            "glancing hit cut the tail: {loud} -> {after}"
        );
    }

    #[test]
    fn ship_without_forcefield_config_never_spikes() {
        let mut app = App::new();
        app.add_message::<OutboundMessage>()
            .init_resource::<ForcefieldAudioState>()
            .init_resource::<Time>()
            .add_systems(Update, process_forcefield_damage);
        app.world_mut().spawn((
            LocalShip,
            ShipAudioSection(ShipAudioConfig::default()), // no [audio.forcefield]
        ));
        send_damage(&mut app, 0.0, 30.0);
        app.update();
        assert_eq!(
            app.world().resource::<ForcefieldAudioState>().intensity,
            0.0
        );
    }

    #[test]
    fn blaster_cue_carries_listener_relative_position() {
        let mut app = App::new();
        app.add_message::<OutboundMessage>()
            .add_message::<AudioCueEvent>()
            .init_resource::<Time>()
            .add_systems(Update, push_blaster_cues);
        app.world_mut().spawn((
            LocalShip,
            ShipPhysics::default(), // at origin, yaw 0 (facing -Z)
            ShipAudioSection(ShipAudioConfig {
                blaster: Some(crate::audio_config::BlasterAudio {
                    file: "assets/sounds/Blaster.mp3".into(),
                    volume: 0.9,
                    ref_distance: 30.0,
                    max_distance: 800.0,
                    rolloff_factor: 1.2,
                    distance_model: crate::audio_config::DistanceModel::Inverse,
                    panning_model: crate::audio_config::PanningModel::EqualPower,
                }),
                ..Default::default()
            }),
        ));
        // A shot 10 units due East of a north-facing ship is off the
        // starboard beam: +X in the listener's frame.
        app.world_mut()
            .resource_mut::<Messages<OutboundMessage>>()
            .write(OutboundMessage {
                target: Target::All,
                msg: ServerMessage::BlasterFired {
                    bank: "fore".into(),
                    source_uuid: "npc-1".into(),
                    projectile_id: "p1".into(),
                    x: 10.0,
                    z: 0.0,
                    heading: 0.0,
                    visual_scale: 1.0,
                },
                delivery: DeliveryClass::Reliable,
            });
        app.update();

        let cues = app.world().resource::<Messages<AudioCueEvent>>();
        let mut cursor = cues.get_cursor();
        let sent: Vec<_> = cursor.read(cues).collect();
        assert_eq!(sent.len(), 1, "expected exactly one blaster cue");
        let cue: AudioCue = serde_json::from_str(&sent[0].json).expect("valid cue JSON");
        assert_eq!(cue.kind, "blaster");
        assert!((cue.x - 10.0).abs() < 1e-4, "got x={}", cue.x);
        assert!(cue.z.abs() < 1e-4, "got z={}", cue.z);
    }

    #[test]
    fn blaster_cue_culled_beyond_max_distance() {
        let mut app = App::new();
        app.add_message::<OutboundMessage>()
            .add_message::<AudioCueEvent>()
            .init_resource::<Time>()
            .add_systems(Update, push_blaster_cues);
        app.world_mut().spawn((
            LocalShip,
            ShipPhysics::default(),
            ShipAudioSection(ShipAudioConfig {
                blaster: Some(crate::audio_config::BlasterAudio {
                    file: "assets/sounds/Blaster.mp3".into(),
                    volume: 0.9,
                    ref_distance: 30.0,
                    max_distance: 800.0,
                    rolloff_factor: 1.2,
                    distance_model: crate::audio_config::DistanceModel::Inverse,
                    panning_model: crate::audio_config::PanningModel::EqualPower,
                }),
                ..Default::default()
            }),
        ));
        app.world_mut()
            .resource_mut::<Messages<OutboundMessage>>()
            .write(OutboundMessage {
                target: Target::All,
                msg: ServerMessage::BlasterFired {
                    bank: "fore".into(),
                    source_uuid: "npc-far".into(),
                    projectile_id: "p2".into(),
                    x: 5000.0, // well beyond max_distance
                    z: 0.0,
                    heading: 0.0,
                    visual_scale: 1.0,
                },
                delivery: DeliveryClass::Reliable,
            });
        app.update();

        let cues = app.world().resource::<Messages<AudioCueEvent>>();
        let mut cursor = cues.get_cursor();
        assert_eq!(
            cursor.read(cues).count(),
            0,
            "inaudible shot should not allocate a JS audio node"
        );
    }
}
