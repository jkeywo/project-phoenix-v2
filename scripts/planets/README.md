# Ecumenopolis material baking

Gas Giant and Ice Moon baking and research notes are in [NATURAL.md](NATURAL.md).

The numeric baker preserves the supplied planet's broad structures while adding
coordinated buildings, roads and illumination at 4096 × 2048. District and
material masks guide palette and physical response. Original inputs remain under
the gitignored `raw/planets/` directory. The base colour uses an enhanced source
in `sources/`, documented there with its generation prompt and original dimensions.
Other layers still use the original sources. Remove `albedoSource` from the bake
configuration to restore the original base colour.

From the repository root, after installing the ordinary npm dependencies:

```powershell
node scripts/planets/bake-ecumenopolis.mjs
node scripts/planets/check-ecumenopolis.mjs
```

The baker accepts explicit raw and output directories as its two arguments.
`ecumenopolis.json` carries reproducible layout settings and palettes. Temporary
PNG intermediates go under `target/planet-bake/`. KTX2 files are RGBA8 or R8 with full
mip chains and lossless Zstd compression, supported by the existing Bevy build.
The current 11-map set is about 53.9 MiB on disk and 112.2 MiB decoded including
mipmaps, down from 60.9 and 163.5 MiB. Zstd reduces download size, not GPU memory.
Base colour and light masks remain 4096 × 2048; surface normals and material
channels are 2048 × 1024. District light colour and smog normals use 512 × 256,
and haze uses 256 × 128. Smog opacity remains 1024 × 512. Opacity and haze store
only their sampled red channel in R8. The nearly black skyglow map is omitted;
the renderer still accepts an optional skyglow for other authored atmospheres.
Revisit the resolution or a
portable GPU-compression path if multiple such planets must be resident together.
UASTC is not enabled: it requires a separately verified browser transcoder path.

## Runtime channels

| Map | Colour space | Channels |
|---|---|---|
| city_albedo | sRGB | Surface colour |
| city_normal | linear | Tangent-space XYZ |
| city_material | linear | R roughness, G ambient occlusion, B metallic, A daytime activity |
| city_emission | sRGB | District light colour |
| city_lights | linear | R windows, G neon, B thermal, A traffic |
| city_glow | sRGB | Filtered city illumination for smog |
| smog_albedo / smog_normal / smog_opacity | sRGB / linear / linear | Moving cloud shell |
| haze | linear | R pollution density |
| optical_depth | linear | Log-encoded molecular and aerosol columns |

Height drives generated normals. No displacement or ground-scale geometry is used.
Daylight roof setbacks and courtyards vary independently of the original night
windows, neon and traffic. Material alpha keeps selected activity patches on at
60% intensity in daylight, blending into full illumination across the terminator.
Activity is encoded from 128 (off) to 255 (on); avoiding zero alpha prevents
image encoders from discarding the independent roughness/AO/metal RGB channels.
All maps share a blended planar polar cap to avoid converging streets; tangent
normals flatten within that cap rather than using an incorrect reprojected basis.
The LUT is 256 × 128, with sun cosine along X and normalized shell altitude
along Y. It encodes `1 - exp(-optical_depth / 8)` for exponential density
falloffs 6 and 12. Its shell ratio MUST match `[planet.atmosphere.scattering]`;
the check script enforces that relationship. Sampling stays inside texel centres
so the planet loader's longitude wrap cannot cross the LUT boundary.

## Visual verification

```powershell
# Build the real browser viewer, then capture it without modifying the page.
$env:NO_COLOR = 'true'
trunk build --config viewer-trunk.toml
node scripts/capture-planet.mjs dist-viewer target/planet-browser

# Same camera/light poses through native Bevy and the shared material builders.
cargo run --features capture --example capture_planet -- target/planet-native
```

The browser capture uses Playwright from `tests/smoke/node_modules`; set
`PLAYWRIGHT_PACKAGE_ROOT` to another installed package root when necessary.
Set `PLANET_ENTITY` to an existing entity path to check a legacy planet.
Compare day, terminator, night, close and pole images, then rotate the planet
interactively to inspect seams and shimmer. The native timing includes startup,
readback and scheduling, and is not a GPU benchmark. Browser SwiftShader captures
prove compatibility and appearance, not hardware performance.

The atmosphere integrates single scattering. Solar transmission uses the LUT;
planet shadow uses ray/sphere intersections. The transparent shell approximates
RGB extinction with scalar luminance coverage. Regional aerosol modulation uses
the sampled column along the sun path too. These approximations keep the browser
implementation compact; they are not a full multiple-scattering sky model.
