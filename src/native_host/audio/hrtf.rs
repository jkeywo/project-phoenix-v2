//! Fixed-position measured HRTF convolution, outside the realtime device path.
//! Public-domain WebKit/IRCAM sample provenance: assets/audio/hrtf/NOTICE.md.
use super::decoder::Pcm;
use std::{sync::Arc, time::Instant};

const DATA: &[u8] = include_bytes!("../../../assets/audio/hrtf/Composite.wav");
const RATE: f32 = 44_100.0;
const TAPS: usize = 256;

fn coefficient(azimuth: usize, elevation: i32, frame: usize, channel: usize) -> f32 {
    let elevation_index = if elevation < 0 {
        (elevation + 150) / 15
    } else {
        elevation / 15
    } as usize;
    let response = (azimuth % 24) * 10 + elevation_index;
    let at = 44 + ((response * TAPS + frame) * 2 + channel) * 2;
    f32::from(i16::from_le_bytes([DATA[at], DATA[at + 1]])) / 32768.0
}

pub fn impulse(position: [f32; 3], rate: u32) -> Vec<[f32; 2]> {
    let [x, y, z] = position;
    let azimuth = if x == 0.0 && z == 0.0 {
        0.0
    } else {
        // IRCAM's database azimuth is reversed from the listener/panner angle.
        (-crate::simmath::atan2(x, -z).to_degrees() + 360.0) % 360.0 / 15.0
    };
    let elevation = crate::simmath::atan2(y, crate::simmath::hypot(x, z))
        .to_degrees()
        .clamp(-45.0, 90.0);
    let lower = (elevation / 15.0).floor() as i32 * 15;
    let upper = (lower + 15).min(90);
    let blend_elevation = (elevation - lower as f32) / 15.0;
    let blend_azimuth = azimuth.fract();
    let at_azimuth = azimuth as usize;
    let interpolate = |frame, channel| {
        let at = |angle, elev| coefficient(angle, elev, frame, channel);
        let blend = |a, b, weight| a + (b - a) * weight;
        blend(
            blend(
                at(at_azimuth, lower),
                at(at_azimuth + 1, lower),
                blend_azimuth,
            ),
            blend(
                at(at_azimuth, upper),
                at(at_azimuth + 1, upper),
                blend_azimuth,
            ),
            blend_elevation,
        )
    };
    // Keep filter duration and DC gain when the authored asset uses another
    // sample rate. The ordinary device renderer resamples the resulting sound.
    let count = (TAPS as f32 * rate as f32 / RATE).ceil() as usize;
    (0..count)
        .map(|frame| {
            let source = frame as f32 * RATE / rate as f32;
            let first = (source as usize).min(TAPS - 1);
            let next = (first + 1).min(TAPS - 1);
            let weight = source.fract();
            std::array::from_fn(|channel| {
                let a = interpolate(first, channel);
                let b = interpolate(next, channel);
                (a + (b - a) * weight) * RATE / rate as f32
            })
        })
        .collect()
}

/// A stale one-shot is discarded during expensive preparation as well as at
/// commit. This prevents HRTF work from turning missed sounds into a backlog.
pub fn render(pcm: &Pcm, position: [f32; 3], deadline: Option<Instant>) -> Option<Arc<Pcm>> {
    // Never decimate the measured FIR: its high-frequency response aliases
    // into the audible band at low source rates (the shipped blaster is 24k).
    // Process those sources at the database rate, as a browser context does.
    let rate = pcm.rate.max(RATE as u32);
    let count = (pcm.frames() as u64 * u64::from(rate)).div_ceil(u64::from(pcm.rate)) as usize;
    let impulse = impulse(position, rate);
    let mut samples = vec![0.0; (count + impulse.len() - 1) * 2];
    for frame in 0..count {
        if frame % 256 == 0 && deadline.is_some_and(|deadline| Instant::now() > deadline) {
            return None;
        }
        let at = frame as f64 * f64::from(pcm.rate) / f64::from(rate);
        let first = (at as usize).min(pcm.frames() - 1);
        let next = (first + 1).min(pcm.frames() - 1);
        let sample = |channel| {
            let a = pcm.samples[first * pcm.channels + channel];
            let b = pcm.samples[next * pcm.channels + channel];
            a + (b - a) * at.fract() as f32
        };
        let left = sample(0);
        let right = sample(pcm.channels - 1);
        for (tap, response) in impulse.iter().enumerate() {
            samples[(frame + tap) * 2] += left * response[0];
            samples[(frame + tap) * 2 + 1] += right * response[1];
        }
    }
    Some(Arc::new(Pcm {
        samples,
        rate,
        channels: 2,
    }))
}
