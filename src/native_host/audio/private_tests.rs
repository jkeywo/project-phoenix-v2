use super::*;
use crate::native_host::{
    bridge_media::MediaSurfaceEntry,
    native_gm::bridge::NativeGmBridge,
    panes::{
        surface::{pump_pane, RecordingSurface},
        PaneBus, PaneIdentity,
    },
};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<FakeState>>);
#[derive(Default)]
struct FakeState {
    devices: Vec<(String, bool)>,
    opened: Vec<String>,
    mixers: BTreeMap<String, Vec<Arc<Mutex<Mixer>>>>,
    failed: Arc<AtomicBool>,
    denied: Option<String>,
}
impl Backend for Fake {
    type Stream = Arc<AtomicBool>;
    fn scan(&mut self) -> Result<Vec<(String, bool)>, String> {
        Ok(self.0.lock().unwrap().devices.clone())
    }
    fn open(&mut self, id: &str, mixer: Arc<Mutex<Mixer>>) -> Result<Self::Stream, String> {
        let mut state = self.0.lock().unwrap();
        state.opened.push(id.into());
        if state.denied.as_deref() == Some(id) {
            return Err("Output access denied".into());
        }
        state.mixers.entry(id.into()).or_default().push(mixer);
        Ok(state.failed.clone())
    }
    fn failed(&self, stream: &Self::Stream) -> bool {
        stream.load(Ordering::Acquire)
    }
}
impl Fake {
    fn with(ids: &[&str]) -> Self {
        let fake = Self::default();
        fake.0.lock().unwrap().devices = ids.iter().map(|id| (id.to_string(), false)).collect();
        fake
    }
    fn sink(&self, id: &str) -> Vec<f32> {
        let mut samples = vec![0.0; 44100 * 2];
        let state = self.0.lock().unwrap();
        for mixer in state.mixers.get(id).into_iter().flatten() {
            let mut output = vec![0.0; samples.len()];
            mixer.lock().unwrap().render(&mut output, 44100, 2);
            for (sample, next) in samples.iter_mut().zip(output) {
                *sample += next;
            }
        }
        assert!(samples.iter().all(|sample| sample.is_finite()));
        samples
    }
}
fn audible(samples: &[f32]) -> bool {
    samples.iter().any(|sample| sample.abs() > 0.00001)
}

#[test]
fn private_mono_reaches_actual_pcm_on_only_its_endpoint_and_survives_reopening_output() {
    let hub = PrivateAudio::new(profile());
    let key = Endpoint::Console(PaneId(1));
    hub.bind(key, "helm");
    let fake = Fake::with(&["output:Helm", "output:GM"]);
    let mut worker = Worker::new(fake.clone());
    worker.step(&hub);
    // Decode an asymmetric authored-shaped asset through production decoding;
    // the worker consumes that prepared asset through its ordinary cue path.
    let pcm = decoder::decode(
        include_bytes!("../../../tests/fixtures/audio-mono-right.wav").to_vec(),
        "wav",
    )
    .unwrap();
    worker
        .pcm
        .insert(worker.manifest["refused"].file.clone(), Ok(pcm));
    hub.submit(key, &record(&hub, key, REFUSED));
    worker.step(&hub);
    let stereo = fake.sink("output:Helm");
    assert!(audible(&stereo));
    assert!(stereo.chunks_exact(2).all(|pair| pair[0] == 0.0));
    let generation = hub.status(key).unwrap().generation;
    hub.submit(
        key,
        &format!("{{\"type\":\"NativePrivateAudio\",\"generation\":{generation},\"mono\":true}}"),
    );
    hub.submit(key, &record(&hub, key, REFUSED));
    worker.step(&hub);
    let output = fake.sink("output:Helm");
    assert!(audible(&output));
    assert!(output.chunks_exact(2).all(|pair| pair[0] == pair[1]));
    assert!(!audible(&fake.sink("output:GM")));
    hub.submit(
        key,
        &format!("{{\"type\":\"NativePrivateAudio\",\"generation\":{generation},\"retry\":true}}"),
    );
    worker.step(&hub);
    assert!(!audible(&fake.sink("output:Helm")));
    hub.submit(key, &record(&hub, key, REFUSED));
    worker.step(&hub);
    let output = fake.sink("output:Helm");
    assert!(audible(&output));
    assert!(output.chunks_exact(2).all(|pair| pair[0] == pair[1]));
    hub.submit(key,&format!("{{\"type\":\"NativePrivateAudio\",\"generation\":{generation},\"mix\":{{\"master\":{{\"level\":0,\"muted\":false}},\"alerts\":{{\"level\":1,\"muted\":false}},\"interface\":{{\"level\":1,\"muted\":false}}}}}}"));
    hub.submit(key, &record(&hub, key, REFUSED));
    worker.step(&hub);
    assert!(!audible(&fake.sink("output:Helm")));
}
fn media(surface: &str, outputs: &[&str]) -> MediaSurfaceEntry {
    MediaSurfaceEntry {
        surface: surface.into(),
        outputs: outputs.iter().map(|id| id.to_string()).collect(),
        camera: None,
        microphones: vec![],
        allow_shared: vec![],
    }
}
fn profile() -> BridgeProfile {
    let mut profile = BridgeProfile::empty();
    profile.media = vec![
        media("helm", &["output:Helm"]),
        media("native-gm", &["output:GM"]),
        media("viewscreen", &["output:Room"]),
    ];
    profile
}
fn record(hub: &PrivateAudio, key: Endpoint, fixture: &str) -> String {
    fixture
        .trim()
        .replace(
            "\"generation\":0",
            &format!("\"generation\":{}", hub.status(key).unwrap().generation),
        )
        .replace(
            "\"at_ms\":0",
            &format!(
                "\"at_ms\":{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis()
            ),
        )
}
const REFUSED: &str = include_str!("../../../tests/fixtures/native-private-refused.json");
const TEST: &str = include_str!("../../../tests/fixtures/native-private-test.json");

#[test]
fn both_private_adapters_apply_reduced_range_to_real_decoded_feedback_without_output_spill() {
    let samples = |enabled: bool| {
        let hub = PrivateAudio::new(profile());
        let fake = Fake::with(&["output:Helm", "output:GM", "output:Room"]);
        let mut worker = Worker::new(fake.clone());
        let bus = PaneBus::default();
        let pane = bus.open(PaneIdentity::adopt("range-private-token", "helm").unwrap());
        bus.attach_audio(hub.clone());
        let gm = NativeGmBridge::default();
        let gm_id = PaneId(99);
        gm.activate(gm_id);
        gm.attach_audio(hub.clone());
        worker.step(&hub);
        let mut surface = RecordingSurface::ready();
        let mut output = Vec::new();
        for (key, endpoint, other) in [
            (Endpoint::Console(pane), "output:Helm", "output:GM"),
            (Endpoint::Gm(gm_id), "output:GM", "output:Helm"),
        ] {
            let mut value: serde_json::Value =
                serde_json::from_str(&record(&hub, key, REFUSED)).unwrap();
            value["reducedRange"] = enabled.into();
            surface.queue_record(serde_json::to_string(&value).unwrap());
            if key == Endpoint::Console(pane) {
                let result = pump_pane(&bus, pane, &mut surface);
                assert_eq!(result.accepted, 0);
                assert!(result.refusals.is_empty());
            } else {
                gm.pump(gm_id, &mut surface);
                assert!(gm.take_records().is_empty());
            }
            worker.step(&hub);
            let current = fake.sink(endpoint);
            assert!(audible(&current));
            assert!(!audible(&fake.sink(other)));
            assert!(!audible(&fake.sink("output:Room")));
            output.push(current);
            // A lost device retires the occurrence, never sends it to room output.
            fake.0
                .lock()
                .unwrap()
                .devices
                .retain(|(id, _)| id != endpoint);
            worker.step(&hub);
            assert!(!audible(&fake.sink(endpoint)));
        }
        output
    };
    let full = samples(false);
    let reduced = samples(true);
    for (full, reduced) in full.iter().zip(&reduced) {
        let energy = |samples: &[f32]| samples.iter().map(|sample| sample * sample).sum::<f32>();
        assert!(energy(reduced) > energy(full) * 1.1);
        assert!(reduced
            .iter()
            .all(|sample| sample.abs() <= super::super::range::RangeSpec::default().ceiling));
    }
}

#[test]
fn both_actual_native_adapters_decode_shared_action_fixture_to_only_their_assigned_samples() {
    // The browser smoke captures these bytes after actual initConsole and
    // mountNativeGmWorkspace correlated Refused actions with visible results.
    let hub = PrivateAudio::new(profile());
    let fake = Fake::with(&["output:Helm", "output:GM", "output:Room"]);
    let mut worker = Worker::new(fake.clone());
    let bus = PaneBus::default();
    let pane = bus.open(PaneIdentity::adopt("ordinary-private-token", "helm").unwrap());
    bus.attach_audio(hub.clone());
    let gm = NativeGmBridge::default();
    let gm_id = PaneId(99);
    gm.activate(gm_id);
    gm.attach_audio(hub.clone());
    worker.step(&hub);
    let mut surface = RecordingSurface::ready();
    surface.queue_record(record(&hub, Endpoint::Console(pane), REFUSED));
    let result = pump_pane(&bus, pane, &mut surface);
    assert_eq!(result.accepted, 0);
    assert!(result.refusals.is_empty());
    worker.step(&hub);
    assert!(audible(&fake.sink("output:Helm")));
    assert!(!audible(&fake.sink("output:GM")));
    assert!(!audible(&fake.sink("output:Room")));
    surface.queue_record(record(&hub, Endpoint::Gm(gm_id), REFUSED));
    gm.pump(gm_id, &mut surface);
    worker.step(&hub);
    assert!(gm.take_records().is_empty());
    assert!(audible(&fake.sink("output:GM")));
    assert!(!audible(&fake.sink("output:Helm")));
    worker.step(&hub);
    assert!(!audible(&fake.sink("output:GM")));
    // Actual WAV and OGG bytes traverse the same production decoder and sink.
    surface.queue_record(record(&hub, Endpoint::Gm(gm_id), TEST));
    gm.pump(gm_id, &mut surface);
    worker.step(&hub);
    assert!(audible(&fake.sink("output:GM")));
    worker.step(&hub);
    assert_eq!(hub.status(Endpoint::Gm(gm_id)).unwrap().test, "idle");
    assert!(fake
        .0
        .lock()
        .unwrap()
        .opened
        .iter()
        .all(|id| id != "output:Room"));
}

#[test]
fn missing_lost_ambiguous_and_unconsented_private_outputs_are_silent_without_fallback() {
    let hub = PrivateAudio::new(BridgeProfile::empty());
    let key = Endpoint::Console(PaneId(1));
    hub.bind(key, "helm");
    let fake = Fake::with(&["output:Helm", "output:Room"]);
    let mut worker = Worker::new(fake.clone());
    worker.step(&hub);
    assert_eq!(
        hub.status(key).unwrap().detail,
        "settings.audio.private_unassigned"
    );
    assert!(fake.0.lock().unwrap().opened.is_empty());
    hub.profile(profile());
    worker.step(&hub);
    hub.submit(key, &record(&hub, key, REFUSED));
    worker.step(&hub);
    // Disconnect stops the already-playing PCM too.
    fake.0
        .lock()
        .unwrap()
        .devices
        .retain(|(id, _)| id != "output:Helm");
    worker.step(&hub);
    assert!(!audible(&fake.sink("output:Helm")));
    assert_eq!(
        hub.status(key).unwrap().detail,
        "settings.audio.selected_missing"
    );
    hub.submit(key, &record(&hub, key, REFUSED));
    fake.0
        .lock()
        .unwrap()
        .devices
        .push(("output:Helm".into(), false));
    worker.step(&hub);
    assert_eq!(hub.status(key).unwrap().status, "failed");
    let retry = format!(
        "{{\"type\":\"NativePrivateAudio\",\"generation\":{},\"retry\":true}}",
        hub.status(key).unwrap().generation
    );
    hub.submit(key, &retry);
    worker.step(&hub);
    assert_eq!(hub.status(key).unwrap().status, "playing");
    assert!(!audible(&fake.sink("output:Helm")));
    fake.0
        .lock()
        .unwrap()
        .devices
        .iter_mut()
        .find(|(id, _)| id == "output:Helm")
        .unwrap()
        .1 = true;
    worker.step(&hub);
    assert_eq!(
        hub.status(key).unwrap().detail,
        "settings.audio.selected_ambiguous"
    );
    let mut shared = profile();
    shared.media[2].outputs = vec!["output:Helm".into()];
    hub.profile(shared);
    worker.step(&hub);
    assert_eq!(
        hub.status(key).unwrap().detail,
        "settings.audio.private_invalid_assignment"
    );
    assert!(!audible(&fake.sink("output:Room")));
}

#[test]
fn private_generations_mute_rebuild_and_failed_device_discard_old_voices_and_test_requests() {
    let hub = PrivateAudio::new(profile());
    let fake = Fake::with(&["output:Helm", "output:GM"]);
    let mut worker = Worker::new(fake.clone());
    let bus = PaneBus::default();
    let pane = bus.open(PaneIdentity::adopt("private-rebuild-token", "helm").unwrap());
    bus.attach_audio(hub.clone());
    worker.step(&hub);
    let key = Endpoint::Console(pane);
    let old = record(&hub, key, REFUSED);
    hub.submit(key, &old);
    worker.step(&hub);
    hub.continuation(10);
    assert!(!audible(&fake.sink("output:Helm")));
    hub.submit(key, &old);
    worker.step(&hub);
    assert!(!audible(&fake.sink("output:Helm")));
    for bus_name in ["master", "interface"] {
        hub.submit(key, &record(&hub, key, TEST));
        worker.step(&hub);
        let mix = format!(
            "{{\"master\":{{\"level\":1,\"muted\":{}}},\"alerts\":{{\"level\":1,\"muted\":false}},\"interface\":{{\"level\":1,\"muted\":{}}}}}",
            bus_name == "master",
            bus_name == "interface"
        );
        hub.submit(
            key,
            &format!(
                "{{\"type\":\"NativePrivateAudio\",\"generation\":{},\"mix\":{mix}}}",
                hub.status(key).unwrap().generation
            ),
        );
        assert!(!audible(&fake.sink("output:Helm")));
        hub.submit(key, &record(&hub, key, TEST));
        worker.step(&hub);
        assert!(!audible(&fake.sink("output:Helm")));
    }
    let old = record(&hub, key, REFUSED);
    let replacement = bus.rebuild(pane).unwrap().0;
    worker.step(&hub);
    let mut surface = RecordingSurface::ready();
    surface.queue_record(old);
    pump_pane(&bus, pane, &mut surface);
    worker.step(&hub);
    assert!(!audible(&fake.sink("output:Helm")));
    let next = Endpoint::Console(replacement);
    hub.submit(next, &record(&hub, next, REFUSED));
    worker.step(&hub);
    fake.0.lock().unwrap().failed.store(true, Ordering::Release);
    worker.step(&hub);
    assert!(!audible(&fake.sink("output:Helm")));
    assert_eq!(
        hub.status(next).unwrap().detail,
        "settings.audio.device_stopped"
    );
}

#[test]
fn explicit_multiple_outputs_have_independent_cursors_and_identity_collisions_stay_silent() {
    let mut config = profile();
    config.media[0].outputs.push("output:Second".into());
    let hub = PrivateAudio::new(config);
    let key = Endpoint::Console(PaneId(1));
    hub.bind(key, "helm");
    let fake = Fake::with(&["output:Helm", "output:Second", "output:GM"]);
    let mut worker = Worker::new(fake.clone());
    worker.step(&hub);
    hub.submit(key, &record(&hub, key, REFUSED));
    worker.step(&hub);
    let a = fake.sink("output:Helm");
    let b = fake.sink("output:Second");
    assert!(audible(&a));
    assert_eq!(a, b);
    hub.bind(Endpoint::Console(PaneId(2)), "helm");
    worker.step(&hub);
    assert_eq!(
        hub.status(key).unwrap().detail,
        "settings.audio.private_surface_collision"
    );
    hub.bind(Endpoint::Console(PaneId(3)), "native-gm");
    hub.bind(Endpoint::Gm(PaneId(3)), "native-gm");
    worker.step(&hub);
    assert_eq!(
        hub.status(Endpoint::Gm(PaneId(3))).unwrap().status,
        "failed"
    );
    hub.close(Endpoint::Console(PaneId(3)));
    worker.step(&hub);
    assert_eq!(
        hub.status(Endpoint::Gm(PaneId(3))).unwrap().status,
        "playing"
    );
}

#[test]
fn resolved_default_room_output_is_only_a_consent_check_never_a_private_fallback() {
    let mut config = profile();
    config.media[2].outputs.clear();
    let hub = PrivateAudio::new(config.clone());
    let key = Endpoint::Console(PaneId(1));
    hub.bind(key, "helm");
    let fake = Fake::with(&["output:Helm", "output:Room"]);
    let mut worker = Worker::new(fake.clone());
    worker.step(&hub);
    assert_eq!(
        hub.status(key).unwrap().detail,
        "settings.audio.private_room_unresolved"
    );
    assert!(fake.0.lock().unwrap().opened.is_empty());
    hub.submit(key, &record(&hub, key, REFUSED));
    hub.room_output(Some("output:Room".into()));
    worker.step(&hub);
    assert_eq!(hub.status(key).unwrap().status, "playing");
    assert!(!audible(&fake.sink("output:Helm")));
    hub.submit(key, &record(&hub, key, REFUSED));
    worker.step(&hub);
    hub.room_output(Some("output:Helm".into()));
    worker.step(&hub);
    assert_eq!(
        hub.status(key).unwrap().detail,
        "settings.audio.private_invalid_assignment"
    );
    assert!(!audible(&fake.sink("output:Helm")));
    config.media[0].allow_shared.push("output:Helm".into());
    config.media[2].allow_shared.push("output:Helm".into());
    hub.profile(config);
    worker.step(&hub);
    assert_eq!(hub.status(key).unwrap().status, "playing");
    assert!(!audible(&fake.sink("output:Helm")));
    hub.room_output(None);
    worker.step(&hub);
    assert_eq!(
        hub.status(key).unwrap().detail,
        "settings.audio.private_room_unresolved"
    );
    assert!(!audible(&fake.sink("output:Room")));
}

#[test]
fn explicit_shared_device_does_not_duplicate_one_operators_cue_and_denial_does_not_fallback() {
    let mut config = profile();
    config.media[1].outputs = vec!["output:Helm".into()];
    for entry in &mut config.media[..2] {
        entry.allow_shared = vec!["output:Helm".into()];
    }
    let hub = PrivateAudio::new(config);
    let console = Endpoint::Console(PaneId(1));
    let gm = Endpoint::Gm(PaneId(2));
    hub.bind(console, "helm");
    hub.bind(gm, "native-gm");
    let fake = Fake::with(&["output:Helm", "output:Room"]);
    let mut worker = Worker::new(fake.clone());
    worker.step(&hub);
    hub.submit(console, &record(&hub, console, REFUSED));
    worker.step(&hub);
    let shared = fake.sink("output:Helm");
    assert!(audible(&shared));
    hub.close(gm);
    worker.step(&hub);
    hub.submit(console, &record(&hub, console, REFUSED));
    worker.step(&hub);
    assert_eq!(shared, fake.sink("output:Helm"));
    hub.close(console);
    worker.step(&hub);
    fake.0.lock().unwrap().denied = Some("output:Helm".into());
    hub.bind(Endpoint::Console(PaneId(3)), "helm");
    worker.step(&hub);
    assert_eq!(
        hub.status(Endpoint::Console(PaneId(3))).unwrap().status,
        "failed"
    );
    assert!(!audible(&fake.sink("output:Room")));
}

#[test]
fn expired_or_muted_then_reenabled_private_slot_is_consumed_without_catchup() {
    let hub = PrivateAudio::new(profile());
    let key = Endpoint::Console(PaneId(1));
    hub.bind(key, "helm");
    let fake = Fake::with(&["output:Helm"]);
    let mut worker = Worker::new(fake.clone());
    worker.step(&hub);
    hub.submit(key, &record(&hub, key, REFUSED));
    hub.0
        .lock()
        .unwrap()
        .entries
        .get_mut(&key)
        .unwrap()
        .pending
        .as_mut()
        .unwrap()
        .0 = Instant::now() - Duration::from_secs(1);
    worker.step(&hub);
    assert!(!audible(&fake.sink("output:Helm")));
    hub.submit(key, &record(&hub, key, REFUSED));
    let generation = hub.status(key).unwrap().generation;
    for muted in [true, false] {
        hub.submit(key,&format!("{{\"type\":\"NativePrivateAudio\",\"generation\":{generation},\"mix\":{{\"master\":{{\"level\":1,\"muted\":{muted}}},\"alerts\":{{\"level\":1,\"muted\":false}},\"interface\":{{\"level\":1,\"muted\":false}}}}}}"));
    }
    worker.step(&hub);
    assert!(!audible(&fake.sink("output:Helm")));
}

#[test]
fn named_room_selection_retires_old_sound_before_unblocking_a_private_default_collision() {
    let mut config = profile();
    config.media[2].outputs.clear();
    let mut audio = super::super::NativeRoomAudio::with_stores(None, false, None, None);
    audio.profile = config.clone();
    audio.private = PrivateAudio::new(config);
    let hub = audio.private.clone();
    let key = Endpoint::Console(PaneId(1));
    hub.bind(key, "helm");
    hub.room_output(Some("output:Helm".into()));
    let fake = Fake::with(&["output:Helm"]);
    let mut worker = Worker::new(fake.clone());
    worker.step(&hub);
    assert_eq!(hub.status(key).unwrap().status, "failed");
    let room_mixer = audio.mixer.clone();
    room_mixer.lock().unwrap().set_loop(
        "room",
        decoder::read("assets/sounds/ui_click.ogg").unwrap(),
        "music",
        0.5,
    );
    // Hold the later UI/persistence lock so the old buggy publication order
    // would visibly unblock private output while the room was still playing.
    let state = audio.state.clone();
    let held = state.lock().unwrap();
    let task = std::thread::spawn(move || {
        audio.command(
            &crate::native_host::host_lobby::HostLobbyRecord::SelectAudioOutput {
                output: Some("output:Room".into()),
            },
        );
        audio
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while !hub.status(key).unwrap().detail.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    worker.step(&hub);
    let opened = hub.status(key).unwrap().status == "playing";
    let old_active = room_mixer.lock().unwrap().active("room");
    drop(held);
    let _audio = task.join().unwrap();
    assert!(opened);
    assert!(!old_active);
}
