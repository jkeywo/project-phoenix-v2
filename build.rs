//! Turns the demo-deploy environment flag into a `cfg` the compiler can strip on.
//!
//! `crate::build_flags::is_demo_build()` already reads `PHOENIX_DEMO_BUILD`
//! through `option_env!`, which is enough to *answer* "am I the demo?" at
//! runtime. It is not enough to make code disappear: an `if` on a compile-time
//! constant still leaves the branch in the source, and issue #940 needs the
//! phone client's debug/cheat admission route to be **absent** from a demo
//! build rather than merely unreachable — the gate and the UI have to vanish
//! together, so there is nothing left to reach even if the UI were forged.
//!
//! `#[cfg]` cannot read an environment variable, so this build script converts
//! the same variable into the `phoenix_demo_build` cfg. `build_flags` carries a
//! test that the two answers always agree, so the cfg can never drift from the
//! `option_env!` the JS side reads back through `wasm_is_demo_build()`.
//!
//! Only `deploy-demo.yml` sets the variable. `TRUNK_BUILD_RELEASE` (which
//! `ci.yml` also sets, for the GitHub Pages dev host) is deliberately NOT
//! consulted here — see `src/build_flags.rs` for why the two flags are separate.

// `DEMO_VALUE`, the single literal this script and `crate::build_flags` both
// compare against. Shared through `include!` rather than written out twice: a
// build script cannot `use` the crate it builds, and two independent literals
// could only diverge in a demo build — which nothing but `deploy-demo.yml`
// produces, and that job runs no tests. See the file for the full story.
include!("src/demo_build_value.rs");

fn main() {
    // Cargo does not otherwise know this script reads the variable, so without
    // this a demo build reusing a dev build's cache would keep the dev cfg.
    println!("cargo::rerun-if-env-changed=PHOENIX_DEMO_BUILD");
    // Emitting any rerun-if-* directive switches off Cargo's default "re-run
    // when any file in the package changed", so the two inputs this script
    // actually has must be named explicitly — the included literal above and
    // this script itself.
    println!("cargo::rerun-if-changed=src/demo_build_value.rs");
    println!("cargo::rerun-if-changed=build.rs");
    // Declare the cfg so `--check-cfg` (on by default since Rust 1.80) does not
    // warn at every `#[cfg(phoenix_demo_build)]` site.
    println!("cargo::rustc-check-cfg=cfg(phoenix_demo_build)");

    if std::env::var("PHOENIX_DEMO_BUILD").as_deref() == Ok(DEMO_VALUE) {
        println!("cargo::rustc-cfg=phoenix_demo_build");
    }

    stage_ultralight_sdk();
}

/// Copy the Ultralight SDK's shared libraries beside the binary at build time.
///
/// `ul-next-sys` links the SDK's import libraries but stages none of its DLLs,
/// so on Windows a freshly built `phoenix-host.exe` fails to start with a silent
/// `STATUS_DLL_NOT_FOUND` (0xC0000135) — Windows resolves the imports at load,
/// before the process can reach `main` to run
/// `native_host::panes::ultralight::stage_sdk()`. That runtime helper therefore
/// cannot bootstrap the very first run. Staging the DLLs here, at build time,
/// closes the gap: the exe always finds them beside itself and starts, and
/// `stage_sdk()` still stages the SDK `resources/` at runtime and covers test
/// binaries under `deps/` (which this function does not touch).
///
/// Deliberately narrow: only the native host build pulls the SDK (the
/// `ultralight` feature), and the missing-DLL failure is Windows-only, so this
/// is a no-op for every other build — wasm, CI on ubuntu, or a plain
/// `--features headless` run.
fn stage_ultralight_sdk() {
    if std::env::var_os("CARGO_FEATURE_ULTRALIGHT").is_none()
        || std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
    {
        return;
    }
    let Some(out_dir) = std::env::var_os("OUT_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    // Keep re-running this script until the DLLs are actually staged.
    // `ul-next-sys` carries no `links` key, so Cargo is free to run its build
    // script - the one that DOWNLOADS the SDK - in parallel with this one, and
    // on a cold checkout the lookup below can therefore run before the DLLs
    // exist. Without a trigger that fails the freshness check, that first miss
    // is permanent: none of this script's other declared inputs change on the
    // next build, so it never re-runs, the copy never happens, and every later
    // build merely replays the cached warning. A stamp written only on success
    // is absent after a miss, and Cargo treats a declared `rerun-if-changed`
    // path that does not exist as dirty.
    let stamp = out_dir.join("ultralight-dlls-staged.stamp");
    println!("cargo::rerun-if-changed={}", stamp.display());
    // OUT_DIR = <target>/<profile>/build/project-phoenix-<hash>/out. The sibling
    // `ul-next-sys-<hash>/out/ul-sdk/bin` holds the DLLs, and <profile> — two
    // levels above build/ — is where `phoenix-host.exe` lands.
    let mut ancestors = out_dir.ancestors();
    let build_dir = ancestors.nth(2).map(std::path::Path::to_path_buf);
    let profile_dir = ancestors.next().map(std::path::Path::to_path_buf);
    let (Some(build_dir), Some(profile_dir)) = (build_dir, profile_dir) else {
        return;
    };
    let sdk_bin = match std::fs::read_dir(&build_dir) {
        Ok(entries) => entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("ul-next-sys-"))
            })
            .map(|p| p.join("out").join("ul-sdk").join("bin"))
            .find(|bin| bin.join("Ultralight.dll").is_file()),
        Err(_) => None,
    };
    let Some(sdk_bin) = sdk_bin else {
        println!(
            "cargo::warning=Ultralight SDK DLLs not found under {}/ul-next-sys-* yet \
             (the SDK download is probably still in flight); phoenix-host will not \
             start until they are staged, so this script re-runs on the next build \
             and stages them then",
            build_dir.display()
        );
        return;
    };
    // Re-run if the SDK is re-fetched (a version bump lands under a new hash and
    // so a different path, which reads as "changed" and re-triggers the copy).
    println!(
        "cargo::rerun-if-changed={}",
        sdk_bin.join("Ultralight.dll").display()
    );
    let mut staged = 0;
    if let Ok(dlls) = std::fs::read_dir(&sdk_bin) {
        for dll in dlls.flatten() {
            let path = dll.path();
            if path.extension().and_then(|e| e.to_str()) == Some("dll") {
                if let Some(name) = path.file_name() {
                    if std::fs::copy(&path, profile_dir.join(name)).is_ok() {
                        staged += 1;
                    }
                }
            }
        }
    }
    // Only a build that actually put DLLs beside the exe stops retrying.
    if staged > 0 {
        let _ = std::fs::write(&stamp, b"");
    }
}
