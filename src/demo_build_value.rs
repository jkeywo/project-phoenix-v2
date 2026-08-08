// The one literal that decides whether a build is the public demo.
//
// `include!`d by BOTH halves of the gate:
//
//   - `build.rs`, which reads `PHOENIX_DEMO_BUILD` from the environment and
//     turns a match into the `phoenix_demo_build` cfg, and
//   - `src/build_flags.rs`, which reads the same variable back at runtime
//     through `option_env!` for the JS side (`wasm_is_demo_build()`).
//
// A file rather than a constant in either place because a build script cannot
// `use` the crate it builds. Before this, the value was written out twice, and
// `build_flags::the_cfg_gate_and_the_runtime_flag_agree` could only catch an
// inverted *sense*: with the variable unset both halves answer `false`
// whatever the literals say, and the only job that ever sets it
// (`deploy-demo.yml`) runs `trunk build` and no tests. Drifted literals would
// therefore have shipped. Now there is one literal, so they cannot drift.
//
// Deliberately NOT a module: `include!` splices this file into whichever scope
// names it, so it must stay a single item with no inner attributes (`//!` doc
// comments), no `mod` and no `use` of its own.

/// The exact value of `PHOENIX_DEMO_BUILD` that means "this is the demo build".
const DEMO_VALUE: &str = "true";
