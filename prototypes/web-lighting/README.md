# WebGL2 lighting lab

A standalone Bevy 0.18.1/WASM comparison of the native lab's approved effects.
It does not start or modify the game. Uses the same cruiser, rocks, platform,
mote textures, streak motion and flare profile as the native lab.

From the repository root, run `run-web-lighting-lab.bat`, then open
<http://127.0.0.1:8094>. Keep the launcher running. It stages the small asset
subset, builds the WASM application, and starts Trunk's local server.
The first build can take several minutes. `scene.toml` tuning is embedded at
build time; restart the launcher after changing it.

| Control | Effect |
|---|---|
| M | Toggle the 440 motes |
| H | Toggle real one-cascade star shadows |
| F | Toggle flare, preserving its intensity |
| O | Toggle approximate occlusion; HUD shows visible percentage |
| Tap or hold - / + | Adjust flare intensity from 0 to 3 (starts at 3) |
| Shift with - / + | Fine intensity adjustment |
| WASD / Q E | Fly / descend and ascend |
| Arrow keys | Turn camera |
| Space | Cruise forward |
| P | Pause motion to compare toggles at the same pose |
| R | Reset camera and animation |

Click the canvas if keyboard controls do not respond. The big moving rock
crosses the star and should fade its flare, while shadows move across the
ribbed platform. Pause and toggle H/F/O individually to judge each change.
Turn away from the star to check that its flare disappears. Cruise or strafe
to judge the motes; M gives an immediate comparison.

## Implementation and limits

- WebGL2, 4x MSAA, HDR/tonemapping; no bloom, depth prepass or volumes.
- [ai] One 1024 shadow map over a 120-unit camera range, versus the native
  lab's three cascades. This is real dynamic shadow mapping with less coverage.
- [ai] The flare uses one additive fullscreen colour pass, reusing the native
  flare profile without reading depth. Twenty-four CPU segment tests across the
  star's disc determine visibility. Rocks use ellipsoids; hull mesh parts and
  platform pieces use their local bounding boxes. Visibility is smoothed.
- These proxies approximate silhouettes: gaps in a mesh's bounding box may
  over-occlude the flare. An object behind the star does not occlude it.
- Native effects and production web rendering are unchanged.

Build only: `node prototypes/web-lighting/stage-assets.mjs`, then
`trunk build --config prototypes/web-lighting/Trunk.toml` with `NO_COLOR=true`
and `CARGO_TARGET_DIR` pointing at the repository's `target` folder.

Geometry checks: `cargo test --manifest-path prototypes/web-lighting/Cargo.toml --target-dir target`.
