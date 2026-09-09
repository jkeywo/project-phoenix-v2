// Numeric material baker. All channels derive from one city layout; the raw
// maps supply macro structure, zoning, weather, and effect placement.
// node scripts/planets/bake-ecumenopolis.mjs [raw-directory] [output-directory]
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { createRequire } from 'node:module';
import sharp from 'sharp';

const here = path.dirname(fileURLToPath(import.meta.url));
const cfg = JSON.parse(await fs.readFile(path.join(here, 'ecumenopolis.json'), 'utf8'));
const source = path.resolve(process.argv[2] ?? 'raw/planets/ecumenopolis_shader_textures_1k');
const output = path.resolve(process.argv[3] ?? 'assets/planets/ecumenopolis');
const temporary = path.resolve('target/planet-bake');
await fs.mkdir(output, { recursive: true });
await fs.mkdir(temporary, { recursive: true });
const require = createRequire(import.meta.url);
const toktx = require.resolve('ktx2tools/toktx.js');
const { width: W, height: H } = cfg;
const count = W * H;
const clamp = x => Math.max(0, Math.min(1, x));
const byte = x => Math.round(clamp(x) * 255);
const smooth = x => { x = clamp(x); return x * x * (3 - 2 * x); };
function hash(x, y, salt = 0) {
  let n = Math.imul(x + cfg.seed, 374761393) ^ Math.imul(y + salt, 668265263);
  n = Math.imul(n ^ (n >>> 13), 1274126177);
  return ((n ^ (n >>> 16)) >>> 0) / 4294967296;
}
async function read(name, width = W, height = H) {
  const filename = name === 'albedo' && cfg.albedoSource
    ? path.resolve(here, cfg.albedoSource)
    : path.join(source, `ecumenopolis_${name}_1k.png`);
  const bytes = await fs.readFile(filename);
  const meta = await sharp(bytes).metadata();
  // sourceTrim is expressed in the original 1024px source's coordinate system.
  // Enhanced albedo keeps the same normalized crop as every aligned mask.
  const trim = Math.round(cfg.sourceTrim * meta.width / 1024);
  let { data } = await sharp(bytes).extract({ left: trim, top: 0, width: meta.width - trim * 2, height: meta.height })
    .resize(width, height).toColourspace('srgb').ensureAlpha().raw().toBuffer({ resolveWithObject: true });
  if (name.endsWith('_rgba')) {
    // Alpha is an independent mask, never coverage. Resize each channel alone
    // so image-library alpha premultiplication cannot mix district meanings.
    data = Buffer.alloc(width * height * 4);
    for (let c = 0; c < 4; c++) {
      const channel = await sharp(bytes).extract({ left: trim, top: 0, width: meta.width - trim * 2, height: meta.height }).extractChannel(c).resize(width, height).raw().toBuffer();
      for (let p = 0; p < width * height; p++) data[p * 4 + c] = channel[p];
    }
  }
  // Identical longitude repair on EVERY source map, including packed alpha.
  const band = Math.round(width / 85);
  for (let y = 0; y < height; y++) for (let x = 0; x < band; x++) {
    const weight = smooth(x / (band - 1));
    for (let c = 0; c < 4; c++) {
      const a = (y * width + x) * 4 + c, b = (y * width + width - 1 - x) * 4 + c;
      const avg = (data[a] + data[b]) * 0.5;
      data[a] = Math.round(avg + (data[a] - avg) * weight);
      data[b] = Math.round(avg + (data[b] - avg) * weight);
    }
  }
  return data;
}
async function resizeChannels(data, width, height, outputWidth) {
  const outputHeight = outputWidth / 2;
  const resized = Buffer.alloc(outputWidth * outputHeight * 4);
  for (let c = 0; c < 4; c++) {
    const channel = await sharp(data, { raw: { width, height, channels: 4 } })
      .extractChannel(c).resize(outputWidth, outputHeight).raw().toBuffer();
    for (let p = 0; p < outputWidth * outputHeight; p++) resized[p * 4 + c] = channel[p];
  }
  return resized;
}
async function write(name, data, width, height, srgb, normal = false, singleChannel = false) {
  // A planar cap keeps streets from converging at the equirectangular poles.
  // Every layer samples the same patch and blends over 30–45 degrees, keeping
  // surface, illumination and weather registered. Flatten tangent normals in
  // the cap because the source tangent basis does not survive reprojection.
  const original = Buffer.from(data);
  for (let y = 0; y < height; y++) {
    const theta = Math.PI * y / (height - 1);
    const latitudeDistance = Math.min(theta, Math.PI - theta);
    const blend = smooth((Math.PI / 4 - latitudeDistance) / (Math.PI / 12));
    if (blend === 0) continue;
    const radius = Math.sin(theta) * 0.38;
    for (let x = 0; x < width; x++) {
      const longitude = x / width * Math.PI * 2;
      const sx = (0.5 + radius * Math.cos(longitude)) * (width - 1);
      const sy = (0.5 + radius * Math.sin(longitude)) * (height - 1);
      const ix = Math.floor(sx), iy = Math.floor(sy), fx = sx - ix, fy = sy - iy;
      for (let c = 0; c < 4; c++) {
        const sample = (xx, yy) => original[(yy * width + xx) * 4 + c];
        const cap = normal ? [128, 128, 255, 255][c]
          : (sample(ix, iy) * (1 - fx) + sample(ix + 1, iy) * fx) * (1 - fy)
            + (sample(ix, iy + 1) * (1 - fx) + sample(ix + 1, iy + 1) * fx) * fy;
        const i = (y * width + x) * 4 + c;
        data[i] = Math.round(original[i] * (1 - blend) + cap * blend);
      }
    }
  }
  const png = path.join(temporary, name + '.png');
  const pngImage = sharp(data, { raw: { width, height, channels: 4 } });
  await (singleChannel ? pngImage.extractChannel(0) : pngImage).png().toFile(png);
  const args = ['--t2', '--genmipmap', '--zcmp', '9', '--assign_oetf', srgb ? 'srgb' : 'linear'];
  if (normal) args.push('--normal_mode');
  if (singleChannel) args.push('--target_type', 'R');
  execFileSync(process.execPath, [toktx, ...args, path.join(output, name + '.ktx2'), png], { stdio: 'pipe' });
  console.log(`${name}: ${width}x${height}, mipmapped ${srgb ? 'sRGB' : 'linear'}`);
}

const names = ['albedo', 'height', 'roughness', 'ao', 'infrastructure_density', 'zone_mask_rgba', 'material_mask_rgba', 'windows_mask', 'lights_mask', 'neon_mask', 'thermal_emissive', 'traffic_lines_mask'];
const maps = {};
for (const name of names) maps[name] = await read(name);
const albedo = Buffer.alloc(count * 4), material = Buffer.alloc(count * 4), lights = Buffer.alloc(count * 4), emission = Buffer.alloc(count * 4);
const height = new Float32Array(count);
for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
  const p = y * W + x, i = p * 4;
  const polar = smooth(Math.sin(Math.PI * y / (H - 1)) * 8);
  const seam = smooth(Math.min(x, W - 1 - x) / 24);
  const detail = polar * seam;
  let district = 0;
  for (let c = 1; c < 4; c++) if (maps.zone_mask_rgba[i + c] > maps.zone_mask_rgba[i + district]) district = c;
  const density = maps.infrastructure_density[i] / 255;
  // Each neighbourhood rotates its block grid; several block sizes prevent
  // the surface reading as one uninterrupted checkerboard.
  const sectorX = Math.floor(x / 96), sectorY = Math.floor(y / 96);
  const rotation = hash(sectorX, sectorY) > 0.5;
  const xx = rotation ? y : x, yy = rotation ? x : y;
  const size = cfg.blockSize + Math.floor(hash(sectorX, sectorY, 3) * 3) * 2;
  const bx = Math.floor(xx / size), by = Math.floor(yy / size);
  const fx = xx % size, fy = yy % size;
  const random = hash(bx, by, sectorX + sectorY * 131);
  const building = fx > 1 && fy > 1 && fx < size - 1 && fy < size - 1 && random < 0.5 + density * 0.5;
  const road = fx < 1 || fy < 1;
  // Vary roof setbacks and courtyards for daylight relief only. Keep the
  // established windows/neon/traffic layout below exactly as authored before.
  const surfaceBuilding = building
    && fx > 1 + hash(bx, by, 101) * 1.5
    && fy < size - 1 - hash(bx, by, 103) * 2.5
    && !(hash(bx, by, 107) > 0.7 && fx > size * 0.5 && fy > size * 0.5);
  const roof = surfaceBuilding ? (0.32 + random * 0.55) : 0.06;
  height[p] = maps.height[i] / 255 * 0.45 + roof * detail * 0.55;
  const lit = building && ((fx % 3 === 0 && fy > 2) || fy === 3) && random > 0.45;
  const neon = building && district === 1 && (fx === 3 || fy === size - 3) && random > 0.65;
  const traffic = road && (fx === 0 || fy === 0) && random > 0.6;
  const palette = cfg.districtColours[district], light = cfg.lightColours[district];
  for (let c = 0; c < 3; c++) {
    const generated = palette[c] * (surfaceBuilding ? 0.55 + random * 0.65 : 0.22);
    albedo[i + c] = Math.round(maps.albedo[i + c] * (1 - detail * 0.35) + generated * detail * 0.35);
    emission[i + c] = light[c];
  }
  albedo[i + 3] = emission[i + 3] = 255;
  material[i] = byte(maps.roughness[i] / 255 * 0.7 + (surfaceBuilding ? 0.16 : 0.28));
  material[i + 1] = byte(maps.ao[i] / 255 * (surfaceBuilding ? 1 : 0.78));
  material[i + 2] = byte(maps.material_mask_rgba[i] / 255 * (surfaceBuilding ? 0.75 : 0.3));
  // Larger, stable activity patches survive minification. This formerly spare
  // height channel selects daylight lights without changing any night-light map.
  // Reserve the lower half: zero alpha makes image encoders discard RGB data.
  material[i + 3] = hash(Math.floor(x / 20), Math.floor(y / 20), 109) > 0.68 ? 255 : 128;
  lights[i] = byte((lit ? 0.65 : 0) * detail + maps.windows_mask[i] / 255 * 0.15 + maps.lights_mask[i] / 255 * 0.4);
  lights[i + 1] = byte((neon ? 0.8 : 0) * detail + maps.neon_mask[i] / 255 * 0.3);
  lights[i + 2] = byte(maps.thermal_emissive[i] / 255 * (0.2 + (district === 0 ? 0.65 : 0)));
  lights[i + 3] = byte((traffic ? 0.9 : 0) * detail + maps.traffic_lines_mask[i] / 255 * 0.25);
}
const normal = Buffer.alloc(count * 4);
for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
  const p = y * W + x, i = p * 4;
  const dx = (height[y * W + (x + 1) % W] - height[y * W + (x + W - 1) % W]) * cfg.normalStrength;
  const dy = (height[Math.min(H - 1, y + 1) * W + x] - height[Math.max(0, y - 1) * W + x]) * cfg.normalStrength;
  const length = Math.hypot(dx, dy, 1);
  normal[i] = byte(0.5 - dx / length * 0.5);
  normal[i + 1] = byte(0.5 - dy / length * 0.5);
  normal[i + 2] = byte(0.5 + 0.5 / length); normal[i + 3] = 255;
}
// Bake the low-frequency illumination that leaks into smog from the SAME
// light channels, so unlit districts cannot illuminate the cloud layer.
const glow = Buffer.alloc(count * 4);
for (let i = 0; i < glow.length; i += 4) {
  const energy = clamp(lights[i] / 255 + lights[i + 1] / 255 * 0.5 + lights[i + 3] / 255 * 0.3);
  for (let c = 0; c < 3; c++) glow[i + c] = Math.round(emission[i + c] * energy);
  glow[i + 3] = 255;
}
await write('city_glow', await sharp(glow, { raw: { width: W, height: H, channels: 4 } }).resize(512, 256).blur(2).raw().toBuffer(), 512, 256, true);
await write('city_albedo', albedo, W, H, true);
await write('city_normal', await resizeChannels(normal, W, H, cfg.normalWidth), cfg.normalWidth, cfg.normalWidth / 2, false, true);
await write('city_lights', lights, W, H, false);
for (const [name, data, srgb] of [['city_material', material, false], ['city_emission', emission, true]]) {
  const size = name === 'city_emission' ? cfg.emissionWidth : W / 2;
  await write(name, await resizeChannels(data, W, H, size), size, size / 2, srgb);
}
for (const [name, src, srgb, normalMap] of [
  ['smog_albedo', 'smog_albedo', true, false], ['smog_normal', 'smog_normal', false, true],
  ['smog_opacity', 'smog_opacity', false, false], ['haze', 'atmosphere_haze_mask', false, false],
]) {
  const size = name === 'haze' ? cfg.hazeWidth : name === 'smog_normal' ? cfg.smogNormalWidth : 1024;
  const data = await read(src, size, size / 2);
  await write(name, data, size, size / 2, srgb, normalMap, name === 'haze' || name === 'smog_opacity');
}

// Optical depth / shell thickness for exponential molecular and aerosol
// density profiles. Log encoding preserves both thin vertical and long limb
// paths in portable RGBA8. Star occultation is evaluated analytically at runtime.
const lw = 256, lh = 128, lut = Buffer.alloc(lw * lh * 4), outer = cfg.atmosphereScale;
for (let y = 0; y < lh; y++) for (let x = 0; x < lw; x++) {
  const r = 1 + (outer - 1) * y / (lh - 1), mu = x / (lw - 1) * 2 - 1;
  const distance = -r * mu + Math.sqrt(r * r * mu * mu + outer * outer - r * r);
  let rayleigh = 0, mie = 0;
  for (let s = 0; s < 64; s++) {
    const t = distance * (s + 0.5) / 64;
    const altitude = Math.max(0, (Math.sqrt(r * r + t * t + 2 * r * mu * t) - 1) / (outer - 1));
    rayleigh += Math.exp(-altitude * 6) * distance / 64 / (outer - 1);
    mie += Math.exp(-altitude * 12) * distance / 64 / (outer - 1);
  }
  const i = (y * lw + x) * 4;
  lut[i] = byte(1 - Math.exp(-rayleigh / 8)); lut[i + 1] = byte(1 - Math.exp(-mie / 8)); lut[i + 3] = 255;
}
// LUT must not acquire a longitude seam or mipmaps: this is parameter space.
await sharp(lut, { raw: { width: lw, height: lh, channels: 4 } }).png().toFile(path.join(output, 'optical_depth.png'));
console.log('Baked aligned ecumenopolis layers and optical-depth table.');
