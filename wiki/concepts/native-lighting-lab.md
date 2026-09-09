---
title: Native Lighting Lab
type: concept
tags: [rendering, native, prototype, dust, lighting]
sources: [prototypes/native-lighting/src/main.rs, prototypes/native-lighting/src/flare.rs, prototypes/native-lighting/flare.wgsl, prototypes/native-lighting/scene.toml, prototypes/native-lighting/README.md, run-lighting-lab.bat, src/server/native_visuals/mod.rs, src/server/native_visuals/flare.rs, src/world/native_render_config.rs, assets/shaders/star_flare.wgsl]
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
`src/server/native_visuals/`, registered by the native `RendererPlugin`.
The browser retains its existing PFX dust path. No volumetric effect is added.
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
motes too; other legacy dust tuning applies to the browser's old field.
The native pool clears outside a mission and hides for non-3D views.
The flare uses the dominant star and samples opaque depth for occlusion;
the renderer supports both single-sample and multisampled depth.
Only one star casts shadows at a time. Star/halo and billboard quads are excluded
from casting; custom unlit surfaces do not receive PBR shadows.

`examples/capture_native_lighting.rs` runs the actual native host in a bounded
mission capture, including the postprocess and normal gameplay antialiasing.
