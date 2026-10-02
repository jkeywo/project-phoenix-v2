//! Real output-device setup and a bounded, explicitly requested surface test.
//! CPAL objects stay on the calling setup thread. Streams never enter a Bevy
//! Resource or a callback-owned destructor. No recording or network calls.

use super::bridge_media::{DeviceAvailability, DiscoveredMediaDevice, MediaKind, RawMediaDevice};
use super::bridge_profile::BridgeProfile;
use super::media_devices::{DeviceCatalogue, Entry, SelectionError};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;
use std::time::Duration;

pub struct OutputDevices {
    pub(crate) catalogue: DeviceCatalogue<cpal::Device>,
}
impl OutputDevices {
    pub fn scan() -> Result<Self, String> {
        let host = cpal::default_host();
        let handles: Vec<_> = host
            .output_devices()
            .map_err(|error| error.to_string())?
            .collect();
        let catalogue = DeviceCatalogue::new(handles.into_iter().map(|device| {
            let raw = RawMediaDevice {
                kind: MediaKind::Output,
                name: device.name().ok(),
                hardware_id: None,
                default: false,
                availability: DeviceAvailability::Available,
            };
            (raw, device)
        }));
        Ok(Self { catalogue })
    }
    pub fn discovered(&self) -> impl Iterator<Item = DiscoveredMediaDevice> + '_ {
        self.catalogue.entries().map(|entry| entry.device.clone())
    }

    /// Enumerated handles are retained until teardown. Duplicate/unnamed
    /// endpoints cannot be rebound safely across launches and are refused.
    pub fn test_surface(
        &self,
        profile: &BridgeProfile,
        surface: &str,
    ) -> Result<Vec<String>, String> {
        let indices = selected_outputs(profile, surface, &self.catalogue)?;
        let mut results = Vec::new();
        for entry in indices {
            let id = &entry.device.identity;
            println!("Surface {surface:?}: testing output {id} (quiet one-second tone)");
            play_tone(&entry.handle).map_err(|error| {
                format!("Surface {surface:?}: output {id}: {error}; the surface remains usable")
            })?;
            results.push(format!(
                "Surface {surface:?}: output {id}: test completed; stream closed"
            ));
        }
        Ok(results)
    }
}

fn selected_outputs<'a, H>(
    profile: &BridgeProfile,
    surface: &str,
    catalogue: &'a DeviceCatalogue<H>,
) -> Result<Vec<&'a Entry<H>>, String> {
    catalogue.surface(profile, surface, MediaKind::Output).map_err(|error| match error {
        SelectionError::InvalidProfile(error) => error,
        SelectionError::UnknownSurface => format!("Unknown media surface {surface:?}"),
        SelectionError::Unassigned => format!("Surface {surface:?} has no assigned outputs"),
        SelectionError::Missing(id) => format!("Surface {surface:?}: output {id} is missing; nothing substituted"),
        SelectionError::Ambiguous(id) => format!("Surface {surface:?}: output {id} has no unique device name; give the endpoint a unique OS name and refresh"),
    })
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
    crate::simmath::sin(frame as f32 * 440.0 * std::f32::consts::TAU / rate as f32)
        * 0.04
        * envelope
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
#[path = "media_output_tests.rs"]
mod tests;
