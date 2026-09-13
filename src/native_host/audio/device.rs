//! CPAL owns one room stream on its worker thread. Callbacks only mix PCM;
//! enumeration, decoding, persistence and stream destruction happen elsewhere.
use super::{engine::Mixer, player::RoomPlayer, Control, NativeAudioState, OutputChoice};
use crate::native_host::media_output::OutputDevices;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub(super) fn run(
    control: Arc<Mutex<Control>>,
    state: Arc<Mutex<NativeAudioState>>,
    mut player: RoomPlayer,
) {
    let mut stream = None;
    let mut last_retry = u64::MAX;
    let mut last_test = 0;
    let mut scan_at = Instant::now() - Duration::from_secs(2);
    let failed = Arc::new(AtomicBool::new(false));
    let mut route = None;
    loop {
        let request = control.lock().unwrap().clone();
        if request.quit {
            break;
        }
        let changed = request.retry != last_retry;
        if changed {
            stream = None;
            player.reset_output();
            player.retry();
            last_retry = request.retry;
            route = request.output.clone();
            failed.store(false, Ordering::Release);
        }
        if changed || scan_at.elapsed() >= Duration::from_secs(1) {
            scan_at = Instant::now();
            match OutputDevices::scan() {
                Ok(devices) => {
                    state.lock().unwrap().devices = devices
                        .discovered
                        .iter()
                        .enumerate()
                        .map(|(i, device)| OutputChoice {
                            id: device.identity.to_string(),
                            label: device
                                .name
                                .clone()
                                .unwrap_or_else(|| device.identity.to_string()),
                            available: !devices.ambiguous[i],
                        })
                        .collect();
                    let selected = route.as_ref().and_then(|id| {
                        devices
                            .discovered
                            .iter()
                            .position(|d| d.identity.as_str() == id)
                    });
                    let bad = match (&route, selected) {
                        (Some(_), None) => Some("settings.audio.selected_missing"),
                        (Some(_), Some(index)) if devices.ambiguous[index] => {
                            Some("settings.audio.selected_ambiguous")
                        }
                        _ => None,
                    };
                    if let Some(error) = bad {
                        stream = None;
                        player.reset_output();
                        let mut status = state.lock().unwrap();
                        status.status = "failed";
                        status.detail = error.into();
                    } else if changed && request.routing_error.is_none() {
                        let device = match selected {
                            Some(index) => Some(devices.handles[index].clone()),
                            None => cpal::default_host().default_output_device(),
                        };
                        let opened = device
                            .ok_or_else(|| "settings.audio.system_missing".to_string())
                            .and_then(|device| open(&device, player.mixer.clone(), failed.clone()));
                        let mut status = state.lock().unwrap();
                        match opened {
                            Ok(opened) => {
                                stream = Some(opened);
                                status.status = "playing";
                                status.detail.clear();
                            }
                            Err(error) => {
                                status.status = "failed";
                                status.detail = error;
                            }
                        }
                    }
                }
                Err(error) => {
                    stream = None;
                    player.reset_output();
                    let mut status = state.lock().unwrap();
                    status.status = "failed";
                    status.detail = error;
                }
            }
        }
        if failed.swap(false, Ordering::AcqRel) {
            stream = None;
            player.reset_output();
            let mut status = state.lock().unwrap();
            status.status = "failed";
            status.detail = "settings.audio.device_stopped".into();
        }
        // File IO and decoding never hold the callback's mixer lock. A newer
        // continuation/output request invalidates the prepared old state.
        player.prepare(&request.input);
        let prepared_blaster = request.blaster.and_then(|(at, position)| {
            (stream.is_some() && at.elapsed() <= Duration::from_millis(250))
                .then(|| {
                    player.prepare_blaster(
                        &request.input,
                        position,
                        Some(at + Duration::from_millis(250)),
                    )
                })
                .flatten()
        });
        let prepared_computer = request.computer.as_ref().and_then(|(at, severity)| {
            (stream.is_some() && at.elapsed() <= Duration::from_millis(250))
                .then(|| player.prepare_computer(&request.input, severity))
                .flatten()
        });
        let mut fresh = control.lock().unwrap();
        if fresh.quit {
            break;
        }
        if fresh.input != request.input
            || fresh.retry != request.retry
            || fresh.blaster != request.blaster
            || fresh.computer != request.computer
        {
            continue;
        }
        let ready = stream.is_some();
        if fresh
            .alert_at
            .is_none_or(|time| time.elapsed() > Duration::from_millis(250))
        {
            player.suppress_edges();
        }
        player.apply(&fresh.input, ready);
        if let Some((at, _)) = fresh.blaster.take() {
            if at.elapsed() <= Duration::from_millis(250) {
                if let Some(prepared) = prepared_blaster {
                    player.play_blaster(&fresh.input, ready, prepared);
                }
            }
        }
        if fresh.take_computer(Instant::now()).is_some() {
            if let Some(prepared) = prepared_computer {
                player.play_computer(&fresh.input, ready, prepared);
            }
        }
        if fresh.test != last_test {
            last_test = fresh.test;
            if fresh
                .test_at
                .is_some_and(|time| time.elapsed() < Duration::from_secs(2))
            {
                player.test_output(ready);
            }
        }
        let routing_error = fresh.routing_error.clone();
        drop(fresh);
        {
            let mut status = state.lock().unwrap();
            status.test = if player.mixer.lock().unwrap().active("test") {
                "playing"
            } else {
                "idle"
            };
            status.asset_failures = player.failures.clone();
            if let Some(error) = routing_error {
                status.status = "failed";
                status.detail = error;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    player.reset_output();
    drop(stream);
}

fn open(
    device: &cpal::Device,
    mixer: Arc<Mutex<Mixer>>,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let stream = match config.sample_format() {
        cpal::SampleFormat::I8 => build::<i8>(device, &config.into(), mixer, failed),
        cpal::SampleFormat::I16 => build::<i16>(device, &config.into(), mixer, failed),
        cpal::SampleFormat::I32 => build::<i32>(device, &config.into(), mixer, failed),
        cpal::SampleFormat::I64 => build::<i64>(device, &config.into(), mixer, failed),
        cpal::SampleFormat::U8 => build::<u8>(device, &config.into(), mixer, failed),
        cpal::SampleFormat::U16 => build::<u16>(device, &config.into(), mixer, failed),
        cpal::SampleFormat::U32 => build::<u32>(device, &config.into(), mixer, failed),
        cpal::SampleFormat::U64 => build::<u64>(device, &config.into(), mixer, failed),
        cpal::SampleFormat::F32 => build::<f32>(device, &config.into(), mixer, failed),
        cpal::SampleFormat::F64 => build::<f64>(device, &config.into(), mixer, failed),
        _ => return Err("Unsupported output sample format".into()),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}
fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mixer: Arc<Mutex<Mixer>>,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let rate = config.sample_rate.0;
    let channels = usize::from(config.channels);
    // Fixed callback scratch space, chunked for any device buffer size.
    let mut scratch = vec![0.0; channels * 2048];
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                if let Ok(mut mixer) = mixer.try_lock() {
                    for chunk in output.chunks_mut(scratch.len()) {
                        mixer.render(&mut scratch[..chunk.len()], rate, channels);
                        for (target, source) in chunk.iter_mut().zip(&scratch) {
                            *target = T::from_sample(*source);
                        }
                    }
                } else {
                    output.fill(T::from_sample(0.0));
                }
            },
            move |_| {
                failed.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|e| e.to_string())
}
