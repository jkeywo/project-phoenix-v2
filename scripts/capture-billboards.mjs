// capture-billboards.mjs — bake every model's far-LOD billboard atlas and swap
// its ladder's final level from a sphere to that billboard.
//
//   node scripts/capture-billboards.mjs              # every model with a ladder
//   node scripts/capture-billboards.mjs alliance_cruiser dynasty_courier
//
// Runs the native `capture-billboard` tool (build it first with
// `cargo build --features capture --bin capture-billboard`) once per model to
// render a transparent yaw-ring atlas, then rewrites each rig sidecar so the far
// band is a `billboard` level sized to the CAPTURED world extent — the actual
// rendered geometry, which the sidecar's own `[extents]` do not reliably match.
//
// Ships have one sidecar and the tool applies their `[base]` rig, so the atlas
// world size already includes the hull's scale → the quad is `[w, h, 1]`. An
// asteroid has several variant sidecars sharing one atlas, and the tool renders
// its base .glb at identity (asteroids ship no `<stem>.model.toml`), so each
// variant's quad is scaled by that variant's `[base]` scale.

import { readdir, readFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { parse as parseToml } from 'smol-toml';
import { ladderFromDoc, replaceLadder, sidecarsForStem } from './viewer-lods.mjs';

const MODELS_DIR = 'assets/models';
const BIN = process.platform === 'win32'
  ? 'target/debug/capture-billboard.exe'
  : 'target/debug/capture-billboard';

const round = (x) => Math.round(x * 1e4) / 1e4;

/** Every model stem that ships a `[[lod]]` ladder (its base sidecar declares one). */
async function stemsWithLadders() {
  const files = await readdir(MODELS_DIR);
  const stems = new Set();
  for (const file of files) {
    if (!file.endsWith('.toml')) continue;
    const text = await readFile(path.join(MODELS_DIR, file), 'utf8');
    let doc;
    try {
      doc = parseToml(text);
    } catch {
      continue;
    }
    if (Array.isArray(doc.lod) && doc.lod.length) {
      // The stem is the filename minus its `.<variant>.toml` / `.model.toml`.
      const withoutExt = file.slice(0, -'.toml'.length);
      const dot = withoutExt.lastIndexOf('.');
      if (dot !== -1) stems.add(withoutExt.slice(0, dot));
    }
  }
  // Drop generated levels' own sidecars (`<stem>_lod1` etc.) — they never have ladders.
  return [...stems].filter((s) => !/_lod\d+$/.test(s)).sort();
}

/** Run the native tool and return its parsed `{world_w, world_h, ...}` line. */
function capture(stem) {
  const glb = `${MODELS_DIR}/${stem}.glb`;
  const png = `${MODELS_DIR}/${stem}_lod3.png`;
  const stdout = execFileSync(BIN, [glb, png], { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 });
  const line = stdout.trim().split('\n').filter(Boolean).pop() ?? '{}';
  const meta = JSON.parse(line);
  return { png, meta };
}

/** Swap the final ladder level for a billboard in every sidecar of the stem. */
async function attachBillboard(stem, png, meta, isAsteroid) {
  const files = await readdir(MODELS_DIR);
  const sidecars = sidecarsForStem(files, stem);
  const written = [];
  for (const file of sidecars) {
    const rel = `${MODELS_DIR}/${file}`;
    const text = await readFile(rel, 'utf8');
    const doc = parseToml(text);
    const levels = ladderFromDoc(doc);
    if (!levels.length) continue;
    // An asteroid variant renders its shared base .glb scaled by its own `[base]`;
    // a ship's atlas already carries its scale.
    const s = isAsteroid ? Number(doc?.base?.scale?.[0] ?? 1) : 1;
    levels[levels.length - 1] = {
      billboard: png,
      scale: [round(meta.world_w * s), round(meta.world_h * s), 1],
      capture: {
        source: `${MODELS_DIR}/${stem}.glb`,
        yaw_views: meta.views,
        resolution: meta.resolution,
        pitch: meta.pitch,
      },
    };
    const next = replaceLadder(text, levels);
    if (next !== text) {
      await writeFile(rel, next);
      written.push(rel);
    }
  }
  return written;
}

async function main() {
  if (!existsSync(BIN)) {
    console.error(`[capture-billboards] ${BIN} not found — build it first:\n  cargo build --features capture --bin capture-billboard`);
    process.exit(1);
  }
  const filters = process.argv.slice(2);
  const all = await stemsWithLadders();
  const stems = filters.length ? all.filter((s) => filters.includes(s)) : all;
  if (!stems.length) {
    console.error('[capture-billboards] no matching models with ladders');
    process.exit(1);
  }

  for (const stem of stems) {
    const isAsteroid = stem.startsWith('asteroid');
    process.stderr.write(`[capture-billboards] ${stem}… `);
    const { png, meta } = capture(stem);
    const written = await attachBillboard(stem, png, meta, isAsteroid);
    console.error(`atlas ${meta.world_w?.toFixed?.(2)}×${meta.world_h?.toFixed?.(2)} → ${written.length} sidecar(s)`);
  }
  console.error(`[capture-billboards] done: ${stems.length} model(s)`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((err) => {
    console.error(`[capture-billboards] ${err.message}`);
    process.exit(1);
  });
}
