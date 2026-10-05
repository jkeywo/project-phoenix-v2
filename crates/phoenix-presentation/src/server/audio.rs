//! Server audio: config push, forcefield envelope, and positional cues.
//!
//! Playback lives in the browser provider or the native process-owned room
//! provider. Both consume the same audience-filtered geometry and envelope:
//!
//! - **Config push** — [`push_audio_config`] merges the local ship's `[audio]`
//!   block with the world's `[audio.red_alert]` and sends it once on game
//!   start, so JS can build its decoded audio graph from TOML rather than
//!   hardcoded markup.
//! - **Forcefield envelope** — [`process_forcefield_damage`] spikes an
//!   intensity on damage and [`drive_forcefield_level`] decays it, pushing the
//!   resulting volume as a bare float. Modelled directly on
//!   `viewscreen_border`'s `process_hull_shake` / `apply_camera_shake` pair.
//!   The envelope lives here rather than in JS because its five tuning knobs
//!   are in the ship TOML, which only Rust parses.
//! - **Positional cues** — [`push_blaster_cues`] rotates each blaster report
//!   into the listener's frame so JS can hand it straight to a `PannerNode`.
//! - **Ship's-computer tone** (issue #1342) — [`push_computer_message_cue`]
//!   fires a non-positional [`crate::audio_config::AudioCue::computer_message`]
//!   every time the narrative stream reports a message shown, keyed by
//!   severity through `[audio.computer_message]`.
//!
//! Both damage and blaster fire are observed by reading [`OutboundMessage`]
//! after `SimSet::Broadcast`, the same sanctioned route
//! `process_shield_flash` / `process_hull_shake` already use. That keeps the
//! whole feature inside `src/server/` — the shared weapons plugin needs no
//! changes, and NPC blasters are picked up for free.
//!
//! Server-only — gated by the `server` feature in `lib.rs`.

use crate::authoritative::{DeclareState, StateClass};
use bevy::prelude::*;

use crate::audio_config::{
    build_audio_payload, forcefield_decay, forcefield_spike, forcefield_volume, AudioCue,
    ForcefieldSource,
};
use crate::console_bridge::{AudioConfigChanged, AudioCueEvent};
use crate::core::codec;
use crate::core::messages::{GamePhase, ServerMessage};
use crate::core::narrative::{NarrativeEvent, NarrativeKind};
use crate::entities::spawner::ShipAudioSection;
use crate::lobby::OutboundMessage;
use crate::server_app::LocalShip;
use crate::ship::state::ShipPhysics;
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

#[derive(Resource, Default)]
pub struct ForcefieldLevel(pub f32);

pub struct ServerAudioPlugin;

impl Plugin for ServerAudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<AudioConfigChanged>()
            .add_message::<AudioCueEvent>()
            .add_message::<crate::gm_presentation::sound::LiveSoundRequest>()
            .init_resource::<crate::gm_presentation::sound::LiveSoundCatalog>()
            .add_systems(
                PostUpdate,
                crate::gm_presentation::sound::publish
                    .after(super::audio_lifecycle::publish_audio_lifecycle),
            )
            .init_resource::<ForcefieldAudioState>()
            .declare_state::<ForcefieldLevel>(StateClass::Presentation, "audio-lifecycle-state")
            .init_resource::<ForcefieldLevel>()
            .init_resource::<AudioConfigSent>()
            .init_resource::<super::audio_lifecycle::RoomAudioLifecycle>()
            .declare_state::<super::audio_lifecycle::RoomAudioLifecycle>(
                StateClass::Presentation,
                "room-audio-lifecycle",
            )
            .add_systems(
                PostUpdate,
                super::audio_lifecycle::publish_audio_lifecycle
                    .in_set(crate::audio_lifecycle::AudioLifecyclePublished),
            )
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
                    // No `.after(SimSet::Broadcast)` edges since issue #895:
                    // the sim's Broadcast set runs in `FixedUpdate`, which
                    // always completes before `Update`, so the OutboundMessage
                    // stream these read is already settled for the frame.
                    process_forcefield_damage,
                    // Deliberately not gated on InProgress: if a hit spikes
                    // the level just as the ship dies, gating here would
                    // freeze the decay and leave the bed looping at full
                    // volume forever. With no LocalShip (lobby) it early-returns.
                    drive_forcefield_level.after(process_forcefield_damage),
                    push_blaster_cues,
                    // Ship's-computer tone (issue #1342). Reads the narrative
                    // stream — written in `FixedUpdate`, already settled for
                    // the frame — rather than a `Changed<ActiveComputerMessage>`
                    // so a replacement at the SAME severity still fires a
                    // fresh cue: a `ComputerMessagePosted` event is written on
                    // every `show`, never suppressed by "nothing looks
                    // different".
                    push_computer_message_cue,
                ),
            );
    }
}

fn reset_audio_config_sent(mut sent: ResMut<AudioConfigSent>) {
    sent.0 = false;
}

/// Restore/recovery continuation: discard the old damage envelope and publish
/// the freshly restored authored config through the existing configuration lane.
pub(super) fn rebase_audio_presentation(world: &mut World) {
    if let Some(mut state) = world.get_resource_mut::<ForcefieldAudioState>() {
        state.intensity = 0.0;
    }
    let ship = world
        .query_filtered::<&ShipAudioSection, With<LocalShip>>()
        .single(world)
        .ok()
        .map(|section| section.0.clone());
    world.insert_resource(ForcefieldLevel(
        ship.as_ref()
            .and_then(|ship| ship.forcefield.as_ref())
            .map_or(0.0, |spec| spec.base_volume),
    ));
    if !world
        .get_resource::<State<GamePhase>>()
        .is_some_and(|phase| *phase.get() == GamePhase::InProgress)
    {
        return;
    }
    let authored_world = world
        .get_resource::<WorldConfig>()
        .and_then(|config| config.audio.clone());
    let payload = build_audio_payload(ship.as_ref(), authored_world.as_ref());
    if let Ok(json) = codec::encode_room_audio_config(
        &payload,
        world.get_resource::<crate::gm_presentation::sound::LiveSoundCatalog>(),
    ) {
        if let Some(mut configs) = world.get_resource_mut::<Messages<AudioConfigChanged>>() {
            configs.clear();
            configs.write(AudioConfigChanged { json });
        }
        if let Some(mut sent) = world.get_resource_mut::<AudioConfigSent>() {
            // A continuation boundary can precede the configured ship. Clear
            // the old mix now, retaining the ordinary late-spawn retry.
            sent.0 = ship.is_some() || authored_world.is_some();
        }
    }
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
    catalog: Option<Res<crate::gm_presentation::sound::LiveSoundCatalog>>,
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
    match codec::encode_room_audio_config(&payload, catalog.as_deref()) {
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
    mut output: ResMut<ForcefieldLevel>,
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
    output.0 = level;
}

/// Emits a positional [`AudioCue`] for every blaster shot — the player's and
/// every NPC's, since all of them are broadcast.
fn push_blaster_cues(
    mut outbound: MessageReader<OutboundMessage>,
    ship_q: Query<
        (
            &ShipAudioSection,
            &ShipPhysics,
            Option<&crate::entities::spawner::EntityUuid>,
            Option<&crate::ship::state::ShipViewMode>,
        ),
        With<LocalShip>,
    >,
    mut writer: MessageWriter<AudioCueEvent>,
    content: Option<Res<crate::world::server::WorldContentRuntime>>,
    tick: Option<Res<crate::sim_tick::SimTick>>,
) {
    let Ok((section, physics, observer, view)) = ship_q.single() else {
        return;
    };
    let Some(cfg) = section.0.blaster.as_ref() else {
        return;
    };
    for msg in outbound.read() {
        let ServerMessage::BlasterFired {
            x, z, source_uuid, ..
        } = &msg.msg
        else {
            continue;
        };
        if let (Some(content), Some(observer), Some(view)) = (content.as_deref(), observer, view) {
            let effective = crate::gm_presentation::resolved_view_mode(
                content.presentation.get(&observer.0),
                tick.as_deref().map_or(0, |tick| tick.0),
                view,
            );
            if crate::gm_information::suppresses_spatial_cue(
                &content.contact_information,
                &content.contact_overrides,
                &observer.0,
                source_uuid,
                &effective,
            ) {
                // Consume the public combat occurrence, never delay/replay it
                // or expose true geometry through an altered Sensors picture.
                continue;
            }
        }
        let pos = crate::audio_config::listener_relative(physics.x, physics.z, physics.yaw, *x, *z);
        // Cull shots beyond the configured falloff: they'd be inaudible, but
        // each one still costs an AudioBufferSourceNode allocation in JS.
        let dist_sq = pos[0] * pos[0] + pos[2] * pos[2];
        if dist_sq > cfg.max_distance * cfg.max_distance {
            continue;
        }
        match codec::to_json(&AudioCue::blaster(pos)) {
            Ok(json) => {
                writer.write(AudioCueEvent { json });
            }
            Err(e) => warn!("failed to encode blaster cue: {e}"),
        }
    }
}

/// Emits a non-positional [`AudioCue::computer_message`] every time a message
/// is shown (issue #1342) — including a replacement at the same severity,
/// since a fresh `ComputerMessagePosted` narrative event is written on every
/// `show`, never suppressed by "the severity didn't change".
///
/// Reads [`NarrativeEvent`] rather than the `ActiveComputerMessage` resource
/// directly: the event already carries the severity as a plain string
/// (`NarrativeKind::ComputerMessagePosted`'s `severity` detail), so this needs
/// no second lookup into narrative's detail-encoding, and it naturally skips
/// silent when the ship has no `[audio.computer_message]` section configured
/// for that severity at all — "missing configuration is silent" (AC4).
fn push_computer_message_cue(
    mut events: MessageReader<NarrativeEvent>,
    ship_q: Query<&ShipAudioSection, With<LocalShip>>,
    mut writer: MessageWriter<AudioCueEvent>,
) {
    let Ok(section) = ship_q.single() else {
        return;
    };
    let Some(cfg) = section.0.computer_message.as_ref() else {
        return;
    };
    for event in events.read() {
        if event.kind != NarrativeKind::ComputerMessagePosted {
            continue;
        }
        let Some(crate::core::narrative::NarrativeValue::Text(severity)) =
            event.detail.get("severity")
        else {
            continue;
        };
        if cfg.for_severity(severity).is_none() {
            continue;
        }
        match codec::to_json(&AudioCue::computer_message(severity)) {
            Ok(json) => {
                writer.write(AudioCueEvent { json });
            }
            Err(e) => warn!("failed to encode computer-message cue: {e}"),
        }
    }
}

#[cfg(test)]
#[path = "audio_tests.rs"]
mod tests;
