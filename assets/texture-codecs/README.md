# Browser planet textures

The Gas Giant, Ice Moon and Ecumenopolis 4096×2048 opaque sRGB base maps use
UASTC in browser hosts and the model viewer. Other maps keep their formats.
Native rendering keeps the originals. Entity files still name those originals,
preserving native capture and baking.

`src/entities/planet.rs::load_planet_image` routes these browser maps to `.ptex`.
`planet_texture.rs` registers a Bevy Image loader after RenderPlugin publishes
enabled device formats. It selects ASTC, BC7, ETC2, then RGBA8. A local worker
transcodes every mip; Bevy parses the result against those supported formats.
Read, transcode or parse failure loads the original into the same Image handle.
An actual device loss remains a renderer recovery concern.

Only one worker decodes at a time; it terminates on completion or after 30s.
There is no global fetch hook, prototype UI, CDN or decoded-image JS cache.
URLs support site subdirectories. The original downloads only on fallback,
but remains in the distribution for compatibility.

Base downloads (MiB): Gas Giant 21.23 → 6.80; Ice Moon 25.93 → 8.84;
Ecumenopolis 23.42 → 9.14. The shared transcoder adds 0.56 MiB on first load.
Compressed storage including mips: 42.67 → 10.67 MiB. RGBA keeps the original
GPU cost. These are byte counts, not GPU memory profiler measurements.

## Rebuild and verify

After changing any base bake, run `node scripts/planets/bake-uastc.mjs`.
It skips variants whose source and output hashes still match; `--force` rebakes all.
The project-pinned `ktx2tools` encodes UASTC quality 3, RDO lambda 0.5,
deterministic RDO, Zstd 9 and 13 mip levels. It regenerates Khronos KTX2
templates and the SHA-256 manifest. `opaque-4k-templates.json` describes the
shared shape/format, independent of pixel content. The map list lives in
`scripts/planets/uastc-maps.mjs`; the worker rejects other layouts and alpha.

- `node scripts/planets/bake-uastc.mjs --check`: source/artifact drift.
- `npx vitest run tests/client/uastc.test.js`: lifecycle and hashes.
- `node scripts/planets/measure-uastc.mjs`: RGB error/PSNR against originals,
  independent of animated layers; writes decoded comparisons under `target/`.
- Build viewer, then `node scripts/planets/check-uastc-browser.mjs`: real
  Bevy/WebGL uploads, compression, no-compression hardware, missing worker,
  missing variant, bad template, invalid worker output, site subdirectory.
- Combat Test and Falling Skyway render smoke tests exercise in-game loading
  and fallback for all three bases. The viewer check covers every listed map;
  `UASTC_ENTITIES=moon_ice,planet_ecumenopolis` selects just those two.

`basis/` vendors `three@0.180.0`'s `examples/jsm/libs/basis/` runtime.
The manifest pins hashes. Basis is Apache-2.0 (`basis/LICENSE`); Three's MIT
license is separate. Upstream: https://github.com/BinomialLLC/basis_universal .
Safari, Firefox and physical mobile GPUs still need device checks.
