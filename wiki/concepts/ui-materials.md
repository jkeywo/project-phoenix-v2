---
title: UiMaterial Shader Pattern
type: concept
tags: [ui, shader, wgsl, ui-material, viewscreen, vignette]
sources: [src/server/viewscreen_border.rs, assets/shaders/red_alert_vignette.wgsl]
updated: 2026-08-27
---

# UiMaterial Shader Pattern

How to back a Bevy UI node with a custom WGSL shader, using a small `AsBindGroup`-derived material struct as the single Rust ↔ shader contract.

## Worked example: `RedAlertVignetteMaterial`

The first use of this pattern in the codebase is the Red Alert vignette in [`src/server/viewscreen_border.rs`](../../src/server/viewscreen_border.rs):

```rust
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct RedAlertVignetteMaterial {
    #[uniform(0)]
    pub intensity: f32,
}

impl UiMaterial for RedAlertVignetteMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/red_alert_vignette.wgsl".into()
    }
}
```

Three things are happening:

1. **Single uniform** — `intensity: f32` is the *only* surface between Rust and the shader. Drive it from a Bevy system, sample it in WGSL.
2. **`AsBindGroup` derive** — generates the binding plumbing automatically; `#[uniform(0)]` tags this field as bind-group binding 0.
3. **`UiMaterial` impl** — `fragment_shader()` points at a WGSL file that Trunk has copied into `dist/`.

## Wiring

```rust
app.add_plugins(UiMaterialPlugin::<RedAlertVignetteMaterial>::default())
   ...

let handle = materials.add(RedAlertVignetteMaterial { intensity: 0.0 });
parent.spawn((
    Node { /* full-bleed */ },
    MaterialNode(handle.clone()),
));
```

The handle is cached in a `Resource` so the per-frame system that drives the uniform (`drive_vignette_intensity` in this case) can mutate it without a query.

## When to reach for this

A custom `UiMaterial` is the right tool when **a UI region needs a procedural visual that depends on game state**, not a static texture or a simple colour. Cases that justify the WGSL hop:

- Radial or linear gradients with state-driven parameters, such as the current
  Red Alert vignette.
- Presentation effects that require a procedural pulse, sweep, or animation
  which cannot be represented by a static texture or colour.

If the visual is just an image or a tint, a plain `ImageNode` or `BackgroundColor` is enough — don't reach for a shader.

## Spawn-order Z layering

Bevy UI renders children in spawn order. The vignette `MaterialNode` is spawned *first* inside the viewscreen border root so the border sprites overlap its outermost ring. No explicit `ZIndex` is needed for this case. If you need finer control across distant subtrees, `ZIndex` is the escape hatch.

## File layout

- WGSL fragment shader: `assets/shaders/<name>.wgsl`
- Trunk `copy-dir` directive in `index.html` to publish the shader into `dist/`
- Material struct + impl: alongside the consuming module (here, `src/server/viewscreen_border.rs`)

## Related

- [Build & Deployment](./build-and-deployment.md) — Trunk asset pipeline
