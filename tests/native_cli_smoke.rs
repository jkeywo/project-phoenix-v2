//! Process-level argument refusals must finish before content, network or GPU startup.
#![cfg(all(
    feature = "host",
    feature = "headless",
    feature = "perf",
    feature = "capture",
    not(target_arch = "wasm32")
))]

use std::process::Command;

#[test]
// A unique temporary directory is test infrastructure, not simulation identity.
#[allow(clippy::disallowed_methods)]
fn all_native_tools_offer_help_and_reject_bad_syntax_without_startup() {
    let tools = [
        (
            env!("CARGO_BIN_EXE_phoenix-host"),
            vec!["--world", "missing.toml", "--unknown"],
        ),
        (
            env!("CARGO_BIN_EXE_phoenix-headless"),
            vec!["--world", "missing.toml", "--hz=NaN"],
        ),
        (
            env!("CARGO_BIN_EXE_phoenix-perf"),
            vec!["report", "--capture", "missing.json", "--unknown"],
        ),
        (
            env!("CARGO_BIN_EXE_capture-billboard"),
            vec!["missing.glb", "out.png", "--views=1.5"],
        ),
        (
            env!("CARGO_BIN_EXE_tune-lods"),
            vec!["missing.glb", "--decimate", "--distance=0"],
        ),
    ];
    let directory =
        std::env::temp_dir().join(format!("phoenix-cli-smoke-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    for (binary, invalid) in tools {
        let help = Command::new(binary)
            .arg("--help")
            .current_dir(&directory)
            .output()
            .unwrap();
        assert!(
            help.status.success(),
            "{binary}: {}",
            String::from_utf8_lossy(&help.stderr)
        );
        assert!(
            String::from_utf8_lossy(&help.stdout).contains("Usage:"),
            "{binary}"
        );
        let error = Command::new(binary)
            .args(invalid)
            .current_dir(&directory)
            .output()
            .unwrap();
        assert_eq!(
            error.status.code(),
            Some(2),
            "{binary}: {}",
            String::from_utf8_lossy(&error.stderr)
        );
        assert!(!error.stderr.is_empty(), "{binary}");
    }
    let perf = Command::new(env!("CARGO_BIN_EXE_phoenix-perf"))
        .current_dir(&directory)
        .output()
        .unwrap();
    assert!(perf.status.success());
    assert!(String::from_utf8_lossy(&perf.stdout).contains("Usage:"));
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
    std::fs::remove_dir(&directory).unwrap();
}
