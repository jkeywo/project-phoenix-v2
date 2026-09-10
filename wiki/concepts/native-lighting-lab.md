---
title: Native Lighting Lab
type: concept
tags: [rendering, native, prototype, dust, lighting]
sources: [prototypes/web-lighting/src/main.rs, prototypes/web-lighting/src/flare.rs, prototypes/web-lighting/README.md, run-web-lighting-lab.bat, prototypes/native-lighting/src/main.rs, prototypes/native-lighting/src/flare.rs, prototypes/native-lighting/flare.wgsl, prototypes/native-lighting/scene.toml, prototypes/native-lighting/README.md, run-lighting-lab.bat, src/server/native_visuals/mod.rs, src/server/native_visuals/flare.rs, src/server/native_visuals/web_flare.rs, src/server/native_visuals/web_occlusion.rs, src/world/native_render_config.rs, assets/shaders/star_flare.wgsl]
---

# Native Lighting Lab

`run-lighting-lab.bat` runs an isolated Bevy 0.18.1 renderer experiment from
`prototypes/native-lighting/`, a standalone Cargo workspace that does not link
the simulation library. It compares directional star shadows, volumetric dust,
a depth-occluded flare, and a representative mote field using the existing
Phoenix mote shader/textures. The hybrid mode keeps a sparse particle layer.

The scene contains a cruiser, moving rock occluders, an emissive star and a
ribbed shadow receiver. `scene.toml` owns prototype tuning. Keyboard controls,
capture invocation, and differences from the production renderer are recorded
in the prototype README. Running the lab does not change game rendering or
world configuration.

The default run uses motes, shadows and flare, with volume scattering disabled.
Flare intensity is adjustable live with - / +, with Shift for finer changes;
`scene.toml` supplies the starting value and `--flare` can override it for a run.


## Native gameplay

The approved motes, star shadows and flare now live in
`src/server/native_visuals/`, registered by `RendererPlugin` on native and web.
Both targets use the same fixed mote pool; the old PFX dust emitter is disabled. No volumetric effect is added.
Both the lab and native gameplay default to flare intensity **3**.

Worlds can override the defaults:

```toml
[render.native]
flare_intensity = 3.0 # 0 disables; range 0..3
star_shadows = true
motes = true
```

`NativeRenderConfig` also owns pool size, dimensions, textures and appearance,
plus shadow resolution and coverage. `[dust].enabled = false` disables native
motes too; other legacy dust tuning is superseded by platform mote settings.
The native pool clears outside a mission and hides for non-3D views.
The flare uses the dominant star and samples opaque depth for occlusion;
the renderer supports both single-sample and multisampled depth.
Only one star casts shadows at a time. Star/halo and billboard quads are excluded
from casting; custom unlit surfaces do not receive PBR shadows.

`examples/capture_native_lighting.rs` runs the actual native host in a bounded
mission capture, including the postprocess and normal gameplay antialiasing.

## WebGL2 comparison lab

`prototypes/web-lighting/` is a separate WASM/Bevy workspace launched by
`run-web-lighting-lab.bat` on localhost port 8094. It retains the native lab's
scene and motes, uses a single directional shadow cascade, and recreates the
flare with a colour-only additive pass whose visibility comes from CPU rays
against ellipsoids and mesh bounding boxes. Its README records controls and
approximation limits. The approved web effects also run in production gameplay.

## Web gameplay

`[render.web]` exposes the same controls as `[render.native]`, with independent
settings and defaults: flare intensity 3, 440 motes, one 1024 shadow map and
120-unit shadow coverage. Omitted or partial web tables retain these defaults.
The shared lifecycle hides motes and flare outside 3D mission views.

`web_flare.rs` uses a colour-only post-tonemapping pass, preserving MSAA without
a depth prepass. `web_occlusion.rs` samples 24 segments toward the star's front
hemisphere, using visible opaque mesh bounds and ellipsoids for planet surfaces.
Transparent effects and the hidden first-person hull cannot occlude it; objects
behind the source cannot block it. Visibility is smoothed. Bounds approximate
mesh silhouettes, so narrow gaps can over-occlude. Custom unlit surfaces still
do not receive PBR shadows. Asset preload resolves the active platform's mote
textures before flight.
