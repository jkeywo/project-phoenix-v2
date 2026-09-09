# Enhanced ecumenopolis albedo

`ecumenopolis_albedo_enhanced.png` is a built-in ImageGen edit of
`raw/planets/ecumenopolis_shader_textures_1k/ecumenopolis_albedo_1k.png`, generated
2026-09-09. Returned size: 1774 × 887. The numeric baker resamples it to the
existing 4096 × 2048 runtime texture and applies the same normalized longitude
crop and polar treatment as the other layers. This is enhanced artwork, not
recovered ground-truth detail. Night-light maps still use the original sources.

Remove `albedoSource` from `../ecumenopolis.json` to bake from the original again.

Prompt used:

Use case: precise-object-edit. Edit target: supplied ecumenopolis albedo texture. Upscale and deblur this exact 2:1 flat texture to a crisp high resolution texture, aiming for 3840x1920. Preserve the full canvas, projection, every district boundary, street route, circular structure, composition and landmark position exactly; no crop, perspective, rotation, shift or redesign. Recover clear fine rooftop, panel and street-edge detail within existing shapes. Preserve the muted grey-blue metal and subtle warm accents, overall exposure and colour balance. It is a base-colour map for a real-time spherical planet, not a rendered scene: no new lighting, shadows, bloom, clouds, haze, glow, text, borders or watermarks. Do not draw a sphere. Avoid uniform checkerboard pixels. Only increase clarity and believable fine detail of the existing image; all larger features must remain registered to the original.
