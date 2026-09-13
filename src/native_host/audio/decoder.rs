//! Production MP3/Vorbis decoder. No device, DOM, simulation or cue history.
use std::{io::Cursor, sync::Arc};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

#[derive(Debug)]
pub struct Pcm {
    pub samples: Vec<f32>,
    pub rate: u32,
    pub channels: usize,
}
impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }
}

pub fn decode(bytes: Vec<u8>, extension: &str) -> Result<Arc<Pcm>, String> {
    let mut hint = Hint::new();
    hint.with_extension(extension);
    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| e.to_string())?
        .format;
    let track = format.default_track().ok_or("No audio track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| e.to_string())?;
    let mut pcm = Pcm {
        samples: Vec::new(),
        rate: 0,
        channels: 0,
    };
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet).map_err(|e| e.to_string())?;
        let spec = *decoded.spec();
        if spec.rate == 0 || spec.channels.count() == 0 || spec.channels.count() > 2 {
            return Err("Only mono or stereo audio is supported".into());
        }
        if pcm.rate != 0 && (pcm.rate != spec.rate || pcm.channels != spec.channels.count()) {
            return Err("Audio format changes within the file".into());
        }
        pcm.rate = spec.rate;
        pcm.channels = spec.channels.count();
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        samples.copy_interleaved_ref(decoded);
        if samples.samples().iter().any(|sample| !sample.is_finite()) {
            return Err("Audio contains a non-finite sample".into());
        }
        // Bounded decoded asset memory: thirty minutes of stereo at 48 kHz is
        // already far beyond a shipped bed. A malformed stream cannot grow forever.
        if pcm.samples.len() + samples.len() > 48_000 * 2 * 60 * 30 {
            return Err("Decoded audio exceeds the supported asset size".into());
        }
        pcm.samples.extend_from_slice(samples.samples());
    }
    if pcm.channels == 0 || pcm.frames() == 0 {
        return Err("Empty audio file".into());
    }
    Ok(Arc::new(pcm))
}

pub fn read(path: &str) -> Result<Arc<Pcm>, String> {
    let path = std::path::Path::new(path);
    // Authored audio is content, never an arbitrary host-local path.
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        || !path.starts_with("assets")
    {
        return Err("Audio must name a content asset under assets/".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    decode(
        bytes,
        path.extension()
            .and_then(|x| x.to_str())
            .unwrap_or_default(),
    )
}
