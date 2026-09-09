// Asset contract: formats, mip completeness, colour spaces, LUT dimensions,
// and shared atmosphere geometry. No renderer or raw authoring files required.
import fs from 'node:fs/promises';
import assert from 'node:assert/strict';
import { parse } from 'smol-toml';
const { planet } = parse(await fs.readFile('assets/entities/planet_ecumenopolis.toml', 'utf8'));
const bake = JSON.parse(await fs.readFile('scripts/planets/ecumenopolis.json', 'utf8'));
assert.equal(planet.atmosphere.scattering.scale, bake.atmosphereScale, 'Optical-depth table must match shell radius');
const files = [
  [planet.surface.albedo, true], [planet.surface.normal, false],
  [planet.surface.roughness, false], [planet.surface.emissive_colour, true], [planet.surface.emissive_mask, false],
  [planet.clouds.albedo, true], [planet.clouds.opacity, false, 1], [planet.clouds.normal, false],
  [planet.clouds.smog.city_glow, true], [planet.atmosphere.scattering.haze, false, 1],
];
assert.equal(planet.atmosphere.scattering.skyglow, undefined, 'Unused skyglow should not be loaded');
const expectedWidths = new Map([
  [planet.surface.albedo, bake.width], [planet.surface.emissive_mask, bake.width],
  [planet.surface.normal, bake.normalWidth], [planet.surface.emissive_colour, bake.emissionWidth],
  [planet.atmosphere.scattering.haze, bake.hazeWidth], [planet.clouds.normal, bake.smogNormalWidth],
]);
let disk = 0, gpu = 0;
for (const [file, srgb, channels = 4] of files) {
  const b = await fs.readFile(file);
  assert.equal(b.subarray(1, 7).toString(), 'KTX 20', file);
  assert.equal(b.readUInt32LE(12), channels === 1 ? 9 : srgb ? 43 : 37, `${file}: channel format and colour space`);
  const w = b.readUInt32LE(20), h = b.readUInt32LE(24), levels = b.readUInt32LE(40);
  assert.equal(w, h * 2, `${file}: spherical projection`);
  if (expectedWidths.has(file)) assert.equal(w, expectedWidths.get(file), `${file}: authored resolution`);
  assert.equal(levels, Math.floor(Math.log2(w)) + 1, `${file}: complete mip chain`);
  assert.equal(b.readUInt32LE(44), 2, `${file}: lossless Zstd supercompression`);
  for (let i = 0; i < levels; i++) {
    const size = Math.max(1, w >> i) * Math.max(1, h >> i) * channels;
    assert.equal(Number(b.readBigUInt64LE(80 + i * 24 + 16)), size, `${file}: mip byte size`);
    gpu += size;
  }
  disk += b.length;
}
const lut = await fs.readFile(planet.atmosphere.scattering.optical_depth);
assert.equal(lut.readUInt32BE(16), 256); assert.equal(lut.readUInt32BE(20), 128);
console.log(JSON.stringify({ maps: files.length + 1, downloadMiB: (disk + lut.length) / 1048576, residentMiB: (gpu + 256 * 128 * 4) / 1048576 }, null, 2));
