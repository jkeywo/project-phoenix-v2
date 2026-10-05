include!("../../src/demo_build_value.rs");
fn main() {
    println!("cargo::rerun-if-env-changed=PHOENIX_DEMO_BUILD");
    println!("cargo::rerun-if-changed=../../src/demo_build_value.rs");
    println!("cargo::rustc-check-cfg=cfg(phoenix_demo_build)");
    if std::env::var("PHOENIX_DEMO_BUILD").as_deref() == Ok(DEMO_VALUE) {
        println!("cargo::rustc-cfg=phoenix_demo_build");
    }
}
