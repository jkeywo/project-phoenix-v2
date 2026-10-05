#[test]
fn authored_pcm_wav_is_decoded_by_the_production_decoder() {
    let samples = [0i16, 16_384, -16_384, 32_767, 0, -32_768, 0, 0];
    let data_size = (samples.len() * 2) as u32;
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + data_size).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes()); // PCM
    wav.extend(1u16.to_le_bytes()); // Mono
    wav.extend(8_000u32.to_le_bytes());
    wav.extend(16_000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(data_size.to_le_bytes());
    for sample in samples {
        wav.extend(sample.to_le_bytes());
    }
    let pcm = super::decode(wav.clone(), "wav").unwrap();
    assert_eq!(
        (pcm.rate, pcm.channels, pcm.frames()),
        (8_000, 1, samples.len())
    );
    assert!((pcm.samples[1] - 0.5).abs() < 0.0001);
    assert!((pcm.samples[2] + 0.5).abs() < 0.0001);
    assert!(super::decode(wav[..30].to_vec(), "wav").is_err());
}
