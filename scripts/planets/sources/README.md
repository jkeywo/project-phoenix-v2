# Enhanced ecumenopolis albedo

## Natural planet sources

`gas_giant_albedo_enhanced.png` and `ice_moon_albedo_enhanced.png` are built-in
ImageGen edits of their corresponding `raw/planets/*_shader_textures_1k/` albedo
maps, generated on 2026-09-09. Both returned 1774 × 887 images. The natural baker
resamples them to 4096 × 2048; other channels use the supplied source maps.
They add interpreted artwork detail rather than recovering original measurements.

Gas Giant prompt:

Edit this gas giant equirectangular albedo texture into a sharper high-detail texture for a 3D planet. Preserve exactly the 2:1 full-map composition, positions and widths of every major horizontal ochre, cream, white and blue-grey cloud band, all large oval vortices especially the orange oval at lower right, and overall restrained palette. Add convincing finer wisps, turbulent filaments and nested cloud swirls within those existing shapes. Flat unlit base colour; no directional lighting, globe, stars, labels, borders, bevel or vignette. Remove the dark edge frame. Match left and right edges seamlessly, taper longitudinal detail smoothly at poles. Fill the entire rectangular image with the map. Maximum sharpness without crunchy halos or synthetic grid patterns. This is enhanced artwork, not a new planet design.

Ice Moon prompt:

Sharpen and enhance this 2:1 equirectangular ice moon albedo map for a game shader. Preserve the original full-map layout: all major branching dark blue fractures, broad pale icy plates, existing circular craters and relative positions. Add fine ice fissures, frosted mineral grains, subtle compressed ridges and crisp crater rims inside that layout. Keep pale blue-white ice with slate-blue exposed substrate, natural varied texture, clear strong readable major cracks. Flat unlit base colour with no baked directional illumination, no globe, stars, text, border or vignette. Remove the dark frame at edges. Exact 2:1 rectangular map filling canvas, seamless left-right wrap, smoothly reduce longitudinal detail at the poles. No invented cities or glowing neon. Increase fine detail and clarity without changing the moon's geography or adding sharpen halos.

## Ecumenopolis source

`ecumenopolis_albedo_enhanced.png` is a built-in ImageGen edit of
`raw/planets/ecumenopolis_shader_textures_1k/ecumenopolis_albedo_1k.png`, generated
2026-09-09. Returned size: 1774 × 887. The numeric baker resamples it to the
existing 4096 × 2048 runtime texture and applies the same normalized longitude
crop and polar treatment as the other layers. This is enhanced artwork, not
recovered ground-truth detail. Night-light maps still use the original sources.

Remove `albedoSource` from `../ecumenopolis.json` to bake from the original again.

Prompt used:

Use case: precise-object-edit. Edit target: supplied ecumenopolis albedo texture. Upscale and deblur this exact 2:1 flat texture to a crisp high resolution texture, aiming for 3840x1920. Preserve the full canvas, projection, every district boundary, street route, circular structure, composition and landmark position exactly; no crop, perspective, rotation, shift or redesign. Recover clear fine rooftop, panel and street-edge detail within existing shapes. Preserve the muted grey-blue metal and subtle warm accents, overall exposure and colour balance. It is a base-colour map for a real-time spherical planet, not a rendered scene: no new lighting, shadows, bloom, clouds, haze, glow, text, borders or watermarks. Do not draw a sphere. Avoid uniform checkerboard pixels. Only increase clarity and believable fine detail of the existing image; all larger features must remain registered to the original.
