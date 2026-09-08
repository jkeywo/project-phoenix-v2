//! Real output-device setup and a bounded, explicitly requested surface test.
//! CPAL objects stay on the calling setup thread. Streams never enter a Bevy
//! Resource or a callback-owned destructor. No recording or network calls.

use super::bridge_media::{
    self, DeviceAvailability, DiscoveredMediaDevice, MediaKind, RawMediaDevice,
};
use super::bridge_profile::BridgeProfile;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;

pub struct OutputDevices {
    pub discovered: Vec<DiscoveredMediaDevice>,
    handles: Vec<cpal::Device>,
    ambiguous: Vec<bool>,
}

impl OutputDevices {
    pub fn scan() -> Result<Self, String> {
        let host = cpal::default_host();
        let handles: Vec<_> = host
            .output_devices()
            .map_err(|error| error.to_string())?
            .collect();
        let raws: Vec<_> = handles
            .iter()
            .map(|device| RawMediaDevice {
                kind: MediaKind::Output,
                name: device.name().ok(),
                hardware_id: None,
                // CPAL 0.15 cannot expose a portable stable endpoint id. No default
                // is guessed by name, and a test always uses the explicit profile.
                default: false,
                availability: DeviceAvailability::Available,
            })
            .collect();
        let mut counts = HashMap::new();
        for raw in &raws {
            *counts.entry(raw.name.clone()).or_insert(0usize) += 1;
        }
        let ambiguous = raws
            .iter()
            .map(|raw| raw.name.is_none() || counts[&raw.name] > 1)
            .collect();
        Ok(Self {
            discovered: bridge_media::identify_media(&raws),
            handles,
            ambiguous,
        })
    }

    /// Enumerated handles are retained until teardown. Duplicate/unnamed
    /// endpoints cannot be rebound safely across launches and are refused.
    pub fn test_surface(
        &self,
        profile: &BridgeProfile,
        surface: &str,
    ) -> Result<Vec<String>, String> {
        let indices = selected_outputs(profile, surface, &self.discovered, &self.ambiguous)?;
        let mut results = Vec::new();
        for index in indices {
            let id = &self.discovered[index].identity;
            println!("Surface {surface:?}: testing output {id} (quiet one-second tone)");
            play_tone(&self.handles[index]).map_err(|error| {
                format!("Surface {surface:?}: output {id}: {error}; the surface remains usable")
            })?;
            results.push(format!(
                "Surface {surface:?}: output {id}: test completed; stream closed"
            ));
        }
        Ok(results)
    }
}

fn selected_outputs(
    profile: &BridgeProfile,
    surface: &str,
    discovered: &[DiscoveredMediaDevice],
    ambiguous: &[bool],
) -> Result<Vec<usize>, String> {
    let validated =
        bridge_media::validate_media(&profile.media).map_err(|error| error.to_string())?;
    let assigned = validated
        .surfaces
        .iter()
        .find(|entry| entry.surface == surface)
        .ok_or_else(|| format!("Unknown media surface {surface:?}"))?;
    if assigned.outputs.is_empty() {
        return Err(format!("Surface {surface:?} has no assigned outputs"));
    }
    // Preflight every requested output before playing any one of them.
    let indices: Vec<usize> = assigned.outputs.iter().map(|id| {
            let index = discovered.iter().position(|device| device.identity == *id)
                .ok_or_else(|| format!("Surface {surface:?}: output {id} is missing; nothing substituted"))?;
            if ambiguous[index] {
                return Err(format!("Surface {surface:?}: output {id} has no unique device name; give the endpoint a unique OS name and refresh"));
            }
            Ok(index)
        }).collect::<Result<_, String>>()?;
    Ok(indices)
}

fn play_tone(device: &cpal::Device) -> Result<(), String> {
    let config = device
        .default_output_config()
        .map_err(|error| error.to_string())?;
    macro_rules! play {
        ($sample:ty) => {
            play_samples::<$sample>(device, &config.into())
        };
    }
    match config.sample_format() {
        cpal::SampleFormat::I8 => play!(i8),
        cpal::SampleFormat::I16 => play!(i16),
        cpal::SampleFormat::I32 => play!(i32),
        cpal::SampleFormat::I64 => play!(i64),
        cpal::SampleFormat::U8 => play!(u8),
        cpal::SampleFormat::U16 => play!(u16),
        cpal::SampleFormat::U32 => play!(u32),
        cpal::SampleFormat::U64 => play!(u64),
        cpal::SampleFormat::F32 => play!(f32),
        cpal::SampleFormat::F64 => play!(f64),
        format => Err(format!("unsupported output sample format {format}")),
    }
}

fn tone_sample(frame: u32, rate: u32) -> f32 {
    if frame >= rate {
        return 0.0;
    }
    let fade = (rate / 50).max(1) as f32;
    let envelope = (frame as f32 / fade)
        .min((rate - frame) as f32 / fade)
        .min(1.0);
    (frame as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.04 * envelope
}

fn play_samples<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
) -> Result<(), String> {
    if config.channels == 0 || config.sample_rate.0 == 0 {
        return Err("invalid output configuration".into());
    }
    // One terminal outcome, never an unbounded callback queue. Completion is
    // sent after a silence tail, not when the last tone frame was merely queued.
    let (done, receive) = mpsc::sync_channel(1);
    let failed = done.clone();
    let channels = usize::from(config.channels);
    let rate = config.sample_rate.0;
    let end = rate.saturating_add(rate / 4);
    let mut frame = 0u32;
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                for samples in data.chunks_mut(channels) {
                    let value = T::from_sample(tone_sample(frame, rate));
                    samples.fill(value);
                    frame = frame.saturating_add(1);
                }
                if frame >= end {
                    let _ = done.try_send(Ok(()));
                }
            },
            move |error| {
                let _ = failed.try_send(Err(error.to_string()));
            },
            None,
        )
        .map_err(|error| error.to_string())?;
    stream.play().map_err(|error| error.to_string())?;
    let result = receive
        .recv_timeout(Duration::from_secs(4))
        .map_err(|_| "output test timed out or device disconnected".to_string())?;
    // Drop is on the owner thread even on error/timeout; WASAPI can join its
    // callback thread here. No stream survives the diagnostic.
    drop(stream);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_refuses_ambiguous_and_missing_members_before_any_playback() {
        let profile: BridgeProfile = toml::from_str(
            r#"
version = 1
[[media]]
surface = "comms"
output = ["output:Headset", "output:Speakers"]
"#,
        )
        .unwrap();
        let devices =
            bridge_media::identify_media(&["Headset", "Speakers"].map(|name| RawMediaDevice {
                kind: MediaKind::Output,
                name: Some(name.into()),
                hardware_id: None,
                default: false,
                availability: DeviceAvailability::Available,
            }));
        assert_eq!(
            selected_outputs(&profile, "comms", &devices, &[false, false]).unwrap(),
            vec![0, 1]
        );
        assert!(selected_outputs(&profile, "comms", &devices[..1], &[false])
            .unwrap_err()
            .contains("missing"));
        assert!(
            selected_outputs(&profile, "comms", &devices, &[false, true])
                .unwrap_err()
                .contains("no unique device name")
        );
        assert!(
            selected_outputs(&profile, "viewscreen", &devices, &[false, false])
                .unwrap_err()
                .contains("Unknown media surface")
        );
    }
    #[test]
    fn the_test_tone_is_quiet_faded_and_finite_with_a_silent_tail() {
        for rate in [8_000, 44_100, 48_000, 192_000] {
            assert_eq!(tone_sample(0, rate), 0.0);
            assert!((0..rate).all(|frame| tone_sample(frame, rate).abs() <= 0.040001));
            assert_eq!(tone_sample(rate, rate), 0.0);
            assert_eq!(tone_sample(u32::MAX, rate), 0.0);
        }
    }
}
