import fs from 'node:fs/promises';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import sharp from 'sharp';

const execFileAsync = promisify(execFile);
const args = process.argv.slice(2);
const FROM_GIT = args.includes('--from-git');
const rootArg = args.find((arg) => !arg.startsWith('--'));
const ROOT = path.resolve(rootArg ?? 'assets/planets');
// The source images have generator feathering baked into both vertical edges.
// Blending those edge pixels together merely pulls the dark/faded bands farther
// into the image. Remove that contaminated longitude first, restore the
// authored dimensions, then join two clean edges over a narrow transition.
const TRIM_TEXELS = 48;
const BLEND_TEXELS = 12;

async function webpFiles(directory) {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  const nested = await Promise.all(entries.map((entry) => {
    const child = path.join(directory, entry.name);
    if (entry.isDirectory()) return webpFiles(child);
    return entry.isFile() && entry.name.endsWith('.webp') ? [child] : [];
  }));
  return nested.flat();
}

const smoothstep = (value) => value * value * (3 - 2 * value);

async function stitch(file) {
  // Decode from memory so Windows does not retain a read handle when the
  // transformed WebP replaces its source file.
  const relative = path.relative(process.cwd(), file).replaceAll('\\', '/');
  const sourceBytes = FROM_GIT
    ? (await execFileAsync('git', ['show', `HEAD:${relative}`], {
        encoding: 'buffer',
        maxBuffer: 16 * 1024 * 1024,
      })).stdout
    : await fs.readFile(file);
  const source = sharp(sourceBytes);
  const metadata = await source.metadata();
  const trim = Math.min(TRIM_TEXELS, Math.floor(metadata.width / 8));
  const { data, info } = await source
    .extract({ left: trim, top: 0, width: metadata.width - trim * 2, height: metadata.height })
    .resize(metadata.width, metadata.height, { kernel: 'lanczos3' })
    .raw()
    .toBuffer({ resolveWithObject: true });
  const blend = Math.min(BLEND_TEXELS, Math.floor(info.width / 4));

  for (let distance = 0; distance < blend; distance += 1) {
    const keep = smoothstep(distance / (blend - 1));
    const leftX = distance;
    const rightX = info.width - 1 - distance;
    for (let y = 0; y < info.height; y += 1) {
      const left = (y * info.width + leftX) * info.channels;
      const right = (y * info.width + rightX) * info.channels;
      for (let channel = 0; channel < info.channels; channel += 1) {
        const leftValue = data[left + channel];
        const rightValue = data[right + channel];
        const average = (leftValue + rightValue) * 0.5;
        data[left + channel] = Math.round(average + (leftValue - average) * keep);
        data[right + channel] = Math.round(average + (rightValue - average) * keep);
      }
    }
  }

  // Resampling and edge blending operate on encoded XYZ components. Restore
  // unit length so the repaired normal maps cannot introduce a lighting band.
  if (/normal\.webp$/i.test(file) && info.channels >= 3) {
    for (let pixel = 0; pixel < info.width * info.height; pixel += 1) {
      const offset = pixel * info.channels;
      let x = data[offset] / 127.5 - 1;
      let y = data[offset + 1] / 127.5 - 1;
      let z = data[offset + 2] / 127.5 - 1;
      const length = Math.hypot(x, y, z) || 1;
      x /= length;
      y /= length;
      z /= length;
      data[offset] = Math.round((x + 1) * 127.5);
      data[offset + 1] = Math.round((y + 1) * 127.5);
      data[offset + 2] = Math.round((z + 1) * 127.5);
    }
  }

  const dataMap = /(?:normal|roughness|opacity|mask)\.webp$/i.test(file);
  const encoder = sharp(data, {
    raw: { width: info.width, height: info.height, channels: info.channels },
  });
  const encoded = await (dataMap
    ? encoder.webp({ lossless: true, effort: 6 })
    : encoder.webp({ quality: 95, smartSubsample: true, effort: 6 })
  ).toBuffer();
  await fs.writeFile(file, encoded);

  process.stdout.write(
    `${relative}: ${metadata.width}x${metadata.height}, trimmed ${trim}px/edge, ${encoded.length} bytes, ${dataMap ? 'lossless data' : 'quality-95 colour'}\n`,
  );
}

const files = await webpFiles(ROOT);
for (const file of files.sort()) await stitch(file);
process.stdout.write(
  `repaired ${files.length} planet textures (${TRIM_TEXELS}px trim, ${BLEND_TEXELS}px join)\n`,
);
