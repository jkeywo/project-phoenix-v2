// average-texture-colour.mjs — the mean base-colour of a model, as an sRGB
// triple for a sphere LOD's `colour` field.
//
//   node scripts/average-texture-colour.mjs assets/models/alliance_battleship.glb
//   → [ 0.3421, 0.3567, 0.3810 ]
//
// The farthest LOD level a model declares is a procedural sphere (see the
// `[[lod]] shape = "sphere"` block in the rig sidecar, src/entities/model_rig.rs).
// A sphere the colour of the hull reads, at 400+ units, as "a ship-sized grey
// thing over there" rather than a default-tinted blob — so this collapses the
// base-colour texture to one representative colour the sidecar can carry.
//
// The renderer feeds `colour` straight to `Color::srgb` (procedural_mesh_material),
// so the number wanted here is the sRGB-encoded mean of the base-colour texels,
// nudged by the material's `baseColorFactor`. `sharp(...).stats()` returns the
// per-channel mean over every pixel; that is the average in the texture's own
// (sRGB) space, which is exactly the space `Color::srgb` reads. No gamma round
// trip, because the destination is gamma space too.
//
// Emissive-only or untextured materials have no base-colour image; the script
// falls back to `baseColorFactor` alone, and to mid-grey if there is not even
// that. It never fails on a model that simply has no texture to average.

import { NodeIO } from '@gltf-transform/core';
import sharp from 'sharp';

/** Round to four decimals — enough for a colour, stable in a TOML diff. */
function round4(x) {
  return Math.round(x * 1e4) / 1e4;
}

/**
 * The mean sRGB colour of a document's base-colour texture, times its
 * `baseColorFactor`. Pure over a parsed document + a decode function so the
 * averaging is testable without a real image codec.
 *
 * `decodeMean` takes `(bytes) => Promise<[r,g,b] 0..1>`; the CLI passes a sharp
 * implementation, a test can pass a stub.
 */
export async function averageBaseColour(document, decodeMean) {
  const materials = document.getRoot().listMaterials();
  // The models here are single-material; if several exist, the first textured
  // one wins — a far sphere does not need a blend of every submaterial.
  let factor = [1, 1, 1];
  let textureMean = null;

  for (const material of materials) {
    const f = material.getBaseColorFactor();
    factor = [f[0], f[1], f[2]];
    const texture = material.getBaseColorTexture();
    if (texture) {
      const image = texture.getImage();
      if (image) {
        textureMean = await decodeMean(image);
        break;
      }
    }
  }

  const base = textureMean ?? [1, 1, 1];
  return [
    round4(base[0] * factor[0]),
    round4(base[1] * factor[1]),
    round4(base[2] * factor[2]),
  ];
}

/** Decode an encoded image (PNG/JPEG bytes) to its mean [r,g,b] in 0..1. */
export async function sharpMean(bytes) {
  const stats = await sharp(Buffer.from(bytes)).stats();
  // channels[0..2] are R,G,B; .mean is 0..255 in the image's own space.
  const [r, g, b] = stats.channels;
  return [r.mean / 255, g.mean / 255, b.mean / 255];
}

async function main() {
  const file = process.argv[2];
  if (!file) {
    console.error('usage: node scripts/average-texture-colour.mjs <model.glb>');
    process.exit(2);
  }
  const document = await new NodeIO().read(file);
  const [r, g, b] = await averageBaseColour(document, sharpMean);
  // Printed as the TOML array the sidecar wants, ready to paste or capture.
  console.log(`[ ${r}, ${g}, ${b} ]`);
}

import { pathToFileURL } from 'node:url';
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    console.error(`[average-texture-colour] ${err.message}`);
    process.exit(1);
  });
}
