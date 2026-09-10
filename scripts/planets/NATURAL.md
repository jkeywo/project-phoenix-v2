# Gas Giant and Ice Moon materials

`bake-natural.mjs` uses the original 1K source sets plus enhanced base artwork
in `sources/`. Run from the repository root:

```powershell
node scripts/planets/bake-natural.mjs raw/planets
node scripts/planets/bake-uastc.mjs
node scripts/planets/check-natural.mjs
node scripts/planets/bake-uastc.mjs --check
```

The generated artwork adds interpreted fine detail. It is not recovered source
resolution. Source geometry/masks stay authoritative for effect placement.
The baker applies the same normalized frame crop, seam repair and narrow polar
fade to every map, renormalizes mixed frost normals, and resizes packed channels
independently. Alpha data is encoded above zero to avoid premultiplication loss.

Each body's lossless set uses 11 maps, 63.67 MiB including GPU mipmaps: 4K base colour, 2K
normals, 1K material/effect masks and cloud colour, 512px emission colour,
aurora, cloud normals and opacity, 256px atmosphere density, and a 256×128
optical-depth LUT. Opacity/density use R8; other KTX2 maps use RGBA8. Colour
maps are sRGB and data maps linear. KTX2 uses Zstd and complete mip chains;
the lossless set needs no GPU compression extension. Browser game/viewer loads
use UASTC variants for both base colours, reducing each body's storage
to 31.67 MiB on supported GPUs. The local Basis worker chooses from Bevy's
enabled formats, with RGBA and original-file fallbacks. Native keeps the
lossless set. See `assets/texture-codecs/README.md` for the loader and checks.

| Map | Gas Giant | Ice Moon |
|---|---|---|
| Material RGBA | Roughness, band/zone flow, storm intensity, encoded height | Roughness, AO, thin/subsurface ice, encoded frost |
| Effect RGB | Nightglow, storm-masked lightning, unused | Nightglow, fracture-masked vents, thermal fractures |
| Upper shell | Cloud colour, normals, opacity; independent bounded band motion | Thin moving frost/haze, low opacity |
| Atmosphere | Warm aerosol scattering and masked polar aurora | Weak blue scattering and restrained polar aurora |

`surface.natural` selects the natural material without reusing city-light channel
semantics. `clouds.dynamics` controls shell opacity, normal strength and flow.
The shader wraps base rotation and bounds latitude shear, preventing textures
from stretching indefinitely during a long session. Storm masks modulate local
turbulence, and cloud shadows use the moving shell's own flow coordinates.
Lightning and vents pulse at independent regional phases and remain attached to
authored masks. These are visual effects, not a weather simulation.

The ice material uses dielectric GGX highlights, roughness/AO derived from
frost, rock and thin ice, and a blue wrap-light approximation limited by the
thin/subsurface-ice mask. It does not make the globe transparent or simulate
light transmission through its diameter. Thermal/vent emission is deliberately
sparse. The atmosphere reuses the existing twelve-sample single-scattering path;
this is not Bruneton's full multiple-scattering implementation.

Research informing the implementation:

- [NASA: Jupiter's belts and zones](https://science.nasa.gov/jupiter/jupiter-facts/)
  describes alternating east/west flow; the shader approximates this through
  bounded latitude-dependent UV advection.
- [NASA: Jovian lightning and moonlit clouds](https://science.nasa.gov/photojournal/jovian-lightning-and-moonlit-clouds/)
  motivates localized flashes illuminating cloud regions.
- [Bruneton's atmospheric scattering implementation](https://ebruneton.github.io/precomputed_atmospheric_scattering/)
  documents lookup-based optical transport, non-Earth density profiles and a
  WebGL2 implementation. We retain the project's smaller existing LUT model.
- [NVIDIA GPU Gems: subsurface-scattering approximations](https://developer.nvidia.com/gpugems/gpugems/part-iii-materials/chapter-16-real-time-approximations-subsurface-scattering)
  supplies the rationale for localized wrap lighting as an inexpensive visual
  approximation. Colour and extent here are authored for fictional ice.

Capture either entity using `PLANET_ENTITY`; both capture tools scale their
camera distances to the body's radius. Compare day, terminator, night, close and
pole views. Set `PLANET_MOTION=1` for the browser capture to also compare two
fixed-camera frames six seconds apart (daytime cloud flow for gas, night vents
for ice). Natural atmospheres soften solar occultation at the local horizon so
the twelve integration samples do not draw hard shadow arcs. WebGL SwiftShader verifies
shader compatibility and appearance, not physical GPU performance.

Validation for this revision: 37 targeted planet/viewer tests passed; the WASM
viewer and native capture example built; five views of each planet rendered in
WebGL and Vulkan with all textures loaded. Fixed-camera browser frames changed
for both cloud flow and ice emission. The ecumenopolis browser regression
capture reported no errors. Texture contracts and PASM validate/scan/traceability
passed. Encoded runtime assets total 28.97 MiB for gas and 33.70 MiB for ice.
