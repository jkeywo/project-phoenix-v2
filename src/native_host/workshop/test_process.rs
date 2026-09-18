//! Disposable native Test process. Parent and child share inherited pipes;
//! neither a delivery listener nor a crew/GM connection carries these controls.
use super::test_clock::{TestClock, TestClockPlugin, TestControl, TestControls};
use crate::workshop::provider::test_snapshot::TestSnapshot;
pub use crate::workshop::test_protocol::{ControlRecord, Launch, TestStatus};
use bevy::prelude::*;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

pub const CHILD_FLAG: &str = "--workshop-test-child";
const STATUS_PREFIX: &str = "PHOENIX_WORKSHOP_TEST ";
const MAX_RECORD_BYTES: u64 = 65_536;

struct Stage {
    path: PathBuf,
    parent: PathBuf,
}
impl Stage {
    #[allow(clippy::disallowed_methods)] // Private host staging identity; never a simulation identity.
    fn create(parent: &Path, snapshot: TestSnapshot) -> Result<(Self, Launch), String> {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
        let stage = Self {
            path: parent.join(uuid::Uuid::new_v4().to_string()),
            parent,
        };
        fs::create_dir(&stage.path).map_err(|e| e.to_string())?;
        for (name, bytes) in snapshot.files {
            if !crate::workshop::provider::test_snapshot::stage_path(&name) {
                return Err("Invalid disposable Test content path".into());
            }
            let path = stage.path.join(name);
            fs::create_dir_all(path.parent().ok_or("Invalid Test content parent")?)
                .map_err(|e| e.to_string())?;
            fs::write(path, bytes).map_err(|e| e.to_string())?;
        }
        let launch = Launch {
            selection: snapshot.selection,
            revision: snapshot.revision,
        };
        fs::write(
            stage.path.join("workshop-test.json"),
            crate::core::codec::encode_workshop_test_launch(&launch)?,
        )
        .map_err(|e| e.to_string())?;
        Ok((stage, launch))
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        // The only recursive deletion is our newly-created UUID child, never
        // the selected project/mod root or a path supplied by the document.
        if self.path.parent() == Some(self.parent.as_path()) && self.path != self.parent {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

struct Shared {
    state: Mutex<TestStatus>,
    changed: Condvar,
}
pub struct TestProcess {
    child: Child,
    input: ChildStdin,
    state: Arc<Shared>,
    output: Option<std::thread::JoinHandle<()>>,
    sequence: u64,
    _stage: Stage,
}
impl TestProcess {
    pub fn start(executable: &Path, stages: &Path, snapshot: TestSnapshot) -> Result<Self, String> {
        let (stage, launch) = Stage::create(stages, snapshot)?;
        let mut command = Command::new(executable);
        command
            .arg(CHILD_FLAG)
            .arg(stage.path.join("workshop-test.json"))
            .current_dir(&stage.path)
            .env_remove("BEVY_ASSET_ROOT")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // Suppress a helper console; winit creates the intended visible
            // interactive Test Viewscreen itself on the child's main thread.
            command.creation_flags(0x0800_0000);
        }
        Self::spawn(command, stage, launch)
    }
    fn spawn(mut command: Command, stage: Stage, launch: Launch) -> Result<Self, String> {
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let pipes = child.stdin.take().zip(child.stdout.take());
        let Some((input, output)) = pipes else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Test private pipes are unavailable".into());
        };
        let state = Arc::new(Shared {
            state: Mutex::new(TestStatus::starting(&launch)),
            changed: Condvar::new(),
        });
        let observed = state.clone();
        let reader = std::thread::Builder::new()
            .name("phoenix-workshop-test-output".into())
            .spawn(move || {
                let mut reader = BufReader::new(output);
                while let Ok(Some(line)) = bounded_line(&mut reader) {
                    let Some(json) = line.strip_prefix(STATUS_PREFIX) else {
                        continue;
                    };
                    let Ok(next) = crate::core::codec::decode_workshop_test_status(json) else {
                        continue;
                    };
                    *observed.state.lock().unwrap_or_else(|e| e.into_inner()) = next;
                    observed.changed.notify_all();
                }
                let mut state = observed.state.lock().unwrap_or_else(|e| e.into_inner());
                state.running = false;
                observed.changed.notify_all();
            })
            .map_err(|error| {
                let _ = child.kill();
                let _ = child.wait();
                error.to_string()
            })?;
        Ok(Self {
            child,
            input,
            state,
            output: Some(reader),
            sequence: 0,
            _stage: stage,
        })
    }
    pub fn status(&mut self) -> TestStatus {
        let mut state = self.state.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(Some(exit)) = self.child.try_wait() {
            state.running = false;
            if !exit.success() && state.error.is_none() {
                state.error = Some(format!("Disposable Test exited with {exit}"));
            }
        }
        state.clone()
    }
    pub fn control(&mut self, control: TestControl) -> Result<TestStatus, String> {
        if matches!(control, TestControl::Rate { multiplier } if !matches!(multiplier, 1 | 2 | 4 | 8))
        {
            return Err("Unsupported Test speed".into());
        }
        if matches!(control, TestControl::Step {}) && !self.status().paused {
            return Err("Pause Test before stepping".into());
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or("Test command sequence exhausted")?;
        // `TestControl` stopped being `Copy` when it gained a view, so the one
        // fact checked after the round trip is kept rather than the whole value.
        let stopping = matches!(control, TestControl::Stop {});
        let record = ControlRecord {
            id: self.sequence,
            control,
        };
        writeln!(
            self.input,
            "{}",
            crate::core::codec::encode_workshop_test_control(&record)?
        )
        .map_err(|e| e.to_string())?;
        self.input.flush().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = self.state.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.running && state.acknowledged < self.sequence {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("Test has not acknowledged this control".into());
            }
            state = self
                .state
                .changed
                .wait_timeout(state, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        if !state.running && !stopping {
            return Err("Disposable Test is closed".into());
        }
        Ok(state.clone())
    }
}

/// Called only while this selected workspace's OS claim is held, before its
/// worker can launch a new Test. A prior hard shutdown can leave staged bytes
/// behind even though the child's inherited-pipe EOF has retired the process.
pub(super) fn retire_abandoned_stages(parent: &Path) {
    let Ok(parent) = fs::canonicalize(parent) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&parent) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let owned_name = entry
            .file_name()
            .to_str()
            .is_some_and(|name| uuid::Uuid::parse_str(name).is_ok_and(|id| id.to_string() == name));
        let regular_directory = entry
            .file_type()
            .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink());
        if !owned_name
            || !regular_directory
            || fs::canonicalize(&path).ok().as_deref() != Some(path.as_path())
        {
            continue;
        }
        // Never reclaim an arbitrary directory someone placed in private
        // storage. Both the UUID name and the bounded typed marker are owed.
        let marker = path.join("workshop-test.json");
        if !fs::symlink_metadata(&marker)
            .is_ok_and(|meta| meta.is_file() && !meta.file_type().is_symlink())
        {
            continue;
        }
        let valid_marker = fs::File::open(marker)
            .ok()
            .and_then(|file| {
                let mut bytes = Vec::new();
                file.take(MAX_RECORD_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .ok()?;
                (bytes.len() as u64 <= MAX_RECORD_BYTES)
                    .then(|| crate::core::codec::decode_workshop_test_launch(&bytes).ok())
                    .flatten()
            })
            .is_some();
        if valid_marker && path.parent() == Some(parent.as_path()) {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(test)]
pub(super) fn pipe_probe(parent: &Path) -> (TestProcess, PathBuf) {
    let (stage, launch) = Stage::create(parent, tests::snapshot()).unwrap();
    let path = stage.path.clone();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "native_host::workshop::test_process::tests::pipe_probe_child",
            "--ignored",
            "--nocapture",
        ])
        .env(
            "PHOENIX_WORKSHOP_PIPE_PROBE",
            stage.path.join("workshop-test.json"),
        )
        .env("BEVY_ASSET_ROOT", env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    (TestProcess::spawn(command, stage, launch).unwrap(), path)
}
impl Drop for TestProcess {
    fn drop(&mut self) {
        // Kill/wait also covers boot failures and an unresponsive simulation;
        // cleanup never waits indefinitely for a Rhai callback or a GPU.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(output) = self.output.take() {
            let _ = output.join();
        }
    }
}

fn bounded_line(reader: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_RECORD_BYTES + 1)
        .read_until(b'\n', &mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() as u64 > MAX_RECORD_BYTES || !bytes.ends_with(b"\n") {
        return Err("Invalid Test pipe record".into());
    }
    String::from_utf8(bytes)
        .map(|line| Some(line.trim_end_matches(['\r', '\n']).into()))
        .map_err(|e| e.to_string())
}

#[derive(Resource)]
struct ChildPipe {
    input: Mutex<std::sync::mpsc::Receiver<ControlRecord>>,
    launch: Launch,
    acknowledged: u64,
}

/// The internal command accepts exactly one host-created descriptor. It is a
/// new process, so cache, seed, ledger and entity IDs cannot inherit a Test run.
pub fn run_child(descriptor: &Path) -> Result<(), String> {
    let mut bytes = Vec::new();
    fs::File::open(descriptor)
        .map_err(|e| e.to_string())?
        .take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("Test launch descriptor is too large".into());
    }
    let launch = crate::core::codec::decode_workshop_test_launch(&bytes)?;
    let result = run_launched_child(descriptor, &launch);
    if let Err(message) = &result {
        let mut status = TestStatus::starting(&launch);
        status.running = false;
        status.error = Some(message.clone());
        write_status(&status);
    }
    result
}

fn run_launched_child(descriptor: &Path, launch: &Launch) -> Result<(), String> {
    if !crate::workshop::provider::safe_path(&launch.selection.world)
        || !launch.selection.world.starts_with("assets/worlds/")
        || !crate::workshop::provider::safe_path(&launch.selection.ship)
        || !launch.selection.ship.starts_with("assets/entities/")
    {
        return Err("Invalid Test selection".into());
    }
    let (send, input) = std::sync::mpsc::sync_channel(8);
    std::thread::Builder::new()
        .name("phoenix-workshop-test-input".into())
        .spawn(move || {
            let mut reader = BufReader::new(std::io::stdin());
            while let Ok(Some(line)) = bounded_line(&mut reader) {
                let Ok(record) = crate::core::codec::decode_workshop_test_control(&line) else {
                    break;
                };
                if send.try_send(record).is_err() {
                    std::process::exit(2);
                }
            }
            // EOF also retires a child still booting or stuck in rendering.
            // This thread exists only in the explicitly disposable CLI child.
            std::process::exit(0);
        })
        .map_err(|e| e.to_string())?;
    pin_test_content_root(descriptor)?;
    let preload = crate::native_host::preload_content_templates(".").map_err(|e| e.to_string())?;
    let mut config = crate::native_host::NativeHostConfig::new(&launch.selection.world);
    config.ship_path = Some(launch.selection.ship.clone());
    config.seed = Some(launch.selection.seed);
    config.solo = true;
    config.deterministic = true;
    config.remember_layout = false;
    config.surface = crate::boot::NativeRenderSurface::Window;
    let mut app =
        crate::native_host::build_native_host_app(&config, &preload).map_err(|e| e.to_string())?;
    let world = app.world_mut();
    for mut window in world.query::<&mut Window>().iter_mut(world) {
        window.title = "Project Phoenix — Test".into();
    }
    use crate::authoritative::{DeclareState, StateClass};
    app.declare_state::<ChildPipe>(StateClass::Timer, "gm-milestone-integrated-workshop")
        .insert_resource(ChildPipe {
            input: Mutex::new(input),
            launch: launch.clone(),
            acknowledged: 0,
        })
        .add_plugins(TestClockPlugin)
        .add_systems(
            First,
            drain_controls.before(super::test_clock::apply_controls),
        )
        .add_systems(Last, publish_status.after(super::test_clock::finish_step));
    app.run();
    Ok(())
}

fn pin_test_content_root(descriptor: &Path) -> Result<PathBuf, String> {
    let content = descriptor
        .parent()
        .ok_or("Test content root is unavailable")?;
    // This is a fresh disposable child, before any asset reader exists. Even a
    // directly invoked child must not inherit a Live host's external asset root.
    std::env::remove_var("BEVY_ASSET_ROOT");
    crate::native_host::pin_content_root(&content.to_string_lossy())
}

fn drain_controls(mut pipe: ResMut<ChildPipe>, mut controls: ResMut<TestControls>) {
    let records: Vec<_> = pipe
        .input
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .try_iter()
        .take(8)
        .collect();
    for record in records {
        if record.id <= pipe.acknowledged {
            continue;
        }
        pipe.acknowledged = record.id;
        controls.0.push_back(record.control);
    }
}
fn publish_status(
    pipe: Res<ChildPipe>,
    clock: Res<TestClock>,
    tick: Res<crate::sim_tick::SimTick>,
    view: Res<crate::workshop::test_view::TestViewState>,
) {
    let state = TestStatus {
        running: true,
        starting: false,
        paused: clock.paused,
        tick: tick.0,
        multiplier: clock.multiplier,
        acknowledged: pipe.acknowledged,
        error: None,
        revision: pipe.launch.revision.clone(),
        selection: pipe.launch.selection.clone(),
        view: view.requested.clone(),
        ships: view.ships.clone(),
    };
    write_status(&state);
}

fn write_status(state: &TestStatus) {
    if let Ok(record) = crate::core::codec::encode_workshop_test_status(state) {
        // Dedicated inherited pipe, never a host channel or crew broadcast.
        let mut output = std::io::stdout().lock();
        let _ = writeln!(output, "{STATUS_PREFIX}{record}");
        let _ = output.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workshop::test_protocol::TestSelection;

    pub(super) fn snapshot() -> TestSnapshot {
        TestSnapshot {
            files: std::collections::BTreeMap::from([
                (
                    "assets/worlds/test.toml".into(),
                    b"# exact\r\n[global]\r\n".to_vec(),
                ),
                (
                    "assets/gui/dpad-button-idle.png".into(),
                    include_bytes!("../../../assets/gui/dpad-button-idle.png").to_vec(),
                ),
                (
                    "assets/shaders/test.wgsl".into(),
                    b"// captured support\r\n".to_vec(),
                ),
            ]),
            selection: TestSelection {
                world: "assets/worlds/test.toml".into(),
                ship: "assets/entities/test.toml".into(),
                seed: 1,
            },
            revision: "fixture".into(),
        }
    }
    struct Fixture(PathBuf);
    impl Fixture {
        #[allow(clippy::disallowed_methods)] // Isolated test filesystem identity.
        fn new() -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("phoenix-workshop-process-{}", uuid::Uuid::new_v4())),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn exact_staged_source_is_removed_on_drop_and_abandoned_owned_stages_are_reclaimed() {
        let fixture = Fixture::new();
        let (stage, _) = Stage::create(&fixture.0, snapshot()).unwrap();
        let path = stage.path.clone();
        assert_eq!(
            fs::read(path.join("assets/worlds/test.toml")).unwrap(),
            b"# exact\r\n[global]\r\n"
        );
        drop(stage);
        assert!(!path.exists());
        let (abandoned, _) = Stage::create(&fixture.0, snapshot()).unwrap();
        let abandoned_path = abandoned.path.clone();
        std::mem::forget(abandoned); // Hard shutdown does not run Drop.
        fs::create_dir_all(fixture.0.join("operator-files")).unwrap();
        let malformed = fixture.0.join("00000000-0000-0000-0000-000000000001");
        fs::create_dir(&malformed).unwrap();
        fs::write(malformed.join("workshop-test.json"), "invalid").unwrap();
        retire_abandoned_stages(&fixture.0);
        assert!(!abandoned_path.exists());
        assert!(fixture.0.join("operator-files").is_dir());
        assert!(malformed.is_dir());
    }

    #[test]
    fn private_records_refuse_oversize_partial_and_extra_authority_fields() {
        assert!(bounded_line(&mut std::io::Cursor::new(vec![
            b'x';
            MAX_RECORD_BYTES as usize + 1
        ]))
        .is_err());
        assert!(bounded_line(&mut std::io::Cursor::new(b"partial")).is_err());
        assert_eq!(
            bounded_line(&mut std::io::Cursor::new(b"complete\r\n")).unwrap(),
            Some("complete".into())
        );
        for value in [
            r#"{"id":1,"control":{"command":"pause","path":"outside"}}"#,
            r#"{"id":1,"control":{"command":"step"},"world":"outside"}"#,
            r#"{"id":-1,"control":{"command":"pause"}}"#,
            r#"{"id":1,"control":{"command":"script","source":"arbitrary"}}"#,
        ] {
            assert!(
                crate::core::codec::decode_workshop_test_control(value).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn inherited_pipe_acknowledgement_and_drop_retire_the_real_child_and_stage() {
        let fixture = Fixture::new();
        let (mut process, path) = pipe_probe(&fixture.0);
        let status = process.control(TestControl::Pause {}).unwrap();
        assert!(status.running && status.paused && !status.starting);
        assert_eq!(status.acknowledged, 1);
        assert!(process
            .control(TestControl::Rate { multiplier: 255 })
            .is_err());
        assert!(path.exists());
        drop(process);
        assert!(!path.exists());
    }

    #[test]
    #[ignore = "Child fixture launched only by inherited-pipe lifecycle tests"]
    fn pipe_probe_child() {
        let Some(path) = std::env::var_os("PHOENIX_WORKSHOP_PIPE_PROBE") else {
            return;
        };
        let path = PathBuf::from(path);
        let launch =
            crate::core::codec::decode_workshop_test_launch(&fs::read(&path).unwrap()).unwrap();
        let root = pin_test_content_root(&path).unwrap();
        assert_eq!(
            fs::canonicalize(std::env::current_dir().unwrap()).unwrap(),
            root
        );
        // Use the actual shared shell's AssetPlugin/AssetServer source, not a
        // direct filesystem read. Cargo's root and a deliberately inherited
        // BEVY_ASSET_ROOT must not make uncaptured shipped content available.
        let app = super::super::build_shell(crate::boot::NativeRenderSurface::Contract).unwrap();
        let source = app
            .world()
            .resource::<AssetServer>()
            .get_source(bevy::asset::io::AssetSourceId::Default)
            .unwrap();
        bevy::tasks::block_on(async {
            for (name, expected) in snapshot().files {
                let mut reader = source
                    .reader()
                    .read(Path::new(name.strip_prefix("assets/").unwrap()))
                    .await
                    .unwrap();
                let mut actual = Vec::new();
                reader.read_to_end(&mut actual).await.unwrap();
                assert_eq!(actual, expected);
            }
            assert!(source
                .reader()
                .read(Path::new("worlds/combat_test.toml"))
                .await
                .is_err());
        });
        let mut status = TestStatus::starting(&launch);
        status.starting = false;
        write_status(&status);
        let mut input = BufReader::new(std::io::stdin());
        while let Some(line) = bounded_line(&mut input).unwrap() {
            let record = crate::core::codec::decode_workshop_test_control(&line).unwrap();
            status.acknowledged = record.id;
            if matches!(record.control, TestControl::Pause {}) {
                status.paused = true;
            }
            if matches!(record.control, TestControl::Stop {}) {
                status.running = false;
            }
            write_status(&status);
            if !status.running {
                break;
            }
        }
    }
}
