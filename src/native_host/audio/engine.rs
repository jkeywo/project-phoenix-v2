//! The application PCM path, used by both CPAL and the hardware-free sink tests.
use super::{decoder::Pcm, mix::AudioMix};
use std::{collections::BTreeMap, sync::Arc};

struct Voice {
    pcm: Arc<Pcm>,
    category: &'static str,
    gain: f32,
    cursor: f64,
    looping: bool,
    remaining: Option<f64>,
    spatial: Option<super::spatial::StereoMatrix>,
}
#[derive(Default)]
pub struct Mixer {
    pub mix: AudioMix,
    mono: bool,
    voices: BTreeMap<String, Voice>,
}
impl Mixer {
    pub fn set_mono(&mut self, enabled: bool) {
        self.mono = enabled;
    }
    pub fn set_mix(&mut self, mix: AudioMix) {
        self.mix = mix.sanitised();
        self.voices
            .retain(|_, voice| voice.looping || self.mix.gain(voice.category, voice.gain) > 0.0);
    }
    pub fn stop_all(&mut self) {
        self.voices.clear();
    }
    pub fn stop(&mut self, id: &str) {
        self.voices.remove(id);
    }
    pub fn active(&self, id: &str) -> bool {
        self.voices.contains_key(id)
    }
    pub fn set_loop(&mut self, id: &str, pcm: Arc<Pcm>, category: &'static str, gain: f32) {
        if let Some(voice) = self.voices.get_mut(id) {
            voice.gain = gain;
            return;
        }
        self.voices.insert(
            id.into(),
            Voice {
                pcm,
                category,
                gain,
                cursor: 0.0,
                looping: true,
                remaining: None,
                spatial: None,
            },
        );
    }
    pub fn cue(
        &mut self,
        id: &str,
        pcm: Arc<Pcm>,
        category: &'static str,
        gain: f32,
        duration: Option<f64>,
    ) {
        if self.mix.gain(category, gain) == 0.0 {
            return;
        }
        // One current occurrence per cue key; replacements coalesce, never queue.
        self.voices.insert(
            id.into(),
            Voice {
                pcm,
                category,
                gain,
                cursor: 0.0,
                looping: false,
                remaining: duration,
                spatial: None,
            },
        );
    }
    pub fn cue_spatial(
        &mut self,
        id: &str,
        pcm: Arc<Pcm>,
        gain: f32,
        matrix: super::spatial::StereoMatrix,
    ) {
        self.cue(id, pcm, "effects", gain, None);
        if let Some(voice) = self.voices.get_mut(id) {
            voice.spatial = Some(matrix);
        }
    }
    /// Fill interleaved device frames directly. Linear resampling preserves the
    /// authored speed on outputs whose negotiated rate differs from an asset.
    pub fn render(&mut self, output: &mut [f32], rate: u32, channels: usize) {
        output.fill(0.0);
        if rate == 0 || channels == 0 {
            return;
        }
        for voice in self.voices.values_mut() {
            let gain = self.mix.gain(voice.category, voice.gain);
            for frame in output.chunks_exact_mut(channels) {
                let count = voice.pcm.frames();
                if voice.cursor >= count as f64 {
                    if voice.looping {
                        voice.cursor %= count as f64;
                    } else {
                        break;
                    }
                }
                if voice.remaining.is_some_and(|remaining| remaining <= 0.0) {
                    break;
                }
                let base = voice.cursor as usize;
                let next = if base + 1 < count {
                    base + 1
                } else if voice.looping {
                    0
                } else {
                    base
                };
                let fraction = (voice.cursor - base as f64) as f32;
                let sample = |channel: usize| {
                    let channel = channel.min(voice.pcm.channels - 1);
                    let a = voice.pcm.samples[base * voice.pcm.channels + channel];
                    let b = voice.pcm.samples[next * voice.pcm.channels + channel];
                    a + (b - a) * fraction
                };
                let mut stereo = voice.spatial.map_or_else(
                    || [sample(0), sample(1)],
                    |matrix| matrix.apply(sample(0), sample(1)),
                );
                // Convert the spatialized signal before the authored/category/
                // Master gains. Changing this flag never restarts a voice.
                if self.mono {
                    stereo = [(stereo[0] + stereo[1]) * 0.5; 2];
                }
                if channels == 1 {
                    frame[0] += (stereo[0] + stereo[1]) * 0.5 * gain;
                } else {
                    frame[0] += stereo[0] * gain;
                    frame[1] += stereo[1] * gain;
                }
                voice.cursor += f64::from(voice.pcm.rate) / f64::from(rate);
                if let Some(remaining) = &mut voice.remaining {
                    *remaining -= 1.0 / f64::from(rate);
                }
            }
        }
        self.voices.retain(|_, voice| {
            (voice.looping || voice.cursor < voice.pcm.frames() as f64)
                && !voice.remaining.is_some_and(|remaining| remaining <= 0.0)
        });
        for sample in output {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }
}
