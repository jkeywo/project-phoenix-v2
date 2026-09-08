//! Explicit local microphone level test. Samples are reduced to a peak value
//! inside the callback and discarded; no audio is retained or transmitted.
use super::{
    bridge_media::{self, DeviceAvailability, DiscoveredMediaDevice, MediaKind, RawMediaDevice},
    bridge_profile::BridgeProfile,
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU32, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

pub struct Microphones {
    pub discovered: Vec<DiscoveredMediaDevice>,
    handles: Vec<cpal::Device>,
    ambiguous: Vec<bool>,
}
impl Microphones {
    pub fn scan() -> Result<Self, String> {
        let handles: Vec<_> = cpal::default_host()
            .input_devices()
            .map_err(|e| e.to_string())?
            .collect();
        let raws: Vec<_> = handles
            .iter()
            .map(|device| RawMediaDevice {
                kind: MediaKind::Microphone,
                name: device.name().ok(),
                hardware_id: None,
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
    pub fn meter_surface(&self, profile: &BridgeProfile, surface: &str) -> Result<(), String> {
        let indices = selected_microphones(profile, surface, &self.discovered, &self.ambiguous)?;
        for index in indices {
            let id = &self.discovered[index].identity;
            println!(
                "Surface {surface:?}: metering {id} for five seconds; samples are not retained"
            );
            meter(&self.handles[index]).map_err(|error| {
                format!("Surface {surface:?}: {id}: {error}; no other device substituted")
            })?;
            println!("Surface {surface:?}: {id}: meter stopped; stream closed");
        }
        Ok(())
    }
}

fn selected_microphones(
    profile: &BridgeProfile,
    surface: &str,
    discovered: &[DiscoveredMediaDevice],
    ambiguous: &[bool],
) -> Result<Vec<usize>, String> {
    let media = bridge_media::validate_media(&profile.media).map_err(|e| e.to_string())?;
    let assigned = media
        .surfaces
        .iter()
        .find(|entry| entry.surface == surface)
        .ok_or("unknown microphone surface")?;
    if assigned.microphones.is_empty() {
        return Err("surface has no assigned microphones".into());
    }
    let indices: Vec<_> = assigned
        .microphones
        .iter()
        .map(|id| {
            let index = discovered
                .iter()
                .position(|device| device.identity == *id)
                .ok_or_else(|| format!("microphone {id} is missing; nothing substituted"))?;
            if ambiguous[index] {
                return Err(format!(
                    "microphone {id} has no unique OS name; rename and refresh before testing"
                ));
            }
            Ok(index)
        })
        .collect::<Result<_, String>>()?;
    Ok(indices)
}

fn meter(device: &cpal::Device) -> Result<(), String> {
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    macro_rules! meter {
        ($sample:ty) => {
            meter_samples::<$sample>(device, &config.into())
        };
    }
    match config.sample_format() {
        cpal::SampleFormat::I8 => meter!(i8),
        cpal::SampleFormat::I16 => meter!(i16),
        cpal::SampleFormat::I32 => meter!(i32),
        cpal::SampleFormat::I64 => meter!(i64),
        cpal::SampleFormat::U8 => meter!(u8),
        cpal::SampleFormat::U16 => meter!(u16),
        cpal::SampleFormat::U32 => meter!(u32),
        cpal::SampleFormat::U64 => meter!(u64),
        cpal::SampleFormat::F32 => meter!(f32),
        cpal::SampleFormat::F64 => meter!(f64),
        format => Err(format!("unsupported microphone sample format {format}")),
    }
}
fn finite_level(value: f32) -> f32 {
    if value.is_finite() {
        value.abs().min(1.0)
    } else {
        0.0
    }
}
fn meter_samples<T: cpal::SizedSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
) -> Result<(), String>
where
    f32: cpal::FromSample<T>,
{
    let peak = Arc::new(AtomicU32::new(0));
    let received = Arc::new(AtomicU32::new(0));
    let samples_peak = peak.clone();
    let samples_received = received.clone();
    let (failed, errors) = mpsc::sync_channel(1);
    let stream = device
        .build_input_stream(
            config,
            move |samples: &[T], _| {
                let level = samples
                    .iter()
                    .map(|sample| finite_level((*sample).to_sample::<f32>()))
                    .fold(0.0f32, f32::max);
                samples_peak.fetch_max(level.to_bits(), Ordering::Relaxed);
                if !samples.is_empty() {
                    samples_received.store(1, Ordering::Release);
                }
            },
            move |error| {
                let _ = failed.try_send(error.to_string());
            },
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut last_samples = started;
    while started.elapsed() < Duration::from_secs(5) {
        match errors.recv_timeout(Duration::from_millis(200)) {
            Ok(error) => return Err(error),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("microphone callback stopped".into())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if received.swap(0, Ordering::Acquire) != 0 {
            last_samples = Instant::now();
        }
        if last_samples.elapsed() > Duration::from_secs(1) {
            return Err("microphone stopped delivering samples; it may be unavailable".into());
        }
        println!(
            "  microphone peak: {:.0}%",
            f32::from_bits(peak.swap(0, Ordering::Relaxed)) * 100.0
        );
    }
    drop(stream);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn microphone_preflight_resolves_only_the_named_surface_and_all_members() {
        let profile: BridgeProfile = toml::from_str(
            r#"version = 1
[[media]]
surface = "comms"
microphone = ["mic:Headset", "mic:Desk"]
"#,
        )
        .unwrap();
        let devices =
            bridge_media::identify_media(&["Headset", "Desk"].map(|name| RawMediaDevice {
                kind: MediaKind::Microphone,
                name: Some(name.into()),
                hardware_id: None,
                default: false,
                availability: DeviceAvailability::Available,
            }));
        assert_eq!(
            selected_microphones(&profile, "comms", &devices, &[false, false]).unwrap(),
            vec![0, 1]
        );
        assert!(
            selected_microphones(&profile, "comms", &devices[..1], &[false])
                .unwrap_err()
                .contains("missing")
        );
        assert!(
            selected_microphones(&profile, "comms", &devices, &[false, true])
                .unwrap_err()
                .contains("unique")
        );
        assert!(selected_microphones(&profile, "viewscreen", &devices, &[false, false]).is_err());
    }
    #[test]
    fn metering_clamps_signal_without_retaining_or_inventing_invalid_levels() {
        assert_eq!(finite_level(-0.75), 0.75);
        assert_eq!(finite_level(2.0), 1.0);
        assert_eq!(finite_level(f32::NAN), 0.0);
        assert_eq!(finite_level(f32::INFINITY), 0.0);
    }
}
