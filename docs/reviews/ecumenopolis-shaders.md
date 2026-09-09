# Ecumenopolis rendering verification — 2026-09-09

Texture-memory revision: 11 maps now occupy 53.88 MiB on disk and 112.17 MiB
including mipmaps on the GPU, down from 60.87 and 163.46 MiB (31.4% less resident
texture data). The base colour, packed light masks, material channels, city glow
and smog colour were byte-checked against the enhanced-albedo bake and retained.
Surface normals are 2K; district light colour and smog normals are 512 pixels
wide; haze is 256. Smog opacity retains its 1K resolution. Haze and opacity use
R8, and the negligible skyglow map is omitted through an optional config field.
All KTX2 maps retain complete mip chains and lossless Zstd. No UASTC or Basis
transcoder was added.

The 37 targeted Rust planet/viewer tests, native capture build, asset contract,
formatting, JavaScript syntax, and PASM validate/scan/traceability passed. Five
native comparison poses retain readable streets and the established night
lighting. Mean absolute channel differences against the enhanced-albedo captures
range from 0.28 to 1.58 on the 0–255 scale over non-background pixels; animated
traffic and drifting haze contribute to these differences. Captures and comparison
data are under `target/planet-review/optimized-native/`.
The WASM build and five WebGL2/SwiftShader captures also passed: all 11 textures
loaded, with no shader errors, missing assets or blank frames. Browser mean
channel differences are 0.40–2.14; captures, statistics and comparison data are
under `target/planet-review/optimized-browser/`. This verifies the R8 loading path
on WebGL, but is not a cross-device hardware performance test.

Base-colour enhancement: the built-in ImageGen tool produced a clearer 1774 × 887
version of the original albedo, baked to the existing 4096 × 2048 runtime map.
Source and exact prompt are in `scripts/planets/sources/`. All ten other KTX2 maps
were byte-checked against the preceding bake and are unchanged. Texture data is
now 60.87 MiB on disk; GPU allocation remains 163.46 MiB. Native and browser
comparisons are under `target/planet-review/enhanced-albedo-{native,browser}/`.
This adds interpreted artwork detail, not physically recovered source resolution.

Current art revision: restored the original night windows, neon, traffic and glow
maps byte-for-byte. Daylight roofs retain street contrast while varying setbacks
and courtyards. Material alpha independently selects daytime activity patches at
60% intensity, encoded in 128–255 so image encoding retains the independent RGB
material data. The decoded base material was checked for zero-roughness corruption:
none. The final texture set is 53.63 MiB on disk (GPU allocation unchanged).
Updated comparisons are in `target/planet-review/restored-night-native/` and
`target/planet-review/restored-night-browser/`; both use the same five poses.
Earlier evidence below describes the initial implementation and its captures.

Implemented against `2f95e796f7a6d9ae15950f4c827d7b3460c30b14` in
`codex/ecumenopolis-layers`, with the final files copied into the working checkout.

The supplied district, infrastructure, weather and emission maps now drive an
offline 4096 × 2048 city bake. Surface materials, windows, neon, heat and animated
traffic have separate controls. Drifting smog receives stationary city glow and
casts sun-relative shadows. A separate atmosphere shell integrates single
scattering with a solar optical-depth lookup. Polar caps use a blended planar
projection; normals flatten inside them. Legacy planets keep their existing mode.

Validation:

- `cargo test --lib --features viewer,capture -- entities::planet::tests viewer::`:
  37 passed, including shipped ecumenopolis configuration and texture-path checks.
- `trunk build --config viewer-trunk.toml`: passed.
- `cargo build --features viewer,capture --example capture_planet`: passed.
- `node scripts/planets/check-ecumenopolis.mjs`: 12 maps; complete mip chains,
  expected colour spaces and matching atmosphere geometry; 54.44 MiB on disk,
  163.46 MiB decoded with mipmaps.
- Real WASM viewer: five ecumenopolis poses (day, terminator, night, close, pole),
  all 12 textures loaded, no reported missing assets, shader errors or blank frames.
  Earth also passed the same five-pose browser check with its six existing maps.
- Native offscreen capture: same five poses completed on local wgpu/Vulkan.
  Vulkan's optional validation layer is unavailable on this machine. Final
  observed averages were 4.9–7.9 ms/frame including startup, readback and scheduling;
  these are capture timings, not a GPU benchmark or a full-scene performance budget.
- PASM `validate`, `scan`, and `traceability`: passed (existing informational
  warnings remain). JavaScript syntax and `git diff --check`: passed.

Final images and browser reports are copied to `target/planet-review/`, with
`before/`, `browser/`, `native/`, and `earth/` directories. Reproduction and channel
details are in `scripts/planets/README.md`.

Limits: the original macro artwork remains 1K, with new fine structure baked at
4K. This uses texture and normal detail rather than building geometry. Atmospheric
extinction uses scalar transparent coverage, and the scattering model omits
multiple scattering. Zstd reduces download size but not GPU memory; multiple
simultaneously resident city planets need a separate memory/performance budget.
Browser captures use SwiftShader, so they establish WebGL compatibility and
appearance rather than hardware frame rate. No full pre-push gate or push was run.
