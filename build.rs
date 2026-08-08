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
}
