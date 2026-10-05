pub use phoenix_sim_world::world::script::fixture::*;
/// Compile a world TOML's script declaration into the live
/// [`WorldScriptRuntime`](crate::world::server::WorldScriptRuntime) the SAME way
/// `compile_world_scripts` does in production: through
/// [`load_world_scripts`](crate::world::script::load::load_world_scripts) with a
/// [`NoSiblingScripts`](crate::world::script::load::NoSiblingScripts) resolver,
/// asserting the set compiles and lints clean, then
/// [`WorldScriptRuntime::from_compiled`](crate::world::server::WorldScriptRuntime::from_compiled).
///
/// The single fixture compiler (issue #1215): the `comms::scripted` dialogue
/// fixtures and the `world::server` scripted-trigger fixtures route through here,
/// so the resolver, the clean-compile assertion, and the production loader are
/// named in one place instead of hand-rolled per call site.
///
/// Panics if the world authors no runnable script — a fixture asking for a
/// runtime must author one.
pub fn compile_world_runtime(
    world_path: &str,
    world_toml: &str,
) -> crate::world::server::WorldScriptRuntime {
    let value: toml::Value = toml::from_str(world_toml).expect("fixture world must be valid TOML");
    let compiled = crate::world::script::load::load_world_scripts(
        world_path,
        &value,
        &crate::world::script::load::NoSiblingScripts,
    );
    assert!(
        !crate::world::validate::has_error(&compiled.findings),
        "{world_path} fixture scripts must compile clean: {:?}",
        compiled.findings
    );
    // The loader used to record this itself; since issue #1241 its caller does.
    // A fixture applies it too, so a test that compiles a world through here sees
    // the same ledger it saw before the lift — the point of routing every fixture
    // compile through the production loader in the first place.
    if let Some(digest) = &compiled.ledger_digest {
        digest.apply();
    }
    crate::world::server::WorldScriptRuntime::from_compiled(compiled)
        .expect("fixture world must author at least one runnable script")
}
