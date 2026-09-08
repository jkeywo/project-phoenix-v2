//! Native window/adapter composition. No camera or microphone is activated:
//! both selected keys are deliberately absent. Needs a Windows display.
#![cfg(all(feature = "host", target_os = "windows"))]

use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
#[ignore = "opens a native diagnostic window; needs a Windows display"]
fn absent_capture_devices_refuse_without_panicking_or_substituting() {
    let profile = std::env::temp_dir().join(format!("phoenix-media-{}.toml", uuid::Uuid::new_v4()));
    std::fs::write(&profile, "version = 1\n[[media]]\nsurface = 'comms'\ncamera = 'camera:phoenix-deliberately-absent'\nmicrophone = ['mic:phoenix-deliberately-absent']\n").unwrap();
    let result = std::panic::catch_unwind(|| {
        for action in ["--meter-microphone", "--preview-camera"] {
            let mut child = Command::new(env!("CARGO_BIN_EXE_phoenix-host"))
                .args(["--setup", "--profile"])
                .arg(&profile)
                .args([action, "comms"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(45);
            loop {
                if child.try_wait().unwrap().is_some() {
                    break;
                }
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("{action} failed to terminate");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let output = child.wait_with_output().unwrap();
            let diagnostic = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.status.code(), Some(1), "{action}: {diagnostic}");
            assert!(
                diagnostic.contains("missing; nothing substituted"),
                "{action}: {diagnostic}"
            );
            assert!(!diagnostic.contains("panicked"), "{action}: {diagnostic}");
        }
    });
    let _ = std::fs::remove_file(profile);
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}
