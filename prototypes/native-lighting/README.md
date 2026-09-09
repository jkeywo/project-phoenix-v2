# Native lighting lab

A standalone Bevy **0.18.1** renderer experiment. It loads Phoenix's cruiser,
mote textures and mote fragment shader, but does not start a mission, transport,
console, or Ultralight. It has its own Cargo workspace and lockfile so building
it does not rebuild or link the Phoenix simulation library.

From this checkout's root:

```powershell
cargo run --manifest-path prototypes/native-lighting/Cargo.toml --target-dir target
```

On Windows, `run-lighting-lab.bat` launches the existing executable immediately,
or builds it if missing. Use the Cargo command above after Rust source edits;
the batch launcher does not check source freshness. Scene tuning is read anew
on every launch.

The default is **motes with shadows and flare**, with volumetric scattering off.
The camera starts beside a cruiser above a ribbed platform. Moving asteroids
cross the star and cast shadows onto the platform and the dust. The platform
is a deliberate shadow receiver for judging detail, not proposed game scenery.

| Control | Action |
|---|---|
| 1 / 2 / 3 | Motes / volume / hybrid |
| WASD | Fly forward/back and sideways |
| Q / E | Fly down/up |
| Arrow keys | Turn the camera |
| Left Shift | Four times flight speed |
| Space | Toggle forward cruise |
| P | Pause flight and asteroid motion; looking and effect toggles still work |
| R | Reset camera and asteroid time; stop cruise |
| L | Toggle volume scattering |
| H | Toggle shadow maps (volumetric star lighting also needs these) |
| F | Toggle lens flare |
| Hold - / + (the = key) | Decrease/increase flare intensity from 0 to 3; the HUD shows the value |
| Left Shift with - / + | Fine flare adjustment |
| [ / ] | Decrease/increase volume density |
| Escape | Close |

`scene.toml` holds the experiment's lighting, dust, model and camera settings.
Restart after editing it. This file is not a game world or a new gameplay schema.
`flare` sets the starting flare intensity; `--flare 0.8` overrides it for one
run. Live adjustments are not saved. F hides/restores the flare without losing
the chosen intensity. Volume/hybrid comparisons remain available with 2/3 and L.

## What to compare

1. Start still: inspect cruiser detail and shadows on the platform. Toggle F to
   distinguish flare from bloom; hold - / + to compare flare intensities.
2. Use Space or W to judge the motes. Try A/D and S as well. For the older
   volume comparison, enable L and use 1/2/3 without resetting.
3. Reset, turn toward and away from the star, and watch a rock cover its disc.
   The flare should attenuate with the visible portion and vanish behind you.
4. Try lower density before increasing it. Dust should not obscure ship detail.
5. Compare frame cadence with the same window size and camera. Displayed FPS
   includes vsync and CPU work; it is **not** an isolated GPU measurement.

## Prototype boundaries

- The mote mode is a representative comparison using the existing shader and
  textures. It uses a fixed recycled world-space pool, not the production
  emitter's speed ramps, three depth-band budgets, or warp transition.
- Hybrid keeps one eighth of that pool alongside the volume. These particles
  still use the existing unlit mote shader, so their brightness does not yet
  respond to stellar shadows.
- The dust is a periodic 64-cubed density texture in a camera-centred box. Its
  sampling offset compensates for camera movement, keeping the pattern fixed
  in space. It has finite box boundaries and a visible repeat distance.
- The star uses a simple emissive sphere, not Phoenix's animated stellar
  material. Its shadow caster is disabled. The single directional light is
  fixed toward the test area, approximating a distant star.
- Flare samples opaque depth at 24 points across the star disc. It has no GPU
  readback and handles the cruiser and moving rock geometry. Transparent
  particles do not occlude it; temporal visibility smoothing is not implemented.
- MSAA is off so the custom flare reads a single-sample depth texture.
- No production rendering behavior, game config, simulation state or networking
  is changed by running this tool.

For a self-terminating capture (mode defaults to 1):

```powershell
cargo run --manifest-path prototypes/native-lighting/Cargo.toml --target-dir target -- --flare 0.8 --capture .phoenix/lighting-lab/flare.png
```

Captures freeze the asteroid pose for comparison. `--cruise` gives the frozen
motes a flight streak; `--no-flare`, `--no-rays` and `--no-shadows` provide
effect comparisons without keyboard input. Scattering now starts off.

It waits eight seconds for asset/pipeline preparation, requests a PNG, and exits
after eleven seconds. Check the output exists and inspect the log for asset or
shader errors; a process exit alone does not establish successful rendering.

The approved motes/shadows/flare selection is integrated into native gameplay under [render.native]; the lab and game both default to flare intensity 3. The game flare also supports MSAA.
